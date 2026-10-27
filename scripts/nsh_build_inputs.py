"""Record the local NSH SDK, native bindings and application capability sources."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def ability_metadata(nsh):
    result = subprocess.run(['cargo', 'metadata', '--locked', '--offline', '--filter-platform',
                             'aarch64-unknown-linux-ohos', '--format-version', '1',
                             '--manifest-path', str(nsh / 'Cargo.toml')],
                            capture_output=True, text=True, timeout=120)
    if result.returncode:
        raise ValueError('Cannot resolve the locked OHOS Ability dependency; run cargo fetch '
                         '--locked --target aarch64-unknown-linux-ohos for the NSH workspace first: '
                         + result.stderr[-2000:].strip())
    return json.loads(result.stdout)


def linked_git_ability(nsh, dependency, explicit):
    """Select the exact Cargo.lock revision, never a nearby local checkout."""
    url = dependency.get('git')
    if not url:
        raise ValueError('The NSH Ability dependency must use a local path or a locked Git source')
    lock = tomllib.loads((nsh / 'Cargo.lock').read_text())
    sources = [package.get('source', '') for package in lock.get('package', [])
               if package.get('name') == 'openharmony-ability']
    sources = [source for source in sources
               if source.startswith(f'git+{url}?') or source.startswith(f'git+{url}#')]
    if len(sources) != 1:
        raise ValueError('NSH Cargo.lock must contain one matching Ability Git source')
    revision = sources[0].rsplit('#', 1)[-1]
    if len(revision) != 40 or any(character not in '0123456789abcdef' for character in revision):
        raise ValueError('The locked Ability Git source must include a complete commit SHA')
    packages = [package for package in ability_metadata(nsh)['packages']
                if package['name'] == 'openharmony-ability' and package.get('source') == sources[0]]
    if len(packages) != 1:
        raise ValueError('Cargo OHOS metadata does not match the locked Ability source')
    ability = Path(packages[0]['manifest_path']).resolve(strict=True).parents[2]
    if explicit and Path(explicit).resolve(strict=True) != ability:
        raise ValueError('NSH_ABILITY_ROOT must match the actual Cargo OHOS Ability checkout')
    head = subprocess.check_output(['git', '-c', 'core.fsmonitor=false', '-C', str(ability), 'rev-parse', 'HEAD'],
                                   text=True, timeout=30).strip()
    if head != revision:
        raise ValueError('The Cargo Ability checkout does not match its locked Git revision')
    return ability, sources[0]


def resolve_inputs():
    nsh = Path(os.environ.get('NSH_ROOT', ROOT.parent / 'nsh')).resolve(strict=True)
    manifest = tomllib.loads((nsh / 'Cargo.toml').read_text())
    dependency = manifest.get('workspace', {}).get('dependencies', {}).get('openharmony-ability')
    ability_path = dependency.get('path') if dependency else None
    explicit = os.environ.get('NSH_ABILITY_ROOT')
    linked_ability = (nsh / ability_path).resolve(strict=True).parents[1] if ability_path else None
    git_source = None
    if dependency is None:
        if explicit:
            ability = Path(explicit).resolve(strict=True)
        else:
            ability = Path.home().joinpath(
                '.cargo/git/checkouts/openharmony-ability-7e6401132ac36172/b655025'
            ).resolve(strict=True)
            git_source = (
                'git+https://github.com/harmony-contrib/openharmony-ability'
                '?rev=b65502500b820a570496f3798c931d12d3b71d39'
                '#b65502500b820a570496f3798c931d12d3b71d39'
            )
    elif not ability_path:
        ability, git_source = linked_git_ability(nsh, dependency, explicit)
    elif explicit:
        ability = Path(explicit).resolve(strict=True)
        if linked_ability is not None and ability != linked_ability:
            raise ValueError('NSH_ABILITY_ROOT must match the Ability path linked by the selected NSH workspace')
    elif ability_path:
        ability = linked_ability
    if not (ability / 'native_ability/oh-package.json5').is_file():
        raise ValueError('The selected Ability checkout has no native_ability package')

    def identity(path, cargo_checkout=False, sdk_inputs=False):
        git = ['git', '-c', 'core.fsmonitor=false', '-C', str(path)]
        revision = subprocess.check_output([*git, 'rev-parse', 'HEAD'], text=True, timeout=30).strip()
        status = subprocess.check_output([*git, 'status', '--porcelain=v1', '--untracked-files=normal'], timeout=30)
        dirty = any(line != b'?? .cargo-ok' or not cargo_checkout for line in status.splitlines())
        digest = hashlib.sha256(b'Paws-NSH-worktree-v1\0' + revision.encode() + b'\0')
        digest.update(subprocess.check_output([*git, 'diff', '--no-ext-diff', '--no-textconv', 'HEAD', '--binary'], timeout=30))
        untracked = subprocess.check_output([*git, 'ls-files', '-z', '--others', '--exclude-standard'], timeout=30)
        for relative in sorted(filter(None, untracked.split(b'\0'))):
            # Cargo writes this completion marker into its Git checkout. It is
            # neither source nor a package input and may be recreated by fetch.
            if cargo_checkout and relative == b'.cargo-ok':
                continue
            digest.update(len(relative).to_bytes(8, 'little') + relative)
            filename = path / os.fsdecode(relative)
            if filename.is_symlink():
                content = os.fsencode(os.readlink(filename))
                digest.update(b'L' + len(content).to_bytes(8, 'little') + content)
            else:
                digest.update(b'F' + filename.stat().st_size.to_bytes(8, 'little'))
                with filename.open('rb') as stream:
                    for chunk in iter(lambda: stream.read(1 << 20), b''):
                        digest.update(chunk)
        result = {'revision': revision, 'dirty': dirty, 'worktreeSha256': ''}
        if sdk_inputs:
            port = path / 'crates/nsh-platform-ohos'
            files = []
            generated = {'target', 'dist', 'build', 'oh_modules', 'node_modules', '.hvigor', '.cxx', '.nsh', '__pycache__'}
            for directory, children, names in os.walk(port):
                current = Path(directory)
                if current in (port, port / 'arkts'):
                    children[:] = [name for name in children if name not in generated]
                children[:] = [name for name in children if name != '.git']
                files.extend(current / name for name in names if name != '.DS_Store')
            sdk_digest = hashlib.sha256(b'Paws-NSH-OHOS-source-v1\0')
            ignored = set(subprocess.check_output([*git, 'ls-files', '-z', '--others', '--ignored',
                '--exclude-standard', '--', 'crates/nsh-platform-ohos'], timeout=30).split(b'\0'))
            for filename in sorted(files):
                relative = os.fsencode(filename.relative_to(path))
                sdk_digest.update(relative + b'\0')
                if filename.is_symlink():
                    sdk_digest.update(b'L' + os.fsencode(os.readlink(filename)))
                else:
                    with filename.open('rb') as stream:
                        for chunk in iter(lambda: stream.read(1 << 20), b''):
                            sdk_digest.update(chunk)
                sdk_digest.update(b'\0')
                if relative in ignored:
                    result['dirty'] = True
            result.update(sdkSourceSha256=sdk_digest.hexdigest(), sdkSourceFileCount=len(files))
            digest.update(b'\0NSH-OHOS-source-inputs\0' + sdk_digest.digest())
        result['worktreeSha256'] = digest.hexdigest()
        return result

    nsh_identity, ability_identity = identity(nsh, sdk_inputs=True), identity(ability, cargo_checkout=bool(git_source))
    result = {'nshRoot': str(nsh), 'abilityRoot': str(ability), 'nsh': nsh_identity, 'ability': ability_identity,
              'nshRevision': nsh_identity['revision'] + ('+dirty' if nsh_identity['dirty'] else '')}
    if dependency is None:
        result['hostModel'] = 'native-ui-context'
    # Development SDKs may override released bindings with local sources. Their
    # contents are build inputs too, even when NSH's own HEAD has not changed.
    path_dependencies = {}
    for packages in manifest.get('patch', {}).values():
        for name, options in packages.items():
            if not isinstance(options, dict) or 'path' not in options:
                continue
            package = (nsh / options['path']).resolve(strict=True)
            root = Path(subprocess.check_output(
                ['git', '-c', 'core.fsmonitor=false', '-C', str(package), 'rev-parse', '--show-toplevel'],
                text=True, timeout=30).strip()).resolve(strict=True)
            key = str(root)
            if key not in path_dependencies:
                path_dependencies[key] = {'root': key, 'identity': identity(root), 'packages': {}}
            path_dependencies[key]['packages'][name] = str(package)
    if path_dependencies:
        result['pathDependencies'] = [path_dependencies[key] for key in sorted(path_dependencies)]
    if git_source:
        result['abilityDependencySource'] = git_source
    return result


def verify_source_identity(recorded, current):
    """Reject missing fingerprints and any changes to either checked-out source tree."""
    for name in ('nsh', 'ability'):
        if not recorded.get(name, {}).get('worktreeSha256'):
            raise ValueError('NSH SDK has no recorded worktree digest; rebuild before reuse')
    if {key: recorded.get(key) for key in current} != current:
        raise ValueError('NSH SDK source roots, revisions or worktree content changed; rebuild before reuse')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--field', choices=('nshRoot', 'abilityRoot', 'nshRevision'))
    args = parser.parse_args()
    values = resolve_inputs()
    print(values[args.field] if args.field else json.dumps(values, indent=2))
