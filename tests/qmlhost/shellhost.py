#!/usr/bin/env python3
"""Offline host for the shell client (compositor/shell/Shell.qml): LumaTV's QML as the compositor's shell, against a stand-in
of the compositor's UI channel.

    xvfb-run -a -s '-screen 0 1920x1080x24' python3 tests/qmlhost/shellhost.py [--wait MS] [--push MS:topic:JSON]
        [--show MS:kind[:warm]] [--key MS:home|sys:CODE] [--luna MS:URI_SUFFIX:JSON] [--eval MS:EXPR] [--shot-home PNG] [--shot-sys PNG]

The module is staged as the packager stages it (qmlhost.stage), the shell's own import modules stand in for LG's
compositor-only ones as they do on the set, `Eos.Window` and `WebOS.Global` are stubs, and a small HTTP server plays the
compositor: the state and the long poll of events (`--push` sets a topic, `--show` sends a panel request), the UI reports
(`POST /v1/ui`, printed as `[report] <json>`) and the luna calls (`POST /v1/luna`, printed as `[luna] <json>`; `--luna`
answers the open call whose URI ends with the suffix). `--key` sends a key by evdev code to one of the two windows, as the
compositor delivers it on wl_keyboard (press and release). `--eval` runs with `home` (the Home window), `sys` and `Link`.

Qt 5.15 from the PyQt5 wheel under Xvfb: it finds load errors and wrong wiring and shows what the windows draw; the set is
the only timing. Exit status 0 when nothing of the client's own warned, 1 otherwise.
"""
import argparse
import json
import os
import re
import shutil
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
SHELL = os.path.join(ROOT, 'compositor', 'shell')

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)
os.environ.setdefault('QML_XHR_ALLOW_FILE_READ', '1')
os.environ.setdefault('QML_XHR_ALLOW_FILE_WRITE', '1')

sys.path.insert(0, HERE)
import qmlhost  # noqa: E402
from PyQt5 import QtCore, QtGui, QtQml  # noqa: E402

IGNORE = re.compile(r'\[host\]|Implicitly defined onFoo|QSslSocket|OpenSSL|place lookup failed|no stories; retry|load glyph failed|XMLHttpRequest: Using GET')


class Channel:
    """The compositor's side of the UI channel, as shell.rs serves it."""

    def __init__(self, token):
        self.token = token
        self.lock = threading.Condition()
        self.seq = 0
        self.boot = int(time.time() * 1000)
        self.events = []
        self.topics = {}
        self.reports = []
        self.luna = []
        self.cancels = []
        self.ime = []
        self.polls = 0

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


def serve(channel, port):
    say_lock = threading.Lock()

    def say(line):
        # The server answers on threads of its own: one line at a time, whole.
        with say_lock:
            print(line, flush=True)

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
                channel.luna.append(doc)
                say('[luna] ' + json.dumps(doc))
                return self.answer(200, {'ok': True, 'tag': doc.get('tag')})
            if self.path == '/v1/cancel':
                channel.cancels.append(doc.get('tag'))
                say('[cancel] ' + json.dumps(doc))
                return self.answer(200, {'ok': True})
            if self.path == '/v1/ime':
                channel.ime.append(doc)
                say('[ime] ' + json.dumps(doc))
                return self.answer(200, {'ok': True})
            self.answer(404, {'ok': False})

    srv = ThreadingHTTPServer(('127.0.0.1', port), Handler)
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--wait', type=int, default=6000)
    ap.add_argument('--port', type=int, default=0, help='the channel port (default: a free one)')
    ap.add_argument('--push', action='append', default=[], metavar='MS:topic:JSON')
    ap.add_argument('--show', action='append', default=[], metavar='MS:kind[:warm]')
    ap.add_argument('--key', action='append', default=[], metavar='MS:home|sys:CODE', help='an evdev code to a window, press then release')
    ap.add_argument('--luna', action='append', default=[], metavar='MS:SUFFIX:JSON', help='answer every open luna call whose URI ends with SUFFIX')
    ap.add_argument('--eval', action='append', default=[], metavar='MS:EXPR')
    ap.add_argument('--shot-home')
    ap.add_argument('--shot-sys')
    ap.add_argument('--snap-sys', action='append', default=[], metavar='MS:PATH', help='grab the system layer at MS')
    ap.add_argument('--audit-sys', action='append', default=[], metavar='MS:PATH[:KINDS]', help='write the layout audit (layoutaudit.py) of the system layer at MS')
    ap.add_argument('--backdrop', help='an image drawn behind everything in the system layer: a still of video, for a render of what is over an app')
    ap.add_argument('--frames-sys', action='append', default=[], metavar='MS:LABEL', help='print how many frames the system layer has swapped since the start, at MS, as `[frames] LABEL N`')
    ap.add_argument('--data')
    ap.add_argument('--theme', help='JSON written as the user theme.json')
    ap.add_argument('--fake-helper', action='store_true')
    a = ap.parse_args(argv)

    data = os.path.abspath(a.data) if a.data else tempfile.mkdtemp(prefix='shellhost-')
    if a.data and os.path.exists(data):
        shutil.rmtree(data)
    os.makedirs(data, exist_ok=True)
    mod = qmlhost.stage(data, os.path.join(ROOT, 'payload', 'compositor', 'lumatv'))
    if a.theme:
        with open(os.path.join(data, 'theme', 'theme.json'), 'w') as f:
            f.write(a.theme)
    token = 'offline-token'
    token_path = os.path.join(data, 'lumatv-shell.token')
    with open(token_path, 'w') as f:
        f.write(token + '\n')
    import socket
    port = a.port
    if not port:
        with socket.socket() as s:
            s.bind(('127.0.0.1', 0))
            port = s.getsockname()[1]
    channel = Channel(token)
    srv = serve(channel, port)

    messages = []

    def handler(mode, ctx, msg):
        messages.append((int(mode), ctx.file or '', ctx.line, msg))
        print(f'[{["debug", "warning", "critical", "fatal", "info"][int(mode)]}] {os.path.basename(ctx.file or "")}:{ctx.line}: {msg}', flush=True)
    QtCore.qInstallMessageHandler(handler)

    # qml-runner's first argument, as the compositor builds it (shell.rs).
    launch = json.dumps({'event': 'launch', 'appId': 'org.nphil.lumatv.shell', 'main': os.path.join(SHELL, 'Shell.qml'),
                         'params': {'port': port, 'tokenFile': token_path, 'dataDir': data, 'moduleDir': mod, 'runtimeDir': data, 'socket': 'offline'}})
    app = QtGui.QGuiApplication([sys.argv[0], launch])
    fmt = QtGui.QSurfaceFormat()
    fmt.setRenderableType(QtGui.QSurfaceFormat.OpenGL)
    fmt.setVersion(2, 1)
    QtGui.QSurfaceFormat.setDefaultFormat(fmt)
    for n in sorted(os.listdir(os.path.join(data, 'fonts'))):
        if n.endswith('.ttf'):
            QtGui.QFontDatabase.addApplicationFont(os.path.join(data, 'fonts', n))
    QtQml.qmlRegisterType(qmlhost.TextFile, 'TextFile', 1, 0, 'TextFile')
    helper_state = None
    # The helper's port is fixed (kit/Helper.qml) and shared with every other host and test on the machine. A host that must find no helper
    # takes the lock shared and waits until nothing answers on the port (another run's stand-in would hand it weather and camera pop-ups of its
    # own, and the system layer would be live for them); a host that serves the stand-in takes it exclusively and retries the bind while the
    # host before it is going away (qmlhost.py does the same).
    import errno
    import fcntl
    _lock = open(os.path.join(tempfile.gettempdir(), 'lumatv-qmlhost.lock'), 'w')
    if a.fake_helper:
        import fakehelper
        fcntl.flock(_lock, fcntl.LOCK_EX)
        for attempt in range(200):
            try:
                _srv, helper_state = fakehelper.serve(18731)
                break
            except OSError as e:
                if e.errno != errno.EADDRINUSE or attempt == 199:
                    raise
                time.sleep(0.05)
    else:
        fcntl.flock(_lock, fcntl.LOCK_SH)
        for _ in range(100):
            with socket.socket() as probe:
                if probe.connect_ex(('127.0.0.1', 18731)) != 0:
                    break
            time.sleep(0.05)

    eng = QtQml.QQmlApplicationEngine()
    # The shell's own modules shadow the host's stubs (addImportPath puts the newest path first), as they shadow LG's
    # compositor-only ones on the set.
    eng.addImportPath(os.path.join(HERE, 'stubs'))
    eng.addImportPath(os.path.join(SHELL, 'imports'))
    QtGui.QCursor.setPos(0, 0)
    eng.load(QtCore.QUrl.fromLocalFile(os.path.join(SHELL, 'Shell.qml')))
    roots = eng.rootObjects()
    if not roots:
        print('ERROR the shell did not load')
        return 1
    home = roots[0]
    sysw = home.findChild(QtGui.QWindow, 'sysWin') or next((c for c in home.children() if isinstance(c, QtGui.QWindow)), None)
    ctx = eng.rootContext()
    ctx.setContextProperty('home', home)
    ctx.setContextProperty('sys', sysw)

    def push(topic, doc):
        channel.push(topic, doc)

    def show(kind, warm):
        channel.push('show', {'kind': kind, 'warm': warm})

    def key(target, code):
        win = home if target == 'home' else sysw
        qt = {103: QtCore.Qt.Key_Up, 108: QtCore.Qt.Key_Down, 105: QtCore.Qt.Key_Left, 106: QtCore.Qt.Key_Right, 28: QtCore.Qt.Key_Return}.get(code, QtCore.Qt.Key_unknown)
        for typ in (QtCore.QEvent.KeyPress, QtCore.QEvent.KeyRelease):
            app.sendEvent(win, QtGui.QKeyEvent(typ, qt, QtCore.Qt.NoModifier, code + 8, 0, 0))

    def luna(suffix, doc):
        for call in channel.luna:
            if call['uri'].endswith(suffix):
                channel.push('luna', {'tag': call['tag'], 'payload': doc})

    def evaluate(expr):
        e = QtQml.QQmlExpression(ctx, home, expr)
        v = e.evaluate()
        print(f'[eval] {v}', flush=True)

    def snap_sys(path):
        sysw.grabWindow().save(path)
        print(f'shot {path}', flush=True)

    def audit_sys(path, kinds):
        import layoutaudit
        found = layoutaudit.audit(sysw.contentItem(), 1920, 1080, sysw.grabWindow(), kinds)
        layoutaudit.write(path, found)
        print(f'audit {path} {len(found)} finding(s)', flush=True)

    def finish():
        if a.shot_home:
            home.grabWindow().save(a.shot_home)
            print(f'shot {a.shot_home}', flush=True)
        if a.shot_sys and sysw is not None:
            sysw.grabWindow().save(a.shot_sys)
            print(f'shot {a.shot_sys}', flush=True)
        app.quit()

    for spec in a.push:
        ms, topic, doc = spec.split(':', 2)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda t=topic, d=json.loads(doc): push(t, d))
    for spec in a.show:
        ms, kind, *rest = spec.split(':')
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda k=kind, w=bool(rest): show(k, w))
    for spec in a.key:
        ms, target, code = spec.split(':')
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda t=target, c=int(code): key(t, c))
    for spec in a.luna:
        ms, suffix, doc = spec.split(':', 2)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda s=suffix, d=json.loads(doc): luna(s, d))
    for spec in a.eval:
        ms, expr = spec.split(':', 1)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda x=expr: evaluate(x))
    for spec in a.snap_sys:
        ms, path = spec.split(':', 1)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda p=path: snap_sys(p))
    for spec in a.audit_sys:
        ms, path, *rest = spec.split(':')
        kinds = rest[0].split(',') if rest else None
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda p=path, k=kinds: audit_sys(p, k))
    if a.backdrop:
        src = 'import QtQuick 2.4\nImage { anchors.fill: parent; source: "file://%s"; fillMode: Image.PreserveAspectCrop; z: -1000 }' % os.path.abspath(a.backdrop)
        QtCore.QTimer.singleShot(300, QtCore.Qt.PreciseTimer, lambda: evaluate('Qt.createQmlObject(%s, sys.contentItem, "backdrop") !== null' % json.dumps(src)))
    frames = {'n': 0}
    if sysw is not None:
        sysw.frameSwapped.connect(lambda: frames.__setitem__('n', frames['n'] + 1))
    for spec in a.frames_sys:
        ms, label = spec.split(':', 1)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda l=label: print(f'[frames] {l} {frames["n"]}', flush=True))
    QtCore.QTimer.singleShot(a.wait, QtCore.Qt.PreciseTimer, finish)
    app.exec_()
    srv.shutdown()
    own = [m for m in messages if m[0] in (1, 2, 3) and not IGNORE.search(m[3])]
    print(f'{len(own)} warning(s) from the shell', flush=True)
    if not a.data:
        shutil.rmtree(data, ignore_errors=True)
    return 1 if own else 0


if __name__ == '__main__':
    sys.exit(main())
