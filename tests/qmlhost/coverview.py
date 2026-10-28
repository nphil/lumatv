#!/usr/bin/env python3
"""Offline run of LG's startup cover (`CoverUI/UserInterfaceLayer/Containers/MainView.qml`), stock or patched, drawn as the set draws it.

    python3 tests/qmlhost/coverview.py [--patched] [--app com.webos.app.hdmi2] [--name HDMI2] [--last] [--stolen] [--shot PATH] [--wait MS]

LG's own MainView is copied from the census (`/data/home/lgtv/uiinv/qml/qml/CoverUI`) and run as it is; what it asks of the module around it
(`systemProperties`, `styleSheet`, `stringSheet`, `fontManager`, `interfaces`, `uiController` and `rootWindow`) is a stand-in whose names and
shapes are the ones the file reads. --app and --name say which app the cover is for, --last that it is the one the TV was on, --stolen LG's
block screen. --patched runs `tools/cover_patch.py` over the copy first. Qt is the PyQt5 wheel's 5.15 on an X display with software GL.
Prints `[report {...}]` at the end (what the cover shows) and exits 0 when nothing in the tree warned, 1 otherwise, 2 without the census.
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
CENSUS = '/data/home/lgtv/uiinv/qml/qml/CoverUI'
TOOL = os.path.join(ROOT, 'tools', 'cover_patch.py')
PALETTE = os.path.join(ROOT, 'tools', 'tokens.py')

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)
sys.path.insert(0, os.path.join(ROOT, 'tests'))
import xdisplay  # noqa: E402  (an X display for the window, started once)
if not os.environ.get('DISPLAY') and xdisplay.DISPLAY:
    os.environ['DISPLAY'] = xdisplay.DISPLAY

HOST = '''import QtQuick 2.4

Item {
    id: rootWindow
    width: 1920; height: 1080
    signal aboutToClose()
    signal powerOff()
    function close() {}
    property var systemProperties: ({
        appNameToLaunch: %(name)s, appToLaunch: { id: "%(app)s" }, appIdToLaunch: "%(app)s", cpAppNames: ({}),
        lastInput: %(last)s, supportsBNO: false, isStolenDevice: %(stolen)s, macAddress: "00:11:22:33:44:55", isRTLLocale: false,
        requestedLaunchAppId: "", foregroundAppIds: [], coreBootDone: false, readyToLaunch: false, screenSaverTimestamp: 0,
        powerStatus: "Active", powerProcessingStatus: "", isAppReadyToUse: false })
    property var styleSheet: ({ getRValue: function(v) { return v * rootWindow.width / 1920 } })
    property var stringSheet: ({ loading: "Loading...", blockTV_3: "MAC", blockTV_4: "Network settings", blockTV_5: "This TV is blocked." })
    property var fontManager: ({ smart_SemiBold: { family: "Manrope", weight: Font.DemiBold } })
    property var uiController: ({ isMouseActive: false })
    property var interfaces: ({ application: { startFadeoutAnimation: false, isSmartServiceReady: false,
        stopTimers: function() {}, assureIBD: function() {}, launchApp: function() {}, powerOffDevice: function() {}, setSystemSetting: function() {},
        firstAppLaunchDone: function() {}, getFirstAppInformation: function() {}, responseScreenSaver: function() {}, launchFallback: function() {} } })
    function printLog(v) { console.log(v) }
    Loader { anchors.fill: parent; source: "%(main)s" }
}
'''


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--patched', action='store_true')
    ap.add_argument('--app', default='com.webos.app.hdmi2')
    ap.add_argument('--name', default='HDMI 2')
    ap.add_argument('--last', action='store_true', help='the app is the one the TV was on')
    ap.add_argument('--stolen', action='store_true')
    ap.add_argument('--shot', help='PNG of the window at --wait')
    ap.add_argument('--wait', type=int, default=1500)
    ap.add_argument('--keep', help='keep the staged copy in this directory')
    a = ap.parse_args(argv)
    if not os.path.isdir(CENSUS):
        print("needs the census copy of LG's CoverUI at", CENSUS)
        return 2
    from PyQt5 import QtCore, QtGui, QtQuick
    work = os.path.abspath(a.keep) if a.keep else tempfile.mkdtemp(prefix='cover-')
    if a.keep and os.path.exists(work):
        shutil.rmtree(work)
    os.makedirs(work, exist_ok=True)
    module = os.path.join(work, 'CoverUI')
    shutil.copytree(CENSUS, module)
    if a.patched:
        pal = subprocess.run([sys.executable, PALETTE, os.path.join(ROOT, 'payload', 'theme', 'theme.json'), '/nonexistent', 'qml'],
                             capture_output=True, text=True, check=True).stdout
        r = subprocess.run([sys.executable, TOOL, module, pal, 'LumaTV'], capture_output=True, text=True)
        if r.returncode != 0:
            print(r.stderr)
            return 1
    main_qml = os.path.join(module, 'UserInterfaceLayer', 'Containers', 'MainView.qml')
    host = os.path.join(work, 'Host.qml')
    last = '{ id: "%s" }' % a.app if a.last else 'undefined'
    with open(host, 'w') as f:
        f.write(HOST % {'name': json.dumps(a.name), 'app': a.app, 'last': last, 'stolen': 'true' if a.stolen else 'false', 'main': 'file://' + main_qml})

    messages = []

    def handler(mode, ctx, msg):
        messages.append((int(mode), msg))
        print('[%s] %s:%s: %s' % (['debug', 'warning', 'critical', 'fatal', 'info'][int(mode)], os.path.basename(ctx.file or ''), ctx.line, msg), flush=True)
    QtCore.qInstallMessageHandler(handler)
    app = QtGui.QGuiApplication(sys.argv[:1])
    fmt = QtGui.QSurfaceFormat()
    fmt.setRenderableType(QtGui.QSurfaceFormat.OpenGL)
    fmt.setVersion(2, 1)
    QtGui.QSurfaceFormat.setDefaultFormat(fmt)
    for n in sorted(os.listdir(os.path.join(ROOT, 'payload', 'fonts'))):
        if n.endswith('.ttf'):
            QtGui.QFontDatabase.addApplicationFont(os.path.join(ROOT, 'payload', 'fonts', n))
    view = QtQuick.QQuickView()
    view.setResizeMode(QtQuick.QQuickView.SizeRootObjectToView)
    view.resize(1920, 1080)
    view.setSource(QtCore.QUrl.fromLocalFile(host))
    if view.status() != QtQuick.QQuickView.Ready:
        for e in view.errors():
            print('ERROR', e.toString())
        return 1
    view.show()

    def finish():
        if a.shot:
            img = view.grabWindow()
            img.save(a.shot)
            print('shot %s %dx%d' % (a.shot, img.width(), img.height()), flush=True)
        root = view.rootObject()
        loader = root.childItems()[0]
        item = loader.property('item')

        def find(o, name):
            if o.property('objectName') == name:
                return o
            for c in o.childItems():
                r = find(c, name)
                if r is not None:
                    return r
            return None
        title = find(item, 'mainTitle') if item else None
        print('[report %s]' % json.dumps({'loaded': item is not None, 'title': title.property('text') if title else None,
                                            'titleVisible': bool(title.property('visible')) if title else None,
                                            'color': item.property('color').name() if item else None}), flush=True)
        app.quit()
    QtCore.QTimer.singleShot(a.wait, QtCore.Qt.PreciseTimer, finish)
    app.exec_()
    bad = [m for m in messages if m[0] in (1, 2, 3) and 'Implicitly defined onFoo' not in m[1]]
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
