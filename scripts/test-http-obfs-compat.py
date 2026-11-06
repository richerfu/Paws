#!/usr/bin/env python3
"""Exercise real pinned HTTP-obfs/AEAD implementations without modifying a phone.

Source replacements exist only in a temporary Go module. Tests deliberately
reject TCP fragmentation bugs; this is a diagnostic gate, not an assertion
that either production kernel passed a full provider/VPN test.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--sing-box-root', type=Path,
                        default=os.environ.get('SING_BOX_ROOT') or ROOT / 'target/engine-sources/sing-box-ohos',
                        help='sing-box checkout (default: SING_BOX_ROOT or target/engine-sources/sing-box-ohos)')
    parser.add_argument('--mihomo-root', type=Path,
                        default=os.environ.get('CLASH_META_ROOT') or ROOT / 'target/engine-sources/Clash.Meta',
                        help='Mihomo checkout (default: CLASH_META_ROOT or target/engine-sources/Clash.Meta)')
    parser.add_argument('--timeout', type=int, default=180)
    parser.add_argument('--output', type=Path)
    parser.add_argument('--node-file', type=Path)
    parser.add_argument('--run', default='TestFirstRequest|TestFragmentedResponse')
    args = parser.parse_args()
    if not 1 <= args.timeout <= 600:
        parser.error('timeout must be 1..600 seconds')
    for path in (args.sing_box_root, args.mihomo_root):
        if not (path / 'go.mod').is_file():
            parser.error(f'missing source module: {path}')
    with tempfile.TemporaryDirectory(prefix='tinypaws-obfs-') as temp:
        work = Path(temp)
        (work / 'httpobfs').mkdir()
        (work / 'tlsretry').mkdir()
        shutil.copy(ROOT / 'vendor/sing-box-bridge/tlsretry/conn.go', work / 'tlsretry/conn.go')
        shutil.copy(ROOT / 'vendor/sing-box-bridge/httpobfs/conn.go', work / 'httpobfs/conn.go')
        for test in (ROOT / 'protocol-tests/http-obfs').glob('*.go'):
            shutil.copy(test, work / test.name)
        (work / 'go.mod').write_text(
            'module tinypaws.local/obfs-compat\n\ngo 1.24.7\n\nrequire (\n'
            ' github.com/sagernet/sing-box v1.13.21\n'
            ' github.com/metacubex/mihomo v1.10.0\n)\n\n'
            f'replace github.com/sagernet/sing-box => {json.dumps(str(args.sing_box_root.resolve()))}\n'
            f'replace github.com/metacubex/mihomo => {json.dumps(str(args.mihomo_root.resolve()))}\n'
        )
        started = datetime.now(timezone.utc).isoformat()
        result = subprocess.run(
            ['go', 'test', '-mod=mod', '-count=1', f'-timeout={min(120,args.timeout)}s', '-run', args.run, '-v', '.'],
            cwd=work, timeout=args.timeout, check=False, capture_output=True, text=True,
            env={**os.environ, **({"TINYPAWS_LIVE_NODE_PATH": str(args.node_file.resolve())} if args.node_file else {})},
        )
        print(result.stdout, end='')
        print(result.stderr, end='')
        if args.output:
            sources = {}
            for engine, source in [('mihomo', args.mihomo_root), ('sing-box', args.sing_box_root)]:
                sources[engine] = {
                    'gitRevision': subprocess.run(
                        ['git', '-C', str(source), 'rev-parse', 'HEAD'],
                        timeout=5, check=True, capture_output=True, text=True,
                    ).stdout.strip(),
                    'httpObfsSha256': hashlib.sha256(
                        (source / 'transport/simple-obfs/http.go').read_bytes()
                    ).hexdigest(),
                }
            report = {
                'format': 'TinyPaws-http-obfs-local-v1',
                'startedAt': started,
                'finishedAt': datetime.now(timezone.utc).isoformat(),
                'platform': platform.platform(),
                'scope': 'Host TCP syscall observation' if args.node_file else 'Deterministic local net.Conn and independent AES-GCM decoding; not live TCP packets',
                'providerValidated': False,
                'vpnValidated': False,
                'sources': sources,
                'dependencies': subprocess.run(
                    ['go', 'list', '-m', '-f', '{{.Path}} {{.Version}}',
                     'github.com/metacubex/sing-shadowsocks2',
                     'github.com/sagernet/sing-shadowsocks2',
                     'github.com/metacubex/sing', 'github.com/sagernet/sing'],
                    cwd=work, timeout=15, check=True, capture_output=True, text=True,
                ).stdout.splitlines(),
                'exitCode': result.returncode,
                'passed': result.returncode == 0,
                'testLog': result.stdout + result.stderr,
            }
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
        return result.returncode


if __name__ == '__main__':
    raise SystemExit(main())
