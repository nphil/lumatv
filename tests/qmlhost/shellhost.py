#!/usr/bin/env python3
"""Offline host for the shell client (compositor/shell/Shell.qml): LumaTV's QML as the compositor's shell, against a stand-in
of the compositor's UI channel.

    xvfb-run -a -s '-screen 0 1920x1080x24' python3 tests/qmlhost/shellhost.py [--wait MS] [--push MS:topic:JSON]
        [--show MS:kind[:warm]] [--key MS:home|sys:CODE] [--luna MS:URI_SUFFIX:JSON] [--eval MS:EXPR] [--shot-home PNG] [--shot-sys PNG]

The module is staged as the packager stages it (qmlhost.stage), the shell's own import modules stand in for LG's
compositor-only ones as they do on the set, `Eos.Window` and `WebOS.Global` are stubs, and a small HTTP server plays the
compositor: the state and the long poll of events (`--push` sets a topic, `--show` sends a panel request), the UI reports
(`POST /v1/ui`, printed as `[report] <json>`) and the luna calls (`POST /v1/luna`, printed as `[luna] <json>`; `POST /v1/switcher`, the app switcher's requests, as `[switcher] <json>`; `--luna`
answers the open call whose URI ends with the suffix; `--tvpower` stands in for tvpower's screen saver list: it takes each `registerScreenSaverRequest` at once, or
refuses the first `--tvpower-refuse N` of them; `--luna-delay MS` holds every luna call that long before the compositor's side handles it, so a later request can overtake it). `--key` sends a key by evdev code to one of the two windows, as the
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
from channel import Channel, emit, serve  # noqa: E402
from PyQt5 import QtCore, QtGui, QtQml  # noqa: E402

IGNORE = re.compile(r'\[host\]|Implicitly defined onFoo|QSslSocket|OpenSSL|place lookup failed|no stories; retry|load glyph failed|XMLHttpRequest: Using GET')


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
    ap.add_argument('--tvpower', action='store_true', help="answer the screen saver list's subscriptions as tvpower does")
    ap.add_argument('--tvpower-refuse', type=int, default=0, metavar='N', help='refuse the first N of them (returnValue false)')
    ap.add_argument('--luna-delay', type=int, default=0, metavar='MS', help='hold each luna call this long before it is handled')
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
    channel = Channel(token, a.tvpower, a.tvpower_refuse, a.luna_delay)
    srv = serve(channel, port)

    messages = []

    def handler(mode, ctx, msg):
        messages.append((int(mode), ctx.file or '', ctx.line, msg))
        emit(f'[{["debug", "warning", "critical", "fatal", "info"][int(mode)]}] {os.path.basename(ctx.file or "")}:{ctx.line}: {msg}')
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
        # The compositor gives the keyboard focus to one window; the offscreen platform gives it to the last window made visible, which is
        # the system layer once its window is kept mapped (2 s after Home loaded), so the window the key is for is made active first.
        win.requestActivate()
        app.processEvents()
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
        emit(f'[eval] {v}')

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
