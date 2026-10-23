#!/usr/bin/env python3
"""Exercise source staging with real Git trees, patches and an offline Cargo graph."""
import importlib.util
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
    def test_only_reviewed_binding_sources_receive_their_required_preparation(self):
        self.assertEqual(module.BuildSource.binding_preparation('e9a376c4313d76eeb41b148d05b682576b57a2e0'), ())
        self.assertEqual(module.BuildSource.binding_preparation('43aec6c125503a47f7e875c793aaf2462e91ad19'),
                         ('arkui-dialog-operation.patch', 'arkui-host-fixtures.patch'))
        with self.assertRaisesRegex(ValueError, 'Rebase'):
            module.BuildSource.binding_preparation('unreviewed')

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
        self.source.joinpath('Cargo.toml').write_text('[workspace]\nmembers=["app"]\nexclude=["paws-patches/ohos-arkui-binding"]\nresolver="2"\n'
            '[workspace.dependencies]\nopenharmony-ability={path="' + str(ability / 'crates/ability') + '"}\n')
        (self.source / 'app/src').mkdir(parents=True)
        (self.source / 'app/Cargo.toml').write_text('[package]\nname="fixture"\nversion="0.1.0"\nedition="2021"\n')
        (self.source / 'app/src/lib.rs').write_text('')
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
        (self.root / 'patches/nsh').mkdir(parents=True)
        self.patch_file = self.root / 'patches/nsh/source.patch'
        self.patch_file.write_text('--- a/source.rs\n+++ b/source.rs\n@@ -1 +1 @@\n-original\n+patched\n')
        self.environment = patch.dict(os.environ, {'NSH_ROOT': str(self.source), 'NSH_ABILITY_ROOT': str(ability)})
        self.environment.start()
        self.addCleanup(self.environment.stop)
        root_patch = patch.object(module, 'ROOT', self.root)
        root_patch.start()
        self.addCleanup(root_patch.stop)
        binding_patch = patch.object(module.BuildSource, 'binding', self.binding)
        binding_patch.start()
        self.addCleanup(binding_patch.stop)

    @staticmethod
    def git(root, *args):
        return subprocess.check_output(['git', '-C', str(root), *args], stderr=subprocess.PIPE, timeout=20)

    @staticmethod
    def binding(builder):
        crate = builder.stage / 'paws-patches/ohos-arkui-binding'
        (crate / 'src').mkdir(parents=True)
        (crate / 'Cargo.toml').write_text('[package]\nname="ohos-arkui-binding"\nversion="0.4.2"\nedition="2021"\n[workspace]\n')
        (crate / 'src/lib.rs').write_text('')
        with (builder.stage / 'app/Cargo.toml').open('a') as stream:
            stream.write('[dependencies]\nohos-arkui-binding={path="../paws-patches/ohos-arkui-binding"}\n')

    def test_actual_source_copy_and_prepared_root_are_idempotent(self):
        before = self.git(self.source, 'diff', 'HEAD', '--binary')
        stage = module.BuildSource().prepare()
        self.assertEqual((stage / 'source.rs').read_text(), 'patched\n')
        self.assertTrue((stage / 'crates/nsh-platform-ohos/native.rs').is_file())
        self.assertFalse((stage / 'crates/nsh-platform-ohos/arkts/oh_modules').exists())
        self.assertEqual(self.git(self.source, 'diff', 'HEAD', '--binary'), before)
        self.assertEqual((self.source / 'source.rs').read_text(), 'original\n')
        self.assertEqual(self.git(stage, 'config', 'core.worktree').decode().strip(), str(stage))
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}):
            self.assertEqual(module.BuildSource().prepare(), stage)
        self.assertEqual(len(list((self.root / 'target/nsh-build-source').glob('*/Cargo.toml'))), 1)

    def test_ignored_actual_source_tampering_is_rejected(self):
        stage = module.BuildSource().prepare()
        (stage / 'crates/nsh-platform-ohos/native.rs').write_text('changed\n')
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}), self.assertRaisesRegex(ValueError, 'Staged NSH sources changed'):
            module.BuildSource().prepare()

    def test_prepared_root_rejects_changed_patch_recipe(self):
        stage = module.BuildSource().prepare()
        self.patch_file.write_text(self.patch_file.read_text() + '\n')
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}), self.assertRaisesRegex(ValueError, 'recipe changed'):
            module.BuildSource().prepare()

    def test_patched_binding_source_tampering_is_rejected(self):
        stage = module.BuildSource().prepare()
        (stage / 'paws-patches/ohos-arkui-binding/src/lib.rs').write_text('changed binding\n')
        with patch.dict(os.environ, {'NSH_ROOT': str(stage)}), self.assertRaisesRegex(ValueError, 'Staged NSH sources changed'):
            module.BuildSource().prepare()

    def test_source_changes_during_copy_fail_before_patching(self):
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
