#!/usr/bin/env python3
"""Offline QML host: loads a compositor surface on the workstation and reports what it logs.

    xvfb-run -a -s '-screen 0 1920x1080x24' python3 tests/qmlhost/qmlhost.py LumaHome.qml \
        [--shot out.png] [--wait 3000] [--prop hostActive=true] [--press KEY[,KEY...]] [--data DIR]

Qt 5.15 from the PyQt5 wheel, Mesa's software GL under Xvfb. It finds load errors, binding loops,
unknown properties and wrong imports, and it renders a screenshot to look at. It does NOT measure
anything: software GL says nothing about the set's 60 fps rule, and Qt 5.15 differs from the set's
5.12 (no `Qt.color`, for one, which the host does not hide). The gate on the set is the only timing.

The surface is staged first, as `tools/build-ipk.sh` and `tools/lumatv` stage it: the brand
placeholders are filled, `theme.json` and `layout.json` go beside the QML as `theme.default.json` and
`layout.default.json`, and a data directory holds the theme, fonts and wallpaper. The compositor-only
modules (`WebOSCompositorBase`, `WebOSServices`, `WebOS.Global`, `TextFile`) are stand-ins: luna
calls are recorded in `LS.calls`, `TextFile` reads and writes real files. `LS` is exported by
`WebOSCompositorBase` only, as on the set, so a file that uses it without that import warns.

Exit status 0 when the surface loaded and printed no warning from its own files, 1 otherwise.
"""
import argparse
import errno
import json
import os
import re
import shutil
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)
os.environ.setdefault('QML_XHR_ALLOW_FILE_READ', '1')
# Qt 5.12 on the set writes local files from XMLHttpRequest (Home saves theme.json that way); 5.15 asks.
os.environ.setdefault('QML_XHR_ALLOW_FILE_WRITE', '1')

from PyQt5 import QtCore, QtGui, QtQml, QtQuick  # noqa: E402
from channel import emit  # noqa: E402


class TextFile(QtCore.QObject):
    """The compositor's TextFile: a file URL or path, read and write of the whole text."""
    nameChanged = QtCore.pyqtSignal()

    def __init__(self, parent=None):
        super().__init__(parent)
        self._name = ''

    def _path(self):
        n = self._name
        return n[len('file://'):] if n.startswith('file://') else n

    @QtCore.pyqtProperty(str, notify=nameChanged)
    def name(self):
        return self._name

    @name.setter
    def name(self, v):
        if v != self._name:
            self._name = v
            self.nameChanged.emit()

    @QtCore.pyqtProperty(bool, notify=nameChanged)
    def exists(self):
        return os.path.isfile(self._path())

    @QtCore.pyqtSlot(result=str)
    def read(self):
        try:
            with open(self._path(), encoding='utf-8') as f:
                return f.read()
        except OSError:
            return ''

    @QtCore.pyqtSlot(str, result=bool)
    def write(self, text):
        try:
            os.makedirs(os.path.dirname(self._path()), exist_ok=True)
            with open(self._path(), 'w', encoding='utf-8') as f:
                f.write(text)
            return True
        except OSError:
            return False


def brand():
    with open(os.path.join(ROOT, 'brand.json')) as f:
        return json.load(f)


def stage(data_dir, module_src, extra_src=None):
    """The surface tree as it is on the set. Returns the staged module directory."""
    b = brand()
    mod = os.path.join(data_dir, 'qml', 'WebOSCompositor', b['qmlModule'])
    os.makedirs(os.path.dirname(mod), exist_ok=True)
    shutil.copytree(module_src, mod)
    for name in ('theme.json', 'layout.json'):
        shutil.copy(os.path.join(ROOT, 'payload', 'theme', name), os.path.join(mod, name.replace('.json', '.default.json')))
    with open(os.path.join(mod, 'settings.json'), 'w') as f:
        f.write('{"fps":false}\n')
    theme = os.path.join(data_dir, 'theme')
    os.makedirs(theme, exist_ok=True)
    for name in ('theme.json', 'layout.json'):
        with open(os.path.join(theme, name), 'w') as f:
            f.write('{}\n')
    wall = os.path.join(ROOT, 'payload', 'wallpaper')
    if os.path.isdir(wall):
        for n in os.listdir(wall):
            if n.endswith('.png'):
                shutil.copy(os.path.join(wall, n), os.path.join(theme, 'wall_1080.png'))
                break
    shutil.copytree(os.path.join(ROOT, 'payload', 'fonts'), os.path.join(data_dir, 'fonts'))
    sysui = os.path.join(ROOT, 'payload', 'systemui')
    for name in ('MasterBrand', 'BootLogo'):
        if os.path.isdir(os.path.join(sysui, name)):
            shutil.copytree(os.path.join(sysui, name), os.path.join(data_dir, 'systemui', name))
    os.makedirs(os.path.join(data_dir, 'cache'), exist_ok=True)
    # The compositor's files in /tmp (`/tmp/lumatv-overlay`, `-beat`, `-wallsample.xpm`) are shared by every host running on the machine: each
    # gets a directory of its own for them (QMLHOST_TMP when a test needs to write one itself).
    door_dir = os.environ.get('QMLHOST_TMP') or os.path.join(data_dir, 'tmp')
    os.makedirs(door_dir, exist_ok=True)
    subst = {'displayName': b['displayName'], 'shortId': b['shortId'], 'appId': b['appId'],
             'dataDir': data_dir, 'qmlModule': b['qmlModule']}
    for dirpath, _, files in list(os.walk(mod)) + list(os.walk(os.path.join(data_dir, 'systemui'))):
        for n in files:
            if n.endswith(('.qml', '.js', '.json')):
                p = os.path.join(dirpath, n)
                with open(p, encoding='utf-8') as f:
                    s = f.read()
                s2 = re.sub(r'\{\{(\w+)\}\}', lambda m: subst.get(m.group(1), m.group(0)), s)
                s2 = s2.replace('/tmp/' + b['shortId'] + '-', door_dir + '/' + b['shortId'] + '-')
                if s2 != s:
                    with open(p, 'w', encoding='utf-8') as f:
                        f.write(s2)
    left = []
    for dirpath, _, files in list(os.walk(mod)) + list(os.walk(os.path.join(data_dir, 'systemui'))):
        for n in files:
            if n.endswith(('.qml', '.js', '.json')):
                with open(os.path.join(dirpath, n), encoding='utf-8') as f:
                    if re.search(r'\{\{\w+\}\}', f.read()):
                        left.append(n)
    if left:
        raise SystemExit('unresolved placeholders in ' + ', '.join(left))
    return mod


# Qt 5.15 and workstation noise that says nothing about the surface: the Connections syntax Qt 5.12
# requires is deprecated in 5.15, the feeds cannot reach the network (and the TLS stack is the wheel's),
# and glyphs a font does not have.
IGNORE = re.compile(r'\[host\]|Implicitly defined onFoo|QSslSocket|OpenSSL|place lookup failed|no stories; retry|'
                    r'load glyph failed|XMLHttpRequest: Using GET')

KEYS = {'LEFT': QtCore.Qt.Key_Left, 'RIGHT': QtCore.Qt.Key_Right, 'UP': QtCore.Qt.Key_Up, 'DOWN': QtCore.Qt.Key_Down,
        'OK': QtCore.Qt.Key_Return, 'BACK': QtCore.Qt.Key_Escape, 'MENU': QtCore.Qt.Key_Menu}


def coerce(v):
    if v in ('true', 'false'):
        return v == 'true'
    try:
        return int(v)
    except ValueError:
        try:
            return float(v)
        except ValueError:
            return v


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('qml', help='file in payload/compositor/lumatv (or a path below it, e.g. logo/LogoOn.qml) to load')
    ap.add_argument('--shot', help='write a PNG of the window here')
    ap.add_argument('--snap', action='append', default=[], metavar='MS:PATH', help='also write a PNG of the window MS after start (repeatable)')
    ap.add_argument('--audit', action='append', default=[], metavar='MS:PATH[:KINDS]',
                    help='write the layout audit (tests/qmlhost/layoutaudit.py) of the whole window as JSON at MS after start (repeatable); '
                         'KINDS limits it to a comma-separated list of finding kinds (the 200 %% text pass asks for overlap)')
    ap.add_argument('--wait', type=int, default=3000, help='ms to run before the screenshot')
    ap.add_argument('--prop', action='append', default=[], help='root property to set, name=value')
    ap.add_argument('--press', help='comma-separated keys sent after --wait/2 (LEFT RIGHT UP DOWN OK BACK MENU)')
    ap.add_argument('--data', help='keep the staged data dir here (default: a temp dir)')
    ap.add_argument('--size', default='1920x1080')
    ap.add_argument('--publish', action='append', default=[], metavar='MS:topic:JSON',
                    help='with --fake-helper: set a topic at MS after start')
    ap.add_argument('--eval', action='append', default=[], metavar='MS:EXPR',
                    help='evaluate a JavaScript expression at MS after start with `root` the surface; prints "[eval] value"')
    ap.add_argument('--pointer', action='append', default=[], metavar='MS:move|click:EXPR',
                    help='at MS after start, a pointer move or click (press and release) at the window position [x, y] that the JavaScript expression returns')
    ap.add_argument('--frames', action='append', default=[], metavar='MS:LABEL',
                    help='print "[frames] LABEL N", the frames the window has swapped so far, at MS after start (repeatable): the frame-driven motions\' count')
    ap.add_argument('--fake-helper', action='store_true', help='serve a stand-in for the native helper on 127.0.0.1:18731 while running')
    ap.add_argument('--theme', help='JSON written as the user theme.json in the data dir (what the owner changed)')
    ap.add_argument('--layout', help='JSON written as the user layout.json in the data dir (the cards the owner arranged)')
    ap.add_argument('--cache', action='append', default=[], metavar='NAME=JSON',
                    help='write the data dir\'s cache/NAME.json (the helper\'s saved weather.json or news.json) before the surface loads')
    ap.add_argument('--remove', action='append', default=[], metavar='PATH',
                    help='delete this path (relative to the staged module dir) after staging, to test a surface without it')
    ap.add_argument('--drop-import', action='append', default=[], metavar='PATH:MODULE',
                    help='delete the `import MODULE x.y` line from this staged file (relative to the staged module dir), to see what a file '
                         'that lacks the import does on the set')
    ap.add_argument('--fire', action='append', default=[], metavar='MS:launch|close[:JSON]',
                    help='call the window life cycle of a system-UI surface at MS after start, e.g. 100:launch:{"reason":"x"}')
    ap.add_argument('--overlay', action='store_true',
                    help='load LumaOverlay.qml beside the surface, as the fullscreen container does: the surface is hidden while its `hostActive` is false, '
                         'and `overlay` is the overlay\'s name in --eval (set `overlay.appFront = true` for an app in front)')
    ap.add_argument('--bgimage', metavar='PNG', help='a picture stretched behind everything: what an app (video) would show')
    ap.add_argument('--wallpaper', metavar='PNG', help='the picture staged as the theme directory\'s wall_1080.png (the host otherwise stages the first PNG of payload/wallpaper, '
                                                     'which is not always the shipped wallpaper)')
    a = ap.parse_args(argv)

    data = os.path.abspath(a.data) if a.data else tempfile.mkdtemp(prefix='qmlhost-')
    if a.data and os.path.exists(data):
        shutil.rmtree(data)
    os.makedirs(data, exist_ok=True)
    src = os.path.join(ROOT, 'payload', 'compositor', 'lumatv')
    mod = stage(data, src)
    if a.wallpaper:
        shutil.copy(a.wallpaper, os.path.join(data, 'theme', 'wall_1080.png'))
    for spec in a.cache:
        name, _, text = spec.partition('=')
        with open(os.path.join(data, 'cache', name + '.json'), 'w') as f:
            f.write(text)
    if a.theme:
        with open(os.path.join(data, 'theme', 'theme.json'), 'w') as f:
            f.write(a.theme)
    if a.layout:
        with open(os.path.join(data, 'theme', 'layout.json'), 'w') as f:
            f.write(a.layout)
    for rel in a.remove:
        target = os.path.join(mod, rel)
        shutil.rmtree(target) if os.path.isdir(target) else os.remove(target)
    for spec in a.drop_import:
        rel, _, module = spec.rpartition(':')
        target = os.path.join(mod, rel)
        with open(target, encoding='utf-8') as f:
            text = f.read()
        kept = re.sub(r'^import %s [\d.]+\n' % re.escape(module), '', text, flags=re.M)
        if kept == text:
            raise SystemExit('%s has no import of %s' % (rel, module))
        with open(target, 'w', encoding='utf-8') as f:
            f.write(kept)

    messages = []

    def handler(mode, ctx, msg):
        messages.append((int(mode), ctx.file or '', ctx.line, msg))
        emit(f'[{["debug", "warning", "critical", "fatal", "info"][int(mode)]}] {os.path.basename(ctx.file or "")}:{ctx.line}: {msg}')
    QtCore.qInstallMessageHandler(handler)

    app = QtGui.QGuiApplication(sys.argv[:1])
    fmt = QtGui.QSurfaceFormat()
    fmt.setRenderableType(QtGui.QSurfaceFormat.OpenGL)
    fmt.setVersion(2, 1)
    QtGui.QSurfaceFormat.setDefaultFormat(fmt)

    # The surface loads Manrope through FontLoaders created after its first Text; Qt 5.15 (unlike the
    # set's 5.12) keeps glyph runs of the fallback face for those and draws garbage, so the host
    # registers the faces first.
    fonts_dir = os.path.join(data, 'fonts')
    for n in sorted(os.listdir(fonts_dir)):
        if n.endswith('.ttf'):
            QtGui.QFontDatabase.addApplicationFont(os.path.join(fonts_dir, n))

    qmlRegister = QtQml.qmlRegisterType
    qmlRegister(TextFile, 'TextFile', 1, 0, 'TextFile')
    if not a.fake_helper:
        # A host that must find no helper shares the lock with the others that do not want one, and is kept apart from every host that serves
        # the stand-in on the fixed port (below): it would otherwise read another run's weather as its own. The server of the host before it can
        # outlive that host's lock for a moment, so the port is waited on until nothing answers.
        import fcntl
        import socket
        _lock = open(os.path.join(tempfile.gettempdir(), 'lumatv-qmlhost.lock'), 'w')
        fcntl.flock(_lock, fcntl.LOCK_SH)
        for _ in range(100):
            with socket.socket() as probe:
                if probe.connect_ex(('127.0.0.1', 18731)) != 0:
                    break
            time.sleep(0.05)
    if a.fake_helper:
        sys.path.insert(0, HERE)
        import fakehelper
        # The helper's port is fixed (kit/Helper.qml): two hosts at once (parallel workers, a test beside a scene
        # render) cannot both bind it, so the second waits for the first to exit. The lock is released when main()
        # returns, a moment before the process (and its listening socket) is gone, so the bind itself retries.
        import fcntl
        _lock = open(os.path.join(tempfile.gettempdir(), 'lumatv-qmlhost.lock'), 'w')
        fcntl.flock(_lock, fcntl.LOCK_EX)
        for attempt in range(200):
            try:
                _srv, helper_state = fakehelper.serve(18731)
                break
            except OSError as e:
                if e.errno != errno.EADDRINUSE or attempt == 199:
                    raise
                time.sleep(0.05)

    w, h = (int(x) for x in a.size.split('x'))
    view = QtQuick.QQuickView()
    eng = view.engine()
    eng.addImportPath(os.path.join(HERE, 'stubs'))
    eng.addImportPath(os.path.join(data, 'qml'))
    view.setResizeMode(QtQuick.QQuickView.SizeRootObjectToView)
    view.resize(w, h)
    # The surface is the child of a plain Item the size of the window, as in the container.
    host = os.path.join(data, 'Host.qml')
    rel, qml_name = os.path.split(a.qml)
    if rel.startswith('systemui/'):
        mod_import = rel                      # staged under the data dir, beside qml/
    else:
        mod_import = 'qml/WebOSCompositor/%s' % brand()['qmlModule'] + ('/' + rel if rel else '')
    mods = ''
    if a.bgimage:
        mods += '  Image { anchors.fill: parent; source: "file://%s"; fillMode: Image.Stretch }\n' % os.path.abspath(a.bgimage)
    vis = '; visible: hostActive' if a.overlay else ''
    mods += '  M.%s { id: surface; objectName: "surface"%s }\n' % (os.path.splitext(qml_name)[0], vis)
    if a.overlay:
        mods += '  M.LumaOverlay { id: overlay; objectName: "overlay"; z: 3 }\n'
    with open(host, 'w') as f:
        f.write('import QtQuick 2.4\nimport "%s" as M\nItem { width: %d; height: %d\n%s}\n' % (mod_import, w, h, mods))
    view.setSource(QtCore.QUrl.fromLocalFile(host))
    if view.status() != QtQuick.QQuickView.Ready:
        for e in view.errors():
            print('ERROR', e.toString())
        return 1
    QtGui.QCursor.setPos(0, 0)      # Xvfb starts the pointer mid-screen, which hovers whatever is there: park it before the window maps
    view.show()
    QtGui.QCursor.setPos(0, 0)
    root = view.rootObject()
    surface = root.findChild(QtCore.QObject, 'surface')
    for p in a.prop:
        k, _, v = p.partition('=')
        surface.setProperty(k, coerce(v))

    def press():
        for name in (a.press or '').split(','):
            if not name:
                continue
            key = KEYS[name.strip().upper()]
            for typ in (QtCore.QEvent.KeyPress, QtCore.QEvent.KeyRelease):
                app.sendEvent(view, QtGui.QKeyEvent(typ, key, QtCore.Qt.NoModifier))

    def shot():
        img = view.grabWindow()
        if a.shot:
            img.save(a.shot)
            print(f'shot {a.shot} {img.width()}x{img.height()}')
        app.quit()

    def fire(name, payload):
        QtCore.QMetaObject.invokeMethod(surface, 'fire', QtCore.Q_ARG('QVariant', name), QtCore.Q_ARG('QVariant', payload))

    def snap(path):
        view.grabWindow().save(path)
        print(f'shot {path}', flush=True)

    def audit(path, kinds):
        if HERE not in sys.path:
            sys.path.insert(0, HERE)
        import layoutaudit
        found = layoutaudit.audit(root, w, h, view.grabWindow(), kinds)
        layoutaudit.write(path, found)
        print(f'audit {path} {len(found)} finding(s)', flush=True)
    for spec in a.audit:
        ms, path, *rest = spec.split(':')
        kinds = rest[0].split(',') if rest else None
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda p=path, k=kinds: audit(p, k))
    for spec in a.snap:
        ms, path = spec.split(':', 1)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda p=path: snap(p))
    for spec in a.fire:
        ms, name, *rest = spec.split(':', 2)
        payload = json.loads(rest[0]) if rest else {}
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda n=name, p=payload: fire(n, p))
    for spec in a.publish:
        ms, topic, data = spec.split(':', 2)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda t=topic, d=json.loads(data): helper_state.publish(t, d))
    view.rootContext().setContextProperty('root', surface)
    if a.overlay:
        view.rootContext().setContextProperty('overlay', root.findChild(QtCore.QObject, 'overlay'))

    def evaluate(expr):
        e = QtQml.QQmlExpression(view.rootContext(), surface, expr)
        v = e.evaluate()
        emit(f'[eval] {v}')
    for spec in a.eval:
        ms, expr = spec.split(':', 1)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda x=expr: evaluate(x))
    def pointer(kind, expr):
        """The Magic Remote's pointer as mouse events at the window position the JavaScript expression returns (an array [x, y], read as "x,y")."""
        v = QtQml.QQmlExpression(view.rootContext(), surface, expr).evaluate()[0]
        x, y = (v.toString() if isinstance(v, QtQml.QJSValue) else str(v)).split(',')
        pos = QtCore.QPointF(float(x), float(y))
        none, left = QtCore.Qt.NoButton, QtCore.Qt.LeftButton
        events = {'move': [(QtCore.QEvent.MouseMove, none, none)],
                  'click': [(QtCore.QEvent.MouseButtonPress, left, left), (QtCore.QEvent.MouseButtonRelease, left, none)]}[kind]
        for typ, button, buttons in events:
            app.sendEvent(view, QtGui.QMouseEvent(typ, pos, pos, view.mapToGlobal(pos.toPoint()), button, buttons, QtCore.Qt.NoModifier))
    for spec in a.pointer:
        ms, kind, expr = spec.split(':', 2)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda k=kind, x=expr: pointer(k, x))
    frames = {'n': 0}
    view.frameSwapped.connect(lambda: frames.__setitem__('n', frames['n'] + 1))
    for spec in a.frames:
        ms, label = spec.split(':', 1)
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda l=label: print(f'[frames] {l} {frames["n"]}', flush=True))
    if a.press:
        QtCore.QTimer.singleShot(a.wait // 2, QtCore.Qt.PreciseTimer, press)
    QtCore.QTimer.singleShot(a.wait, QtCore.Qt.PreciseTimer, shot)
    app.exec_()

    own = [m for m in messages if m[0] in (1, 2, 3) and not IGNORE.search(m[3])]
    print(f'{len(own)} warning(s) from the surface')
    if not a.data:
        shutil.rmtree(data, ignore_errors=True)
    return 1 if own else 0


if __name__ == '__main__':
    sys.exit(main())
