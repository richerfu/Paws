"""Resolve the checksum-pinned go-patch compiler without modifying Go sources."""
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[1]
MANIFEST = json.loads((ROOT / 'scripts/go-ohos-toolchain.json').read_text())


def host():
    architecture = {'aarch64': 'arm64', 'x86_64': 'amd64'}.get(platform.machine(), platform.machine())
    return platform.system().lower() + '-' + architecture


def sha256(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def validate(root):
    """Reject stock Go, old source distributions and mismatched host releases."""
    info = json.loads((root / 'BUILD-INFO.json').read_text())
    if (info.get('upstream') != MANIFEST['version']
            or info.get('patch_sha256') != MANIFEST['patchSha256']
            or info.get('target', {}).get('name') != host()):
        raise ValueError('Expected the pinned go-patch release for ' + host())
    environment = dict(os.environ, GOTOOLCHAIN='local', GOROOT=str(root))
    command = [str(root / 'bin/go')]
    version = subprocess.check_output([*command, 'env', 'GOVERSION'], env=environment, text=True).strip()
    targets = subprocess.check_output([*command, 'tool', 'dist', 'list'], env=environment, text=True).splitlines()
    if version != MANIFEST['version'] or 'openharmony/arm64' not in targets:
        raise ValueError('go-patch compiler version or OpenHarmony target is unavailable')
    for wrapper in ('cc-arm64', 'cxx-arm64'):
        if not os.access(root / 'ohos-scripts' / wrapper, os.X_OK):
            raise ValueError('Missing go-patch SDK compiler wrapper: ' + wrapper)
    return info


def prepare(selected=None):
    if selected:
        root = Path(selected).resolve(strict=True)
        validate(root)
        return root
    key = host()
    if key not in MANIFEST['assets']:
        raise ValueError('No pinned go-patch distribution for ' + key + '; set GO_OHOS_ROOT')
    asset = MANIFEST['assets'][key]
    directory = ROOT / 'target/toolchains'
    destination = directory / ('go-patch-' + MANIFEST['version'][2:] + '-' + key)
    if destination.exists():
        validate(destination)
        return destination
    directory.mkdir(parents=True, exist_ok=True)
    downloads = directory / 'downloads'
    downloads.mkdir(exist_ok=True)
    archive = downloads / asset['url'].rsplit('/', 1)[-1]
    if not archive.exists() or sha256(archive) != asset['sha256']:
        with tempfile.NamedTemporaryFile(dir=downloads, suffix='.part') as temporary:
            subprocess.run(['curl', '-fL', '--retry', '3', '--output', temporary.name, asset['url']],
                           check=True, stdout=sys.stderr)
            if sha256(Path(temporary.name)) != asset['sha256']:
                raise ValueError('go-patch download SHA256 mismatch')
            archive.write_bytes(Path(temporary.name).read_bytes())
    with tempfile.TemporaryDirectory(dir=directory, prefix='go-patch-extract-') as temporary:
        with tarfile.open(archive) as bundle:
            bundle.extractall(temporary, filter='data')
        extracted = Path(temporary) / 'go'
        validate(extracted)
        extracted.rename(destination)
    return destination


def compiler_identity():
    root = prepare(os.environ.get('GO_COMPILER_ROOT') or os.environ.get('GO_OHOS_ROOT'))
    return {'release': MANIFEST['repository'] + '/releases/tag/v' + MANIFEST['version'][2:],
            'buildInfo': validate(root), 'goSha256': sha256(root / 'bin/go')}


if __name__ == '__main__':
    print(prepare(sys.argv[1] if len(sys.argv) > 1 else os.environ.get('GO_OHOS_ROOT')))
