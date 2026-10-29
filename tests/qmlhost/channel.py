#!/usr/bin/env python3
"""The compositor's side of the UI channel, as shell.rs serves it, as a stand-in for the shell clients' offline hosts: shellhost.py
(the QML client) and tests/test_rust_shell.py (the Rust client, `lumatv-ui --host`). The state and the long poll of events
(`push` sets a topic), the UI reports (`POST /v1/ui`, printed as `[report] <json>`), the luna calls (`POST /v1/luna`, `[luna]`),
cancels (`[cancel]`), the keyboard (`[ime]`) and the switcher (`[switcher]`); `--tvpower` answers the screen saver list's
subscriptions as tvpower does, `--luna-delay` holds each luna call before it is handled.
"""
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class Channel:
    """The compositor's side of the UI channel, as shell.rs serves it."""

    def __init__(self, token, tvpower=False, refuse=0, luna_delay=0):
        self.token = token
        self.tvpower = tvpower
        self.refuse = refuse
        self.luna_delay = luna_delay / 1000.0
        self.lock = threading.Condition()
        self.seq = 0
        self.boot = int(time.time() * 1000)
        self.events = []
        self.topics = {}
        self.reports = []
        self.luna = []
        self.cancels = []
        self.ime = []
        self.switcher = []
        self.polls = 0
        self.refused = 0

    def push(self, topic, data):
        with self.lock:
            self.seq += 1
            self.events.append((self.seq, topic, data))
            self.topics[topic] = data
            self.lock.notify_all()

    def since(self, after, timeout):
        with self.lock:
            deadline = time.time() + timeout
            while not [e for e in self.events if e[0] > after]:
                left = deadline - time.time()
                if left <= 0:
                    break
                self.lock.wait(left)
            return self.seq, [{'topic': t, 'data': d} for s, t, d in self.events if s > after]


def serve(channel, port, say=None):
    """Serves `channel` on `port`; `say(line)` prints what the client sent (one line at a time, whole), the console by default."""
    say_lock = threading.Lock()
    out = say or (lambda line: print(line, flush=True))

    def say(line):
        # The server answers on threads of its own: one line at a time, whole.
        with say_lock:
            out(line)

    class Handler(BaseHTTPRequestHandler):
        protocol_version = 'HTTP/1.1'

        def log_message(self, *args):
            pass

        def answer(self, status, doc):
            body = json.dumps(doc).encode()
            self.send_response(status)
            self.send_header('Content-Type', 'application/json')
            self.send_header('Content-Length', str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def body(self):
            n = int(self.headers.get('Content-Length') or 0)
            return json.loads(self.rfile.read(n) or b'{}') if n else {}

        def do_GET(self):
            if self.headers.get('X-Luma-Token') != channel.token:
                return self.answer(403, {'ok': False, 'error': 'token'})
            path, _, query = self.path.partition('?')
            q = dict(p.split('=', 1) if '=' in p else (p, '') for p in query.split('&') if p)
            if path == '/v1/state':
                with channel.lock:
                    return self.answer(200, {'ok': True, 'boot': channel.boot, 'seq': channel.seq, 'topics': channel.topics})
            if path == '/v1/events':
                channel.polls += 1
                seq, events = channel.since(int(q.get('after', 0)), min(int(q.get('timeout', 25)), 60))
                return self.answer(200, {'ok': True, 'boot': channel.boot, 'seq': seq, 'events': events})
            self.answer(404, {'ok': False})

        def do_POST(self):
            if self.headers.get('X-Luma-Token') != channel.token:
                return self.answer(403, {'ok': False, 'error': 'token'})
            doc = self.body()
            if self.path == '/v1/ui':
                channel.reports.append(doc)
                say('[report] ' + json.dumps(doc))
                return self.answer(200, {'ok': True})
            if self.path == '/v1/luna':
                if channel.luna_delay:
                    time.sleep(channel.luna_delay)
                channel.luna.append(doc)
                say('[luna] ' + json.dumps(doc))
                if channel.tvpower and doc.get('uri', '').endswith('/registerScreenSaverRequest'):
                    with channel.lock:
                        refused = channel.refused < channel.refuse
                        channel.refused += refused
                    channel.push('luna', {'tag': doc['tag'], 'payload': {'returnValue': False, 'errorText': 'refused by the stand-in'} if refused else {'returnValue': True, 'subscribed': True}})
                return self.answer(200, {'ok': True, 'tag': doc.get('tag')})
            if self.path == '/v1/cancel':
                channel.cancels.append(doc.get('tag'))
                say('[cancel] ' + json.dumps(doc))
                return self.answer(200, {'ok': True})
            if self.path == '/v1/ime':
                channel.ime.append(doc)
                say('[ime] ' + json.dumps(doc))
                return self.answer(200, {'ok': True})
            if self.path == '/v1/switcher':
                channel.switcher.append(doc)
                say('[switcher] ' + json.dumps(doc))
                return self.answer(200, {'ok': True})
            self.answer(404, {'ok': False})

    srv = ThreadingHTTPServer(('127.0.0.1', port), Handler)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv
