#!/usr/bin/env python3
"""Exercise real Settings Field handlers and CoreClient payloads in an owned copy.

The CLI has no native text-input event flag. Only the copied Field's handler
registration is instrumented; this is not native typing, IME or device proof.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
from nsh_runner import run_nsh

ROOT = Path(__file__).resolve().parents[3]
UI = ROOT / "ui/nsh"
ENTRY = "tests/SettingsMtu.tsx"
spec = importlib.util.spec_from_file_location("mtu_native_tree", Path(__file__).with_name("run-visual-tests.py"))
reader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)
REGISTER = '''
// Test-only: record the exact handler assigned to the real Field's Input.
function registerSettingsMtuInput(change: (value: string) => void, editable: () => boolean) {
  const host = globalThis as typeof globalThis & {
    __pawsSettingsMtuInput?: { change: (value: string) => void; editable: () => boolean };
  };
  host.__pawsSettingsMtuInput = { change, editable };
  return change;
}
'''


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def sources():
    return sorted([*UI.glob("src/**/*.ts"), *UI.glob("src/**/*.tsx"),
                   UI / ENTRY, Path(__file__).resolve(), UI / "tests/DomainHost.ts",
                   UI / "tests/fixture.ts", UI / "tests/run-visual-tests.py"])


def hashes():
    return {str(path.relative_to(ROOT)): digest(path) for path in sources()}


def snapshots(output):
    tree, result, current = reader.NativeTree(), {}, 0
    for line in output.splitlines():
        header = re.search(r"PAWS TAP #(\d+)\b", line)
        if header and int(header[1]) != current:
            result[current] = copy.deepcopy(tree)
            current = int(header[1])
        tree.apply(line)
        item = re.search(r"Input \{ id: (\d+), config:", line)
        if item and item[1] in tree.nodes:
            tree.nodes[item[1]]["inputConfig"] = line
        value = re.search(r'InputValue \{ id: (\d+), text: ("(?:[^"\\]|\\.)*"), count:', line)
        if value and value[1] in tree.nodes:
            tree.nodes[value[1]]["inputValue"] = json.loads(value[2])
    result[current] = tree
    return result


def one(tree, test_id):
    nodes = [node for node in tree.nodes.values()
             if node.get("testId") == test_id and tree.visible(node)]
    assert len(nodes) == 1, f"Expected one mounted {test_id}, got {len(nodes)}"
    return nodes[0]


def state(tree, value, save_enabled, editable=True):
    field = one(tree, "vpn-mtu")
    assert field["kind"] == "Input" and field["frame"][2] > 0 and field["frame"][3] > 0
    assert field.get("inputValue") == value, (value, field.get("inputValue"))
    assert field.get("accessibilityLabel") == "MTU"
    assert 'placeholder: "1500"' in field["inputConfig"] and "keyboard: Number" in field["inputConfig"]
    assert f"editable: {str(editable).lower()}" in field["inputConfig"]
    assert f"disabled: {str(not save_enabled).lower()}" in one(tree, "save-vpn")["props"]
    assert one(tree, "engine-preview-status")["frame"][3] == 24
    assert not any(n.get("testId") == "engine-preview-reasons" and tree.visible(n)
                   for n in tree.nodes.values())


def run(binary, stage, output_dir, name, taps, checks, markers, expected_writes):
    cmd = [str(binary), "trace", str(stage / ENTRY), "--wait", "40", "--width", "390", "--height", "844"]
    for tap in taps:
        cmd.extend(["--tap", "id:" + tap])
    process = run_nsh(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, timeout=60)
    output = process.stdout
    log = output_dir / f"{name}.log"
    log.write_text(output)
    assert process.returncode == 0, f"{name}: CLI failed, inspect {log}"
    assert not re.search(r"(?im)^(?:error(?:\s|:)|fatal(?:\s|:)|uncaught exception(?:\s|:))", output)
    presses = len(re.findall(r"PAWS PRESS #\b", output))
    assert presses == len(taps), f"{name}: missing completed test step"
    for marker in markers:
        assert marker in output, f"{name}: missing {marker}"
    assert output.count("PASS MTU: actual settings.vpn payload mtu=2816 profile/revision preserved") == expected_writes
    states = snapshots(output)
    for index, value, enabled, editable in checks:
        state(states[index], value, enabled, editable)
    print(f"PASS Settings MTU: {name} ({presses} completed step; {expected_writes} settings.vpn request)")
    return {"case": name, "presses": presses, "settingsVpnRequests": expected_writes, "log": str(log)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nsh-bin", type=Path, default=Path(os.environ.get("NSH_BIN", ROOT / "dist/nsh-sdk/release/nsh")))
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/nsh-settings-mtu-tests")
    args = parser.parse_args()
    args.output_dir = args.output_dir.resolve()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    stage = args.output_dir / "owned-ui"
    assert not stage.exists(), f"Refusing to overwrite existing owned test copy: {stage}"
    before = hashes()
    shutil.copytree(UI, stage, ignore=shutil.ignore_patterns(".nsh", "__pycache__"))
    components = stage / "src/components.tsx"
    source = components.read_text()
    old = "          onChange={props.onChange}\n"
    new = '''          onChange={props.id === "vpn-mtu"
            ? registerSettingsMtuInput(props.onChange, () => !props.disabled)
            : props.onChange}
'''
    assert source.count(old) == 1, "Field Input handler registration span changed; refusing broad instrumentation"
    original_components_sha = digest(components)
    components.write_text(source.replace(old, new, 1) + REGISTER)
    assert digest(UI / "src/components.tsx") == original_components_sha
    nav = ["nav-tools", "tool-settings"]
    results = [
        run(args.nsh_bin.resolve(), stage, args.output_dir, "same-value-stays-clean",
            nav + ["mtu-same-initial", "mtu-assert-no-write"],
            [(2, "1500", False, True), (4, "1500", False, True)], ["PASS MTU: no-write"], 0),
        run(args.nsh_bin.resolve(), stage, args.output_dir, "range-and-integer-validation",
            nav + ["mtu-legacy-minimum", "mtu-below", "mtu-above", "mtu-fraction", "mtu-empty", "mtu-minimum", "mtu-maximum", "mtu-assert-no-write"],
            [(3, "576", False, True), (4, "1279", False, True), (5, "65536", False, True), (6, "1500.5", False, True),
             (7, "", False, True), (8, "1280", True, True), (9, "65535", True, True)], ["PASS MTU: no-write"], 0),
        run(args.nsh_bin.resolve(), stage, args.output_dir, "payload-and-authoritative-barrier",
            nav + ["mtu-2816", "save-vpn", "mtu-assert-write", "mtu-write-result", "mtu-assert-barrier",
                   "mtu-query-result", "mtu-assert-saved", "mtu-2816"],
            [(3, "2816", True, True), (5, "2816", False, False), (7, "2816", False, False),
             (9, "2816", False, True), (10, "2816", False, True)],
            ["PASS MTU: write", "PASS MTU: barrier", "PASS MTU: saved"], 1),
    ]
    assert before == hashes(), "Production/test source changed during the MTU traces"
    binary = args.nsh_bin.resolve()
    (args.output_dir / "summary.json").write_text(json.dumps({
        "results": results, "inputs": before, "productionSourcesUnchanged": True,
        "nshBin": str(binary), "nshBinSha256": digest(binary),
        "ownedCopy": str(stage), "instrumentedFile": str(components),
        "componentsOriginalSha256": original_components_sha,
        "componentsInstrumentedSha256": digest(components),
        "limits": ["Real Field Input.onChange handler, Settings, DraftEdits and CoreClient; controlled DomainHost receipts",
                   "Only the owned copy registers its exact native Input handler; original components and SDK are unchanged",
                   "Actual native mount configuration/Press and request payload; not native text input, IME, device MTU or traffic proof"]
    }, indent=2) + "\n")


if __name__ == "__main__":
    main()
