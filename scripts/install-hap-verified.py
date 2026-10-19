#!/usr/bin/env python3
"""Install an already signed HAP after verifying both transferred file hashes."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import uuid


class InstallError(RuntimeError):
    def __init__(self, stage: str, message: str, **facts: str | int):
        super().__init__(message)
        self.record = {"success": False, "stage": stage, "error": message, **facts}


def emit(record: dict) -> None:
    print(json.dumps(record, ensure_ascii=True), flush=True)


def file_sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def require_success(output: str, action: str) -> None:
    # HDC and bm can exit zero even when the operation reports [Fail].
    if re.search(r"\[(?:fail|error)\]|\b(?:failed|failure|error|unsuccessful)\b", output, re.I):
        raise InstallError(action, "device reported failure")
    expected = r"install\s+bundle\s+success(?:ful(?:ly)?)\b" if action == "install" else r"start\s+ability\s+success(?:ful(?:ly)?)\b"
    if not re.search(expected, output, re.I):
        raise InstallError(action, "expected success message was absent")


class Device:
    def __init__(self, hdc: str, target: str):
        self.prefix = [hdc, "-t", target]

    def run(self, stage: str, *args: str, timeout: float = 30) -> str:
        try:
            result = subprocess.run(
                [*self.prefix, *args], capture_output=True, text=True,
                encoding="utf-8", errors="replace", timeout=timeout, check=False,
            )
        except subprocess.TimeoutExpired:
            raise InstallError(stage, "HDC timeout") from None
        except OSError as error:
            raise InstallError(stage, "cannot execute HDC", errno=error.errno or 0) from None
        output = result.stdout + result.stderr
        if result.returncode != 0 or re.search(r"\[(?:fail|error)\]", output, re.I):
            raise InstallError(stage, "HDC operation failed", exitCode=result.returncode)
        return output

    def shell(self, stage: str, command: str, timeout: float = 30) -> str:
        marker = "PAWS_INSTALL_OK_" + uuid.uuid4().hex
        output = self.run(stage, "shell", command + " && printf '\\n%s\\n' "
                          + shlex.quote(marker), timeout=timeout)
        if output.splitlines().count(marker) != 1:
            raise InstallError(stage, "remote command did not confirm success")
        return output

    def sha256(self, path: str, expected: str, stage: str) -> None:
        output = self.shell(stage, f"sha256sum {shlex.quote(path)}")
        hashes = re.findall(r"(?mi)^([0-9a-f]{64})[ \t]+\*?" + re.escape(path) + r"[ \t]*$", output)
        if len(hashes) != 1:
            raise InstallError(stage, "remote SHA256 output was missing or ambiguous")
        actual = hashes[0].lower()
        if actual != expected:
            raise InstallError(stage, "SHA256 mismatch", expectedSHA256=expected, actualSHA256=actual)


def install(args: argparse.Namespace) -> dict:
    hap = Path(args.hap).expanduser().resolve(strict=True)
    if not hap.is_file():
        raise InstallError("source", "HAP must be a regular file")
    hap_sha = file_sha256(hap)
    token = f"{hap_sha[:12]}-{uuid.uuid4().hex[:8]}"
    remote_dir = f"/data/local/tmp/paws-install-{token}"
    remote_hap = f"{remote_dir}/{token}.hap"
    remote_gzip = remote_hap + ".gz"
    device = Device(args.hdc, args.target)
    local_gzip: Path | None = None
    remote_owned = False
    failure: InstallError | None = None
    try:
        fd, name = tempfile.mkstemp(prefix="paws-install-", suffix=".hap.gz")
        local_gzip = Path(name)
        with os.fdopen(fd, "wb") as destination:
            os.fchmod(destination.fileno(), 0o600)
            with hap.open("rb") as source, gzip.GzipFile(
                filename="", mode="wb", fileobj=destination, mtime=0,
            ) as compressed:
                shutil.copyfileobj(source, compressed, length=1024 * 1024)
        gzip_sha = file_sha256(local_gzip)
        emit({"stage": "prepared", "hapPath": str(hap), "hapSHA256": hap_sha,
              "gzipSHA256": gzip_sha, "remoteHapPath": remote_hap})
        # Atomic ownership also handles a UUID collision: never reuse or clean
        # another operation's files. The output HAP has never existed here.
        device.shell("reserve", f"mkdir -m 700 {shlex.quote(remote_dir)}")
        remote_owned = True
        device.run("transfer", "file", "send", str(local_gzip), remote_gzip, timeout=120)
        device.sha256(remote_gzip, gzip_sha, "verify-gzip")
        device.shell("decompress", f"gzip -df {shlex.quote(remote_gzip)}", timeout=120)
        device.sha256(remote_hap, hap_sha, "verify-hap")
        emit({"stage": "verified", "hapSHA256": hap_sha, "remoteHapPath": remote_hap})
        output = device.shell("install", f"bm install -r -p {shlex.quote(remote_hap)}", timeout=120)
        require_success(output, "install")
        if args.launch:
            output = device.shell("launch", "aa start -b " + shlex.quote(args.bundle)
                                  + " -a " + shlex.quote(args.ability))
            require_success(output, "launch")
    except InstallError as error:
        failure = error
    except OSError as error:
        failure = InstallError("local", "local file operation failed", errno=error.errno or 0)
    finally:
        if remote_owned:
            try:
                device.shell("cleanup", f"rm -f -- {shlex.quote(remote_hap)} {shlex.quote(remote_gzip)}")
                device.shell("cleanup-directory", f"rmdir {shlex.quote(remote_dir)}")
            except InstallError as error:
                if failure is not None:
                    failure.record["remoteCleanupFailed"] = True
                failure = failure or error
        if local_gzip is not None:
            try:
                local_gzip.unlink(missing_ok=True)
            except OSError as error:
                if failure is not None:
                    failure.record["localCleanupFailed"] = True
                failure = failure or InstallError("cleanup-local", "cannot remove local gzip", errno=error.errno or 0)
    if failure is not None:
        raise failure
    return {"success": True, "hapPath": str(hap), "hapSHA256": hap_sha,
            "installed": True, "launched": bool(args.launch)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--hap", required=True, help="already signed HAP")
    parser.add_argument("--target", required=True, help="HDC target")
    parser.add_argument("--hdc", required=True, help="HDC executable")
    parser.add_argument("--bundle", default="com.richerfu.paws")
    parser.add_argument("--ability", default="EntryAbility")
    parser.add_argument("--launch", action="store_true")
    args = parser.parse_args(argv)
    try:
        emit(install(args))
        return 0
    except InstallError as error:
        emit(error.record)
    except OSError as error:
        emit({"success": False, "stage": "source", "error": "local file unavailable",
              "errno": error.errno or 0})
    except KeyboardInterrupt:
        emit({"success": False, "stage": "interrupted"})
        return 130
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
