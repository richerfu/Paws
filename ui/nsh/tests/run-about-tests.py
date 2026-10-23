"""Exercise real About layout and link commands with public synthetic data.

Mount geometry is not native glyph rendering or physical scroll/touch proof.
"""
import argparse
import hashlib
import importlib.util
import itertools
import json
import math
import os
from pathlib import Path
import re
import subprocess


ROOT = Path(__file__).resolve().parents[3]
ENTRY = 'ui/nsh/tests/AboutPreview.tsx'
VISUAL_READER = Path(__file__).with_name('run-visual-tests.py')
spec = importlib.util.spec_from_file_location('visual_mount_reader', VISUAL_READER)
visual = importlib.util.module_from_spec(spec)
spec.loader.exec_module(visual)
NSH_VERSION = ('c8779bac2365a790efd9c7aa91d3fb635815c66e+dirty.worktree-sha256-'
               + '0123456789abcdef' * 8)


class AboutTree(visual.NativeTree):
    def apply(self, line):
        super().apply(line)
        scroll = re.search(r'ScrollContent \{ id: (\d+), size: Size \{ width: ([\deE.+-]+), height: ([\deE.+-]+) \}', line)
        if scroll and scroll[1] in self.nodes:
            self.nodes[scroll[1]]['scrollContent'] = tuple(map(float, scroll.groups()[1:]))
        text = re.search(r'Text \{ id: (\d+), text: "(?:[^"\\]|\\.)*", content: ([\deE.+-]+)x([\deE.+-]+) @ ([\deE.+-]+),([\deE.+-]+)', line)
        if text and text[1] in self.nodes:
            self.nodes[text[1]]['textContent'] = tuple(map(float, text.groups()[1:]))
        events = re.search(r'Events \{ id: (\d+), events: EventMask\((\d+)\)', line)
        if events and events[1] in self.nodes:
            self.nodes[events[1]]['events'] = int(events[2])

    def one(self, test_id):
        found = [n for n in self.nodes.values() if n.get('testId') == test_id and self.visible(n)]
        assert len(found) == 1, f'{test_id}: expected one mounted node, got {len(found)}'
        assert 'frame' in found[0] and min(found[0]['frame'][2:]) > 0, f'{test_id}: no positive frame'
        return found[0]

    def rect(self, node):
        x, y, w, h = node['frame']
        current, seen = node, {node['id']}
        while current['parent'] in self.nodes and current['kind'] != 'Modal':
            current = self.nodes[current['parent']]
            assert current['id'] not in seen, 'Native mount tree cycle'
            seen.add(current['id'])
            px, py, _, _ = current.get('frame', (0, 0, 0, 0))
            x, y = x + px, y + py
        assert all(map(math.isfinite, (x, y, w, h)))
        return x, y, x + w, y + h


def contains(outer, inner, label):
    assert (outer[0] - 1 <= inner[0] and outer[1] - 1 <= inner[1]
            and inner[2] <= outer[2] + 1 and inner[3] <= outer[3] + 1), f'{label}: parent clipping {outer} / {inner}'


def no_overlap(a, b, label):
    overlap_w = min(a[2], b[2]) - max(a[0], b[0])
    overlap_h = min(a[3], b[3]) - max(a[1], b[1])
    assert overlap_w <= 1 or overlap_h <= 1, f'{label}: sibling overlap {a} / {b}'


def text_fits(tree, node):
    assert node.get('text'), f'{node["id"]}: missing actual Text payload'
    assert 'textContent' in node, f'{node["id"]}: missing actual text measure'
    cw, ch, cx, cy = node['textContent']
    _, _, width, height = node['frame']
    assert cx >= -1 and cy >= -1 and cx + cw <= width + 1 and cy + ch <= height + 1, f'Text layout exceeds frame: {node["text"][:48]}'


def validate_layout(tree, width, height, language, theme):
    visual.validate(tree, 'about', width, height)
    scroll = tree.one('about-scroll')
    nodes = [scroll, *tree.descendants(scroll)]
    framed = [n for n in nodes if 'frame' in n and tree.visible(n)]
    for node in framed:
        parent = tree.nodes.get(node['parent'])
        if parent and parent['kind'] != 'Scroll' and 'frame' in parent:
            contains(tree.rect(parent), tree.rect(node), node.get('testId', node['id']))
        if node['kind'] == 'Text':
            text_fits(tree, node)
    sections = [tree.one(f'about-{part}-section') for part in ('version', 'engine', 'ack', 'privacy')]
    for a, b in zip(sections, sections[1:]):
        assert tree.rect(a)[3] <= tree.rect(b)[1] + 1, 'About sections overlap/reorder vertically'
    component_results = []
    for index, name in [(0, 'Mihomo'), (2, 'sing-box'), (4, 'meow-rs'), (6, 'Native Shell')]:
        component, title, version, actions = [tree.one(f'about-{part}-{index}') for part in ('component', 'name', 'version', 'actions')]
        assert title.get('text') == name
        assert title['frame'][2] >= 80 and 0 < title['frame'][3] <= 40, f'{name}: name squeezed into narrow/tall column'
        assert tree.rect(title)[3] <= tree.rect(version)[1] + 1, f'{name}: name/version overlap'
        assert tree.rect(version)[3] <= tree.rect(actions)[1] + 1, f'{name}: version/actions overlap'
        if index == 6:
            assert version.get('text') == NSH_VERSION, '191-character NSH revision was truncated before native mount'
        source, license = tree.one(f'about-source-{index}'), tree.one(f'about-license-{index}')
        for button in (source, license):
            assert button.get('events', 0) != 0, 'Link lost native event registration'
            assert button['frame'][3] >= 30, 'Link touch frame collapsed'
            contains(tree.rect(actions), tree.rect(button), 'About link actions')
            assert 'role: Button' in button.get('props', '')
            assert any(n.get('text') for n in tree.descendants(button)), 'Link label missing'
        no_overlap(tree.rect(source), tree.rect(license), name + ' link buttons')
        component_results.append({'name': name, 'nameFrame': title['frame'], 'versionChars': len(version['text']),
                                  'versionFrame': version['frame'], 'componentFrame': component['frame']})
    for parent_id, component_ids in [('about-engine-section', ['about-component-0', 'about-component-2', 'about-component-4']),
                                     ('about-ack-section', ['about-component-6', 'about-component-flclash'])]:
        parent = tree.one(parent_id)
        children = [tree.one(test_id) for test_id in component_ids]
        for child in children:
            contains(tree.rect(parent), tree.rect(child), parent_id)
        for a, b in zip(children, children[1:]):
            assert tree.rect(a)[3] <= tree.rect(b)[1] + 1, 'Component siblings overlap vertically'
    source, adaptations = tree.one('about-source-8'), tree.one('about-adaptations')
    no_overlap(tree.rect(source), tree.rect(adaptations), 'FlClash links')
    expected_label = 'Third-party components and adaptations' if language == 'en' else '第三方组件与适配说明'
    assert any(n.get('text') == expected_label for n in tree.descendants(adaptations)), 'Translated long adaptations label missing'
    for button in (source, adaptations, tree.one('about-privacy')):
        assert button.get('events', 0) != 0 and button['frame'][3] >= 30
    content_w, content_h = scroll['scrollContent']
    viewport = tree.rect(scroll)
    privacy = tree.rect(tree.one('about-privacy'))
    assert content_h > scroll['frame'][3], 'Long About fixture did not need scrolling'
    content_privacy = (privacy[0] - viewport[0], privacy[1] - viewport[1], privacy[2] - viewport[0], privacy[3] - viewport[1])
    contains((0, 0, content_w, content_h), content_privacy, 'Tail action in scroll extent')
    offset = content_h - scroll['frame'][3]
    assert 0 <= content_privacy[1] - offset and content_privacy[3] - offset <= scroll['frame'][3] + 1, 'Tail action cannot fit at maximum scroll offset'
    # Theme and language are driven through the actual fixture controls, not
    # inserted into an isolated imitation of About.
    cards = [n for n in tree.descendants(tree.one('about-ack-section')) if 'background_color: Color(' in n.get('style', '')]
    assert cards, 'Native About card style missing'
    color = re.search(r'background_color: Color\((\d+)\)', cards[0]['style'])
    return {'components': component_results, 'mountedAboutNodes': len(framed), 'nativeCardColor': color[1],
            'scrollViewport': scroll['frame'], 'scrollContent': scroll['scrollContent'],
            'tailAtMaximumOffset': [content_privacy[1] - offset, content_privacy[3] - offset],
            'tailReachability': 'geometry only; physical phone scrolling remains separate'}


def trace(binary, output, width, height, taps):
    command = [str(binary.resolve()), 'trace', ENTRY, '--width', str(width), '--height', str(height), '--wait', '30']
    for tap in taps:
        command.extend(['--tap', 'id:' + tap])
    run = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=45)
    output.write_text(run.stdout)
    assert run.returncode == 0, f'CLI failed: {output}'
    assert not re.search(r'(?im)^(?:error(?:\s|:)|uncaught exception(?:\s|:))', run.stdout), str(output)
    assert len(re.findall(r'perf event Press on view\b', run.stdout)) == len(taps), f'Missing actual Press: {output}'
    tree = AboutTree()
    for line in run.stdout.splitlines():
        tree.apply(line)
    return tree, run.stdout


def source_hashes():
    paths = [ROOT / ENTRY, Path(__file__).resolve(), VISUAL_READER, *sorted((ROOT / 'ui/nsh/src').rglob('*.ts')), *sorted((ROOT / 'ui/nsh/src').rglob('*.tsx'))]
    return {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest() for p in paths}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--nsh-bin', type=Path, default=Path(os.environ.get('NSH_BIN', ROOT / 'dist/nsh-sdk/release/nsh')))
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'target/nsh-about-tests')
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    inputs, binary_hash = source_hashes(), hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest()
    results = []
    colors = {}
    for width, language, theme in itertools.product((320, 390, 1280), ('zh', 'en'), ('light', 'dark')):
        height = 900 if width == 1280 else 844
        taps = (['about-preview-language'] if language == 'en' else []) + (['about-preview-theme'] if theme == 'dark' else [])
        taps += ['about-preview-hide', 'nav-tools', 'tool-about']
        path = args.output_dir / f'{width}-{language}-{theme}.log'
        tree, _ = trace(args.nsh_bin, path, width, height, taps)
        result = validate_layout(tree, width, height, language, theme)
        colors[(width, language, theme)] = result['nativeCardColor']
        results.append({'case': 'layout', 'width': width, 'height': height, 'language': language, 'theme': theme, 'log': str(path), **result})
        print(f'PASS About layout: {width} {language} {theme}', flush=True)
    for width, language in itertools.product((320, 390, 1280), ('zh', 'en')):
        assert colors[(width, language, 'light')] != colors[(width, language, 'dark')], 'Theme toggle did not reach actual About card'
    for language in ('zh', 'en'):
        taps = (['about-preview-language'] if language == 'en' else []) + ['nav-tools', 'tool-about']
        taps += [test_id for i in (0, 2, 4, 6) for test_id in (f'about-source-{i}', f'about-license-{i}')]
        taps += ['about-source-8', 'about-adaptations']
        path = args.output_dir / f'320-{language}-links.log'
        _, log = trace(args.nsh_bin, path, 320, 844, taps)
        actual = [int(n) for n in re.findall(r'PASS ABOUT LINK: (\d+)', log)]
        assert actual == list(range(10)), f'Wrong real CoreClient link command sequence: {actual}'
        results.append({'case': 'linkCommands', 'language': language, 'width': 320, 'indices': actual, 'log': str(path)})
        print(f'PASS About link commands: {language} (10 public fake-host URL receipts)', flush=True)
    for width in (320, 390, 1280):
        height = 900 if width == 1280 else 844
        path = args.output_dir / f'{width}-privacy-route.log'
        tree, _ = trace(args.nsh_bin, path, width, height, ['about-preview-hide', 'nav-tools', 'tool-about', 'about-privacy'])
        assert any(tree.visible(n) and n.get('text', '').startswith('Only public synthetic preview data.') for n in tree.nodes.values()), 'Privacy action did not navigate to real Privacy page'
        results.append({'case': 'privacyRoute', 'width': width, 'log': str(path), 'physicalScrollingProved': False})
        print(f'PASS About privacy route: {width}', flush=True)
    assert inputs == source_hashes(), 'UI/test source changed during traces'
    assert binary_hash == hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest(), 'Matching CLI changed during traces'
    summary = {'passed': len(results), 'results': results, 'sourceSha256': inputs, 'nshBinSha256': binary_hash,
               'sourceFenceStable': True, 'fixtureNshVersionChars': len(NSH_VERSION),
               'coverageBlindSpotCorrected': 'Prior horizontal-only About checks accepted a 10x169 Native Shell name column at 320px; these tests check vertical containment, sibling ordering, button labels, full long-version payloads and scroll extent.',
               'limits': ['Actual PawsApp/About/CoreClient with public synthetic fake host; no Core/network/VPN',
                          'ApproxTextEngine/native mount geometry does not certify physical glyph clipping, contrast, FPS or touch targeting',
                          'Source full text plus measured Text content bounds are checked; real device native glyph appearance still requires review',
                          'Offscreen selector taps prove command wiring, not physical scrolling; tail reachability is a scroll-extent calculation',
                          'Phone must independently scroll to acknowledgements and privacy bottom, inspect name/version and tap both links']}
    (args.output_dir / 'summary.json').write_text(json.dumps(summary, indent=2, ensure_ascii=False) + '\n')


if __name__ == '__main__':
    main()
