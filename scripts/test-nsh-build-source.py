#!/usr/bin/env python3
"""Exercise isolated source staging and patches with an offline Cargo graph."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('build_source', Path(__file__).with_name('prepare-nsh-build-source.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class BuildSourceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        base = Path(temporary.name).resolve()
        self.source, ability, self.root = base / 'nsh', base / 'ability', base / 'paws'
        for repo in (self.source, ability):
            repo.mkdir()
            self.git(repo, 'init', '-q')
            for key, value in [('user.name', 'Build test'), ('user.email', 'build@example.invalid'),
                               ('commit.gpgsign', 'false'), ('core.hooksPath', '/dev/null')]:
                self.git(repo, 'config', key, value)
        (ability / 'native_ability').mkdir()
        (ability / 'native_ability/oh-package.json5').write_text('{}')
        (ability / 'crates/ability').mkdir(parents=True)
        self.source.joinpath('Cargo.toml').write_text('[workspace]\nmembers=["app", "binding"]\nresolver="2"\n')
        (self.source / 'app/src').mkdir(parents=True)
        (self.source / 'app/Cargo.toml').write_text('[package]\nname="fixture"\nversion="0.1.0"\nedition="2021"\n'
            '[dependencies]\nohos-arkui-binding={path="../binding"}\n')
        (self.source / 'app/src/lib.rs').write_text('')
        (self.source / 'binding/src').mkdir(parents=True)
        (self.source / 'binding/Cargo.toml').write_text('[package]\nname="ohos-arkui-binding"\nversion="0.4.2"\nedition="2021"\n')
        (self.source / 'binding/src/lib.rs').write_text('')
        subprocess.run(['cargo', 'generate-lockfile', '--offline'], cwd=self.source, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
        (self.source / 'source.rs').write_text('original\n')
        (self.source / '.gitignore').write_text('/crates/nsh-platform-ohos/\n/target/\n')
        for repo in (self.source, ability):
            self.git(repo, 'add', '.')
            self.git(repo, 'commit', '-qm', 'fixture')
        self.git(self.source, 'config', 'core.worktree', str(self.source))
        port = self.source / 'crates/nsh-platform-ohos'
        (port / 'arkts/oh_modules').mkdir(parents=True)
        (port / 'native.rs').write_text('ignored actual source\n')
        (port / 'arkts/oh_modules/generated.js').write_text('cache')
        (self.root / 'scripts').mkdir(parents=True)
        shutil.copyfile(Path(__file__).with_name('nsh_build_inputs.py'), self.root / 'scripts/nsh_build_inputs.py')
        self.environment = patch.dict(os.environ, {'NSH_ROOT': str(self.source), 'NSH_ABILITY_ROOT': str(ability)})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        root_patch = patch.object(module, 'ROOT', self.root)
        root_patch.start()
        self.addCleanup(root_patch.stop)
    @staticmethod
    def git(root, *args):
        return subprocess.check_output(['git', '-C', str(root), *args], stderr=subprocess.PIPE, timeout=20)

    def test_actual_source_copy_and_prepared_root_are_idempotent(self):
        before = self.git(self.source, 'diff', 'HEAD', '--binary')
        stage = module.BuildSource().prepare()
        self.assertEqual((stage / 'source.rs').read_text(), 'original\n')
        self.assertEqual(self.git(stage, 'diff', 'HEAD', '--binary'), before)
        record = json.loads(stage.with_suffix('.json').read_text())
        self.assertEqual(record['originalInputs']['nsh'], record['preparedInputs']['nsh'])
        self.assertEqual(record['originalInputs']['ability'], record['preparedInputs']['ability'])
        self.assertTrue((stage / 'crates/nsh-platform-ohos/native.rs').is_file())
        self.assertFalse((stage / 'crates/nsh-platform-ohos/arkts/oh_modules').exists())
        self.assertEqual(self.git(self.source, 'diff', 'HEAD', '--binary'), before)
        self.assertEqual((self.source / 'source.rs').read_text(), 'original\n')
        self.assertEqual(self.git(stage, 'config', 'core.worktree').decode().strip(), str(stage))
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}):
            self.assertEqual(module.BuildSource().prepare(), stage)
        self.assertEqual(len(list((self.root / 'target/nsh-build-source').glob('*/Cargo.toml'))), 1)

    def test_pinned_checkout_ignores_the_source_branch_and_local_edits(self):
        revision = self.git(self.source, 'rev-parse', 'HEAD').decode().strip()
        self.git(self.source, 'checkout', '-qb', 'another-branch')
        (self.source / 'source.rs').write_text('newer branch\n')
        self.git(self.source, 'commit', '-qam', 'another revision')
        (self.source / 'source.rs').write_text('uncommitted work\n')
        (self.source / 'untracked.rs').write_text('personal work\n')
        before = self.git(self.source, 'status', '--porcelain=v1')
        target = self.root / 'target/nsh-checkouts' / revision
        module.BuildSource.checkout_at(self.source, target)
        self.assertEqual(self.git(target, 'rev-parse', 'HEAD').decode().strip(), revision)
        self.assertEqual((target / 'source.rs').read_text(), 'original\n')
        self.assertFalse((target / 'untracked.rs').exists())
        self.assertEqual(self.git(self.source, 'status', '--porcelain=v1'), before)
        self.assertEqual((self.source / 'source.rs').read_text(), 'uncommitted work\n')
        module.BuildSource.checkout_at(self.source, target)
        (target / 'source.rs').write_text('tampered\n')
        with self.assertRaisesRegex(ValueError, 'pinned source checkout changed'):
            module.BuildSource.checkout_at(self.source, target)

    def test_patches_are_isolated_fingerprinted_and_reapplied_after_changes(self):
        patch_dir = self.root / 'patches/nsh'
        patch_dir.mkdir(parents=True)
        patch_file = patch_dir / 'dialog.patch'
        patch_file.write_text('--- a/source.rs\n+++ b/source.rs\n@@ -1 +1 @@\n-original\n+patched\n')
        stage = module.BuildSource().prepare()
        self.assertEqual((self.source / 'source.rs').read_text(), 'original\n')
        self.assertEqual((stage / 'source.rs').read_text(), 'patched\n')
        record = json.loads(stage.with_suffix('.json').read_text())
        self.assertNotEqual(record['originalInputs']['nsh'], record['preparedInputs']['nsh'])
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}):
            self.assertEqual(module.BuildSource().prepare(), stage)
            patch_file.write_text(patch_file.read_text().replace('+patched', '+updated'))
            with self.assertRaisesRegex(ValueError, 'recipe changed'):
                module.BuildSource()
        updated = module.BuildSource().prepare()
        self.assertNotEqual(updated, stage)
        self.assertEqual((updated / 'source.rs').read_text(), 'updated\n')

    def test_incompatible_patch_fails_without_changing_original_source(self):
        patch_dir = self.root / 'patches/nsh'
        patch_dir.mkdir(parents=True)
        (patch_dir / 'incompatible.patch').write_text(
            '--- a/source.rs\n+++ b/source.rs\n@@ -1 +1 @@\n-missing\n+patched\n')
        with self.assertRaisesRegex(ValueError, 'Cannot apply NSH patch incompatible.patch'):
            module.BuildSource().prepare()
        self.assertEqual((self.source / 'source.rs').read_text(), 'original\n')
        self.assertEqual(list((self.root / 'target/nsh-build-source').glob('*.json')), [])

    def test_ignored_actual_source_tampering_is_rejected(self):
        stage = module.BuildSource().prepare()
        (stage / 'crates/nsh-platform-ohos/native.rs').write_text('changed\n')
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}), self.assertRaisesRegex(ValueError, 'Staged NSH sources changed'):
            module.BuildSource().prepare()

    def test_prepared_root_rejects_changed_staging_recipe(self):
        stage = module.BuildSource().prepare()
        recipe = self.root / 'scripts/nsh_build_inputs.py'
        recipe.write_text(recipe.read_text() + '\n')
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}), self.assertRaisesRegex(ValueError, 'recipe changed'):
            module.BuildSource().prepare()

    def test_binding_source_tampering_is_rejected(self):
        stage = module.BuildSource().prepare()
        (stage / 'binding/src/lib.rs').write_text('changed binding\n')
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}), self.assertRaisesRegex(ValueError, 'Staged NSH sources changed'):
            module.BuildSource().prepare()

    def test_legacy_ability_sdk_is_rejected_without_modifying_the_source(self):
        manifest = self.source / 'Cargo.toml'
        with manifest.open('a') as stream:
            stream.write('[workspace.dependencies]\nopenharmony-ability={path="'
                         + str(Path(os.environ['NSH_ABILITY_ROOT']) / 'crates/ability') + '"}\n')
        before = manifest.read_bytes()
        with self.assertRaisesRegex(ValueError, 'legacy Ability SDK overlays are unsupported'):
            module.BuildSource().prepare()
        self.assertEqual(manifest.read_bytes(), before)

    def test_source_changes_during_copy_are_rejected(self):
        copytree = shutil.copytree

        def copying(source, target, *args, **kwargs):
            result = copytree(source, target, *args, **kwargs)
            if Path(source) == self.source:
                (self.source / 'source.rs').write_text('changed concurrently\n')
            return result

        with patch.object(module.shutil, 'copytree', copying), self.assertRaisesRegex(ValueError, 'changed during staging'):
            module.BuildSource().prepare()


if __name__ == '__main__':
    unittest.main()
