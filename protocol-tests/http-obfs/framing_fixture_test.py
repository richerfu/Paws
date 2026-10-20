#!/usr/bin/env python3
"""Compare actual local Meow/Mihomo/Sing HTTP-obfs writes and authenticated frames."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    with tempfile.TemporaryDirectory(prefix="paws-local-framing-") as temporary:
        work = Path(temporary)
        environment = dict(os.environ)
        for name in ("TINYPAWS_LIVE_NODE_PATH", "TINYPAWS_MEOW_WIRE_FIXTURE", "TINYPAWS_HELLO_FIXTURE_OUT"):
            environment.pop(name, None)
        command = [sys.executable, str(ROOT / "scripts/test-http-obfs-compat.py"),
                   "--run", "^Test(FirstRequest|TLSFlightFraming)$", "--timeout", "180"]
        subprocess.run(command, env={**environment, "TINYPAWS_HELLO_FIXTURE_OUT": str(work / "hello.bin")},
                       check=True, capture_output=True, timeout=180)
        source = ROOT / "protocol-tests/http-obfs/meow_wire_test.rs"
        manifest = ('[package]\nname="paws_meow_wire_fixture"\nversion="0.1.0"\nedition="2021"\n'
                    '[[bin]]\nname="meow-wire-fixture"\npath=' + json.dumps(str(source)) + '\n'
                    '[dependencies]\nshadowsocks={version="=1.24.0",default-features=false,features=["aead-cipher"]}\n'
                    'tokio={version="1",features=["full"]}\nserde_json="1"\n'
                    'meow-transport={path=' + json.dumps(str(ROOT / 'vendor/meow-transport')) +
                    ',default-features=false,features=["simple-obfs"]}\n')
        (work / "Cargo.toml").write_text(manifest)
        with (work / "meow.json").open("w") as capture:
            subprocess.run(["cargo", "run", "--offline", "--quiet", "--manifest-path", str(work / "Cargo.toml"),
                            "--target-dir", str(work / "target"), "--", str(work / "hello.bin")],
                           env=environment, stdout=capture, stderr=subprocess.PIPE, check=True, timeout=180)
        report_path = work / "report.json"
        subprocess.run([*command, "--output", str(report_path)],
                       env={**environment, "TINYPAWS_MEOW_WIRE_FIXTURE": str(work / "meow.json")},
                       check=True, capture_output=True, timeout=180)
        report = json.loads(report_path.read_text())
        report["meow"] = {"shadowsocksVersion": "1.24.0",
                          "httpObfsSha256": hashlib.sha256((ROOT / "vendor/meow-transport/src/simple_obfs/client.rs").read_bytes()).hexdigest()}
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps({"passed": report["passed"], "report": str(args.output),
                          "scope": "local writes and independent AEAD decoding; no live node"}))


if __name__ == "__main__":
    main()
