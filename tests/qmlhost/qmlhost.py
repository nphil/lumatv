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
calls are recorded in `LS.calls`, `TextFile` reads and writes real files.

Exit status 0 when the surface loaded and printed no warning from its own files, 1 otherwise.
"""
import argparse
import json
import os
import re
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)
os.environ.setdefault('QML_XHR_ALLOW_FILE_READ', '1')

from PyQt5 import QtCore, QtGui, QtQml, QtQuick  # noqa: E402


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
    os.makedirs(os.path.join(data_dir, 'cache'), exist_ok=True)
    subst = {'displayName': b['displayName'], 'shortId': b['shortId'], 'appId': b['appId'],
             'dataDir': data_dir, 'qmlModule': b['qmlModule']}
    for dirpath, _, files in os.walk(mod):
        for n in files:
            if n.endswith(('.qml', '.js', '.json')):
                p = os.path.join(dirpath, n)
                with open(p, encoding='utf-8') as f:
                    s = f.read()
                s2 = re.sub(r'\{\{(\w+)\}\}', lambda m: subst.get(m.group(1), m.group(0)), s)
                if s2 != s:
                    with open(p, 'w', encoding='utf-8') as f:
                        f.write(s2)
    left = []
    for dirpath, _, files in os.walk(mod):
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
IGNORE = re.compile(r'Implicitly defined onFoo|QSslSocket|OpenSSL|place lookup failed|no stories; retry|'
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
    ap.add_argument('--wait', type=int, default=3000, help='ms to run before the screenshot')
    ap.add_argument('--prop', action='append', default=[], help='root property to set, name=value')
    ap.add_argument('--press', help='comma-separated keys sent after --wait/2 (LEFT RIGHT UP DOWN OK BACK MENU)')
    ap.add_argument('--data', help='keep the staged data dir here (default: a temp dir)')
    ap.add_argument('--size', default='1920x1080')
    a = ap.parse_args(argv)

    data = os.path.abspath(a.data) if a.data else tempfile.mkdtemp(prefix='qmlhost-')
    if a.data and os.path.exists(data):
        shutil.rmtree(data)
    os.makedirs(data, exist_ok=True)
    src = os.path.join(ROOT, 'payload', 'compositor', 'lumatv')
    mod = stage(data, src)

    messages = []

    def handler(mode, ctx, msg):
        messages.append((int(mode), ctx.file or '', ctx.line, msg))
        print(f'[{["debug", "warning", "critical", "fatal", "info"][int(mode)]}] {os.path.basename(ctx.file or "")}:{ctx.line}: {msg}', flush=True)
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
    mod_import = 'qml/WebOSCompositor/%s' % brand()['qmlModule'] + ('/' + rel if rel else '')
    with open(host, 'w') as f:
        f.write('import QtQuick 2.4\nimport "%s" as M\nItem { width: %d; height: %d\n  M.%s { id: surface; objectName: "surface" }\n}\n'
                % (mod_import, w, h, os.path.splitext(qml_name)[0]))
    view.setSource(QtCore.QUrl.fromLocalFile(host))
    if view.status() != QtQuick.QQuickView.Ready:
        for e in view.errors():
            print('ERROR', e.toString())
        return 1
    view.show()
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

    if a.press:
        QtCore.QTimer.singleShot(a.wait // 2, press)
    QtCore.QTimer.singleShot(a.wait, shot)
    app.exec_()

    own = [m for m in messages if m[0] in (1, 2, 3) and not IGNORE.search(m[3])]
    print(f'{len(own)} warning(s) from the surface')
    if not a.data:
        shutil.rmtree(data, ignore_errors=True)
    return 1 if own else 0


if __name__ == '__main__':
    sys.exit(main())
