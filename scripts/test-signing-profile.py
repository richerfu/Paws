#!/usr/bin/env python3
"""Verify private signing injection without invoking DevEco or reading secrets."""
import json
import os
from pathlib import Path
import signal
import shutil
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

from signing_profile import SigningProfile, run_signed_command


class SigningProfileTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.environment = patch.dict(os.environ, {}, clear=False)
        self.environment.start()
        self.addCleanup(self.environment.stop)
        os.environ.pop('PAWS_SIGNING_PROFILE', None)
        self.root = Path(self.directory.name)
        self.project = self.root / 'build-profile.json5'
        self.original = (json.dumps({'app': {
            'signingConfigs': [], 'products': [{'name': 'default', 'runtimeOS': 'HarmonyOS'}]},
            'modules': [{'name': 'current-runtime', 'srcPath': './target/ohos/current'}]}) + '\n').encode()
        self.project.write_bytes(self.original)
        self.project.chmod(0o640)
        for name in ('key.p12', 'profile.p7b', 'cert.cer'):
            (self.root / name).write_text('synthetic test material')
        self.local = self.root / '.ohos-sign/build-profile.json5'
        self.local.parent.mkdir()
        self.local.write_text(json.dumps({'app': {
            'signingConfigs': [{'name': 'device', 'material': {
                'storeFile': 'key.p12', 'profile': 'profile.p7b', 'certpath': 'cert.cer',
                'storePassword': 'synthetic-test-secret'}}],
            'products': [{'name': 'default', 'signingConfig': 'device'}]},
            'modules': [{'name': 'obsolete-local-module'}]}))

    def assert_restored(self):
        self.assertEqual(self.project.read_bytes(), self.original)
        self.assertEqual(stat.S_IMODE(self.project.stat().st_mode), 0o640)

    def test_only_signing_is_injected_and_original_bytes_mode_restore(self):
        local_before = self.local.read_bytes()
        with SigningProfile(self.root):
            current = json.loads(self.project.read_bytes())
            self.assertEqual(current['modules'][0]['name'], 'current-runtime')
            self.assertEqual(current['app']['products'][0]['runtimeOS'], 'HarmonyOS')
            self.assertEqual(current['app']['products'][0]['signingConfig'], 'device')
            self.assertEqual(len(current['app']['signingConfigs']), 1)
            self.assertEqual(stat.S_IMODE(self.project.stat().st_mode), 0o600)
        self.assert_restored()
        self.assertEqual(self.local.read_bytes(), local_before)

    def test_failing_build_still_restores_project(self):
        with self.assertRaisesRegex(RuntimeError, 'synthetic failure'):
            with SigningProfile(self.root):
                raise RuntimeError('synthetic failure')
        self.assert_restored()

    def test_concurrent_edit_is_not_overwritten(self):
        changed = b'{"app": "concurrent edit"}\n'
        with self.assertRaisesRegex(RuntimeError, 'changed concurrently'):
            with SigningProfile(self.root):
                self.project.write_bytes(changed)
        self.assertEqual(self.project.read_bytes(), changed)

    def test_missing_material_does_not_touch_project(self):
        (self.root / 'key.p12').unlink()
        with self.assertRaisesRegex(ValueError, 'storeFile'):
            with SigningProfile(self.root):
                self.fail('Invalid signing material was accepted')
        self.assert_restored()

    def test_explicit_missing_profile_does_not_fall_back(self):
        with self.assertRaisesRegex(ValueError, 'unavailable'):
            with SigningProfile(self.root, self.root / 'missing.json'):
                self.fail('Missing explicit profile was accepted')
        self.assert_restored()

    def test_relative_environment_override(self):
        os.environ['PAWS_SIGNING_PROFILE'] = '.ohos-sign/build-profile.json5'
        with SigningProfile(self.root):
            self.assertEqual(json.loads(self.project.read_bytes())['app']['products'][0]['signingConfig'], 'device')
        self.assert_restored()

    def test_existing_deveco_profile_remains_supported(self):
        profile = json.loads(self.original)
        profile['app'] = json.loads(self.local.read_bytes())['app']
        self.original = (json.dumps(profile) + '\n').encode()
        self.project.write_bytes(self.original)
        self.local.unlink()
        with SigningProfile(self.root):
            self.assertEqual(len(json.loads(self.project.read_bytes())['app']['signingConfigs']), 1)
        self.assert_restored()

    def test_failed_command_and_timeout_restore_and_reap_the_signer(self):
        with self.assertRaises(subprocess.CalledProcessError):
            run_signed_command(self.root, [sys.executable, '-c', 'raise SystemExit(7)'], check=True)
        self.assert_restored()
        with self.assertRaises(subprocess.TimeoutExpired):
            run_signed_command(self.root, [sys.executable, '-c', 'import time; time.sleep(30)'],
                               timeout=0.1, termination_grace=0.1)
        self.assert_restored()

    def test_package_cli_command_separator_is_compatible_and_secret_free(self):
        scripts = self.root / 'scripts'
        scripts.mkdir()
        launcher = scripts / 'signing_profile.py'
        shutil.copy2(Path(__file__).with_name('signing_profile.py'), launcher)
        result = subprocess.run([sys.executable, '-B', str(launcher), '--',
            sys.executable, '-c', 'print("public synthetic command")'],
            cwd=self.root, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5)
        self.assertEqual(result.returncode, 0, result.stderr.decode())
        self.assertEqual(result.stdout, b'public synthetic command\n')
        self.assertNotIn(b'synthetic-test-secret', result.stdout + result.stderr)
        self.assert_restored()

    def test_concurrent_permission_edit_is_not_overwritten(self):
        with self.assertRaisesRegex(RuntimeError, 'changed concurrently'):
            with SigningProfile(self.root):
                self.project.chmod(0o660)
        self.assertEqual(stat.S_IMODE(self.project.stat().st_mode), 0o660)
        self.assertTrue((self.local.parent / 'profile-recovery.json').exists())

    def launch(self, command, *, just_inject=False):
        scripts = str(Path(__file__).resolve().parent)
        if just_inject:
            body = '''from pathlib import Path
import sys,time
from signing_profile import SigningProfile
with SigningProfile(Path(sys.argv[1])):
    Path(sys.argv[1], 'ready').write_text('ready')
    time.sleep(60)
'''
        else:
            body = '''from pathlib import Path
import sys
from signing_profile import SigningInterrupted, run_signed_command
try:
    result = run_signed_command(Path(sys.argv[1]), sys.argv[2:], termination_grace=0.1)
    raise SystemExit(result.returncode)
except SigningInterrupted as interrupted:
    raise SystemExit(128 + interrupted.signum)
'''
        environment = dict(os.environ, PYTHONPATH=scripts)
        process = subprocess.Popen([sys.executable, '-B', '-c', body, str(self.root), *command],
                                   stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   env=environment, start_new_session=True)
        process.fixture_pids = []
        self.addCleanup(self.cleanup_process, process)
        return process

    def cleanup_process(self, process):
        if process.stdout.closed and process.stderr.closed:
            return
        # Capture each subtest's own children before the next subtest can
        # replace any marker. Successful communicate() already proves EOF on
        # the stdout/stderr inherited by both child and grandchild.
        pids = process.fixture_pids or [int((self.root / name).read_text())
            for name in ('child.pid', 'grandchild.pid') if (self.root / name).exists()]
        for pid in pids:
            try:
                os.kill(pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
        if process.poll() is None:
            os.killpg(process.pid, signal.SIGKILL)
        process.communicate(timeout=5)

    def wait_for(self, predicate):
        deadline = time.monotonic() + 5
        while not predicate():
            if time.monotonic() >= deadline:
                self.fail('Synthetic signer did not reach the expected lifecycle boundary')
            time.sleep(0.01)

    def test_term_and_hup_restore_bytes_mode_and_reap_owned_process_groups(self):
        for number in (signal.SIGTERM, signal.SIGHUP):
            with self.subTest(signal=number):
                for name in ('ready', 'child.pid', 'grandchild.pid'):
                    (self.root / name).unlink(missing_ok=True)
                grandchild = "import os,signal,time; from pathlib import Path; signal.signal(signal.SIGTERM,signal.SIG_IGN); Path('grandchild.pid').write_text(str(os.getpid())); time.sleep(60)"
                child = f'''import os,signal,subprocess,sys,time
from pathlib import Path
os.chdir(sys.argv[1])
signal.signal(signal.SIGTERM,signal.SIG_IGN)
subprocess.Popen([sys.executable, '-c', {grandchild!r}])
Path('child.pid').write_text(str(os.getpid()))
while not Path('grandchild.pid').exists(): time.sleep(.01)
Path('ready').write_text('ready')
time.sleep(60)
'''
                process = self.launch([sys.executable, '-c', child, str(self.root)])
                try:
                    self.wait_for(lambda: (self.root / 'ready').exists())
                    process.fixture_pids = [int((self.root / name).read_text()) for name in ('child.pid', 'grandchild.pid')]
                    for pid in process.fixture_pids:
                        os.kill(pid, 0)
                    self.assertEqual(stat.S_IMODE((self.local.parent / 'profile-recovery.json').stat().st_mode), 0o600)
                    process.send_signal(number)
                    # Both descendants deliberately retain the capture pipes
                    # and ignore TERM. EOF plus the waited wrapper receipt
                    # therefore requires the owned group cleanup to finish.
                    stdout, stderr = process.communicate(timeout=5)
                    self.assertEqual(process.returncode, 128 + number, stderr.decode())
                    self.assertNotIn(b'synthetic-test-secret', stdout + stderr)
                    self.assertTrue(process.stdout.closed and process.stderr.closed)
                    self.assert_restored()
                    self.assertFalse((self.local.parent / 'profile-recovery.json').exists())
                finally:
                    self.cleanup_process(process)

    def test_child_term_is_not_masked_and_previous_handlers_are_restored(self):
        child = '''import os,signal,sys,time
from pathlib import Path
os.chdir(sys.argv[1])
def stopped(*_):
    Path('terminated').write_text('TERM handled')
    raise SystemExit(0)
signal.signal(signal.SIGTERM, stopped)
Path('child.pid').write_text(str(os.getpid()))
Path('ready').write_text('ready')
time.sleep(60)
'''
        process = self.launch([sys.executable, '-c', child, str(self.root)])
        self.wait_for(lambda: (self.root / 'ready').exists())
        process.terminate()
        process.communicate(timeout=5)
        self.assertEqual((self.root / 'terminated').read_text(), 'TERM handled')
        self.assert_restored()
        previous = {number: signal.getsignal(number) for number in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)}
        run_signed_command(self.root, [sys.executable, '-c', 'pass'], check=True)
        self.assertEqual(previous, {number: signal.getsignal(number) for number in previous})

    def test_sigkill_retains_child_mutex_then_recovers_original_on_next_invocation(self):
        child = "import os,sys,time; from pathlib import Path; os.chdir(sys.argv[1]); Path('child.pid').write_text(str(os.getpid())); Path('ready').write_text('ready'); time.sleep(60)"
        process = self.launch([sys.executable, '-c', child, str(self.root)])
        self.wait_for(lambda: (self.root / 'ready').exists())
        process.kill()
        self.assertEqual(process.wait(timeout=5), -signal.SIGKILL)
        self.assertNotEqual(self.project.read_bytes(), self.original)
        with self.assertRaisesRegex(RuntimeError, 'active'):
            with SigningProfile(self.root):
                self.fail('An orphan active signer lost its mutex')
        os.kill(int((self.root / 'child.pid').read_text()), signal.SIGKILL)
        process.communicate(timeout=5)
        with SigningProfile(self.root) as recovered:
            self.assertEqual(recovered.original, self.original)
            self.assertEqual(recovered.mode, 0o640)
        self.assert_restored()

    def test_sigkill_recovery_refuses_to_overwrite_a_concurrent_project_edit(self):
        process = self.launch([], just_inject=True)
        self.wait_for(lambda: (self.root / 'ready').exists())
        process.kill()
        process.communicate(timeout=5)
        changed = b'{"concurrent": "public synthetic edit"}\n'
        self.project.write_bytes(changed)
        with self.assertRaisesRegex(RuntimeError, 'changed concurrently'):
            with SigningProfile(self.root):
                self.fail('Concurrent project edit was overwritten during recovery')
        self.assertEqual(self.project.read_bytes(), changed)
        self.assertTrue((self.local.parent / 'profile-recovery.json').exists())


if __name__ == '__main__':
    unittest.main()
