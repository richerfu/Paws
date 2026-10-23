#!/usr/bin/env python3
"""Run the real NSH type checker and headless UI acceptance entries."""

from __future__ import annotations

import argparse
import os
from pathlib import Path
import re
import subprocess
import sys
import time


ROOT = Path(__file__).resolve().parents[2]
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
RUNTIME_ERROR = re.compile(r"(?im)^(?:error(?:\s|:)|fatal(?:\s|:)|uncaught exception(?:\s|:))")
PRESS = re.compile(r"(?m)^perf event Press on view\b")


def cases(log_dir):
    import_taps = ["id:nav-profiles", "id:add-profile", "id:dialog-close", "id:hold-profiles"]
    for _ in range(10):
        import_taps.extend(["id:add-profile", "id:dialog-close"])
    import_taps.extend(["id:hold-config", "id:add-profile", "id:assert-import-blocked", "id:dialog-close", "id:release-import-reads", "id:assert-import-ready"])
    phone_taps = [
        "id:vpn-toggle", "id:vpn-toggle", "id:nav-profiles",
        "id:profile-menu-test-profile", "id:edit-subscription-test-profile", "id:save-subscription",
        "id:profile-menu-test-profile", "id:profile-export-test-profile", "label:导出 YAML 文件",
        "id:profile-qr-test-profile", "id:export-profile-qr", "id:dialog-close",
        "id:nav-dashboard", "id:all-proxies", "id:group-Proxy", "id:select-proxy-Test node", "id:select-proxy-Test node", "label:测试延迟",
        "id:nav-back", "id:nav-tools", "id:tool-connections", "id:rule-for-connection-1", "id:save-manual-rule",
        "id:nav-back", "id:tool-logs", "id:log-2026-10-21T00:00:00Z", "id:dialog-close", "id:logs-tab-history",
        "id:nav-back", "id:tool-about", "label:隐私说明", "id:nav-back", "id:nav-back",
        "id:tool-converter", "id:converter-build",
        "id:nav-back", "id:nav-profiles", "id:profile-menu-test-profile", "id:edit-yaml-test-profile", "id:validate-profile", "id:dialog-close", "id:add-profile", "id:dialog-close", "id:add-profile", "id:dialog-close",
    ]
    desktop_taps = [
        "id:nav-tools", "id:tool-resources", "id:rule-down-rule-1",
        "label:详情", "label:测速", "id:dialog-close",
        "label:GeoIP", "id:dialog-close", "id:nav-back",
        "id:tool-appearance", "label:深色", "label:浅色",
        "id:nav-back", "id:tool-resources", "id:lookup-rule", "id:dialog-close", "id:add-rule", "id:dialog-close", "id:lookup-rule", "id:dialog-close",
    ]

    def wire(taps, width, height):
        args = [
            "trace", "ui/nsh/tests/Preview.tsx", "--wait", "25",
            "--width", str(width), "--height", str(height),
        ]
        for selector in taps:
            args.extend(["--tap", selector])
        return args

    return [
        (
            "import-lifetime", ["trace", "ui/nsh/tests/ImportLifecycle.tsx", "--wait", "10", *[arg for tap in import_taps for arg in ("--tap", tap)]],
            ("PASS: cached profiles remains visible while its query is pending", "PASS: opening import does not bypass config freshness at submission", "PASS: import Dialog close and reopen preserve its page owner and pending reads"), len(import_taps),
        ),
        ("gateway-queue", ["trace", "ui/nsh/tests/GatewayQueue.tsx", "--wait", "120", "--tap", "id:queue-start-stop", "--tap", "id:queue-import-cancel"], ("PASS: native gateway ACK delivers synchronous start/stop and import/cancel independently of completion",), 2),
        ("check", ["check", "ui/nsh"], (), None),
        ("production-entry", ["build", "ui/nsh", "-o", str(log_dir / "production-entry")], (), None),
        (
            "state", ["trace", "ui/nsh/tests/State.tsx", "--wait", "120", "--taps", "0"],
            (
                "PASS: original control semantics, observer errors and VPN cancellation",
                "PASS: converter delayed save, parse conflict, retry and failure preserve edits",
                "PASS: VPN deadline retains receipt, blocks repeat start and accepts late receipt or explicit stop",
                "PASS: late durable VPN effect becomes unconfirmed and cannot replace a newer Stop",
                "PASS: query interests, generations, coalescing, loading and committed-result barriers preserve state without aggregate pushes",
            ), 0,
        ),
        (
            "lifetime", ["trace", "ui/nsh/tests/Lifetime.tsx", "--wait", "80", "--taps", "0"],
            (
                "PASS: disposed page drops asynchronous results",
                "PASS: disposed converter drops delayed native parse receipt",
            ), 0,
        ),
        (
            "phone-wire", wire(phone_taps, 390, 844),
            tuple(f"PASS WIRE: {command}" for command in (
                "vpn.start", "vpn.stop", "profiles.subscription", "profiles.read",
                "platform.qr", "platform.exportQr",
                "proxies.select", "proxies.unfix", "proxies.testGroup", "rules.add", "logs.recordingStatus", "converter.buildUrl", "converter.saveDraft", "profiles.validate",
            )), len(phone_taps),
        ),
        (
            "desktop-wire", wire(desktop_taps, 1280, 900),
            ("PASS WIRE: rules.reorder", "PASS WIRE: providers.testProxy", "PASS WIRE: preferences.set"), len(desktop_taps),
        ),
        (
            "widgets", ["trace", "ui/nsh/tests/Widgets.tsx", "--wait", "300", "--width", "390", "--height", "844",
                "--tap", "label:Short options", "--tap", "id:select-option-two", "--tap", "label:Long options", "--tap", "id:select-dismiss",
                "--tap", "id:notify-one", "--tap", "id:dismiss-toast-1", "--tap", "id:notify-many",
                "--tap", "id:show-dialog", "--tap", "id:notify-dialog", "--tap", "id:dialog-close"], (), 10,
        ),
        (
            "notifications", ["trace", "ui/nsh/tests/Notifications.tsx", "--wait", "2800", "--taps", "0"],
            ("PASS: original Sonner queue is bounded and all entries expire independently",), 0,
        ),
        (
            "interests", ["trace", "ui/nsh/tests/Interests.tsx", "--wait", "40", "--width", "390", "--height", "844",
                "--tap", "id:summary-group-Proxy", "--tap", "id:summary-group-Proxy", "--tap", "id:all-proxies", "--tap", "id:group-Proxy", "--tap", "id:nav-back",
                "--tap", "id:nav-tools", "--tap", "id:tool-resources", "--tap", "id:nav-back", "--tap", "id:tool-appearance", "--tap", "id:nav-back", "--tap", "id:nav-dashboard", "--tap", "id:interest-tick"],
            ("PASS DOMAIN: proxies[Proxy]", "PASS: production Stack forward/back releases detail interests and Home only requeries its cheap summary"), 12,
        ),
    ]


def validate_output(name, output, markers, expected_taps):
    if RUNTIME_ERROR.search(output):
        raise ValueError(f"{name}: NSH reported a runtime error")
    # Engine console output and native mount tracing can write concurrently;
    # the CLI occasionally appends a mount operation to the same console line.
    missing = [marker for marker in markers if marker not in output]
    if missing:
        raise ValueError(f"{name}: missing acceptance marker(s): {', '.join(missing)}")
    if expected_taps is not None:
        taps = len(PRESS.findall(output))
        if taps != expected_taps:
            raise ValueError(f"{name}: expected {expected_taps} delivered presses, observed {taps}")
    if name == "import-lifetime":
        modals = re.findall(r"Create \{[^\n]*kind: Modal", output)
        if len(modals) != 12:
            raise ValueError(f"import-lifetime: expected twelve actual import Dialog mounts, observed {len(modals)}")
    if name == "gateway-queue":
        delivered = re.findall(r'ComponentProps.*?"id": Number\((\d+)\).*?"command": String\("([^\"]+)"\)', output)
        if delivered != [("1", "state.subscribe"), ("2", "state.query"), ("3", "state.query"), ("4", "vpn.start"), ("5", "vpn.stop"), ("6", "profiles.importUrl"), ("7", "profiles.cancelImport")]:
            raise ValueError(f"gateway-queue: missing, duplicate or reordered native request props: {delivered}")
        if output.count('"intentId": Number(1)') != 2:
            raise ValueError("gateway-queue: import and cancellation did not share the native intent token")
    if name == "widgets":
        presses = re.split(r"(?m)^perf event Press on view\b", output)[1:]
        if "kind: Scroll" in presses[0] or presses[2].count("kind: Scroll") != 1:
            raise ValueError("widgets: short Select scrolled or long Select lost its scroll viewport")
        if 'placeholder: "https://example.com/profile"' not in output or "line_height: 20.0" not in output:
            raise ValueError("widgets: native input lost its placeholder or original text metrics")
        if 'test_id: Some("sonner-viewport")' not in output or "Feedback above the dialog" not in output:
            raise ValueError("widgets: notification did not reach the mounted Sonner/dialog layer")
        if 'bounces: true' in output:
            raise ValueError("widgets: Select enabled an unrequested overscroll interaction")
    if name == "notifications":
        painted = set(re.findall(r'test_id: Some\("toast-(\d+)"\)', output))
        if painted != {"38", "39", "40"}:
            raise ValueError(f"notifications: hidden queued entries flashed outside the three-card cap: {painted}")
    if name == "interests":
        actual = re.findall(r"PASS INTEREST: ([a-zA-Z,\[\]]+)", output)
        expected = ["config,status,summary", "config,proxies[Proxy],status,summary", "config,status,summary", "config,proxies,status", "config,status,summary", "config,status", "config,geodata,providers,rules,status,summary", "config,status", "config,status,summary"]
        if actual != expected:
            raise ValueError(f"interests: retained routes or dialogs changed the requested domain scopes: {actual}")
        if output.count("PASS DOMAIN: proxies[Proxy]") != 1 or len(re.findall(r"(?m)^PASS DOMAIN: proxies\s*$", output)) != 1:
            raise ValueError("interests: Home expansion did not use one scoped query or Proxies did not use one full-roster query")
        if 'test_id: Some("summary-select-proxy-Test node")' not in output:
            raise ValueError("interests: scoped Home query did not mount its actual node row")
    if name == "desktop-wire":
        validate_resource_dialogs(output)
        # Actual native mounts for the dark/light presses: only the selected
        # radio dot changes identity. Translated option rows must stay mounted.
        presses = re.split(r"(?m)^perf event Press on view\b", output)[1:]
        for index in (10, 11):
            if presses[index].count("Create {") > 1 or presses[index].count("Delete {") > 1:
                raise ValueError("desktop-wire: theme change recreated translated control rows")
    if name == "phone-wire":
        validate_proxy_modes(output)
        for command, count in (("profiles.read", 2), ("converter.loadDraft", 1), ("logs.recordingStatus", 2)):
            if output.count(f"PASS WIRE: {command}") != count:
                raise ValueError(f"phone-wire: native wrapper duplicated {command} page initialization")
        latency = output.split("PASS WIRE: proxies.testGroup", 1)[-1].split("perf event Press", 1)[0]
        if "role: Progressbar" not in latency:
            raise ValueError("phone-wire: latency test did not mount its pending Spinner")
        if "data:image/svg+xml" in output:
            raise ValueError("phone-wire: OHOS bitmap decoding cannot render SVG icon images")


def validate_proxy_modes(output):
    """Assert original product navigation and pin/unpin on actual mounted nodes."""
    controls = {}
    text_by_id = {}
    press = 0
    for line in output.splitlines():
        props = re.search(r'ViewProps \{ id: (\d+), .*test_id: (?:Some\("([^\"]+)"\)|None)', line)
        deleted = re.search(r'^\s+Delete \{ id: (\d+) \}', line)
        text = re.search(r'^\s+Text \{ id: (\d+), text: "(.*)", content:', line)
        if props:
            controls.pop(props[1], None)
            if props[2]:
                controls[props[1]] = props[2]
        elif deleted:
            controls.pop(deleted[1], None)
            text_by_id.pop(deleted[1], None)
        elif text:
            text_by_id[text[1]] = text[2]
        elif PRESS.match(line):
            press += 1
            if press in (15, 16, 17, 18, 22, 23, 26, 27, 28, 31, 35):
                if "bottom-navigation" in controls.values():
                    raise ValueError(f"phone-wire: secondary page exposed the bottom nav before press {press}")
            if press in (16, 17, 18):
                pinned = "恢复自动" in text_by_id.values()
                if pinned != (press == 17):
                    raise ValueError(f"phone-wire: automatic/pinned member mismatch before press {press}")
            if press == 23 and "DOMAIN,api.example.com,DIRECT" not in text_by_id.values():
                raise ValueError("phone-wire: activity rule dialog did not preserve the domain preview")
            if press == 24 and not any("规则已保存，但关联连接未能断开：Fixture close unavailable" in value for value in text_by_id.values()):
                raise ValueError("phone-wire: partial rule success did not show its close warning")
            if press == 27 and "Fixture runtime ready" not in text_by_id.values():
                raise ValueError("phone-wire: full log detail was not mounted")
            if press in (43, 45) and ("网络导入" not in text_by_id.values() or "配置内容" in text_by_id.values()):
                raise ValueError("phone-wire: import dialog diverged from the original network-only flow")
            if press == 32 and "Test privacy summary" not in text_by_id.values():
                raise ValueError("phone-wire: independent privacy page did not show the privacy disclosure")


def validate_resource_dialogs(output):
    controls = {}
    press = 0
    for line in output.splitlines():
        props = re.search(r'ViewProps \{ id: (\d+), .*test_id: (?:Some\("([^\"]+)"\)|None)', line)
        deleted = re.search(r'^\s+Delete \{ id: (\d+) \}', line)
        if props:
            controls.pop(props[1], None)
            if props[2]:
                controls[props[1]] = props[2]
        elif deleted:
            controls.pop(deleted[1], None)
        elif PRESS.match(line):
            press += 1
            expected = "rule-lookup-query" if press in (16, 20) else "manual-rule-value" if press == 18 else None
            if expected and expected not in controls.values():
                raise ValueError(f"desktop-wire: resource intent did not mount {expected} before press {press}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--nsh-bin", default=os.environ.get("NSH_BIN", str(ROOT / "dist/nsh-sdk/release/nsh")))
    parser.add_argument("--log-dir", type=Path, default=ROOT / "target/nsh-ui-tests")
    parser.add_argument("--timeout", type=float, default=90, help="Per-command timeout in seconds")
    options = parser.parse_args()
    binary = Path(options.nsh_bin).expanduser()
    if not binary.is_absolute():
        binary = ROOT / binary
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error(f"NSH executable is unavailable: {binary}; build the SDK or set NSH_BIN")
    if options.timeout <= 0:
        parser.error("--timeout must be positive")
    options.log_dir = options.log_dir.expanduser().resolve()
    options.log_dir.mkdir(parents=True, exist_ok=True)

    test_cases = cases(options.log_dir)
    for name, arguments, markers, expected_taps in test_cases:
        started = time.monotonic()
        print(f"Running NSH {name}...", flush=True)
        log = options.log_dir / f"{name}.log"
        try:
            result = subprocess.run(
                [str(binary), *arguments], cwd=ROOT, stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT, text=True, encoding="utf-8", errors="replace",
                timeout=options.timeout, check=False,
            )
            output = ANSI.sub("", result.stdout)
            log.write_text(output, encoding="utf-8")
            if result.returncode:
                raise ValueError(f"{name}: NSH exited with status {result.returncode}")
            validate_output(name, output, markers, expected_taps)
            if name == "production-entry":
                artifact = (options.log_dir / "production-entry/bundle.js").read_text(encoding="utf-8")
                entry = artifact.rsplit("//#region App.tsx", 1)[-1].split("//#endregion", 1)[0]
                if not re.search(r"const (\w+) = new CoreClient\(\);\s*return component\(PawsApp, \{ \1 \}\)", entry):
                    raise ValueError("production-entry: CoreClient must be constructed once in the mount owner and passed as a stable value")
                if re.search(r"get client\(|client:\s*new CoreClient", entry):
                    raise ValueError("production-entry: reactive prop getter constructs a second CoreClient")
        except subprocess.TimeoutExpired as error:
            raw = error.stdout or b""
            output = raw.decode("utf-8", errors="replace") if isinstance(raw, bytes) else raw
            log.write_text(ANSI.sub("", output), encoding="utf-8")
            print(f"FAIL: {name}: exceeded {options.timeout:g}s; log: {log}", file=sys.stderr)
            return 1
        except (OSError, ValueError) as error:
            print(f"FAIL: {error}; log: {log}", file=sys.stderr)
            if log.is_file():
                print("\n".join(log.read_text(encoding="utf-8").splitlines()[-20:]), file=sys.stderr)
            return 1
        print(f"PASS: {name} ({time.monotonic() - started:.2f}s)", flush=True)

    print(f"PASS: all {len(test_cases)} NSH acceptance checks; logs: {options.log_dir}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
