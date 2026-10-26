#!/usr/bin/env python3
"""Offline preview of LG's on-screen keyboard (maliit's `view-global` plugin, the one a Latin locale uses): the letter keys, the symbol
keys, the number pad, the candidate row and the HID candidate bar, stock or restyled by `tools/maliit_patch.py`.

    python3 tests/qmlhost/maliit.py --scene letters|shift|symbols|numbers|candidates|hid|focus-enter|focus-shift|focus-delete|focus-space|focus-language
        [--patched] [--light] [--text PERCENT] [--audit [KINDS]] [--shot out.png]
        [--wait MS] [--backdrop IMG] [--press MS:KEY[,KEY...]] [--probe MS:OBJECT:PROP] [--keep DIR]

LG's own QML, copied from the set (`/usr/lib/maliit/plugins/qml`, 85 files: `/data/home/lgtv/uiinv/maliit/qml`, or the archive in
docs/research/round9/r11-2026-10-26/), is loaded here as it is: `view-global/main.qml` is the plugin's root item, run the way the plugin
runs it. What is native stays a stand-in written from how the QML uses it: `PluginProxy` (the plugin object: content type, language
layout, panel size, the keys' release signal), `Prediction` (the candidate list), `PmLog`, `WebOSServices.Service` (which answers the
five luna calls main.qml makes), the `com.meego.maliit` enums and LG's language-style library. The three things the QML reads from the
set's file system are redirected inside the staged copy, as a text edit and nothing else: the library's absolute path, and the
`qrc:///images-2k/` pictures (compiled into the plugin on the set) to the same files on disk.

--patched runs `tools/maliit_patch.py` over the staged copy first, exactly as `tools/maliit.sh stage` does on the set. Qt is the PyQt5
wheel's 5.15 on Xvfb with software GL: this finds load errors and shows what the keyboard looks like, and measures nothing. LG's own
fonts are not on this machine, so the stock render falls back to a default face; the patched one draws Manrope from payload/fonts.
`--text 200` doubles every `fontSize` of the style sheets and `--audit` runs tests/qmlhost/layoutaudit.py on the picture (`overlap`
unless kinds are named): findings are printed as `[layout N%] ...` and make the exit status 1. The key under LG's focus rectangle is a
run of words behind another (the rectangle is opaque and hides it): that pair is not a finding, and the host says so by reading the
rectangle's colour (`[probe]`-style, in the test), not by looking away.

Exit status 0 when nothing from the tree warned, 1 otherwise, 2 when LG's copy is not on this machine.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..', '..'))
ARCHIVE = os.path.join(ROOT, 'docs', 'research', 'round9', 'r11-2026-10-26', 'maliit-qml.tgz')
FONTS = os.path.join(ROOT, 'payload', 'fonts')

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)

# Qt 5.15 deprecates the Connections syntax LG's tree (written for 5.12) uses throughout. LG's own bugs, on the set too, and the same
# in the patched tree (`[known] N` counts them: a patch that adds one changes the number): `ButtonBase.radius` reads the `radius` of a
# style that has none, the language key's TTS string and the focus rectangle's `focusedItem` are assigned `undefined`, the page-count
# row reads `fontfamily`, which nothing defines.
KNOWN = re.compile(r'ButtonBase\.qml:30:5: Unable to assign \[undefined\] to int|LeftFunctionArea\.qml:165:13: Unable to assign \[undefined\] to QString|'
                   r'FocusPanel\.qml:27:5: Unable to assign \[undefined\] to QQuickItem\*|PredictionPanel\.qml:128: ReferenceError: fontfamily is not defined')
IGNORE = re.compile(r'Implicitly defined onFoo|Binding loop detected for property "(?:implicit|)[Ww]idth"')

# The remote's OK reaches the keyboard as Qt.Key_Execute (ButtonBase.qml's `Keys.onPressed`), not Key_Return.
KEYS = {'LEFT': 'Key_Left', 'RIGHT': 'Key_Right', 'UP': 'Key_Up', 'DOWN': 'Key_Down', 'OK': 'Key_Execute', 'BACK': 'Key_Escape'}

# Maliit::TextContentType and Maliit::EnterKeyType of libmaliit (what `com.meego.maliit`'s `Maliit` carries into QML).
FREE_TEXT, NUMBER, PHONE_NUMBER, EMAIL, URL = 0, 1, 2, 3, 4
DEFAULT_ENTER, SEARCH_ENTER = 0, 5

# One entry per screen: the plugin's state when it is shown, and the keys the host sends once it stands (`press`: ms after it does).
SCENES = {
    'letters': {},
    'shift': {'press': ((300, 'DOWN,LEFT,LEFT,LEFT,LEFT,LEFT'), (900, 'OK'))},
    'symbols': {'press': ((300, 'LEFT,LEFT,LEFT,LEFT,LEFT'), (900, 'OK'))},
    'numbers': {'content': NUMBER},
    'numbers-symbols': {'content': NUMBER, 'press': ((300, 'LEFT,LEFT,LEFT,LEFT,LEFT'), (900, 'OK'))},
    'candidates': {'words': ['the', 'then', 'there', 'these', 'they', 'think'], 'enter': SEARCH_ENTER},
    'candidates-focus': {'words': ['the', 'then', 'there', 'these', 'they', 'think'], 'press': ((300, 'UP,UP,UP,UP'),)},
    'hid': {'source': 0, 'words': ['the', 'then', 'there', 'these', 'they']},
    'focus-enter': {'enter': SEARCH_ENTER, 'press': ((300, 'RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,UP'),)},
    'focus-shift': {'press': ((300, 'DOWN,LEFT,LEFT,LEFT,LEFT,LEFT'),)},
    'focus-delete': {'press': ((300, 'RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,UP,UP'),)},
    'focus-space': {'press': ((300, 'DOWN,DOWN'),)},
    'focus-language': {'press': ((300, 'UP,UP,LEFT,LEFT,LEFT,LEFT,LEFT'),)},
    'focus-key': {'press': ((300, 'DOWN,RIGHT,RIGHT'),)},
    'focus-voice': {'press': ((300, 'RIGHT,RIGHT,RIGHT,RIGHT,RIGHT,RIGHT'),)},
}

# The English layout the plugin hands over (`PluginProxy.languageChanged`'s `data`): four rows of ten, as on the set.
ROWS = [('1234567890', '!@#$%^&*()'), ('qwertyuiop', 'QWERTYUIOP'), ("asdfghjkl?", 'ASDFGHJKL!'), ('@zxcvbnm,.', '#ZXCVBNM;:')]

HOST_QML = '''import QtQuick 2.12
import QtQuick.Window 2.2

Item {
    id: screen
    width: 1920; height: 1080
    property url backdrop: ""
    property url keyboardFile: ""
    property int panelHeight: PluginProxy.panelH
    property var spec: ({})

    Rectangle { anchors.fill: parent; color: "#0d1020" }
    Image { anchors.fill: parent; source: screen.backdrop; fillMode: Image.PreserveAspectCrop; asynchronous: false }
    // The compositor puts the plugin's window at the bottom of the screen, as tall as the panel size the plugin reported.
    Loader {
        id: loader
        objectName: "loader"
        x: 0
        y: screen.height - screen.panelHeight
        focus: true
        onLoaded: item.forceActiveFocus()
    }
    function load() { loader.source = screen.keyboardFile }
    // What the plugin's C++ is connected to: the keys the keyboard sends, as lines the test reads.
    Connections {
        target: loader.item
        onKeyPressed: console.log("[key] " + rawcode + " " + shift + " " + eventType)
        onKeysymPressed: console.log("[keysym] " + keysym + " " + eventType)
        onTextKeyPressed: console.log("[text] " + text)
        onSwitchContext: console.log("[luna] switchContext")
        onActivateSTT: console.log("[luna] stt")
        onClearAllPressed: console.log("[luna] clearAll")
        onHideRequested: console.log("[luna] hide " + reset)
        onMoveCursorPosition: console.log("[luna] cursor " + direction + " " + eventType)
    }
    // What the plugin's C++ does once the window exists: the language and its layout, then the field's kind, then the panel is shown.
    function begin(json) {
        var s = JSON.parse(json)
        PluginProxy.languageCountChanged(s.languages)
        PluginProxy.languageChanged("English", "ENG", "en", s.layout)
        PluginProxy.contentType = s.content
        PluginProxy.enterKeyType = s.enter
        PluginProxy.inputSource = s.source
        Prediction.list = s.words
        PluginProxy.visible = true
    }
    // The keyboard stands when the plugin has been told its size and, in the two states that have a focus box, the focus has landed on a key:
    // a time to wait for would be a guess that a loaded machine breaks, this is what the tests and the renders wait for.
    function ready() {
        var item = loader.item
        if (!item || PluginProxy.panelH <= 0) return false
        if (item.state === "Hid-Prediction") return true
        function find(from, name) {
            if (from.objectName === name) return from
            for (var i = 0; i < from.children.length; i++) {
                var found = find(from.children[i], name)
                if (found) return found
            }
            return null
        }
        var keyboard = find(item, item.state === "Number-Keyboard" ? "numberKeyboard" : "normalKeyboard")
        var box = keyboard ? find(keyboard, "focusRect") : null
        return !!box && box.visible && box.width > 0 && box.focusTxt !== ""
    }
    // The plugin tells the keys that the OK key came up (a key that is `Select` goes back to `Focus` on it).
    function released() { PluginProxy.keyReleased(true) }
    // The property of the first item named `name` in the drawn tree.
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
        if (!item) return "<no such item>"
        var v = item
        var path = String(prop).split(".")
        for (var j = 0; j < path.length && v !== undefined && v !== null; j++) v = v[path[j]]
        return String(v)
    }
}
'''

PLUGINPROXY_QML = '''import QtQuick 2.12
QtObject {
    property int contentType: 0
    property bool hiddenText: false
    property int inputSource: 1
    property int enterKeyType: 0
    property bool visible: false
    property string translatorLocale: "en-US"
    property string emptyString: ""
    property bool inputMax: false
    property bool cursorVisible: false
    property int panelW: 0
    property int panelH: 0
    signal keyReleased(bool isReleased)
    signal translatorChanged()
    signal languageCountChanged(int languageCount)
    signal languageChanged(string language, string label, string langCode, var data)
    signal setDefaultFocus()
    signal showLanguageNotification(string label)
    signal forceFocusTo(string label)
    signal resetRequested()
    function setPanelSize(w, h) { panelW = w; panelH = h }
    function setMovableArea(kind, x, y, w, h) {}
    function onHideRequested(reset) { console.log("[luna] hide") }
}
'''

PREDICTION_QML = '''import QtQuick 2.12
QtObject {
    property var list: []
    property bool active: true
    signal predictionListChanged()
    onListChanged: predictionListChanged()
    function onSelected(word) { console.log("[luna] select " + word) }
}
'''

# The `Maliit` namespace is enums on the set; here a JavaScript resource of the module, which QML reads the same way (`Maliit.NumberContentType`).
MALIIT_JS = '''.pragma library
var FreeTextContentType = %d, NumberContentType = %d, PhoneNumberContentType = %d, EmailContentType = %d, UrlContentType = %d
var DefaultEnterKeyType = %d, SearchEnterKeyType = %d
var MovableAreaLeft = 0, MovableAreaTop = 1, MovableAreaRight = 2, MovableAreaBottom = 3
''' % (FREE_TEXT, NUMBER, PHONE_NUMBER, EMAIL, URL, DEFAULT_ENTER, SEARCH_ENTER)

PMLOG_QML = '''import QtQuick 2.12
QtObject {
    property string context: ""
    function info(id, value) {}
    function warning(id, value) {}
}
'''

# main.qml's five luna subscriptions, answered the way the services answer them (one payload a call, delivered a tick later).
SERVICE_QML = '''import QtQuick 2.12
QtObject {
    id: svc
    property string appId: ""
    property string service: ""
    property string method: ""
    signal response(string method, string payload)
    signal error(string method, string payload)
    function call(uri, m, payload) {}
    function callService(params) {
        var doc = null
        if (method === "getSystemSettings") doc = {returnValue: true, settings: {audioGuidance: "off"}}
        else if (method === "getConfigs") doc = {returnValue: true, configs: {"system.supportVoiceRecognition": true}}
        else if (method === "getForegroundAppInfo") doc = {returnValue: true, foregroundAppInfo: [{appId: "com.webos.app.voice", windowType: "_WEBOS_WINDOW_TYPE_CARD"}]}
        else if (method === "getOrientationInfo") doc = {returnValue: true, orientation: "landscape"}
        else if (method === "getLastInputType") doc = {returnValue: true, lastInputType: "key"}
        else if (method === "speak") console.log("[luna] speak " + JSON.stringify(params))
        if (doc === null) return
        reply.pending = JSON.stringify(doc)
        reply.restart()
    }
    property Timer reply: Timer {
        property string pending: ""
        interval: 1
        onTriggered: svc.response(svc.method, pending)
    }
}
'''

# LG's language-style library as the keyboard's style sheet calls it (`getStyle(locale).item.fontFamily[0]`, `.info.languageClass`).
LANGUAGE_STYLE = '''function getStyle(locale) {
    if (typeof locale !== "string" || !locale.length) throw "invalid data type"
    return {info: {locale: locale, languageClass: "latin"}, item: {fontFamily: ['"LG Smart UI"']}}
}
'''


def census():
    """LG's QML as copied from the set: the workstation's copy, else the archive that is in the repository (extracted next to the work)."""
    p = '/data/home/lgtv/uiinv/maliit/qml'
    return p if os.path.isdir(p) else None


def extract_archive(dest):
    with tarfile.open(ARCHIVE) as t:
        t.extractall(dest)
    return os.path.join(dest, 'qml')


def palette(light):
    """The palette `tools/maliit.sh stage` hands the patch: tokens.py's `qml` output. The shipped one always comes from the dark material;
    `light` swaps the light material in, to see the same files in a theme whose sheet is light."""
    shipped = os.path.join(ROOT, 'payload', 'theme', 'theme.json')
    user, tmp = os.devnull, None
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


def scale_text(qml, percent):
    """Every `property int fontSize` of the style sheets multiplied by `percent` / 100: the 200 % text pass the Lucent rules ask of a screen (the TV has no
    text-size setting; this only shows what a larger face would do to the layout)."""
    k = percent / 100.0
    n = 0
    for base, _, names in os.walk(qml):
        for name in names:
            if not name.endswith('.qml'):
                continue
            path = os.path.join(base, name)
            with open(path, encoding='utf-8') as f:
                text = f.read()
            text, c = re.subn(r'(\bproperty\s+(?:int|real|double)\s+fontSize\s*:\s*)([^\n;{}]+?)(\s*(?:\n|;|\}))', lambda m: '%s(%s) * %s%s' % (m.group(1), m.group(2), k, m.group(3)), text)
            if c:
                n += c
                with open(path, 'w', encoding='utf-8') as f:
                    f.write(text)
    if n < 20:
        raise SystemExit('text scale: only %d fontSize found in the style sheets' % n)


def stage(dest, source, patched, light=False, text=100):
    """A copy of LG's tree under `dest/qml`, patched the way the set's staging does when asked, then the three redirections of the host."""
    qml = os.path.join(dest, 'qml')
    shutil.copytree(source, qml)
    if patched:
        r = subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'maliit_patch.py'), qml, palette(light), FONTS], capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit('patch failed: ' + r.stderr)
    if text != 100:
        scale_text(qml, text)
    lib = os.path.join(dest, 'language-style.js')
    with open(lib, 'w') as f:
        f.write(LANGUAGE_STYLE)
    images = os.path.join(dest, 'img')
    for res in ('4k', '2k', 'hd'):
        out = os.path.join(images, 'images-' + res)
        os.makedirs(out)
        for src in (os.path.join(qml, 'keyboard-common', 'style', 'images', res), os.path.join(qml, 'keyboard-common', 'style-portrait', 'images', res)):
            if os.path.isdir(src):
                for name in os.listdir(src):
                    shutil.copy(os.path.join(src, name), out)
    for base, _, names in os.walk(qml):
        for name in names:
            if not name.endswith('.qml'):
                continue
            path = os.path.join(base, name)
            with open(path, encoding='utf-8') as f:
                body = f.read()
            new = body.replace('"/usr/share/javascript/language-style/language-style.js"', '"%s"' % lib).replace('qrc:///images-', 'file://%s/images-' % images)
            if new != body:
                with open(path, 'w', encoding='utf-8') as f:
                    f.write(new)
    return qml


def layout():
    keys = []
    for plain, shifted in ROWS:
        for a, b in zip(plain, shifted):
            keys.append({'mainChar': a, 'shiftedChar': b, 'mainAudioChar': a, 'shiftedAudioChar': b, 'rawcode': str(ord(a)), 'shiftRawcode': str(ord(b))})
    return {'row': 4, 'column': 10, 'keys': keys, 'supportAccent': False, 'supportAccentEx': False}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--scene', choices=sorted(SCENES), default='letters')
    ap.add_argument('--patched', action='store_true', help='run tools/maliit_patch.py over the copy first')
    ap.add_argument('--light', action='store_true', help='with --patched: stage with the light material as the palette')
    ap.add_argument('--shot', action='append', default=[], metavar='MS:PATH', help='PNG of the window at MS after start (repeatable); a bare PATH is taken at --wait')
    ap.add_argument('--wait', type=int, default=1500, help='ms from the moment the keyboard stands to the end of the run')
    ap.add_argument('--backdrop', help='image drawn behind the keyboard')
    ap.add_argument('--press', action='append', default=[], metavar='MS:KEYS', help='keys sent at MS, comma separated')
    ap.add_argument('--probe', action='append', default=[], metavar='MS:OBJECT:PROP', help='print a property of the item with this objectName at MS, as `[probe] OBJECT.PROP=value`')
    ap.add_argument('--keep', help='keep the staged tree in this directory')
    ap.add_argument('--text', type=int, default=100, help='text size in percent (200 doubles every fontSize of the style sheets)')
    ap.add_argument('--audit', metavar='KINDS', nargs='?', const='overlap', help='run the layout audit of the finished picture (tests/qmlhost/layoutaudit.py) for these kinds, comma separated; `all` for every kind; findings are warnings')
    a = ap.parse_args(argv)
    source = census()
    work = os.path.abspath(a.keep) if a.keep else tempfile.mkdtemp(prefix='maliit-')
    if a.keep and os.path.exists(work):
        shutil.rmtree(work)
    os.makedirs(work, exist_ok=True)
    if source is None:
        if not os.path.isfile(ARCHIVE):
            print("needs LG's maliit QML: /data/home/lgtv/uiinv/maliit/qml or", ARCHIVE)
            return 2
        source = extract_archive(os.path.join(work, 'archive'))
    qml = stage(work, source, a.patched, a.light, a.text)
    spec = SCENES[a.scene]

    stubs = os.path.join(work, 'stubs')
    for rel, text in (('PmLog/qmldir', 'module PmLog\nPmLog 1.0 PmLog.qml\n'), ('PmLog/PmLog.qml', PMLOG_QML),
                      ('com/meego/maliit/qmldir', 'module com.meego.maliit\nMaliit 1.0 Maliit.js\n'), ('com/meego/maliit/Maliit.js', MALIIT_JS),
                      ('WebOSServices/qmldir', 'module WebOSServices\nService 1.0 Service.qml\n'), ('WebOSServices/Service.qml', SERVICE_QML),
                      ('PluginProxy.qml', PLUGINPROXY_QML), ('Prediction.qml', PREDICTION_QML)):
        os.makedirs(os.path.dirname(os.path.join(stubs, rel)), exist_ok=True)
        with open(os.path.join(stubs, rel), 'w') as f:
            f.write(text)
    host = os.path.join(work, 'Host.qml')
    with open(host, 'w') as f:
        f.write(HOST_QML)

    from PyQt5 import QtCore, QtGui, QtQml, QtQuick
    messages = []
    closing = [False]

    def handler(mode, ctx, msg):
        if closing[0]:
            return      # the context properties go null as the window closes
        messages.append((int(mode), ctx.file or '', ctx.line, msg))
        print('[%s] %s:%s: %s' % (['debug', 'warning', 'critical', 'fatal', 'info'][int(mode)], os.path.basename(ctx.file or ''), ctx.line, msg), flush=True)
    QtCore.qInstallMessageHandler(handler)

    app = QtGui.QGuiApplication(sys.argv[:1])
    fmt = QtGui.QSurfaceFormat()
    fmt.setRenderableType(QtGui.QSurfaceFormat.OpenGL)
    fmt.setVersion(2, 1)
    QtGui.QSurfaceFormat.setDefaultFormat(fmt)

    view = QtQuick.QQuickView()
    view.engine().addImportPath(stubs)
    ctx = view.rootContext()
    objects = []
    for name in ('PluginProxy', 'Prediction'):
        comp = QtQml.QQmlComponent(view.engine(), QtCore.QUrl.fromLocalFile(os.path.join(stubs, name + '.qml')))
        obj = comp.create()
        if obj is None:
            for e in comp.errors():
                print('ERROR', e.toString())
            return 1
        ctx.setContextProperty(name, obj)
        objects.append(obj)
    ctx.setContextProperty('speakLanguage', False)
    view.setResizeMode(QtQuick.QQuickView.SizeRootObjectToView)
    view.resize(1920, 1080)
    view.setSource(QtCore.QUrl.fromLocalFile(host))
    if view.status() != QtQuick.QQuickView.Ready:
        for e in view.errors():
            print('ERROR', e.toString())
        return 1
    root = view.rootObject()
    root.setProperty('keyboardFile', QtCore.QUrl.fromLocalFile(os.path.join(qml, 'view-global', 'main.qml')))
    if a.backdrop:
        root.setProperty('backdrop', QtCore.QUrl.fromLocalFile(os.path.abspath(a.backdrop)))
    view.show()
    view.requestActivate()

    def call(fn, *args):
        return QtCore.QMetaObject.invokeMethod(root, fn, QtCore.Qt.DirectConnection, QtCore.Q_RETURN_ARG('QVariant'),
                                               *[QtCore.Q_ARG('QVariant', x) for x in args])

    call('load')
    begin = json.dumps({'layout': layout(), 'content': spec.get('content', FREE_TEXT), 'enter': spec.get('enter', DEFAULT_ENTER),
                        'source': spec.get('source', 1), 'words': spec.get('words', []), 'languages': spec.get('languages', 2)})
    QtCore.QTimer.singleShot(200, lambda: call('begin', begin))

    def press(keys):
        for name in keys.split(','):
            if not name:
                continue
            key = getattr(QtCore.Qt, KEYS[name.strip().upper()])
            for typ in (QtCore.QEvent.KeyPress, QtCore.QEvent.KeyRelease):
                app.sendEvent(view, QtGui.QKeyEvent(typ, key, QtCore.Qt.NoModifier))
            if name.strip().upper() == 'OK':
                call('released')

    def probe(name, prop):
        print('[probe] %s.%s=%s' % (name, prop, call('probe', name, prop)), flush=True)

    found_layout = []

    def shot(path):
        img = view.grabWindow()
        img.save(path)
        print('shot %s %dx%d' % (path, img.width(), img.height()), flush=True)
        if a.audit:
            import layoutaudit
            kinds = None if a.audit == 'all' else [k for k in a.audit.split(',') if k]
            for line in layoutaudit.lines(own_findings(layoutaudit.audit(root, 1920, 1080, img, kinds)), 'layout %d%%' % a.text):
                found_layout.append(line)
                print(line, flush=True)

    def own_findings(findings):
        """LG's focus rectangle is opaque and hides the words of the key it stands on: the audit sees both runs of words, the eye one. The
        pair is dropped only while the rectangle's colour is fully opaque (the test reads that colour on its own)."""
        opaque = QtGui.QColor(call('probe', 'focusRect', 'color')).alphaF() >= 0.999
        if opaque:
            findings = [f for f in findings if not (f['kind'] == 'overlap' and 'focusRect' in f['detail'] + f['item'])]
        # The audit measures the ink between the cap line and the baseline; an underscore and a grave accent are drawn outside that band, so the
        # band holds only the key and reads 1:1 (the same in LG's tree). Those two marks are measured by the pair of colours, in the unit tests.
        return [f for f in findings if not (f['kind'] == 'contrast' and f['item'].endswith(('Text[_]', 'Text[`]')))]

    shots = []
    for s in a.shot:
        ms, sep, path = s.partition(':')
        shots.append((int(ms), path) if sep and ms.isdigit() else (a.wait, s))
    presses = list(spec.get('press', ()))
    for s in a.press:
        ms, _, keys = s.partition(':')
        presses.append((int(ms), keys))
    probes = []
    for s in a.probe:
        ms, name, prop = s.split(':', 2)
        probes.append((int(ms), name, prop))
    end = max([a.wait] + [m for m, _, _ in probes] + [m for m, _ in shots] + [m for m, _ in presses])
    standing = [False]
    waited = [0]

    def schedule():
        """The keyboard stands: every time of the run, in ms, counts from now."""
        for ms, keys in presses:
            QtCore.QTimer.singleShot(ms, lambda k=keys: press(k))
        for ms, name, prop in probes:
            QtCore.QTimer.singleShot(ms, lambda n=name, p=prop: probe(n, p))
        for ms, path in shots:
            QtCore.QTimer.singleShot(ms, lambda p=path: shot(p))
        QtCore.QTimer.singleShot(end + 100, lambda: closing.__setitem__(0, True))
        QtCore.QTimer.singleShot(end + 200, app.quit)

    def poll():
        waited[0] += 50
        if call('ready') is True:
            standing[0] = True
            QtCore.QTimer.singleShot(250, schedule)
            return
        if waited[0] >= 30000:
            print('[host] the keyboard never stood in 30 s', flush=True)
            closing[0] = True
            app.quit()
            return
        QtCore.QTimer.singleShot(50, poll)
    QtCore.QTimer.singleShot(250, poll)
    app.exec_()

    # LG's own `console.warn` lines (the keyboard logs its state that way) are not diagnostics: Qt's own start with the file's URL.
    tree = [m for m in messages if m[0] in (1, 2, 3) and m[3].startswith(('file://', 'qrc:')) and not IGNORE.search(m[3])]
    own = [m for m in tree if not KNOWN.search(m[3])]
    print('[known] %d' % (len(tree) - len(own)))
    print('%d warning(s) from the tree' % len(own))
    if found_layout:
        print('%d layout finding(s)' % len(found_layout))
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    return 1 if own or found_layout or not standing[0] else 0


if __name__ == '__main__':
    sys.exit(main())
