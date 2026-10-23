#!/usr/bin/env python3
"""Prepare or verify a same-bundle isolated engine QA project; never build or install."""
import argparse
import hashlib
import ipaddress
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[3]
TEMPLATES = Path(__file__).resolve().parent / 'templates'
NONCE = re.compile(r'[a-f0-9]{32}\Z')
GENERATED = {'build', '.hvigor', '.cxx', 'oh_modules', 'node_modules', '__pycache__',
             '.DS_Store', 'BuildProfile.ets'}
COPY_DIRS = ('AppScope', 'entry/src', 'hvigor', 'target/ohos/arkts-deps')
COPY_FILES = ('hvigorfile.ts', 'oh-package.json5', 'oh-package-lock.json5',
              'entry/build-profile.json5', 'entry/hvigorfile.ts', 'entry/oh-package.json5',
              'entry/oh-package-lock.json5', 'entry/obfuscation-rules.txt',
              'target/ohos/nsh-source-inputs.json', 'target/ohos/arkts-source-inputs.json')
ENTRY = 'entry/src/main/ets/entryability/EntryAbility.ets'
VPN = 'entry/src/main/ets/vpnability/PawsVpnExtensionAbility.ets'
BACKUP = 'entry/src/main/ets/entrybackupability/EntryBackupAbility.ets'
FORM = 'entry/src/main/ets/widget/TinyPawsFormAbility.ets'
FORM_UPDATE = 'entry/src/main/ets/widget/TinyPawsFormUpdate.ets'
BACKUP_CONFIG = 'entry/src/main/resources/base/profile/backup_config.json'
GATEWAY = 'entry/src/main/ets/components/PawsCoreGateway.ets'
AUTOMATION = 'entry/src/main/ets/automation/DebugAutomation.ets'


def digest(data):
    return hashlib.sha256(data).hexdigest()


def file_hash(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def inventory(root, relative_paths):
    """Only source/build inputs, never private application data or credentials."""
    records = {}
    for relative in relative_paths:
        source = root / relative
        if not source.exists():
            raise ValueError(f'Missing required candidate input: {relative}')
        paths = [source] if source.is_file() else sorted(source.rglob('*'))
        for path in paths:
            name = path.relative_to(root)
            if any(part in GENERATED for part in name.parts):
                continue
            if path.is_symlink():
                raise ValueError(f'Symlink is not an attested stage input: {name}')
            if path.is_file():
                records[name.as_posix()] = {'bytes': path.stat().st_size, 'sha256': file_hash(path)}
    return dict(sorted(records.items()))


def load_config(path):
    try:
        return json.loads(path.read_text())
    except json.JSONDecodeError:
        studio = Path(os.environ.get('DEVECO_STUDIO_HOME', '/Applications/DevEco-Studio.app/Contents'))
        # Same JSON5 parser as prepare-nsh-ability.py; no package installation.
        parser = studio / 'tools/hvigor/hvigor-ohos-plugin/node_modules/json5'
        return json.loads(subprocess.check_output([str(studio / 'tools/node/bin/node'), '-e',
            'const fs=require("fs"),j=require(process.argv[1]);'
            'process.stdout.write(JSON.stringify(j.parse(fs.readFileSync(process.argv[2],"utf8"))))',
            str(parser), str(path)], text=True, timeout=30))


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + '\n')


def replace_exact(text, before, after, label, count=1):
    actual = text.count(before)
    if actual != count:
        raise ValueError(f'Host source changed at {label}: expected {count} span(s), found {actual}; review final source')
    return text.replace(before, after)


def patch_host(stage, nonce, fixture_host='127.0.0.1', http_port=18746, dns_port=18753, udp_port=18754, ui_error_probe=False):
    """Span edits are stage-only, count-checked, and fully listed in the manifest."""
    changes = []

    def mutate(relative, operation, reason):
        path = stage / relative
        before = path.read_bytes()
        after = operation(before.decode()).encode()
        path.write_bytes(after)
        changes.append({'path': relative, 'reason': reason, 'beforeSha256': digest(before),
                        'afterSha256': digest(after)})

    def entry(text):
        text = replace_exact(text, '  public onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {',
            '  public onCreate(want: Want, launchParam: AbilityConstant.LaunchParam): void {\n'
            '    assertEngineParityDebug(this.context.applicationInfo.debug === true);', 'Entry debug gate')
        text = replace_exact(text, 'pawsUi.configureAppHome(`${this.context.filesDir}/paws`);',
            'startEngineParityTrace(this.context.filesDir);\n    pawsUi.configureAppHome(engineParityHome(this.context.filesDir));', 'Entry native home')
        text = replace_exact(text, 'this.systemExitRecorder.start(this.context.filesDir,',
            'this.systemExitRecorder.start(engineParityBase(this.context.filesDir),', 'Entry exit recorder')
        text = replace_exact(text, '    this.hostServices?.dispose();',
            '    cancelEngineParityNetwork();\n    closeEngineParityTrace();\n    this.hostServices?.dispose();', 'Entry QA diagnostics disposal')
        if ui_error_probe:
            text = replace_exact(text, '    cancelEngineParityNetwork();',
                '    cancelEngineParityUiError();\n    cancelEngineParityNetwork();', 'Entry UI error probe disposal')
            text = "import { cancelEngineParityUiError } from '../diagnostics/EngineParityUiError';\n" + text
        return ("import { assertEngineParityDebug, engineParityBase, engineParityHome } from '../diagnostics/EngineParityPaths';\n"
                "import { startEngineParityTrace, closeEngineParityTrace } from '../diagnostics/EngineParityTrace';\n"
                "import { cancelEngineParityNetwork } from '../diagnostics/EngineParityNetwork';\n" + text)

    def extension(text):
        text = replace_exact(text, '  public onCreate(want: Want): void {',
            '  public onCreate(want: Want): void {\n'
            '    assertEngineParityDebug(this.context.applicationInfo.debug === true);', 'VPN debug gate')
        text = replace_exact(text, 'pawsUi.configureAppHome(`${this.context.filesDir}/paws`);',
            'pawsUi.configureAppHome(engineParityHome(this.context.filesDir));', 'VPN native home')
        for expression in ('this.journal.open', 'this.systemExitRecorder.start'):
            text = replace_exact(text, expression + '(this.context.filesDir',
                expression + '(engineParityBase(this.context.filesDir)', expression)
        # Three current producers. A new producer requires review instead of
        # silently receiving the personal files directory.
        text = replace_exact(text, 'updateTinyPawsForms(this.context.filesDir,',
            'updateTinyPawsForms(engineParityBase(this.context.filesDir),', 'VPN widget paths', count=3)
        return ("import { assertEngineParityDebug, engineParityBase, engineParityHome } from '../diagnostics/EngineParityPaths';\n" + text)

    mutate(ENTRY, entry, 'debug-only; configure isolated Core before any host initialization; isolated exit metadata')
    mutate(VPN, extension, 'same isolated Core/journal/exit/widget base in the separate VPN process')
    def gateway(text):
        for name in ('UiRequest', 'UiResult', 'DomainRevision', 'StateBarrier', 'StateChanged'):
            text = replace_exact(text, f'\ninterface {name} ', f'\nexport interface {name} ',
                                 f'Gateway original nominal type {name}')
        text = replace_exact(text,
            "      if (!Number.isSafeInteger(request.id) || request.id < 0 || typeof request.command !== 'string') { continue; }",
            "      if (!Number.isSafeInteger(request.id) || request.id < 0 || typeof request.command !== 'string') { continue; }\n      traceParityReceived(request);",
            'Gateway accepted request')
        text = replace_exact(text, "    this.emit('result', result);\n",
            "    traceParityResult(request, result, queryChars);\n    this.emit('result', result);\n", 'Gateway real result')
        text = replace_exact(text, '  private emitChanged(change: StateChanged): void {',
            '  private emitChanged(change: StateChanged): void {\n    traceParityChanged(change);', 'Gateway real observation')
        if ui_error_probe:
            text = replace_exact(text, "    const query = request.command === 'state.query';\n    try {\n",
                "    const query = request.command === 'state.query';\n    try {\n      throwEngineParityUiError(request);\n",
                'Gateway optional one-shot failure inside the real request catch')
            text = "import { throwEngineParityUiError } from '../diagnostics/EngineParityUiError';\n" + text
        return "import { traceParityReceived, traceParityResult, traceParityChanged } from '../diagnostics/EngineParityTrace';\n" + text
    mutate(GATEWAY, gateway, 'stage-only bounded metadata trace using original nominal interfaces; no payload copy, no extra Core query')
    def automation(text):
        imports = "import { runEngineParityNetwork } from '../diagnostics/EngineParityNetwork';\n"
        gate = ''
        if ui_error_probe:
            imports += "import { runEngineParityUiError } from '../diagnostics/EngineParityUiError';\n"
            gate = ("  if (want.parameters?.['pawsEngineParityProbe'] === 'ui-error-multiline' ||\n"
                    "    want.parameters?.['pawsEngineParityProbe'] === 'ui-error-unbroken') {\n"
                    "    return runEngineParityUiError(want, applicationDebug);\n  }\n")
        return imports + replace_exact(text, '  if (!applicationDebug) {\n', gate +
            "  if (want.parameters?.['pawsEngineParityProbe'] !== undefined) {\n    return runEngineParityNetwork(want, applicationDebug);\n  }\n  if (!applicationDebug) {\n",
            'debug automation explicit diagnostic probe gate')
    mutate(AUTOMATION, automation, 'optional nonce-gated diagnostics; no automatic query, configuration or VPN mutation')
    mutate(BACKUP, lambda text: replace_exact(text,
        "const PAWS_BACKUP_ROOT = '/data/storage/el2/base/files/paws';",
        f"const PAWS_BACKUP_ROOT = '/data/storage/el2/base/files/engine-parity/{nonce}/paws';",
        'backup log root'), 'backup extension root label isolated; OS backup disabled separately')
    mutate(BACKUP_CONFIG, lambda _: json.dumps({'allowToBackupRestore': False, 'includes': [],
        'excludes': []}, indent=2) + '\n', 'disable OS backup/restore of personal files in QA stage')
    for relative, name in ((FORM, 'TinyPawsFormAbility.ets'), (FORM_UPDATE, 'TinyPawsFormUpdate.ets')):
        mutate(relative, lambda _, name=name: (TEMPLATES / name).read_text(),
               'stage-only widget operations disabled; no personal form-ID read or OS widget updates')
    target = stage / 'entry/src/main/ets/diagnostics/EngineParityPaths.ets'
    target.parent.mkdir(parents=True, exist_ok=True)
    content = (TEMPLATES / 'EngineParityPaths.ets.in').read_text().replace('__NONCE__', nonce)
    target.write_text(content)
    changes.append({'path': target.relative_to(stage).as_posix(), 'reason': 'compile-time QA paths, no Want path override',
                    'beforeSha256': None, 'afterSha256': digest(content.encode())})
    replacements = {'__COMMAND_LIST__': json.dumps(json.loads((TEMPLATES / 'commands.json').read_text())),
                    '__FIXTURE_HOST__': fixture_host, '__HTTP_PORT__': str(http_port), '__DNS_PORT__': str(dns_port), '__UDP_PORT__': str(udp_port)}
    for name in ('EngineParityTrace.ets', 'EngineParityNetwork.ets.in'):
        content = (TEMPLATES / name).read_text()
        for key, value in replacements.items():
            content = content.replace(key, value)
        target = stage / 'entry/src/main/ets/diagnostics' / name.removesuffix('.in')
        target.write_text(content)
        changes.append({'path': target.relative_to(stage).as_posix(), 'reason': 'bounded stage-only diagnostic; compiled whitelist/endpoint',
                        'beforeSha256': None, 'afterSha256': digest(content.encode())})
    if ui_error_probe:
        errors = json.loads((TEMPLATES / 'ui-errors.json').read_text())
        content = (TEMPLATES / 'EngineParityUiError.ets.in').read_text()
        for key, value in errors.items():
            content = content.replace(f'__{key.upper()}__', json.dumps(value, ensure_ascii=False))
        target = stage / 'entry/src/main/ets/diagnostics/EngineParityUiError.ets'
        target.write_text(content)
        changes.append({'path': target.relative_to(stage).as_posix(),
                        'reason': 'explicit optional one-shot public error; next proxies query only, 10s TTL, no automatic Core call',
                        'beforeSha256': None, 'afterSha256': digest(content.encode())})
    audit_isolation(stage, nonce)
    return changes


def audit_isolation(stage, nonce):
    if not NONCE.fullmatch(nonce):
        raise ValueError('Nonce must be exactly 32 lowercase hex characters')
    for path in (stage / 'entry/src/main/ets').rglob('*.ets'):
        text = path.read_text()
        relative = path.relative_to(stage).as_posix()
        for line in text.splitlines():
            remaining = line.replace('engineParityBase(this.context.filesDir)', '').replace(
                'engineParityHome(this.context.filesDir)', '').replace('startEngineParityTrace(this.context.filesDir)', '')
            if '.filesDir' in remaining:
                raise ValueError(f'Unisolated application filesDir consumer: {relative}')
        if '/data/storage/el2/base/files/paws' in text:
            raise ValueError(f'Personal absolute root remains: {relative}')
    if load_config(stage / BACKUP_CONFIG) != {'allowToBackupRestore': False, 'includes': [], 'excludes': []}:
        raise ValueError('OS backup/restore is not disabled')
    for relative in (FORM, FORM_UPDATE):
        text = (stage / relative).read_text()
        if 'formProvider' in text or 'fileIo' in text:
            raise ValueError('QA widgets must not read IDs or invoke OS widget updates')
    expected = (TEMPLATES / 'EngineParityPaths.ets.in').read_text().replace('__NONCE__', nonce)
    if (stage / 'entry/src/main/ets/diagnostics/EngineParityPaths.ets').read_text() != expected:
        raise ValueError('QA path helper was changed')


def payload_from_hap(hap, manifest):
    expected = {manifest.get('signedHapSha256'), manifest.get('unsignedHapSha256')}
    if file_hash(hap) not in expected:
        raise ValueError('HAP does not match the supplied normal build manifest')
    if manifest.get('hapBuildMode') != 'debug' or manifest.get('diagnosticBypass') is not False \
            or manifest.get('wireCapture') is not False:
        raise ValueError('Require a normal debug HAP without diagnostic bypass or wire capture')
    with zipfile.ZipFile(hap) as archive:
        names = archive.namelist()
        if len(names) != len(set(names)):
            raise ValueError('Duplicate HAP ZIP members')
        module = json.loads(archive.read('module.json'))
        if module.get('app', {}).get('debug') is not True:
            raise ValueError('Packaged normal HAP is not a debug application')
        libraries = {}
        rawfiles = {}
        for name in names:
            path = PurePosixPath(name)
            if path.is_absolute() or '..' in path.parts:
                raise ValueError('Unsafe HAP member path')
            if len(path.parts) == 3 and path.parts[:2] == ('libs', 'arm64-v8a') and path.suffix == '.so':
                data = archive.read(name)
                proof = manifest.get('nativeLibraries', {}).get(path.name, {})
                if digest(data) != proof.get('packagedSha256') or proof.get('loadSegmentsMatchCompiledLibrary') is not True:
                    raise ValueError(f'Normal packaged native proof mismatch: {path.name}')
                libraries[path.name] = data
            if path.parts[:2] == ('resources', 'rawfile') and not name.endswith('/'):
                rawfiles[PurePosixPath(*path.parts[2:]).as_posix()] = archive.read(name)
        if set(libraries) != set(manifest.get('nativeLibraries', {})):
            raise ValueError('Normal HAP native library inventory is incomplete')
        bundle = rawfiles.get('paws.bundle.js')
        bundle_proof = manifest.get('uiRuntime', {}).get('bundle', {})
        if bundle is None or digest(bundle) != bundle_proof.get('packagedSha256') \
                or bundle_proof.get('matchesCompiledBundle') is not True:
            raise ValueError('Normal packaged PawsApp bundle proof mismatch')
        if not {'libpaws_ui.so', 'libnsh_platform_ohos.so', 'libclash.so', 'libsingbox.so'}.issubset(libraries):
            raise ValueError('Normal HAP is missing an engine or UI runtime library')
        return module['app']['bundleName'], libraries, rawfiles


def select_normal_native_artifacts(stage):
    """Exclude only the rebuilt HAR duplicate, never select by iteration order.

    Hvigor process-libs applies selection to the originating package, while our
    exact normal artifact is the local entry library. Final HAP verification
    still requires every native byte to equal the normal candidate.
    """
    package = load_config(stage / 'target/ohos/arkts-deps/ability/oh-package.json5')
    if package.get('name') != '@ohos-rs/ability':
        raise ValueError('Review native artifact selection for the new Ability package')
    path = stage / 'entry/build-profile.json5'
    profile = load_config(path)
    native = profile.setdefault('buildOption', {}).setdefault('nativeLib', {})
    rules = native.setdefault('filter', {}).setdefault('select', [])
    if any(rule.get('package') == package['name'] for rule in rules):
        raise ValueError('Existing Ability native selection requires explicit review')
    rules.append({'package': package['name'], 'excludePattern': ['**/libnative_ability.so']})
    native.setdefault('debugSymbol', {})['strip'] = False
    for option in profile.get('buildOptionSet', []):
        option.setdefault('nativeLib', {}).setdefault('debugSymbol', {})['strip'] = False
    write_json(path, profile)
    return {'path': 'entry/build-profile.json5',
            'reason': 'exclude only rebuilt @ohos-rs/ability native duplicate; retain exact normal entry SO, disable re-stripping'}


def prepare(repo, stage, hap, manifest_path, nonce, fixture_host='127.0.0.1', http_port=18746, dns_port=18753, udp_port=18754, ui_error_probe=False):
    if not NONCE.fullmatch(nonce):
        raise ValueError('Nonce must be exactly 32 lowercase hex characters')
    ipaddress.IPv4Address(fixture_host)
    if any(type(port) is not int or not 1 <= port <= 65535 for port in (http_port, dns_port, udp_port)):
        raise ValueError('Fixture ports must be 1..65535')
    if type(ui_error_probe) is not bool:
        raise ValueError('UI error probe must be an explicit boolean')
    repo = repo.resolve(strict=True)
    stage = stage.resolve(strict=False)
    if stage.exists():
        raise ValueError('Stage already exists; retain it as evidence and choose a new nonce/path')
    if repo == stage or repo in stage.parents and not stage.is_relative_to(repo / 'target'):
        raise ValueError('Use an independent target directory, never the production source tree')
    candidate_hashes = {'hap': file_hash(hap), 'manifest': file_hash(manifest_path)}
    recipe_hash = file_hash(Path(__file__))
    template_inputs = inventory(TEMPLATES, tuple(path.name for path in sorted(TEMPLATES.iterdir()) if path.is_file()))
    normal = json.loads(manifest_path.read_text())
    bundle, libraries, rawfiles = payload_from_hap(hap, normal)
    inputs = (*COPY_DIRS, *COPY_FILES, 'build-profile.json5')
    before = inventory(repo, inputs)
    scope = load_config(repo / 'AppScope/app.json5')
    if scope['app']['bundleName'] != bundle:
        raise ValueError('Current project and supplied normal HAP bundle identities differ')
    # Preserve source-pair attestations and require the normal manifest to agree.
    arkts = repo / 'target/ohos/arkts-source-inputs.json'
    if file_hash(arkts) != normal['uiRuntime']['arktsSources']['manifestSha256']:
        raise ValueError('HAR source attestation is not the normal candidate attestation')
    sidecar = json.loads(arkts.read_text())
    derived_har_inputs = {}
    for package in sidecar['packages']:
        relative = Path(package['stagedRoot'])
        if relative.is_absolute() or '..' in relative.parts:
            raise ValueError('HAR attestation path is outside the candidate project')
        current = inventory(repo, (relative,))
        actual = {Path(name).relative_to(relative).as_posix(): record['sha256'] for name, record in current.items()}
        expected = {name: record['stagedSha256'] for name, record in package['files'].items()}
        # OHPM may materialize this lock after source attestation. It is a
        # derived resolution input, never an attested source file. Allow only
        # this exact known extra and bind its bytes separately to the stage.
        if package['name'] == '@nsh/ohos' and set(actual) - set(expected) == {'oh-package-lock.json5'}:
            lock = (relative / 'oh-package-lock.json5').as_posix()
            derived_har_inputs[lock] = current[lock]
            actual.pop('oh-package-lock.json5')
        if actual != expected:
            raise ValueError(f'HAR inputs changed after normal build: {package["name"]}')
    stage.mkdir(parents=True)
    for relative in COPY_DIRS:
        shutil.copytree(repo / relative, stage / relative, ignore=shutil.ignore_patterns(*GENERATED))
    for relative in COPY_FILES:
        target = stage / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(repo / relative, target)
    # No signing credentials, local IDE data, app data, generated build products,
    # or previous probe are copied. Root signs the result with its existing key.
    profile = load_config(repo / 'build-profile.json5')
    profile['app']['signingConfigs'] = []
    for product in profile['app'].get('products', []):
        product.pop('signingConfig', None)
    write_json(stage / 'build-profile.json5', profile)
    native = stage / 'entry/libs/arm64-v8a'
    native.mkdir(parents=True)
    for name, data in libraries.items():
        (native / name).write_bytes(data)
    raw = stage / 'entry/src/main/resources/rawfile'
    # entry/src is already copied. Discard only this freshly created stage's
    # rawfiles so no newer/unpackaged workspace resource sneaks into the test.
    if raw.exists():
        shutil.rmtree(raw)
    raw.mkdir(parents=True)
    for relative, data in rawfiles.items():
        target = raw / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
    changes = patch_host(stage, nonce, fixture_host, http_port, dns_port, udp_port, ui_error_probe)
    changes.append(select_normal_native_artifacts(stage))
    after = inventory(repo, inputs)
    if before != after:
        raise ValueError('Candidate host/HAR inputs changed while staging; keep failed stage and restage after freeze')
    if candidate_hashes != {'hap': file_hash(hap), 'manifest': file_hash(manifest_path)}:
        raise ValueError('Normal HAP or build manifest changed while staging; discard candidate identity')
    if recipe_hash != file_hash(Path(__file__)) or template_inputs != inventory(
            TEMPLATES, tuple(path.name for path in sorted(TEMPLATES.iterdir()) if path.is_file())):
        raise ValueError('QA recipe/templates changed while staging; restage after freeze')
    result = {'format': 'Paws-engine-parity-stage-v1', 'status': 'staged-not-built-not-signed-not-installed',
        'nonce': nonce, 'bundleName': bundle, 'normalHap': {'path': str(hap.resolve()), 'sha256': file_hash(hap)},
        'normalManifest': {'path': str(manifest_path.resolve()), 'sha256': file_hash(manifest_path)},
        'normalBuildFingerprint': normal['buildFingerprint'], 'sourceInputs': before,
        'derivedHarInputs': derived_har_inputs,
        'stageDifferences': changes + [{'path': 'build-profile.json5', 'reason': 'signing entries removed; no credentials copied'}],
        'isolation': {'home': f'<filesDir>/engine-parity/{nonce}/paws',
            'journalAndExitBase': f'<filesDir>/engine-parity/{nonce}',
            'widgets': 'disabled; no ID reads or OS form updates', 'osBackupRestore': False,
            'personalCoreInitialized': False, 'runtimePathOverride': False},
        'nativeLibraries': {name: {'sha256': digest(data), 'bytes': len(data)} for name, data in sorted(libraries.items())},
        'rawfiles': {name: {'sha256': digest(data), 'bytes': len(data)} for name, data in sorted(rawfiles.items())},
        'execution': {'core': 'production', 'gateway': 'production', 'uiCommandSession': 'production',
            'uiBundle': 'normal HAP bytes, not FakeHost', 'signing': 'not performed; same-bundle normal key required'},
        'networkProbe': {'host': fixture_host, 'httpPort': http_port, 'dnsUpstreamPort': dns_port,
            'udpEchoPort': udp_port,
            'dnsTunAddress': '172.19.0.2:53', 'timeoutMs': 5000, 'closeTimeoutMs': 1000,
            'automaticVpnOrConfigurationMutation': False},
        'uiErrorProbe': {'enabled': ui_error_probe, 'ttlMs': 10000, 'queryDomain': 'proxies',
            'actions': ['ui-error-multiline', 'ui-error-unbroken'] if ui_error_probe else [],
            'automaticCoreCalls': False, 'fixedPublicErrors': {
                key: {'utf8Bytes': len(value.encode()), 'utf16Units': len(value.encode('utf-16-le')) // 2,
                      'sha256': digest(value.encode())}
                for key, value in json.loads((TEMPLATES / 'ui-errors.json').read_text()).items()
            } if ui_error_probe else {}, 'notProofOf': 'native query failure cause or recovery'},
        'nativeArtifactSelection': {'source': 'normal HAP entry/libs bytes',
            'excludedDependency': '@ohos-rs/ability', 'excludedPattern': '**/libnative_ability.so',
            'strip': False, 'requiresPostBuildExactSha256': True},
        'builderSha256': recipe_hash, 'templates': template_inputs}
    result['stagedInputs'] = inventory(stage, (*COPY_DIRS, *COPY_FILES, 'build-profile.json5', 'entry/libs'))
    write_json(stage / 'engine-parity-manifest.json', result)
    return result


def verify(stage, hap=None):
    manifest = json.loads((stage / 'engine-parity-manifest.json').read_text())
    audit_isolation(stage, manifest['nonce'])
    current = inventory(stage, (*COPY_DIRS, *COPY_FILES, 'build-profile.json5', 'entry/libs'))
    if current != manifest['stagedInputs']:
        raise ValueError('Staged inputs changed; review and create a fresh stage')
    if hap is not None:
        with zipfile.ZipFile(hap) as archive:
            module = json.loads(archive.read('module.json'))
            if module['app']['bundleName'] != manifest['bundleName'] or module['app'].get('debug') is not True:
                raise ValueError('QA HAP debug/bundle identity mismatch')
            expected = {f'libs/arm64-v8a/{name}': value for name, value in manifest['nativeLibraries'].items()}
            expected.update({f'resources/rawfile/{name}': value for name, value in manifest['rawfiles'].items()})
            for name, proof in expected.items():
                if digest(archive.read(name)) != proof['sha256']:
                    raise ValueError(f'QA payload differs from normal candidate bytes: {name}')
            natives = {name for name in archive.namelist() if name.startswith('libs/') and name.endswith('.so')}
            if natives != {name for name in expected if name.startswith('libs/')}:
                raise ValueError('QA native library inventory changed')
    return {'stageVerified': True, 'hapPayloadVerified': hap is not None, 'nonce': manifest['nonce'],
            'notProofOf': ['runtime isolation', 'engine parity', 'cryptographic signing', 'device installation']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--repo', type=Path, default=ROOT)
    parser.add_argument('--nonce', help='new compile-time 32-character lowercase hex identity')
    parser.add_argument('--normal-hap', type=Path)
    parser.add_argument('--normal-manifest', type=Path)
    parser.add_argument('--stage', type=Path)
    parser.add_argument('--verify', action='store_true')
    parser.add_argument('--qa-hap', type=Path, help='optional post-build byte/provenance verification only')
    parser.add_argument('--fixture-host', default='127.0.0.1', help='compiled IPv4 allowlist; set reachable LAN fixture host for device tests')
    parser.add_argument('--http-port', type=int, default=18746)
    parser.add_argument('--dns-port', type=int, default=18753)
    parser.add_argument('--udp-port', type=int, default=18754)
    parser.add_argument('--ui-error-probe', action='store_true', help='optional debug/nonce-gated one-shot public error on the next proxies query; no automatic query')
    args = parser.parse_args()
    if args.verify:
        if not args.stage:
            parser.error('--verify requires --stage')
        print(json.dumps(verify(args.stage, args.qa_hap), indent=2))
        return
    if not args.nonce or not args.normal_hap or not args.normal_manifest:
        parser.error('staging requires --nonce, --normal-hap and --normal-manifest')
    stage = args.stage or args.repo / 'target/engine-parity' / args.nonce
    result = prepare(args.repo, stage, args.normal_hap, args.normal_manifest, args.nonce,
                     args.fixture_host, args.http_port, args.dns_port, args.udp_port, args.ui_error_probe)
    print(json.dumps({'stage': str(stage), 'nonce': result['nonce'], 'status': result['status']}, indent=2))


if __name__ == '__main__':
    main()
