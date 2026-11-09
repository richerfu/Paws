#!/usr/bin/env python3
"""Declared source identity for the exact-pin sing-box native build."""
import hashlib
import json
from pathlib import Path
import sys

from go_toolchain import compiler_identity

root = Path(sys.argv[1]).resolve()
inputs = [root / name for name in [
    'scripts/build-sing-box-ohos.sh', 'scripts/sing-box-source-inventory.py',
    'scripts/prepare-go-ohos.sh', 'scripts/ohos-env.sh',
    'scripts/go_toolchain.py', 'scripts/go-ohos-toolchain.json',
    'patches/sing-box-http-obfs-handshake.patch',
    'patches/sing-box-http-obfs-first-flight.patch',
    'patches/sing-box-benchmark-probe.patch',
    'patches/sing-box-paws-management.patch',
]]
for directory in ['vendor/sing-box-bridge', 'vendor/paws-engine-observation']:
    inputs.extend(path for path in (root / directory).rglob('*')
                  if path.is_file() and (path.suffix in {'.go', '.c', '.h'}
                                         or path.name in {'go.mod', 'go.sum'}))
print(json.dumps({
    'pin': '628cb31ffa79cffffd34c2f9cde6cae044e4fc12',
    'compiler': compiler_identity(),
    'sources': {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in sorted(set(inputs))},
}, sort_keys=True, indent=2))
