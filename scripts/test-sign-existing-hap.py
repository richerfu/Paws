#!/usr/bin/env python3
"""Exercise the real signing workflow with public fixtures and a fake signer."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import sys
import tempfile
import unittest
from unittest.mock import patch
import zipfile


def load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


sign = load('paws_sign_existing', 'sign-existing-hap.py')
build = sign._build_tools()
fixtures = load('paws_manifest_fixtures', 'test-write-build-manifest.py')


class SignExistingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='paws-synthetic-sign-')
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.environment = patch.dict(os.environ, {}, clear=False)
        self.environment.start()
        self.addCleanup(self.environment.stop)
        os.environ.pop('PAWS_SIGNING_PROFILE', None)
        self.project = self.root / 'build-profile.json5'
        self.original = b'{"app":{"signingConfigs":[],"products":[{"name":"default"}]}}\n'
        self.project.write_bytes(self.original)
        self.project.chmod(0o640)
        private = self.root / '.ohos-sign'
        private.mkdir()
        for name in ('key.p12', 'profile.p7b', 'cert.cer'):
            (private / name).write_text('synthetic fixture, not a signing key')
        material = {'storeFile': '.ohos-sign/key.p12', 'profile': '.ohos-sign/profile.p7b',
                    'certpath': '.ohos-sign/cert.cer', 'storePassword': 'synthetic-never-export'}
        (private / 'build-profile.json5').write_text(json.dumps({'app': {
            'signingConfigs': [{'name': 'test', 'material': material}],
            'products': [{'name': 'default', 'signingConfig': 'test'}]}}))
        self.dist = self.root / 'dist'
        self.dist.mkdir()
        self.output = self.root / 'entry/build/default/outputs/default'
        self.output.mkdir(parents=True)
        self.unsigned = self.dist / 'Paws-release-unsigned.hap'
        self.entry_unsigned = self.output / 'entry-default-unsigned.hap'
        self.destination = self.dist / 'Paws-release-signed.hap'
        self.destination.write_bytes(b'previous signed artifact; must survive failure')
        self.destination.chmod(0o640)
        self.bundle = b'mount(PublicSyntheticApp);'
        self.write_hap(self.unsigned)
        shutil.copyfile(self.unsigned, self.entry_unsigned)
        native = self.root / 'synthetic-compiled.so'
        native.write_bytes(fixtures.elf_fixture())
        self.previous = {'format': 'TinyPaws-native-build-v1', 'hapBuildMode': 'release',
            'nativeProfile': 'release', 'builtAt': '2000-01-01T00:00:00+00:00',
            'buildFingerprint': 'a' * 64, 'sourceRevision': 'b' * 40,
            'sourceStatusSha256': 'c' * 64, 'sourceDirty': True,
            'nativeLibraries': {'libfixture.so': build.native_library_info(native)},
            'uiRuntime': {'source': {'sha256': 'd' * 64}, 'bundle': {
                'hapPath': 'resources/rawfile/paws.bundle.js',
                'bytes': len(self.bundle), 'sha256': build.hashlib.sha256(self.bundle).hexdigest()}}}
        build.attach_hap_provenance(self.previous, self.unsigned)
        self.manifest_path = self.dist / 'Paws-build-manifest.json'
        self.manifest_path.write_text(json.dumps(self.previous, indent=2) + '\n')
        self.manifest_path.chmod(0o640)
        self.before_manifest = self.manifest_path.read_bytes()
        self.before_signed = self.destination.read_bytes()
        self.signed_source = self.root / 'fake-signer-output.hap'
        self.write_hap(self.signed_source, signed=True)
        self.command = [sys.executable, '-c',
            "import shutil,sys; shutil.copyfile(sys.argv[1],sys.argv[2])",
            str(self.signed_source), str(self.output / 'entry-default-signed.hap')]
        self.log = self.root / 'synthetic-sign.log'

    def write_hap(self, path, *, signed=False, changed_member=None, loaded_byte=0x42):
        members = {
            'module.json': json.dumps({'app': {'debug': False, 'versionCode': 1, 'versionName': 'public.synthetic'}}).encode(),
            'libs/arm64-v8a/libfixture.so': fixtures.elf_fixture(stripped=True, loaded_byte=loaded_byte),
            'resources/rawfile/paws.bundle.js': self.bundle,
        }
        members.update({f'resources/public-{index:02}.txt': f'public fixture {index}'.encode() for index in range(27)})
        self.assertEqual(len(members), 30)
        if changed_member:
            members[changed_member] = b'changed synthetic payload'
        with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
            for name, data in members.items():
                archive.writestr(name, data)
            archive.comment = b'fake signer container' if signed else b'unsigned fixture'
            if signed:
                archive.writestr('.pages.info', bytes([0x21, 0x03, 0, 0, 0, 0, 0, 0]), compress_type=zipfile.ZIP_STORED)

    def run_signer(self):
        with patch.object(build, 'build_manifest', side_effect=AssertionError('Current source reattribution is forbidden')):
            # The production workflow loads the same real validation module.
            with patch.object(sign, '_build_tools', return_value=build):
                return sign.sign_existing(self.root, command=self.command, log_path=self.log)

    def repackage(self, path, *, changed=None, added=None, removed=None):
        with zipfile.ZipFile(self.unsigned) as original:
            members = {item.filename: original.read(item) for item in original.infolist()}
        if changed:
            members[changed] = b'changed synthetic unsigned member'
        if added:
            members[added] = bytes(8)
        if removed:
            members.pop(removed)
        with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_STORED) as result:
            for name, data in reversed(list(members.items())):
                member = zipfile.ZipInfo(name, date_time=(2001, 2, 3, 4, 5, 6))
                result.writestr(member, data)
            result.comment = b'public synthetic SignHap repack'

    def repack_signer(self, source, *, change_dist=False):
        self.command = [sys.executable, '-c',
            'import shutil,sys; shutil.copyfile(sys.argv[1],sys.argv[2]); shutil.copyfile(sys.argv[3],sys.argv[4])'
            + ('; shutil.copyfile(sys.argv[3],sys.argv[5])' if change_dist else ''),
            str(self.signed_source), str(self.output / 'entry-default-signed.hap'),
            str(source), str(self.entry_unsigned), str(self.unsigned)]

    def assert_original_outputs(self, *, absent=False):
        self.assertEqual(self.manifest_path.read_bytes(), self.before_manifest)
        if absent:
            self.assertFalse(self.destination.exists())
        else:
            self.assertEqual(self.destination.read_bytes(), self.before_signed)
            self.assertEqual(stat.S_IMODE(self.destination.stat().st_mode), 0o640)
        self.assertEqual(self.project.read_bytes(), self.original)
        self.assertEqual(stat.S_IMODE(self.project.stat().st_mode), 0o640)

    def test_success_preserves_original_source_native_and_ui_build_provenance(self):
        result = self.run_signer()
        actual = json.loads(self.manifest_path.read_bytes())
        self.assertEqual(self.destination.read_bytes(), self.signed_source.read_bytes())
        for key, value in self.previous.items():
            self.assertEqual(actual[key], value, key)
        self.assertEqual(actual['signedHapSha256'], build.sha(self.signed_source))
        self.assertTrue(actual['signedPayloadMatchesPackagedUnsignedPayload'])
        self.assertEqual(actual['signedAddedMetadata']['.pages.info']['bytes'], 8)
        self.assertEqual(result['sha256'], actual['signedHapSha256'])
        self.assertTrue(result['projectProfileRestored'])
        self.assertEqual(self.project.read_bytes(), self.original)
        self.assertNotIn('synthetic-never-export', json.dumps(actual))

    def test_two_equal_replaced_unsigned_files_cannot_override_previous_artifact_identity(self):
        self.write_hap(self.unsigned, changed_member='resources/public-00.txt')
        shutil.copyfile(self.unsigned, self.entry_unsigned)
        self.assertEqual(build.sha(self.unsigned), build.sha(self.entry_unsigned))
        with self.assertRaisesRegex(ValueError, 'previous verified'):
            self.run_signer()
        self.assert_original_outputs()
        self.assertFalse(self.log.exists(), 'A mismatched unsigned artifact must not start the signer')

    def test_signer_repackages_entry_unsigned_without_reattributing_the_original_build(self):
        repacked = self.root / 'repacked-unsigned.hap'
        self.repackage(repacked)
        self.assertNotEqual(build.sha(repacked), self.previous['unsignedHapSha256'])
        self.assertEqual(build.sha(self.entry_unsigned), self.previous['unsignedHapSha256'])
        self.repack_signer(repacked)
        self.run_signer()
        self.assertEqual(build.sha(self.unsigned), self.previous['unsignedHapSha256'])
        self.assertEqual(build.sha(self.entry_unsigned), build.sha(repacked))
        actual = json.loads(self.manifest_path.read_bytes())
        for key, value in self.previous.items():
            self.assertEqual(actual[key], value, key)
        self.assertEqual(self.destination.read_bytes(), self.signed_source.read_bytes())
        self.assertEqual(self.project.read_bytes(), self.original)

    def test_repack_cannot_change_add_or_remove_any_unsigned_payload_member(self):
        original = self.entry_unsigned.read_bytes()
        for options in ({'changed': 'resources/public-26.txt'}, {'added': '.pages.info'},
                        {'removed': 'resources/public-00.txt'}):
            with self.subTest(options=options):
                repacked = self.root / 'invalid-repacked-unsigned.hap'
                self.repackage(repacked, **options)
                self.repack_signer(repacked)
                try:
                    with self.assertRaisesRegex(build.ProvenanceError, 'Generated unsigned HAP ZIP payload'):
                        self.run_signer()
                    self.assert_original_outputs()
                finally:
                    self.entry_unsigned.write_bytes(original)

    def test_signer_cannot_repackage_the_preserved_dist_unsigned(self):
        repacked = self.root / 'repacked-unsigned.hap'
        self.repackage(repacked)
        self.repack_signer(repacked, change_dist=True)
        with self.assertRaisesRegex(ValueError, 'previous verified'):
            self.run_signer()
        self.assert_original_outputs()

    def test_either_unsigned_location_must_match_the_manifest(self):
        for path in (self.unsigned, self.entry_unsigned):
            with self.subTest(path=path.name):
                original = path.read_bytes()
                path.write_bytes(b'not the previously verified artifact')
                try:
                    with self.assertRaisesRegex(ValueError, 'previous verified'):
                        self.run_signer()
                    self.assert_original_outputs()
                finally:
                    path.write_bytes(original)

    def test_changed_signer_zip_payload_cannot_publish(self):
        for member in ('resources/public-00.txt', 'resources/public-26.txt', 'resources/rawfile/paws.bundle.js'):
            with self.subTest(member=member):
                self.write_hap(self.signed_source, signed=True, changed_member=member)
                with self.assertRaisesRegex(build.ProvenanceError, 'ZIP payload'):
                    self.run_signer()
                self.assert_original_outputs()

    def test_previous_native_and_ui_fingerprints_are_checked_before_signing(self):
        for key in ('native', 'ui'):
            with self.subTest(key=key):
                changed = copy.deepcopy(self.previous)
                if key == 'native':
                    changed['nativeLibraries']['libfixture.so']['loadSegmentSha256'] = 'f' * 64
                    reason = 'PT_LOAD'
                else:
                    changed['uiRuntime']['bundle']['sha256'] = 'e' * 64
                    reason = 'NSH bundle'
                self.manifest_path.write_text(json.dumps(changed))
                changed_bytes = self.manifest_path.read_bytes()
                with self.assertRaisesRegex(build.ProvenanceError, reason):
                    self.run_signer()
                self.assertEqual(self.manifest_path.read_bytes(), changed_bytes)
                self.assertEqual(self.destination.read_bytes(), self.before_signed)
                self.assertEqual(self.project.read_bytes(), self.original)
                self.assertFalse(self.log.exists())
        self.manifest_path.write_bytes(self.before_manifest)

    def test_second_atomic_replace_failure_rolls_back_existing_or_absent_signed_target(self):
        replace = os.replace
        for absent in (False, True):
            with self.subTest(absent=absent):
                if absent:
                    self.destination.unlink()
                triggered = False

                def fail_manifest_once(source, target):
                    nonlocal triggered
                    if Path(target) == self.manifest_path and not triggered:
                        triggered = True
                        raise OSError('synthetic second publish failure')
                    return replace(source, target)

                with patch.object(sign.os, 'replace', side_effect=fail_manifest_once):
                    with self.assertRaisesRegex(OSError, 'second publish failure'):
                        self.run_signer()
                self.assertTrue(triggered)
                self.assert_original_outputs(absent=absent)

    def test_signer_failure_or_missing_output_never_reuses_previous_generated_hap(self):
        generated = self.output / 'entry-default-signed.hap'
        for code in (0, 9):
            with self.subTest(code=code):
                generated.write_bytes(self.signed_source.read_bytes())
                self.command = [sys.executable, '-c', f'raise SystemExit({code})']
                with self.assertRaises((ValueError, sign.subprocess.CalledProcessError)):
                    self.run_signer()
                self.assert_original_outputs()

    def test_concurrent_project_edit_prevents_publication_and_is_not_overwritten(self):
        changed = b'{"concurrent": "synthetic project edit"}\n'
        self.command = [sys.executable, '-c',
            'import shutil,sys; from pathlib import Path; shutil.copyfile(sys.argv[1],sys.argv[2]); Path(sys.argv[3]).write_bytes(' + repr(changed) + ')',
            str(self.signed_source), str(self.output / 'entry-default-signed.hap'), str(self.project)]
        with self.assertRaisesRegex(RuntimeError, 'changed concurrently'):
            self.run_signer()
        self.assertEqual(self.project.read_bytes(), changed)
        self.assertEqual(self.destination.read_bytes(), self.before_signed)
        self.assertEqual(self.manifest_path.read_bytes(), self.before_manifest)


if __name__ == '__main__':
    unittest.main()
