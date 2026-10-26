#!/usr/bin/env python3
"""Offline preview of LG's Search app (`com.webos.app.voice`): the search field with its keyword rows, the auto-keyword list, the notice
with its button, the option popup and the confirm popup, stock or restyled by `tools/voice_patch.py`.

    python3 tests/qmlhost/voice.py --scene search|search-focus|auto|keywords|noti|option|confirm|switch
        [--patched] [--light] [--text PERCENT] [--audit [KINDS]] [--shot out.png]
        [--wait MS] [--backdrop IMG] [--press MS:KEY[,KEY...]] [--probe MS:OBJECT:PROP] [--keep DIR]

LG's own QML, copied from the set (`/data/home/lgtv/uiinv/voice/qml`, 202 files), is loaded here as it is: its `Style.qml` is the real style
sheet, `StringSet.qml` the real string table, `GlobalVars.qml` and every `Service` wrapper the real ones. What is native or is the rest of
the app's wiring is a stand-in, written the way Main.qml and MainView.qml offer it to the views: the root `main` (screen size, the
`getRelative*` helpers, the four font families), `mainView`, `models`, `inputManager`, `voiceHintManager`, `ttsManager`, `util`, and
`Service`, which answers LG's search server with a fixed list of keywords. The picture files of the app (`Views/images`) are not in the
census copy; where the views would draw one, the host leaves the picture out.

--patched runs `tools/voice_patch.py` over the staged copy first, exactly as `tools/voice.sh stage` does on the set. Qt is the PyQt5
wheel's 5.15 on Xvfb with software GL: this finds load errors and shows what a screen looks like, and measures nothing. LG's own fonts are
not on this machine, so the stock render falls back to a default face; the patched one draws Manrope from payload/fonts. `--text 200`
doubles every size the views take from `getRelativeFontSize` and `--audit` runs tests/qmlhost/layoutaudit.py on the picture (`overlap`
unless kinds are named): findings are printed as `[layout N%] ...` and make the exit status 1.

Exit status 0 when nothing from the tree warned, 1 otherwise, 2 when LG's copy is not on this machine.
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
STOCK_CANDIDATES = ['/data/home/lgtv/uiinv/voice/qml', '/tmp/tvsrc/apps/com.webos.app.voice/qml']
STOCK = next((p for p in STOCK_CANDIDATES if os.path.isdir(p)), STOCK_CANDIDATES[0])
FONTS = os.path.join(ROOT, 'payload', 'fonts')

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)

# Qt 5.15 deprecates the Connections syntax LG's tree (written for 5.12) uses throughout. LG's own bug, on the set too: SearchView's
# `KeyNavigation.up` returns nothing while the dim backdrop is not up, and the auto-keyword delegates read their `ListView` while the model is cleared. The picture files are not in the census copy:
# an Image whose file is absent says so, which is not the tree's fault.
IGNORE = re.compile(r'Implicitly defined onFoo|Unable to assign \[undefined\] to QQuickItem\*|Cannot read property .(?:left|right). of null|QML Image: Cannot open|QML Image: Failed to get image|Binding loop detected for property "(?:implicit|)[Ww]idth"')

KEYS = {'LEFT': 'Key_Left', 'RIGHT': 'Key_Right', 'UP': 'Key_Up', 'DOWN': 'Key_Down', 'OK': 'Key_Return', 'BACK': 'Key_Escape'}

# One entry per screen: the file the host loads, the objects it needs, what it does once loaded.
SCENES = {
    # the field with the keyword rows under it, the field focused (LG's first state)
    'search': {'file': 'Views/SearchView.qml', 'keywords': True},
    # the same with the focus on the second keyword
    'search-focus': {'file': 'Views/SearchView.qml', 'keywords': True, 'press': ((1200, 'DOWN'), (1500, 'DOWN'), (1800, 'RIGHT'))},
    # text typed: the list of auto keywords under the field, one focused
    'auto': {'file': 'Views/SearchView.qml', 'keywords': False, 'typed': 'the great', 'auto': True, 'press': ((900, 'DOWN'),)},
    'keywords': {'file': 'Views/SearchView.qml', 'keywords': True, 'press': ((1200, 'DOWN'), (1500, 'DOWN'))},
    # LG's notice under the field, with the button it offers
    'noti': {'file': 'Views/SearchView.qml', 'keywords': False, 'noti': True},
    'option': {'file': 'Views/PopupView/optionPopup.qml', 'popup': True},
    # the confirm popup: a question and two buttons (LG's own strings for the music-search question are not on this machine)
    'confirm': {'file': 'Views/PopupView/ConfirmPopup.qml', 'popup': True,
                'props': {'message': 'Do you want to go to "Network Settings" now to check the connection?',
                          'buttons': [{'label': 'Yes', 'onclickAction': 'networkSettings'}, {'label': 'No', 'onclickAction': ''}]}},
    'switch': {'file': 'Views/PopupView/optionPopup.qml', 'popup': True},
}

HOST_QML = '''import QtQuick 2.12
import "qml" as V
import "qml/Utils" as U
import "qml/ConfigManager" as C

Item {
    id: main
    width: 1920; height: 1080
    property url backdrop: ""
    property url screenFile: ""
    property bool focused: true

    // ---- what Main.qml offers every view
    property real textScale: %(scale)s
    property bool isPortraitMode: false
    property bool supportPortrait: false
    property string orientation: "landscape"
    property bool isRTL: false
    property bool highContrast: false
    property string lastInputType: "key"
    property string loginId: "nphil"
    property var loginUserData: ({profileBg: "#359BDB", iconNick: "N"})
    property bool generalTermsAllowed: true
    property bool isInternetConnectionAvailable: true
    property bool supportAIKeyword: true
    property var allCategoryDomains: ["epg_onair", "tvshow", "movie", "app"]
    property var genre_navigator: ({genre: [{genre: "Drama", image: ""}, {genre: "Comedy", image: ""}, {genre: "Sports", image: ""}]})
    property string debugstatus: ""
    property string serviceCountry: "US"
    property bool supportBNO: false
    property bool focusEmphasis: false
    property bool supportThinQtvNlp: false
    property string voiceModel: "thinQtvStt"
    property bool supportBnoModel: false
    property bool isPartialModeForAtelier: false
    property string cardTypeForegroundAppId: "com.webos.app.home"
    property bool isFirstLangLoaded: true
    property string imagesPath: "file://%(images)s/"
    property string mp3Path: ""
    property int _KEY_BACK: Qt.Key_Escape
    property int _KEY_SCREENREMOTE: 0x01200032
    property int _KEY_VOICESEARCH: 0x01200034
    property string normal300_fontFamily: "LG Smart UI"
    property int    normal300_fontWeight: Font.Light
    property string normal400_fontFamily: "LG Smart UI"
    property int    normal400_fontWeight: Font.Medium
    property bool   normal400_fontItalic: false
    property string normal600_fontFamily: "LG Smart UI"
    property int    normal600_fontWeight: Font.DemiBold
    property bool   normal600_fontItalic: false
    property string normal700_fontFamily: "LG Smart UI"
    property int    normal700_fontWeight: Font.Bold
    property double _widthRatio: 1
    property double _heightRatio: 1
    function getRelativeWidth(width) { return width }
    function getRelativeHeight(height) { return height }
    function getRelativeFontSize(pixelSize) { return Math.round(pixelSize * textScale) }

    property var stringSet: U.StringSet {
        _transObj: QtObject { property string emptyString: "" }
        _isRTL: false
    }
    property var globalVars: C.GlobalVars { isFullMode: true }
    // The marquee of a text longer than its field waits this long before it scrolls; a picture taken mid-scroll holds words that are
    // clipped on the set and not here, so the pause outlasts the run.
    property var voiceStyle: V.Style { pauseTextMarqueeDuration: 600000 }
    property var util: QtObject {
        function printConsoleLog(m) {}
        function printInfoLog(a, b) {}
        function printNormalLog(a, b) {}
        function printErrorLog(a, b) {}
        function isChangedObjArr(a, b) { return false }
        function launchApp() { console.log("[luna] launchApp " + JSON.stringify(arguments)) }
        function powerOff() {}
    }
    property var ttsManager: QtObject { function speak(t, c) {} function stopSpeaking(x) {} }
    property var voiceHintManager: QtObject {
        property bool multipleMatchSession: false
        function addIntentId(a, b) { return "intent" }
        function removeIntentId(a) { return null }
        function addItem(a, b, c) { return "item" }
        function removeItem(a, b) {}
        function updateItem(a, b, c) {}
        function setVisible(a, b) {}
        function setScroll(a, b) { return "" }
        function removeScroll(a) { return "" }
        function finishMultipleSession(a, b) {}
    }
    property var inputManager: QtObject {
        property var dbHandler: QtObject { property bool complete: true; function mergeDB() {} }
        property var searchHandler: QtObject { function searchText(a, t) { console.log("[luna] searchText " + t) } }
        property var errorHandler: QtObject { function handleErrorCases(a, b) {} }
    }
    property var models: QtObject {
        property var contentsListModel: QtObject { property var modelWrapper: QtObject { property int count: 0 } }
        function initialize() {}
    }
    property var mainView: QtObject {
        property var resultsViewObject: null
        property var searchView: main.keywordsStub
        property var emptyStatus: QtObject { property int nlpSttStatus: 0; property int localStatus: 0 }
        function getCurrentResultsViewType() { return "" }
        function requestDeactivateApp(t) { console.log("[luna] deactivate " + t) }
        function stopTimeOut() {}
        function startAppTimer(s) {}
        function createPopupView(t, o) {}
        function destroyPopupView(b) {}
    }
    readonly property int _EMPTYSTATUS_NULL: 0
    property var keywordsStub: QtObject {
        property var keywordsView: QtObject { function clearPreviousKeywordsList() {} function updateKeywordsModel(x) {} }
    }

    Rectangle { anchors.fill: parent; color: "#0d1020" }
    Image { anchors.fill: parent; source: main.backdrop; fillMode: Image.PreserveAspectCrop; asynchronous: false }
    property var screenProps: ({})
    // As Main.qml does when the locale arrives: the families come out of LG's language table.
    function load() { voiceStyle.reloadFontFamily("en-US"); loader.setSource(main.screenFile, main.screenProps) }
    Loader {
        id: loader
        objectName: "loader"
        anchors.fill: parent
        focus: true
        onLoaded: {
            %(wire)s
            if (main.focused) item.forceActiveFocus()
        }
    }
    Timer {
        interval: 300; running: true
        onTriggered: {
            var item = loader.item
            %(later)s
        }
    }
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
        var item = walk(main)
        if (!item) return "<no such item>"
        var v = item
        var path = String(prop).split(".")
        for (var j = 0; j < path.length && v !== undefined && v !== null; j++) v = v[path[j]]
        return String(v)
    }
    // Runs `fn(item)` on the first item named `name`.
    function with_(name, fn) {
        function walk(item) {
            if (item.objectName === name) return item
            for (var i = 0; i < item.children.length; i++) {
                var found = walk(item.children[i])
                if (found) return found
            }
            return null
        }
        var item = walk(main)
        if (item) fn(item)
        return !!item
    }
}
'''

# What the search server answers (`search/popular_keyword`, `search/auto_keyword`): the shape LG's SdxService hands the views.
POPULAR = [{'keyword': k, 'related_keywords': r, 'genre_code1': 'g', 'program_id': str(i)} for i, (k, r) in enumerate(
    [('The Great British Bake Off', ['Bread week', 'Final']), ('Severance', ['Season 2']), ('Planet Earth III', []), ('Formula 1', ['Race'])])]
AUTO = [{'keyword': k} for k in ('the great british bake off', 'the great escape', 'the great gatsby', 'the great wall')]

SERVICE_QML = '''import QtQuick 2.12
QtObject {
    id: svc
    property string appId: ""
    property string service: ""
    property string method: ""
    signal callSuccess(var response)
    signal callFailure(var response)
    signal response(string method, string payload)
    function cancel() {}
    function call(uri, method, payload) {}
    function callService(params) {
        var url = params && params.url ? params.url : ""
        var results = []
        if (url === "search/popular_keyword") results = %(popular)s
        else if (url === "search/auto_keyword") results = %(auto)s
        else return
        reply.pending = JSON.stringify({response: JSON.stringify({response: {results: results}})})
        reply.restart()
    }
    property Timer reply: Timer {
        property string pending: ""
        interval: 1
        onTriggered: { var p = JSON.parse(pending); svc.callSuccess({serverResponse: p}) }
    }
}
'''

# LG's language table, for a Latin locale (the app takes it from /usr/share/javascript/language-style/language-style.js).
LANGUAGE_STYLE = '''.pragma library
function getStyle(locale) {
    function face(weight, qml) { return {fontFamily: ["LG Smart UI"], qmlWeight: qml} }
    return {info: {languageClass: "latin"}, normal300: face(300, 25), normal400: face(400, 57), normal600: face(600, 63), normal700: face(700, 75)}
}
'''


def palette(light):
    """The palette `tools/voice.sh stage` hands the patch: tokens.py's `qml` output. The shipped one always comes from the dark material;
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


def stage(dest, patched, light=False):
    """A copy of LG's tree under `dest/qml`, patched the way the set's staging does when asked."""
    qml = os.path.join(dest, 'qml')
    shutil.copytree(STOCK, qml)
    if patched:
        r = subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'voice_patch.py'), qml, palette(light), FONTS],
                           capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit('patch failed: ' + r.stderr)
    return qml


def wire(spec):
    """What the host does to the screen once it is loaded, and 300 ms later (the keyword row is a Loader of its own)."""
    now, later = [], []
    if spec['file'].endswith('SearchView.qml'):
        now.append('main.mainView.searchView = item')
        later.append('item.initialize(true, true)')
        if spec.get('keywords'):
            later.append('item.keywordsView.finalize()')
            later.append('item.keywordsView.keywordsMode(true)')
            later.append('item.keywordsView.setPreviousKeywordsList(["bake off", "severance season two"])')
        if spec.get('typed'):
            later.append('item.updateUserText(%s)' % json.dumps(spec['typed']))
        if spec.get('auto'):
            later.append('item.textInputMode(true)')
        if spec.get('noti'):
            later.append('item.showNotiView("No results found.", "Go to Internet", "", "com.webos.app.browser", {})')
    return '\n            '.join(now), '\n            '.join(later)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--scene', choices=sorted(SCENES), default='search')
    ap.add_argument('--patched', action='store_true', help='run tools/voice_patch.py over the copy first')
    ap.add_argument('--light', action='store_true', help='with --patched: stage with the light material as the palette')
    ap.add_argument('--shot', action='append', default=[], metavar='MS:PATH', help='PNG of the window at MS after start (repeatable); a bare PATH is taken at --wait')
    ap.add_argument('--wait', type=int, default=3000)
    ap.add_argument('--backdrop', help='image drawn behind the screen')
    ap.add_argument('--press', action='append', default=[], metavar='MS:KEYS', help='keys sent at MS, comma separated')
    ap.add_argument('--probe', action='append', default=[], metavar='MS:OBJECT:PROP', help='print a property of the item with this objectName at MS, as `[probe] OBJECT.PROP=value`')
    ap.add_argument('--keep', help='keep the staged tree in this directory')
    ap.add_argument('--text', type=int, default=100, help='text size in percent (200 doubles every size the views take from getRelativeFontSize)')
    ap.add_argument('--audit', metavar='KINDS', nargs='?', const='overlap', help='run the layout audit of the finished picture (tests/qmlhost/layoutaudit.py) for these kinds, comma separated; `all` for every kind; findings are warnings')
    a = ap.parse_args(argv)
    if not os.path.isdir(STOCK):
        print("needs the census copy of LG's Search QML at", STOCK)
        return 2

    from PyQt5 import QtCore, QtGui, QtQml, QtQuick
    work = os.path.abspath(a.keep) if a.keep else tempfile.mkdtemp(prefix='voice-')
    if a.keep and os.path.exists(work):
        shutil.rmtree(work)
    os.makedirs(work, exist_ok=True)
    qml = stage(work, a.patched, a.light)
    spec = SCENES[a.scene]

    # LG's own `Service` and its language table, as the views would find them on the set.
    stubs = os.path.join(work, 'stubs')
    os.makedirs(os.path.join(stubs, 'WebOSServices'))
    with open(os.path.join(stubs, 'WebOSServices', 'qmldir'), 'w') as f:
        f.write('module WebOSServices\nService 1.0 Service.qml\n')
    with open(os.path.join(stubs, 'WebOSServices', 'Service.qml'), 'w') as f:
        f.write(SERVICE_QML % {'popular': json.dumps(POPULAR), 'auto': json.dumps(AUTO)})
    os.makedirs(os.path.join(work, 'emulator', 'modules'))
    with open(os.path.join(work, 'emulator', 'modules', 'language-style.js'), 'w') as f:
        f.write(LANGUAGE_STYLE)
    images = os.path.join(work, 'images')
    os.makedirs(images)
    host = os.path.join(work, 'Host.qml')
    with open(host, 'w') as f:
        now, later = wire(spec)
        f.write(HOST_QML % {'scale': a.text / 100.0, 'images': images, 'wire': now, 'later': later})

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
    view.rootContext().setContextProperty('DESKTOP_BUILD', True)
    view.setResizeMode(QtQuick.QQuickView.SizeRootObjectToView)
    view.resize(1920, 1080)
    view.setSource(QtCore.QUrl.fromLocalFile(host))
    if view.status() != QtQuick.QQuickView.Ready:
        for e in view.errors():
            print('ERROR', e.toString())
        return 1
    root = view.rootObject()
    root.setProperty('screenFile', QtCore.QUrl.fromLocalFile(os.path.join(qml, spec['file'])))
    root.setProperty('screenProps', spec.get('props', {}))
    if a.backdrop:
        root.setProperty('backdrop', QtCore.QUrl.fromLocalFile(os.path.abspath(a.backdrop)))
    view.show()
    view.requestActivate()
    QtCore.QMetaObject.invokeMethod(root, 'load', QtCore.Qt.DirectConnection)

    def call(fn, *args):
        return QtCore.QMetaObject.invokeMethod(root, fn, QtCore.Qt.DirectConnection, QtCore.Q_RETURN_ARG('QVariant'),
                                               *[QtCore.Q_ARG('QVariant', x) for x in args])

    def press(keys):
        for name in keys.split(','):
            if not name:
                continue
            key = getattr(QtCore.Qt, KEYS[name.strip().upper()])
            for typ in (QtCore.QEvent.KeyPress, QtCore.QEvent.KeyRelease):
                app.sendEvent(view, QtGui.QKeyEvent(typ, key, QtCore.Qt.NoModifier))

    for ms, keys in spec.get('press', ()):
        QtCore.QTimer.singleShot(ms, lambda k=keys: press(k))
    for s in a.press:
        ms, _, keys = s.partition(':')
        QtCore.QTimer.singleShot(int(ms), lambda k=keys: press(k))

    def probe(name, prop):
        print('[probe] %s.%s=%s' % (name, prop, call('probe', name, prop)), flush=True)
    probes = []
    for s in a.probe:
        ms, name, prop = s.split(':', 2)
        probes.append(int(ms))
        QtCore.QTimer.singleShot(int(ms), lambda n=name, p=prop: probe(n, p))

    found_layout = []

    def shot(path):
        img = view.grabWindow()
        img.save(path)
        print('shot %s %dx%d' % (path, img.width(), img.height()), flush=True)
        if a.audit:
            import layoutaudit
            kinds = None if a.audit == 'all' else [k for k in a.audit.split(',') if k]
            for line in layoutaudit.lines(layoutaudit.audit(root, 1920, 1080, img, kinds), 'layout %d%%' % a.text):
                found_layout.append(line)
                print(line, flush=True)

    shots = []
    for s in a.shot:
        ms, sep, path = s.partition(':')
        shots.append((int(ms), path) if sep and ms.isdigit() else (a.wait, s))
    for ms, path in shots:
        QtCore.QTimer.singleShot(ms, lambda p=path: shot(p))

    end = max([a.wait] + probes + [m for m, _ in shots])
    QtCore.QTimer.singleShot(end + 100, lambda: closing.__setitem__(0, True))
    QtCore.QTimer.singleShot(end + 200, app.quit)
    app.exec_()

    own = [m for m in messages if m[0] in (1, 2, 3) and not IGNORE.search(m[3])]
    print('%d warning(s) from the tree' % len(own))
    if found_layout:
        print('%d layout finding(s)' % len(found_layout))
    if not a.keep:
        shutil.rmtree(work, ignore_errors=True)
    return 1 if own or found_layout else 0


if __name__ == '__main__':
    sys.exit(main())
