#!/usr/bin/env python3
"""Regression tests for benchmark evidence admission and measurements."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('benchmark', Path(__file__).with_name('benchmark-harmony-vpn.py'))
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)

class EvidenceTests(unittest.TestCase):
    def test_request_identity_is_assigned_before_completion(self):
        with patch.object(benchmark, 'curl_sample', return_value={'success':False}):
            self.assertEqual(benchmark.numbered_curl_sample(83, 'https://example.test', None, 2)['requestId'],83)

    def test_probe_id_is_sent_only_in_connect_header(self):
        def run(command, timeout):
            self.assertIn('--proxy-header', command)
            self.assertEqual(command[command.index('--proxy-header')+1], 'X-TinyPaws-Probe: tp-0123456789abcdef-17')
            return subprocess.CompletedProcess(command,0,'204\t0.1\t0\t0.1\t0\t0.05\t0.01\t200\t50001','')
        with patch.object(benchmark,'run',side_effect=run):
            benchmark.curl_sample('https://example.test','http://127.0.0.1:1',2,'tp-0123456789abcdef-17')
        with patch.object(benchmark,'curl_sample',return_value={'success':True}) as sample:
            result=benchmark.numbered_curl_sample(17,'https://example.test','http://127.0.0.1:1',2)
            self.assertRegex(result['probeId'],r'^tp-[0-9a-f]{16}-17$')
            self.assertEqual(sample.call_args.kwargs['probe_id'],result['probeId'])

    def test_proxy_connect_status_is_separate_from_target_http_status(self):
        result=subprocess.CompletedProcess([],35,'000\t5.2\t0\t0\t0\t0\t0.001\t200\t53221','TLS ended')
        with patch.object(benchmark,'run',return_value=result):
            sample=benchmark.curl_sample('https://example.test','http://127.0.0.1:1',12)
        self.assertFalse(sample['success'])
        self.assertEqual((sample['status'],sample['proxyConnectStatus'],sample['hostLocalPort']),(0,200,53221))

    def test_explicit_proxy_cannot_be_bypassed_by_no_proxy(self):
        with patch.dict(benchmark.os.environ, {'NO_PROXY':'example.test','no_proxy':'example.test'}):
            self.assertNotIn('NO_PROXY',benchmark.clean_env())
            self.assertNotIn('no_proxy',benchmark.clean_env())
        def run(command, timeout):
            self.assertEqual(command[command.index('--noproxy')+1],'')
            return subprocess.CompletedProcess(command,0,'204\t0.1\t0\t0.1\t0\t0.05\t0.01\t000\t50001','')
        with patch.object(benchmark,'run',side_effect=run):
            sample=benchmark.curl_sample('https://example.test','http://127.0.0.1:1',2)
        self.assertFalse(sample['success'])
        self.assertEqual(sample['error'],'proxy-connect-not-established')

    def test_rejects_fallback_and_wrong_ingress(self):
        args = argparse.Namespace(engine='sing-box', device_mixed_port=18492)
        record = dict(format='TinyPaws-engine-session-v1', engine='mihomo', ingressPort=18490,
                      session='s', profileFingerprint='f', buildFingerprint='b', version='v', pid=123)
        with patch.object(benchmark, 'hdc', return_value=subprocess.CompletedProcess([], 0, json.dumps(record))):
            with self.assertRaisesRegex(ValueError, 'differs'):
                benchmark.engine_session(args)
        record.update(engine='sing-box')
        with patch.object(benchmark, 'hdc', return_value=subprocess.CompletedProcess([], 0, json.dumps(record))):
            with self.assertRaisesRegex(ValueError, 'dedicated'):
                benchmark.engine_session(args)
        record.update(ingressPort=18492)
        with patch.object(benchmark, 'hdc', return_value=subprocess.CompletedProcess([], 0, json.dumps(record))):
            self.assertEqual(benchmark.engine_session(args)['pid'], 123)

    def test_throughput_requires_complete_bodies_and_every_stream(self):
        transfer = dict(singleStream=dict(success=True, bytes=100),
                        multiStream=dict(streams=4, successes=4, bytes=400))
        self.assertTrue(benchmark.throughput_passed(transfer, 100))
        transfer['multiStream']['successes'] = 3
        self.assertFalse(benchmark.throughput_passed(transfer, 100))
        transfer['multiStream'].update(successes=4, bytes=399)
        self.assertFalse(benchmark.throughput_passed(transfer, 100))
        self.assertTrue(benchmark.throughput_passed({}, 0))

    def test_curl_keeps_binary_body_separate_from_metrics(self):
        def run(command, timeout):
            Path(command[command.index('--output')+1]).write_bytes(b'\xff\x00\xfe')
            return subprocess.CompletedProcess(command, 0, '200\t0.5\t3\t0.2\t6\t0.1\t0.05\t200\t50001', '')
        with patch.object(benchmark, 'run', side_effect=run):
            sample = benchmark.curl_sample('https://example.test', 'http://127.0.0.1:1', 2)
        self.assertTrue(sample['success'])
        self.assertEqual((sample['latencyMs'], sample['ttfbMs'], sample['bytes']), (500,200,3))
        self.assertEqual(len(sample['bodyFingerprint']), 16)
        with patch.object(benchmark, 'run', side_effect=subprocess.TimeoutExpired('curl', 2)):
            self.assertEqual(benchmark.curl_sample('https://example.test', None, 2)['error'], 'subprocess-timeout')

    def test_hdc_zero_exit_error_and_stale_forward_are_rejected(self):
        args = argparse.Namespace(target="phone")
        with patch.object(benchmark, 'forward_mapping', return_value='tcp:18490'):
            with self.assertRaisesRegex(ValueError, 'already has'):
                benchmark.open_forward(args, 'tcp:18890', 'tcp:18492')
        with patch.object(benchmark, 'forward_mapping', return_value=None), patch.object(benchmark, 'hdc', return_value=subprocess.CompletedProcess([],0,'[Fail]TCP Port listen failed','')):
            with self.assertRaisesRegex(ValueError, 'did not establish'):
                benchmark.open_forward(args, 'tcp:18890', 'tcp:18492')
        with patch.object(benchmark, 'forward_mapping', side_effect=['tcp:18492',None]), patch.object(benchmark, 'hdc') as hdc:
            benchmark.close_owned_forward(args, 'tcp:18890', 'tcp:18492')
            hdc.assert_called_once_with(args, 'fport','rm','tcp:18890','tcp:18492',timeout=5)

    def test_proxy_environment_is_removed_and_percentiles_are_deterministic(self):
        with patch.dict(benchmark.os.environ, {'HTTPS_PROXY':'private', 'ALL_PROXY':'private', 'TASK_VALUE':'keep'}):
            self.assertNotIn('HTTPS_PROXY', benchmark.clean_env())
            self.assertNotIn('ALL_PROXY', benchmark.clean_env())
            self.assertEqual(benchmark.clean_env()['TASK_VALUE'], 'keep')
        self.assertEqual(benchmark.percentile([100,1,4,3,2], .5), 3)
        self.assertIsNone(benchmark.percentile([], .99))

if __name__ == '__main__':
    unittest.main()
