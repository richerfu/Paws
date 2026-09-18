#!/usr/bin/env python3
"""Sign a debug-only test HAP using the SDK's public development material.

No user signing configuration is read or changed. Generated target-specific
profiles remain in a private temporary directory outside the repository.
"""

import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import time
import uuid
import zipfile


def run(command, label, work):
    result = subprocess.run(command, capture_output=True, text=True)
    # Diagnostics are local only; never print commands containing password/UDID.
    (work / f"{label}.log").write_text(result.stdout + result.stderr, encoding="utf-8")
    if result.returncode:
        raise RuntimeError(f"{label} failed (exit {result.returncode}); diagnostics: {work}")
    return result.stdout


def sign(args):
    if args.output.exists():
        raise ValueError("output already exists; use a new test artifact path")
    with zipfile.ZipFile(args.input) as hap:
        manifest = json.loads(hap.read("module.json"))
        if manifest["app"].get("debug") is not True:
            raise ValueError("only a debug HAP may use public development signing")
        bundle = manifest["app"]["bundleName"]
        if bundle != "com.richerfu.paws":
            raise ValueError(f"expected the Paws debug bundle, got {bundle}")
        compatible = json.loads(hap.read("pack.info"))["summary"]["modules"][0]["apiVersion"]["compatible"]
    password = os.environ.get("PAWS_TEST_KEYSTORE_PASSWORD")
    if not password:
        raise ValueError("set PAWS_TEST_KEYSTORE_PASSWORD for the SDK public test keystore")
    sdk = args.sdk_lib.resolve()
    keystore = sdk / "OpenHarmony.p12"
    work = Path(tempfile.mkdtemp(prefix="paws-test-sign-"))
    os.chmod(work, 0o700)
    profile = json.loads((sdk / "UnsgnedDebugProfileTemplate.json").read_text())
    profile["uuid"] = str(uuid.uuid4())
    profile["validity"] = {"not-before": int(time.time()) - 86400,
                           "not-after": int(time.time()) + 86400 * 30}
    profile["bundle-info"]["bundle-name"] = bundle
    profile["acls"]["allowed-acls"] = []
    profile["permissions"]["restricted-permissions"] = []
    device_ids = []
    for index, target in enumerate(args.target):
        output = run([args.hdc, "-s", args.hdc_server, "-t", target,
                      "shell", "bm", "get", "-u"], f"target-{index}", work)
        values = re.findall(r"(?<![0-9A-Fa-f])[0-9A-Fa-f]{64,128}(?![0-9A-Fa-f])", output)
        if len(values) != 1:
            raise ValueError(f"could not obtain exactly one UDID for target {target}")
        device_ids.append(values[0])
    profile["debug-info"]["device-ids"] = device_ids
    profile_path = work / "profile.json"
    profile_path.write_text(json.dumps(profile), encoding="utf-8")

    # The keystore's app-release entry is self-signed. The template carries the
    # matching CA-signed leaf; concatenating the self-signed leaf fails code signing.
    chain = profile["bundle-info"]["development-certificate"] + "\n"
    for index, alias in enumerate(("openharmony application ca", "openharmony application root ca")):
        chain += run([args.keytool, "-exportcert", "-rfc", "-alias", alias,
                      "-keystore", str(keystore), "-storepass", password], f"ca-{index}", work) + "\n"
    app_chain = work / "app-chain.cer"
    app_chain.write_text(chain, encoding="utf-8")
    base = [args.java, "-jar", str(sdk / "hap-sign-tool.jar")]
    common = ["-mode", "localSign", "-keyPwd", password, "-signAlg", "SHA256withECDSA",
              "-keystoreFile", str(keystore), "-keystorePwd", password]
    signed_profile = work / "profile.p7b"
    run(base + ["sign-profile", "-keyAlias", "openharmony application profile debug",
                "-profileCertFile", str(sdk / "OpenHarmonyProfileDebug.pem"),
                "-inFile", str(profile_path), "-outFile", str(signed_profile)] + common,
        "sign-profile", work)
    run(base + ["sign-app", "-keyAlias", "openharmony application release",
                "-appCertFile", str(app_chain), "-profileFile", str(signed_profile),
                "-inFile", str(args.input.resolve()), "-outFile", str(args.output.resolve()),
                "-compatibleVersion", str(compatible), "-signCode", args.sign_code, "-inForm", "zip"] + common,
        "sign-app", work)
    if not args.output.is_file():
        raise RuntimeError("signer returned without producing the HAP")
    print(f"Signed debug HAP for {bundle} on {len(device_ids)} explicit targets: {args.output}")


def main():
    deveco = Path(os.environ.get("DEVECO_STUDIO_HOME", "/Applications/DevEco-Studio.app/Contents"))
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("input", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--sdk-lib", type=Path,
                        default=deveco / "sdk/default/openharmony/toolchains/lib")
    parser.add_argument("--target", required=True, action="append")
    parser.add_argument("--sign-code", choices=("0", "1"), default="1")
    parser.add_argument("--hdc", default="hdc")
    parser.add_argument("--hdc-server", default="127.0.0.1:8710")
    parser.add_argument("--java", default=str(deveco / "jbr/Contents/Home/bin/java"))
    parser.add_argument("--keytool", default=str(deveco / "jbr/Contents/Home/bin/keytool"))
    args = parser.parse_args()
    try:
        sign(args)
    except (OSError, ValueError, RuntimeError, KeyError, zipfile.BadZipFile) as error:
        parser.exit(1, f"Test signing failed: {error}\n")


if __name__ == "__main__":
    main()
