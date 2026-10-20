#!/usr/bin/env python3
"""Quantify the active HarmonyOS VPN path without disclosing the public IP.

The benchmark forwards the app's loopback mixed proxy over HDC, compares its
public-exit fingerprint with a proxy-free host request, then runs bounded HTTPS
requests and records latency, success rate, process identity and memory change.
It therefore rejects the common false positive where a core is running but the
test request went directly from the development host.
"""

from __future__ import annotations

import argparse
from datetime import datetime, timezone
import concurrent.futures
import hashlib
import ipaddress
import secrets
import json
import os
from pathlib import Path
import statistics
import subprocess
import sys
import time
import tempfile
import threading
import urllib.parse
import urllib.request


PROBE_RUN_ID = secrets.token_hex(8)

PROXY_ENV = {
    "ALL_PROXY",
    "all_proxy",
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "NO_PROXY",
    "no_proxy",
}


def clean_env() -> dict[str, str]:
    return {key: value for key, value in os.environ.items() if key not in PROXY_ENV}


def run(command: list[str], timeout: float = 15) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        command,
        text=True,
        capture_output=True,
        timeout=timeout,
        env=clean_env(),
        check=False,
    )


def curl_sample(url: str, proxy: str | None, timeout: int, probe_id: str | None = None) -> dict[str, object]:
    started_at = datetime.now(timezone.utc).isoformat()
    with tempfile.NamedTemporaryFile(prefix="tinypaws-request-") as body_file:
        command = ["curl", "--silent", "--show-error", "--location", "--max-time", str(timeout),
                   "--output", body_file.name, "--write-out",
                   "%{http_code}\t%{time_total}\t%{size_download}\t%{time_starttransfer}\t%{speed_download}\t%{time_appconnect}\t%{time_connect}\t%{http_connect}\t%{local_port}"]
        command += ["--proxy", proxy, "--noproxy", ""] if proxy else ["--noproxy", "*"]
        if proxy and probe_id:
            command += ["--proxy-header", "X-TinyPaws-Probe: " + probe_id]
        command.append(url)
        started = time.monotonic()
        try:
            result = run(command, timeout=timeout + 3)
        except subprocess.TimeoutExpired:
            return {"success": False, "status": 0, "latencyMs": round((time.monotonic()-started)*1000),
                    "ttfbMs": None, "bytes": 0, "bytesPerSecond": 0, "bodyFingerprint": "", "error": "subprocess-timeout",
                    "startedAt": started_at, "finishedAt": datetime.now(timezone.utc).isoformat()}
        fields = result.stdout.strip().split("\t")
        valid = len(fields) in (7, 9)
        status = int(fields[0]) if valid and fields[0].isdigit() else 0
        proxy_status = int(fields[7]) if len(fields) == 9 and fields[7].isdigit() else None
        requires_connect = bool(proxy and urllib.parse.urlsplit(url).scheme == "https")
        connect_proven = not requires_connect or proxy_status == 200
        success = result.returncode == 0 and 200 <= status < 400 and connect_proven
        body_file.seek(0)
        body = body_file.read(1024 * 1024) if success else b""
        body_hash = hashlib.sha256(body).hexdigest()[:16] if success else ""
        try:
            body_is_ip = ipaddress.ip_address(body.decode("ascii").strip()).is_global
        except (ValueError, UnicodeDecodeError):
            body_is_ip = False
        return {"success": success, "status": status,
                "latencyMs": round(float(fields[1])*1000) if valid else round((time.monotonic()-started)*1000),
                "bytes": round(float(fields[2])) if valid else 0,
                "ttfbMs": round(float(fields[3])*1000) if valid else None,
                "bytesPerSecond": round(float(fields[4])) if valid else 0,
                "tlsMs": round(float(fields[5])*1000) if valid else None,
                "connectMs": round(float(fields[6])*1000) if valid else None,
                "proxyConnectStatus": proxy_status,
                "hostLocalPort": int(fields[8]) if len(fields)==9 and fields[8].isdigit() else None,
                "bodyFingerprint": body_hash, "bodyIsPublicIp": body_is_ip, "error": "" if success else ("proxy-connect-not-established" if not connect_proven else (result.stderr.strip()[:240] or f"curl-{result.returncode}")),
                "startedAt": started_at, "finishedAt": datetime.now(timezone.utc).isoformat()}


def numbered_curl_sample(request_id: int, url: str, proxy: str | None, timeout: int) -> dict[str, object]:
    probe_id = f"tp-{PROBE_RUN_ID}-{request_id}"
    sample = curl_sample(url, proxy, timeout, probe_id=probe_id)
    sample["probeId"] = probe_id
    sample["requestId"] = request_id
    return sample


def engine_session(args: argparse.Namespace) -> dict[str, object]:
    path = "/data/app/el2/100/base/com.richerfu.paws/haps/entry/files/paws/engine-session.json"
    result = hdc(args, "shell", f"head -c 8192 {path}")
    record = json.loads(result.stdout)
    if record.get("format") != "TinyPaws-engine-session-v1" or record.get("engine") != args.engine:
        raise ValueError("Actual device engine differs from requested engine; fallback is not a candidate result")
    expected = {"mihomo": 18490, "meow": 18491, "sing-box": 18492}[args.engine]
    if record.get("ingressPort") != expected or args.device_mixed_port != expected:
        raise ValueError("Benchmark must enter the actual engine's dedicated native listener")
    if not record.get("session") or not record.get("profileFingerprint") or not record.get("version") or not record.get("buildFingerprint"):
        raise ValueError("Device engine attestation is incomplete")
    return record


def percentile(values: list[int], quantile: float) -> int | None:
    if not values:
        return None
    ordered = sorted(values)
    index = min(len(ordered) - 1, max(0, round((len(ordered) - 1) * quantile)))
    return ordered[index]


def throughput_passed(throughput: dict[str, object], expected_bytes: int) -> bool:
    if not expected_bytes:
        return True
    single = throughput.get("singleStream", {})
    multi = throughput.get("multiStream", {})
    return bool(single.get("success") and single.get("bytes") == expected_bytes
                and multi.get("successes") == multi.get("streams")
                and multi.get("bytes") == expected_bytes * multi.get("streams", 0))


def hdc(args: argparse.Namespace, *parts: str, timeout: int = 15) -> subprocess.CompletedProcess[str]:
    return run([args.hdc, "-t", args.target, *parts], timeout=timeout)


def clock_alignment(args: argparse.Namespace) -> dict[str, object]:
    samples = []
    for _ in range(3):
        start = time.time_ns() / 1_000_000
        result = hdc(args, "shell", "date", "+%s%3N", timeout=5)
        end = time.time_ns() / 1_000_000
        value = result.stdout.strip()
        if result.returncode == 0 and value.isdigit() and len(value) == 13:
            samples.append({"deviceMinusHostMs": round(int(value)-(start+end)/2, 2),
                            "roundTripMs": round(end-start, 2),
                            "uncertaintyMs": round((end-start)/2+1, 2)})
    if not samples:
        return {"available": False}
    best = min(samples, key=lambda x: x["roundTripMs"])
    return {"available": True, "samples": len(samples), **best}


def forward_mapping(args: argparse.Namespace, local: str) -> str | None:
    result = hdc(args, "fport", "ls", timeout=5)
    if result.returncode or "[Fail]" in result.stdout:
        raise ValueError("Cannot attest HDC forwarding table")
    for line in result.stdout.splitlines():
        parts = line.split()
        if len(parts) >= 4 and parts[0] == args.target and parts[1] == local and parts[3] == "[Forward]":
            return parts[2]
    return None


def open_forward(args: argparse.Namespace, local: str, remote: str) -> None:
    if forward_mapping(args, local) is not None:
        raise ValueError("Benchmark local port already has an HDC task; choose another --mixed-forward-port")
    result = hdc(args, "fport", local, remote)
    # HDC reports several errors on stdout with exit status zero.
    if result.returncode or "[Fail]" in result.stdout or forward_mapping(args, local) != remote:
        raise ValueError("HDC forward did not establish the requested native engine route")


def close_owned_forward(args: argparse.Namespace, local: str, remote: str) -> None:
    if forward_mapping(args, local) == remote:
        # taskstr is the full pair, not just the local port.
        hdc(args, "fport", "rm", local, remote, timeout=5)
        if forward_mapping(args, local) == remote:
            raise ValueError("Benchmark HDC forwarding task could not be removed")


def process_sample(args: argparse.Namespace) -> dict[str, object]:
    processes = hdc(args, "shell", "pidof com.richerfu.paws:vpn")
    pid = next((token for token in processes.stdout.split() if token.isdigit()), "")
    if not pid:
        return {"available": False}
    result = hdc(args, "shell", f"cat /proc/{pid}/stat; cat /proc/{pid}/status")
    if result.returncode:
        return {"available": False}
    lines = result.stdout.splitlines()
    stat = next((line for line in lines if line.startswith(f"{pid} (")), "")
    suffix = stat.rsplit(")", 1)[-1].split()

    def status_value(prefix: str) -> int | None:
        line = next((line for line in lines if line.startswith(prefix)), "")
        value = line.split()[1] if len(line.split()) > 1 else ""
        return int(value) if value.isdigit() else None

    fd_result = hdc(args, "shell", f"ls /proc/{pid}/fd")
    descriptor_tokens = fd_result.stdout.split() if fd_result.returncode == 0 else []
    numeric_descriptors = [token for token in descriptor_tokens if token.isdigit()]
    descriptors = len(numeric_descriptors) if numeric_descriptors else None
    if len(suffix) < 20:
        return {"available": False}
    return {
        "available": True,
        "pid": int(pid),
        "startTicks": int(suffix[19]),
        "rssKb": status_value("VmRSS:") or 0,
        "peakRssKb": status_value("VmHWM:") or 0,
        "fdCount": descriptors,
    }


def controller_json(port: int, path: str) -> object:
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(f"http://127.0.0.1:{port}{path}", timeout=5) as response:
        return json.load(response)


def controller_patch(port: int, path: str, value: object) -> None:
    request = urllib.request.Request(
        f"http://127.0.0.1:{port}{path}",
        data=json.dumps(value).encode(),
        headers={"Content-Type": "application/json"},
        method="PATCH",
    )
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    with opener.open(request, timeout=5):
        return


def observe_selected_route(controller_port: int, mixed_port: int, selected: str) -> dict[str, object]:
    command = [
        "curl",
        "--silent",
        "--max-time",
        "8",
        "--limit-rate",
        "128k",
        "--proxy",
        f"http://127.0.0.1:{mixed_port}",
        "https://speed.cloudflare.com/__down?bytes=8000000",
        "--output",
        os.devnull,
    ]
    process = subprocess.Popen(command, env=clean_env(), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    observed: list[str] = []
    peer_addresses: set[str] = set()
    try:
        for _ in range(30):
            try:
                payload = controller_json(controller_port, "/connections")
                for connection in payload.get("connections", []):
                    metadata = connection.get("metadata", {})
                    if metadata.get("host") != "speed.cloudflare.com":
                        continue
                    observed = [str(item) for item in connection.get("chains", [])]
                    try:
                        peer = ipaddress.ip_address(metadata.get("remoteDestination", ""))
                        peer_addresses.add(hashlib.sha256(str(peer).encode()).hexdigest()[:16])
                    except ValueError:
                        pass
                    break
            except Exception:
                pass
            if observed:
                break
            time.sleep(0.2)
    finally:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait(timeout=2)
    return {
        "selectedProxyObserved": bool(selected and selected in observed),
        "chainFingerprints": [hashlib.sha256(item.encode()).hexdigest()[:16] for item in observed],
        "physicalPeerAddressFingerprints": sorted(peer_addresses),
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, help="HDC target IP:port")
    parser.add_argument("--hdc", default="hdc")
    parser.add_argument("--requests", type=int, default=30)
    parser.add_argument("--concurrency", type=int, default=3)
    parser.add_argument("--timeout", type=int, default=12)
    parser.add_argument(
        "--minimum-success-rate",
        type=float,
        default=None,
        help="required percentage of successful requests (default: 99.5)",
    )
    parser.add_argument("--url", default="https://cp.cloudflare.com/generate_204")
    parser.add_argument("--exit-url", default="https://api.ipify.org")
    parser.add_argument("--controller-port", type=int, default=19090)
    parser.add_argument("--mixed-forward-port", type=int, default=18890)
    parser.add_argument("--device-mixed-port", type=int)
    parser.add_argument("--warmup", type=int, default=5)
    parser.add_argument("--throughput-bytes", type=int, default=0)
    parser.add_argument("--throughput-streams", type=int, default=4)
    parser.add_argument(
        "--engine",
        choices=("meow", "sing-box", "mihomo"),
        required=True,
        help="engine that owns the forwarded mixed listener",
    )
    parser.add_argument(
        "--force-global",
        action="store_true",
        help="temporarily force global mode and restore the original mode afterward",
    )
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    args.minimum_success_rate = args.minimum_success_rate if args.minimum_success_rate is not None else (99.0 if args.requests <= 100 else 99.5)
    args.device_mixed_port = args.device_mixed_port or {"mihomo":18490,"meow":18491,"sing-box":18492}[args.engine]
    if not 0 <= args.warmup <= 30 or not 0 <= args.throughput_bytes <= 64*1024*1024 or not 1 <= args.throughput_streams <= 8:
        parser.error("invalid warmup or bounded throughput settings")
    if not 1 <= args.timeout <= 60:
        parser.error("timeout must be 1..60 seconds")
    observed_at = datetime.now(timezone.utc).isoformat()
    try:
        actual_engine = engine_session(args)
    except Exception as error:
        parser.error(f"Device engine attestation failed: {error}")
    if not 1 <= args.requests <= 10_000 or not 1 <= args.concurrency <= 32:
        parser.error("requests must be 1..10000 and concurrency 1..32")
    if not 0 < args.minimum_success_rate <= 100:
        parser.error("minimum-success-rate must be in (0, 100]")

    targets = run([args.hdc, "list", "targets"], timeout=10)
    if args.target not in targets.stdout.splitlines():
        parser.error(f"HDC target is unavailable: {args.target}")
    local = f"tcp:{args.mixed_forward_port}"
    remote = f"tcp:{args.device_mixed_port}"
    try:
        open_forward(args, local, remote)
    except ValueError as error:
        parser.error(str(error))

    original_mode = None
    monitor_stop = threading.Event()
    monitor_thread = None
    try:
        configs = controller_json(args.controller_port, "/configs")
        original_mode = configs.get("mode") if isinstance(configs, dict) else None
        if args.force_global and original_mode != "global":
            controller_patch(args.controller_port, "/configs", {"mode": "global"})
        version = controller_json(args.controller_port, "/version")
        proxies = controller_json(args.controller_port, "/proxies")
        global_proxy = proxies.get("proxies", {}).get("GLOBAL", {}) if isinstance(proxies, dict) else {}
        selected = str(global_proxy.get("now", ""))
        selected_fingerprint = hashlib.sha256(selected.encode()).hexdigest()[:16] if selected else ""
    except Exception as error:  # Controller is optional for non-mihomo engines.
        version = {"unavailable": type(error).__name__}
        selected = ""
        selected_fingerprint = ""
    try:
        clock = clock_alignment(args)
        proxy_url = f"http://127.0.0.1:{args.mixed_forward_port}"
        for _ in range(args.warmup):
            curl_sample(args.url, proxy_url, args.timeout)
        before = process_sample(args)
        if not before.get("available") or before.get("pid") != actual_engine["pid"]:
            raise ValueError("Engine evidence does not belong to the live VPN process")
        direct = curl_sample(args.exit_url, None, args.timeout)
        proxied_probe = curl_sample(
            args.exit_url, f"http://127.0.0.1:{args.mixed_forward_port}", args.timeout
        )
        route_observation = observe_selected_route(
            args.controller_port, args.mixed_forward_port, selected
        ) if selected else {"selectedProxyObserved":False,"chainFingerprints":[]}
        proxy_url = f"http://127.0.0.1:{args.mixed_forward_port}"
        samples = []
        progress_step = max(10, args.requests // 100)
        started_at = time.monotonic()
        process_samples = [before]
        def monitor():
            while not monitor_stop.wait(2):
                try:
                    sample = process_sample(args)
                    sample["observedAt"] = datetime.now(timezone.utc).isoformat()
                    process_samples.append(sample)
                except Exception:
                    process_samples.append({"available":False})
        monitor_thread = threading.Thread(target=monitor, daemon=True)
        monitor_thread.start()
        with concurrent.futures.ThreadPoolExecutor(max_workers=args.concurrency) as pool:
            futures = [
                pool.submit(numbered_curl_sample, request_id, args.url, proxy_url, args.timeout)
                for request_id in range(1, args.requests + 1)
            ]
            for completed, future in enumerate(concurrent.futures.as_completed(futures), 1):
                sample = future.result()
                sample["completionOrder"] = completed
                sample["actualEngine"] = {"name":actual_engine["engine"],"version":actual_engine["version"],"session":actual_engine["session"],"pid":actual_engine["pid"],"ingressPort":actual_engine["ingressPort"],"profileFingerprint":actual_engine["profileFingerprint"],"buildFingerprint":actual_engine["buildFingerprint"],"mode":actual_engine.get("mode"),"physicalNetId":actual_engine["physicalNetId"]}
                samples.append(sample)
                if completed % progress_step == 0 or completed == args.requests:
                    elapsed = max(0.001, time.monotonic() - started_at)
                    ok = sum(1 for sample in samples if sample["success"])
                    print(
                        f"progress {completed}/{args.requests} "
                        f"success={ok} rate={completed / elapsed:.1f}/s",
                        file=sys.stderr,
                        flush=True,
                    )
        throughput = {}
        if args.throughput_bytes:
            transfer_url = f"https://speed.cloudflare.com/__down?bytes={args.throughput_bytes}"
            throughput["singleStream"] = curl_sample(transfer_url, proxy_url, max(args.timeout, 30))
            transfer_started = time.monotonic()
            with concurrent.futures.ThreadPoolExecutor(max_workers=args.throughput_streams) as transfer_pool:
                transfers = list(transfer_pool.map(lambda _:curl_sample(transfer_url,proxy_url,max(args.timeout,30)),range(args.throughput_streams)))
            throughput["multiStream"] = {"streams":args.throughput_streams,"successes":sum(t["success"] for t in transfers),"bytes":sum(t["bytes"] for t in transfers),"wallSeconds":round(time.monotonic()-transfer_started,3),"bytesPerSecond":round(sum(t["bytes"] for t in transfers)/max(.001,time.monotonic()-transfer_started))}
        monitor_stop.set()
        monitor_thread.join(timeout=20)
        after = process_sample(args)
        process_samples.append(after)
        final_engine = engine_session(args)
        forward_stable = forward_mapping(args, local) == remote
    finally:
        monitor_stop.set()
        if monitor_thread is not None:
            monitor_thread.join(timeout=20)
        if args.force_global and original_mode and original_mode != "global":
            try:
                controller_patch(args.controller_port, "/configs", {"mode": original_mode})
            except Exception:
                pass
        close_owned_forward(args, local, remote)

    successes = [sample for sample in samples if sample["success"]]
    latencies = [int(sample["latencyMs"]) for sample in successes]
    errors: dict[str, int] = {}
    for sample in samples:
        if sample["success"]:
            continue
        key = str(sample["error"])
        errors[key] = errors.get(key, 0) + 1
    engine_stable = all(actual_engine.get(key)==final_engine.get(key) for key in ["engine","session","pid","ingressPort","profileFingerprint","buildFingerprint","mode"])
    identity_stable = bool(
        before.get("available")
        and after.get("available")
        and (before.get("pid"), before.get("startTicks"))
        == (after.get("pid"), after.get("startTicks"))
    )
    exit_changed = bool(
        direct["success"]
        and proxied_probe["success"]
        and direct.get("bodyIsPublicIp", False)
        and proxied_probe.get("bodyIsPublicIp", False)
        and direct["bodyFingerprint"] != proxied_probe["bodyFingerprint"]
    )
    route_proven = bool(route_observation["selectedProxyObserved"] or exit_changed)
    success_rate = 100 * len(successes) / len(samples)
    report = {
        "format": "TinyPaws-harmony-vpn-benchmark-v3",
        "startedAt": observed_at,
        "finishedAt": datetime.now(timezone.utc).isoformat(),
        "actualEngine":actual_engine,
        "engineSessionStable":engine_stable,
        "hdcForwardEvidence":{"local":local,"remote":remote,"verifiedBeforeRequests":True,"verifiedAfterRequests":forward_stable},
        "requestEvidence":samples,
        "probeRunId":PROBE_RUN_ID,
        "clockAlignment":clock,
        "warmupRequestsExcluded":args.warmup,
        "throughput":throughput,
        "engine": args.engine,
        "ingress": {
            "kind": "dedicated-native-engine-owned-mixed",
            "devicePort": args.device_mixed_port,
        },
        "target": args.target,
        "requestUrl": args.url,
        "exitProbeUrl": args.exit_url,
        "requestCount": args.requests,
        "concurrency": args.concurrency,
        "controller": version,
        "testMode": "global" if args.force_global and selected else actual_engine.get("mode", original_mode),
        "selectedProxyFingerprint": selected_fingerprint,
        "routeProof": {
            "directRequestSucceeded": direct["success"],
            "vpnRequestSucceeded": proxied_probe["success"],
            "publicExitChanged": exit_changed,
            "directExitIsValidPublicIp":direct.get("bodyIsPublicIp",False),
            "vpnExitIsValidPublicIp":proxied_probe.get("bodyIsPublicIp",False),
            "selectedProxyObservedInControllerChain": route_observation[
                "selectedProxyObserved"
            ],
            "controllerChainFingerprints": route_observation["chainFingerprints"],
            "physicalPeerAddressFingerprints": route_observation.get("physicalPeerAddressFingerprints", []),
            "directExitFingerprint": direct["bodyFingerprint"],
            "vpnExitFingerprint": proxied_probe["bodyFingerprint"],
        },
        "results": {
            "successes": len(successes),
            "failures": len(samples) - len(successes),
            "successRatePercent": round(success_rate, 3),
            "requiredSuccessRatePercent": args.minimum_success_rate,
            "latencyMs": {
                "min": min(latencies) if latencies else None,
                "median": round(statistics.median(latencies)) if latencies else None,
                "p95": percentile(latencies, 0.95),
                "p99": percentile(latencies, 0.99),
                "max": max(latencies) if latencies else None,
            },
            "ttfbMs":{"median":percentile([s["ttfbMs"] for s in successes if s["ttfbMs"] is not None],.5),"p95":percentile([s["ttfbMs"] for s in successes if s["ttfbMs"] is not None],.95),"p99":percentile([s["ttfbMs"] for s in successes if s["ttfbMs"] is not None],.99)},
            "errors": errors,
        },
        "process": {
            "identityStable": identity_stable and all(s.get("available") and s.get("pid")==before.get("pid") and s.get("startTicks")==before.get("startTicks") for s in process_samples),
            "samples":process_samples,
            "rssSteadyGrowthMiB":round((int(after.get("rssKb",0))-int(before.get("rssKb",0)))/1024,3),
            "before": before,
            "after": after,
            "rssDeltaKb": int(after["rssKb"]) - int(before["rssKb"])
            if identity_stable
            else None,
            "fdDelta": int(after["fdCount"]) - int(before["fdCount"])
            if identity_stable
            and before.get("fdCount") is not None
            and after.get("fdCount") is not None
            else None,
        },
        "passed": forward_stable and engine_stable and route_proven and identity_stable and all(s.get("available") and s.get("pid")==before.get("pid") and s.get("startTicks")==before.get("startTicks") for s in process_samples) and success_rate >= args.minimum_success_rate and int(after.get("rssKb",0))-int(before.get("rssKb",0)) < 20*1024 and throughput_passed(throughput, args.throughput_bytes),
        "limits": [
            "This measures the named native proxy data plane. It does not certify TUN, app rendering, lock-screen or handover endurance.",
            "RSS drift is measured over this run only; it is not a 24-hour memory claim.",
            "Exit fingerprints are hashes; the public IP is never written to the report.",
        ],
    }
    rendered = json.dumps(report, ensure_ascii=False, indent=2)
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
