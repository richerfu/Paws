#!/usr/bin/env python3
"""Regression tests for evidence joins, replay accounting and safe exports."""
import importlib.util
import json
from pathlib import Path
import stat
import tempfile
import unittest
from unittest import mock

SCRIPT = Path(__file__).with_name("analyze-socket-evidence.py")
spec = importlib.util.spec_from_file_location("socket_evidence", SCRIPT)
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)
PIN = "1234567890abcdef" * 4
PROBE = "tp-0123456789abcdef-7"


def request(request_id=7, *, success=True, probe=PROBE, tls=20):
    return {"requestId": request_id, "probeId": probe, "success": success,
            "startedAt": "1970-01-01T00:00:01+00:00", "finishedAt": "1970-01-01T00:00:02+00:00",
            "latencyMs": 1000, "tlsMs": tls, "error": "" if success else "curl: (35) [sensitive error omitted]",
            "proxyConnectStatus": 200}


def report(requests):
    successes = sum(r["success"] for r in requests)
    return {"requestEvidence": requests, "actualEngine": {
        "diagnosticPeerPinned": True, "diagnosticPeerFingerprint": PIN,
        "diagnosticPeerAddressFingerprint": "a" * 64, "subscriptionUrl": "SENSITIVE_SENTINEL"},
        "requestCount": len(requests), "concurrency": 4, "engineSessionStable": True,
        "results": {"successes": successes, "failures": len(requests)-successes}}


def parse(engine, text):
    with tempfile.TemporaryDirectory() as directory:
        path = Path(directory) / "socket.log"
        path.write_text(text)
        return a.read_events(engine, path)


def rust_rows(rows):
    return "\n".join(json.dumps(row) for row in rows)


def native_socket(socket_id=1, *, probe=PROBE, target="probe", peer=PIN[:16]):
    return {"socketId": socket_id, "logicalFlowId": str(socket_id), "probeId": probe,
            "targetClass": target, "connectedAtMs": 1010, "startedAtMs": 1000,
            "terminalAtMs": 1999, "received": 30, "sent": 40, "peerEnd": None,
            "localShutdownBeforePeerEnd": False, "peerFingerprint": peer, "events": []}


class EvidenceTests(unittest.TestCase):
    def test_import_performs_no_private_file_reads(self):
        another = importlib.util.module_from_spec(spec)
        with mock.patch.object(Path, "read_text", side_effect=AssertionError("import read a file")), \
             mock.patch.object(Path, "open", side_effect=AssertionError("import opened a file")):
            spec.loader.exec_module(another)

    def test_rust_missing_later_span_does_not_split_socket(self):
        rows = [{"atMs": 1000, "flow": "connection=7 source=127.0.0.1:1111 target=cp.cloudflare.com:443",
                 "event": "socket-trace event=connected · id=4, peer=1234567890abcdef"},
                {"atMs": 1050, "flow": "", "event": "socket-trace · id=4, event=peer-fin, sent=587, received=0"},
                {"atMs": 1051, "flow": "", "event": "socket-trace · id=4, event=local-close, sent=587, received=0"}]
        sockets, _ = parse("meow", rust_rows(rows))
        self.assertEqual(len(sockets), 1)
        self.assertEqual(sockets[0]["logicalFlowId"], "7")
        self.assertEqual(sockets[0]["targetClass"], "probe")
        self.assertEqual(sockets[0]["peerEnd"]["kind"], "peer-fin")

    def test_nested_native_flow_uses_actual_child_and_never_inherits_unrelated_probe(self):
        rows = [
            {"atMs":1000,"flow":f"flow · probe_id={PROBE} | flow · connection=8, source=127.0.0.1:1111, target=cp.cloudflare.com:443", "event":"socket-trace event=connected id=8"},
            {"atMs":1050,"flow":"flow · connection=8, source=127.0.0.1:1111, target=cp.cloudflare.com:443 | flow · connection=29, source=127.0.0.1:2222, target=internal.example:443", "event":"socket-trace event=connected id=31"},
        ]
        sockets,_=parse("meow",rust_rows(rows))
        self.assertEqual(sockets[1]["logicalFlowId"],"29")
        self.assertEqual(sockets[1]["targetClass"],"other")
        self.assertEqual(sockets[1]["inboundSourcePort"],2222)
        self.assertNotIn("probeId",sockets[1])
        result=a.analysis("meow",report([request()]),sockets,[])
        self.assertEqual(result["probeOutcomes"][0]["socketIds"],[8])

    def test_nested_probe_comes_only_from_the_inner_connections_local_ancestors(self):
        inner_probe = "tp-0123456789abcdef-8"
        parent = f"flow · probe_id={PROBE} | flow · connection=7, source=127.0.0.1:1111, target=cp.cloudflare.com:443"
        child = "flow · connection=8, source=[::1]:2222, target=cp.cloudflare.com:443"
        fields = a.scope_metadata(f"{parent} | flow · probe_id={inner_probe} | {child}", a.DEFAULT_TARGET)
        self.assertEqual(fields, {"probeId": inner_probe, "logicalFlowId": "8", "targetClass": "probe", "inboundSourcePort": 2222})
        self.assertIsNone(a.scope_metadata(f"{parent} | {child}", a.DEFAULT_TARGET)["probeId"])
        middle = f"flow · probe_id={inner_probe} | {child}"
        grandchild = "flow · connection=9, source=127.0.0.1:3333, target=cp.cloudflare.com:443"
        self.assertIsNone(a.scope_metadata(f"{parent} | {middle} | {grandchild}", a.DEFAULT_TARGET)["probeId"])
        nearest = a.scope_metadata(f"{parent} | flow · probe_id={PROBE} | flow · probe_id={inner_probe} | {child}", a.DEFAULT_TARGET)
        self.assertEqual(nearest["probeId"], inner_probe)

    def test_missing_inner_source_and_target_do_not_inherit_other_spans(self):
        parent = f"flow · probe_id={PROBE} | flow · connection=7, source=127.0.0.1:1111, target=cp.cloudflare.com:443"
        for child in ("flow · connection=8", "flow · connection=8 | flow · source=127.0.0.1:2222 target=cp.cloudflare.com:443"):
            fields = a.scope_metadata(f"{parent} | {child}", a.DEFAULT_TARGET)
            self.assertEqual(fields, {"probeId": None, "logicalFlowId": "8", "targetClass": "unknown", "inboundSourcePort": None})
        fields = a.scope_metadata(f"{parent} | flow · connection=8, target=internal.example:443", a.DEFAULT_TARGET)
        self.assertEqual(fields["targetClass"], "other")
        self.assertIsNone(fields["inboundSourcePort"])

    def test_nested_same_target_without_inner_probe_is_not_an_exact_join(self):
        scope = f"flow · probe_id={PROBE} | flow · connection=7, source=127.0.0.1:1111, target=cp.cloudflare.com:443 | flow · connection=8, source=127.0.0.1:2222, target=cp.cloudflare.com:443"
        sockets, retries = parse("meow", rust_rows([
            {"atMs": 1000, "flow": scope, "event": "socket-trace event=connected id=8"},
            {"atMs": 1100, "flow": "", "event": "socket-trace event=tls-first-flight-retry flow=8 reason=peer-eof-before-tls-response"},
        ]))
        result = a.analysis("meow", report([request()]), sockets, retries)
        self.assertFalse(result["probeOutcomes"][0]["idJoined"])
        self.assertEqual(result["counts"]["benchmarkGuardRetryEvents"], 0)

    def test_additional_native_sockets_require_the_same_probe_and_known_target(self):
        other_probe = "tp-0123456789abcdef-8"
        rows = []
        for sid, scope, peer in [
            (1, f"probe_id={PROBE} connection=7 target=cp.cloudflare.com:443", PIN[:16]),
            (2, "connection=7", "f" * 16),
            (3, f"probe_id={PROBE} connection=9 target=internal.example:443", "f" * 16),
            (4, f"probe_id={PROBE} connection=10 target=cp.cloudflare.com:443", PIN[:16]),
            (5, f"probe_id={other_probe} connection=11 target=cp.cloudflare.com:443", "f" * 16),
        ]:
            rows.append({"atMs": 1000 + sid, "flow": scope, "event": f"socket-trace event=connected id={sid} peer={peer}"})
        sockets, retries = parse("meow", rust_rows(rows))
        self.assertNotIn("probeId", sockets[1])
        self.assertNotIn("probeId", sockets[2])
        result = a.analysis("meow", report([request()]), sockets, retries)
        self.assertEqual(result["probeOutcomes"][0]["socketIds"], [1, 4])
        self.assertEqual(result["benchmarkPeerPin"]["associatedBenchmarkSockets"], 2)
        self.assertTrue(result["benchmarkPeerPin"]["allAssociatedBenchmarkSocketsMatchPin"])

    def test_go_ansi_is_removed_and_inner_timestamp_is_used(self):
        text = "info atMs=1003 \x1b[32mINFO\x1b[0m[0000] [71 0ms] inbound/mixed[sing-box-benchmark]: inbound connection to cp.cloudflare.com:443\n"
        text += "info atMs=1010 obfs-session id=8 flow=71 atMs=1009 event=connected peer=1234567890abcdef\n"
        text += "info atMs=1090 obfs-session id=8 flow=71 atMs=1089 event=peer-reset sent=1 received=3"
        sockets, _ = parse("sing-box", text)
        self.assertEqual(sockets[0]["targetClass"], "probe")
        self.assertEqual(sockets[0]["terminalAtMs"], 1089)

    def test_time_candidate_excludes_other_target_and_is_not_an_id_join(self):
        req = request(success=False, probe=None, tls=0)
        candidate = native_socket(probe=None)
        candidate["received"] = 0
        other = dict(candidate, socketId=5, targetClass="other")
        result = a.analysis("meow", report([req]), [candidate, other], [])
        failure = result["failures"][0]
        self.assertEqual([s["socketId"] for s in failure["candidates"]], [1])
        self.assertFalse(failure["idJoined"])
        self.assertIsNone(result["probeOutcomes"][0]["guardRetryEvents"])
        self.assertIsNone(result["probeOutcomes"][0]["firstAttemptTransportAnomalyObserved"])
        self.assertEqual(failure["association"], "unique-temporal-candidate")
        self.assertEqual(failure["serverInternalReason"], "unknown-no-server-log")

    def test_clock_offset_applies_only_to_unjoined_time_candidates(self):
        req = request(success=False, probe=None, tls=0)
        candidate = native_socket(probe=None)
        candidate.update(startedAtMs=11000, terminalAtMs=11999, received=0)
        data = report([req])
        self.assertEqual(a.analysis("meow", data, [candidate], [])["failures"][0]["candidates"], [])
        data["clockAlignment"] = {"available": True, "deviceMinusHostMs": 10000, "secret": "SENSITIVE_SENTINEL"}
        result = a.analysis("meow", data, [candidate], [])
        self.assertEqual(len(result["failures"][0]["candidates"]), 1)
        req["probeId"] = PROBE
        candidate["probeId"] = PROBE
        data["clockAlignment"]["deviceMinusHostMs"] = 100000000
        exact = a.analysis("meow", data, [candidate], [])
        self.assertTrue(exact["failures"][0]["idJoined"])
        self.assertNotIn("startDeltaMs", exact["failures"][0]["candidates"][0])

    def test_meow_direct_probe_span_keeps_retry_flow_and_ipv6_source_port(self):
        rows = [
            {"atMs": 1000, "flow": "", "event": f'socket-trace event=benchmark-probe · probe_id="{PROBE}", source_port=1111'},
            {"atMs": 1010, "flow": f'probe_id="{PROBE}" connection=9 source=[::1]:1111 target=cp.cloudflare.com:443', "event": "socket-trace event=connected · id=4"},
            {"atMs": 1051, "flow": "", "event": "socket-trace · id=4, event=local-close, sent=587, received=0"},
            {"atMs": 1100, "flow": "", "event": "socket-trace event=tls-first-flight-retry flow=9 reason=peer-eof-before-tls-response"},
            {"atMs": 6000, "flow": "connection=9 source=[::1]:1111 target=cp.cloudflare.com:443", "event": "socket-trace event=connected · id=5"},
        ]
        sockets, retries = parse("meow", rust_rows(rows))
        self.assertEqual({s["probeId"] for s in sockets}, {PROBE})
        self.assertEqual(retries[0]["probeId"], PROBE)
        self.assertEqual(retries[0]["logicalFlowId"], "9")

    def test_meow_source_port_reuse_never_creates_exact_join_or_recovery(self):
        rows = [
            {"atMs": 1000, "flow": "", "event": f'socket-trace event=benchmark-probe probe_id="{PROBE}" source_port=1111'},
            {"atMs": 1010, "flow": f'probe_id="{PROBE}" connection=9 source=127.0.0.1:1111 target=cp.cloudflare.com:443', "event": "socket-trace event=connected id=4 peer=1234567890abcdef"},
            {"atMs": 1051, "flow": "", "event": "socket-trace id=4 event=local-close sent=20 received=30"},
            {"atMs": 6000, "flow": "connection=10 source=127.0.0.1:1111 target=cp.cloudflare.com:443", "event": "socket-trace event=connected id=5 peer=ffffffffffffffff"},
            {"atMs": 6100, "flow": "", "event": "socket-trace event=tls-first-flight-retry flow=10 reason=peer-eof-before-tls-response"},
        ]
        sockets, retries = parse("meow", rust_rows(rows))
        self.assertNotIn("probeId", sockets[1])
        self.assertNotIn("probeId", retries[0])
        result = a.analysis("meow", report([request()]), sockets, retries)
        self.assertEqual(result["probeOutcomes"][0]["socketIds"], [4])
        self.assertEqual(result["counts"]["benchmarkGuardRetryEvents"], 0)
        self.assertEqual(result["counts"]["successfulProbesRecoveredAfterRetry"], 0)
        self.assertEqual(result["benchmarkPeerPin"]["associatedBenchmarkSockets"], 1)

    def test_meow_legacy_port_header_without_direct_span_is_not_exact(self):
        rows = [
            {"atMs": 1000, "flow": "", "event": f'socket-trace event=benchmark-probe probe_id="{PROBE}" source_port=1111'},
            {"atMs": 1010, "flow": "connection=9 source=127.0.0.1:1111 target=cp.cloudflare.com:443", "event": "socket-trace event=connected id=4"},
        ]
        sockets, retries = parse("meow", rust_rows(rows))
        result = a.analysis("meow", report([request()]), sockets, retries)
        self.assertFalse(result["probeOutcomes"][0]["idJoined"])

    def test_go_probe_join_uses_flow_id_and_excludes_unrelated_fin(self):
        text = f"info atMs=1000 INFO[0000] [71 0ms] inbound/mixed[sing-box-benchmark]: benchmark-probe probeId={PROBE}\n"
        text += "info atMs=1003 INFO[0000] [71 0ms] inbound/mixed[sing-box-benchmark]: inbound connection to cp.cloudflare.com:443\n"
        text += "info atMs=1010 obfs-session id=8 flow=71 atMs=1009 event=connected\n"
        text += "info atMs=1090 obfs-session id=8 flow=71 atMs=1089 event=local-close sent=1 received=0\n"
        text += "info atMs=1088 obfs-session id=9 flow=72 atMs=1088 event=peer-fin sent=1 received=0"
        sockets, retries = parse("sing-box", text)
        result = a.analysis("sing-box", report([request(success=False, tls=0)]), sockets, retries)
        self.assertTrue(result["failures"][0]["idJoined"])
        self.assertEqual([s["socketId"] for s in result["failures"][0]["candidates"]], [8])

    def test_guard_retry_counts_are_separate_from_final_success_and_pin_scope(self):
        recovered = request()
        failed_id = "tp-0123456789abcdef-8"
        failed = request(8, success=False, probe=failed_id)
        sockets = [native_socket(), native_socket(2), native_socket(3, probe=failed_id),
                   native_socket(4, probe="tp-0123456789abcdef-9", peer="f"*16)]
        retries = [{"atMs": 1050, "logicalFlowId": "1", "probeId": PROBE},
                   {"atMs": 1050, "logicalFlowId": "3", "probeId": failed_id},
                   {"atMs": 1050, "logicalFlowId": "4", "probeId": "tp-0123456789abcdef-9"}]
        result = a.analysis("meow", report([recovered, failed]), sockets, retries)
        counts = result["counts"]
        self.assertEqual(counts["benchmarkGuardRetryEvents"], 2)
        self.assertEqual(counts["successfulProbesRecoveredAfterRetry"], 1)
        self.assertEqual(counts["failedProbesDespiteRetry"], 1)
        self.assertEqual(counts["unjoinedOrNonBenchmarkGuardRetryEvents"], 1)
        self.assertEqual(counts["firstAttemptTransportAnomaliesObserved"], 2)
        self.assertEqual(counts["finalFailedRequests"], 1)
        self.assertEqual(len(result["recoveryEvidence"]), 2)
        self.assertEqual(result["recoveryEvidence"][0]["sockets"][0]["socketId"], 1)
        self.assertTrue(result["benchmarkPeerPin"]["allAssociatedBenchmarkSocketsMatchPin"])
        self.assertEqual(result["benchmarkPeerPin"]["associatedBenchmarkSockets"], 3)

    def test_recovery_only_capture_has_request_coverage_but_no_initial_socket_coverage(self):
        for success in (False, True):
            with self.subTest(success=success):
                recovered = native_socket(2)
                recovered.update(logicalFlowId="7", connectedAtMs=1500,
                                 received=30 if success else 0,
                                 peerEnd=None if success else {"atMs": 1900, "kind": "peer-reset"})
                retries = [{"atMs": 1400, "logicalFlowId": "7", "probeId": PROBE}]
                result = a.analysis("meow", report([request(success=success)]), [recovered], retries)
                outcome = result["probeOutcomes"][0]
                self.assertTrue(result["probeJoinCoverage"]["allRequestsExactlyJoined"])
                self.assertFalse(outcome["firstAttemptObserved"])
                self.assertIsNone(outcome["firstAttemptSocketId"])
                self.assertIsNone(outcome["firstAttemptZeroReturnPeerEndObserved"])
                self.assertTrue(outcome["firstAttemptTransportAnomalyObserved"])
                self.assertEqual(outcome["recoveredAfterGuardRetry"], success)
                coverage = result["firstAttemptCoverage"]
                self.assertFalse(coverage["allFirstAttemptSocketsObserved"])
                self.assertEqual(coverage["requestsWithoutFirstAttemptSocket"], 1)
                self.assertEqual(coverage["retryingRequestsMissingFirstAttemptSocket"], 1)
                self.assertEqual(coverage["probeIdsMissingFirstAttemptSocket"], [PROBE])
                self.assertFalse(result["recoveryEvidence"][0]["firstAttemptObserved"])
                self.assertIsNone(result["recoveryEvidence"][0]["firstAttemptSocketId"])

    def test_initial_socket_at_or_before_first_retry_is_observed(self):
        for connected_at in (1100, 1200):
            with self.subTest(connectedAtMs=connected_at):
                first = native_socket(1)
                first.update(logicalFlowId="7", connectedAtMs=connected_at, received=0,
                             peerEnd={"atMs": 1200, "kind": "peer-fin"})
                fresh = native_socket(2)
                fresh.update(logicalFlowId="7", connectedAtMs=1300)
                result = a.analysis("meow", report([request()]), [fresh, first],
                                    [{"atMs": 1200, "logicalFlowId": "7", "probeId": PROBE}])
                self.assertTrue(result["firstAttemptCoverage"]["allFirstAttemptSocketsObserved"])
                self.assertEqual(result["probeOutcomes"][0]["firstAttemptSocketId"], 1)
                self.assertTrue(result["probeOutcomes"][0]["firstAttemptZeroReturnPeerEndObserved"])

    def test_missing_connect_event_does_not_attest_initial_socket(self):
        first = native_socket()
        first.update(connectedAtMs=None, received=0, peerEnd={"atMs": 1200, "kind": "peer-fin"})
        result = a.analysis("meow", report([request()]), [first],
                            [{"atMs": 1200, "logicalFlowId": "1", "probeId": PROBE}])
        self.assertTrue(result["probeOutcomes"][0]["idJoined"])
        self.assertFalse(result["probeOutcomes"][0]["firstAttemptObserved"])
        self.assertIsNone(result["probeOutcomes"][0]["firstAttemptZeroReturnPeerEndObserved"])

    def test_retry_with_matching_probe_but_different_native_flow_is_not_joined(self):
        result = a.analysis("meow", report([request()]), [native_socket()],
                            [{"atMs": 1200, "logicalFlowId": "99", "probeId": PROBE}])
        self.assertEqual(result["counts"]["benchmarkGuardRetryEvents"], 0)
        self.assertEqual(result["counts"]["unjoinedOrNonBenchmarkGuardRetryEvents"], 1)

    def test_pin_mismatch_and_missing_hash_cannot_pass(self):
        data = report([request()])
        socket = native_socket(peer="f"*16)
        result = a.analysis("meow", data, [socket], [])
        self.assertFalse(result["benchmarkPeerPin"]["allAssociatedBenchmarkSocketsMatchPin"])
        self.assertEqual(result["benchmarkPeerPin"]["mismatchedSocketIds"], [1])
        socket.pop("peerFingerprint")
        result = a.analysis("meow", data, [socket], [])
        self.assertFalse(result["benchmarkPeerPin"]["allSocketPeerHashesAvailable"])
        self.assertFalse(result["benchmarkPeerPin"]["allAssociatedBenchmarkSocketsMatchPin"])

    def test_thousand_probe_ids_have_complete_exact_coverage(self):
        requests, sockets = [], []
        for request_id in range(1, 1001):
            pid = f"tp-0123456789abcdef-{request_id}"
            requests.append(request(request_id, probe=pid))
            sockets.append(native_socket(request_id, probe=pid))
        result = a.analysis("meow", report(requests), sockets, [])
        self.assertEqual(result["probeJoinCoverage"]["requestsJoined"], 1000)
        self.assertTrue(result["probeJoinCoverage"]["allRequestsExactlyJoined"])
        self.assertEqual(result["firstAttemptCoverage"]["requestsWithFirstAttemptSocket"], 1000)
        self.assertTrue(result["firstAttemptCoverage"]["allFirstAttemptSocketsObserved"])
        self.assertEqual(len(result["probeOutcomes"]), 1000)

    def test_duplicate_malformed_or_mismatched_probe_ids_are_not_exact(self):
        duplicate = [request(), request()]
        result = a.analysis("meow", report(duplicate), [native_socket()], [])
        self.assertEqual(result["probeJoinCoverage"]["requestsJoined"], 0)
        self.assertEqual(result["probeJoinCoverage"]["duplicateProbeIds"], [PROBE])
        for bad in ("SENSITIVE_SENTINEL", "tp-0123456789abcdef-0", "tp-0123456789abcdef-8"):
            result = a.analysis("meow", report([request(probe=bad)]), [native_socket()], [])
            self.assertEqual(result["probeJoinCoverage"]["requestsJoined"], 0)
            self.assertNotIn(bad, json.dumps(result))

    def test_tcp_info_numeric_allowlist_and_session_pin_fields(self):
        event = 'socket-trace tcp_info="state:1 unacked:2 lost:3 retrans:4 rttUs:5 totalRetrans:6 Authorization:SENSITIVE_SENTINEL"'
        self.assertEqual(a.tcp_info(event), dict(zip(a.TCP_FIELDS, range(1, 7))))
        self.assertEqual(a.tcp_info("tcpState=1 unacked=2 password=SENSITIVE_SENTINEL"), {"state": 1, "unacked": 2})
        self.assertEqual(a.tcp_info("tcpState=99999999999999999999 retrans=not-a-number"), {})
        safe = a.analysis("meow", report([request()]), [native_socket()], [])
        self.assertEqual(safe["session"]["diagnosticPeerFingerprint"], PIN)
        self.assertEqual(safe["session"]["diagnosticPeerAddressFingerprint"], "a"*64)
        self.assertNotIn("SENSITIVE_SENTINEL", json.dumps(safe))

    def test_allowed_session_field_values_and_custom_target_are_sanitized(self):
        data = report([request()])
        for field in a.SESSION_FIELDS:
            data["actualEngine"][field] = "SENSITIVE_SENTINEL"
        data["results"]["latencyMs"] = {"median": "SENSITIVE_SENTINEL", "extra": "SENSITIVE_SENTINEL"}
        safe = a.analysis("meow", data, [native_socket()], [], target="SENSITIVE_SENTINEL")
        self.assertNotIn("SENSITIVE_SENTINEL", json.dumps(safe))
        self.assertEqual(safe["probeTarget"], "custom-probe-target")
        self.assertEqual(len(safe["probeTargetFingerprint"]), 64)
        for version in ("meow-rs 0.21.2 / OHOS socket bridge 2", "sing-box 1.13.21 / TinyPaws OpenHarmony bridge 2", "1.10.0"):
            self.assertEqual(a.safe_session({"version": version})["version"], version)

    def test_cli_requires_explicit_input_and_writes_private_safe_json(self):
        with self.assertRaises(SystemExit), mock.patch("sys.stderr"):
            a.main([])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root/"after-meow-1.json").write_text(json.dumps(report([request()])))
            (root/"after-meow-socket.log").write_text("")
            with mock.patch("builtins.print"):
                a.main(["--input-dir", directory])
            output = root/"after-meow-analysis-safe.json"
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
            self.assertNotIn("SENSITIVE_SENTINEL", output.read_text())

    def test_known_concatenated_native_span_prefix_is_a_validated_id_boundary(self):
        scope = f"flow · probe_id={PROBE}flow · connection=18, source=127.0.0.1:48908, target=cp.cloudflare.com:443"
        fields = a.scope_metadata(scope, a.DEFAULT_TARGET)
        self.assertEqual(fields["probeId"], PROBE)
        self.assertEqual(fields["logicalFlowId"], "18")
        self.assertEqual(fields["targetClass"], "probe")
        malformed = f"flow · probe_id={PROBE}flower connection=18, source=127.0.0.1:48908, target=cp.cloudflare.com:443"
        self.assertIsNone(a.scope_metadata(malformed, a.DEFAULT_TARGET)["probeId"])
        rows = [{"atMs": 1000, "flow": scope, "event": "socket-trace event=connected id=19 peer=1234567890abcdef"},
                {"atMs": 2000, "flow": "", "event": "socket-trace event=local-close id=19 sent=621 received=0"},
                {"atMs": 2001, "flow": "", "event": "socket-trace event=tls-first-flight-retry flow=18 reason=no-tls-response-within-five-seconds"}]
        sockets, retries = parse("meow", rust_rows(rows))
        result = a.analysis("meow", report([request(success=False, tls=0)]), sockets, retries)
        self.assertTrue(result["failures"][0]["idJoined"])
        self.assertEqual(result["counts"]["failedProbesDespiteRetry"], 1)

    def test_target_class_requires_exact_match(self):
        fields = a.scope_metadata("connection=7 source=127.0.0.1:1 target=not-cp.cloudflare.com:443", a.DEFAULT_TARGET)
        self.assertEqual(fields["targetClass"], "other")


if __name__ == "__main__":
    unittest.main()
