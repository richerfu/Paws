#!/usr/bin/env python3
"""Export allowlisted socket evidence without promoting timestamp proximity to causality.

Input: <stage>-{meow,sing-box}-<requestCount>.json plus matching
<stage>-<engine>-socket.log. Output stays in --input-dir with mode 0600.
Native payload, addresses, raw error text and arbitrary log fields are never copied.
"""
import argparse
import collections
import datetime
import hashlib
import json
import math
import os
from pathlib import Path
import re
import tempfile

ANSI = re.compile(r"\x1b\[[0-9;]*m")
PROBE_ID = re.compile(r"(?<![a-zA-Z0-9_-])tp-[0-9a-f]{16}-[1-9][0-9]{0,9}(?![a-zA-Z0-9_-])")
HASH = re.compile(r"[0-9a-f]{64}\Z")
DEFAULT_TARGET = "cp.cloudflare.com:443"
SESSION_FIELDS = (
    "engine", "version", "pid", "mode", "buildFingerprint", "nodeFingerprint",
    "profileFingerprint", "physicalNetId", "diagnosticBypass", "diagnosticPeerPinned",
    "diagnosticPeerFingerprint", "diagnosticPeerAddressFingerprint",
)
TCP_FIELDS = ("state", "unacked", "lost", "retrans", "rttUs", "totalRetrans")
RETRY_REASONS = {
    "peer-eof-before-tls-response", "peer-error-before-tls-response",
    "no-tls-response-within-five-seconds",
    "zero-return-client-hello; fresh-cipher; maximum-retries=1",
}


def probe_id(text):
    matches = PROBE_ID.findall(text)
    if len(matches) == 1 and int(matches[0].rsplit("-", 1)[1]) <= 1_000_000_000:
        return matches[0]
    return None


def number(text, field):
    found = re.search(r"\b" + re.escape(field) + r"=(\d+)", text)
    return int(found.group(1)) if found else None


def scope_metadata(scope, target):
    # Older diagnostic builds concatenate LogMessageVisitor span fragments.
    # "flow · " is the exact formatter prefix and cannot occur in a validated
    # probe ID. Restore only this known boundary; never trim arbitrary suffixes.
    scope = scope.replace("flow · ", " | flow · ")
    # Nested DNS/health dials can retain an ancestor connection span. The
    # actual dial belongs to the innermost flow, never the first ancestor.
    connections = list(re.finditer(r"\bconnection=(\d+)", scope))
    connection, direct_probe, destination, port = None, None, None, None
    if connections:
        current = connections[-1]
        connection = int(current.group(1))
        end = scope.find("|", current.end())
        end = len(scope) if end < 0 else end
        # Connection fields belong to this span only. Missing inner fields
        # must not inherit a parent source/target or a later child span.
        fields = scope[current.end():end]
        source = re.search(r"\bsource=([^,\s]+)", fields)
        port = re.search(r':(\d+)[\])"\']*$', source.group(1)) if source else None
        destination_field = re.search(r"\btarget=([^,\s]+)", fields)
        destination = destination_field.group(1).strip("\"'") if destination_field else None
        # Only local probe ancestors after the preceding connection boundary
        # can attest this connection. Never borrow a parent connection's ID.
        start = connections[-2].end() if len(connections) > 1 else 0
        probe_fields = list(re.finditer(r"\bprobe_id=[\"']?([^,\s\"']+)", scope[start:end]))
        for field in reversed(probe_fields):
            direct_probe = probe_id(field.group(1))
            if direct_probe:
                break
    return {
        "probeId": direct_probe,
        "logicalFlowId": str(connection) if connection is not None else None,
        "targetClass": "probe" if destination == target else ("other" if destination else "unknown"),
        "inboundSourcePort": int(port.group(1)) if port else None,
    }


def tcp_info(event):
    result = {}
    if "tcp_info" not in event and "tcpState=" not in event:
        return result
    for field in TCP_FIELDS:
        pattern = r"\btcpState=(\d+)" if field == "state" and "tcpState=" in event else r"\b" + field + r"[:=](\d+)"
        found = re.search(pattern, event)
        if found and int(found.group(1)) <= 0xFFFFFFFF:
            result[field] = int(found.group(1))
    return result


def retry_reason(event):
    # Arbitrary error strings can contain endpoint details: retain only known constants.
    return next((reason for reason in RETRY_REASONS if reason in event), "unclassified-guard-retry")


def read_events(engine, path, target=DEFAULT_TARGET):
    sockets, metadata, retries = {}, {}, []
    with Path(path).open() as log:
        for original in log:
            if engine == "meow":
                try:
                    record = json.loads(original)
                except ValueError:
                    continue
                event, at = record.get("event", ""), record.get("atMs")
                if not isinstance(event, str) or not isinstance(at, int):
                    continue
                fields = scope_metadata(record.get("flow", ""), target)
                if "tls-first-flight-retry" in event:
                    connection = number(event, "flow")
                    if connection is None:
                        connection = number(event, "connection")
                    retries.append({"atMs": at, "logicalFlowId": str(connection) if connection is not None else fields["logicalFlowId"], "reason": retry_reason(event)})
                sid = number(event, "id")
                if sid is None:
                    continue
                key = str(sid)  # Later poll/drop rows can lose their span. Do not split them.
            else:
                event = ANSI.sub("", original).rstrip("\r\n")
                times = re.findall(r"\batMs=(\d+)", event)
                if not times:
                    continue
                at = int(times[-1])  # Inner socket timestamp, not delayed ring-log timestamp.
                flow = re.search(r"\[(\d+) [^\]]+\]", event)
                if flow:
                    fid = flow.group(1)
                    meta = metadata.setdefault(fid, {})
                    if "inbound/mixed[sing-box-benchmark]: benchmark-probe" in event:
                        pid = probe_id(event)
                        if pid and not meta.get("probeIdConflict"):
                            previous = meta.get("probeId")
                            if previous is not None and previous != pid:
                                meta.pop("probeId", None)
                                meta["probeIdConflict"] = True
                            else:
                                meta["probeId"] = pid
                    if "inbound/mixed[sing-box-benchmark]: inbound connection from " in event:
                        meta["inboundAtMs"] = at
                        port = re.search(r":(\d+)\s*$", event)
                        if port:
                            meta["inboundSourcePort"] = int(port.group(1))
                    if "inbound/mixed[sing-box-benchmark]: inbound connection to " in event:
                        meta["targetClass"] = "probe" if event.endswith(target) else "other"
                if "tls-first-flight-retry" in event:
                    fid = number(event, "flow")
                    retries.append({"atMs": at, "logicalFlowId": str(fid) if fid is not None else None, "reason": retry_reason(event)})
                match = re.search(r"\bobfs-session id=(\d+) flow=(\d+)", event)
                if not match:
                    continue
                sid, fid = int(match.group(1)), match.group(2)
                key = f"{fid}/{sid}"
                fields = {"logicalFlowId": fid, "targetClass": "unknown", "inboundSourcePort": None}
            socket = sockets.setdefault(key, {"socketId": sid, "logicalFlowId": None, "targetClass": "unknown", "events": [], "sent": 0, "received": 0})
            for field, value in fields.items():
                if value is not None and (field != "targetClass" or value != "unknown"):
                    socket[field] = value
            kind = re.search(r"\bevent=[\"']?([a-z-]+)", event)
            row = {"atMs": at, "kind": kind.group(1) if kind else "unknown"}
            for field in ("sent", "received", "amount", "writeBytes", "readBytes"):
                value = number(event, field)
                if value is not None:
                    row[field] = value
                    if field in ("sent", "received"):
                        socket[field] = max(socket[field], value)
            if row["kind"] == "first-read":
                socket["received"] = max(socket["received"], row.get("readBytes", row.get("amount", 0)))
            if row["kind"] == "first-write":
                socket["sent"] = max(socket["sent"], row.get("writeBytes", row.get("amount", 0)))
            info = tcp_info(event)
            if info:
                row["tcpInfo"] = info
            peer = re.search(r"\bpeer=[\"']?([0-9a-f]{16})\b", event)
            if peer:
                socket["peerFingerprint"] = peer.group(1)
            socket["events"].append(row)
    for socket in sockets.values():
        if engine == "sing-box":
            socket.update(metadata.get(socket["logicalFlowId"], {}))
        socket["events"].sort(key=lambda row: row["atMs"])
        socket["connectedAtMs"] = next((r["atMs"] for r in socket["events"] if r["kind"] == "connected"), None)
        socket["startedAtMs"] = socket.get("inboundAtMs", socket["connectedAtMs"])
        ends = [r for r in socket["events"] if r["kind"] in ("peer-fin", "peer-reset", "local-close", "local-read-closed")]
        socket["terminalAtMs"] = ends[-1]["atMs"] if ends else None
        peer_ends = [r for r in socket["events"] if r["kind"] in ("peer-fin", "peer-reset")]
        socket["peerEnd"] = peer_ends[0] if peer_ends else None
        shutdowns = [r for r in socket["events"] if r["kind"] == "local-shutdown"]
        socket["localShutdownBeforePeerEnd"] = bool(shutdowns and peer_ends and shutdowns[0]["atMs"] <= peer_ends[0]["atMs"])
    # Exact joins require a validated ID on the native dial span (Meow), or
    # the native HTTP flow (Sing). Port reuse/time proximity never upgrades a
    # header observation to an exact socket join.
    flow_ids = collections.defaultdict(set)
    for socket in sockets.values():
        if socket.get("probeId") and socket["targetClass"] == "probe" and socket["logicalFlowId"] is not None:
            flow_ids[socket["logicalFlowId"]].add(socket["probeId"])
    flow_probes = {flow: next(iter(ids)) for flow, ids in flow_ids.items() if len(ids) == 1}
    for socket in sockets.values():
        flow = socket["logicalFlowId"]
        if socket["targetClass"] != "probe":
            socket.pop("probeId", None)
        elif flow in flow_ids and len(flow_ids[flow]) > 1:
            socket.pop("probeId", None)
        elif flow in flow_probes:
            socket["probeId"] = flow_probes[flow]
            socket["probeJoinMethod"] = "validated-header-and-native-dial-span" if engine == "meow" else "validated-header-and-native-flow-id"
    for retry in retries:
        if retry["logicalFlowId"] in flow_probes:
            retry["probeId"] = flow_probes[retry["logicalFlowId"]]
    return list(sockets.values()), retries


def epoch(text):
    return round(datetime.datetime.fromisoformat(text).timestamp() * 1000)


def clock_alignment(data):
    clock = data.get("clockAlignment", {})
    value = clock.get("deviceMinusHostMs")
    if clock.get("available") is True and isinstance(value, (int, float)) and math.isfinite(value):
        return {"available": True, "deviceMinusHostMs": value,
                **{k: clock[k] for k in ("roundTripMs", "uncertaintyMs", "samples") if isinstance(clock.get(k), (int, float)) and math.isfinite(clock[k])}}
    return {"available": False}


def valid_request_probe(request):
    candidate = request.get("probeId")
    if not isinstance(candidate, str) or probe_id(candidate) != candidate:
        return None
    return candidate if int(candidate.rsplit("-", 1)[1]) == request.get("requestId") else None


def temporal_candidates(request, sockets, clock):
    offset = clock.get("deviceMinusHostMs", 0)
    start, end = epoch(request["startedAt"]) + offset, epoch(request["finishedAt"]) + offset
    candidates = []
    for socket in sockets:
        if socket["targetClass"] == "other" or socket["startedAtMs"] is None or socket["terminalAtMs"] is None:
            continue
        if not start - 250 <= socket["startedAtMs"] <= min(end, start + 2000) or abs(socket["terminalAtMs"] - end) > 250:
            continue
        if bool(request.get("tlsMs", 0)) != bool(socket["received"]):
            continue
        candidates.append(dict(socket, startDeltaMs=socket["startedAtMs"]-start, terminalDeltaMs=socket["terminalAtMs"]-end))
    return candidates



def safe_number(value):
    return value if type(value) in (int, float) and math.isfinite(value) and value >= 0 else None


def safe_session(actual):
    result = {}
    for key in SESSION_FIELDS:
        value = actual.get(key)
        if key.endswith("Fingerprint"):
            result[key] = value if isinstance(value, str) and HASH.fullmatch(value) else None
        elif key in ("diagnosticBypass", "diagnosticPeerPinned"):
            result[key] = value if type(value) is bool else None
        elif key in ("pid", "physicalNetId"):
            result[key] = value if type(value) is int and 0 < value < 0x80000000 else None
        elif key == "engine":
            result[key] = value if value in ("meow", "sing-box", "mihomo") else None
        elif key == "mode":
            result[key] = value if value in ("rule", "global", "direct") else None
        elif key == "version":
            pattern = r"(?:(?:meow-rs|sing-box) )?v?\d+\.\d+(?:\.\d+)?(?:[-+][a-zA-Z0-9._-]+)?(?: / (?:OHOS socket bridge|TinyPaws OpenHarmony bridge) \d+)?"
            result[key] = value if isinstance(value, str) and len(value) <= 128 and (value == "unknown" or re.fullmatch(pattern, value)) else None
    return result


def safe_latency(value):
    value = value if isinstance(value, dict) else {}
    return {key: safe_number(value.get(key)) for key in ("min", "median", "p95", "p99", "max")}


def analysis(engine, data, sockets, retries, *, stage="after", target=DEFAULT_TARGET):
    requests, actual = data["requestEvidence"], data["actualEngine"]
    clock = clock_alignment(data)
    request_ids = [valid_request_probe(r) for r in requests]
    frequencies = collections.Counter(p for p in request_ids if p)
    unique_ids = {p for p, count in frequencies.items() if count == 1}
    joined = {p: [s for s in sockets if s.get("probeId") == p and s["targetClass"] == "probe"] for p in unique_ids}
    joined = {p: rows for p, rows in joined.items() if rows}
    benchmark_sockets = [s for rows in joined.values() for s in rows]
    benchmark_flows = {(s["logicalFlowId"], s["probeId"]) for s in benchmark_sockets}
    benchmark_retries = [r for r in retries if (r["logicalFlowId"], r.get("probeId")) in benchmark_flows]
    retry_counts = collections.Counter(r["probeId"] for r in benchmark_retries)
    failures, outcomes = [], []
    for request, pid in zip(requests, request_ids):
        exact = joined.get(pid, [])
        attempts = sorted(exact, key=lambda s: (s["connectedAtMs"] is None, s["connectedAtMs"] or 0))
        guard_count = retry_counts[pid] if exact else 0
        first_retry_at = min((r["atMs"] for r in benchmark_retries if r["probeId"] == pid), default=None)
        initial = [s for s in attempts if s["connectedAtMs"] is not None
                   and (first_retry_at is None or s["connectedAtMs"] <= first_retry_at)]
        first = initial[0] if initial else None
        zero_return_peer_end = bool(first and first["peerEnd"] and first["received"] == 0 and not first["localShutdownBeforePeerEnd"])
        outcomes.append({"requestId": request["requestId"], "probeId": pid, "success": bool(request["success"]),
                         "idJoined": bool(exact), "socketIds": [s["socketId"] for s in attempts],
                         "observedSocketAttempts": len(attempts), "guardRetryEvents": guard_count if exact else None,
                         "firstAttemptObserved": first is not None,
                         "firstAttemptSocketId": first["socketId"] if first else None,
                         "firstAttemptTransportAnomalyObserved": True if guard_count else (zero_return_peer_end if first else None),
                         "firstAttemptZeroReturnPeerEndObserved": zero_return_peer_end if first else None,
                         "recoveredAfterGuardRetry": bool(request["success"] and guard_count) if exact else None,
                         "failedDespiteGuardRetry": bool(not request["success"] and guard_count) if exact else None})
        if request["success"]:
            continue
        candidates = exact or temporal_candidates(request, sockets, clock)
        code = re.search(r"curl: \((\d+)\)", request.get("error", ""))
        failures.append({"requestId": request["requestId"], "completionOrder": request.get("completionOrder"),
                         "startedAt": request["startedAt"], "finishedAt": request["finishedAt"],
                         "latencyMs": request["latencyMs"], "proxyConnectStatus": request.get("proxyConnectStatus"),
                         "hostLocalPort": request.get("hostLocalPort"), "tlsCompleted": bool(request.get("tlsMs", 0)),
                         "curlExitCode": int(code.group(1)) if code else None, "probeId": pid,
                         "association": "probe-id-joined" if exact else ("unique-temporal-candidate" if len(candidates) == 1 else ("ambiguous-temporal-candidates" if candidates else "unassociated")),
                         "idJoined": bool(exact), "candidates": candidates, "serverInternalReason": "unknown-no-server-log"})
    pin = actual.get("diagnosticPeerFingerprint")
    expected = pin[:16] if isinstance(pin, str) and HASH.fullmatch(pin) else None
    checked = [s for s in benchmark_sockets if s.get("peerFingerprint")]
    mismatches = [s for s in checked if expected and s["peerFingerprint"] != expected]
    all_hashes = len(checked) == len(benchmark_sockets) and bool(benchmark_sockets)
    pinned = actual.get("diagnosticPeerPinned") is True and expected is not None
    recovery_evidence = [
        {"requestId": o["requestId"], "probeId": o["probeId"], "finalSuccess": o["success"],
         "guardRetryEvents": o["guardRetryEvents"], "firstAttemptObserved": o["firstAttemptObserved"],
         "firstAttemptSocketId": o["firstAttemptSocketId"], "sockets": joined[o["probeId"]],
         "retryEvents": [r for r in benchmark_retries if r["probeId"] == o["probeId"]]}
        for o in outcomes if o["idJoined"] and o["firstAttemptTransportAnomalyObserved"]
    ]
    counts = {
        "allNativeGuardRetryEvents": len(retries), "benchmarkGuardRetryEvents": len(benchmark_retries),
        "unjoinedOrNonBenchmarkGuardRetryEvents": len(retries)-len(benchmark_retries),
        "benchmarkProbesWithGuardRetry": sum(bool(o["guardRetryEvents"]) for o in outcomes),
        "successfulProbesRecoveredAfterRetry": sum(o["recoveredAfterGuardRetry"] is True for o in outcomes),
        "failedProbesDespiteRetry": sum(o["failedDespiteGuardRetry"] is True for o in outcomes),
        "firstAttemptTransportAnomaliesObserved": sum(o["firstAttemptTransportAnomalyObserved"] is True for o in outcomes),
        "finalSuccessfulRequests": data["results"]["successes"], "finalFailedRequests": data["results"]["failures"],
    }
    return {
        "format": "tinypaws-safe-socket-analysis-v3", "stage": stage, "engine": engine,
        "requestCount": data["requestCount"], "concurrency": data["concurrency"],
        "successes": data["results"]["successes"], "failuresCount": data["results"]["failures"],
        "latencyMs": safe_latency(data["results"].get("latencyMs")), "latencyStatisticsPopulation": "successful requests only",
        "session": safe_session(actual),
        "engineSessionStable": data["engineSessionStable"] is True,
        "probeTarget": DEFAULT_TARGET if target == DEFAULT_TARGET else "custom-probe-target",
        "probeTargetFingerprint": hashlib.sha256(target.encode()).hexdigest(), "clockAlignment": clock,
        "requestSocketJoin": "Validated header/native metadata where available; otherwise temporal-only. HDC host ports are never joined.",
        "probeJoinCoverage": {"requestsTotal": len(requests), "requestsWithValidProbeId": sum(bool(p) for p in request_ids),
                              "uniqueProbeIds": len(frequencies), "duplicateProbeIds": sorted(p for p,n in frequencies.items() if n > 1),
                              "requestsJoined": sum(o["idJoined"] for o in outcomes), "allRequestsExactlyJoined": bool(requests) and all(o["idJoined"] for o in outcomes),
                              "unassociatedProbeIds": sorted(unique_ids-set(joined)),
                              "requestsWithInvalidOrMismatchedProbeId": sum(bool(r.get("probeId")) and not p for r,p in zip(requests,request_ids))},
        "firstAttemptCoverage": {"requestsTotal": len(requests),
                                 "requestsWithFirstAttemptSocket": sum(o["firstAttemptObserved"] for o in outcomes),
                                 "requestsWithoutFirstAttemptSocket": sum(not o["firstAttemptObserved"] for o in outcomes),
                                 "allFirstAttemptSocketsObserved": bool(requests) and all(o["firstAttemptObserved"] for o in outcomes),
                                 "retryingRequests": sum(bool(o["guardRetryEvents"]) for o in outcomes),
                                 "retryingRequestsMissingFirstAttemptSocket": sum(bool(o["guardRetryEvents"]) and not o["firstAttemptObserved"] for o in outcomes),
                                 "probeIdsMissingFirstAttemptSocket": sorted({o["probeId"] for o in outcomes if o["probeId"] and not o["firstAttemptObserved"]}),
                                 "scope": "Request ID coverage does not imply attempt coverage. An observed retry requires an initial socket connected no later than its first retry event; absent retry events do not prove no retry occurred."},
        "benchmarkPeerPin": {"pinEnabled": pinned, "expectedPeerFingerprintPrefix": expected,
                             "associatedBenchmarkSockets": len(benchmark_sockets), "socketsWithPeerFingerprint": len(checked),
                             "allSocketPeerHashesAvailable": all_hashes, "mismatchedSocketIds": [s["socketId"] for s in mismatches],
                             "allAssociatedBenchmarkSocketsMatchPin": pinned and all_hashes and not mismatches,
                             "scope": "Only sockets ID-joined to this report; ordinary app flows excluded."},
        "historical98of100": "Original four failures lack contemporaneous socket capture; these later observations cannot retroactively establish their causes.",
        "socketCount": len(sockets), "probeSocketCount": sum(s["targetClass"] == "probe" for s in sockets),
        "unknownTargetSocketCount": sum(s["targetClass"] == "unknown" for s in sockets),
        "counts": counts, "recoveryEvents": benchmark_retries, "recoveryEvidence": recovery_evidence,
        "probeOutcomes": outcomes, "failures": failures,
        "interpretation": "Initial-attempt/recovery counters cover only ID-joined benchmark requests; zero with incomplete request or attempt coverage does not prove no incidents. Missing initial socket observations are null even when the request ID is joined. A guard retry is an initial transport anomaly, not a final request failure or proof of server-internal cause.",
        "associationPolicy": {"maxStartSkewBeforeMs": 250, "maxStartDelayMs": 2000, "maxEndSkewMs": 250,
                              "clockCalibrationUsedOnlyForUnjoinedTemporalCandidates": True,
                              "requiresCompatibleTlsReturnState": True, "excludesKnownOtherTargets": True,
                              "exactJoinsRequireKnownProbeTarget": True, "probeAncestryBoundedByPreviousConnection": True,
                              "initialSocketMustPrecedeObservedRetry": True,
                              "temporalCandidateIsNotCausalProof": True},
    }


def write_private_json(path, value):
    with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", prefix=".socket-safe-", dir=path.parent, delete=False) as temp:
        try:
            json.dump(value, temp, indent=2)
            temp.write("\n")
            temp.flush()
            os.fsync(temp.fileno())
            os.replace(temp.name, path)
        except BaseException:
            Path(temp.name).unlink(missing_ok=True)
            raise


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input-dir", type=Path, required=True)
    parser.add_argument("--stage", default="after")
    parser.add_argument("--probe-target", default=DEFAULT_TARGET)
    args = parser.parse_args(argv)
    if not re.fullmatch(r"[A-Za-z0-9_-]+", args.stage):
        parser.error("--stage must contain only letters, digits, underscore or hyphen")
    analyzed = 0
    for engine in ("meow", "sing-box"):
        pattern = re.compile(re.escape(f"{args.stage}-{engine}-") + r"(\d+)\.json\Z")
        reports = [(int(m.group(1)), p) for p in args.input_dir.glob(f"{args.stage}-{engine}-*.json") if (m := pattern.fullmatch(p.name))]
        log = args.input_dir / f"{args.stage}-{engine}-socket.log"
        if not reports or not log.exists():
            continue
        _, report = max(reports)
        data = json.loads(report.read_text())
        sockets, retries = read_events(engine, log, args.probe_target)
        safe = analysis(engine, data, sockets, retries, stage=args.stage, target=args.probe_target)
        out = args.input_dir / f"{args.stage}-{engine}-analysis-safe.json"
        write_private_json(out, safe)
        analyzed += 1
        print(json.dumps({"engine": engine, "outputFile": out.name, "successes": safe["successes"],
                          "probeJoinCoverage": safe["probeJoinCoverage"], "counts": safe["counts"],
                          "benchmarkPeerPin": safe["benchmarkPeerPin"]}))
    if not analyzed:
        parser.error("no matching report and socket log pairs")


if __name__ == "__main__":
    main()
