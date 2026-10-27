#!/usr/bin/env python3
"""Stage matching Ability sources inside Paws without changing sibling checkouts."""
import os
import hashlib
import json
import subprocess
from pathlib import Path
import shutil
from nsh_build_inputs import resolve_inputs

ROOT = Path(__file__).resolve().parents[1]
inputs = resolve_inputs()
source = Path(inputs['abilityRoot'])
destination = ROOT / 'target/ohos/arkts-deps'
packages = {'ability': source / 'native_ability',
            'nsh': Path(inputs['nshRoot']) / 'crates/nsh-platform-ohos/arkts'}
packages.update({name: source / 'plugins' / name for name in (
    'resource', 'clipboard', 'window', 'url', 'files', 'accessibility', 'webview')})
if destination.exists():
    for stale in destination.iterdir():
        if stale.name not in packages:
            shutil.rmtree(stale) if stale.is_dir() else stale.unlink()
har_inputs = []
for name, path in packages.items():
    if not (path / 'oh-package.json5').is_file():
        raise SystemExit(f'Matching Ability package is unavailable: {path}')
    target = destination / name
    if target.exists():
        shutil.rmtree(target)
    shutil.copytree(path, target, ignore=shutil.ignore_patterns(
        'build', '.hvigor', '.cxx', 'oh_modules', 'oh-package-lock.json5', 'BuildProfile.ets'))
    manifest = target / 'oh-package.json5'
    manifest.write_text(
        manifest.read_text()
        .replace('"@ohos-rs/ability": "workspace:*"', '"@ohos-rs/ability": "file:../ability"')
        .replace(
            '"libnsh_platform_ohos.so": "file:./src/main/cpp/types/libnsh_platform_ohos"',
            '"libnsh_platform_ohos.so": "file:../../../../entry/src/main/cpp/types/libnsh_platform_ohos"',
        )
    )
    if name in ('ability', 'nsh'):
        original_hash, staged_hash = hashlib.sha256(), hashlib.sha256()
        files = {}
        for staged in sorted(item for item in target.rglob('*') if item.is_file()):
            relative = staged.relative_to(target).as_posix()
            original, compiled = (path / relative).read_bytes(), staged.read_bytes()
            original_hash.update(relative.encode() + b'\0' + original + b'\0')
            staged_hash.update(relative.encode() + b'\0' + compiled + b'\0')
            files[relative] = {'sourceSha256': hashlib.sha256(original).hexdigest(),
                               'stagedSha256': hashlib.sha256(compiled).hexdigest()}
        # Reuse Hvigor's JSON5 parser, including comments and trailing commas.
        studio = Path(os.environ['DEVECO_STUDIO_HOME'])
        parser = studio / 'tools/hvigor/hvigor-ohos-plugin/node_modules/json5'
        package = json.loads(subprocess.check_output([
            str(studio / 'tools/node/bin/node'), '-e',
            'const fs=require("fs"),j=require(process.argv[1]);process.stdout.write(JSON.stringify(j.parse(fs.readFileSync(process.argv[2],"utf8"))))',
            str(parser), str(manifest)], text=True, timeout=30))
        har_inputs.append({'name': package['name'], 'version': package['version'],
            'sourceRoot': str(path), 'stagedRoot': str(target.relative_to(ROOT)),
            'sourceSha256': original_hash.hexdigest(), 'stagedSha256': staged_hash.hexdigest(),
            'fileCount': len(files), 'files': files})
sidecar = {'format': 'Paws-matching-ArkTS-sources-v1', 'nsh': inputs['nsh'],
           'ability': inputs['ability'], 'abilityDependencySource': inputs.get('abilityDependencySource'),
           'packages': har_inputs}
(ROOT / 'target/ohos/arkts-source-inputs.json').write_text(json.dumps(sidecar, indent=2) + '\n')
print(destination)
