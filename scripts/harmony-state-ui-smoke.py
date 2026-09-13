#!/usr/bin/env python3
"""Read-only page navigation and rendered-state acceptance on DevEco.

Does not install, clear app data, activate profiles or start VPN. The app may
persist its normal navigation-independent caches. VPN acceptance belongs on
QEMU; a successful screenshot is not by itself a successful assertion.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import time


TARGET = os.environ.get("HDC_TARGET", "127.0.0.1:5555")
BUNDLE = "com.richerfu.paws"
OUT = Path(os.environ.get("PAWS_SHOT_DIR", "smoke-logs/state-ui"))
REMOTE = f"/data/local/tmp/paws-state-ui-{time.time_ns()}"
LAYOUT_SEQUENCE = 0


def hdc(*args):
    result = subprocess.run(
        ["hdc", "-t", TARGET, *args], capture_output=True, text=True,
        timeout=40, check=True,
    )
    output = result.stdout + result.stderr
    if any(error in output for error in (
        "Connect server failed", "[Fail]", "[Error]", "Permission denied",
    )):
        raise RuntimeError(output)
    return output


def shell(*args):
    return hdc("shell", *args)


def nodes(value):
    if isinstance(value, dict):
        if "attributes" in value:
            yield value["attributes"]
        for child in value.values():
            yield from nodes(child)
    elif isinstance(value, list):
        for child in value:
            yield from nodes(child)


def layout(name="current"):
    global LAYOUT_SEQUENCE
    LAYOUT_SEQUENCE += 1
    remote = f"{REMOTE}/{name}-{LAYOUT_SEQUENCE}.json"
    local = OUT / f"{name}.json"
    output = shell("uitest", "dumpLayout", "-p", remote, "-a", "-b", BUNDLE)
    assert "DumpLayout saved to:" in output, output
    hdc("file", "recv", remote, str(local))
    tree = json.loads(local.read_text())
    result = list(nodes(tree))
    assert result, "No app UI nodes (app may be backgrounded)"
    labels = [node.get("text", "") for node in result]
    assert not any("Encountered panic" in label for label in labels), labels
    return result


def bounds(node):
    numbers = list(map(int, re.findall(r"-?\d+", node.get("bounds", ""))))
    return numbers if len(numbers) == 4 else None


def click(x, y):
    shell("uitest", "uiInput", "click", str(x), str(y))
    time.sleep(0.5)


def click_text(label, scroll=False):
    for attempt in range(4 if scroll else 1):
        candidates = []
        for node in layout():
            box = bounds(node)
            text = node.get("text", "")
            matches = text == label
            if scroll:
                # Settings section headings can have the same text as a
                # still-offscreen entry. Only its rendered Button is actionable.
                matches = node.get("type") == "Button" and (
                    text == label or text.startswith(label + ", ")
                )
            if matches and box and box[3] > box[1]:
                candidates.append(box)
        if candidates:
            # Settings has both an About heading and entry. The lower exact
            # match is the actionable entry; no synthetic coordinate guesses.
            left, top, right, bottom = max(candidates, key=lambda box: box[1])
            click((left + right) // 2, (top + bottom) // 2)
            return
        if attempt < 3 and scroll:
            shell("uitest", "uiInput", "swipe", "660", "2240", "660", "950", "600")
    raise AssertionError(f"Missing rendered control: {label}")


def capture(name, title):
    for _ in range(4):
        current = layout(name)
        if page_title(current) == title:
            break
        time.sleep(0.5)
    assert page_title(current) == title, (name, title, page_title(current))
    remote = f"{REMOTE}/{name}.jpeg"
    shell("snapshot_display", "-f", remote)
    hdc("file", "recv", remote, str(OUT / f"{name}.jpeg"))
    assert (OUT / f"{name}.jpeg").stat().st_size > 1000
    print(f"PASS {name}: {title}, no rendered panic", flush=True)


def page_title(current):
    for node in current:
        box = bounds(node)
        if node.get("type") == "Text" and box and 150 <= box[1] < 300:
            return node.get("text")
    return None


def back(expected_parents=("设置", "首页")):
    # Use the observed header back button. System Back can background the
    # Ability on this renderer and is not interchangeable with page navigation.
    current = layout()
    original_title = page_title(current)
    for _ in range(3):
        candidates = [bounds(node) for node in current
                      if node.get("type") == "Button" and bounds(node)]
        candidates = [box for box in candidates if box[0] < 100 and box[1] < 330]
        assert candidates, "Missing page header back button"
        left, top, right, bottom = min(candidates, key=lambda box: box[1])
        click((left + right) // 2, (top + bottom) // 2)
        current = layout()
        if page_title(current) != original_title:
            assert page_title(current) in expected_parents, page_title(current)
            return
    raise AssertionError(f"Header back did not leave {original_title}")


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    shell("mkdir", "-p", REMOTE)
    shell("aa", "force-stop", BUNDLE)
    shell("aa", "start", "-b", BUNDLE, "-a", "EntryAbility")
    time.sleep(2)
    capture("01-dashboard", "首页")
    if any(node.get("text") == "全部" for node in layout()):
        click_text("全部")
        capture("02-proxies", "代理节点")
        back()
    click_text("订阅")
    capture("03-profiles", "订阅")
    click_text("流量")
    capture("04-traffic", "流量")
    click_text("设置")
    capture("05-tools", "设置")
    for name, title in (
        ("06-appearance", "界面设置"), ("07-network", "网络设置"),
        ("08-converter", "订阅转化规则"), ("09-requests", "请求"),
        ("10-connections", "连接"), ("11-resources", "资源"),
        ("12-logs", "日志"), ("13-about", "关于"),
    ):
        click_text(title, scroll=True)
        capture(name, title)
        if title == "关于":
            click_text("隐私与出口 IP")
            capture("14-privacy", "隐私与出口 IP")
            back(("关于",))
        back()
    print(f"UI acceptance passed: {OUT.resolve()}", flush=True)


if __name__ == "__main__":
    main()
