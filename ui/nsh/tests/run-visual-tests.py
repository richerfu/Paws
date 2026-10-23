"""Read actual native mount geometry; this does not measure rendered glyphs/FPS."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess


class NativeTree:
    def __init__(self):
        self.nodes = {}
        self.root = None
        self.clipboard = []

    def apply(self, line):
        clipboard = re.search(r'Command \{ id: \d+, command: SetClipboard\(("(?:[^"\\]|\\.)*")\)', line)
        if clipboard:
            self.clipboard.append(json.loads(clipboard[1]))
        create = re.search(r'Create \{ id: (\d+), kind: (\w+) \}', line)
        insert = re.search(r'Insert \{ parent: (\d+), child: (\d+),', line)
        remove = re.search(r'(Remove|Delete) \{ id: (\d+) \}', line)
        frame = re.search(r'Frame \{ id: (\d+), x: ([\deE.+-]+), y: ([\deE.+-]+), w: ([\deE.+-]+), h: ([\deE.+-]+) \}', line)
        text = re.search(r'Text \{ id: (\d+), text: ("(?:[^"\\]|\\.)*"), content:', line)
        props = re.search(r'ViewProps \{ id: (\d+),', line)
        if create:
            self.nodes[create[1]] = {'id': create[1], 'kind': create[2], 'parent': None, 'hidden': False}
            if self.root is None:
                self.root = create[1]
        if insert and insert[2] in self.nodes:
            self.nodes[insert[2]]['parent'] = insert[1]
        if remove and remove[2] in self.nodes:
            if remove[1] == 'Delete':
                del self.nodes[remove[2]]
            else:
                self.nodes[remove[2]]['parent'] = None
        if frame and frame[1] in self.nodes:
            self.nodes[frame[1]]['frame'] = tuple(map(float, frame.groups()[1:]))
        if text and text[1] in self.nodes:
            self.nodes[text[1]]['text'] = json.loads(text[2])
        if props and props[1] in self.nodes:
            node = self.nodes[props[1]]
            node['props'] = line
            label = re.search(r'label: Some\(("(?:[^"\\]|\\.)*")\)', line)
            node['accessibilityLabel'] = json.loads(label[1]) if label else None
            test_id = re.search(r'test_id: Some\(("(?:[^"\\]|\\.)*")\)', line)
            if test_id:
                node['testId'] = json.loads(test_id[1])
        label_update = re.search(r'AccessibilityLabel \{ id: (\d+), label: (Some\(("(?:[^"\\]|\\.)*")\)|None) \}', line)
        if label_update and label_update[1] in self.nodes:
            self.nodes[label_update[1]]['accessibilityLabel'] = json.loads(label_update[3]) if label_update[3] else None
        style = re.search(r'ViewStyle \{ id: (\d+),', line)
        if style and style[1] in self.nodes:
            self.nodes[style[1]]['hidden'] = 'hidden: true' in line
            self.nodes[style[1]]['style'] = line

    def visible(self, node):
        seen = set()
        while node['id'] != self.root:
            if node['hidden'] or node['id'] in seen:
                return False
            if 'frame' in node and (node['frame'][2] <= 0 or node['frame'][3] <= 0):
                return False
            seen.add(node['id'])
            parent = self.nodes.get(node['parent'])
            if parent is None:
                return False
            node = parent
        return not node['hidden']

    def x_bounds(self, node):
        left, _, width, _ = node.get('frame', (0, 0, 0, 0))
        seen = {node['id']}
        while node['parent'] in self.nodes and node['kind'] != 'Modal':
            node = self.nodes[node['parent']]
            if node['id'] in seen:
                raise AssertionError('Native mount tree cycle')
            seen.add(node['id'])
            left += node.get('frame', (0, 0, 0, 0))[0]
        assert math.isfinite(left) and math.isfinite(width)
        return left, left + width

    def descendants(self, parent):
        result = []
        for node in self.nodes.values():
            current = node
            while current['parent'] in self.nodes:
                if current['parent'] == parent['id']:
                    result.append(node)
                    break
                current = self.nodes[current['parent']]
        return result

    def tags(self, label):
        return [n for n in self.nodes.values() if self.visible(n)
                and 'accessible: Some(true)' in n.get('props', '')
                and n.get('accessibilityLabel') == label]


ROOT = Path(__file__).resolve().parents[3]
ENTRY = 'ui/nsh/tests/VisualPreview.tsx'
CASES = {
    'home': [], 'profiles': ['nav-profiles'],
    'profile-error': ['nav-profiles', 'profile-menu-visual-error', 'profile-refresh-error'],
    'profile-error-details': ['nav-profiles', 'profile-menu-visual-error', 'profile-refresh-error',
                              'profile-refresh-failure-details', 'profile-refresh-failure-copy'],
    'resources': ['nav-tools', 'tool-resources'],
    'rule-detail': ['nav-tools', 'tool-resources', 'rule-details-rule-1'],
    'history': ['nav-tools', 'tool-requests'],
    'logs-detail': ['nav-tools', 'tool-logs', 'log-2026-10-22T00:00:00Z'],
    'traffic': ['nav-traffic'], 'diagnostics': ['nav-tools', 'tool-diagnostics'],
    'stale-health': ['nav-tools', 'tool-diagnostics', 'visual-stale-health', 'refresh-checks'],
    'about': ['nav-tools', 'tool-about'],
}
LABELS = {
    'profiles': ['当前', '刷新失败'], 'profile-error': ['刷新失败'], 'profile-error-details': ['刷新失败'],
    'resources': ['已加载', '规则', '缓存已加载', '刷新失败', '等待缓存'],
    'rule-detail': ['启用'], 'history': ['活跃', '已结束'],
    'logs-detail': ['日志记录已关闭', 'ERROR', 'WARN'], 'traffic': ['保护未确认'],
    'diagnostics': ['通过', '失败', '不适用', '未测试'], 'stale-health': ['通过', '失败', '未测试'],
}


def bg(node):
    return re.search(r'background_color: Color\((\d+)\)', node['style'])[1]


def error_fixture(output):
    records = [json.loads(line.split('VISUAL ERROR FIXTURE: ', 1)[1])
               for line in output.splitlines() if line.startswith('VISUAL ERROR FIXTURE: ')]
    assert len(records) == 1, 'Public full-error fixture witness missing or duplicated'
    fixture = records[0]
    assert len(fixture['source']) > 300, 'Full-error negative control lost its long source'
    assert ''.join(row['raw'] for row in fixture['rows']) == fixture['source'], 'Error rows lost full source bytes'
    assert all(len(row['text']) <= 48 and not re.search(r'[\r\n\v\f\u0085\u2028\u2029]', row['text'])
               for row in fixture['rows']), 'Display rows are not bounded scalar chunks'
    return fixture


def validate_page_error(tree, mounted, name, width, height, fixture):
    prefix = 'profile-refresh-failure'
    assert any(n.get('testId') == 'dialog-close' and n['frame'][2] > 0 for n in mounted), 'Dialog close control missing'
    summary = [n for n in mounted if n.get('testId') == prefix + '-summary']
    assert len(summary) == 1, 'Actual profile PageError summary missing or duplicated'
    summary = summary[0]
    assert summary.get('text') == fixture['source'][:160] + '…', 'Summary lost the exact bounded source prefix'
    assert len(summary['text']) == 161 and 0 < summary['frame'][3] <= 60, 'Inline error summary is unbounded'
    assert any(n.get('testId') == prefix + '-details' for n in mounted), 'Actual details control missing'
    viewports = [n for n in mounted if n.get('testId') == prefix + '-list']
    chunks = [n for n in mounted if n.get('testId', '').startswith(prefix + '-chunk-')]
    if name == 'profile-error':
        assert not viewports and not chunks, 'Full error details were eagerly mounted before the user opened them'
        assert not tree.clipboard, 'Summary alone unexpectedly copied the full error'
        return {'summaryScalars': len(summary['text']), 'summaryHeight': summary['frame'][3],
                'detailsMountedBeforeOpen': False}
    assert len(viewports) == 1, 'Actual details tap did not open the PageError List'
    viewport = viewports[0]
    assert 0 < viewport['frame'][3] <= min(320, height * 0.4), 'Error detail viewport is unbounded'
    assert any(n['kind'] == 'Scroll' for n in tree.descendants(viewport)), 'Actual virtual List native scroll missing'
    retained = [n for n in tree.descendants(viewport)
                if n.get('testId', '').startswith(prefix + '-chunk-') and 'frame' in n]
    assert 0 < len(retained) <= 28, 'Error virtual List lost its bounded retained pool'
    for node in retained:
        index = int(node['testId'].rsplit('-', 1)[1])
        expected = fixture['rows'][index]
        body = node.get('text', '')
        assert body == expected['text'], 'Actual mounted chunk changed or truncated the source row'
        assert node.get('accessibilityLabel') == expected['raw'], 'Accessible chunk lost its exact hard breaks'
        assert len(body) <= 48 and not re.search(r'[\r\n\v\f\u0085\u2028\u2029]', body)
        assert not re.search(r'^[\uDC00-\uDFFF]|[\uD800-\uDBFF]$', body), 'Chunk split a surrogate pair'
        w, h = node['frame'][2:]
        assert 40 <= h <= 60 and math.ceil(w * 3.25) * math.ceil(h * 3.25) < 4_194_304, 'Error chunk exceeds bounded raster geometry'
        left, right = tree.x_bounds(node)
        assert -1 <= left <= right <= width + 1
    for suffix in ('copy', 'close'):
        assert any(n.get('testId') == prefix + '-' + suffix for n in mounted), f'Details {suffix} control missing'
    assert tree.clipboard == [fixture['source']], 'Actual Copy command lost or truncated the complete error source'
    return {'summaryScalars': len(summary['text']), 'summaryHeight': summary['frame'][3],
            'detailViewportHeight': viewport['frame'][3], 'retainedChunks': len(retained),
            'fullSourceScalars': len(fixture['source']), 'fullSourceRows': len(fixture['rows']),
            'fullSourceCopyCommandExact': True}


def validate(tree, name, width, height, fixture):
    mounted = [n for n in tree.nodes.values() if tree.visible(n) and 'frame' in n]
    assert len(mounted) > 20, 'Actual Paws page did not mount'
    for node in mounted:
        left, right = tree.x_bounds(node)
        assert -1 <= left <= right <= width + 1, f'Viewport overflow: {node.get("text", node["id"])} [{left}, {right}]'
        parent = tree.nodes.get(node['parent'])
        if parent and 'frame' in parent and node['kind'] != 'Modal':
            x, _, w, _ = node['frame']
            assert x >= -1 and x + w <= parent['frame'][2] + 1, f'Horizontal parent overflow: node {node["id"]}'
    unmeasured_tags = []
    for label in LABELS.get(name, []):
        tags = tree.tags(label)
        assert tags, f'Missing accessible status Tag: {label}'
        for tag in tags:
            if 'frame' in tag:
                assert tag['frame'][2] > 0 and tag['frame'][3] > 0
            else:
                # The virtual list can prepare an offscreen row without yet
                # measuring it. Keep semantic coverage separate from geometry.
                unmeasured_tags.append(label)
            assert any(n.get('text') == label for n in tree.descendants(tag)), f'Status {label} lost its explicit text'
    alignment = []
    if name in ('diagnostics', 'stale-health'):
        expected = {'配置载入': '通过', '出站保护': '失败', '节点 TCP': '未测试', 'HTTPS': '失败' if name == 'diagnostics' else '未测试'}
        for label, value in expected.items():
            rows = [tree.nodes[n['parent']] for n in mounted if n.get('text') == label and n['parent'] in tree.nodes]
            matches = [(row, tag) for row in rows for tag in tree.tags(value) if tag in tree.descendants(row)]
            assert matches, f'Detail {label} has wrong actual state {value}'
            for row, tag in matches:
                delta = abs(tree.x_bounds(row)[1] - tree.x_bounds(tag)[1])
                assert delta <= 1, f'Tag right edge differs for {label}: {delta}'
                alignment.append({'label': label, 'value': value, 'rightEdgeDelta': delta})
        assert bg(tree.tags('通过')[0]) != bg(tree.tags('失败')[0]), 'Success/failure Tag visuals identical'
        if name == 'diagnostics':
            assert bg(tree.tags('未测试')[0]) != bg(tree.tags('通过')[0]), 'Untested health looks like a passing result'
        else:
            assert not tree.tags('不适用'), 'Stale-session health remained current'
    if name == 'resources':
        rows = [tree.nodes[n['parent']] for n in mounted if n.get('text') in ('URL', '路径')]
        assert len(rows) >= 2, 'Long URL/path fixtures did not mount'
        for row in rows:
            direct = [n for n in mounted if n['parent'] == row['id']]
            label = next(n for n in direct if n['kind'] == 'Text')
            value = next(n for n in direct if n['kind'] == 'View')
            assert abs(tree.x_bounds(label)[0] - tree.x_bounds(value)[0]) <= 1, 'Stacked value left edge differs'
            assert value['frame'][1] >= label['frame'][1] + label['frame'][3], 'Stacked Detail overlaps label'
    error_presentation = validate_page_error(tree, mounted, name, width, height, fixture) if name in ('profile-error', 'profile-error-details') else None
    if name in ('rule-detail', 'logs-detail'):
        prefix = 'DOMAIN-SUFFIX,long-public-domain-segment.' if name == 'rule-detail' else 'Synthetic error:'
        texts = [n for n in mounted if n.get('text', '').startswith(prefix) and len(n['text']) > 300]
        bounded = []
        for node in texts:
            current = node
            while current['parent'] in tree.nodes:
                current = tree.nodes[current['parent']]
                if current['kind'] == 'Scroll' and 0 < current.get('frame', (0, 0, 0, 0))[3] <= height * 0.45:
                    bounded.append({'textChars': len(node['text']), 'scrollHeight': current['frame'][3]})
                    break
        assert bounded, f'{name}: full synthetic body missing from bounded native Scroll'
        assert any(n.get('testId') == 'dialog-close' and n['frame'][2] > 0 for n in mounted), 'Dialog close control missing'
    return {'mountedNodes': len(mounted), 'tagLabels': LABELS.get(name, []), 'unmeasuredOffscreenTagLabels': unmeasured_tags, 'tagRightAlignment': alignment, 'errorPresentation': error_presentation}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--nsh-bin', type=Path, default=Path(os.environ.get('NSH_BIN', ROOT / 'dist/nsh-sdk/release/nsh')))
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'target/nsh-visual-tests')
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    source = [ROOT / ENTRY, Path(__file__).resolve(), *[ROOT / 'ui/nsh/src' / n for n in ['components.tsx', 'page-error.tsx', 'theme.ts', 'pages/activity.tsx', 'pages/logs.tsx', 'pages/overview.tsx', 'pages/profiles.tsx', 'pages/resources.tsx', 'pages/tools.tsx']]]
    inputs = {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in source}
    cli_sha = hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest()
    results = []
    for width, height in [(320, 844), (390, 844), (1280, 900)]:
        for name, taps in CASES.items():
            command = [str(args.nsh_bin.resolve()), 'trace', ENTRY, '--width', str(width), '--height', str(height), '--wait', '30']
            for selector in ['visual-hide-controls', *taps]:
                command.extend(['--tap', 'id:' + selector])
            run = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=40)
            path = args.output_dir / f'{width}-{name}.log'
            path.write_text(run.stdout)
            assert run.returncode == 0, f'{width}/{name}: CLI failed, see {path}'
            assert not re.search(r'(?im)^(?:error(?:\s|:)|uncaught exception(?:\s|:))', run.stdout), str(path)
            assert len(re.findall(r'perf event Press on view\b', run.stdout)) == len(taps) + 1, f'{width}/{name}: missing actual Press'
            tree = NativeTree()
            for line in run.stdout.splitlines():
                tree.apply(line)
            result = validate(tree, name, width, height, error_fixture(run.stdout))
            results.append({'width': width, 'height': height, 'case': name, 'log': str(path), **result})
            print(f'PASS visual: {width} {name} ({result["mountedNodes"]} mounted nodes)', flush=True)
    assert inputs == {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in source}, 'UI/test source changed during traces'
    assert cli_sha == hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest(), 'CLI changed during traces'
    (args.output_dir / 'summary.json').write_text(json.dumps({'results': results,
        'nshBinSha256': cli_sha, 'sourceSha256': inputs, 'sourceFenceStable': True,
        'limits': ['Actual PawsApp/controls and synthetic DomainHost; no Core/network/VPN',
                   'ApproxTextEngine geometry, not native glyph pixels, contrast certification or FPS',
                   'Mounted nodes may be vertically clipped by scrolling; Tag assertions do not claim all were simultaneously on screen',
                   'Virtualized offscreen nodes may have native props without Frame; these status labels are explicitly listed as unmeasured',
                   'Headless selector taps can target prepared offscreen controls; phone scroll/reachability still needs device verification',
                   'Full error copy validates the emitted native SetClipboard command, not the OS clipboard service',
                   'Modal coordinates use native window origin rather than logical-parent flow']}, indent=2, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
