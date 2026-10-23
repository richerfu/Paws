#!/usr/bin/env python3
"""Hash the explicit native build inputs, excluding generated crate outputs."""
import hashlib
import os
from pathlib import Path
import sys

root = Path(sys.argv[1])
digest = hashlib.sha256()
files = [Path(p) for p in sys.argv[2:]] + [root / 'Cargo.lock', root / 'Cargo.toml',
    root / 'scripts/build-sing-box-ohos.sh', root / 'scripts/sing-box-source-inventory.py',
    root / 'scripts/build-mihomo-ohos.sh', root / 'scripts/mihomo-source-inventory.py',
    root / 'scripts/prepare-go-ohos.sh', root / 'scripts/ohos-env.sh',
    root / 'entry/libs/arm64-v8a/libnsh_platform_ohos.so',
    root / 'entry/src/main/resources/rawfile/paws.bundle.js', root / 'target/ohos/nsh-source-inputs.json']
for key in ['PAWS_RAW_ENGINE_DIAGNOSTIC', 'PAWS_BENCH_PEER_IP', 'PAWS_BENCH_NODE_FINGERPRINT', 'PAWS_SOCKET_WIRE_CAPTURE']:
    digest.update(key.encode() + b'\0' + os.environ.get(key, '').encode() + b'\0')
for directory in ['vendor/meow-listener', 'vendor/meow-common', 'vendor/meow-proxy', 'vendor/meow-anytls', 'vendor/meow-dns',
                  'vendor/meow-config', 'vendor/meow-rules', 'vendor/meow-tunnel', 'vendor/meow-transport', 'vendor/mihomo-bridge', 'vendor/sing-box-bridge', 'vendor/paws-engine-observation',
                  'patches', 'ui/nsh/src']:
    files.extend(p for p in (root / directory).rglob('*') if p.is_file()
                 and p.suffix in ['.rs', '.go', '.c', '.cpp', '.h', '.toml', '.patch', '.ts', '.tsx'])
for crate in sorted((root / 'crates').iterdir()):
    if not crate.is_dir():
        continue
    files.extend(p for p in (crate / 'src').rglob('*.rs') if p.is_file())
    files.extend(p for p in [crate / 'Cargo.toml', crate / 'build.rs'] if p.is_file())
for path in sorted(files):
    digest.update(path.name.encode() + b'\0')
    digest.update(path.read_bytes())
print(digest.hexdigest())
