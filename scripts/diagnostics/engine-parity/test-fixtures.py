#!/usr/bin/env python3
"""Use real local protocol sockets; no native engine, device or private input."""
import importlib.util
import ipaddress
import json
from pathlib import Path
import socket
import struct
import time
import unittest
from urllib.error import HTTPError
from urllib.request import ProxyHandler, Request, build_opener

spec = importlib.util.spec_from_file_location('engine_fixture', Path(__file__).with_name('fixtures.py'))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)
NONCE = '1234567890abcdef1234567890abcdef'


class ProtocolTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.servers = fixture.FixtureServers('127.0.0.1', '127.0.0.1', NONCE,
            0, 0, 0, 0, 0, 0, ('223.5.5.5', '119.29.29.29')).start()
        cls.state = cls.servers.state
        cls.opener = build_opener(ProxyHandler({}))

    @classmethod
    def tearDownClass(cls):
        cls.servers.close()
        assert all(not thread.is_alive() for thread in cls.servers.threads)

    def setUp(self):
        self.state.control({'revision': 'A', 'httpDelayMs': 0, 'httpStatus': 200,
            'proxyADelayMs': 0, 'proxyBDelayMs': 0, 'proxyAFail': False,
            'proxyBFail': False, 'dnsFail': False, 'dnsTtl': 2,
            'udpEchoFail': False, 'udpEchoDelayMs': 0,
            'tcpEchoFail': False, 'tcpEchoDelayMs': 0})

    def fetch(self, path, data=None, headers=None):
        request = Request(self.state.url(path), data=data, headers=headers or {})
        return self.opener.open(request, timeout=5)

    def control(self, **values):
        with self.fetch('/control', json.dumps(values).encode(), {'Content-Type': 'application/json'}) as response:
            self.assertEqual(response.status, 200)

    def connect_proxy(self, which='A', host='fixture.qa.invalid', port=None):
        stream = socket.create_connection(('127.0.0.1', self.state.ports['proxy' + which]), timeout=3)
        stream.sendall(b'\x05\x01\x00')
        self.assertEqual(fixture.exact(stream, 2), b'\x05\x00')
        domain = host.encode()
        stream.sendall(b'\x05\x01\x00\x03' + bytes([len(domain)]) + domain +
                       struct.pack('!H', port or self.state.ports['http']))
        reply = fixture.exact(stream, 10)
        return stream, reply[1]

    def dns_query(self, query_type, tcp=False):
        domain = f'probe.{NONCE}.qa.invalid'
        question = b''.join(bytes([len(label)]) + label.encode() for label in domain.split('.')) + b'\0' + struct.pack('!HH', query_type, 1)
        request = struct.pack('!6H', 0x1234, 0x100, 1, 0, 0, 0) + question
        if tcp:
            with socket.create_connection(('127.0.0.1', self.state.ports['dnsTcp']), timeout=3) as stream:
                stream.sendall(struct.pack('!H', len(request)) + request)
                return fixture.exact(stream, struct.unpack('!H', fixture.exact(stream, 2))[0])
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as stream:
            stream.settimeout(3)
            stream.sendto(request, ('127.0.0.1', self.state.ports['dnsUdp']))
            return stream.recv(4096)

    def associate(self, which='A'):
        control = socket.create_connection(('127.0.0.1', self.state.ports['proxy' + which]), timeout=3)
        control.sendall(b'\x05\x01\x00')
        self.assertEqual(fixture.exact(control, 2), b'\x05\x00')
        control.sendall(b'\x05\x03\x00\x01' + b'\0' * 6)
        reply = fixture.exact(control, 10)
        return control, reply[1], (socket.inet_ntoa(reply[4:8]), struct.unpack('!H', reply[8:10])[0])

    def udp_body(self, case='public_udp'):
        return f'paws-engine-parity-udp:{NONCE}\ncase={case}\n'.encode()

    def tcp_body(self, case='public_tcp'):
        return f'paws-engine-parity-tcp:{NONCE}\ncase={case}\n'.encode()

    def read_to_close(self, stream):
        result = bytearray()
        while True:
            part = stream.recv(1025 - len(result))
            if not part:
                return bytes(result)
            result.extend(part)
            self.assertLessEqual(len(result), 1024)

    def udp_frame(self, payload, port=None, host='127.0.0.1'):
        address = b'\x01' + socket.inet_aton(host) if host == '127.0.0.1' else b'\x03' + bytes([len(host)]) + host.encode()
        return b'\0\0\0' + address + struct.pack('!H', port or self.state.ports['udpEcho']) + payload

    def wait_event(self, event, **fields):
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            with self.state.lock:
                found = next((record for record in reversed(self.state.events)
                    if record['protocol'] == 'socks5-udp' and record['event'] == event
                    and all(record.get(key) == value for key, value in fields.items())), None)
            if found: return found
            time.sleep(.005)
        self.fail(f'Missing bounded association event {event} {fields}')

    def test_http_profiles_and_provider_revision_etag(self):
        with self.fetch('/profiles/features.yaml') as response:
            profile = json.load(response)
        self.assertEqual(len(profile['proxies']), 2)
        self.assertEqual(profile['proxy-groups'][1]['type'], 'url-test')
        self.assertIn(f'DST-PORT,{self.state.ports["tcpEcho"]},QA-SELECT', profile['rules'])
        self.assertIn(NONCE, profile['proxy-providers']['QA-PROVIDER']['url'])
        with self.fetch('/providers/proxies.yaml') as response:
            self.assertEqual(json.load(response)['proxies'][0]['name'], 'QA-provider-A')
        with self.assertRaises(HTTPError) as context:
            self.fetch('/providers/proxies.yaml', headers={'If-None-Match': '"qa-A"'})
        self.assertEqual(context.exception.code, 304)
        self.control(revision='B')
        with self.fetch('/providers/proxies.yaml', headers={'If-None-Match': '"qa-A"'}) as response:
            self.assertEqual(json.load(response)['proxies'][0]['name'], 'QA-provider-B')
        with self.fetch('/providers/rules.yaml') as response:
            self.assertEqual(json.load(response)['payload'], ['provider-b.fixture.invalid'])

    def test_real_resolver_policy_is_explicit_and_fixture_names_are_scoped(self):
        for route in ('/profiles/direct.yaml', '/profiles/features.yaml'):
            with self.fetch(route) as response:
                profile = json.load(response)
            dns = profile['dns']
            self.assertEqual(dns['nameserver'], ['223.5.5.5', '119.29.29.29'])
            self.assertEqual(dns['nameserver-policy'], {f'+.{NONCE}.qa.invalid':
                [f'udp://127.0.0.1:{self.state.ports["dnsUdp"]}']})
            self.assertTrue(dns['ipv6'])
            self.assertNotIn('fallback', dns)
            self.assertIn('DOMAIN,cp.cloudflare.com,DIRECT', profile['rules'])
        unconfigured = fixture.FixtureState(NONCE, '127.0.0.1')
        with self.assertRaisesRegex(ValueError, 'explicit Internet DNS'):
            unconfigured.document('/profiles/features.yaml', 'A')
        for values in ([], ['system'], ['https://dns.invalid'], ['1.1.1.1'] * 2,
                       ['1.1.1.1', '2.2.2.2', '3.3.3.3'], ['0.0.0.0'], ['ff02::1'], ['fe80::1%en0']):
            with self.subTest(values=values), self.assertRaises(ValueError):
                fixture.internet_resolvers(values)
        self.assertEqual(fixture.internet_resolvers(['2001:0db8::1']), ('2001:db8::1',))

    def test_http_controlled_delay_and_failure(self):
        self.control(httpDelayMs=60, httpStatus=500)
        start = time.monotonic()
        with self.assertRaises(HTTPError) as context:
            self.fetch('/probe')
        self.assertEqual(context.exception.code, 500)
        self.assertGreaterEqual(time.monotonic() - start, .05)

    def test_real_socks5_tunnels_exact_fixture_http_bytes(self):
        for which in ('A', 'B'):
            with self.subTest(node=which):
                stream, code = self.connect_proxy(which)
                with stream:
                    self.assertEqual(code, 0)
                    stream.sendall(f'GET /probe?nonce={NONCE} HTTP/1.1\r\nHost: fixture.qa.invalid\r\nConnection: close\r\n\r\n'.encode())
                    response = bytearray()
                    while True:
                        part = stream.recv(4096)
                        if not part:
                            break
                        response.extend(part)
                    self.assertIn(b'200 OK', response)
                    self.assertTrue(response.endswith(f'paws-engine-parity:{NONCE}\n'.encode()))

    def test_proxy_target_allowlist_and_failure_control(self):
        stream, code = self.connect_proxy(host='personal.example', port=443)
        stream.close()
        self.assertEqual(code, 2)
        self.control(proxyAFail=True, proxyBDelayMs=60)
        stream, code = self.connect_proxy()
        stream.close()
        self.assertEqual(code, 2)
        start = time.monotonic()
        stream, code = self.connect_proxy('B')
        stream.close()
        self.assertEqual(code, 0)
        self.assertGreaterEqual(time.monotonic() - start, .05)

    def test_udp_tcp_dns_a_aaaa_ttl_and_revision(self):
        for tcp in (False, True):
            for query_type, ip in ((1, '203.0.113.11'), (28, '2001:db8::11')):
                with self.subTest(tcp=tcp, queryType=query_type):
                    response = self.dns_query(query_type, tcp)
                    self.assertEqual(struct.unpack('!6H', response[:12])[3], 1)
                    self.assertTrue(response.endswith(ipaddress.ip_address(ip).packed))
                    size = 4 if query_type == 1 else 16
                    self.assertEqual(struct.unpack('!I', response[-size - 6:-size - 2])[0], 2)
        self.control(revision='B')
        self.assertTrue(self.dns_query(1).endswith(ipaddress.ip_address('203.0.113.12').packed))
        self.control(dnsFail=True)
        self.assertEqual(struct.unpack('!H', self.dns_query(1)[2:4])[0] & 15, 2)

    def test_nonce_gate_bounded_stream_and_no_private_echo(self):
        url = self.state.url('/probe').replace(NONCE, 'not-the-case')
        with self.assertRaises(HTTPError) as context:
            self.opener.open(url, timeout=3)
        self.assertEqual(context.exception.code, 403)
        with self.fetch('/stream') as response:
            self.assertEqual(len(response.read()), fixture.MAX_STREAM_BYTES)
        with self.assertRaises(HTTPError) as context:
            self.fetch('/control', b'{"unknownField": true}')
        self.assertEqual(context.exception.code, 400)

    def test_events_have_bounded_watermark_and_no_arbitrary_destination(self):
        self.test_proxy_target_allowlist_and_failure_control()
        for _ in range(fixture.MAX_EVENTS + 5):
            self.state.record('test', 'bounded', bytes=1)
        with self.fetch('/events') as response:
            events = json.load(response)
        self.assertEqual(len(events['events']), fixture.MAX_EVENTS)
        self.assertEqual(events['events'][-1]['sequence'], events['highWatermark'])
        self.assertNotIn('personal.example', json.dumps(events))

    def test_plain_udp_echo_requires_exact_public_nonce_and_case(self):
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.settimeout(.15)
            body = self.udp_body()
            udp.sendto(body, ('127.0.0.1', self.state.ports['udpEcho']))
            self.assertEqual(udp.recv(4096), body)
            udp.sendto(b'private-or-unframed-data', ('127.0.0.1', self.state.ports['udpEcho']))
            with self.assertRaises(socket.timeout): udp.recv(4096)
        with self.state.lock:
            self.assertNotIn('private-or-unframed-data', json.dumps(list(self.state.events)))

    def test_raw_tcp_echo_requires_exact_nonce_case_and_does_not_speak_http(self):
        body = self.tcp_body()
        with socket.create_connection(('127.0.0.1', self.state.ports['tcpEcho']), timeout=2) as stream:
            # Real stream fragmentation; no HTTP headers/status on this port.
            stream.sendall(body[:13]); stream.sendall(body[13:])
            self.assertEqual(self.read_to_close(stream), body)
        for invalid in (body.replace(NONCE.encode(), b'0' * 32), b'private\ncontent\n', b'x' * 1025):
            with self.subTest(size=len(invalid)), socket.create_connection(
                    ('127.0.0.1', self.state.ports['tcpEcho']), timeout=2) as stream:
                stream.sendall(invalid)
                try:
                    self.assertEqual(stream.recv(1024), b'')
                except ConnectionResetError:
                    pass
        with self.state.lock:
            self.assertNotIn('private\\ncontent', json.dumps(list(self.state.events)))

    def test_real_socks_tcp_echo_for_both_nodes_and_controlled_failure(self):
        self.control(tcpEchoDelayMs=30, proxyBDelayMs=30)
        for which in ('A', 'B'):
            with self.subTest(node=which):
                started = time.monotonic()
                stream, code = self.connect_proxy(which, port=self.state.ports['tcpEcho'])
                with stream:
                    self.assertEqual(code, 0)
                    body = self.tcp_body('node_' + which)
                    stream.sendall(body)
                    self.assertEqual(self.read_to_close(stream), body)
                self.assertGreaterEqual(time.monotonic() - started, .025)
        self.control(tcpEchoFail=True)
        stream, code = self.connect_proxy(port=self.state.ports['tcpEcho'])
        with stream:
            self.assertEqual(code, 0)
            stream.sendall(self.tcp_body('controlled_failure'))
            self.assertEqual(stream.recv(1024), b'')
        with self.state.lock:
            connected = [item for item in self.state.events if item['protocol'] == 'socks5'
                         and item['event'] == 'connected' and item.get('target') == 'tcp-echo']
        self.assertTrue(all(any(item['node'] == node for item in connected) for node in ('A', 'B')))

    def test_real_socks_udp_associate_relays_echo_for_both_nodes(self):
        for which in ('A', 'B'):
            with self.subTest(node=which):
                control, code, relay = self.associate(which)
                with control, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
                    self.assertEqual(code, 0)
                    udp.settimeout(2)
                    body = self.udp_body('node_' + which)
                    udp.sendto(self.udp_frame(body), relay)
                    host, port, actual = fixture.decode_socks_udp(udp.recv(4096))
                    self.assertEqual((host, port, actual), ('127.0.0.1', self.state.ports['udpEcho'], body))
                    self.wait_event('returned', node=which)

    def test_udp_association_delay_failure_switch_and_live_control_cleanup(self):
        control, code, relay = self.associate('B')
        self.assertEqual(code, 0)
        association = self.wait_event('associated', node='B')['association']
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.settimeout(.15)
            body = self.udp_body('delayed_B')
            self.control(proxyBFail=True)
            udp.sendto(self.udp_frame(body), relay)
            with self.assertRaises(socket.timeout): udp.recv(4096)
            self.control(proxyBFail=False, proxyBDelayMs=60)
            start = time.monotonic()
            udp.sendto(self.udp_frame(body), relay)
            self.assertEqual(fixture.decode_socks_udp(udp.recv(4096))[2], body)
            self.assertGreaterEqual(time.monotonic() - start, .05)
            control.close()
            self.wait_event('closed', association=association)
            udp.sendto(self.udp_frame(body), relay)
            with self.assertRaises(socket.timeout): udp.recv(4096)
        self.control(proxyAFail=True)
        control, code, _ = self.associate('A')
        control.close()
        self.assertEqual(code, 2)

    def test_udp_association_rejects_fragment_outside_target_and_foreign_sender(self):
        control, code, relay = self.associate()
        self.assertEqual(code, 0)
        with control, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as foreign:
            udp.settimeout(.1); foreign.settimeout(.1)
            body = self.udp_body('allowlisted')
            invalid = bytearray(self.udp_frame(body)); invalid[2] = 1
            udp.sendto(invalid, relay)
            with self.assertRaises(socket.timeout): udp.recv(4096)
            udp.sendto(self.udp_frame(body, port=443, host='outside.invalid'), relay)
            with self.assertRaises(socket.timeout): udp.recv(4096)
            udp.sendto(self.udp_frame(body, host='fixture.qa.invalid'), relay)
            self.assertEqual(fixture.decode_socks_udp(udp.recv(4096))[2], body)
            foreign.sendto(self.udp_frame(body), relay)
            with self.assertRaises(socket.timeout): foreign.recv(4096)
            udp.sendto(self.udp_frame(body), relay)
            self.assertEqual(fixture.decode_socks_udp(udp.recv(4096))[2], body)

    def test_socks_udp_association_dns_uses_real_dns_protocol_separately_from_echo(self):
        control, code, relay = self.associate()
        self.assertEqual(code, 0)
        name = f'udp.{NONCE}.qa.invalid'
        question = b''.join(bytes([len(label)]) + label.encode() for label in name.split('.')) + b'\0' + struct.pack('!HH', 1, 1)
        dns = struct.pack('!6H', 0x5678, 0x100, 1, 0, 0, 0) + question
        with control, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            udp.settimeout(2)
            udp.sendto(self.udp_frame(dns, port=self.state.ports['dnsUdp']), relay)
            host, port, answer = fixture.decode_socks_udp(udp.recv(4096))
            self.assertEqual(port, self.state.ports['dnsUdp'])
            self.assertTrue(answer.endswith(ipaddress.ip_address('203.0.113.11').packed))
            self.assertEqual(struct.unpack('!H', answer[:2])[0], 0x5678)

    def test_udp_association_datagram_budget_revokes_controlled_relay(self):
        control, code, relay = self.associate()
        self.assertEqual(code, 0)
        association = self.wait_event('associated', node='A')['association']
        with control, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as udp:
            for _ in range(fixture.MAX_UDP_DATAGRAMS):
                udp.sendto(b'bad-fragment', relay)
                time.sleep(.001)
            closed = self.wait_event('closed', association=association)
            self.assertEqual(closed['datagrams'], fixture.MAX_UDP_DATAGRAMS)
            self.assertEqual(closed['bytes'], 0)


if __name__ == '__main__':
    unittest.main()
