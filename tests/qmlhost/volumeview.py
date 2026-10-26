#!/usr/bin/env python3
"""Offline run of LG's `StarfishVolumeView.qml` (the compositor view that draws the volume and mute popup), stock or patched.

    python3 tests/qmlhost/volumeview.py [--patched] [--output external_arc] [--muted] [--ukraine]

LG's own `BaseView` and its own view file are copied from the census (`/data/home/lgtv/uiinv/qml/qml`) and run as they are; what the view
asks of the compositor is a stand-in: the audio service (`LS.audioService`, with LG's property names and `volumeUpdated`), the settings
singleton (nothing is subscribed, so `volumeOSD` is undefined, which LG reads as on), and the two popups it hosts (`StarfishVolume`,
`StarfishVolumeMute`), which count the times they are asked to open. The host reports, at 200 ms (before anything) and at 2000 ms
(after the audio service has announced a change at 500 ms and, with --muted, a mute at 900 ms), `[report {...}]`: `available`, whether
the view and the popups opened, and how many times they were asked to. With --ukraine the TV is muted and the country setting becomes Ukraine at
300 ms (where LG keeps the mute bubble up) and nothing else is announced: the one path that calls `updateVisibility()` without asking `available`.

--patched runs `tools/volume_patch.py` over the copy first. Exit status 0 when nothing from the tree warned, 1 otherwise, 2 when LG's copy is
not on this machine.
"""
import argparse
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
VIEW = os.path.join(CENSUS, 'WebOSCompositor', 'views', 'StarfishVolumeView.qml')
BASE_VIEW = os.path.join(CENSUS, 'WebOSCompositorBase', 'views', 'base', 'BaseView.qml')

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)

IGNORE = re.compile(r'Implicitly defined onFoo')

QMLDIRS = {
    'WebOSCompositorBase': ('module WebOSCompositorBase\nsingleton LS 1.0 LS.qml\nsingleton Settings 1.0 Settings.qml\nBaseView 1.0 BaseView.qml\n'),
    'WebOSCompositor': 'module WebOSCompositor\nNothing 1.0 Nothing.qml\n',
}

FILES = {
    'WebOSCompositorBase/LS.qml': ('pragma Singleton\nimport QtQuick 2.4\nQtObject {\n'
                                   '    property QtObject audioService: QtObject {\n'
                                   '        property bool muted: false\n        property bool disabled: false\n        property int volume: 10\n'
                                   '        property int volumeMax: 100\n        property string soundOutput: "external_arc"\n'
                                   '        property bool volumeSyncable: false\n        property string mode: "normal"\n        property string cause: "volumeUp"\n'
                                   '        property bool ossActivate: false\n        signal volumeUpdated()\n'
                                   '        function volumeUp() {}\n        function volumeDown() {}\n    }\n}\n'),
    'WebOSCompositorBase/Settings.qml': ('pragma Singleton\nimport QtQuick 2.4\nQtObject {\n    function subscribe(a, b, c) { return undefined }\n'
                                         '    property var local: ({ volume: { barMiddleHeight: 534 } })\n    property var l10n: ({ tr: "", locale: "en-US" })\n}\n'),
    'WebOSCompositor/Nothing.qml': 'import QtQuick 2.4\nItem {}\n',
    'WebOSCompositor/views/PartialViewPositioner.qml': 'import QtQuick 2.4\nItem { property string currentOrientation: "landscape" }\n',
    'WebOSCompositor/controllers/base/StarfishRollingScreenService.qml': ('import QtQuick 2.4\nQtObject { property var viewState: ({ ui: "full" })\n'
                                                                           '    property string supportScreenType: "none" }\n'),
    # the two popups the view hosts: they count how often they are asked to open
    'WebOSCompositor/views/volume/StarfishVolume.qml': ('import QtQuick 2.4\nItem {\n    property bool partialViewState: false\n'
                                                        '    property string supportScreenType: "none"\n    property var positioner: null\n    property bool harmony: false\n'
                                                        '    property real volumeBarMiddleLength: 0\n    property string volumeDirectionString: ""\n'
                                                        '    property bool volumeBarVisible: false\n    property bool isOpen: false\n    property int opens: 0\n'
                                                        '    property string volumeStr: "volume"\n    property string upStr: "up"\n    property string downStr: "down"\n'
                                                        '    property string volumeDisabledStr: "disabled"\n    signal opened()\n    signal closed()\n'
                                                        '    function update() {}\n    function restartClosingTimer() {}\n    function handleVolumeBannerTimeout(r) {}\n'
                                                        '    function openView() { opens++; isOpen = true; opened() }\n    function closeView() { isOpen = false; closed() }\n}\n'),
    'WebOSCompositor/views/volume/StarfishVolumeMute.qml': ('import QtQuick 2.4\nItem {\n    property bool isOpen: false\n    property int opens: 0\n    property string state: "closed"\n'
                                                            '    signal opened()\n    signal closed()\n'
                                                            '    function openView() { opens++; isOpen = true; opened() }\n    function closeView() { isOpen = false; closed() }\n}\n'),
}

HOST_QML = '''import QtQuick 2.4
import WebOSCompositorBase 1.0
import WebOSCompositor 1.0

Item {
    id: screen
    width: 1920; height: 1080
    StarfishVolumeView {
        id: view
        objectName: "view"
        width: parent.width; height: parent.height
    }
    function popup() { return view.children }
    function change(muted) {
        LS.audioService.muted = muted
        LS.audioService.volume = muted ? 10 : 11
        LS.audioService.volumeUpdated()
    }
    function ukraine() {
        LS.audioService.muted = true
        view.setLocaleInfo({ country: "UKR" })
    }
    function report() {
        var opens = 0, muteOpens = 0
        for (var i = 0; i < view.data.length; i++) {
            var c = view.data[i]
            if (c && c.opens !== undefined) { if (c.volumeStr !== undefined) opens = c.opens; else muteOpens = c.opens }
        }
        return JSON.stringify({ available: view.available, viewOpen: view.isOpen, visible: view.visible, popupOpens: opens, muteOpens: muteOpens })
    }
}
'''


def stage(dest, patched):
    qml = os.path.join(dest, 'qml')
    for rel, text in FILES.items():
        os.makedirs(os.path.dirname(os.path.join(qml, rel)), exist_ok=True)
        with open(os.path.join(qml, rel), 'w') as f:
            f.write(text)
    for d, text in QMLDIRS.items():
        with open(os.path.join(qml, d, 'qmldir'), 'w') as f:
            f.write(text)
    shutil.copy(BASE_VIEW, os.path.join(qml, 'WebOSCompositorBase', 'BaseView.qml'))
    view = os.path.join(qml, 'WebOSCompositor', 'views', 'StarfishVolumeView.qml')
    shutil.copy(VIEW, view)
    if patched:
        r = subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'volume_patch.py'), view], capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit('patch failed: ' + r.stderr)
    with open(os.path.join(qml, 'WebOSCompositor', 'qmldir'), 'a') as f:
        f.write('StarfishVolumeView 1.0 views/StarfishVolumeView.qml\n')
    host = os.path.join(qml, 'Host.qml')
    with open(host, 'w') as f:
        f.write(HOST_QML)
    return qml, host


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--patched', action='store_true')
    ap.add_argument('--output', default='external_arc', help='the audio service\'s soundOutput')
    ap.add_argument('--muted', action='store_true', help='announce a mute as well')
    ap.add_argument('--ukraine', action='store_true', help='mute the TV and change the country setting to Ukraine, announcing nothing')
    a = ap.parse_args(argv)
    if not os.path.isfile(VIEW):
        print("needs the census copy of LG's volume view at", VIEW)
        return 2

    from PyQt5 import QtCore, QtGui, QtQml, QtQuick
    work = tempfile.mkdtemp(prefix='volumeview-')
    qml, host = stage(work, a.patched)
    with open(os.path.join(qml, 'WebOSCompositorBase', 'LS.qml')) as f:
        text = f.read().replace('"external_arc"', '"%s"' % a.output)
    with open(os.path.join(qml, 'WebOSCompositorBase', 'LS.qml'), 'w') as f:
        f.write(text)
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

    class Compositor(QtCore.QObject):
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

    def change(muted):
        QtCore.QMetaObject.invokeMethod(root, 'change', QtCore.Q_ARG('QVariant', muted))

    def report(tag):
        v = QtCore.QMetaObject.invokeMethod(root, 'report', QtCore.Qt.DirectConnection, QtCore.Q_RETURN_ARG('QVariant'))
        print('[report %s] %s' % (tag, v), flush=True)
    QtCore.QTimer.singleShot(200, lambda: report('before'))
    if a.ukraine:
        QtCore.QTimer.singleShot(300, lambda: QtCore.QMetaObject.invokeMethod(root, 'ukraine'))
    else:
        QtCore.QTimer.singleShot(500, lambda: change(False))
        if a.muted:
            QtCore.QTimer.singleShot(900, lambda: change(True))
    QtCore.QTimer.singleShot(2000, lambda: report('after'))
    QtCore.QTimer.singleShot(2200, app.quit)
    t = time.time()
    app.exec_()
    shutil.rmtree(work, ignore_errors=True)
    own = [m for m in messages if m[0] in (1, 2, 3) and not IGNORE.search(m[3])]
    print('%d warning(s) from the tree (%.1f s)' % (len(own), time.time() - t))
    return 1 if own else 0


if __name__ == '__main__':
    sys.exit(main())
