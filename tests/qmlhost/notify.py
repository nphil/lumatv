#!/usr/bin/env python3
"""Offline preview of LG's notification view (toasts, alerts, input popups, the PIN prompt).

    python3 tests/qmlhost/notify.py [--patched] [--shot out.png] [--wait MS] [--backdrop IMG]
        [--toast JSON] [--alert JSON] [--input JSON] [--pin JSON] [--press KEY[,KEY...]]

LG's own `SystemUI/Notification` tree, copied from the census (`/data/home/lgtv/uiinv/qml/qml`), runs
here with its real data layer: `Services.qml` and its four models are LG's and are fed through the
luna stand-in, so a JSON document published with --toast or --alert is parsed, queued and drawn by
the same code as on the set. Everything native is a stub (the luna bridge, PmLog, iLib, Eos, the
locale service) and records or ignores what it is asked.

--patched runs `tools/notification_patch.py` over the copy first, exactly as `lumatv notifications on`
does on the set. Qt is the PyQt5 wheel's 5.15 on Xvfb with software GL: this finds load errors and
shows what a notification looks like, and measures nothing. The gate on the set is the only timing.

Each --toast / --alert / --input / --pin is `MS:JSON`, published that many ms after start; the JSON is
what the notification service answers the view's subscription with (see DEFAULTS). Exit status 0 when nothing
from the tree warned, 1 otherwise.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
CENSUS = '/data/home/lgtv/uiinv/qml/qml'
STOCK = os.path.join(CENSUS, 'SystemUI', 'Notification')

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)

# Qt 5.15 deprecates the Connections syntax LG's tree (written for 5.12) uses throughout; LG's own PIN pad
# writes through a null KeyNavigation.left when it is not the partial-screen layout (the stock tree only).
IGNORE = re.compile(r'Implicitly defined onFoo|PincodePrompt\.qml:489: TypeError: Value is null'
                    # LG's delegates bind to `parent` and warn once as the item is removed, stock tree and ours alike
                    r"|AlertView\.qml:113: TypeError: Cannot read property 'top' of null|AdvancedAlertView\.qml:65: TypeError: Cannot read property 'width' of null")

KEYS = {'LEFT': 'Key_Left', 'RIGHT': 'Key_Right', 'UP': 'Key_Up', 'DOWN': 'Key_Down', 'OK': 'Key_Return',
        'BACK': 'Key_Escape', 'MENU': 'Key_Menu', 'BACKSPACE': 'Key_Backspace'}
KEYS.update({str(d): 'Key_%d' % d for d in range(10)})

# What a notification of each kind looks like to the model, with the fields LG's views read.
DEFAULTS = {
    'toast': {'sourceId': 'com.webos.app.home', 'message': 'Volume 24 set by Cody', 'type': 'standard', 'iconPath': '', 'readable': '',
              'timeout': 8, 'userData': {'id': 'preview'}},
    'alert': {'alertInfo': {'sourceId': 'com.webos.app.home', 'title': 'Uninstall Crunchyroll?',
                            'message': 'The app and its sign-in on this TV are removed. Downloads in the app are lost.',
                            'readable': '', 'showTimeout': False, 'modal': True, 'timeout': 0, 'timestamp': 1,
                            'buttons': [{'label': 'Keep it', 'focus': True, 'params': {}}, {'label': 'Uninstall', 'params': {}}]}},
    'input': {'alertAction': 'open',
              'alertInfo': {'sourceId': 'com.webos.app.hdmi3', 'timeout': 15, 'timestamp': 2, 'portName': 'HDMI 3',
                            'portIcon': os.path.join(ROOT, 'assets', 'icon160.png'),
                            'action': {'serviceURI': 'luna://com.webos.applicationManager', 'serviceMethod': '/launch',
                                       'launchParams': {'id': 'com.webos.app.hdmi3'}}}},
    'pin': {'pincodePromptAction': 'open',
            'pincodePromptInfo': {'appId': 'com.webos.app.hdmi3', 'title': 'Enter the TV PIN', 'message': 'HDMI 3 is locked.',
                                  'promptType': 'parental', 'retry': 0, 'timestamp': 3}},
}


def palette(light):
    """The menu palette `lumatv notifications on` stages with. The shipped one always comes from the dark material (the
    menus are always drawn on it, tokens.py); `light` swaps the light material in for it, to see the same view in a
    theme whose sheet is light."""
    shipped = os.path.join(ROOT, 'payload', 'theme', 'theme.json')
    user = os.devnull
    tmp = None
    if light:
        with open(shipped, encoding='utf-8') as f:
            tmp = tempfile.NamedTemporaryFile('w', suffix='.json', delete=False)
            json.dump({'dark': json.load(f)['light']}, tmp)
            tmp.close()
        user = tmp.name
    try:
        return subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'tokens.py'), shipped, user, 'qml'],
                              capture_output=True, text=True, check=True).stdout.strip()
    finally:
        if tmp:
            os.unlink(tmp.name)


def stage(dest, patched, light=False):
    """The module tree the view runs in, as import root `dest`."""
    qml = os.path.join(dest, 'qml')
    os.makedirs(qml)
    note = os.path.join(qml, 'SystemUI', 'Notification')
    shutil.copytree(STOCK, note)
    if patched:
        sys.path.insert(0, os.path.join(ROOT, 'tools'))
        r = subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'notification_patch.py'), note, palette(light),
                            os.path.join(ROOT, 'payload', 'notification')], capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit('patch failed: ' + r.stderr)
    # The language style is a file import on the set; the apps' own dummy copy has the same shape.
    shutil.copy(os.path.join(CENSUS, 'QuickSettings', 'AdaptationLayer', 'Dummy', 'language-style.js'),
                os.path.join(note, 'language-style.js'))
    p = os.path.join(note, 'NotificationStyle.qml')
    with open(p, encoding='utf-8') as f:
        s = f.read()
    s = s.replace('"file:///usr/share/javascript/language-style/language-style.js"', '"language-style.js"')
    with open(p, 'w', encoding='utf-8') as f:
        f.write(s)
    with open(os.path.join(note, 'NLocaleService', 'NLocaleService.qml'), 'w') as f:
        f.write('pragma Singleton\nimport QtQuick 2.4\nQtObject {\n  property string currentLocale: "en-US"\n'
                '  property bool isRTLLocale: false\n  property bool isTallglyph: false\n  property string emptyString: ""\n}\n')
    for name in ('WebOSServices', 'PmLog', 'iLib', 'Eos'):
        shutil.copytree(os.path.join(HERE, 'notify_stubs', name), os.path.join(qml, name))
    shutil.copytree(os.path.join(HERE, 'notify_stubs', 'WebOS'), os.path.join(qml, 'WebOS'))
    host = os.path.join(qml, 'Host.qml')
    with open(host, 'w') as f:
        f.write(HOST_QML)
    return qml, host


HOST_QML = '''import QtQuick 2.4
import WebOSServices 1.0
import "SystemUI/Notification" as N

Item {
    id: screen
    width: 1920; height: 1080
    property url backdrop: ""
    Rectangle { anchors.fill: parent; color: "#0d1020" }
    Image { anchors.fill: parent; source: screen.backdrop; fillMode: Image.PreserveAspectCrop; asynchronous: false }
    N.NotificationView {
        id: view
        objectName: "view"
        style: ({})
        access: ({ alerts: true, toasts: true, pincodePrompts: true })
        systemSettings: ({ powerSuspending: false, isOsdDisplayOff: false, powerOnlyMode: false, substrateMode: false,
                           rollingScreenState: { ui: "full" }, twinWideLauncherMode: false, supportScreenType: "" })
        // What the compositor does with a request: the component gets the keyboard focus.
        onRequestFocus: { console.log("[view] requestFocus " + type); if (component) component.forceActiveFocus() }
        onReleaseFocus: console.log("[view] releaseFocus " + type)
        onOpened: console.log("[view] opened " + type)
        onClosed: console.log("[view] closed " + type)
    }
    property string last: ""
    Timer {
        interval: 250; repeat: true; running: true
        onTriggered: { var t = screen.state(); if (t !== screen.last) { screen.last = t; console.log("[state] " + t) } }
    }
    function publish(kind, json) {
        var method = { toast: "/getToastNotification", alert: "/getAlertNotification", input: "/getInputAlertNotification",
                       pin: "/getPincodePromptNotification" }[kind]
        console.log("[publish] " + kind + " to " + LS.deliver("luna://com.webos.notification", method, json) + " service(s)")
    }
    // The property of the first item named `name` in the drawn tree, however it was reparented.
    function probe(name, prop) {
        function walk(item) {
            if (item.objectName === name) return item
            for (var i = 0; i < item.children.length; i++) {
                var found = walk(item.children[i])
                if (found) return found
            }
            return null
        }
        var item = walk(screen)
        return item ? String(item[prop]) : "<no such item>"
    }
    function calls() { return JSON.stringify(LS.calls) }
    function state() {
        return JSON.stringify({ toasts: view.toastCount, alerts: view.alertCount, advanced: view.advancedAlertCount, pin: view.pincodePromptCount,
                                alertActive: view.alertActive, modalAlertActive: view.modalAlertActive, pinActive: view.pincodePromptActive })
    }
}
'''


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--patched', action='store_true', help='run tools/notification_patch.py over the copy first')
    ap.add_argument('--light', action='store_true', help='with --patched: stage with the light material as the palette')
    ap.add_argument('--shot', action='append', default=[], metavar='MS:PATH', help='PNG of the window at MS after start (repeatable); a bare PATH is taken at --wait')
    ap.add_argument('--wait', type=int, default=2500)
    ap.add_argument('--backdrop', help='image drawn behind the view (a screenshot of Home from qmlhost.py)')
    for kind in DEFAULTS:
        ap.add_argument('--' + kind, action='append', default=[], metavar='MS[:JSON]', help='publish a %s at MS (default document if no JSON)' % kind)
    ap.add_argument('--press', action='append', default=[], metavar='MS:KEYS', help='keys sent at MS, comma separated')
    ap.add_argument('--probe', action='append', default=[], metavar='MS:OBJECT:PROP', help='print a property of the item with this objectName at MS, as `[probe] OBJECT.PROP=value`')
    ap.add_argument('--calls', action='store_true', help='at the end print every luna call the view made, as `[calls] JSON`')
    ap.add_argument('--keep', help='keep the staged tree in this directory')
    a = ap.parse_args(argv)
    if not os.path.isdir(STOCK):
        print('needs the census copy of LG\'s notification tree at', STOCK)
        return 2

    from PyQt5 import QtCore, QtGui, QtQml, QtQuick
    work = os.path.abspath(a.keep) if a.keep else tempfile.mkdtemp(prefix='notify-')
    if a.keep and os.path.exists(work):
        shutil.rmtree(work)
    os.makedirs(work, exist_ok=True)
    qml, host = stage(work, a.patched, a.light)

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
    for n in sorted(os.listdir(os.path.join(ROOT, 'payload', 'fonts'))):
        if n.endswith('.ttf'):
            QtGui.QFontDatabase.addApplicationFont(os.path.join(ROOT, 'payload', 'fonts', n))

    class Comp(QtCore.QObject):
        @QtCore.pyqtSlot('QVariant')
        def setMouseFocus(self, _):
            pass

        @QtCore.pyqtSlot()
        def updateCursorFocus(self):
            pass
    comp = Comp()

    view = QtQuick.QQuickView()
    eng = view.engine()
    eng.addImportPath(qml)
    view.rootContext().setContextProperty('compositor', comp)
    view.setResizeMode(QtQuick.QQuickView.SizeRootObjectToView)
    view.resize(1920, 1080)
    view.setSource(QtCore.QUrl.fromLocalFile(host))
    if view.status() != QtQuick.QQuickView.Ready:
        for e in view.errors():
            print('ERROR', e.toString())
        return 1
    root = view.rootObject()
    if a.backdrop:
        root.setProperty('backdrop', QtCore.QUrl.fromLocalFile(os.path.abspath(a.backdrop)))
    view.show()

    def publish(kind, text):
        QtCore.QMetaObject.invokeMethod(root, 'publish', QtCore.Q_ARG('QVariant', kind), QtCore.Q_ARG('QVariant', text))

    serial = [100]

    def doc(kind, js):
        d = json.loads(js) if js else json.loads(json.dumps(DEFAULTS[kind]))
        if not js:
            serial[0] += 1
            info = d.get('alertInfo') or d.get('pincodePromptInfo')
            if info is not None:
                info['timestamp'] = serial[0]
            else:
                d['message'] = d['message'] + ' (%d)' % serial[0]
        return json.dumps(d)

    for kind in DEFAULTS:
        for spec in getattr(a, kind):
            ms, _, js = spec.partition(':')
            QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda k=kind, j=js: publish(k, doc(k, j)))

    def press(keys):
        for name in keys.split(','):
            if not name:
                continue
            key = getattr(QtCore.Qt, KEYS[name.strip().upper()])
            for typ in (QtCore.QEvent.KeyPress, QtCore.QEvent.KeyRelease):
                app.sendEvent(view, QtGui.QKeyEvent(typ, key, QtCore.Qt.NoModifier))
    for spec in a.press:
        ms, _, keys = spec.partition(':')
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda k=keys: press(k))

    def probe(name, prop):
        ok = QtCore.QMetaObject.invokeMethod(root, 'probe', QtCore.Qt.DirectConnection, QtCore.Q_RETURN_ARG('QVariant'), QtCore.Q_ARG('QVariant', name), QtCore.Q_ARG('QVariant', prop))
        print('[probe] %s.%s=%s' % (name, prop, ok), flush=True)
    probes = []
    for spec in a.probe:
        ms, name, prop = spec.split(':', 2)
        probes.append(int(ms))
        QtCore.QTimer.singleShot(int(ms), QtCore.Qt.PreciseTimer, lambda n=name, p=prop: probe(n, p))

    def shot(path):
        img = view.grabWindow()
        img.save(path)
        print('shot %s %dx%d' % (path, img.width(), img.height()), flush=True)

    shots = []
    for spec in a.shot:
        ms, sep, path = spec.partition(':')
        if sep and ms.isdigit():
            shots.append((int(ms), path))
        else:
            shots.append((a.wait, spec))
    for ms, path in shots:
        QtCore.QTimer.singleShot(ms, QtCore.Qt.PreciseTimer, lambda p=path: shot(p))

    def finish():
        # one timer, not two 100 ms apart: Qt's coarse timers may fire in either order within their 5 % slack, and quitting first loses the calls
        if a.calls:
            print('[calls] %s' % QtCore.QMetaObject.invokeMethod(root, 'calls', QtCore.Qt.DirectConnection, QtCore.Q_RETURN_ARG('QVariant')), flush=True)
        app.quit()
    QtCore.QTimer.singleShot(max([a.wait] + probes + [m for m, _ in shots]) + 200, QtCore.Qt.PreciseTimer, finish)
    app.exec_()

    own = [m for m in messages if m[0] in (1, 2, 3) and not IGNORE.search(m[3])]
    print('%d warning(s) from the tree' % len(own))
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    return 1 if own else 0


if __name__ == '__main__':
    sys.exit(main())
