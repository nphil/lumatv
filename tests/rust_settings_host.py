#!/usr/bin/env python3
"""Runs the Rust shell's Settings (`lumatv-ui --host --kinds settings`, rustui/ui) against the compositor's stand-in channel
(`qmlhost/channel.py`) with LG's settings service stood in (`qmlhost/lgsettings.py`) and a small fake of the helper's API: what the
tests of Settings (`test_rust_settings.py`) and the renders (`render_rust_settings.py`) share."""
import json
import os
import re
import socket
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, os.path.join(HERE, 'qmlhost'))
import channel as ch  # noqa: E402
import lgsettings  # noqa: E402

BIN = os.path.join(ROOT, 'rustui', 'target', 'debug', 'lumatv-ui')
MESA = '/usr/lib/x86_64-linux-gnu/libEGL.so.1'


def available():
    return os.path.exists(BIN) and os.path.exists(MESA)


class FakeHelper:
    """The helper's API as far as Settings reads it: `routes` maps "METHOD /path" to a payload (or a function of the request body); every
    request is kept in `requests`. A long poll answers at once with no events, so the shell's helper feed is up."""

    def __init__(self, routes=None):
        self.routes = dict(routes or {})
        self.requests = []
        self.srv = None

    def start(self):
        helper = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *a):
                pass

            def send(self, code, doc):
                body = json.dumps(doc).encode()
                self.send_response(code)
                self.send_header('Content-Type', 'application/json')
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def serve(self, method):
                path = self.path.split('?')[0]
                n = int(self.headers.get('Content-Length') or 0)
                body = json.loads(self.rfile.read(n) or b'{}') if n else {}
                helper.requests.append((method, path, body))
                if path == '/v1/state':
                    return self.send(200, {'ok': True, 'boot': 1, 'seq': 0, 'topics': helper.routes.get('topics', {})})
                if path == '/v1/events':
                    time.sleep(0.4)
                    return self.send(200, {'ok': True, 'boot': 1, 'seq': 0, 'events': []})
                answer = helper.routes.get(f'{method} {path}')
                if callable(answer):
                    answer = answer(body)
                if answer is None:
                    return self.send(404, {'ok': False, 'error': {'code': 'not_found', 'message': 'no such route in the fake'}})
                if isinstance(answer, (bytes, bytearray)):
                    # a picture: the helper's stills and previews come back as image bytes, not JSON
                    self.send_response(200)
                    self.send_header('Content-Type', 'image/jpeg')
                    self.send_header('Content-Length', str(len(answer)))
                    self.end_headers()
                    self.wfile.write(bytes(answer))
                    return
                status = answer.pop('_status', 200) if isinstance(answer, dict) else 200
                self.send(status, answer)

            def do_GET(self):
                self.serve('GET')

            def do_POST(self):
                self.serve('POST')

            def do_DELETE(self):
                self.serve('DELETE')

        with socket.socket() as s:
            s.bind(('127.0.0.1', 0))
            self.port = s.getsockname()[1]
        self.srv = ThreadingHTTPServer(('127.0.0.1', self.port), Handler)
        self.srv.daemon_threads = True
        threading.Thread(target=self.srv.serve_forever, daemon=True).start()
        return self

    def stop(self):
        if self.srv:
            self.srv.shutdown()

    def posted(self, path):
        return [b for m, p, b in self.requests if m == 'POST' and p == path]


def run(steps=(), wait=4000, extra=(), lg=None, helper=None, kinds='settings', text_scale=1.0, mode='dark', theme=None, timeout=240):
    """Runs the client for `wait` ms. `steps` are (ms, fn(channel)) done on the channel's side. Returns (rc, output, channel, lg, helper)."""
    token = 'offline-token'
    data = tempfile.mkdtemp(prefix='rustsettings-')
    token_path = os.path.join(data, 'lumatv-ui.token')
    with open(token_path, 'w') as f:
        f.write(token + '\n')
    lg = lg or lgsettings.LgService()
    helper = helper or FakeHelper()
    helper.start()
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        port = s.getsockname()[1]
    chan = ch.Channel(token, responder=lg.respond)
    srv = ch.serve(chan, port, say=lambda line: None)

    def run_steps():
        t0 = time.time()
        for ms, fn in sorted(steps, key=lambda st: st[0]):
            time.sleep(max(0.0, t0 + ms / 1000.0 - time.time()))
            fn(chan)
    threading.Thread(target=run_steps, daemon=True).start()
    cmd = [BIN, '--host', '--kinds', kinds, '--port', str(port), '--token-file', token_path, '--helper-port', str(helper.port),
           '--wait', str(wait), '--data-dir', data, '--shm', os.path.join(data, 'shm'), '--text-scale', str(text_scale), '--mode', mode,
           '--module-dir', os.path.join(ROOT, 'compositor', 'assets', 'ui'), *extra]
    if theme is not None:
        cmd += ['--theme', json.dumps(theme)]
    env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1')
    r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=timeout)
    srv.shutdown()
    helper.stop()
    return r.returncode, r.stdout + r.stderr, chan, lg, helper


def evals(out):
    return [json.loads(m.group(1)) for m in re.finditer(r"^\[eval\] (.*)$", out, re.M)]


def reports(out):
    return [json.loads(m.group(1)) for m in re.finditer(r"^\[report\] (.*)$", out, re.M)]
