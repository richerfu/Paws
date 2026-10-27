"""Run existing fixture scenarios with the current public nsh/test API.

Mount items come from `nsh test --trace`; PAWS markers identify requested
steps, not performance events. A missing/disabled target rejects app.press
and fails the process. No SDK code or application handler is patched here.
"""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile


class TestStep(argparse.Action):
    def __call__(self, parser, namespace, values, option_string=None):
        namespace.steps = [*getattr(namespace, 'steps', []), (option_string, values)]


def run_nsh(command, **options):
    if len(command) < 2 or command[1] != 'trace':
        return subprocess.run(command, **options)
    parser = argparse.ArgumentParser()
    parser.add_argument('entry', type=Path)
    parser.add_argument('--width', type=int, default=390)
    parser.add_argument('--height', type=int, default=844)
    parser.add_argument('--wait', type=int, default=30)
    parser.add_argument('--tap', action=TestStep)
    parser.add_argument('--scroll', action=TestStep, nargs=2, metavar=('SELECTOR', 'DISTANCE'))
    parser.add_argument('--taps', type=int, default=0)
    args = parser.parse_args(command[2:])
    if args.taps:
        raise ValueError('Specify each actual test target with --tap')
    entry = args.entry.resolve(strict=True)
    steps = ['import { open, test } from "nsh/test";',
             'test("fixture acceptance", async () => {',
             'const controls = new Set(), replies = new Map();',
             f'const app = await open({json.dumps(str(entry))}, '
             f'{{width:{args.width},height:{args.height},locale:"zh-CN",'
             'host:{handlers:{"paws.test.register":id=>controls.add(id),'
             '"paws.test.done":reply=>replies.set(reply.token,reply)}}});',
             f'await app.wait({args.wait});']
    index = 0
    for kind, values in getattr(args, 'steps', []):
        if kind == '--scroll':
            selector, distance = values
            steps.append(f'await app.scroll({json.dumps(selector)}, {float(distance)});')
            steps.append(f'await app.wait({args.wait});')
            continue
        tap = values
        index += 1
        steps.extend([f'console.log("PAWS TAP #{index}");',
                      f'console.log("PAWS PRESS #{index}");',
                      f'if (controls.has({json.dumps(tap.removeprefix("id:"))})) {{',
                      f'await app.host.send("paws.test.action", {{id:{json.dumps(tap.removeprefix("id:"))},token:{index}}});',
                      f'await app.waitUntil(()=>replies.has({index}));',
                      f'if(replies.get({index}).error) throw new Error(replies.get({index}).error);',
                      f'}} else await app.press({json.dumps(tap)});',
                      f'await app.wait({args.wait});'])
    steps.append('});')
    directory = entry.parent / '.nsh/test'
    directory.mkdir(parents=True, exist_ok=True)
    with tempfile.NamedTemporaryFile(mode='w', suffix='.test.ts', dir=directory) as script:
        script.write('\n'.join(steps))
        script.flush()
        return subprocess.run([command[0], 'test', '--trace', script.name], **options)
