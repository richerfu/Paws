#!/usr/bin/env python3
"""Stage the local NSH sources without modifying the SDK checkout.

The user's NSH checkout and Cargo's immutable Git cache remain untouched.
The unmodified workspace is covered by the normal NSH source fingerprint
and is reused only while that identity matches.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess

from nsh_build_inputs import ROOT, default_ability_root, default_nsh_root, resolve_inputs


class BuildSource:
    def __init__(self):
        if not os.environ.get('NSH_ROOT'):
            self.checkout()
        self.inputs = resolve_inputs()
        self.source = Path(self.inputs['nshRoot'])
        if self.inputs.get('hostModel') != 'native-ui-context':
            raise ValueError('Select the pinned native UIContext SDK; legacy Ability SDK overlays are unsupported')
        recipe = hashlib.sha256(Path(__file__).read_bytes())
        recipe.update((ROOT / 'scripts/nsh_build_inputs.py').read_bytes())
        self.recipe = recipe.hexdigest()
        # package-hap exports this exact root to its build subprocess. Passing
        # an already prepared root must verify it, never copy it again.
        if self.source.parent == ROOT / 'target/nsh-build-source':
            self.stage = self.source
            self.record = self.stage.with_suffix('.json')
            if not self.record.is_file():
                raise ValueError('Incomplete NSH stage; select the original NSH_ROOT')
            expected = json.loads(self.record.read_text())
            if expected.get('recipeSha256') != self.recipe:
                raise ValueError('NSH staging recipe changed; select the original NSH_ROOT')
            if expected['preparedInputs'] != self.inputs:
                raise ValueError('Staged NSH sources changed; select the original NSH_ROOT')
            return
        digest = hashlib.sha256(json.dumps(self.inputs, sort_keys=True).encode())
        digest.update(self.recipe.encode())
        self.stage = ROOT / 'target/nsh-build-source' / digest.hexdigest()[:24]
        self.record = self.stage.with_suffix('.json')

    @classmethod
    def checkout(cls):
        """Copy Git objects from the sibling repository, never its worktree."""
        cls.checkout_at(ROOT.parent / 'nsh', default_nsh_root())
        if not os.environ.get('NSH_ABILITY_ROOT'):
            target = default_ability_root()
            source = ROOT.parent / 'openharmony-ability'
            if not target.exists():
                candidates = [source, *sorted(Path.home().joinpath('.cargo/git/db').glob('openharmony-ability-*'))]
                for candidate in candidates:
                    result = subprocess.run(['git', '-C', str(candidate), 'cat-file', '-e', target.name + '^{commit}'],
                                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
                    if result.returncode == 0:
                        source = candidate
                        break
                else:
                    raise ValueError('Application Ability sources are unavailable locally; set NSH_ABILITY_ROOT')
            cls.checkout_at(source, target)

    @staticmethod
    def checkout_at(source, target):
        """Create a clean pinned checkout and reject subsequent tampering."""
        revision = target.name
        git = ['git', '-c', 'core.fsmonitor=false']
        if not target.exists():
            subprocess.run([*git, '-C', str(source), 'cat-file', '-e', revision + '^{commit}'],
                           check=True, timeout=30)
            target.parent.mkdir(parents=True, exist_ok=True)
            subprocess.run([*git, 'clone', '--no-hardlinks', '--no-checkout', '--', str(source), str(target)],
                           check=True, stdout=subprocess.DEVNULL, timeout=180)
            subprocess.run([*git, '-C', str(target), '-c', 'core.hooksPath=/dev/null',
                            'checkout', '--detach', revision],
                           check=True, stdout=subprocess.DEVNULL, timeout=60)
        head = subprocess.check_output([*git, '-C', str(target), 'rev-parse', 'HEAD'],
                                       text=True, timeout=30).strip()
        status = subprocess.check_output([*git, '-C', str(target), 'status', '--porcelain=v1',
                                          '--untracked-files=all'], timeout=30)
        if head != revision or status:
            raise ValueError(f'The pinned source checkout changed: {target}')

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

    def prepare(self):
        if self.record.exists():
            expected = json.loads(self.record.read_text())
            if (expected.get('recipeSha256') != self.recipe
                    or self.identity(self.stage) != expected['preparedInputs']):
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
        result = subprocess.run(['cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
                                 '--filter-platform', 'aarch64-unknown-linux-ohos'],
                                cwd=self.stage, capture_output=True, text=True, timeout=180)
        if result.returncode:
            raise ValueError(result.stderr[-4000:])
        packages = json.loads(result.stdout)['packages']
        arkui = [p for p in packages if p['name'] == 'ohos-arkui-binding']
        if len(arkui) != 1:
            raise ValueError('The native SDK graph must contain one ArkUI binding')
        self.record.write_text(json.dumps({'originalInputs': self.inputs, 'recipeSha256': self.recipe,
                                          'preparedInputs': self.identity(self.stage)}, indent=2) + '\n')
        return self.stage


if __name__ == '__main__':
    print(BuildSource().prepare())
