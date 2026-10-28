#!/usr/bin/env python3
"""Offline render of the two blocks `tools/splash_patch.py` puts into LG's launch splash and launch spinner views.

    python3 tests/qmlhost/splashview.py card|spinner [--name Crunchyroll] [--icon PATH] [--word] [--shot PATH] [--wait MS]

The blocks (payload/splash/card.qml, spinner.qml) are cut out of LG's views, so they run here inside a stand-in for the part of the view
they read: `root` (the view: `lumaCard`, `lumaName`, `lumaIcon`, `isOpen`, its size), `wiper` (`state`). `card` draws the launch screen as an
app with no picture of its own gets it; `spinner` the dimmed screen with the ring (`--word`: a second later, with "Loading"). The look is
what the host can say; where the blocks sit in LG's files, and that those files still take the patch, is tests/test_splash_patch.py.
Qt is the PyQt5 wheel's 5.15 on an X display with software GL. Exit status 0 when nothing warned, 1 otherwise.
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)
sys.path.insert(0, os.path.join(ROOT, 'tests'))
import xdisplay  # noqa: E402
if not os.environ.get('DISPLAY') and xdisplay.DISPLAY:
    os.environ['DISPLAY'] = xdisplay.DISPLAY

HOST = '''import QtQuick 2.4

Item {
    id: root
    width: 1920; height: 1080
    property bool lumaCard: true
    property string lumaName: %(name)s
    property url lumaIcon: "%(icon)s"
    property bool isOpen: true
    // what LG's view has behind the block: the screen the app was launched from
    Image { anchors.fill: parent; source: "%(behind)s"; fillMode: Image.PreserveAspectCrop }
    QtObject { id: wiper; property string state: "open" }
%(block)s
}
'''


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('which', choices=('card', 'spinner'))
    ap.add_argument('--name', default='Crunchyroll')
    ap.add_argument('--icon', default=os.path.join(ROOT, 'docs', 'mocks', 'img', 'apps', 'crunchyroll.png'))
    ap.add_argument('--behind', default=os.path.join(ROOT, 'docs', 'mocks', 'img', 'still.jpg'))
    ap.add_argument('--shot')
    ap.add_argument('--wait', type=int, default=1800)
    a = ap.parse_args(argv)
    from PyQt5 import QtCore, QtGui, QtQuick
    pal = json.loads(subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'tokens.py'), os.path.join(ROOT, 'payload', 'theme', 'theme.json'),
                                     '/nonexistent', 'qml'], capture_output=True, text=True, check=True).stdout)
    text = open(os.path.join(ROOT, 'payload', 'splash', 'card.qml' if a.which == 'card' else 'spinner.qml')).read()
    block = re.sub(r'@@(\w+)@@', lambda m: str(pal[m.group(1)]), text)
    work = tempfile.mkdtemp(prefix='splash-')
    host = os.path.join(work, 'Host.qml')
    with open(host, 'w') as f:
        f.write(HOST % {'name': json.dumps(a.name), 'icon': 'file://' + a.icon, 'behind': 'file://' + a.behind, 'block': block})
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
        app.quit()
    QtCore.QTimer.singleShot(a.wait, QtCore.Qt.PreciseTimer, finish)
    app.exec_()
    bad = [m for m in messages if m[0] in (1, 2, 3)]
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
