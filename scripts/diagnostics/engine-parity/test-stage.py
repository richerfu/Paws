#!/usr/bin/env python3
"""Filesystem/provenance tests only. Synthetic ZIP bytes are not an engine test."""
import hashlib
import importlib.util
import json
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch
import zipfile

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location('engine_parity_stage', HERE / 'stage.py')
stage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(stage)
NONCE = '1234567890abcdef1234567890abcdef'


class StageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='paws-engine-parity-stage-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name) / 'project'
        self.root.mkdir()
        for relative in stage.COPY_DIRS:
            (self.root / relative).mkdir(parents=True, exist_ok=True)
        for relative in stage.COPY_FILES:
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('{}\n')
        # Execute exact span changes against actual current host code, not a
        # reimplementation of host initialization in the test.
        for relative in (stage.ENTRY, stage.VPN, stage.GATEWAY, stage.AUTOMATION, stage.BACKUP, stage.FORM, stage.FORM_UPDATE, stage.BACKUP_CONFIG):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(stage.ROOT / relative, path)
        self.write('AppScope/app.json5', {'app': {'bundleName': 'com.richerfu.paws'}})
        self.write('build-profile.json5', {'app': {'signingConfigs': [{'name': 'private', 'password': 'not-copied'}],
            'products': [{'name': 'default', 'signingConfig': 'private'}]}})
        dep = self.root / 'target/ohos/arkts-deps/nsh/oh-package.json5'
        dep.parent.mkdir(parents=True)
        dep.write_text('{}\n')
        self.write('target/ohos/arkts-deps/ability/oh-package.json5', {'name': '@ohos-rs/ability'})
        self.write('target/ohos/arkts-source-inputs.json', {'packages': [{
            'name': '@nsh/ohos', 'stagedRoot': 'target/ohos/arkts-deps/nsh',
            'files': {'oh-package.json5': {'stagedSha256': stage.file_hash(dep)}}}]})
        self.hap = Path(self.temp.name) / 'normal.hap'
        self.payload = {'module.json': json.dumps({'app': {'bundleName': 'com.richerfu.paws', 'debug': True}}).encode(),
                        'resources/rawfile/paws.bundle.js': b'actual-ui-artifact-placeholder-for-file-test',
                        'resources/rawfile/geodata/fixture.dat': b'geodata'}
        for name in ('libpaws_ui.so', 'libnsh_platform_ohos.so', 'libclash.so', 'libsingbox.so', 'libnative_ability.so'):
            self.payload[f'libs/arm64-v8a/{name}'] = f'not-an-ELF:{name}'.encode()
        self.pack(self.hap)
        self.manifest = {'hapBuildMode': 'debug', 'diagnosticBypass': False, 'wireCapture': False,
            'signedHapSha256': stage.file_hash(self.hap), 'buildFingerprint': 'normal-test-build',
            'nativeLibraries': {Path(name).name: {'packagedSha256': stage.digest(data),
                'loadSegmentsMatchCompiledLibrary': True} for name, data in self.payload.items() if name.endswith('.so')},
            'uiRuntime': {'bundle': {'packagedSha256': stage.digest(self.payload['resources/rawfile/paws.bundle.js']),
                'matchesCompiledBundle': True}, 'arktsSources': {
                    'manifestSha256': stage.file_hash(self.root / 'target/ohos/arkts-source-inputs.json')}}}
        self.manifest_path = Path(self.temp.name) / 'manifest.json'
        self.save_manifest()
        self.out = self.root / 'target/engine-parity' / NONCE

    def write(self, relative, value):
        path = self.root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(json.dumps(value))

    def pack(self, path):
        with zipfile.ZipFile(path, 'w') as archive:
            for name, data in self.payload.items():
                archive.writestr(name, data)

    def save_manifest(self):
        self.manifest_path.write_text(json.dumps(self.manifest))

    def prepare(self, **options):
        return stage.prepare(self.root, self.out, self.hap, self.manifest_path, NONCE, **options)

    def test_stage_isolated_and_preserves_real_gateway_and_bundle(self):
        before = stage.inventory(self.root, (*stage.COPY_DIRS, *stage.COPY_FILES, 'build-profile.json5'))
        result = self.prepare()
        self.assertEqual(before, stage.inventory(self.root, (*stage.COPY_DIRS, *stage.COPY_FILES, 'build-profile.json5')))
        self.assertEqual(result['bundleName'], 'com.richerfu.paws')
        self.assertEqual(result['status'], 'staged-not-built-not-signed-not-installed')
        self.assertIn('this.hostServices = installPawsHost(this.context);', (self.out / stage.ENTRY).read_text())
        self.assertNotIn('personalCore', (self.out / stage.ENTRY).read_text())
        self.assertEqual((self.out / 'entry/src/main/resources/rawfile/paws.bundle.js').read_bytes(),
                         self.payload['resources/rawfile/paws.bundle.js'])
        self.assertEqual(stage.verify(self.out, self.hap)['hapPayloadVerified'], True)
        self.assertEqual(len(result['stageDifferences']), 13)
        self.assertEqual(result['uiErrorProbe']['enabled'], False)
        self.assertEqual(result['uiErrorProbe']['fixedPublicErrors'], {})
        self.assertFalse((self.out / 'entry/src/main/ets/diagnostics/EngineParityUiError.ets').exists())
        for relative in (stage.GATEWAY, stage.AUTOMATION, stage.ENTRY):
            self.assertNotIn('EngineParityUiError', (self.out / relative).read_text())
        native = json.loads((self.out / 'entry/build-profile.json5').read_text())['buildOption']['nativeLib']
        self.assertEqual(native['filter']['select'], [{'package': '@ohos-rs/ability', 'excludePattern': ['**/libnative_ability.so']}])
        self.assertEqual(native['debugSymbol']['strip'], False)

    def test_optional_ui_error_probe_uses_real_request_catch_and_exact_normal_payload(self):
        result = self.prepare(ui_error_probe=True)
        self.assertEqual(len(result['stageDifferences']), 14)
        self.assertEqual(result['uiErrorProbe']['enabled'], True)
        self.assertEqual(result['uiErrorProbe']['ttlMs'], 10000)
        self.assertEqual(result['uiErrorProbe']['automaticCoreCalls'], False)
        expected = json.loads((stage.TEMPLATES / 'ui-errors.json').read_text())
        helper = (self.out / 'entry/src/main/ets/diagnostics/EngineParityUiError.ets').read_text()
        for key, value in expected.items():
            self.assertIn(json.dumps(value, ensure_ascii=False), helper)
            self.assertEqual(result['uiErrorProbe']['fixedPublicErrors'][key]['sha256'], stage.digest(value.encode()))
        self.assertEqual(result['uiErrorProbe']['fixedPublicErrors']['multiline']['utf8Bytes'], 2134)
        gateway = (self.out / stage.GATEWAY).read_text()
        self.assertIn("    try {\n      throwEngineParityUiError(request);\n      if (subscribe)", gateway)
        self.assertIn('error: describeUserError(error)', gateway)
        self.assertIn('cancelEngineParityUiError();', (self.out / stage.ENTRY).read_text())
        automation = (self.out / stage.AUTOMATION).read_text()
        self.assertLess(automation.index('return runEngineParityUiError'), automation.index('return runEngineParityNetwork'))
        self.assertEqual(stage.verify(self.out, self.hap)['hapPayloadVerified'], True)

    def test_optional_ui_error_hook_rejects_changed_request_try_span(self):
        path = self.root / stage.GATEWAY
        path.write_text(path.read_text().replace("    const query = request.command === 'state.query';\n    try {\n",
                                                "    const query = request.command === 'state.query';\n    try { /* changed */\n"))
        with self.assertRaisesRegex(ValueError, 'optional one-shot failure'):
            self.prepare(ui_error_probe=True)
        self.assertFalse((self.out / 'engine-parity-manifest.json').exists())

    def test_ui_error_option_is_not_a_truthy_runtime_override(self):
        for value in ('true', 'false', 1, None):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, 'explicit boolean'):
                self.prepare(ui_error_probe=value)
        self.assertFalse(self.out.exists())

    def test_no_signing_secrets_copied(self):
        self.prepare()
        profile = (self.out / 'build-profile.json5').read_text()
        self.assertNotIn('not-copied', profile)
        self.assertEqual(json.loads(profile)['app']['signingConfigs'], [])
        self.assertNotIn('signingConfig', json.loads(profile)['app']['products'][0])

    def test_nonce_rejects_paths_and_noncanonical_identity(self):
        for nonce in ('', '../personal', 'a' * 31, 'A' * 32, 'a' * 33, 'a' * 32 + '\n'):
            with self.subTest(nonce=nonce), self.assertRaisesRegex(ValueError, 'Nonce'):
                stage.prepare(self.root, self.out, self.hap, self.manifest_path, nonce)
        self.assertFalse(self.out.exists())

    def test_existing_evidence_is_never_deleted(self):
        self.out.mkdir(parents=True)
        (self.out / 'keep').write_text('previous evidence')
        with self.assertRaisesRegex(ValueError, 'already exists'):
            self.prepare()
        self.assertEqual((self.out / 'keep').read_text(), 'previous evidence')

    def test_reject_wrong_candidate_hap(self):
        self.hap.write_bytes(b'changed')
        with self.assertRaisesRegex(ValueError, 'does not match'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_reject_diagnostic_or_release_candidate(self):
        for key, value in (('diagnosticBypass', True), ('wireCapture', True), ('hapBuildMode', 'release')):
            original = self.manifest[key]
            self.manifest[key] = value
            self.save_manifest()
            with self.assertRaisesRegex(ValueError, 'normal debug'):
                self.prepare()
            self.manifest[key] = original
        self.assertFalse(self.out.exists())

    def test_reject_native_or_bundle_hash_mismatch(self):
        for proof in (self.manifest['nativeLibraries']['libclash.so'], self.manifest['uiRuntime']['bundle']):
            original = proof['packagedSha256']
            proof['packagedSha256'] = '0' * 64
            self.save_manifest()
            with self.assertRaisesRegex(ValueError, 'proof mismatch'):
                self.prepare()
            proof['packagedSha256'] = original

    def test_reject_incomplete_native_inventory(self):
        self.manifest['nativeLibraries']['missing.so'] = {'packagedSha256': '0' * 64}
        self.save_manifest()
        with self.assertRaisesRegex(ValueError, 'inventory is incomplete'):
            self.prepare()

    def test_reject_har_attestation_drift(self):
        path = self.root / 'target/ohos/arkts-deps/nsh/oh-package.json5'
        path.write_text('{"changed":true}')
        with self.assertRaisesRegex(ValueError, 'HAR inputs changed'):
            self.prepare()

    def test_known_ohpm_derived_lock_is_bound_without_weakening_source_attestation(self):
        lock = 'target/ohos/arkts-deps/nsh/oh-package-lock.json5'
        self.write(lock, {'lockfileVersion': 3})
        result = self.prepare()
        self.assertEqual(result['derivedHarInputs'][lock]['sha256'], stage.file_hash(self.root / lock))
        self.assertEqual((self.root / lock).read_bytes(), (self.out / lock).read_bytes())
        stage.verify(self.out)
        (self.out / lock).write_text('changed')
        with self.assertRaisesRegex(ValueError, 'Staged inputs changed'):
            stage.verify(self.out)

    def test_other_unattested_har_files_are_rejected_even_beside_known_lock(self):
        self.write('target/ohos/arkts-deps/nsh/oh-package-lock.json5', {'lockfileVersion': 3})
        self.write('target/ohos/arkts-deps/nsh/unexpected-source.ets', {'extra': True})
        with self.assertRaisesRegex(ValueError, 'HAR inputs changed'):
            self.prepare()
        self.assertFalse(self.out.exists())

    def test_source_span_failure_and_new_file_writer_fail_closed(self):
        path = self.root / stage.ENTRY
        original = path.read_text()
        path.write_text(original.replace('  private configureNativeHome(): void {',
            '  private configureNativeHome(): void {\n    consume(this.context.filesDir);'))
        with self.assertRaisesRegex(ValueError, 'Unisolated'):
            self.prepare()

    def test_unknown_initialization_shape_is_not_patched_by_guess(self):
        path = self.root / stage.ENTRY
        path.write_text(path.read_text().replace('pawsUi.configureAppHome(', 'differentConfigure('))
        with self.assertRaisesRegex(ValueError, 'expected 1 span'):
            self.prepare()

    def test_verify_rejects_stage_tampering(self):
        self.prepare()
        (self.out / stage.ENTRY).write_text((self.out / stage.ENTRY).read_text() + '\n// changed\n')
        with self.assertRaisesRegex(ValueError, 'Staged inputs changed'):
            stage.verify(self.out)

    def test_verify_rejects_packaged_native_change(self):
        self.prepare()
        self.payload['libs/arm64-v8a/libclash.so'] = b'wrong engine'
        changed = Path(self.temp.name) / 'wrong.hap'
        self.pack(changed)
        with self.assertRaisesRegex(ValueError, 'differs from normal candidate'):
            stage.verify(self.out, changed)

    def test_source_fence_rejects_concurrent_host_edit(self):
        original = stage.patch_host
        def mutate_then_edit(*args):
            result = original(*args)
            path = self.root / stage.ENTRY
            path.write_text(path.read_text() + '\n// concurrent edit\n')
            return result
        with patch.object(stage, 'patch_host', mutate_then_edit), self.assertRaisesRegex(ValueError, 'changed while staging'):
            self.prepare()
        self.assertFalse((self.out / 'engine-parity-manifest.json').exists())


if __name__ == '__main__':
    unittest.main()
