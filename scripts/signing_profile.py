#!/usr/bin/env python3
"""Run one local signer with recoverable private profile injection."""
import base64
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import signal
import stat
import subprocess
import sys
import tempfile


SIGNALS = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)


class SigningInterrupted(BaseException):
    def __init__(self, signum):
        self.signum = signum


def _digest(data):
    return hashlib.sha256(data).hexdigest()


def _atomic_write(path, data, mode, temporary_dir=None):
    descriptor, temporary = tempfile.mkstemp(prefix='.' + path.name + '-', dir=temporary_dir or path.parent)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            os.fchmod(stream.fileno(), mode)
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


@contextmanager
def _blocked_signals():
    previous = signal.pthread_sigmask(signal.SIG_BLOCK, SIGNALS)
    try:
        yield
    finally:
        signal.pthread_sigmask(signal.SIG_SETMASK, previous)


class _Interrupts:
    def __init__(self):
        self.pending = None
        self.defer = False

    def interrupted(self, number, frame):
        if self.defer:
            self.pending = (number, frame)
            return
        # Let the first signal unwind; subsequent signals cannot interrupt the
        # owned process-group cleanup or restoration transaction.
        for item in SIGNALS:
            signal.signal(item, signal.SIG_IGN)
        raise SigningInterrupted(number)

    def __enter__(self):
        self.previous = {number: signal.getsignal(number) for number in SIGNALS}
        for number in SIGNALS:
            signal.signal(number, self.interrupted)
        return self

    def __exit__(self, *_):
        for number, handler in self.previous.items():
            signal.signal(number, handler)

    @contextmanager
    def spawning(self):
        # Deferral, not a pthread mask: the exec'ed signer must receive normal
        # TERM/HUP delivery rather than inherit the parent's blocked mask.
        self.defer = True
        try:
            yield
        finally:
            self.defer = False
            if self.pending is not None:
                number, frame = self.pending
                self.pending = None
                self.interrupted(number, frame)


class SigningProfile:
    def __init__(self, root: Path, source: Path | None = None):
        self.root = root
        self.project = root / 'build-profile.json5'
        self.private = root / '.ohos-sign'
        self.recovery = self.private / 'profile-recovery.json'
        self.lock = None
        configured = os.environ.get('PAWS_SIGNING_PROFILE')
        self.source = source or (Path(configured).expanduser() if configured else
                                 self.private / 'build-profile.json5')
        if not self.source.is_absolute():
            self.source = root / self.source

    def _acquire(self):
        self.private.mkdir(mode=0o700, exist_ok=True)
        descriptor = os.open(self.private / 'signing.lock', os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
        self.lock = os.fdopen(descriptor, 'rb+')
        os.fchmod(descriptor, 0o600)
        try:
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            self.lock.close()
            self.lock = None
            raise RuntimeError('Another local signing command is active') from None

    def _release(self):
        if self.lock is not None:
            fcntl.flock(self.lock.fileno(), fcntl.LOCK_UN)
            self.lock.close()
            self.lock = None

    def _restore(self):
        if not self.recovery.exists():
            return
        record = json.loads(self.recovery.read_bytes())
        original = base64.b64decode(record['originalBase64'], validate=True)
        mode = record['mode']
        if (record['format'] != 'Paws-signing-profile-recovery-v1'
                or _digest(original) != record['originalSha256']
                or type(mode) is not int or mode < 0 or mode > 0o777):
            raise RuntimeError('Invalid private signing recovery record')
        if self.project.is_symlink() or not self.project.is_file():
            raise RuntimeError('Project profile changed concurrently; refusing to overwrite it')
        current = self.project.read_bytes()
        current_hash = _digest(current)
        allowed_modes = set()
        if current_hash == record['temporarySha256']:
            allowed_modes.add(0o600)
        if current_hash == record['originalSha256']:
            allowed_modes.add(mode)
        if stat.S_IMODE(self.project.stat().st_mode) not in allowed_modes:
            raise RuntimeError('Project profile changed concurrently; refusing to overwrite it')
        _atomic_write(self.project, original, mode, self.private)
        self.recovery.unlink()

    def __enter__(self):
        self._acquire()
        try:
            with _blocked_signals():
                self._restore()
                if self.project.is_symlink():
                    raise ValueError('Project profile must be a regular file')
                self.original = self.project.read_bytes()
                self.mode = stat.S_IMODE(self.project.stat().st_mode)
                profile = json.loads(self.original)
                if self.source.is_file():
                    local = json.loads(self.source.read_bytes())['app']
                    profile['app']['signingConfigs'] = local['signingConfigs']
                    products = {item['name']: item for item in local['products']}
                    for product in profile['app']['products']:
                        selected = products.get(product['name'], {}).get('signingConfig')
                        if selected:
                            product['signingConfig'] = selected
                elif os.environ.get('PAWS_SIGNING_PROFILE') or self.source != self.private / 'build-profile.json5':
                    raise ValueError('The selected private signing profile is unavailable')

                product = next(item for item in profile['app']['products'] if item['name'] == 'default')
                selected = product.get('signingConfig') or 'default'
                signing = next(item for item in profile['app']['signingConfigs'] if item['name'] == selected)
                for key in ('storeFile', 'profile', 'certpath'):
                    path = Path(signing['material'][key]).expanduser()
                    if not path.is_absolute():
                        path = self.root / path
                    if not path.is_file():
                        raise ValueError(f'Existing signing material is unavailable: {key}')
                product['signingConfig'] = selected
                self.temporary = (json.dumps(profile, indent=2) + '\n').encode()
                record = {'format': 'Paws-signing-profile-recovery-v1',
                          'originalBase64': base64.b64encode(self.original).decode('ascii'),
                          'originalSha256': _digest(self.original),
                          'temporarySha256': _digest(self.temporary), 'mode': self.mode}
                # Journal is durable before the tracked project ever contains
                # private fields. Both files are atomically replaced, not torn.
                _atomic_write(self.recovery, (json.dumps(record) + '\n').encode(), 0o600)
                _atomic_write(self.project, self.temporary, 0o600, self.private)
            return self
        except BaseException:
            try:
                with _blocked_signals():
                    self._restore()
            finally:
                self._release()
            raise

    def __exit__(self, exc_type, exc_value, traceback):
        try:
            self.restore()
        finally:
            self._release()

    def restore(self):
        # Keep the mutex through artifact validation/publication even after
        # private fields have been removed from the tracked project.
        with _blocked_signals():
            self._restore()

    @staticmethod
    def _stop_group(process, grace):
        with _blocked_signals():
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=grace)
            except subprocess.TimeoutExpired:
                pass
            finally:
                # Include descendants even if the leader has already exited.
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()

    def run(self, command, *, timeout=None, check=False, termination_grace=2, **kwargs):
        if self.lock is None or not hasattr(self, 'interrupts'):
            raise RuntimeError('A signal-safe signing session must own the project lock')
        process = None
        try:
            # A signal during Popen must not strand a newly created group before
            # its Popen handle is assigned. The inherited flock also survives a
            # wrapper SIGKILL until the signer exits; another invocation refuses
            # to restore/reinject while that owned signer remains active.
            with self.interrupts.spawning():
                process = subprocess.Popen(command, start_new_session=True,
                    pass_fds=(self.lock.fileno(),), **kwargs)
            code = process.wait(timeout=timeout)
            self._stop_group(process, termination_grace)
        except BaseException:
            if process is not None:
                self._stop_group(process, termination_grace)
            raise
        if check and code:
            raise subprocess.CalledProcessError(code, command)
        return subprocess.CompletedProcess(command, code)


@contextmanager
def signing_session(root, source=None):
    with _Interrupts() as interrupts:
        with SigningProfile(root, source) as profile:
            profile.interrupts = interrupts
            yield profile


def run_signed_command(root, command, **kwargs):
    with signing_session(root) as profile:
        return profile.run(command, **kwargs)


def main():
    command = sys.argv[1:]
    if command[:1] == ['--']:
        command.pop(0)
    if not command:
        raise SystemExit('Usage: signing_profile.py -- <build command> [arguments...]')
    try:
        return run_signed_command(Path(__file__).resolve().parents[1], command).returncode
    except SigningInterrupted as interrupted:
        return 128 + interrupted.signum
    except (KeyError, StopIteration, OSError, ValueError, RuntimeError) as error:
        raise SystemExit(f'Local signing profile unavailable ({type(error).__name__})') from None


if __name__ == '__main__':
    raise SystemExit(main())
