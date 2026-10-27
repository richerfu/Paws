#!/usr/bin/env python3
"""Real query-failure presentation: bounded text, full-source rows and Retry.

Headless native mount geometry is checked at the device scale, not OHOS pixels.
An owned copy captures only List.ref to drive the public scrollToIndex command.
"""
from __future__ import annotations

import argparse
import copy
import hashlib
import importlib.util
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
from nsh_runner import run_nsh

ROOT = Path(__file__).resolve().parents[3]
UI = ROOT / "ui/nsh"
ENTRY = "tests/PageError.tsx"
spec = importlib.util.spec_from_file_location("page_error_tree", Path(__file__).with_name("run-visual-tests.py"))
reader = importlib.util.module_from_spec(spec)
spec.loader.exec_module(reader)


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def hashes():
    sources = sorted([*UI.glob("src/**/*.ts"), *UI.glob("src/**/*.tsx"), UI / ENTRY,
                      Path(__file__).resolve(), UI / "tests/DomainHost.ts", UI / "tests/fixture.ts",
                      UI / "tests/run-visual-tests.py"])
    return {str(path.relative_to(ROOT)): sha(path) for path in sources}


def snapshots(output):
    tree, states, current = reader.NativeTree(), {}, 0
    for line in output.splitlines():
        # Mount Debug uses Rust's \u{...} for invisible Unicode; the shared
        # geometry reader decodes JSON string escapes.
        line = re.sub(r"\\u\{([0-9a-fA-F]+)\}",
                      lambda match: json.dumps(chr(int(match[1], 16)))[1:-1], line)
        header = re.search(r"PAWS TAP #(\d+)\b", line)
        if header and int(header[1]) != current:
            states[current] = copy.deepcopy(tree)
            current = int(header[1])
        tree.apply(line)
    states[current] = tree
    return states


def one(tree, test_id):
    found = [node for node in tree.nodes.values()
             if node.get("testId") == test_id and tree.visible(node)]
    assert len(found) == 1, (test_id, len(found))
    return found[0]


def verify_summary(tree, prefix="page-error"):
    summary = one(tree, prefix + "-summary")
    assert len(summary.get("text", "")) <= 321
    assert 0 < summary["frame"][3] <= 60
    one(tree, prefix + "-details")


def verify_detail(tree, width, tail, prefix="page-error"):
    viewport = one(tree, prefix + "-list")
    assert 0 < viewport["frame"][3] <= 320
    assert any(n["kind"] == "Scroll" for n in tree.descendants(viewport))
    chunks = [n for n in tree.descendants(viewport)
              if n.get("testId", "").startswith(prefix + "-chunk-") and "frame" in n]
    assert chunks, "Virtual list did not mount real error chunks"
    assert len(chunks) <= 28, "Retained list pool exceeded its minimum-row-height window bound"
    for node in chunks:
        body = node.get("text", "")
        assert len(body) <= 48
        assert not re.search(r"[\r\n\v\f\u0085\u2028\u2029]", body)
        w, h = node["frame"][2:]
        assert math.ceil(w * 3.25) * math.ceil(h * 3.25) < 4_194_304, (node["id"], w, h)
        left, right = tree.x_bounds(node)
        assert -1 <= left <= right <= width + 1
    if tail:
        ordered = sorted(chunks, key=lambda node: int(node["testId"].rsplit("-", 1)[1]))
        assert tail in "".join(n.get("text", "") for n in ordered), "Actual List tail command did not bind final source chunks"
    one(tree, prefix + "-copy")
    one(tree, prefix + "-close")
    return len(chunks)


def run(binary, stage, out, name, width, taps, summary_index, detail_index=None, tail_index=None):
    cmd = [str(binary), "trace", str(stage / ENTRY), "--wait", "50", "--width", str(width), "--height", "844"]
    for tap in taps:
        cmd.extend(["--tap", tap])
    process = run_nsh(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, timeout=75)
    output = process.stdout
    log = out / f"{name}-{width}.log"
    log.write_text(output)
    assert process.returncode == 0, f"CLI failed: {log}"
    assert not re.search(r"(?im)^(?:error(?:\s|:)|fatal(?:\s|:)|uncaught exception(?:\s|:))", output)
    assert len(re.findall(r"PAWS PRESS #\b", output)) == len(taps)
    assert "PASS PAGE ERROR: exact raw reconstruction and Unicode/hard-break bounds" in output
    assert "PASS PAGE ERROR: readonly Retry recovered without replaying select" in output
    if name != "initial-query-error":
        assert "PASS PAGE ERROR: real select receipt followed by failed proxies query" in output
    states = snapshots(output)
    verify_summary(states[summary_index])
    mounted = verify_detail(states[detail_index], width, None) if detail_index else 0
    if tail_index:
        verify_detail(states[tail_index], width, "END-UNICODE-ERROR" if name == "long-unicode-error" else "END-SYNTHETIC-STACK")
    final = states[len(taps)]
    assert not any(n.get("testId") == "page-error-summary" and final.visible(n) for n in final.nodes.values())
    print(f"PASS page error: {name} width={width} ({len(taps)} completed step)")
    return {"case": name, "width": width, "actualPresses": len(taps), "mountedChunksAtOpen": mounted, "log": str(log)}


def run_nested(binary, stage, out):
    taps = ["error-rules-failure", "nav-tools", "tool-connections", "rule-for-connection-1",
            "manual-rule-read-error-details", "manual-rule-read-error-close",
            "error-assert-nested-closed", "dialog-close", "error-assert-nested-closed"]
    cmd = [str(binary), "trace", str(stage / ENTRY), "--wait", "50", "--width", "390", "--height", "844"]
    for tap in taps:
        cmd.extend(["--tap", "id:" + tap])
    process = run_nsh(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, timeout=75)
    output = process.stdout
    log = out / "nested-dialog-readonly-close-390.log"
    log.write_text(output)
    assert process.returncode == 0, f"Nested CLI failed: {log}"
    assert not re.search(r"(?im)^(?:error(?:\s|:)|fatal(?:\s|:)|uncaught exception(?:\s|:))", output)
    assert len(re.findall(r"PAWS PRESS #\b", output)) == len(taps)
    assert output.count("PASS PAGE ERROR: nested details close does not retry or mutate") == 2
    states = snapshots(output)
    verify_summary(states[4], "manual-rule-read-error")
    chunks = verify_detail(states[5], 390, None, "manual-rule-read-error")
    verify_summary(states[7], "manual-rule-read-error")
    assert not any(n.get("testId") == "manual-rule-read-error-list" and states[7].visible(n)
                   for n in states[7].nodes.values())
    one(states[7], "dialog-close")
    assert not any(n.get("testId") == "manual-rule-read-error-summary" and states[9].visible(n)
                   for n in states[9].nodes.values())
    print("PASS page error: nested details close leaves parent, then parent closes (9 completed step)")
    return {"case": "nested-dialog-readonly-close", "width": 390, "actualPresses": len(taps),
            "mountedChunksAtOpen": chunks, "log": str(log)}


def run_window(binary, stage, out, width, scale, dynamic=False):
    """Exercise retained cells, including short->long rows and repeated scroll.

    The owned copy emulates platform font multiplication in the same Text style;
    this is headless scaled-layout evidence, not an OHOS system-font-scale test.
    """
    taps = ["error-qa-unbroken", f"error-scale-{1 if dynamic else scale}",
            "all-proxies", "group-Proxy", "select-proxy-Test node",
            "error-assert-selected-failure", "page-error-details"]
    if dynamic:
        taps += ["error-scan"] * 12 + [f"error-scale-{scale}"]
    taps += ["error-scan"] * 28 + ["error-tail", "page-error-copy", "page-error-close",
             "error-allow", "page-data-retry", "error-assert-recovered"]
    cmd = [str(binary), "trace", str(stage / ENTRY), "--wait", "50", "--width", str(width), "--height", "844"]
    for tap in taps:
        cmd.extend(["--tap", "id:" + tap])
    process = run_nsh(cmd, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, timeout=90)
    log = out / f"retained-window-{width}-scale-{scale}{'-dynamic' if dynamic else ''}.log"
    log.write_text(process.stdout)
    assert process.returncode == 0, log
    output = process.stdout
    assert not re.search(r"(?im)^(?:error(?:\s|:)|fatal(?:\s|:)|uncaught exception(?:\s|:))", output)
    assert len(re.findall(r"PAWS PRESS #\b", output)) == len(taps)
    assert "PASS PAGE ERROR: readonly Retry recovered without replaying select" in output
    states = snapshots(output)
    peak_count = peak_height = peak_reservation = 0
    for number, tree in states.items():
        viewports = [n for n in tree.nodes.values() if n.get("testId") == "page-error-list" and tree.visible(n)]
        if not viewports:
            continue
        chunks = [n for n in tree.descendants(viewports[0])
                  if n.get("testId", "").startswith("page-error-chunk-") and "frame" in n]
        assert len(chunks) <= 28, (log, number, len(chunks))
        reserved = 0
        for node in chunks:
            w, h = node["frame"][2:]
            body = node.get("text", "")
            assert len(body) <= 48 and 40 <= h <= 60 * scale + 1, (log, number, w, h, body)
            assert w <= min(width - 66, 446) + 1, (log, number, w)
            # Count every retained descendant, including offscreen cells. This
            # is only the latest headless map, not unpublished/native Arc peaks.
            units = len(body.encode("utf-16-le")) // 2
            reserved += math.ceil(w * 3.25) * math.ceil(h * 3.25) * 12 + len(body.encode()) + units * 512
        peak_count = max(peak_count, len(chunks))
        peak_height = max(peak_height, *(n["frame"][3] for n in chunks))
        peak_reservation = max(peak_reservation, reserved)
    assert peak_count > 4, "Trace did not exercise a retained recycling pool"
    final_detail = states[len(taps) - 5]
    viewport = one(final_detail, "page-error-list")
    chunks = [n for n in final_detail.descendants(viewport) if n.get("testId", "").startswith("page-error-chunk-")]
    expected_last = 24 + math.ceil(4608 / max(1, math.floor(48 / scale))) - 1
    assert any(n.get("testId") == f"page-error-chunk-{expected_last}" for n in chunks), (log, expected_last)
    result = {"case": "retained-window", "width": width, "scale": scale,
              "dynamicScaleAfterOpen": dynamic, "actualPresses": len(taps),
              "peakRetainedChunks": peak_count, "maxApproxRowHeightVp": peak_height,
              "peakLatestDetailReservationEstimateBytes": peak_reservation, "log": str(log)}
    print(f"PASS retained window: {width} scale={scale} dynamic={dynamic}, {peak_count} cells, {peak_height}vp row")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nsh-bin", type=Path, default=Path(os.environ.get("NSH_BIN", ROOT / "dist/nsh-sdk/release/nsh")))
    parser.add_argument("--output-dir", type=Path, default=ROOT / "target/nsh-page-error-tests")
    args = parser.parse_args()
    out = args.output_dir.resolve()
    out.mkdir(parents=True, exist_ok=True)
    stage = out / "owned-ui"
    assert not stage.exists(), f"Refusing to overwrite owned copy {stage}"
    before = hashes()
    shutil.copytree(UI, stage, ignore=shutil.ignore_patterns(".nsh", "__pycache__"))
    page = stage / "src/page-error.tsx"
    original_sha = sha(page)
    source = page.read_text()
    old = "            <List\n              bounces={false}\n"
    new = '''            <List
              ref={(node) => {
                (globalThis as typeof globalThis & { __pawsPageErrorList?: import("nsh").YNode })
                  .__pawsPageErrorList = node;
              }}
              bounces={false}
'''
    assert source.count(old) == 1, "List ref span changed; refusing broad instrumentation"
    page.write_text(source.replace(old, new, 1))
    assert sha(UI / "src/page-error.tsx") == original_sha
    nav = ["id:all-proxies", "id:group-Proxy", "id:select-proxy-Test node", "id:error-assert-selected-failure"]
    recover = ["id:dialog-close", "id:error-allow", "id:page-data-retry", "id:error-assert-recovered"]
    results = []
    for width in (320, 390, 1280):
        results.append(run(args.nsh_bin.resolve(), stage, out, "stack-2134-error", width,
            nav + ["id:page-error-details", "id:page-error-copy", "id:error-tail"] + recover, 4, 5, 7))
        results.append(run(args.nsh_bin.resolve(), stage, out, "long-unicode-error", width,
            ["id:error-long"] + nav + ["id:page-error-details", "id:error-tail"] + recover, 5, 6, 7))
    results.append(run(args.nsh_bin.resolve(), stage, out, "initial-query-error", 390,
        ["id:error-initial-failure", "id:all-proxies", "id:page-error-details", "id:dialog-close",
         "id:error-allow", "id:page-data-retry", "id:error-assert-recovered"], 2, 3))
    results.append(run_nested(args.nsh_bin.resolve(), stage, out))
    scaled_stage = out / "owned-scaled-ui"
    shutil.copytree(stage, scaled_stage, ignore=shutil.ignore_patterns(".nsh", "__pycache__"))
    scaled_page = scaled_stage / "src/page-error.tsx"
    scaled_source = scaled_page.read_text()
    replacements = [
        ('  fontScale,\n', '  fontScale as nativeFontScale,\n'),
        ('const SUMMARY_SCALARS = 160;',
         '''const SUMMARY_SCALARS = 160;
// Owned test copy only: emulate native font multiplication at fontScale=1.
const fontScale = signal(1);
(globalThis as typeof globalThis & { __pawsPageErrorSetScale?: (scale: number) => void })
  .__pawsPageErrorSetScale = (scale) => fontScale.set(scale);'''),
        ('minHeight: rowHeight(),\n                    fontSize: 13,',
         'minHeight: rowHeight(),\n                    fontSize: 13 * fontScale(),'),
        ('lineHeight: DETAIL_LINE_HEIGHT,', 'lineHeight: DETAIL_LINE_HEIGHT * fontScale(),'),
    ]
    for old, new in replacements:
        assert scaled_source.count(old) == 1, "Scaled-layout span changed; refusing broad instrumentation"
        scaled_source = scaled_source.replace(old, new, 1)
    scaled_page.write_text(scaled_source)
    for width in (320, 390):
        for scale in (1, 2, 3):
            results.append(run_window(args.nsh_bin.resolve(), scaled_stage, out, width, scale))
    results.append(run_window(args.nsh_bin.resolve(), scaled_stage, out, 320, 3, dynamic=True))
    assert before == hashes(), "Production UI/test source changed during traces"
    binary = args.nsh_bin.resolve()
    (out / "summary.json").write_text(json.dumps({"results": results, "inputs": before,
        "sourceFenceUnchanged": True, "nshBin": str(binary), "nshBinSha256": sha(binary),
        "ownedCopy": str(stage), "pageErrorOriginalSha256": original_sha,
        "pageErrorWithRefSha256": sha(page), "limits": [
            "Real production app/client receives query failure after an actual selected-node command",
            "Only owned copied List.ref is captured to send the public scrollToIndex command",
            "Headless geometry at scale 3.25 is a budget estimate, not OHOS raster/native scrolling/clipboard proof",
            "Scaled-layout tests emulate font multiplication in an owned copy; they are not system font-scale delivery proof",
            "Retained-cell estimates omit background UI, unpublished frames and native/selection lifetime overlap",
            "Ordinary Text backend full-block raster fatal behavior remains unchanged"
        ], "scaledCopy": str(scaled_stage), "scaledPageErrorSha256": sha(scaled_page),
        "scaledCopyExactReplacements": [old for old, _ in replacements]}, indent=2) + "\n")


if __name__ == "__main__":
    main()
