#!/usr/bin/env python3
"""Stage the local NSH sources with the small, pinned dialog-mask binding fix.

The user's NSH checkout and Cargo's immutable Git cache remain untouched.
The resulting workspace, including its patched binding, is covered by the
normal NSH source fingerprint and is reused only while that identity matches.
"""
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tomllib

from nsh_build_inputs import ROOT, ability_metadata, resolve_inputs


class BuildSource:
    @staticmethod
    def binding_preparation(revision):
        # 43aec6c keeps the same native ownership APIs, but removed the Dialog
        # operation guard and its host fixtures. Restore the exact reviewed
        # guard/fixtures before applying the existing lifecycle/viewport fixes.
        recipes = {
            '6922af6454bc5f2789befd356c6527aaffb75a70': (),
            'e9a376c4313d76eeb41b148d05b682576b57a2e0': (),
            '43aec6c125503a47f7e875c793aaf2462e91ad19': (
                'arkui-dialog-operation.patch', 'arkui-host-fixtures.patch'),
        }
        if revision not in recipes:
            raise ValueError('Rebase the dialog-mask patch for the newly selected binding revision')
        return recipes[revision]

    def __init__(self):
        self.inputs = resolve_inputs()
        self.source = Path(self.inputs['nshRoot'])
        self.patches = ROOT / 'patches/nsh'
        recipe = hashlib.sha256(Path(__file__).read_bytes())
        recipe.update((ROOT / 'scripts/nsh_build_inputs.py').read_bytes())
        for path in sorted(self.patches.glob('*.patch')):
            recipe.update(path.name.encode() + path.read_bytes())
        self.recipe = recipe.hexdigest()
        # package-hap exports this exact root to its build subprocess. Passing
        # an already prepared root must verify it, never copy/patch it again.
        if self.source.parent == ROOT / 'target/nsh-build-source':
            self.stage = self.source
            self.record = self.stage.with_suffix('.json')
            if not self.record.is_file():
                raise ValueError('Incomplete NSH stage; select the original NSH_ROOT')
            expected = json.loads(self.record.read_text())
            if expected.get('recipeSha256') != self.recipe:
                raise ValueError('NSH patch recipe changed; select the original NSH_ROOT')
            if expected['patchedInputs'] != self.inputs:
                raise ValueError('Staged NSH sources changed; select the original NSH_ROOT')
            return
        digest = hashlib.sha256(json.dumps(self.inputs, sort_keys=True).encode())
        digest.update(self.recipe.encode())
        self.stage = ROOT / 'target/nsh-build-source' / digest.hexdigest()[:24]
        self.record = self.stage.with_suffix('.json')

    def identity(self, path):
        previous = os.environ.get('NSH_ROOT')
        os.environ['NSH_ROOT'] = str(path)
        try:
            return resolve_inputs()
        finally:
            if previous is None:
                os.environ.pop('NSH_ROOT', None)
            else:
                os.environ['NSH_ROOT'] = previous

    @staticmethod
    def toml(data):
        """Emit the manifest's scalar/list/table subset without extra packages."""
        lines = []

        def table(value, keys):
            if keys:
                lines.append('[' + '.'.join(json.dumps(key) for key in keys) + ']')
            for key, item in value.items():
                if not isinstance(item, dict):
                    lines.append(json.dumps(key) + ' = ' + json.dumps(item))
            lines.append('')
            for key, item in value.items():
                if isinstance(item, dict):
                    table(item, [*keys, key])

        table(data, [])
        return '\n'.join(lines)

    def binding(self):
        package = [p for p in ability_metadata(self.source)['packages']
                   if p['name'] == 'ohos-arkui-binding']
        if len(package) != 1:
            raise ValueError('Expected exactly one locked ArkUI binding')
        original = Path(package[0]['manifest_path']).resolve().parent
        workspace = tomllib.loads((original.parents[1] / 'Cargo.toml').read_text())['workspace']
        dependency = tomllib.loads((self.source / 'Cargo.toml').read_text())['workspace']['dependencies']['ohos-arkui-binding']
        preparation = self.binding_preparation(dependency.get('rev'))
        if package[0].get('source', '').rsplit('#', 1)[-1] != dependency.get('rev'):
            raise ValueError('The locked ArkUI source must match its reviewed binding revision')
        target = self.stage / 'paws-patches/ohos-arkui-binding'
        shutil.copytree(original, target)
        manifest = tomllib.loads((target / 'Cargo.toml').read_text())
        manifest['lints'] = copy.deepcopy(workspace['lints'])
        for name, value in manifest['package'].items():
            if isinstance(value, dict) and value.get('workspace'):
                manifest['package'][name] = copy.deepcopy(workspace['package'][name])

        def dependencies(table):
            for key, value in table.items():
                if not isinstance(value, dict):
                    continue
                if key not in ('dependencies', 'build-dependencies', 'dev-dependencies'):
                    dependencies(value)
                    continue
                for name, options in value.items():
                    if not isinstance(options, dict) or not options.get('workspace'):
                        continue
                    options = {k: v for k, v in options.items() if k != 'workspace'}
                    inherited = copy.deepcopy(workspace['dependencies'][name])
                    if isinstance(inherited, str):
                        inherited = {'version': inherited}
                    if 'path' in inherited:
                        inherited.pop('path')
                        inherited.update(git=dependency['git'], rev=dependency['rev'])
                    features = list(dict.fromkeys([*inherited.get('features', []), *options.get('features', [])]))
                    inherited.update(options)
                    if features:
                        inherited['features'] = features
                    value[name] = inherited

        dependencies(manifest)
        # This is a standalone patched crate, not an extra NSH workspace member.
        manifest['workspace'] = {}
        (target / 'Cargo.toml').write_text(self.toml(manifest))
        for filename in preparation:
            self.apply(target, filename)
        self.apply(target, 'arkui-dialog-mask.patch')
        cargo = self.stage / 'Cargo.toml'
        existing = tomllib.loads(cargo.read_text()).get('patch', {}).get(dependency['git'], {})
        if 'ohos-arkui-binding' in existing:
            raise ValueError('NSH already patches ArkUI; rebase the Paws dialog patch explicitly')
        with cargo.open('a') as output:
            output.write('\n[patch.' + json.dumps(dependency['git']) + '.ohos-arkui-binding]\n')
            output.write('path = "paws-patches/ohos-arkui-binding"\n')

    def apply(self, root, filename):
        subprocess.run(['patch', '--batch', '--fuzz=0', '-p1', '-i', str(self.patches / filename)],
                       cwd=root, check=True, stdout=subprocess.DEVNULL)

    def prepare(self):
        if self.record.exists():
            expected = json.loads(self.record.read_text())
            if expected.get('recipeSha256') != self.recipe or self.identity(self.stage) != expected['patchedInputs']:
                raise ValueError('Staged NSH sources changed; remove this generated stage and rebuild')
            return self.stage
        if self.stage.exists():
            # Only an incomplete, content-addressed build directory is removed.
            shutil.rmtree(self.stage)
        excluded = {'target', 'dist', 'build', 'node_modules', 'oh_modules', '.hvigor',
                    '.cxx', '.nsh', '__pycache__', '.DS_Store'}
        if not (self.source / '.git').is_dir() or (self.source / '.git').is_symlink():
            raise ValueError('NSH source needs an independent .git directory for read-only staging')

        def ignore(directory, names):
            skipped = set(names) & excluded
            for name in names:
                path = Path(directory) / name
                if stat.S_ISSOCK(path.lstat().st_mode) or ('.git' in path.parts and name.endswith('.lock')):
                    skipped.add(name)
            return skipped

        shutil.copytree(self.source, self.stage,
                        ignore=ignore, symlinks=True)
        for key, value in [('core.worktree', str(self.stage)), ('core.fsmonitor', 'false'),
                           ('core.hooksPath', '/dev/null'), ('commit.gpgsign', 'false')]:
            subprocess.run(['git', '-C', str(self.stage), 'config', '--local', key, value],
                           check=True, timeout=30)
        copied = self.identity(self.stage)
        if copied['nsh'] != self.inputs['nsh'] or self.identity(self.source) != self.inputs:
            raise ValueError('Local NSH changed during staging; retry with a stable source tree')
        self.binding()
        for patch in sorted(self.patches.glob('*.patch')):
            if patch.name not in {'arkui-dialog-mask.patch', 'arkui-dialog-operation.patch',
                                   'arkui-host-fixtures.patch'}:
                self.apply(self.stage, patch.name)
        result = subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1',
                                 '--filter-platform', 'aarch64-unknown-linux-ohos'],
                                cwd=self.stage, capture_output=True, text=True, timeout=180)
        if result.returncode:
            raise ValueError(result.stderr[-4000:])
        packages = json.loads(result.stdout)['packages']
        arkui = [p for p in packages if p['name'] == 'ohos-arkui-binding']
        if len(arkui) != 1 or arkui[0]['source'] is not None:
            raise ValueError('The patched graph must contain one ArkUI binding')
        self.record.write_text(json.dumps({'originalInputs': self.inputs, 'recipeSha256': self.recipe,
                                          'patchedInputs': self.identity(self.stage)}, indent=2) + '\n')
        return self.stage


if __name__ == '__main__':
    print(BuildSource().prepare())
