"""Check actual Activity row geometry; does not certify native glyph pixels/FPS."""
import argparse
import hashlib
import importlib.util
import itertools
import json
import os
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[3]
ENTRY = 'ui/nsh/tests/ActivityPreview.tsx'
ABOUT_READER = Path(__file__).with_name('run-about-tests.py')
spec = importlib.util.spec_from_file_location('about_mount_reader', ABOUT_READER)
about = importlib.util.module_from_spec(spec)
spec.loader.exec_module(about)
ROWS = [('short-active', 'a.example', True),
        ('long-ended', 'public-synthetic-unbroken-ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-' * 3 + '.example', False),
        ('host-fallback', 'public-host-only.example', True),
        ('ip-fallback', '203.0.113.99', False)]


def near(actual, expected, label):
    assert abs(actual - expected) <= 1, f'{label}: {actual} != {expected}'


def center(rect):
    return (rect[0] + rect[2]) / 2, (rect[1] + rect[3]) / 2


def children(tree, node):
    return [n for n in tree.nodes.values() if n['parent'] == node['id'] and tree.visible(n) and 'frame' in n]


def validate(tree, mode, width, height, language):
    about.visual.validate(tree, 'history' if mode == 'request' and language == 'zh' else 'home', width, height)
    results = []
    for row_id, host, active in ROWS:
        suffix = f'{mode}-{row_id}'
        row, header, title, actions, meta = [tree.one(f'activity-{part}-{suffix}') for part in ('row', 'header', 'title', 'actions', 'meta')]
        near(row['frame'][3], 104, 'Row height')
        near(header['frame'][3], 32, 'Header height')
        near(title['frame'][3], 32, 'Title height')
        near(actions['frame'][2], 72, 'Action group width')
        near(actions['frame'][3], 32, 'Action group height')
        near(meta['frame'][3], 22, 'Metadata row height')
        for node in (header, meta):
            about.contains(tree.rect(row), tree.rect(node), 'Row direct content')
        about.contains(tree.rect(header), tree.rect(title), 'Title in main row')
        about.contains(tree.rect(header), tree.rect(actions), 'Actions in main row')
        about.no_overlap(tree.rect(title), tree.rect(actions), 'Long title/action group')
        assert title['frame'][2] >= 80, 'Title collapsed under actions'
        text = next(n for n in children(tree, title) if n['kind'] == 'Text')
        assert text.get('text') == host, 'Host/domain/IP fallback changed'
        assert title.get('accessibilityLabel') == host, 'Full host accessibility label lost'
        assert text['frame'][3] <= 20, 'Host text displaced main-row actions by wrapping'
        about.text_fits(tree, text)
        near(center(tree.rect(text))[1], center(tree.rect(header))[1], 'Host/action vertical center')
        if mode == 'request':
            assert title.get('events', 0) != 0 and 'role: Button' in title.get('props', ''), 'History-to-connections navigation disappeared'
        else:
            assert title.get('events', 0) == 0 and 'role: None' in title.get('props', ''), 'Live host became a noop button'
        plus = tree.one(f'rule-for-{row_id}')
        about.contains(tree.rect(actions), tree.rect(plus), 'Add-rule button')
        assert plus.get('events', 0) != 0
        near(plus['frame'][2], 32, 'Plus width')
        near(plus['frame'][3], 32, 'Plus height')
        near(center(tree.rect(plus))[1], center(tree.rect(header))[1], 'Plus main-row center')
        if mode == 'request' and not active:
            close = tree.one(f'activity-close-slot-request-{row_id}')
            assert 'accessible: Some(false)' in close.get('props', '') and 'pointer_events: None' in close.get('props', ''), 'Ended close spacer is accessible/interactive'
            assert close.get('events', 0) == 0 and not tree.descendants(close), 'Ended close spacer owns events/content'
            assert not any(n.get('testId') == f'activity-close-request-{row_id}' and tree.visible(n) for n in tree.nodes.values()), 'Ended history still has Close action'
        else:
            close = tree.one(f'activity-close-{suffix}')
            assert close.get('events', 0) != 0 and 'role: Button' in close.get('props', '')
        near(close['frame'][2], 32, 'Close slot width')
        near(close['frame'][3], 32, 'Close slot height')
        near(tree.rect(plus)[2] + 8, tree.rect(close)[0], 'Plus/close gap')
        about.no_overlap(tree.rect(plus), tree.rect(close), 'Main-row icon buttons')
        metadata = next(n for n in children(tree, meta) if n['kind'] == 'Text')
        assert metadata['frame'][2] > 0 and metadata['frame'][3] <= 20, 'Metadata text squeezed/wrapped'
        about.text_fits(tree, metadata)
        tag_center_delta = None
        if mode == 'request':
            label = ('活跃' if active else '已结束') if language == 'zh' else ('Active' if active else 'Ended')
            tags = [n for n in tree.descendants(meta) if n.get('accessibilityLabel') == label and 'accessible: Some(true)' in n.get('props', '')]
            assert len(tags) == 1, f'Missing real {label} status Tag'
            tag = tags[0]
            about.contains(tree.rect(meta), tree.rect(tag), 'Tag in metadata row')
            about.no_overlap(tree.rect(tag), tree.rect(metadata), 'Status and route metadata')
            tag_center_delta = abs(center(tree.rect(tag))[1] - center(tree.rect(metadata))[1])
            assert tag_center_delta <= 1, 'Status and metadata vertical centers differ'
        else:
            assert not any('accessible: Some(true)' in n.get('props', '') and n.get('accessibilityLabel') in ('活跃', '已结束', 'Active', 'Ended') for n in tree.descendants(meta)), 'History-only Tag leaked into live connections'
        bytes_row = next(n for n in children(tree, row) if n['kind'] == 'Text')
        about.contains(tree.rect(row), tree.rect(bytes_row), 'Transfer bytes line')
        assert tree.rect(header)[3] <= tree.rect(meta)[1] + 1 and tree.rect(meta)[3] <= tree.rect(bytes_row)[1] + 1, 'Main/metadata/bytes lines overlap'
        results.append({'rowId': row_id, 'active': active if mode == 'request' else True,
                        'titleWidth': title['frame'][2], 'plusRect': tree.rect(plus), 'closeRect': tree.rect(close),
                        'endedCloseIsNoninteractiveSpacer': mode == 'request' and not active,
                        'tagMetadataCenterDelta': tag_center_delta})
    for item in results[1:]:
        near(item['plusRect'][0], results[0]['plusRect'][0], 'Active/ended plus column')
        near(item['closeRect'][0], results[0]['closeRect'][0], 'Active/ended close column')
    # Header Back/title/Clear action are separate actual production siblings.
    label = ('清空记录' if mode == 'request' else '关闭全部') if language == 'zh' else ('Clear history' if mode == 'request' else 'Close all')
    controls = [n for n in tree.nodes.values() if tree.visible(n) and n.get('accessibilityLabel') == label and 'frame' in n]
    assert len(controls) == 1, 'Page-level clear/close action missing'
    control = controls[0]
    top = tree.nodes[control['parent']]
    top_children = children(tree, top)
    for a, b in itertools.combinations(top_children, 2):
        about.no_overlap(tree.rect(a), tree.rect(b), 'Page header siblings')
    for child in top_children:
        about.contains(tree.rect(top), tree.rect(child), 'Page header bounds')
    return {'rows': results, 'headerActionRect': tree.rect(control), 'measuredRows': len(results)}


def inputs():
    paths = [ROOT / ENTRY, Path(__file__).resolve(), ABOUT_READER, about.VISUAL_READER,
             *sorted((ROOT / 'ui/nsh/src').rglob('*.ts')), *sorted((ROOT / 'ui/nsh/src').rglob('*.tsx'))]
    return {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--nsh-bin', type=Path, default=Path(os.environ.get('NSH_BIN', ROOT / 'dist/nsh-sdk/release/nsh')))
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'target/nsh-activity-tests')
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    before, binary_hash = inputs(), hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest()
    results = []
    for width, mode, language, theme in itertools.product((320, 390, 1280), ('request', 'connection'), ('zh', 'en'), ('light', 'dark')):
        height = 900 if width == 1280 else 844
        taps = (['activity-preview-language'] if language == 'en' else []) + (['activity-preview-theme'] if theme == 'dark' else [])
        taps += ['activity-preview-hide', 'nav-tools', 'tool-requests' if mode == 'request' else 'tool-connections']
        command = [str(args.nsh_bin.resolve()), 'trace', ENTRY, '--width', str(width), '--height', str(height), '--wait', '30']
        for tap in taps:
            command.extend(['--tap', 'id:' + tap])
        run = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=45)
        path = args.output_dir / f'{width}-{mode}-{language}-{theme}.log'
        path.write_text(run.stdout)
        assert run.returncode == 0, f'CLI failed: {path}'
        assert not re.search(r'(?im)^(?:error(?:\s|:)|uncaught exception(?:\s|:))', run.stdout), str(path)
        assert len(re.findall(r'perf event Press on view\b', run.stdout)) == len(taps), f'Missing actual Press: {path}'
        tree = about.AboutTree()
        for line in run.stdout.splitlines():
            tree.apply(line)
        result = validate(tree, mode, width, height, language)
        results.append({'width': width, 'height': height, 'mode': mode, 'language': language, 'theme': theme, 'log': str(path), **result})
        print(f'PASS Activity: {width} {mode} {language} {theme} (4 actual rows)', flush=True)
    assert before == inputs(), 'Source changed during Activity traces'
    assert binary_hash == hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest(), 'CLI changed during Activity traces'
    (args.output_dir / 'summary.json').write_text(json.dumps({'passed': len(results), 'results': results, 'sourceSha256': before,
        'nshBinSha256': binary_hash, 'sourceFenceStable': True,
        'limits': ['Real PawsApp, native mount events/geometry and public DomainHost; no Core/network/VPN or writes',
                   'Four prepared rows are measured; physical scrolling, native glyph ellipsis and pixel colors remain phone checks',
                   'No close/rule command was submitted; this suite checks action presence, slot noninteractivity and geometry, not business side effects',
                   'ApproxTextEngine/native mount geometry is not FPS or native typography certification']}, indent=2, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
