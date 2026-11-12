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
from nsh_runner import run_nsh, navigation_steps


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
    if width >= 840:
        return validate_table(tree, mode, language)
    results = []
    for row_id, host, active in ROWS:
        suffix = f'{mode}-{row_id}'
        row, title, meta, bytes_node = [tree.one(f'activity-{part}-{suffix}') for part in ('row', 'title', 'meta', 'bytes')]
        assert row['frame'][3] >= 58, 'Activity content clipped vertically'
        assert title['frame'][2] >= 80, 'Title collapsed'
        assert meta['frame'][3] >= 18, 'Metadata row height'
        for node in (title, meta, bytes_node):
            about.contains(tree.rect(row), tree.rect(node), 'Row content')
        about.no_overlap(tree.rect(title), tree.rect(bytes_node), 'Title and transfer bytes')
        about.no_overlap(tree.rect(meta), tree.rect(bytes_node), 'Metadata and transfer bytes')
        assert not any(n.get('testId') == f'rule-for-{row_id}' and tree.visible(n) for n in tree.nodes.values()), 'Touch row cluttered with inline action buttons'
        text = next(n for n in children(tree, title) if n['kind'] == 'Text')
        assert text.get('text') == host, 'Host/domain/IP fallback changed'
        assert row.get('accessibilityLabel') == host, 'Full host accessibility label lost'
        assert text['frame'][3] <= 22, 'Host text wrapped in main row'
        about.text_fits(tree, text)
        assert tree.rect(text)[3] <= tree.rect(meta)[1] + 4, 'Host and metadata lines overlap'
        assert row.get('events', 0) != 0 and 'role: Button' in row.get('props', ''), 'History navigation / live details lost its action'
        metadata = next(n for n in children(tree, meta) if n['kind'] == 'Text')
        assert metadata['frame'][2] > 0 and metadata['frame'][3] <= 20, 'Metadata text squeezed/wrapped'
        about.text_fits(tree, metadata)
        if mode == 'request':
            label = ('活跃' if active else '已结束') if language == 'zh' else ('Active' if active else 'Ended')
            tags = [n for n in tree.descendants(title) if n.get('accessibilityLabel') == label and 'accessible: Some(true)' in n.get('props', '')]
            assert len(tags) == 1, f'Missing real {label} status Tag'
            tag = tags[0]
            about.contains(tree.rect(title), tree.rect(tag), 'Tag in title row')
            about.no_overlap(tree.rect(text), tree.rect(tag), 'Host and status Tag')
            tag_center_delta = abs(center(tree.rect(tag))[1] - center(tree.rect(text))[1])
            assert tag_center_delta <= 1.5, 'Host and status Tag vertical centers differ'
        else:
            net_label = 'UDP' if row_id == 'ip-fallback' else 'TCP'
            tags = [n for n in tree.descendants(title) if n.get('accessibilityLabel') == net_label and 'accessible: Some(true)' in n.get('props', '')]
            assert len(tags) == 1, f'Missing {net_label} network Tag'
            assert not any('accessible: Some(true)' in n.get('props', '') and n.get('accessibilityLabel') in ('活跃', '已结束', 'Active', 'Ended') for n in tree.descendants(row)), 'History-only Tag leaked into live connections'
            tag_center_delta = abs(center(tree.rect(tags[0]))[1] - center(tree.rect(text))[1])
        results.append({'rowId': row_id, 'active': active if mode == 'request' else True,
                        'titleWidth': title['frame'][2], 'bytesRect': tree.rect(bytes_node),
                        'secondaryActionsUseBottomSheet': True,
                        'tagTitleCenterDelta': tag_center_delta})
    for item in results[1:]:
        near(item['bytesRect'][2], results[0]['bytesRect'][2], 'Transfer bytes right alignment')
    # Header Back/title/Clear action are separate actual production siblings.
    label = ('清空…' if mode == 'request' else '关闭全部…') if language == 'zh' else ('Clear…' if mode == 'request' else 'Close all…')
    controls = [n for n in tree.nodes.values() if tree.visible(n) and n.get('accessibilityLabel') == label and 'frame' in n]
    assert len(controls) == 1, 'Page-level clear/close action missing'
    control = controls[0]
    top = tree.nodes[control['parent']]
    while 'frame' not in top and top['parent'] in tree.nodes:
        top = tree.nodes[top['parent']]
    top_children = children(tree, top)
    for a, b in itertools.combinations(top_children, 2):
        about.no_overlap(tree.rect(a), tree.rect(b), 'Page header siblings')
    for child in top_children:
        about.contains(tree.rect(top), tree.rect(child), 'Page header bounds')
    return {'rows': results, 'headerActionRect': tree.rect(control), 'measuredRows': len(results)}


def validate_table(tree, mode, language):
    header = tree.one('activity-table-header')
    expected_columns = 6 if mode == 'request' else 5
    results = []
    for row_id, host, active in ROWS:
        row, destination, actions = [tree.one(f'activity-table-{part}-{row_id}') for part in ('row', 'destination', 'actions')]
        assert row['frame'][3] >= 46
        assert destination['accessibilityLabel'] == host
        assert destination['frame'][2] > 80
        for node in (destination, actions):
            about.contains(tree.rect(row), tree.rect(node), 'Desktop table cell')
        about.no_overlap(tree.rect(destination), tree.rect(actions), 'Destination and actions')
        row_children = children(tree, row)
        header_children = children(tree, header)
        assert len(row_children) == len(header_children) == expected_columns, 'Desktop table column mismatch'
        for cell, heading in zip(row_children, header_children):
            near(tree.rect(cell)[0], tree.rect(heading)[0], 'Desktop column alignment')
        plus, menu = tree.one(f'rule-for-{row_id}'), tree.one(f'activity-menu-{row_id}')
        near(plus['frame'][2], 36, 'Desktop action size')
        near(menu['frame'][2], 36, 'Desktop menu size')
        about.no_overlap(tree.rect(plus), tree.rect(menu), 'Desktop table actions')
        results.append({'rowId': row_id, 'destinationRect': tree.rect(destination), 'actionsRect': tree.rect(actions)})
    label = ('清空…' if mode == 'request' else '关闭全部…') if language == 'zh' else ('Clear…' if mode == 'request' else 'Close all…')
    controls = [n for n in tree.nodes.values() if tree.visible(n) and n.get('accessibilityLabel') == label and 'frame' in n]
    assert len(controls) == 1, 'Desktop workspace clear action missing'
    return {'rows': results, 'headerActionRect': tree.rect(controls[0]), 'measuredRows': len(results), 'desktopTable': True}


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
        taps += ['activity-preview-hide', *navigation_steps('requests' if mode == 'request' else 'connections', width)]
        command = [str(args.nsh_bin.resolve()), 'trace', ENTRY, '--width', str(width), '--height', str(height), '--wait', '30']
        for tap in taps:
            command.extend(['--tap', 'id:' + tap])
        run = run_nsh(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=45)
        path = args.output_dir / f'{width}-{mode}-{language}-{theme}.log'
        path.write_text(run.stdout)
        assert run.returncode == 0, f'CLI failed: {path}'
        assert not re.search(r'(?im)^(?:error(?:\s|:)|uncaught exception(?:\s|:))', run.stdout), str(path)
        assert len(re.findall(r'PAWS PRESS #\b', run.stdout)) == len(taps), f'Missing completed step: {path}'
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
