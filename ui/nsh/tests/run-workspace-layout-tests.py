"""Check desktop workspaces and compact touch layouts using actual native frames."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
from nsh_runner import run_nsh

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('workspace_tree', Path(__file__).with_name('run-about-tests.py'))
reader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--nsh-bin', type=Path, default=ROOT / 'dist/nsh-sdk/release/nsh')
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'target/nsh-workspace-tests')
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    results = []
    devices = [('ios', width, 844, False) for width in [320, 360, 390, 430, 600, 840, 1024, 1280]]
    devices += [('ios', 840, 844, True)]
    devices += [('windows', width, height, False) for width, height in [(640, 480), (800, 600), (1100, 900), (1440, 900)]]
    for platform, width, height, toggle in devices:
        for page in ['home', 'nodes']:
            command = [str(args.nsh_bin.resolve()), 'trace', 'ui/nsh/tests/Preview.tsx',
                       '--platform', platform, '--width', str(width), '--height', str(height), '--wait', '30']
            if toggle:
                command += ['--tap', 'id:desktop-toggle-sidebar']
            if page == 'nodes':
                command += ['--tap', 'id:all-proxies']
            result = run_nsh(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=45)
            log = args.output_dir / f'{platform}-{width}-{page}{"-expanded" if toggle else ""}.log'
            log.write_text(result.stdout)
            assert result.returncode == 0, log
            tree = reader.AboutTree()
            for line in result.stdout.splitlines():
                tree.apply(line)
            reader.visual.validate(tree, 'home', width, height)
            sidebar = platform == 'windows' or width >= 840
            sidebar_width = (232 if width >= 1100 or toggle else 64) if sidebar else 0
            inset = 20 if sidebar else 16
            usable = min(width - sidebar_width, (960 if page == 'home' else 1120) if sidebar else 640) - inset * 2
            gap = (16 if usable >= 640 else 8) if sidebar else 12
            columns = sidebar and usable >= (620 if page == 'home' else 560) + gap
            if page == 'home':
                first, second = (tree.rect(tree.one(key)) for key in ['dashboard-status-panel', 'dashboard-groups-panel'])
            else:
                first = tree.rect(tree.one('desktop-proxy-groups'))
                second = tree.rect(tree.one('proxy-members-panel')) if columns else None
            if columns:
                assert first[2] + gap - 1 <= second[0], (platform, width, page, first, second)
                assert abs(first[1] - second[1]) <= 1, 'Pane tops differ'
                assert first[2] - first[0] >= (300 if page == 'home' else 240) - 1
                assert second[2] - second[0] >= 320 - 1, (log, 'detail minimum', second)
                assert second[2] <= width + 1
            elif second:
                assert abs(first[0] - second[0]) <= 1 and abs(first[2] - second[2]) <= 1
                assert first[3] <= second[1] + 1, 'Stacked panels overlap'
            else:
                assert first[2] - first[0] >= usable - 1
                assert not any(node.get('testId') == 'proxy-members-panel' and tree.visible(node) for node in tree.nodes.values())
            results.append({'platform': platform, 'width': width, 'height': height, 'sidebar': sidebar_width, 'page': page,
                            'columns': columns, 'firstPane': first, 'secondPane': second})
            print('PASS workspace:', platform, width, page, sidebar_width, 'columns' if columns else 'stack', flush=True)
    (args.output_dir / 'summary.json').write_text(json.dumps({'passed': len(results), 'results': results,
        'limits': 'Headless native frames; not OHOS pixels, mouse dispatch or frame cadence.'}, indent=2) + '\n')


if __name__ == '__main__':
    main()
