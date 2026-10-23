#!/usr/bin/env python3
"""Declared source identity for the exact-pin Mihomo native build."""
import hashlib
import json
from pathlib import Path
import sys

root = Path(sys.argv[1]).resolve()
inputs = [root / name for name in [
    'scripts/build-mihomo-ohos.sh', 'scripts/mihomo-source-inventory.py',
    'scripts/prepare-go-ohos.sh', 'scripts/ohos-env.sh',
    'patches/go-ohos-1.24.7-compat.patch', 'patches/gvisor-fstat.patch',
    'patches/mihomo-openharmony.patch',
    'patches/mihomo-ohos-tun-fd-ownership.patch',
    'patches/mihomo-paws-management.patch',
]]
for directory in ['vendor/mihomo-bridge', 'vendor/paws-engine-observation']:
    inputs.extend(path for path in (root / directory).rglob('*')
                  if path.is_file() and (path.suffix in {'.go', '.c', '.h'}
                                         or path.name in {'go.mod', 'go.sum'}))
print(json.dumps({
    'pin': '8597778c7df410ebe88c57e59eaab32d203156df',
    'sources': {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in sorted(set(inputs))},
}, sort_keys=True, indent=2))
