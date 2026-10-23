#!/usr/bin/env python3
"""Sign an already verified HAP without reattributing it to current sources."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import zipfile

from signing_profile import SigningInterrupted, _blocked_signals, signing_session

ROOT = Path(__file__).resolve().parents[1]


def _build_tools():
    spec = importlib.util.spec_from_file_location('paws_build_manifest', Path(__file__).with_name('write-build-manifest.py'))
    build = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(build)
    return build


def _verify_repacked_unsigned(build, original, generated):
    # SignHap may rewrite ZIP compression, order, timestamps and comments. Its
    # unsigned payload must still have exactly every original member and byte;
    # unlike a signed container, no added .pages.info is permitted here.
    if generated.stat().st_size > build.MAX_HAP_BYTES:
        raise build.ProvenanceError('Generated unsigned HAP exceeds its bounded size limit')
    with zipfile.ZipFile(original) as source, zipfile.ZipFile(generated) as result:
        original_members = build._zip_members(source)
        generated_members = build._zip_members(result)
        if (original_members.keys() != generated_members.keys()
                or build._zip_payload_hashes(source, original_members)
                != build._zip_payload_hashes(result, generated_members)):
            raise build.ProvenanceError('Generated unsigned HAP ZIP payload differs from the verified artifact')


def _publish(artifact, manifest_bytes, destination, manifest_path):
    """Atomic per-file replacement with rollback if either publication fails."""
    parent = destination.parent
    with tempfile.TemporaryDirectory(prefix='.paws-sign-publish-', dir=parent) as temporary:
        work = Path(temporary)
        targets = ((destination, artifact), (manifest_path, work / 'manifest.json'))
        (work / 'manifest.json').write_bytes(manifest_bytes)
        backups = []
        for index, (target, candidate) in enumerate(targets):
            if target.is_symlink():
                raise ValueError('Signing output must be a regular file')
            backup = work / f'previous-{index}' if target.exists() else None
            mode = stat.S_IMODE(target.stat().st_mode) if target.exists() else 0o644
            if backup is not None:
                # Replacement leaves this old inode intact for exact rollback.
                os.link(target, backup)
            candidate.chmod(mode)
            with candidate.open('rb') as stream:
                os.fsync(stream.fileno())
            backups.append(backup)
        replaced = []
        try:
            with _blocked_signals():
                for (target, candidate), backup in zip(targets, backups):
                    os.replace(candidate, target)
                    replaced.append((target, backup))
        except BaseException:
            with _blocked_signals():
                for target, backup in reversed(replaced):
                    if backup is None:
                        target.unlink(missing_ok=True)
                    else:
                        os.replace(backup, target)
            raise


def sign_existing(root, *, command=None, log_path=Path('/tmp/paws-current-sign.log')):
    build = _build_tools()
    manifest_path = root / 'dist/Paws-build-manifest.json'
    with signing_session(root) as signing:
        previous_bytes = manifest_path.read_bytes()
        previous = json.loads(previous_bytes)
        mode = previous['hapBuildMode']
        if mode not in ('debug', 'release'):
            raise ValueError('Verified HAP build mode is invalid')
        unsigned = root / f'dist/Paws-{mode}-unsigned.hap'
        generated = root / 'entry/build/default/outputs/default/entry-default-signed.hap'
        current_unsigned = generated.with_name('entry-default-unsigned.hap')
        destination = root / f'dist/Paws-{mode}-signed.hap'
        expected = previous['unsignedHapSha256']

        def verify_original_unsigned():
            if build.sha(unsigned) != expected:
                raise ValueError('Unsigned HAP differs from the previous verified build artifact')

        verify_original_unsigned()
        if build.sha(current_unsigned) != expected:
            raise ValueError('Unsigned HAP differs from the previous verified build artifact')
        # This validates the existing artifact against its recorded native/UI
        # fingerprints. Never call build_manifest against today's source tree.
        build.attach_hap_provenance(copy.deepcopy(previous), unsigned)
        destination_before = build.sha(destination) if destination.exists() else None
        generated.unlink(missing_ok=True)
        if command is None:
            studio = Path(os.environ['DEVECO_STUDIO_HOME'])
            command = [str(studio / 'tools/hvigor/bin/hvigorw'), 'default@SignHap', '--mode',
                       'module', '-p', 'module=entry@default', '-p', f'buildMode={mode}', '--no-daemon']
        descriptor = os.open(log_path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC | os.O_NOFOLLOW, 0o600)
        with os.fdopen(descriptor, 'w') as log:
            os.fchmod(log.fileno(), 0o600)
            signing.run(command, cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True, timeout=300)
        if not generated.is_file():
            raise ValueError('Signing did not produce a HAP')
        # Validate an owned immutable copy, not a mutable signer output. A bad
        # signer never touches the previous dist HAP or provenance manifest.
        descriptor, temporary = tempfile.mkstemp(prefix='.paws-signed-', suffix='.hap', dir=destination.parent)
        os.close(descriptor)
        candidate = Path(temporary)
        try:
            shutil.copyfile(generated, candidate)
            manifest = copy.deepcopy(previous)
            build.attach_hap_provenance(manifest, unsigned, candidate)
            verify_original_unsigned()
            _verify_repacked_unsigned(build, unsigned, current_unsigned)
            verify_original_unsigned()
            current_destination = build.sha(destination) if destination.exists() else None
            if manifest_path.read_bytes() != previous_bytes or current_destination != destination_before:
                raise ValueError('Verified signing outputs changed concurrently')
            signing.restore()
            _publish(candidate, (json.dumps(manifest, indent=2) + '\n').encode(), destination, manifest_path)
        finally:
            candidate.unlink(missing_ok=True)
    return {'signedHap': str(destination), 'sha256': build.sha(destination),
            'manifest': str(manifest_path), 'projectProfileRestored': True}


def main():
    print(json.dumps(sign_existing(ROOT)))


if __name__ == '__main__':
    try:
        main()
    except SigningInterrupted as interrupted:
        raise SystemExit(128 + interrupted.signum) from None
    except (KeyError, StopIteration, OSError, ValueError, RuntimeError, subprocess.SubprocessError, zipfile.BadZipFile) as error:
        raise SystemExit(f'Signing failed ({type(error).__name__}); private diagnostics: /tmp/paws-current-sign.log') from None
