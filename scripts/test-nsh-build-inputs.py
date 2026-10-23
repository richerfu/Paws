#!/usr/bin/env python3
"""Exercise SDK reuse against real clean, tracked-dirty and untracked Git trees."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

from nsh_build_inputs import resolve_inputs, verify_source_identity


class SourceIdentityTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        root = Path(self.temporary.name)
        self.nsh, self.ability = root / 'nsh', root / 'ability'
        for repository in (self.nsh, self.ability):
            repository.mkdir()
            self.git(repository, 'init', '-q')
            self.git(repository, 'config', 'user.name', 'Build test')
            self.git(repository, 'config', 'user.email', 'build@example.invalid')
            self.git(repository, 'config', 'commit.gpgsign', 'false')
            self.git(repository, 'config', 'core.hooksPath', '/dev/null')
            (repository / 'source.rs').write_text('initial source\n')
        (self.nsh / 'Cargo.toml').write_text('[workspace.dependencies]\nopenharmony-ability = { path = "../ability/crates/ability" }\n')
        (self.ability / 'crates/ability').mkdir(parents=True)
        (self.ability / 'crates/ability/Cargo.toml').write_text('[package]\nname="openharmony-ability"\nversion="1.0.0"\n')
        (self.ability / 'native_ability').mkdir()
        (self.ability / 'native_ability/oh-package.json5').write_text('{}')
        for repository in (self.nsh, self.ability):
            self.git(repository, 'add', '.')
            self.git(repository, 'commit', '-qm', 'initial')
        self.environment = patch.dict(os.environ, {'NSH_ROOT': str(self.nsh), 'NSH_ABILITY_ROOT': str(self.ability)})
        self.environment.start()
        self.addCleanup(self.environment.stop)

    def git(self, root, *arguments):
        return subprocess.check_output(['git', '-C', str(root), *arguments], stderr=subprocess.PIPE, timeout=20)

    def test_unchanged_sources_reuse_and_old_records_fail_closed(self):
        before = resolve_inputs()
        verify_source_identity(before, resolve_inputs())
        legacy = json.loads(json.dumps(before))
        del legacy['nsh']['worktreeSha256']
        with self.assertRaisesRegex(ValueError, 'no recorded worktree digest'):
            verify_source_identity(legacy, before)

    def test_same_head_and_dirty_flag_do_not_hide_tracked_changes(self):
        for repository, name in ((self.nsh, 'nsh'), (self.ability, 'ability')):
            (repository / 'source.rs').write_text('dirty first\n')
            before = resolve_inputs()
            (repository / 'source.rs').write_text('dirty second\n')
            after = resolve_inputs()
            self.assertEqual(before[name]['revision'], after[name]['revision'])
            self.assertTrue(before[name]['dirty'] and after[name]['dirty'])
            self.assertNotEqual(before[name]['worktreeSha256'], after[name]['worktreeSha256'])
            with self.assertRaisesRegex(ValueError, 'worktree content changed'):
                verify_source_identity(before, after)

    def test_untracked_contents_and_paths_are_part_of_identity(self):
        (self.nsh / 'new.rs').write_bytes(b'first')
        before = resolve_inputs()
        (self.nsh / 'new.rs').write_bytes(b'other')
        after = resolve_inputs()
        with self.assertRaises(ValueError):
            verify_source_identity(before, after)
        (self.nsh / 'new.rs').rename(self.nsh / 'renamed.rs')
        with self.assertRaises(ValueError):
            verify_source_identity(after, resolve_inputs())

    def test_ability_override_cannot_stage_a_different_checkout_than_the_native_link(self):
        other = self.ability.parent / 'other-ability'
        other.mkdir()
        with patch.dict(os.environ, {'NSH_ABILITY_ROOT': str(other)}):
            with self.assertRaisesRegex(ValueError, 'must match the Ability path linked'):
                resolve_inputs()

    def git_ability_fixture(self):
        revision = self.git(self.ability, 'rev-parse', 'HEAD').decode().strip()
        source = f'git+https://example.invalid/openharmony-ability?rev={revision}#{revision}'
        (self.nsh / 'Cargo.toml').write_text('[workspace.dependencies]\nopenharmony-ability = '
            f'{{ git="https://example.invalid/openharmony-ability", rev="{revision}" }}\n')
        (self.nsh / 'Cargo.lock').write_text('version=4\n[[package]]\nname="openharmony-ability"\n'
            f'version="1.0.0"\nsource="{source}"\n')
        return source, {'packages': [{'name': 'openharmony-ability', 'source': source,
            'manifest_path': str(self.ability / 'crates/ability/Cargo.toml')}]}

    def test_git_dependency_stages_the_actual_locked_cargo_checkout(self):
        source, metadata = self.git_ability_fixture()
        with patch('nsh_build_inputs.ability_metadata', return_value=metadata):
            actual = resolve_inputs()
            self.assertEqual(actual['abilityRoot'], str(self.ability.resolve()))
            self.assertEqual(actual['abilityDependencySource'], source)
            verify_source_identity(actual, resolve_inputs())
            other = self.ability.parent / 'wrong-ability'
            other.mkdir()
            with patch.dict(os.environ, {'NSH_ABILITY_ROOT': str(other)}):
                with self.assertRaisesRegex(ValueError, 'actual Cargo OHOS Ability checkout'):
                    resolve_inputs()

    def test_git_dependency_rejects_changed_checkout_head_and_stale_metadata(self):
        _, metadata = self.git_ability_fixture()
        with patch('nsh_build_inputs.ability_metadata', return_value=metadata):
            (self.ability / 'source.rs').write_text('different commit\n')
            self.git(self.ability, 'add', '.')
            self.git(self.ability, 'commit', '-qm', 'different')
            with self.assertRaisesRegex(ValueError, 'locked Git revision'):
                resolve_inputs()
        metadata['packages'][0]['source'] = 'git+https://example.invalid/other#' + '0' * 40
        with patch('nsh_build_inputs.ability_metadata', return_value=metadata):
            with self.assertRaisesRegex(ValueError, 'metadata does not match'):
                resolve_inputs()

    def test_cargo_git_completion_marker_is_not_a_source_input(self):
        _, metadata = self.git_ability_fixture()
        with patch('nsh_build_inputs.ability_metadata', return_value=metadata):
            before = resolve_inputs()
            (self.ability / '.cargo-ok').write_text('cargo generated marker\n')
            after = resolve_inputs()
            self.assertFalse(after['ability']['dirty'])
            verify_source_identity(before, after)

    def test_ignored_sdk_source_is_fingerprinted_but_generated_dependencies_are_not(self):
        (self.nsh / '.gitignore').write_text('/crates/nsh-platform-ohos/src/ignored.rs\n**/oh_modules/\n')
        self.git(self.nsh, 'add', '.gitignore')
        self.git(self.nsh, 'commit', '-qm', 'ignore SDK fixture')
        source = self.nsh / 'crates/nsh-platform-ohos/src/ignored.rs'
        source.parent.mkdir(parents=True)
        source.write_text('first SDK source\n')
        before = resolve_inputs()
        source.write_text('changed SDK source\n')
        after = resolve_inputs()
        self.assertTrue(before['nsh']['dirty'])
        self.assertEqual(before['nsh']['sdkSourceFileCount'], 1)
        self.assertNotEqual(before['nsh']['sdkSourceSha256'], after['nsh']['sdkSourceSha256'])
        with self.assertRaises(ValueError):
            verify_source_identity(before, after)
        generated = self.nsh / 'crates/nsh-platform-ohos/arkts/oh_modules/ignored.ts'
        generated.parent.mkdir(parents=True)
        generated.write_text('generated dependency\n')
        verify_source_identity(after, resolve_inputs())


if __name__ == '__main__':
    unittest.main()
