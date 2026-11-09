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
    for platform, width, height in [('ios', 320, 844), ('ios', 390, 844), ('ios', 1024, 768),
                                     ('windows', 1280, 900), ('windows', 620, 760)]:
        for page in ['home', 'nodes']:
            command = [str(args.nsh_bin.resolve()), 'trace', 'ui/nsh/tests/Preview.tsx',
                       '--platform', platform, '--width', str(width), '--height', str(height), '--wait', '30']
            if page == 'nodes':
                command += ['--tap', 'id:all-proxies']
            result = run_nsh(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=45)
            log = args.output_dir / f'{platform}-{width}-{page}.log'
            log.write_text(result.stdout)
            assert result.returncode == 0, log
            tree = reader.AboutTree()
            for line in result.stdout.splitlines():
                tree.apply(line)
            reader.visual.validate(tree, 'home', width, height)
            columns = platform == 'windows' and (page == 'nodes' or width >= 1100)
            if page == 'home':
                first, second = (tree.rect(tree.one(key)) for key in ['dashboard-status-panel', 'dashboard-groups-panel'])
            elif platform == 'windows':
                first, second = (tree.rect(tree.one(key)) for key in ['desktop-proxy-groups', 'proxy-groups-list'])
            else:
                tree.one('proxy-groups-list')
                assert not any(node.get('testId') == 'desktop-proxy-workspace' and tree.visible(node) for node in tree.nodes.values())
                first = second = None
            if first and columns:
                assert first[2] + 15 <= second[0], (platform, width, page, first, second)
                assert abs(first[1] - second[1]) <= 1, 'Desktop pane tops differ'
                assert second[2] <= width + 1
            elif first:
                assert abs(first[0] - second[0]) <= 1 and abs(first[2] - second[2]) <= 1
                assert first[3] <= second[1] + 1, 'Stacked panels overlap'
            results.append({'platform': platform, 'width': width, 'page': page, 'columns': columns,
                            'firstPane': first, 'secondPane': second})
            print('PASS workspace:', platform, width, page, 'columns' if columns else 'stack', flush=True)
    (args.output_dir / 'summary.json').write_text(json.dumps({'passed': len(results), 'results': results,
        'limits': 'Headless native frames; not OHOS pixels, mouse dispatch or frame cadence.'}, indent=2) + '\n')


if __name__ == '__main__':
    main()
