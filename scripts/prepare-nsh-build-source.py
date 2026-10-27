#!/usr/bin/env python3
"""Stage the local NSH sources without modifying the SDK checkout.

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
    UPSTREAM_ABILITY = 'b65502500b820a570496f3798c931d12d3b71d39'
    UPSTREAM_BINDING = '43aec6c125503a47f7e875c793aaf2462e91ad19'
    UPSTREAM_SDK = '4f5bd5542fe94396a2e26bd3feb282235b356bf0ce0e1527a17298cef58cf112'
    UPSTREAM_WORKTREE = '746ee6d3eff519df0b1bc8d4a2ee80f69dc8deec4c4fa745d74ffc4fd419706a'
    BINDING_STAGED = ('arkui-custom-event-receiver.patch', 'arkui-dialog-lifecycle.patch',
                      'arkui-editor-viewport.patch', 'modal-parent-layout.patch',
                      'modal-presentation-order.patch')
    BINDING_DIRECT = ('arkui-dialog-operation.patch', 'arkui-host-fixtures.patch', 'arkui-dialog-mask.patch')
    UPSTREAM_OMITTED = {
        'editor-host-timing.patch': 'Legacy editor instrumentation; not carried into the revised upstream editor.',
        'editor-phase-tracing.patch': 'Legacy editor instrumentation; not carried into the revised upstream editor.',
        'editor-queued-event-delivery.patch': 'Legacy editor callback patch; current upstream ownership/callback implementation is used unchanged.',
        'editor-viewport-measure.patch': 'Legacy editor viewport patch; current upstream standard editor is used unchanged.',
        'fixed-input-viewport.patch': 'Legacy Core viewport patch for the Paws document backend, which is not ported by this upgrade.',
        'native-editor-line-height.patch': 'Legacy editor styling overlay; current upstream standard editor is used unchanged.',
        'native-editor-update-domains.patch': 'Legacy editor state/update overlay; incompatible old contexts are not replayed.',
        'native-editor-value-caret.patch': 'Legacy editor value/caret overlay; incompatible old contexts are not replayed.',
        'transparent-modal-mask.patch': 'Upstream modal.rs in 26e18e01 already sets modal_mode(!full) and background_color(0) for full-screen dialogs.',
        'zz-document-editor.patch': 'Paws-specific document/secure architecture is absent upstream and is intentionally not ported in this upgrade.',
        'zzz-text-raster-budget-diagnostics.patch': 'Legacy 4MP/12-byte raster diagnostics do not describe the revised upstream raster model.',
        'zzzz-text-raster-lifetimes.patch': 'Legacy Core/Engine text lifetime overlay is not assumed equivalent to revised upstream behaviour.',
        'zzzzz-text-raster-budget-wait.patch': 'Paws-specific 100ms raster wait is not present upstream and is intentionally not ported in this upgrade.',
    }

    @classmethod
    def patch_plan(cls, inputs, names, binding_revision):
        names = sorted(names)
        if inputs.get('hostModel') == 'native-ui-context':
            return {'name': 'local-native-ui-context', 'stagedPatches': [],
                    'omittedPatches': {name: 'The current local SDK owns native input and dialogs; legacy Ability overlays do not apply.'
                                       for name in names}}
        if inputs['ability']['revision'] != cls.UPSTREAM_ABILITY:
            return {'name': 'legacy-paws-editor',
                    'stagedPatches': [name for name in names if name not in cls.BINDING_DIRECT],
                    'omittedPatches': {}}
        if (inputs['nsh'].get('sdkSourceSha256') != cls.UPSTREAM_SDK
                or inputs['nsh']['worktreeSha256'] != cls.UPSTREAM_WORKTREE
                or binding_revision != cls.UPSTREAM_BINDING):
            raise ValueError('Rebase the reviewed upstream-native recipe for the changed NSH/Ability/binding source')
        expected = {*cls.BINDING_DIRECT, *cls.BINDING_STAGED, *cls.UPSTREAM_OMITTED}
        if set(names) != expected or len(names) != len(expected):
            raise ValueError('Review the explicit upstream-native patch inventory before staging')
        return {'name': 'upstream-native-ability-b6550250',
                'bindingPatches': [*cls.BINDING_DIRECT, *cls.BINDING_STAGED],
                'stagedPatches': list(cls.BINDING_STAGED),
                'omittedPatches': dict(cls.UPSTREAM_OMITTED)}

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
        manifest = tomllib.loads((self.source / 'Cargo.toml').read_text())
        binding_revision = manifest['workspace']['dependencies'].get('ohos-arkui-binding', {}).get('rev')
        patch_names = [path.name for path in self.patches.glob('*.patch')]
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
            self.plan = self.patch_plan(expected['originalInputs'], patch_names, binding_revision)
            if expected.get('recipeSha256') != self.recipe:
                raise ValueError('NSH patch recipe changed; select the original NSH_ROOT')
            if expected['patchedInputs'] != self.inputs:
                raise ValueError('Staged NSH sources changed; select the original NSH_ROOT')
            if expected.get('patchPlan') != self.plan:
                raise ValueError('NSH patch plan changed; select the original NSH_ROOT')
            return
        self.plan = self.patch_plan(self.inputs, patch_names, binding_revision)
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
            if (expected.get('recipeSha256') != self.recipe
                    or expected.get('patchPlan') != self.plan
                    or self.identity(self.stage) != expected['patchedInputs']):
                raise ValueError('Staged NSH sources changed; remove this generated stage and rebuild')
            return self.stage
        if self.stage.exists():
            # Only an incomplete, content-addressed build directory is removed.
            shutil.rmtree(self.stage)
        excluded = {'target', 'dist', 'build', 'node_modules', 'oh_modules', '.hvigor',
                    '.cxx', '.nsh', '.DS_Store'}
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
        if self.plan['name'] != 'local-native-ui-context':
            self.binding()
        for filename in self.plan['stagedPatches']:
            self.apply(self.stage, filename)
        result = subprocess.run(['cargo', 'metadata', '--offline', '--format-version', '1',
                                 '--filter-platform', 'aarch64-unknown-linux-ohos'],
                                cwd=self.stage, capture_output=True, text=True, timeout=180)
        if result.returncode:
            raise ValueError(result.stderr[-4000:])
        packages = json.loads(result.stdout)['packages']
        arkui = [p for p in packages if p['name'] == 'ohos-arkui-binding']
        if len(arkui) != 1 or (self.plan['name'] != 'local-native-ui-context' and arkui[0]['source'] is not None):
            raise ValueError('The patched graph must contain one ArkUI binding')
        self.record.write_text(json.dumps({'originalInputs': self.inputs, 'recipeSha256': self.recipe,
                                          'patchPlan': self.plan,
                                          'patchedInputs': self.identity(self.stage)}, indent=2) + '\n')
        return self.stage


if __name__ == '__main__':
    print(BuildSource().prepare())
