"""Measure production control sizes for Phone, Pad and desktop idioms."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
from nsh_runner import run_nsh, navigation_steps

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('density_tree', Path(__file__).with_name('run-about-tests.py'))
reader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'target/nsh-density-tests')
    parser.add_argument('--nsh-bin', type=Path, default=ROOT / 'dist/nsh-sdk/release/nsh')
    args = parser.parse_args()
    output = args.output_dir
    output.mkdir(parents=True, exist_ok=True)
    results = []
    for name, platform, width, height, button, line in [
        ('phone', 'ios', 390, 844, 40, 16),
        ('small-phone', 'ios', 320, 844, 40, 16),
        ('pad-portrait', 'ios', 768, 1024, 40, 16),
        ('pad-landscape', 'ios', 1024, 768, 40, 16),
        ('pc', 'windows', 1280, 900, 32, 17),
        ('pc-narrow', 'windows', 620, 760, 32, 17),
    ]:
        nav = ['desktop-nav-settings'] if platform == 'windows' else navigation_steps('settings', width)
        command = [str(args.nsh_bin.resolve()), 'trace', 'ui/nsh/tests/Preview.tsx',
                   '--platform', platform, '--width', str(width), '--height', str(height), '--wait', '30']
        for step in nav:
            command += ['--tap', 'id:' + step]
        result = run_nsh(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=45)
        (output / (name + '.log')).write_text(result.stdout)
        assert result.returncode == 0, name
        tree = reader.AboutTree()
        for record in result.stdout.splitlines():
            tree.apply(record)
        save, mtu, engine = (tree.one(key) for key in ['save-vpn', 'vpn-mtu', 'vpn-engine'])
        assert abs(save['frame'][3] - button) < 1, (name, 'button', save['frame'])
        assert abs(mtu['frame'][3] - (40 if platform == 'ios' else 36)) < 1, (name, 'input', mtu['frame'])
        assert abs(engine['frame'][3] - (40 if platform == 'ios' else 35)) < 1, (name, 'select', engine['frame'])
        label = next(node for node in tree.descendants(save) if node.get('text') == '保存 VPN 设置')
        assert abs(label['frame'][3] - line) < 1, (name, 'button text', label['frame'])
        reader.contains(tree.rect(save), tree.rect(label), 'Button label containment')
        sidebar = [n for n in tree.nodes.values() if n.get('testId') == 'desktop-sidebar' and tree.visible(n)]
        if platform == 'windows':
            assert len(sidebar) == 1, 'Desktop lost its sidebar at narrow width'
            assert not any(n.get('testId') == 'bottom-navigation' and tree.visible(n) for n in tree.nodes.values())
            assert sidebar[0]['frame'][2] == (232 if width >= 1100 else 64)
            if width < 1100:
                icon = next(node for node in tree.descendants(tree.one('nav-profiles')) if node['kind'] == 'Canvas')
                icon_rect, rail_rect = tree.rect(icon), tree.rect(sidebar[0])
                assert abs((icon_rect[0] + icon_rect[2]) - (rail_rect[0] + rail_rect[2])) < 1, 'Desktop rail icons are not centered'
        results.append({'device': name, 'buttonHeight': save['frame'][3], 'inputHeight': mtu['frame'][3],
                        'selectHeight': engine['frame'][3], 'measuredLabelHeight': label['frame'][3], 'sidebarWidth': sidebar[0]['frame'][2] if sidebar else None})
        print('PASS density:', name, results[-1], flush=True)
    (output / 'summary.json').write_text(json.dumps({'passed': len(results), 'results': results,
        'limits': 'Public headless native frames, not OHOS glyph pixels or frame rate.'}, indent=2) + '\n')


if __name__ == '__main__':
    main()
