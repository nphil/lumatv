#!/usr/bin/env python3
"""Offline run of LG's `StarfishKeyboardView.qml` (the compositor view that hosts the on-screen keyboard's window), stock or patched.

    python3 tests/qmlhost/keyboard.py [--patched] [--wait MS] [--layer-ms N]

LG's own `BaseView` (the open/close state machine the view animates through) is copied from the census (`/data/home/lgtv/uiinv/qml/qml`)
and runs as it is; the compositor's native types are stand-ins: a window model that emits `surfaceAdded`/`surfaceRemoved`, a surface
item, `compositor.inputMethod`, the settings and log singletons. The host adds a surface (the IME's window, 1920x420) at 300 ms, removes
it 1200 ms later, and prints what the view did, timed from the add/remove (`[event] opened MS`, `[event] closed MS`), the panel rect it
reported and, at 1400 ms and at the end, a `[report ...]` of where the window is. The animation clock here runs a little under the
wall clock (software GL), so a duration is read against another run's, never against a stopwatch: this measures nothing about speed.

--patched runs `tools/keyboard_patch.py` over the copy first, with the shipped theme.json. Exit status 0 when nothing from the tree
warned, 1 otherwise, 2 when LG's copy is not on this machine.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
CENSUS = '/data/home/lgtv/uiinv/qml/qml'
VIEW = os.path.join(CENSUS, 'WebOSCompositor', 'views', 'StarfishKeyboardView.qml')
BASE_VIEW = os.path.join(CENSUS, 'WebOSCompositorBase', 'views', 'base', 'BaseView.qml')

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)

# LG's view reads move-area window properties the stand-in surface does not carry (`undefined` to a string, on the set too)
IGNORE = re.compile(r'Implicitly defined onFoo|Unable to assign \[undefined\] to QString')

QMLDIRS = {
    'WebOSCoreCompositor': 'module WebOSCoreCompositor\nWindowModel 1.0 WindowModel.qml\n',
    'WebOSCompositorBase': ('module WebOSCompositorBase\nsingleton Settings 1.0 Settings.qml\nsingleton Utils 1.0 Utils.qml\n'
                            'BaseView 1.0 BaseView.qml\n'),
    'WebOSCompositor': 'module WebOSCompositor\nNothing 1.0 Nothing.qml\n',
    'Eos/Items': 'module Eos.Items\nBeziergon 0.1 Beziergon.qml\n',
}

FILES = {
    'WebOSCoreCompositor/WindowModel.qml': 'import QtQuick 2.4\nQtObject { signal surfaceAdded(var item); signal surfaceRemoved(var item); property int count: 0; function get(i) { return null } }\n',
    'WebOSCompositorBase/Settings.qml': 'pragma Singleton\nimport QtQuick 2.4\nQtObject { function subscribe(a, b, c) { return "landscape" } }\n',
    'WebOSCompositorBase/Utils.qml': 'pragma Singleton\nimport QtQuick 2.4\nQtObject { property var pmLog: QtObject { function info(k, v) {} } }\n',
    'WebOSCompositor/Nothing.qml': 'import QtQuick 2.4\nItem {}\n',
    # the native mesh item LG's keyboard view squashes; only the stock view has one
    'Eos/Items/Beziergon.qml': ('import QtQuick 2.4\nItem { property var sourceItem; property color color; property bool clampToEdge; property point controlTopLeft;\n'
                                ' property point topLeft; property point topRight; property point bottomLeft; property point bottomRight }\n'),
}

HOST_QML = '''import QtQuick 2.4
import WebOSCoreCompositor 1.0

Item {
    id: screen
    width: 1920; height: 1080
    Rectangle { anchors.fill: parent; color: "#0d1020" }
    WindowModel { id: model }
    property var t0: 0
    property var opened: -1
    property var closed: -1
    StarfishKeyboardView {
        id: view
        objectName: "view"
        anchors.fill: parent
        model: model
        onOpening: console.log("[event] opening " + (Date.now() - screen.t0))
        onClosing: console.log("[event] closing " + (Date.now() - screen.t0))
        onOpened: { console.log("[event] opened " + (Date.now() - screen.t0)); if (screen.opened < 0) screen.opened = Date.now() - screen.t0 }
        onClosed: { console.log("[event] closed " + (Date.now() - screen.t0)); if (screen.closed < 0) screen.closed = Date.now() - screen.t0 }
    }
    Component {
        id: surface
        Item {
            property var windowProperties: ({})
            property string type: "_WEBOS_WINDOW_TYPE_KEYBOARD"
            property bool useTextureAlpha: false
            function close() {}
            width: 1920; height: 420
            Rectangle { anchors.fill: parent; color: "#403a4a" }
        }
    }
    property var item: null
    // the window's top edge while the view opens: LG's OutBack carries it past its place (a smaller y), an ease-out does not
    property real minY: 1e9
    Timer {
        interval: 4; repeat: true; running: screen.item !== null && screen.opened < 0
        onTriggered: { var y = screen.item.mapToItem(screen, 0, 0).y; if (y < screen.minY) screen.minY = y }
    }
    function add() {
        item = surface.createObject(screen)
        t0 = Date.now()
        model.count = 1
        model.surfaceAdded(item)
    }
    function remove() {
        t0 = Date.now()
        model.count = 0
        model.surfaceRemoved(item)
    }
    // where the keyboard's window is on screen, and whether the view and the window are shown
    function report() {
        return JSON.stringify({opened: opened, closed: closed, visible: view.visible, isOpen: view.isOpen, itemVisible: item ? item.visible : null,
                               itemY: item ? item.mapToItem(screen, 0, 0).y : null, minY: minY})
    }
}
'''


def stage(dest, patched, layer_ms):
    qml = os.path.join(dest, 'qml')
    for rel, text in FILES.items():
        os.makedirs(os.path.dirname(os.path.join(qml, rel)), exist_ok=True)
        with open(os.path.join(qml, rel), 'w') as f:
            f.write(text)
    for d, text in QMLDIRS.items():
        with open(os.path.join(qml, d, 'qmldir'), 'w') as f:
            f.write(text)
    shutil.copy(BASE_VIEW, os.path.join(qml, 'WebOSCompositorBase', 'BaseView.qml'))
    shutil.copy(VIEW, os.path.join(qml, 'StarfishKeyboardView.qml'))
    if patched:
        theme = os.path.join(ROOT, 'payload', 'theme', 'theme.json')
        if layer_ms:
            with open(theme) as f:
                t = json.load(f)
            t['motion']['layerMs'] = layer_ms
            theme = os.path.join(dest, 'theme.json')
            with open(theme, 'w') as f:
                json.dump(t, f)
        r = subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'keyboard_patch.py'), os.path.join(qml, 'StarfishKeyboardView.qml'), theme],
                           capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit('patch failed: ' + r.stderr)
    with open(os.path.join(qml, 'qmldir'), 'w') as f:
        f.write('StarfishKeyboardView 1.0 StarfishKeyboardView.qml\n')
    host = os.path.join(qml, 'Host.qml')
    with open(host, 'w') as f:
        f.write(HOST_QML)
    return qml, host


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--patched', action='store_true')
    ap.add_argument('--wait', type=int, default=2600)
    ap.add_argument('--layer-ms', type=int, default=0, help='with --patched: a theme whose motion.layerMs is this')
    a = ap.parse_args(argv)
    if not os.path.isfile(VIEW):
        print("needs the census copy of LG's keyboard view at", VIEW)
        return 2

    from PyQt5 import QtCore, QtGui, QtQml, QtQuick
    work = tempfile.mkdtemp(prefix='keyboard-')
    qml, host = stage(work, a.patched, a.layer_ms)
    messages = []

    def handler(mode, ctx, msg):
        messages.append((int(mode), ctx.file or '', ctx.line, msg))
        print('[%s] %s:%s: %s' % (['debug', 'warning', 'critical', 'fatal', 'info'][int(mode)], os.path.basename(ctx.file or ''), ctx.line, msg), flush=True)
    QtCore.qInstallMessageHandler(handler)

    app = QtGui.QGuiApplication(sys.argv[:1])
    fmt = QtGui.QSurfaceFormat()
    fmt.setRenderableType(QtGui.QSurfaceFormat.OpenGL)
    fmt.setVersion(2, 1)
    QtGui.QSurfaceFormat.setDefaultFormat(fmt)

    class Input(QtCore.QObject):
        panelSurfaceSizeChanged = QtCore.pyqtSignal()
        allowedChanged = QtCore.pyqtSignal()
        panelRectChanged = QtCore.pyqtSignal()

        def __init__(self):
            super().__init__()
            self._allowed, self._rect = True, QtCore.QRectF()

        @QtCore.pyqtProperty(bool, notify=allowedChanged)
        def allowed(self):
            return self._allowed

        @allowed.setter
        def allowed(self, v):
            self._allowed = v

        @QtCore.pyqtProperty(QtCore.QRectF, notify=panelRectChanged)
        def panelRect(self):
            return self._rect

        @panelRect.setter
        def panelRect(self, v):
            self._rect = v
            print('[panelRect] %d,%d %dx%d' % (v.x(), v.y(), v.width(), v.height()), flush=True)

        @QtCore.pyqtSlot()
        def deactivate(self):
            print('[inputMethod] deactivate', flush=True)

    input_method = Input()

    class Compositor(QtCore.QObject):
        @QtCore.pyqtProperty(QtCore.QObject, constant=True)
        def inputMethod(self):
            return input_method

        @QtCore.pyqtSlot()
        def updateCursorFocus(self):
            pass
    compositor = Compositor()

    view = QtQuick.QQuickView()
    view.engine().addImportPath(qml)
    view.rootContext().setContextProperty('compositor', compositor)
    view.setResizeMode(QtQuick.QQuickView.SizeRootObjectToView)
    view.resize(1920, 1080)
    view.setSource(QtCore.QUrl.fromLocalFile(host))
    if view.status() != QtQuick.QQuickView.Ready:
        for e in view.errors():
            print('ERROR', e.toString())
        return 1
    root = view.rootObject()
    view.show()

    def call(name):
        QtCore.QMetaObject.invokeMethod(root, name)
    QtCore.QTimer.singleShot(300, lambda: call('add'))
    QtCore.QTimer.singleShot(1500, lambda: call('remove'))

    def report(tag):
        v = QtCore.QMetaObject.invokeMethod(root, 'report', QtCore.Qt.DirectConnection, QtCore.Q_RETURN_ARG('QVariant'))
        print('[report %s] %s' % (tag, v), flush=True)
    QtCore.QTimer.singleShot(1400, lambda: report('open'))
    QtCore.QTimer.singleShot(a.wait, lambda: report('closed'))
    QtCore.QTimer.singleShot(a.wait + 200, app.quit)
    t = time.time()
    app.exec_()
    shutil.rmtree(work, ignore_errors=True)
    own = [m for m in messages if m[0] in (1, 2, 3) and not IGNORE.search(m[3])]
    print('%d warning(s) from the tree (%.1f s)' % (len(own), time.time() - t))
    return 1 if own else 0


if __name__ == '__main__':
    sys.exit(main())
