#!/usr/bin/env python3
"""Declared source identity for the exact-pin Mihomo native build."""
import hashlib
import json
from pathlib import Path
import sys

from go_toolchain import compiler_identity

root = Path(sys.argv[1]).resolve()
inputs = [root / name for name in [
    'scripts/build-mihomo-ohos.sh', 'scripts/mihomo-source-inventory.py',
    'scripts/prepare-go-ohos.sh', 'scripts/ohos-env.sh',
    'scripts/go_toolchain.py', 'scripts/go-ohos-toolchain.json',
    'patches/mihomo-openharmony.patch',
    'patches/mihomo-paws-management.patch',
]]
for directory in ['vendor/mihomo-bridge', 'vendor/paws-engine-observation']:
    inputs.extend(path for path in (root / directory).rglob('*')
                  if path.is_file() and (path.suffix in {'.go', '.c', '.h'}
                                         or path.name in {'go.mod', 'go.sum'}))
print(json.dumps({
    'pin': '8597778c7df410ebe88c57e59eaab32d203156df',
    'compiler': compiler_identity(),
    'sources': {str(path.relative_to(root)): hashlib.sha256(path.read_bytes()).hexdigest()
                for path in sorted(set(inputs))},
}, sort_keys=True, indent=2))
