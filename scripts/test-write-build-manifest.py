#!/usr/bin/env python3
"""Verify build provenance with self-contained, non-sensitive ELF/HAP fixtures."""
import copy
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import struct
import tempfile
import unittest
from unittest.mock import patch
import zipfile

SPEC = importlib.util.spec_from_file_location('build_manifest', Path(__file__).with_name('write-build-manifest.py'))
build = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(build)


def elf_fixture(*, stripped=False, loaded_byte=0x42, memsz=512, tail=b'non-loaded-section'):
    data = bytearray(256)
    data[:16] = b'\x7fELF\x02\x01\x01' + bytes(9)
    struct.pack_into('<HHIQQQIHHHHHH', data, 16, 3, 183, 1, 0, 64,
                     0 if stripped else 256, 0, 64, 56, 1,
                     0 if stripped else 64, 0 if stripped else 2, 0 if stripped else 1)
    struct.pack_into('<IIQQQQQQ', data, 64, 1, 5, 0, 4096, 4096, 256, memsz, 4096)
    data[200] = loaded_byte
    return bytes(data) + (b'' if stripped else tail)


def write_hap(path, library, *, debug=False, resource=b'public synthetic resource', comment=b''):
    app = {'debug': debug, 'versionCode': 1002009, 'versionName': '1.2.9-synthetic'}
    with zipfile.ZipFile(path, 'w', compression=zipfile.ZIP_DEFLATED) as archive:
        archive.writestr('module.json', json.dumps({'app': app}))
        archive.writestr('libs/arm64-v8a/libfixture.so', library)
        archive.writestr('resources/public.txt', resource)
        archive.comment = comment


class LoadSegmentTests(unittest.TestCase):
    def fingerprint(self, data):
        return build.elf_load_fingerprint(io.BytesIO(data), len(data))

    def test_hvigor_section_metadata_and_unloaded_tail_may_differ(self):
        compiled, packaged = elf_fixture(), elf_fixture(stripped=True)
        self.assertNotEqual(hashlib.sha256(compiled).digest(), hashlib.sha256(packaged).digest())
        loaded = self.fingerprint(compiled)
        self.assertEqual(loaded, self.fingerprint(packaged))
        self.assertEqual((loaded['loadSegmentCount'], loaded['loadSegmentBytes']), (1, 256))

    def test_loaded_bytes_and_segment_metadata_are_authenticated_by_hash(self):
        baseline = self.fingerprint(elf_fixture())['loadSegmentSha256']
        for changed in (elf_fixture(loaded_byte=0x43), elf_fixture(memsz=513)):
            self.assertNotEqual(baseline, self.fingerprint(changed)['loadSegmentSha256'])

    def test_rejects_bad_headers_and_out_of_bounds_segments(self):
        bad_magic = bytearray(elf_fixture()); bad_magic[0] = 0
        elf32 = bytearray(elf_fixture()); elf32[4] = 1
        bad_range = bytearray(elf_fixture()); struct.pack_into('<Q', bad_range, 96, 4096)
        bad_count = bytearray(elf_fixture()); struct.pack_into('<H', bad_count, 56, 65535)
        bad_alignment = bytearray(elf_fixture()); struct.pack_into('<Q', bad_alignment, 112, 3)
        for invalid in (b'ELF', bad_magic, elf32, bad_range, bad_count, bad_alignment):
            with self.subTest(length=len(invalid)), self.assertRaises(build.ProvenanceError):
                self.fingerprint(invalid)


class ManifestTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.library = self.root / 'entry/libs/arm64-v8a/libfixture.so'
        self.library.parent.mkdir(parents=True)
        self.library.write_bytes(elf_fixture())
        self.hap = self.root / 'unsigned.hap'
        self.signed = self.root / 'signed.hap'
        write_hap(self.hap, elf_fixture(stripped=True), comment=b'unsigned container')
        write_hap(self.signed, elf_fixture(stripped=True), comment=b'synthetic signature container')

    def make_manifest(self, environment=None):
        def command(command, root):
            if command[:2] == ['rustc', '--version']: return 'rustc synthetic'
            if command[1] == 'rev-parse': return 'c' * 40
            return ' M synthetic.rs\0'
        environment = {'PAWS_ENGINE_BUILD_ID': 'a' * 64} if environment is None else environment
        with patch.object(build, '_command_output', side_effect=command):
            return build.build_manifest(self.root, 'release', 'release', environment)

    def test_flags_dirty_state_and_absence_of_raw_pins_or_credentials(self):
        environment = {'PAWS_ENGINE_BUILD_ID': 'a' * 64, 'PAWS_RAW_ENGINE_DIAGNOSTIC': '1',
                       'PAWS_SOCKET_WIRE_CAPTURE': '1', 'PAWS_BENCH_PEER_IP': '8.8.8.8',
                       'PAWS_BENCH_NODE_FINGERPRINT': 'b' * 64, 'PASSWORD': 'synthetic-do-not-export'}
        manifest = self.make_manifest(environment)
        self.assertTrue(manifest['wireCapture'])
        self.assertTrue(manifest['diagnosticBypass'])
        self.assertTrue(manifest['diagnosticPeerPinned'])
        self.assertTrue(manifest['sourceDirty'])
        encoded = json.dumps(manifest)
        for private in ('8.8.8.8', 'b' * 64, 'synthetic-do-not-export', 'synthetic.rs'):
            self.assertNotIn(private, encoded)
        environment['PAWS_RAW_ENGINE_DIAGNOSTIC'] = '0'
        environment['PAWS_SOCKET_WIRE_CAPTURE'] = 'true'
        disabled = self.make_manifest(environment)
        self.assertFalse(disabled['diagnosticBypass'])
        self.assertFalse(disabled['wireCapture'])
        self.assertFalse(disabled['diagnosticPeerPinned'])

    def test_unsigned_and_signed_hap_provenance(self):
        manifest = build.attach_hap_provenance(self.make_manifest(), self.hap, self.signed)
        library = manifest['nativeLibraries']['libfixture.so']
        self.assertNotEqual(library['sha256'], library['packagedSha256'])
        self.assertEqual(library['loadSegmentSha256'], library['packagedLoadSegmentSha256'])
        self.assertTrue(library['loadSegmentsMatchCompiledLibrary'])
        self.assertEqual(library['packagedBytes'], len(elf_fixture(stripped=True)))
        self.assertFalse(manifest['unsignedHapVersion']['debug'])
        self.assertTrue(manifest['signedPayloadMatchesPackagedUnsignedPayload'])
        self.assertEqual(manifest['unsignedHapSha256'], build.sha(self.hap))
        self.assertEqual(manifest['signedHapSha256'], build.sha(self.signed))
        self.assertNotEqual(manifest['unsignedHapSha256'], manifest['signedHapSha256'])

    def test_hap_native_or_signed_payload_change_is_rejected(self):
        write_hap(self.hap, elf_fixture(stripped=True, loaded_byte=0x43))
        with self.assertRaisesRegex(build.ProvenanceError, 'PT_LOAD'):
            build.attach_hap_provenance(self.make_manifest(), self.hap)
        write_hap(self.hap, elf_fixture(stripped=True))
        write_hap(self.signed, elf_fixture(stripped=True), resource=b'changed synthetic resource')
        with self.assertRaisesRegex(build.ProvenanceError, 'ZIP payload'):
            build.attach_hap_provenance(self.make_manifest(), self.hap, self.signed)

    def test_signed_pages_info_is_the_only_allowed_new_metadata(self):
        bitmap = bytes([0x21, 0x03, 0, 0, 0, 0, 0, 0])
        with zipfile.ZipFile(self.signed, 'a') as archive:
            archive.writestr('.pages.info', bitmap, compress_type=zipfile.ZIP_STORED)
        manifest = build.attach_hap_provenance(self.make_manifest(), self.hap, self.signed)
        self.assertTrue(manifest['signedPayloadMatchesPackagedUnsignedPayload'])
        metadata = manifest['signedAddedMetadata']['.pages.info']
        self.assertEqual(metadata['bytes'], len(bitmap))
        self.assertEqual(metadata['sha256'], hashlib.sha256(bitmap).hexdigest())
        self.assertTrue(metadata['structureValidated'])
        self.assertIn('cryptographic signatures are not verified', metadata['validationScope'])

    def test_signed_pages_info_rejects_invalid_size_bits_or_container(self):
        for bitmap, compression, symlink in ((b'1', 0, False), (bytes([4])*8, 0, False),
                (bytes(build.MAX_PAGES_INFO_BYTES+8), 0, False),
                (bytes(8), zipfile.ZIP_DEFLATED, False), (bytes(8), 0, True)):
            with self.subTest(size=len(bitmap), compression=compression, symlink=symlink):
                write_hap(self.signed, elf_fixture(stripped=True))
                with zipfile.ZipFile(self.signed, 'a') as archive:
                    member = zipfile.ZipInfo('.pages.info')
                    if symlink: member.external_attr = (0o120777 << 16)
                    archive.writestr(member, bitmap, compress_type=compression)
                with self.assertRaisesRegex(build.ProvenanceError, 'pages.info'):
                    build.attach_hap_provenance(self.make_manifest(), self.hap, self.signed)

    def test_signed_extra_hidden_files_or_directories_are_not_ignored(self):
        for name in ('.unexpected', '.pages.info.extra', 'nested/.pages.info', '.unexpected/'):
            with self.subTest(name=name):
                write_hap(self.signed, elf_fixture(stripped=True))
                with zipfile.ZipFile(self.signed, 'a') as archive:
                    archive.writestr('.pages.info', bytes(8))
                    archive.writestr(name, b'')
                with self.assertRaisesRegex(build.ProvenanceError, 'ZIP payload'):
                    build.attach_hap_provenance(self.make_manifest(), self.hap, self.signed)

    def test_signed_original_hidden_member_or_pages_info_cannot_change(self):
        for name in ('resources/.existing', '.pages.info'):
            with self.subTest(name=name):
                write_hap(self.hap, elf_fixture(stripped=True))
                write_hap(self.signed, elf_fixture(stripped=True))
                with zipfile.ZipFile(self.hap, 'a') as archive: archive.writestr(name, bytes(8))
                with zipfile.ZipFile(self.signed, 'a') as archive: archive.writestr(name, bytes([1])*8)
                with self.assertRaisesRegex(build.ProvenanceError, 'ZIP payload'):
                    build.attach_hap_provenance(self.make_manifest(), self.hap, self.signed)

    def test_signed_original_member_cannot_be_removed(self):
        with zipfile.ZipFile(self.signed, 'w') as archive:
            archive.writestr('module.json', json.dumps({'app': {'debug':False,'versionCode':1002009,'versionName':'1.2.9-synthetic'}}))
            archive.writestr('libs/arm64-v8a/libfixture.so', elf_fixture(stripped=True))
            archive.writestr('.pages.info', bytes(8))
        with self.assertRaisesRegex(build.ProvenanceError, 'ZIP payload'):
            build.attach_hap_provenance(self.make_manifest(), self.hap, self.signed)

    def test_wrong_hap_mode_or_native_library_set_is_rejected(self):
        write_hap(self.hap, elf_fixture(stripped=True), debug=True)
        with self.assertRaisesRegex(build.ProvenanceError, 'debug flag'):
            build.attach_hap_provenance(self.make_manifest(), self.hap)
        write_hap(self.hap, elf_fixture(stripped=True))
        manifest = self.make_manifest(); manifest['nativeLibraries']['libmissing.so'] = copy.deepcopy(manifest['nativeLibraries']['libfixture.so'])
        with self.assertRaisesRegex(build.ProvenanceError, 'library set'):
            build.attach_hap_provenance(manifest, self.hap)

    def test_bounded_reads_reject_oversized_stream(self):
        with self.assertRaises(build.ProvenanceError):
            build.stream_sha(io.BytesIO(b'12345'), 4)

    def test_main_keeps_original_arguments_and_accepts_post_packaging_hap(self):
        manifest = self.make_manifest()
        for extra in ([], ['--hap-path', str(self.hap), '--signed-hap-path', str(self.signed)]):
            with patch.object(build, 'ROOT', self.root), patch.object(build, 'build_manifest', return_value=copy.deepcopy(manifest)):
                self.assertEqual(build.main(['--hap-mode', 'release', '--native-profile', 'release', *extra]), 0)
            output = json.loads((self.root / 'dist/Paws-build-manifest.json').read_text())
            self.assertEqual('unsignedHapSha256' in output, bool(extra))
            self.assertFalse(output['wireCapture'])
            self.assertFalse(output['diagnosticPeerPinned'])


if __name__ == '__main__':
    unittest.main()
