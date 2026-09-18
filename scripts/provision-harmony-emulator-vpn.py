#!/usr/bin/env python3
"""Provision Paws VPN authorization in a stopped DevEco emulator instance.

Only the instance userdata overlay is replaced. The original overlay is kept
beside it as a complete backup. Never run this against a running emulator.
"""

import argparse
import json
import os
from pathlib import Path
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
from datetime import datetime


RDB = "/app/el1/0/database/com.ohos.settingsdata/entry/rdb"
DATABASES = ("settingsdata.db", "settingsdata_slave.db")
SIDECARS = ("-wal", "-shm", "-dwr")
BUNDLE = "com.richerfu.paws"


def command(*args: str, accepted: tuple[int, ...] = (0,)) -> str:
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode not in accepted:
        raise RuntimeError(
            f"{args[0]} exited {result.returncode}: {(result.stdout + result.stderr)[-1800:]}"
        )
    return result.stdout + result.stderr


def tool(name: str, fallback: str | None = None) -> str:
    found = shutil.which(name)
    if found:
        return found
    if fallback and Path(fallback).is_file():
        return fallback
    raise RuntimeError(f"required tool is missing: {name}")


def assert_stopped(instance: Path, emulator: str, overlay: Path) -> None:
    listing = json.loads(command(emulator, "-list", "-details", "-instancePath", str(instance.parent)))
    matches = [item for item in listing if Path(item["instancePath"]).resolve() == instance.resolve()]
    if len(matches) != 1 or str(matches[0]["isRunning"]).lower() != "false":
        raise RuntimeError("the exact DevEco emulator instance must exist and be stopped")
    holders = command("lsof", str(overlay), accepted=(0, 1)).strip()
    if holders:
        raise RuntimeError(f"the userdata overlay is still open:\n{holders}")


def debugfs_query(debugfs: str, raw: Path, query: str) -> str:
    output = command(debugfs, "-R", query, str(raw))
    if "File not found by ext2_lookup" in output or "File not found" in output:
        raise RuntimeError(f"debugfs could not read {query}: {output[-600:]}")
    return output


def guest_names(debugfs: str, raw: Path) -> set[str]:
    output = debugfs_query(debugfs, raw, f"ls -l {RDB}")
    return {line.split()[-1] for line in output.splitlines() if line.strip().startswith(tuple("0123456789"))}


def guest_metadata(debugfs: str, raw: Path, name: str, work: Path) -> tuple[str, str, str, dict[str, Path]]:
    guest = f"{RDB}/{name}"
    stat = debugfs_query(debugfs, raw, f"stat {guest}")
    mode = re.search(r"Mode:\s+([0-7]+)", stat)
    owner = re.search(r"User:\s+(\d+)\s+Group:\s+(\d+)", stat)
    if not mode or not owner:
        raise RuntimeError(f"could not read mode and owner for {guest}")
    attributes: dict[str, Path] = {}
    for attr in re.findall(r"^\s+([\w.]+) \(\d+\) =", stat, re.MULTILINE):
        destination = work / f"{name}.{attr.replace('.', '_')}"
        debugfs_query(debugfs, raw, f"ea_get -f {destination} {guest} {attr}")
        if not destination.is_file():
            raise RuntimeError(f"could not export {attr} for {guest}")
        attributes[attr] = destination
    return mode.group(1), owner.group(1), owner.group(2), attributes


def rows(path: Path, user_id: int) -> list[tuple[str, str]]:
    connection = sqlite3.connect(f"file:{path}?mode=ro", uri=True)
    try:
        integrity = connection.execute("PRAGMA integrity_check").fetchone()
        if integrity != ("ok",):
            raise RuntimeError(f"SQLite integrity check failed for {path}: {integrity}")
        return connection.execute(
            "SELECT KEYWORD, VALUE FROM SETTINGSDATA WHERE KEYWORD IN (?, ?) ORDER BY KEYWORD",
            (BUNDLE, f"{BUNDLE}_{user_id}"),
        ).fetchall()
    finally:
        connection.close()


def patch_database(path: Path, user_id: int) -> None:
    connection = sqlite3.connect(path)
    try:
        connection.execute("PRAGMA wal_checkpoint(FULL)")
        mode = connection.execute("PRAGMA journal_mode=DELETE").fetchone()
        if mode != ("delete",):
            raise RuntimeError(f"could not checkpoint {path}: journal mode {mode}")
        with connection:
            for key in (BUNDLE, f"{BUNDLE}_{user_id}"):
                connection.execute(
                    "INSERT INTO SETTINGSDATA(KEYWORD, VALUE) VALUES (?, '1') "
                    "ON CONFLICT(KEYWORD) DO UPDATE SET VALUE=excluded.VALUE",
                    (key,),
                )
        if connection.execute("PRAGMA integrity_check").fetchone() != ("ok",):
            raise RuntimeError(f"SQLite integrity check failed after patching {path}")
    finally:
        connection.close()


def verify_rows(path: Path, user_id: int) -> None:
    expected = [(BUNDLE, "1"), (f"{BUNDLE}_{user_id}", "1")]
    actual = sorted(rows(path, user_id))
    if actual != sorted(expected):
        raise RuntimeError(f"authorization records differ in {path}: {actual}")


def provision(args: argparse.Namespace) -> None:
    instance = args.instance_dir.resolve()
    overlay = instance / "userdata.img.qcow2"
    if not overlay.is_file() or args.user_id < 0:
        raise RuntimeError("instance overlay is missing or user ID is invalid")
    emulator = str(args.emulator)
    if not Path(emulator).is_file():
        raise RuntimeError(f"DevEco Emulator executable is missing: {emulator}")
    qemu_img = tool("qemu-img")
    debugfs = tool("debugfs", "/opt/homebrew/opt/e2fsprogs/sbin/debugfs")
    e2fsck = tool("e2fsck", "/opt/homebrew/opt/e2fsprogs/sbin/e2fsck")
    assert_stopped(instance, emulator, overlay)
    info = json.loads(command(qemu_img, "info", "--output=json", str(overlay)))
    if info.get("format") != "qcow2" or info.get("backing-filename-format") != "raw":
        raise RuntimeError("expected a qcow2 instance overlay with a raw userdata backing file")
    backing = Path(info["full-backing-filename"])
    if not backing.is_file():
        raise RuntimeError(f"backing file is missing: {backing}")
    command(qemu_img, "check", str(overlay))
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    backup = instance / f"userdata.img.qcow2.before-paws-vpn-auth-{stamp}"
    if backup.exists():
        raise RuntimeError(f"backup already exists: {backup}")
    try:
        command("cp", "-c", str(overlay), str(backup))
    except RuntimeError:
        shutil.copy2(overlay, backup)
    command(qemu_img, "check", str(backup))
    work_parent = args.work_parent or instance.parent
    if not work_parent.is_dir():
        raise RuntimeError(f"temporary work parent does not exist: {work_parent}")
    work = Path(tempfile.mkdtemp(prefix="paws-vpn-auth-", dir=work_parent.resolve()))
    print(f"Backup: {backup}", flush=True)
    print(f"Temporary workspace: {work}", flush=True)
    try:
        raw = work / "userdata.raw"
        command(qemu_img, "convert", "-f", "qcow2", "-O", "raw", str(overlay), str(raw))
        (work / "fsck-before.log").write_text(command(e2fsck, "-fy", str(raw), accepted=(0, 1)))
        names = guest_names(debugfs, raw)
        extracted = work / "extracted"
        patched = work / "patched"
        verified = work / "verified"
        for directory in (extracted, patched, verified):
            directory.mkdir()
        metadata = {}
        for name in DATABASES:
            if name not in names:
                raise RuntimeError(f"SettingsData database is missing: {name}")
            metadata[name] = guest_metadata(debugfs, raw, name, work)
            for suffix in ("", "-wal"):
                if name + suffix in names:
                    destination = extracted / (name + suffix)
                    debugfs_query(debugfs, raw, f"dump {RDB}/{name + suffix} {destination}")
                    if not destination.is_file():
                        raise RuntimeError(f"could not export {name + suffix}")
            shutil.copy2(extracted / name, patched / name)
            if (extracted / (name + "-wal")).exists():
                shutil.copy2(extracted / (name + "-wal"), patched / (name + "-wal"))
            print(f"{name} before: {rows(extracted / name, args.user_id)}", flush=True)
            patch_database(patched / name, args.user_id)
            verify_rows(patched / name, args.user_id)
        instructions = []
        for name in DATABASES:
            guest = f"{RDB}/{name}"
            for suffix in SIDECARS:
                if name + suffix in names:
                    instructions.append(f"rm {guest + suffix}")
            instructions.extend((
                f"rm {guest}",
                f"write {patched / name} {guest}",
                f"sif {guest} mode 010{metadata[name][0]}",
                f"sif {guest} uid {metadata[name][1]}",
                f"sif {guest} gid {metadata[name][2]}",
            ))
            for attr, value_file in metadata[name][3].items():
                instructions.append(f"ea_set -f {value_file} {guest} {attr}")
        instruction_file = work / "patch.debugfs"
        instruction_file.write_text("\n".join(instructions) + "\n")
        patch_log = command(debugfs, "-w", "-f", str(instruction_file), str(raw))
        (work / "patch.log").write_text(patch_log)
        if re.search(r"File not found|Could not|Operation not permitted|Invalid argument", patch_log, re.I):
            raise RuntimeError(f"debugfs patch reported an error; inspect {work / 'patch.log'}")
        (work / "fsck-after.log").write_text(command(e2fsck, "-fy", str(raw), accepted=(0, 1)))
        command(e2fsck, "-fn", str(raw))
        for name in DATABASES:
            destination = verified / name
            debugfs_query(debugfs, raw, f"dump {RDB}/{name} {destination}")
            verify_rows(destination, args.user_id)
            for suffix in SIDECARS:
                if name + suffix in guest_names(debugfs, raw):
                    raise RuntimeError(f"stale SettingsData sidecar remains: {name + suffix}")
        replacement = work / "userdata.patched.qcow2"
        command(qemu_img, "convert", "-f", "raw", "-O", "qcow2", "-B", str(backing),
                "-F", "raw", str(raw), str(replacement))
        command(qemu_img, "check", str(replacement))
        assert_stopped(instance, emulator, overlay)
        os.replace(replacement, overlay)
        command(qemu_img, "check", str(overlay))
        print("Paws VPN authorization is provisioned in both SettingsData databases.")
    except Exception:
        print(f"Provisioning stopped; backup and workspace are retained: {backup}, {work}", file=sys.stderr)
        raise
    else:
        shutil.rmtree(work)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--instance-dir", required=True, type=Path)
    parser.add_argument("--user-id", required=True, type=int)
    parser.add_argument("--work-parent", type=Path)
    parser.add_argument("--emulator", type=Path,
                        default=Path("/Applications/DevEco-Studio.app/Contents/tools/emulator/Emulator"))
    args = parser.parse_args()
    try:
        provision(args)
    except (OSError, RuntimeError, sqlite3.Error, subprocess.SubprocessError) as error:
        parser.exit(1, f"Provisioning failed: {error}\n")


if __name__ == "__main__":
    main()
