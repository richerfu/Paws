#!/usr/bin/env python3
"""Trace real refresh controls with held synthetic command/query receipts.

No Core, network, personal profiles, device or native service is used. Native
animation protocol operations are verified; physical rotation still needs OHOS.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
from nsh_runner import run_nsh

ROOT = Path(__file__).resolve().parents[3]
ENTRY = "ui/nsh/tests/Refresh.tsx"


class Tree:
    def __init__(self):
        self.props = {}
        self.children = {}

    def apply(self, line):
        insert = re.search(r"Insert \{ parent: (\d+), child: (\d+),", line)
        remove = re.search(r"(?:Remove|Delete) \{ id: (\d+) \}", line)
        props = re.search(r"ViewProps \{ id: (\d+), props:", line)
        if insert:
            self.children.setdefault(insert[1], set()).add(insert[2])
        elif remove:
            for children in self.children.values():
                children.discard(remove[1])
            if "Delete {" in line:
                self.props.pop(remove[1], None)
                self.children.pop(remove[1], None)
        elif props:
            self.props[props[1]] = line

    def node(self, test_id):
        matches = [node for node, props in self.props.items() if f'test_id: Some("{test_id}")' in props]
        if len(matches) != 1:
            raise AssertionError(f"Expected one actual mounted {test_id}: {matches}")
        return matches[0]

    def progress(self, node):
        return "role: Progressbar" in self.props.get(node, "") or any(self.progress(child) for child in self.children.get(node, ()))

    def control(self, test_id, busy):
        node = self.node(test_id)
        # nsh/base Button owns its disabled/selected accessibility state;
        # progress is represented by its actual progressbar child.
        assert self.progress(node) == busy, f"{test_id} actual Spinner differs"
        if busy:
            assert "disabled: true" in self.props[node], f"{test_id} still enabled while loading"


def snapshots(output):
    tree, result, current = Tree(), {}, 0
    for line in output.splitlines():
        # Engine perf stderr may share a line with the CLI mount-batch stdout.
        header = re.search(r"PAWS TAP #(\d+)\b", line)
        if header and int(header[1]) != current:
            result[current] = copy.deepcopy(tree)
            current = int(header[1])
        tree.apply(line)
    result[current] = tree
    return result


def run_case(binary, output_dir, name, taps, checks, markers=(), blocked=False):
    args = [str(binary), "trace", ENTRY, "--wait", "30", "--width", "390", "--height", "844"]
    for tap in taps:
        args.extend(["--tap", "id:" + tap])
    result = run_nsh(args, cwd=ROOT, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, timeout=45)
    output = result.stdout
    log = output_dir / (name + ".log")
    log.write_text(output)
    delivered = len(re.findall(r"PAWS PRESS #\b", output))
    if blocked:
        assert result.returncode != 0 and ("no pressable view matches" in output or "is disabled;" in output), f"{name}: loading control accepted a repeated press"
        assert delivered == len(taps), f"{name}: unexpected attempted step count"
        label = "action" if taps[0].endswith("action") else "icon"
        assert output.count(f"REFRESH SHARED: {label}=1") == 1 and f"REFRESH SHARED: {label}=2" not in output
    else:
        assert result.returncode == 0, f"{name}: CLI failed; see {log}"
        assert not re.search(r"(?im)^(?:error(?:\s|:)|fatal(?:\s|:)|uncaught exception(?:\s|:))", output), name
        assert delivered == len(taps), f"{name}: not every requested native Press was delivered"
        for marker in markers:
            assert marker in output, f"{name}: missing {marker}"
    states = snapshots(output)
    for index, test_id, busy in checks:
        states[index].control(test_id, busy)
    if name == "profile-failure-retry":
        for index, busy in [(4, True), (7, False), (9, True), (12, False)]:
            assert states[index].progress(states[index].node("activate-profile-test-profile")) == busy, "Per-profile leading Spinner did not follow the real receipt"
    if name == "all-profiles-config-gate":
        assert "disabled: true" in states[4].props[states[4].node("refresh-profiles")], "Header stayed enabled while its required config domain was stale"
    if name == "report-failure-retry":
        assert re.search(r'InputValue \{ id: \d+, text: "Synthetic report without personal data", count:', output), "Successful report receipt did not reach the actual native preview Input"
    starts = set(re.findall(r"Animation\(StartComposite \{ animation: (\d+),.*?spec: Loop", output))
    stops = set(re.findall(r"Animation\(StopAnimation \{ animation: (\d+) \}", output))
    if not blocked:
        assert starts and starts <= stops, f"{name}: native loop not stopped on result/failure/unmount: {starts - stops}"
    print(f"PASS refresh: {name} ({delivered}/{len(taps)} completed steps; {len(starts)} native loops)")
    return {"case": name, "log": str(log), "requestedPresses": len(taps), "deliveredPresses": delivered, "nativeLoops": len(starts), "expectedDuplicateBlock": blocked}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nsh-bin", type=Path, default=Path(os.environ.get("NSH_BIN", ROOT / "dist/nsh-sdk/release/nsh")))
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/nsh-refresh-tests")
    args = parser.parse_args()
    args.output_dir.mkdir(parents=True, exist_ok=True)
    hide = "refresh-hide-controls"
    cases = [
        ("shared-unmount", ["refresh-shared-action", "refresh-shared-icon", "refresh-assert-shared", hide],
         [(1, "refresh-shared-action", True), (2, "refresh-shared-icon", True)], ["PASS REFRESH: shared-one"]),
        ("shared-action-duplicate", ["refresh-shared-action", "refresh-shared-action"], [(1, "refresh-shared-action", True)], [], True),
        ("shared-icon-duplicate", ["refresh-shared-icon", "refresh-shared-icon"], [(1, "refresh-shared-icon", True)], [], True),
        ("all-profiles-barrier", [hide, "nav-profiles", "refresh-hold-query", "refresh-profiles", "refresh-assert-one", "refresh-release-success", "refresh-assert-barrier", "refresh-release-query", "refresh-assert-idle"],
         [(4, "refresh-profiles", True), (7, "refresh-profiles", True), (9, "refresh-profiles", False)], ["PASS REFRESH: barrier", "PASS REFRESH: idle"]),
        ("all-profiles-sync-warning", [hide, "nav-profiles", "refresh-profiles", "refresh-release-warning", "refresh-assert-warning"],
         [(3, "refresh-profiles", True), (5, "refresh-profiles", False)], ["PASS REFRESH: warning"]),
        ("all-profiles-config-gate", [hide, "nav-profiles", "refresh-hold-config", "refresh-assert-config-held", "refresh-release-query", "refresh-assert-idle", "refresh-profiles", "refresh-release-success", "refresh-assert-idle"],
         [(4, "refresh-profiles", False), (7, "refresh-profiles", True), (9, "refresh-profiles", False)], ["PASS REFRESH: config-held", "PASS REFRESH: idle"]),
        ("profile-failure-retry", [hide, "nav-profiles", "profile-menu-test-profile", "refresh-profile-test-profile", "refresh-assert-one", "refresh-release-failure", "refresh-assert-error", "profile-menu-test-profile", "refresh-profile-test-profile", "refresh-assert-one", "refresh-release-success", "refresh-assert-idle"],
         [], ["PASS REFRESH: error", "PASS REFRESH: idle"]),
        ("provider-identity", [hide, "nav-tools", "tool-resources", "refresh-provider-proxy-Test provider", "refresh-assert-one", "refresh-release-failure", "refresh-assert-error", "healthcheck-provider-Second provider", "refresh-assert-one", "refresh-release-success", "refresh-assert-idle"],
         [(4, "refresh-provider-proxy-Test provider", True), (4, "refresh-provider-proxy-Second provider", False), (4, "healthcheck-provider-Test provider", False), (7, "refresh-provider-proxy-Test provider", False),
          (8, "healthcheck-provider-Second provider", True), (8, "refresh-provider-proxy-Second provider", False), (8, "healthcheck-provider-Test provider", False), (11, "healthcheck-provider-Second provider", False)], ["PASS REFRESH: error", "PASS REFRESH: idle"]),
        ("diagnostics-refresh", [hide, "nav-tools", "tool-diagnostics", "refresh-assert-one", "refresh-release-success", "refresh-assert-health-initial", "refresh-checks", "refresh-assert-one", "refresh-release-failure", "refresh-assert-error", "refresh-checks", "refresh-release-success", "refresh-assert-health-refreshed"],
         [(3, "refresh-checks", True), (6, "refresh-checks", False), (7, "refresh-checks", True), (10, "refresh-checks", False), (11, "refresh-checks", True), (13, "refresh-checks", False)], ["PASS REFRESH: error", "PASS REFRESH: health-initial", "PASS REFRESH: health-refreshed"]),
        ("report-failure-retry", [hide, "nav-tools", "tool-advanced-diagnostics", "generate-report", "refresh-assert-one", "refresh-release-failure", "refresh-assert-error", "generate-report", "refresh-release-success", "refresh-assert-idle"],
         [(4, "generate-report", True), (7, "generate-report", False), (8, "generate-report", True), (10, "generate-report", False)], ["PASS REFRESH: error", "PASS REFRESH: idle"]),
        ("pending-page-unmount", [hide, "nav-tools", "tool-resources", "refresh-provider-proxy-Test provider", "refresh-hide-app", "refresh-release-success", "refresh-assert-idle"],
         [(4, "refresh-provider-proxy-Test provider", True)], ["PASS REFRESH: idle"]),
    ]
    results = [run_case(args.nsh_bin.resolve(), args.output_dir, *case) for case in cases]
    inputs = [ROOT / ENTRY, Path(__file__).resolve(), *[ROOT / "ui/nsh/src" / name for name in ["components.tsx", "app.tsx", "pages/profiles.tsx", "pages/resources.tsx", "pages/tools.tsx"]]]
    (args.output_dir / "summary.json").write_text(json.dumps({"entry": ENTRY, "nshBin": str(args.nsh_bin.resolve()),
        "nshBinSha256": hashlib.sha256(args.nsh_bin.read_bytes()).hexdigest(),
        "sourceSha256": {str(path.relative_to(ROOT)): hashlib.sha256(path.read_bytes()).hexdigest() for path in inputs}, "results": results,
        "limits": ["Actual components/CoreClient/DomainStore; synthetic server boundary only", "Native animation commands are checked; physical OHOS rotation/presentation is not a host-test claim", "No real profile/provider refresh or network activity"]}, indent=2) + "\n")


if __name__ == "__main__":
    main()
