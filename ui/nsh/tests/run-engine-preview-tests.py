#!/usr/bin/env python3
"""Actual Settings/CoreClient compact readonly preview and write-barrier traces."""
import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import subprocess
from nsh_runner import run_nsh

ROOT = Path(__file__).resolve().parents[3]
ENTRY = 'ui/nsh/tests/EnginePreview.tsx'
spec = importlib.util.spec_from_file_location('preview_native_tree', Path(__file__).with_name('run-visual-tests.py'))
reader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)
LONG_REASON = 'Long compatibility explanation ' + 'x' * 200
FALLBACK_REASON = 'Requested engine cannot compile this public synthetic rule'


def snapshots(output):
    tree, result, current = reader.NativeTree(), {}, 0
    for line in output.splitlines():
        header = re.search(r'PAWS TAP #(\d+)\b', line)
        if header and int(header[1]) != current:
            result[current] = copy.deepcopy(tree)
            current = int(header[1])
        tree.apply(line)
    result[current] = tree
    return result


def mounted(tree, test_id):
    return [node for node in tree.nodes.values() if node.get('testId') == test_id and tree.visible(node)]


def one(tree, test_id):
    matches = mounted(tree, test_id)
    assert len(matches) == 1, f'Expected one mounted {test_id}, got {len(matches)}'
    return matches[0]


def box(tree, node):
    x, y, w, h = node['frame']
    seen = {node['id']}
    while node['parent'] in tree.nodes and node['kind'] != 'Modal':
        node = tree.nodes[node['parent']]
        assert node['id'] not in seen
        seen.add(node['id'])
        dx, dy, _, _ = node.get('frame', (0, 0, 0, 0))
        x, y = x + dx, y + dy
    return x, y, w, h


def detail_text(tree):
    viewport = one(tree, 'engine-preview-info-list')
    chunks = [node for node in tree.descendants(viewport)
              if node.get('testId', '').startswith('engine-preview-info-chunk-')]
    chunks.sort(key=lambda node: int(node['testId'].rsplit('-', 1)[1]))
    return ''.join(node.get('text', '') for node in chunks)


def verify_geometry(states, width):
    baseline = None
    for index, tree in states.items():
        if not mounted(tree, 'vpn-engine'):
            continue
        geometry = {name: box(tree, one(tree, name)) for name in ('vpn-engine', 'vpn-stack', 'vpn-mtu')}
        if baseline is None:
            baseline = geometry
        assert geometry == baseline, (index, geometry, baseline)
        engine, stack = geometry['vpn-engine'], geometry['vpn-stack']
        assert 0 < stack[1] - engine[1] - engine[3] <= 48, (index, engine, stack)
        assert one(tree, 'engine-preview-status')['frame'][3] == 24
        for name in ('engine-preview-reasons', 'engine-preview-info-summary', 'page-data-issues'):
            assert not mounted(tree, name), (index, name)
        # Internal observation metadata remains in the actual store, while no
        # normal page text presents its implementation detail as a banner.
        assert not any('INTERNAL_CONFIGURATION_PREVIEW_METADATA' in node.get('text', '')
                       for node in tree.nodes.values() if tree.visible(node))
        for name in ('vpn-engine', 'vpn-stack', 'engine-preview-details'):
            left, _, w, h = box(tree, one(tree, name))
            assert 0 <= left <= left + w <= width + 1 and h > 0, (index, name, left, w, h)
    assert baseline, 'Settings never mounted'
    return baseline


def run(binary, output_dir, name, taps, checks, markers=(), width=390):
    args = [str(binary), 'trace', ENTRY, '--wait', '40', '--width', str(width), '--height', '844']
    for tap in taps:
        args += ['--tap', tap]
    process = run_nsh(args, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, timeout=75)
    output = process.stdout
    path = output_dir / (name + '.log')
    path.write_text(output)
    assert process.returncode == 0, f'{name}: CLI failed; inspect {path}'
    assert not re.search(r'(?im)^(?:error(?:\s|:)|fatal(?:\s|:)|uncaught exception(?:\s|:))', output)
    presses = len(re.findall(r'PAWS PRESS #\b', output))
    assert presses == len(taps), f'{name}: missing completed test step'
    for marker in markers:
        assert marker in output, f'{name}: missing {marker}'
    states = snapshots(output)
    geometry = verify_geometry(states, width)
    for index, expected_status, expected_reason, enabled, warning in checks:
        tree = states[index]
        trigger = one(tree, 'engine-preview-details')
        status = trigger.get('accessibilityLabel', '')
        assert expected_status in status, (name, index, status)
        engine = one(tree, 'vpn-engine')
        assert f'disabled: {str(not enabled).lower()}' in engine['props'], 'Preview blocked the Select trigger'
        busy = 'Checking' in expected_status or '正在检查' in expected_status
        assert f'busy: {str(busy).lower()}' in engine['props'], (name, index, engine['props'])
        assert f'disabled: {str(busy).lower()}' in trigger['props'], (name, index, trigger['props'])
        assert bool(mounted(tree, 'engine-preview-warning')) == warning, (name, index, warning)
        if expected_reason is not None:
            body = detail_text(tree)
            assert expected_reason in body and expected_status in body, (name, index, body)
            one(tree, 'engine-preview-info-copy')
            one(tree, 'engine-preview-info-close')
            assert len([n for n in tree.nodes.values() if n['kind'] == 'Modal' and tree.visible(n)]) == 1
        else:
            assert not mounted(tree, 'engine-preview-info-list'), (name, index, 'Details opened automatically')
            assert not any('reason-' in n.get('text', '') or LONG_REASON[:25] in n.get('text', '')
                           for n in tree.nodes.values() if tree.visible(n)), (name, index, 'Verbose reason remained inline')
    if name == 'same-option-is-not-a-change':
        assert 'disabled: true' in one(states[len(taps)], 'save-vpn')['props'], 'Reselecting current option dirtied Settings'
    if name == 'metadata-hidden-actual-error-retry':
        assert mounted(states[5], 'page-error-summary') and mounted(states[5], 'page-data-retry')
        assert not mounted(states[8], 'page-error-summary')
    print(f'PASS engine preview: {name} ({presses} completed step)')
    return {'case': name, 'actualPresses': presses, 'width': width, 'stableSettingsGeometry': geometry, 'log': str(path)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--nsh-bin', type=Path, default=Path(os.environ.get('NSH_BIN', ROOT / 'dist/nsh-sdk/release/nsh')))
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'target/nsh-engine-preview-tests')
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    files = [ROOT / ENTRY, Path(__file__).resolve(), ROOT / 'ui/nsh/src/client.tsx',
             ROOT / 'ui/nsh/src/components.tsx', ROOT / 'ui/nsh/src/domain-store.ts',
             ROOT / 'ui/nsh/src/command-data.ts', ROOT / 'ui/nsh/src/pages/settings.tsx',
             ROOT / 'ui/nsh/src/page-error.tsx', ROOT / 'ui/nsh/src/app.tsx', ROOT / 'ui/nsh/src/theme.ts',
             ROOT / 'ui/nsh/tests/DomainHost.ts', ROOT / 'ui/nsh/tests/fixture.ts', ROOT / 'ui/nsh/tests/run-visual-tests.py']
    before = {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in files}
    binary_before = hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest()
    nav = ['id:nav-tools', 'id:tool-settings']
    pick = lambda engine: ['id:vpn-engine', f'id:select-option-{engine}']
    info, close = 'id:engine-preview-details', 'id:engine-preview-info-close'
    cases = [
        ('long-success-reason-on-demand', nav + ['id:preview-release-long', info, close],
         [(3, 'meow', None, True, False), (4, 'meow', LONG_REASON, True, False)], []),
        ('same-option-is-not-a-change', nav + pick('auto') + ['id:preview-assert-same-option'],
         [(5, '正在检查', None, True, False)], ['PASS PREVIEW: same-option']),
        ('rapid-out-of-order', nav + ['id:preview-assert-nonblocking'] + pick('mihomo') + pick('singbox') +
         ['id:preview-release-last', info, close, 'id:preview-release-first', info, close, 'id:preview-release-first', info, close],
         [(3, '正在检查', None, True, False), (9, 'sing-box', 'reason-singbox', True, False),
          (12, 'sing-box', 'reason-singbox', True, False), (15, 'sing-box', 'reason-singbox', True, False)],
         ['PASS PREVIEW: nonblocking']),
        ('preserve-reasons-and-failure', nav + ['id:preview-release-first', info, close] + pick('mihomo') +
         ['id:preview-assert-nonblocking', 'id:preview-fail-first', 'id:preview-assert-error-local', info, close],
         [(4, 'meow', 'reason-auto', True, False), (8, '正在检查', None, True, False),
          (10, '内核检查失败', None, True, True), (11, '内核检查失败', 'Error: Controlled preview failure', True, True)],
         ['PASS PREVIEW: nonblocking', 'PASS PREVIEW: error-local']),
        ('revision-profile-fences', nav + ['id:preview-revision', 'id:preview-profile',
         'id:preview-fail-first', 'id:preview-release-first', 'id:preview-release-last', info, close],
         [(5, '正在检查', None, True, False), (6, '正在检查', None, True, False), (8, 'meow', 'reason-auto', True, False)], []),
        ('real-write-query-barrier', nav + pick('mihomo') + ['id:save-vpn', 'id:preview-assert-write',
         'id:preview-write-result', 'id:preview-assert-barrier', 'id:preview-query-result', 'id:preview-assert-idle'],
         [(6, '正在检查', None, False, False), (8, '正在检查', None, False, False), (10, '正在检查', None, True, False)],
         ['PASS PREVIEW: write', 'PASS PREVIEW: barrier', 'PASS PREVIEW: idle']),
        ('unmount-late-error-reopen', nav + ['id:preview-hide', 'id:preview-fail-first',
         'id:preview-show'] + nav + ['id:preview-release-last', 'id:preview-assert-error-local', info, close],
         [(10, 'meow', 'reason-auto', True, False)], ['PASS PREVIEW: error-local']),
        ('metadata-hidden-actual-error-retry', nav + ['id:preview-assert-metadata-hidden',
         'id:preview-query-error', 'id:preview-assert-query-error', 'id:preview-query-allow',
         'id:page-data-retry', 'id:preview-assert-query-recovered'],
         [(3, '正在检查', None, True, False), (5, '正在检查', None, True, False), (8, '正在检查', None, True, False)],
         ['PASS PREVIEW: metadata-hidden', 'PASS PREVIEW: query-error', 'PASS PREVIEW: query-recovered']),
    ]
    results = [run(args.nsh_bin.resolve(), args.output_dir, *case) for case in cases]
    for width in (320, 390):
        for english in (False, True):
            prefix = ['id:preview-english'] if english else []
            shift = len(prefix)
            taps = prefix + nav + ['id:preview-release-first'] + pick('singbox') + [
                'id:preview-release-fallback', info, close] + pick('meow') + [
                'id:preview-fail-first', info, close, 'id:nav-back', 'id:tool-settings', 'id:preview-release-first']
            checks = [(6, 'mihomo', None, True, True), (7, 'mihomo', FALLBACK_REASON, True, True),
                      (11, 'Engine check failed' if english else '内核检查失败', None, True, True),
                      (12, 'Engine check failed' if english else '内核检查失败', 'Error: Controlled preview failure', True, True),
                      (16, 'meow', None, True, False)]
            results.append(run(args.nsh_bin.resolve(), args.output_dir,
                f'compact-fallback-close-back-{width}-{"en" if english else "zh"}', taps,
                [(i + shift, *rest) for i, *rest in checks], width=width))
    assert before == {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in files}, 'Source changed during preview verification'
    assert binary_before == hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest()
    (args.output_dir / 'summary.json').write_text(json.dumps({'results': results, 'inputs': before,
        'sourceFenceUnchanged': True, 'cliFenceUnchanged': True,
        'nshBin': str(args.nsh_bin.resolve()), 'nshBinSha256': binary_before,
        'limits': ['Production Settings/CoreClient/DomainStore; controlled server receipts with actual Engine enum values',
                   'Native mount state/Press/geometry and on-demand single Modal; not OHOS pixels/IME/clipboard or native engine proof']}, indent=2) + '\n')


if __name__ == '__main__':
    main()
