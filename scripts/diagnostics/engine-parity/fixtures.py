#!/usr/bin/env python3
"""Bounded public synthetic HTTP/SOCKS5/DNS fixtures. Never read app/user files."""
import argparse
from collections import deque
import ipaddress
import json
import re
import select
import signal
import socket
import socketserver
import struct
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlsplit


MAX_EVENTS = 512
MAX_STREAM_BYTES = 1 << 20
MAX_PROXY_BYTES = 4 << 20
MAX_UDP_BYTES = 256 << 10
MAX_UDP_DATAGRAMS = 64
MAX_UDP_DATAGRAM_BYTES = 4096
UDP_CASE = re.compile(rb'paws-engine-parity-udp:([a-f0-9]{32})\ncase=([a-zA-Z0-9_-]{1,48})\n\Z')
TCP_CASE = re.compile(rb'paws-engine-parity-tcp:([a-f0-9]{32})\ncase=([a-zA-Z0-9_-]{1,48})\n\Z')


def internet_resolvers(values):
    if not 1 <= len(values) <= 2:
        raise ValueError('one or two explicit numeric Internet DNS resolvers required')
    result = []
    for value in values:
        if '%' in value:
            raise ValueError('DNS resolver scope identifiers are not supported')
        address = ipaddress.ip_address(value)
        if address.is_unspecified or address.is_multicast:
            raise ValueError('unicast DNS resolver address required')
        normalized = str(address)
        if normalized in result:
            raise ValueError('DNS resolvers must be distinct')
        result.append(normalized)
    return tuple(result)


def exact(stream, size):
    data = bytearray()
    while len(data) < size:
        part = stream.recv(size - len(data))
        if not part:
            raise EOFError('short protocol frame')
        data.extend(part)
    return bytes(data)


class FixtureState:
    def __init__(self, nonce, advertise_host, internet_dns=()):
        if len(nonce) != 32 or any(value not in '0123456789abcdef' for value in nonce):
            raise ValueError('nonce must be 32 lowercase hexadecimal characters')
        ipaddress.IPv4Address(advertise_host)
        self.nonce, self.host = nonce, advertise_host
        self.internet_dns = internet_resolvers(internet_dns) if internet_dns else ()
        self.lock = threading.Lock()
        self.events = deque(maxlen=MAX_EVENTS)
        self.sequence = 0
        self.stopped = threading.Event()
        self.udp_slots = threading.BoundedSemaphore(8)
        self.udp_sequence = 0
        self.controls = {'revision': 'A', 'httpDelayMs': 0, 'httpStatus': 200,
                         'proxyADelayMs': 0, 'proxyBDelayMs': 100,
                         'proxyAFail': False, 'proxyBFail': False,
                         'dnsFail': False, 'dnsTtl': 2,
                         'udpEchoFail': False, 'udpEchoDelayMs': 0,
                         'tcpEchoFail': False, 'tcpEchoDelayMs': 0}
        self.ports = {}

    def record(self, protocol, event, **numbers):
        # Callers pass fixed route labels/public fixture identities and numbers,
        # never arbitrary Host headers, proxy destinations, request bodies/URLs.
        with self.lock:
            self.sequence += 1
            self.events.append({'sequence': self.sequence, 'atUnixNs': time.time_ns(),
                                'protocol': protocol, 'event': event, **numbers})

    def read_controls(self):
        with self.lock:
            return dict(self.controls)

    def control(self, values):
        if not isinstance(values, dict) or set(values) - set(self.controls):
            raise ValueError('unknown control')
        for key, value in values.items():
            if key == 'revision':
                if value not in ('A', 'B'):
                    raise ValueError('revision must be A or B')
            elif key.endswith('Fail'):
                if type(value) is not bool:
                    raise ValueError('failure switch must be boolean')
            elif type(value) is not int or not 0 <= value <= (3000 if key.endswith('Ms') else 65535):
                raise ValueError('control outside bounded integer range')
            elif key == 'httpStatus' and value not in (200, 404, 500):
                raise ValueError('HTTP status must be 200/404/500')
        with self.lock:
            self.controls.update(values)

    def url(self, path):
        return f'http://{self.host}:{self.ports["http"]}{path}?nonce={self.nonce}'

    def node(self, name, which):
        return {'name': name, 'type': 'socks5', 'server': self.host,
                'port': self.ports['proxy' + which], 'udp': True}

    def document(self, route, revision):
        # JSON is also valid YAML; profile/provider parsers consume real bytes.
        if route == '/providers/proxies.yaml':
            return {'proxies': [self.node('QA-provider-' + revision, 'A' if revision == 'A' else 'B')]}
        if route == '/providers/rules.yaml':
            return {'payload': ['provider-' + revision.lower() + '.fixture.invalid']}
        if not self.internet_dns:
            raise ValueError('explicit Internet DNS resolver policy required for profiles')
        direct = route == '/profiles/direct.yaml'
        result = {'mode': 'direct' if direct else 'rule', 'log-level': 'debug',
            'proxies': [] if direct else [self.node('QA-A', 'A'), self.node('QA-B', 'B')],
            'proxy-groups': [{'name': 'QA-SELECT', 'type': 'select', 'proxies': ['DIRECT'] if direct else ['QA-A', 'QA-B', 'DIRECT']}],
            'rules': ['DOMAIN,cp.cloudflare.com,DIRECT', 'MATCH,DIRECT'],
            'dns': {'enable': True, 'listen': '127.0.0.1:1053', 'enhanced-mode': 'redir-host',
                    'ipv6': True, 'nameserver': list(self.internet_dns),
                    'nameserver-policy': {f'+.{self.nonce}.qa.invalid':
                        [f'udp://{self.host}:{self.ports["dnsUdp"]}']}},
            'paws': {'mixed-enabled': False, 'controller-enabled': False}}
        if not direct:
            result['proxy-groups'] += [{'name': 'QA-AUTO', 'type': 'url-test', 'proxies': ['QA-A', 'QA-B'],
                'url': self.url('/probe'), 'interval': 1, 'tolerance': 0},
                {'name': 'QA-PROVIDED', 'type': 'select', 'use': ['QA-PROVIDER']}]
            result['proxy-providers'] = {'QA-PROVIDER': {'type': 'http', 'url': self.url('/providers/proxies.yaml'),
                'path': './providers/qa-proxies.yaml', 'interval': 60}}
            result['rule-providers'] = {'QA-RULES': {'type': 'http', 'behavior': 'domain',
                'url': self.url('/providers/rules.yaml'), 'path': './providers/qa-rules.yaml', 'interval': 60}}
            result['rules'] = ['DOMAIN,cp.cloudflare.com,DIRECT',
                f'DST-PORT,{self.ports["udpEcho"]},QA-SELECT',
                f'DST-PORT,{self.ports["tcpEcho"]},QA-SELECT',
                'RULE-SET,QA-RULES,QA-SELECT', 'MATCH,DIRECT']
        return result


class FixtureHTTP(BaseHTTPRequestHandler):
    protocol_version = 'HTTP/1.1'

    def setup(self):
        super().setup()
        self.connection.settimeout(5)

    def log_message(self, *_):
        pass  # Never log arbitrary request lines/Host headers.

    def authorized(self):
        parts = urlsplit(self.path)
        query = parse_qs(parts.query)
        return parts.path, query.get('nonce') == [self.server.state.nonce]

    def reply(self, status, body=b'', headers=None):
        self.send_response(status)
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Connection', 'close')
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_POST(self):
        route, allowed = self.authorized()
        if not allowed or route != '/control':
            self.reply(403)
            return
        try:
            size = int(self.headers.get('Content-Length', '-1'))
            if not 0 <= size <= 2048:
                raise ValueError('bounded control body required')
            self.server.state.control(json.loads(self.rfile.read(size)))
            self.server.state.record('http', 'control')
            self.reply(200, b'{"ok":true}\n')
        except (ValueError, TypeError):
            self.reply(400)

    def do_GET(self):
        route, allowed = self.authorized()
        state = self.server.state
        if not allowed:
            self.reply(403)
            return
        if route == '/events':
            with state.lock:
                body = json.dumps({'highWatermark': state.sequence, 'boundedCapacity': MAX_EVENTS,
                                   'events': list(state.events)}).encode()
            self.reply(200, body, {'Content-Type': 'application/json'})
            return
        routes = {'/probe', '/stream', '/echo', '/profiles/direct.yaml', '/profiles/features.yaml',
                  '/providers/proxies.yaml', '/providers/rules.yaml'}
        if route not in routes:
            self.reply(404)
            return
        controls = state.read_controls()
        time.sleep(controls['httpDelayMs'] / 1000)
        status = controls['httpStatus']
        etag = '"qa-' + controls['revision'] + '"'
        headers = {'ETag': etag, 'Last-Modified': 'Thu, 22 Oct 2026 00:00:00 GMT'}
        if route.endswith('.yaml'):
            try:
                body = json.dumps(state.document(route, controls['revision'])).encode()
            except ValueError:
                state.record('http', 'profile-policy-unconfigured', status=503)
                self.reply(503, b'explicit Internet DNS resolver policy required\n')
                return
            headers['Content-Type'] = 'application/yaml'
            if status == 200 and self.headers.get('If-None-Match') == etag:
                status, body = 304, b''
        elif route == '/stream':
            body = (f'paws-engine-parity:{state.nonce}\n'.encode() * 32768)[:MAX_STREAM_BYTES]
        elif route == '/echo':
            # Echo only the public case ID; never arbitrary request body data.
            case = parse_qs(urlsplit(self.path).query).get('case', [''])[0]
            if len(case) > 80 or any(not (c.isascii() and (c.isalnum() or c in '_-')) for c in case):
                self.reply(400)
                return
            body = f'paws-engine-parity:{state.nonce}\ncase={case}\n'.encode()
        else:
            body = f'paws-engine-parity:{state.nonce}\n'.encode()
        if status not in (200, 304):
            body = f'controlled HTTP {status}\n'.encode()
        state.record('http', route, status=status, bytes=len(body), revision=controls['revision'])
        self.reply(status, body, headers)


class BoundedThreads(socketserver.ThreadingMixIn):
    daemon_threads = True
    daemon_threads = True

    def __init__(self, *args, **kwargs):
        self._handler_slots = threading.BoundedSemaphore(32)
        super().__init__(*args, **kwargs)

    def process_request(self, request, client_address):
        if not self._handler_slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, client_address)
        except BaseException:
            self._handler_slots.release()
            raise

    def process_request_thread(self, request, client_address):
        try:
            super().process_request_thread(request, client_address)
        finally:
            self._handler_slots.release()


class ThreadedTCP(BoundedThreads, socketserver.TCPServer):
    allow_reuse_address = True


class ThreadedUDP(BoundedThreads, socketserver.UDPServer):
    allow_reuse_address = True


class SocksNode(socketserver.BaseRequestHandler):
    def handle(self):
        stream = self.request
        stream.settimeout(8)
        state, which = self.server.state, self.server.which
        try:
            version, count = exact(stream, 2)
            methods = exact(stream, count)
            if version != 5 or 0 not in methods:
                stream.sendall(b'\x05\xff')
                return
            stream.sendall(b'\x05\x00')
            version, command, reserved, address_type = exact(stream, 4)
            if address_type == 1:
                host = socket.inet_ntop(socket.AF_INET, exact(stream, 4))
            elif address_type == 3:
                host = exact(stream, exact(stream, 1)[0]).decode('ascii')
            else:
                raise ValueError('only bounded fixture IPv4/domain supported')
            port = struct.unpack('!H', exact(stream, 2))[0]
            controls = state.read_controls()
            if version == 5 and command == 3 and reserved == 0:
                self.associate_udp(host, port)
                return
            time.sleep(controls['proxy' + which + 'DelayMs'] / 1000)
            permitted = (version == 5 and command == 1 and reserved == 0
                         and host in {state.host, '127.0.0.1', 'fixture.qa.invalid'}
                         and port in {state.ports['http'], state.ports['tcpEcho']})
            if not permitted or controls['proxy' + which + 'Fail']:
                state.record('socks5', 'rejected', node=which, allowedTarget=permitted)
                stream.sendall(b'\x05\x02\x00\x01' + b'\0' * 6)
                return
            with socket.create_connection(('127.0.0.1', port), timeout=3) as upstream:
                stream.sendall(b'\x05\x00\x00\x01' + b'\0' * 6)
                target = 'tcp-echo' if port == state.ports['tcpEcho'] else 'http'
                state.record('socks5', 'connected', node=which, target=target)
                total, deadline = 0, time.monotonic() + 10
                while not state.stopped.is_set() and total < MAX_PROXY_BYTES and time.monotonic() < deadline:
                    ready, _, _ = select.select([stream, upstream], [], [], .2)
                    for source in ready:
                        data = source.recv(min(65536, MAX_PROXY_BYTES - total))
                        if not data:
                            state.record('socks5', 'closed', node=which, bytes=total)
                            return
                        total += len(data)
                        (upstream if source is stream else stream).sendall(data)
                state.record('socks5', 'bounded-close', node=which, bytes=total)
        except (OSError, EOFError, ValueError, UnicodeError):
            state.record('socks5', 'protocol-error', node=which)

    def associate_udp(self, requested_host, requested_port):
        state, which, control = self.server.state, self.server.which, self.request
        peer = self.client_address[0]
        permitted = requested_host in {'0.0.0.0', peer}
        if not permitted or state.read_controls()['proxy' + which + 'Fail'] or not state.udp_slots.acquire(blocking=False):
            state.record('socks5-udp', 'association-rejected', node=which, allowedTarget=permitted)
            control.sendall(b'\x05\x02\x00\x01' + b'\0' * 6)
            return
        with state.lock:
            state.udp_sequence += 1
            association = state.udp_sequence
        total, packets = 0, 0
        try:
            with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as relay, socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as upstream:
                relay.bind((self.server.server_address[0], 0))
                upstream.bind(('127.0.0.1', 0))
                relay.setblocking(False)
                upstream.setblocking(False)
                control.sendall(b'\x05\x00\x00\x01' + socket.inet_aton(state.host) + struct.pack('!H', relay.getsockname()[1]))
                state.record('socks5-udp', 'associated', node=which, association=association)
                client = None
                deadline, idle = time.monotonic() + 10, time.monotonic() + 3
                pending = deque(maxlen=MAX_UDP_DATAGRAMS)
                while not state.stopped.is_set() and time.monotonic() < min(deadline, idle) and total < MAX_UDP_BYTES and packets < MAX_UDP_DATAGRAMS:
                    ready, _, _ = select.select([control, relay, upstream], [], [], .05)
                    if control in ready:
                        # The TCP association is a lifetime owner, never a data
                        # channel. EOF or unexpected bytes revoke it immediately.
                        break
                    if relay in ready:
                        frame, sender = relay.recvfrom(MAX_UDP_DATAGRAM_BYTES + 1)
                        packets += 1
                        if sender[0] != peer or requested_port and sender[1] != requested_port or client is not None and sender != client:
                            state.record('socks5-udp', 'foreign-client-rejected', node=which, association=association)
                            continue
                        try:
                            host, port, payload = decode_socks_udp(frame)
                            allowed = host in {state.host, '127.0.0.1', 'fixture.qa.invalid'} and port in {state.ports['udpEcho'], state.ports['dnsUdp']}
                            if not allowed: raise ValueError('outside fixture target')
                        except (ValueError, UnicodeError, struct.error):
                            state.record('socks5-udp', 'datagram-rejected', node=which, association=association)
                            continue
                        controls = state.read_controls()
                        if controls['proxy' + which + 'Fail']:
                            state.record('socks5-udp', 'controlled-failure', node=which, association=association)
                            continue
                        if total + len(payload) > MAX_UDP_BYTES: break
                        client = sender
                        # Delay only this owned datagram; TCP close/fixture stop
                        # remain observable while latency is injected.
                        delayed_until = time.monotonic() + controls['proxy' + which + 'DelayMs'] / 1000
                        closed = False
                        while time.monotonic() < delayed_until:
                            if state.stopped.is_set() or time.monotonic() >= deadline:
                                closed = True; break
                            if select.select([control], [], [], min(.05, delayed_until - time.monotonic()))[0]:
                                closed = True; break
                        if closed: break
                        upstream.sendto(payload, ('127.0.0.1', port))
                        pending.append(port)
                        total += len(payload); idle = time.monotonic() + 3
                        state.record('socks5-udp', 'forwarded', node=which, association=association, bytes=len(payload), target='echo' if port == state.ports['udpEcho'] else 'dns')
                    if upstream in ready:
                        payload, sender = upstream.recvfrom(MAX_UDP_DATAGRAM_BYTES + 1)
                        if client is None or sender[0] != '127.0.0.1' or sender[1] not in pending or len(payload) > MAX_UDP_DATAGRAM_BYTES:
                            continue
                        if total + len(payload) > MAX_UDP_BYTES: break
                        pending.remove(sender[1])
                        frame = b'\0\0\0\x01' + socket.inet_aton(state.host) + struct.pack('!H', sender[1]) + payload
                        relay.sendto(frame, client)
                        total += len(payload); idle = time.monotonic() + 3
                        state.record('socks5-udp', 'returned', node=which, association=association, bytes=len(payload))
        except (OSError, ValueError):
            state.record('socks5-udp', 'association-error', node=which, association=association)
        finally:
            state.udp_slots.release()
            state.record('socks5-udp', 'closed', node=which, association=association, bytes=total, datagrams=packets)


def decode_socks_udp(frame):
    if len(frame) > MAX_UDP_DATAGRAM_BYTES or len(frame) < 10 or frame[:3] != b'\0\0\0':
        raise ValueError('fragmented or oversized SOCKS datagram')
    if frame[3] == 1:
        host, at = socket.inet_ntoa(frame[4:8]), 8
    elif frame[3] == 3:
        count = frame[4]
        if not 1 <= count <= 253 or len(frame) < 5 + count + 2:
            raise ValueError('invalid SOCKS datagram name')
        host, at = frame[5:5 + count].decode('ascii'), 5 + count
    else:
        raise ValueError('only fixture IPv4/domain datagrams are supported')
    port = struct.unpack('!H', frame[at:at + 2])[0]
    return host, port, frame[at + 2:]


class EchoUDP(socketserver.BaseRequestHandler):
    def handle(self):
        payload, stream = self.request
        state = self.server.state
        match = UDP_CASE.fullmatch(payload) if len(payload) <= 1024 else None
        if match is None or match[1].decode() != state.nonce:
            state.record('udp-echo', 'rejected', bytes=len(payload))
            return
        controls = state.read_controls()
        if state.stopped.wait(controls['udpEchoDelayMs'] / 1000) or controls['udpEchoFail']:
            state.record('udp-echo', 'controlled-failure', bytes=len(payload))
            return
        stream.sendto(payload, self.client_address)
        state.record('udp-echo', 'returned', bytes=len(payload), case=match[2].decode())


class EchoTCP(socketserver.BaseRequestHandler):
    def handle(self):
        state, stream = self.server.state, self.request
        deadline, payload = time.monotonic() + 3, bytearray()
        try:
            while payload.count(b'\n') < 2 and len(payload) <= 1024 and not state.stopped.is_set():
                remaining = deadline - time.monotonic()
                if remaining <= 0:
                    raise TimeoutError('bounded echo request')
                stream.settimeout(min(.2, remaining))
                try:
                    chunk = stream.recv(1025 - len(payload))
                except socket.timeout:
                    continue
                if not chunk:
                    break
                payload.extend(chunk)
            match = TCP_CASE.fullmatch(payload) if len(payload) <= 1024 else None
            if match is None or match[1].decode() != state.nonce:
                state.record('tcp-echo', 'rejected', bytes=len(payload))
                return
            controls = state.read_controls()
            if state.stopped.wait(controls['tcpEchoDelayMs'] / 1000) or controls['tcpEchoFail']:
                state.record('tcp-echo', 'controlled-failure', bytes=len(payload))
                return
            stream.settimeout(1)
            stream.sendall(payload)
            state.record('tcp-echo', 'returned', bytes=len(payload), case=match[2].decode())
        except OSError:
            state.record('tcp-echo', 'closed', bytes=len(payload))


def dns_reply(state, request, transport):
    if not 12 <= len(request) <= 4096:
        return b''
    transaction, flags, questions, _, _, _ = struct.unpack('!6H', request[:12])
    if questions != 1:
        return struct.pack('!6H', transaction, 0x8181, 0, 0, 0, 0)
    cursor, labels = 12, []
    try:
        while True:
            count = request[cursor]
            cursor += 1
            if count == 0:
                break
            if count > 63 or cursor + count > len(request):
                raise ValueError('bad DNS name')
            labels.append(request[cursor:cursor + count].decode('ascii').lower())
            cursor += count
        query_type, query_class = struct.unpack('!HH', request[cursor:cursor + 4])
        question = request[12:cursor + 4]
    except (IndexError, ValueError, UnicodeError, struct.error):
        return struct.pack('!6H', transaction, 0x8181, 0, 0, 0, 0)
    allowed = len(labels) >= 3 and labels[-3:] == [state.nonce, 'qa', 'invalid']
    controls = state.read_controls()
    code = 5 if not allowed else (2 if controls['dnsFail'] else 0)
    answer = b''
    if code == 0 and query_class == 1 and query_type in (1, 28):
        address = '203.0.113.11' if query_type == 1 else '2001:db8::11'
        if controls['revision'] == 'B':
            address = '203.0.113.12' if query_type == 1 else '2001:db8::12'
        data = ipaddress.ip_address(address).packed
        answer = b'\xc0\x0c' + struct.pack('!HHIH', query_type, 1, controls['dnsTtl'], len(data)) + data
    state.record('dns-' + transport, 'answer' if allowed else 'refused',
                 queryType=query_type, rcode=code, ttl=controls['dnsTtl'])
    header = struct.pack('!6H', transaction, 0x8080 | (flags & 0x0100) | code, 1, int(bool(answer)), 0, 0)
    return header + question + answer


class DnsUDP(socketserver.BaseRequestHandler):
    def handle(self):
        request, stream = self.request
        reply = dns_reply(self.server.state, request, 'udp')
        if reply:
            stream.sendto(reply, self.client_address)


class DnsTCP(socketserver.BaseRequestHandler):
    def handle(self):
        self.request.settimeout(3)
        try:
            count = struct.unpack('!H', exact(self.request, 2))[0]
            if count > 4096:
                return
            reply = dns_reply(self.server.state, exact(self.request, count), 'tcp')
            self.request.sendall(struct.pack('!H', len(reply)) + reply)
        except (OSError, EOFError):
            pass


class FixtureServers:
    def __init__(self, bind, advertise_host, nonce, http_port=18746, proxy_a=18747, proxy_b=18748,
                 dns_port=18753, udp_port=18754, tcp_echo_port=18755, internet_dns=()):
        self.state = FixtureState(nonce, advertise_host, internet_dns)
        self.servers, self.threads = [], []
        try:
            for protocol, handler, port in [('http', FixtureHTTP, http_port), ('proxyA', SocksNode, proxy_a),
                                            ('proxyB', SocksNode, proxy_b), ('dnsTcp', DnsTCP, dns_port),
                                            ('tcpEcho', EchoTCP, tcp_echo_port)]:
                server_type = ThreadingHTTPServer if protocol == 'http' else ThreadedTCP
                server = server_type((bind, port), handler)
                server.state = self.state
                server.which = protocol[-1]
                self.state.ports[protocol] = server.server_address[1]
                self.servers.append(server)
            udp = ThreadedUDP((bind, self.state.ports['dnsTcp']), DnsUDP)
            udp.state = self.state
            self.state.ports['dnsUdp'] = udp.server_address[1]
            self.servers.append(udp)
            echo = ThreadedUDP((bind, udp_port), EchoUDP)
            echo.state = self.state
            self.state.ports['udpEcho'] = echo.server_address[1]
            self.servers.append(echo)
        except Exception:
            for server in self.servers:
                server.server_close()
            raise

    def start(self):
        for server in self.servers:
            thread = threading.Thread(target=server.serve_forever, kwargs={'poll_interval': .05}, daemon=True)
            thread.start()
            self.threads.append(thread)
        return self

    def close(self):
        self.state.stopped.set()
        for server in self.servers:
            server.shutdown()
            server.server_close()
        for thread in self.threads:
            thread.join(timeout=2)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bind', default='127.0.0.1')
    parser.add_argument('--advertise-host', required=True, help='fixture host IPv4 reachable by the phone')
    parser.add_argument('--nonce', required=True)
    parser.add_argument('--http-port', type=int, default=18746)
    parser.add_argument('--proxy-a-port', type=int, default=18747)
    parser.add_argument('--proxy-b-port', type=int, default=18748)
    parser.add_argument('--dns-port', type=int, default=18753)
    parser.add_argument('--udp-port', type=int, default=18754)
    parser.add_argument('--tcp-echo-port', type=int, default=18755)
    parser.add_argument('--internet-dns', nargs='+', required=True,
                        help='one or two explicit numeric real resolvers; no fixture/system guess')
    args = parser.parse_args()
    try:
        resolvers = internet_resolvers(args.internet_dns)
    except ValueError as error:
        parser.error(str(error))
    servers = FixtureServers(args.bind, args.advertise_host, args.nonce, args.http_port,
                             args.proxy_a_port, args.proxy_b_port, args.dns_port, args.udp_port,
                             args.tcp_echo_port, resolvers).start()
    stopped = threading.Event()
    for code in (signal.SIGTERM, signal.SIGINT):
        signal.signal(code, lambda *_: stopped.set())
    print(json.dumps({'status': 'ready', 'nonce': args.nonce, 'ports': servers.state.ports,
                      'profileUrl': servers.state.url('/profiles/features.yaml'),
                      'directUrl': servers.state.url('/profiles/direct.yaml'),
                      'controlUrl': servers.state.url('/control'), 'eventsUrl': servers.state.url('/events'),
                      'dnsName': f'probe.{args.nonce}.qa.invalid', 'boundedEvents': MAX_EVENTS,
                      'internetDns': list(resolvers), 'startupHealthResolverPolicyConfigured': True,
                      'startupHealthVerified': False}), flush=True)
    try:
        stopped.wait()
    finally:
        servers.close()


if __name__ == '__main__':
    main()
