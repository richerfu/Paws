#!/usr/bin/env python3
"""Test actual OHOS editor methods and generated NAPI serializers from an NSH stage.

Production files are read-only. The host harness substitutes SDK ABI effects;
its timings are not measurements of the device bridge or native text renderer.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path, help='Prepared NSH source root to inspect')
    parser.add_argument('--core-source', type=Path, help='nsh-core crate path, only for a partial isolated draft')
    parser.add_argument('--target-dir', type=Path, default=ROOT / 'target/nsh-core-intrinsic-tests')
    parser.add_argument('--work-dir', type=Path, default=ROOT / 'target/nsh-editor-update-domains')
    args = parser.parse_args()
    source = args.source.resolve(strict=True)
    core = (args.core_source or source / 'crates/nsh-core').resolve(strict=True)
    templates = ROOT / 'scripts/tests/nsh-editor-update-domains'
    inputs = ['Cargo.toml', 'build.rs', 'src/lib.rs', 'src/parity_tests.rs']
    digest = hashlib.sha256(str(source).encode())
    for relative in inputs:
        digest.update(relative.encode() + (templates / relative).read_bytes())
    work = args.work_dir.resolve() / digest.hexdigest()[:16]
    harness = work / 'harness'
    for relative in inputs:
        target = harness / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(templates / relative, target)
    # Workspace dependencies stay centralized; the selected Core is never copied or modified.
    (work / 'Cargo.toml').write_text('''[workspace]
resolver = "3"
members = ["harness"]
[workspace.package]
edition = "2024"
[workspace.dependencies]
nsh-core = { path = ''' + json.dumps(str(core)) + ''' }
napi-derive-backend-ohos = "=1.2.0"
quote = "1"
proc-macro2 = "1"
syn = { version = "2", features = ["full", "visit"] }
serde_json = "1"
log = "0.4"
''')
    environment = dict(os.environ, NSH_EDITOR_TEST_SOURCE=str(source), CARGO_INCREMENTAL='0',
                       CARGO_PROFILE_DEV_DEBUG='0', CARGO_PROFILE_TEST_DEBUG='0',
                       CARGO_TARGET_DIR=str(args.target_dir.resolve()))
    commands = [
        ('rust', ['cargo', 'test', '--offline', '--manifest-path', str(work / 'Cargo.toml'), '--', '--nocapture']),
        ('arkts', ['node', '--test', str(source / 'crates/nsh-platform-ohos/arkts/tests/editors.test.mjs')]),
    ]
    failed = False
    print('Read-only NSH source:', source, flush=True)
    print('Host SDK ABI recorder; no device bridge timing claims.', flush=True)
    for name, command in commands:
        log = work / (name + '.log')
        try:
            result = subprocess.run(command, cwd=ROOT, env=environment, text=True,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=300)
            log.write_text(result.stdout)
            print(name + ': exit ' + str(result.returncode) + ', log ' + str(log), flush=True)
            print('\n'.join(result.stdout.splitlines()[-12:]), flush=True)
            failed = failed or result.returncode != 0
        except subprocess.TimeoutExpired as error:
            output = error.stdout or b''
            log.write_text(output.decode() if isinstance(output, bytes) else output)
            print(name + ': timeout; log ' + str(log), flush=True)
            failed = True
    raise SystemExit(1 if failed else 0)


if __name__ == '__main__':
    main()
