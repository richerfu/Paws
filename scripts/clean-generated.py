#!/usr/bin/env python3
"""Remove only disposable TinyPaws caches, after builds/tests have finished."""
import argparse
from datetime import datetime,timezone
import json
from pathlib import Path
import shutil

ROOT=Path(__file__).resolve().parents[1]
# No source, compiled libraries, dependency downloads, SDKs, geodata, or signing keys.
DISPOSABLE=[
 'target/debug/incremental',
 'target/aarch64-unknown-linux-ohos/debug/incremental',
 'target/aarch64-unknown-linux-ohos/release/incremental',
 'local-protocol-tests/target/debug/incremental',
 'local-protocol-tests/generated',
 'scripts/__pycache__',
]

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--apply',action='store_true')
 parser.add_argument('--manifest',type=Path,default=ROOT/'dist/TinyPaws-1.2.8-cleanup.json')
 args=parser.parse_args()
 if not (ROOT/'Cargo.lock').is_file() or not (ROOT/'entry/src/main/module.json5').is_file():
  parser.error('Expected TinyPaws workspace markers are missing')
 before=shutil.disk_usage(ROOT).free;items=[]
 for name in DISPOSABLE:
  path=ROOT/name
  if not path.exists():continue
  if path.is_symlink() or ROOT not in path.resolve().parents:raise ValueError('Refusing escaped/symlink cache path')
  size=sum(p.stat().st_size for p in path.rglob('*') if p.is_file() and not p.is_symlink()
           and not (name == 'local-protocol-tests/generated' and p.name == '.gitkeep'))
  items.append(dict(path=name,logicalBytes=size,reason='regenerable cache or expired local fixture',removed=args.apply))
  if args.apply:
   if name == 'local-protocol-tests/generated':
    for child in path.iterdir():
     if child.name == '.gitkeep': continue
     if child.is_dir() and not child.is_symlink():shutil.rmtree(child)
     else:child.unlink()
   else:shutil.rmtree(path)
 report=dict(format='TinyPaws-cleanup-v1',observedAt=datetime.now(timezone.utc).isoformat(),applied=args.apply,
             items=items,logicalBytes=sum(i['logicalBytes'] for i in items),freeSpaceDeltaBytes=shutil.disk_usage(ROOT).free-before,
             retained=['source and vendored patches','Cargo/ohpm locks and dependency downloads','release libraries and build outputs','geodata','external build toolchains and source checkouts','signing materials','final signed HAP and 1.2.7 rollback HAP'])
 if args.apply:
  args.manifest.parent.mkdir(parents=True,exist_ok=True);args.manifest.write_text(json.dumps(report,indent=2)+'\n')
 print(json.dumps(report,indent=2))
if __name__=='__main__':main()
