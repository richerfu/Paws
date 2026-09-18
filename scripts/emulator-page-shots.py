#!/usr/bin/env python3
"""Navigate Paws on a HarmonyOS emulator and capture one screenshot per page."""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

TARGET = os.environ.get("HDC_TARGET", "127.0.0.1:5555")
BUNDLE = "com.richerfu.paws"
ABILITY = "EntryAbility"
REMOTE_DIR = "/data/local/tmp/paws-ui"
OUT_DIR = Path(os.environ.get("PAWS_SHOT_DIR", "smoke-logs/ui-screenshots"))

BOUNDS_RE = re.compile(r"\[(-?\d+),(-?\d+)\]\[(-?\d+),(-?\d+)\]")


def hdc(*args: str, check: bool = True) -> subprocess.CompletedProcess[str]:
    cmd = ["hdc", "-t", TARGET, *args]
    return subprocess.run(cmd, check=check, text=True, capture_output=True)


def hdc_shell(command: str, check: bool = True) -> str:
    result = hdc("shell", command, check=check)
    return (result.stdout or "") + (result.stderr or "")


def wake_and_unlock() -> None:
    hdc_shell("power-shell wakeup", check=False)
    time.sleep(0.4)
    hdc_shell("uitest uiInput keyEvent Home", check=False)
    time.sleep(0.6)
    hdc_shell("uitest uiInput swipe 660 2100 660 700 800", check=False)
    time.sleep(0.8)


def start_app() -> None:
    hdc_shell(f"aa start -b {BUNDLE} -a {ABILITY}", check=False)
    time.sleep(2.5)


def dump_tree() -> dict:
    remote = f"{REMOTE_DIR}/layout.json"
    hdc_shell(f"mkdir -p {REMOTE_DIR}", check=False)
    hdc_shell(f"rm -f {remote}", check=False)
    hdc_shell(f"uitest dumpLayout -p {remote}")
    local = OUT_DIR / "_layout.json"
    hdc("file", "recv", remote, str(local))
    text = local.read_text(encoding="utf-8")
    return json.loads(text)


def iter_nodes(node: object):
    if isinstance(node, list):
        for item in node:
            yield from iter_nodes(item)
        return
    if not isinstance(node, dict):
        return
    yield node
    children = node.get("children") or node.get("child") or []
    if isinstance(children, list):
        for child in children:
            yield from iter_nodes(child)


def node_attrs(node: dict) -> dict:
    attrs = node.get("attributes") or node
    if isinstance(attrs, dict):
        return {str(k): "" if v is None else str(v) for k, v in attrs.items()}
    return {}


def parse_bounds(raw: str) -> tuple[int, int, int, int] | None:
    match = BOUNDS_RE.search(raw or "")
    if not match:
        return None
    left, top, right, bottom = map(int, match.groups())
    if right <= left or bottom <= top:
        return None
    return left, top, right, bottom


def node_center(node: dict) -> tuple[int, int] | None:
    bounds = parse_bounds(node_attrs(node).get("bounds", ""))
    if not bounds:
        return None
    left, top, right, bottom = bounds
    return (left + right) // 2, (top + bottom) // 2


def visible_text(node: dict) -> str:
    attrs = node_attrs(node)
    for key in ("text", "content", "description", "accessibilityText", "hint"):
        value = attrs.get(key, "").strip()
        if value:
            return value
    return ""


def find_text(tree: dict, text: str, *, contains: bool = False) -> dict | None:
    matches: list[tuple[int, dict]] = []
    for node in iter_nodes(tree):
        label = visible_text(node)
        if not label:
            continue
        if contains:
            ok = text in label
        else:
            ok = label == text
        if not ok:
            continue
        center = node_center(node)
        if center is None:
            continue
        bounds = parse_bounds(node_attrs(node).get("bounds", ""))
        area = 0 if bounds is None else (bounds[2] - bounds[0]) * (bounds[3] - bounds[1])
        matches.append((area, node))
    if not matches:
        return None
    matches.sort(key=lambda item: item[0])
    return matches[0][1]


def click_xy(x: int, y: int) -> None:
    hdc_shell(f"uitest uiInput click {x} {y}")
    time.sleep(0.9)


def click_text(text: str, *, contains: bool = False, required: bool = True) -> bool:
    tree = dump_tree()
    node = find_text(tree, text, contains=contains)
    if node is None:
        if required:
            labels = sorted(
                {
                    visible_text(item)
                    for item in iter_nodes(tree)
                    if visible_text(item)
                }
            )
            raise SystemExit(f"missing UI text {text!r}; visible={labels[:80]}")
        return False
    center = node_center(node)
    if center is None:
        if required:
            raise SystemExit(f"no bounds for {text!r}")
        return False
    click_xy(*center)
    return True


def go_back() -> None:
    hdc_shell("uitest uiInput keyEvent Back", check=False)
    time.sleep(0.8)


def capture(name: str) -> Path:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    remote = f"{REMOTE_DIR}/{name}.png"
    hdc_shell(f"rm -f {remote}", check=False)
    hdc_shell(f"uitest screenCap -p {remote}")
    local = OUT_DIR / f"{name}.png"
    if local.exists():
        local.unlink()
    hdc("file", "recv", remote, str(local))
    print(f"captured {local} ({local.stat().st_size} bytes)", flush=True)
    return local


def wait_for(text: str, attempts: int = 8) -> None:
    for _ in range(attempts):
        tree = dump_tree()
        if find_text(tree, text, contains=True) is not None:
            return
        time.sleep(0.6)
    raise SystemExit(f"timed out waiting for {text!r}")


def main() -> int:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    wake_and_unlock()
    start_app()
    wait_for("首页")
    time.sleep(0.8)

    capture("01-dashboard")

    click_text("订阅")
    wait_for("订阅")
    capture("02-profiles")

    if click_text("全部", required=False):
        wait_for("代理节点")
        capture("03-proxies")
        go_back()
        wait_for("订阅")
    else:
        print("skipping proxies: no 全部 entry", flush=True)

    click_text("流量")
    wait_for("流量")
    capture("04-traffic")

    click_text("设置")
    wait_for("设置")
    capture("05-tools")

    nested = [
        ("界面设置", "06-appearance"),
        ("网络设置", "07-settings"),
        ("订阅转化规则", "08-subscription-converter"),
        ("请求", "09-requests"),
        ("连接", "10-connections"),
        ("资源", "11-resources"),
        ("日志", "12-logs"),
        ("关于", "13-about"),
    ]
    for title, shot in nested:
        click_text("设置", required=False)
        wait_for("设置")
        click_text(title)
        wait_for(title)
        capture(shot)
        if title == "关于":
            if click_text("隐私与出口 IP", required=False) or click_text(
                "隐私", contains=True, required=False
            ):
                time.sleep(0.6)
                capture("14-privacy")
                go_back()
        go_back()
        wait_for("设置")

    print(f"screenshots written to {OUT_DIR.resolve()}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
