#!/usr/bin/env python3
"""Offline preview of LG's input banners (`com.webos.app.inputcommon`): the pill, the OK banner and the INFO detail card.

    python3 tests/qmlhost/inputcommon.py --scene pill|focused-pill|ok|detail|update [--patched] [--light] [--shot out.png]
        [--wait MS] [--backdrop IMG] [--press MS:KEY[,KEY...]] [--probe MS:OBJECT:PROP] [--keep DIR]

LG's own banner QML, copied from the set (`/data/home/lgtv/uiinv/inputcommon/qml`, 273 files), is loaded here as it
is: its `GlobalStyle.qml` is the real style sheet, its Selectable, ScrollText, ImageButton and banner items are the real
components. Only what the process gets from native code is a stub: `systemProperties`, `interfaces` (the input banner's
controller and the external-device controller), `stringSheet`, `timeManager`, `webOS` and `logger`. The stubs hold what
the set reports for HDMI 2 behind the Denon (device name, CEC label, remote), so the pill shows what Nitin sees.

--patched runs `tools/inputcommon_patch.py` over the staged copy first, exactly as `tools/inputcommon.sh stage` does on
the set. Qt is the PyQt5 wheel's 5.15 on Xvfb with software GL: this finds load errors and shows what a banner looks
like, and measures nothing. LG's own fonts are not on this machine, so the stock render falls back to a default face;
the patched one draws Manrope from payload/fonts.

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
STOCK_CANDIDATES = ['/data/home/lgtv/uiinv/inputcommon/qml', '/tmp/tvsrc/inputcommon/qml']
STOCK = next((p for p in STOCK_CANDIDATES if os.path.isdir(p)), STOCK_CANDIDATES[0])
FONTS = os.path.join(ROOT, 'payload', 'fonts')

os.environ.setdefault('QT_QPA_PLATFORM', 'xcb')
os.environ.setdefault('LIBGL_ALWAYS_SOFTWARE', '1')
os.environ.setdefault('XDG_RUNTIME_DIR', '/tmp/qmlhost-runtime')
os.makedirs(os.environ['XDG_RUNTIME_DIR'], mode=0o700, exist_ok=True)

# Qt 5.15 deprecates the Connections syntax LG's tree (written for 5.12) uses throughout.
# LG's ImageButton reads `bFocusScaleEffect`, which nothing defines: the state is never entered on the set either.
IGNORE = re.compile(r'Implicitly defined onFoo|bFocusScaleEffect is not defined')

KEYS = {'LEFT': 'Key_Left', 'RIGHT': 'Key_Right', 'UP': 'Key_Up', 'DOWN': 'Key_Down', 'OK': 'Key_Return', 'BACK': 'Key_Escape'}

# The banner list's own geometry (BannerListView: left 0, top `mBannerTopMargin`; the detail card has its own left margin).
BANNER_TOP = 562

SYSTEM_PROPERTIES = '''import QtQuick 2.12
QtObject {
    property bool isRTLLocale: false
    property bool isTouchScreen: false
    property bool isTouchMode: false
    property bool storeMode: false
    property bool magicCursorVisible: false
    property bool isSILKLocale: false
    property bool isAirtelChannel: false
    property bool isActiveFocusInLiveTVGroup: false
    property bool highContrast: false
    property bool hasAppFocus: true
    property bool isExternalInput: true
    property bool isTvInput: false
    property string country: "USA"
    property string bannerInterfaceName: "inputBanner"
    property bool isAppActivated: true
    property bool isAirtelOperator: false
    property bool isBNOTV: false
    property bool noSignalGuideAreaVisible: true
    property bool audioGuidanceOn: false
    property bool isChannelZappingStatus: false
    property bool isISDB: false
    property bool isATSC: true
    property bool isFreesatChannel: false
    property bool isSmartCityChannel: false
    property bool isLivePip: false
    property string currentChannelId: "0_11_1"
}
'''

# What the set reports for HDMI 2 (the Denon, named over CEC) in the input banner's controller.
INTERFACES = '''import QtQuick 2.12
QtObject {
    property var inputBanner: QtObject {
        property bool isOkBanner: %(ok)s
        property bool isRightSide: false
        property bool isZappingBanner: %(zap)s
        property bool isLaunchingBanner: false
        property bool isDetailBanner: %(detail)s
        property bool isPlacedTop: false
        property bool isViewVisible: true
        property int bannerPosState: 2
        function showZappingBanner() {}
        property real nsuProgressBarValue: %(nsu)s
        property var detailInfoListModel: ({
            threed:     JSON.stringify({dingbatText: "", dingbat: "", iconVisible: false, dingbatEnable: false, guideText: ""}),
            storeMode:  JSON.stringify({dingbatText: "", dingbat: "", iconVisible: false, dingbatEnable: false, guideText: ""}),
            ratio:      JSON.stringify({dingbatText: "16:9", dingbat: "", iconVisible: true, dingbatEnable: false, guideText: "16:9"}),
            resolution: JSON.stringify({dingbatText: "4K", dingbat: "", iconVisible: true, dingbatEnable: false, guideText: "4K"}),
            hdr:        JSON.stringify({dingbatText: "HDR10", dingbat: "", iconVisible: true, dingbatEnable: false, guideText: "HDR10"}),
            bt2020:     JSON.stringify({dingbatText: "BT.2020", dingbat: "", iconVisible: true, dingbatEnable: false, guideText: "BT.2020"}),
            inputAudio: JSON.stringify({dingbatText: "PCM", dingbat: "", iconVisible: true, dingbatEnable: false, guideText: "PCM"}),
            rating:     JSON.stringify({dingbatText: "", dingbat: "", iconVisible: false, dingbatEnable: false, guideText: ""}),
            dolbyAudio: JSON.stringify({dingbatText: "DD+", dingbat: "", iconVisible: true, dingbatEnable: false, guideText: "Dolby Digital Plus"}),
            dolby:      JSON.stringify({dingbatText: "", dingbat: "", iconVisible: false, dingbatEnable: false, guideText: ""}),
            atmos:      JSON.stringify({dingbatText: "ATMOS", dingbat: "", iconVisible: true, dingbatEnable: false, guideText: "Dolby Atmos"})
        })
        property int pressed: 0
        property int more: 0
        function handleBannerPressed() { pressed++; console.log("[luna] handleBannerPressed") }
        function launchMoreActions() { more++; console.log("[luna] launchMoreActions") }
        function restartHideTimer() {}
        function hide() {}
    }
    property var externalDeviceController: QtObject {
        property bool hasUrcuDevice: true
        property string _inputLabel: "AVR-S760H"
        function getDeviceName() { return "HDMI 2" }
    }
    property var hotelMode: QtObject { property bool isHotelModeReady: false; property bool isHidePopup: false }
    property var audioGuide: QtObject { function readButtonText() {} function readText() {} }
    property var cursor: QtObject { function requestPreventCursorHide() {} function releaseOkButton() {} }
    property var dvr: QtObject { property var recInfoModel: null }
    property var common: QtObject { function setSystemSettings(kind, what) { console.log("[luna] setSystemSettings " + JSON.stringify(what)) } }
    %(extra)s
}
'''

# What the Live TV channel banner's controller reports: the OK banner and the zapping strip of the local station Nitin's antenna would get.
EXTRA_CHANNEL = '''property var channelBanner: QtObject {
        property bool isOkBanner: %(ok)s
        property bool isRightSide: false
        property bool isZappingBanner: %(zap)s
        property bool isLaunchingBanner: false
        property bool isDetailBanner: %(detail)s
        property bool isPlacedTop: false
        property bool isViewVisible: true
        property bool isLive: true
        property bool rfIpChannel: false
        property bool isBannerRequestByVideoStart: false
        property bool isUpdatedChannelByTimeshift: false
        property string channelNumber: "11.1"
        property string channelMajorNumber: "11"
        property string channelMinorNumber: ".1"
        property string channelName: "WXIA"
        property string programName: "Antiques Roadshow: Atlanta, Hour 2"
        property string channelIconUrl: ""
        property var channelColorType: ({})
        property string channelFavorite: ""
        property int channelSignalLevel: 4
        property int channelSourceType: 2
        property int shortCut: -1
        property bool voiceSelectResult: false
        property var progressBar: ({visible: true, barVisible: true, startEndTime: "", progressRate: 0.42, isFocused: false})
        property real nsuProgressBarValue: 0
        property bool isPrevExist: true
        property bool isNextExist: true
        property bool isAvailableNavigation: true
        property bool isAvailableWatch: true
        property var detailInfoListModel: %(model)s
        signal changeCurrentIndex(int index)
        signal positionViewAtIndex(int index)
        function navigationChannelUpDown(up) {}
        function navigationChannelChange() {}
        function changePrevProgram() {}
        function changeNextProgram() {}
        function handleDetailInfoBannerPressed() {}
        function checkIsAvailableNextProgram() { return true }
        property int more: 0
        function isInternetChannel() { return false }
        function handleBannerPressed() { console.log("[luna] handleBannerPressed") }
        function launchMoreActions() { more++; console.log("[luna] launchMoreActions") }
        function restartHideTimer() {}
        function hide() {}
    }'''

# The no-signal screen's controller: no photo setting, the block of words moving on black.
EXTRA_SCREENSAVER = '''property var screensaver: QtObject {
        id: ss
        property var noSignalPhotoModel: QtObject {
            property bool enabled: %(photo)s
            property bool opened: %(photo)s
            property bool opend: %(photo)s
            property bool isBlockGuide: false
            property bool hasChangedPhoto: false
            property int currentIdx: 0
            property int numOfFileNotFoundError: 0
            property int numOfTryCountInError: 3
            property var imgArray: ["%(still)s"]
            signal showScreenSaverGuide()
            function initImgPathToDefault() {}
            function updateFile() {}
        }
        property string currentSignalState: "bad"
        property string currentScreensaver: ""
        property bool isStaticScreensaver: false
        property bool isSupportGuideAreaToggle: true
        property bool visibleScreensaver: true
        property bool isVideoValid: false
        property string currentViewType: ""
        property Timer arm: Timer { interval: 150; running: true; onTriggered: ss.currentScreensaver = "NO_SIGNAL" }
        signal toggleGuideKeyAreaByClick()
        function getDynamicMessage(key) { return "No Signal" }
        function getDynamicSubMessage(key) { return "Check the device on this input, or switch to another input." }
        function getStaticHeaderMessage(key) { return "No Signal" }
        function getStaticBodyMessage(key) { return "Check the device on this input, or switch to another input." }
        function getAvailViewTypes(key) { return ["WITH_IMAGE", "TEXT_ONLY"] }
        function setScreensaverVisible(v) {}
        function showHidePincodePrompt(v) {}
        function audioGuideForNosignal() {}
    }'''

# The prompt inputcommon opens over the picture: LG's "Input Locked"-style question with two answers.
EXTRA_POPUP = '''property var popup: QtObject {
        property bool isModal: false
        property bool isSecondFocus: false
        property string titleText: ""
        property string messageText: "Do you want to turn on your set-top box?"
        property string audioGuideMessageText: ""
        property string firstBtnText: "Yes"
        property string secondBtnText: "No"
        property bool alertPopupVisible: false
        function yesFeedBackFromAlertPopup() { console.log("[luna] yes") }
        function noFeedBackFromAlertPopup() { console.log("[luna] no") }
        function alertPopupHide() {}
    }'''

STRING_SHEET = '''import QtQuick 2.12
QtObject {
    property string banner_6: "Current Time"
    property string banner_9: "Back"
    property string banner_10: "Home"
    property string banner_25: "button"
    property string banner_28: "Store Demo"
    property string banner_44: "Options"
    property string ch_banner_24: "Input Information"
    property string ch_banner_25: "Universal Remote"
    property string ch_banner_39: "REC"
    property string ch_banner_12: "Move the banner here"
    property string ch_banner_13: "Upper left"
    property string ch_banner_14: "Upper right"
    property string ch_banner_15: "Lower left"
    property string ch_banner_16: "Lower right"
    property string banner_41: "Scroll down"
    property string banner_42: "Scroll up"
    property string banner_30: "Channel"
    property string banner_12: "Watch"
    property string banner_2: "Selected"
    property string banner_3: "Not selected"
    property string ch_banner_23: "Channel information"
    property string internetChannelPrefix: "IP"
    property var screensaver: QtObject { property string live_tv_235: "Show or hide"; property string live_tv_235_tts: "Press red to show or hide" }
}
'''

TIME_MANAGER = 'import QtQuick 2.12\nQtObject { property string currentTime: "9:58 PM"; property string currentDate: "Sun, Oct 25" }\n'
WEBOS = {'Key_webOS_CursorShow': 16777600, 'Key_webOS_CursorHide': 16777601}
LOGGER = 'import QtQuick 2.12\nQtObject { function i() {} function w() {} function e() {} function d() {} function normal() {} }\n'

# LG's font manager hands the style sheet font descriptions; its own families are not on this machine.
FONT_MANAGER = '''import QtQuick 2.12
QtObject {
    property var smart_Light: ({family: "LG Smart UI", weight: Font.Light})
    property var smart_Regular: ({family: "LG Smart UI", weight: Font.Normal})
    property var smart_SemiBold: ({family: "LG Smart UI", weight: Font.DemiBold})
    property var smart_Bold: ({family: "LG Smart UI", weight: Font.Bold})
    property var cond_Regular: ({family: "LG Smart UI Cond", weight: Font.Normal})
    property var number_Regular: ({family: "LG Smart UI NUMBER", weight: Font.Normal})
    property var dingbat_Regular: ({family: "LG Smart UI Dingbat", weight: Font.Normal})
}
'''

# The banner list's job for one item: put it where the list does.
SCENES = {
    # `size`: the banner list gives an item its parent's width; the detail card has a size of its own, which a sized loader would override.
    'pill':    {'file': 'MainViewLayer/InputBanner/InputBanner.qml', 'ok': False, 'zap': True, 'detail': False, 'size': 'width: 1920', 'focused': False},
    'focused-pill': {'file': 'MainViewLayer/InputBanner/InputBanner.qml', 'ok': False, 'zap': True, 'detail': False, 'size': 'width: 1920', 'focused': True},
    'ok':      {'file': 'MainViewLayer/InputBanner/InputBanner.qml', 'ok': True, 'zap': False, 'detail': False, 'size': 'width: 1920', 'focused': True},
    'detail':  {'file': 'MainViewLayer/InputBanner/DetailInfoBanner.qml', 'ok': False, 'zap': False, 'detail': True, 'size': '', 'focused': False},
    # the TV's software update, in the detail card's header (`NsuProgressBar`)
    'update':  {'file': 'MainViewLayer/InputBanner/DetailInfoBanner.qml', 'ok': False, 'zap': False, 'detail': True, 'size': '', 'focused': False, 'nsu': 64},
    # Live TV's channel banner, the same two states as the input's (the strip alone; OK with the options key)
    'channel': {'file': 'MainViewLayer/ChannelBanner/ChannelBanner.qml', 'ok': False, 'zap': True, 'detail': False, 'size': 'width: 1920', 'focused': False, 'tv': True, 'extra': ('channel',)},
    'channel-ok': {'file': 'MainViewLayer/ChannelBanner/ChannelBanner.qml', 'ok': True, 'zap': False, 'detail': False, 'size': 'width: 1920', 'focused': True, 'tv': True, 'extra': ('channel',)},
    # the no-signal screen over black, and over LG's photo setting (the text box on a still)
    'channel-detail': {'file': 'MainViewLayer/ChannelBanner/DetailInfoBanner.qml', 'ok': False, 'zap': False, 'detail': True, 'size': '', 'focused': False, 'tv': True,
                       'extra': ('channel',), 'model': True},
    'nosignal': {'file': 'BgViewLayer/Screensaver.qml', 'ok': False, 'zap': False, 'detail': False, 'size': 'width: 1920; height: 1080', 'focused': False,
                 'top': 0, 'backdrop': False, 'extra': ('screensaver',), 'photo': False},
    'nosignal-photo': {'file': 'BgViewLayer/Screensaver.qml', 'ok': False, 'zap': False, 'detail': False, 'size': 'width: 1920; height: 1080', 'focused': False,
                       'top': 0, 'backdrop': False, 'extra': ('screensaver',), 'photo': True},
    # where the banner goes: four places, one with the focus
    'banner-edit': {'file': 'MainViewLayer/BannerEditView.qml', 'ok': False, 'zap': False, 'detail': False, 'size': 'width: 1920; height: 1080', 'focused': True, 'top': 0},
    # the question over the picture (set-top box, input lock)
    'alert': {'file': 'PopupLayer/AlertPopupView.qml', 'ok': False, 'zap': False, 'detail': False, 'size': 'width: 1920; height: 1080', 'focused': True, 'top': 0, 'extra': ('popup',)},
}
# The programme under the channel banner's INFO card: one entry of LG's list model (every field is a JSON string, as the controller hands them over).
DETAIL_FIELDS = ('additionalGenre atmos atvAudio audioDescription audioLanguage bmlChannelJP bt2020 ccText closedCaption closedCaptionJP copyRightJP desc dolby '
                 'dolbyAudio dtvAudio freesatSt genre ginga hbbtv hdr hdsd mailJP mainLanguage mheg mono monoForced multiAudio programImage rating ratio '
                 'recordSchedule resolution rfIpChannel sap satellite scramble signLanguage spokenSubtitle stereo stereoForced storeMode subLanguage '
                 'subtitle teletext threed linkage watchSchedule').split()
DETAIL_ICONS = {'ratio': '16:9', 'resolution': '1080i', 'dolbyAudio': 'DD', 'stereo': 'Stereo', 'closedCaption': 'CC', 'rating': 'TV-PG'}
DETAIL_TEXT = ('Appraisers travel to Atlanta to value family heirlooms, from a Civil War sword to a mid-century glass collection. '
               'A rare find draws the biggest crowd of the day, and one guest learns what a grandmother\'s trunk was worth.')


def detail_model():
    def icon(name):
        text = DETAIL_ICONS.get(name, '')
        return json.dumps(json.dumps({'dingbatText': text, 'dingbat': '', 'iconVisible': bool(text), 'dingbatEnable': False, 'guideText': text, 'valueText': '', 'valueImage': ''}))
    fields = ['%s: %s' % (n, icon(n)) for n in DETAIL_FIELDS]
    fields.append('title: ' + json.dumps(json.dumps({'valueText': 'Antiques Roadshow: Atlanta, Hour 2'})))
    fields.append('description: ' + json.dumps(json.dumps({'valueText': DETAIL_TEXT})))
    fields.append('progressBar: ' + json.dumps(json.dumps({'visible': True, 'barVisible': True, 'startEndTime': '8:00 - 9:00 PM', 'progressRate': 0.42, 'isFocused': False})))
    return 'ListModel { ListElement { %s } }' % '; '.join(fields)


EXTRAS = {'channel': EXTRA_CHANNEL, 'screensaver': EXTRA_SCREENSAVER, 'popup': EXTRA_POPUP}

HOST_QML = '''import QtQuick 2.12

Item {
    id: screen
    width: 1920; height: 1080
    property url backdrop: ""
    property url banner: ""
    property int bannerTop: %(top)d
    property bool focused: %(focused)s
    Rectangle { anchors.fill: parent; color: "%(base)s" }
    Image { anchors.fill: parent; source: screen.backdrop; fillMode: Image.PreserveAspectCrop; asynchronous: false }
    Item {
        id: slot
        x: 0; y: screen.bannerTop; width: parent.width; height: %(slotH)d
        Loader {
            id: loader
            objectName: "loader"
            %(size)s
            source: screen.banner
            focus: true
            onLoaded: { if (screen.focused) { item.forceActiveFocus(); if (item.setDefaultFocus) item.setDefaultFocus() } }
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
        var item = walk(screen)
        if (!item) return "<no such item>"
        var v = item
        var path = String(prop).split(".")
        for (var j = 0; j < path.length && v !== undefined && v !== null; j++) v = v[path[j]]
        return String(v)
    }
}
'''


def palette(light):
    """The palette `tools/inputcommon.sh stage` hands the patch: tokens.py's `qml` output. The shipped one always comes from
    the dark material; `light` swaps the light material in, to see the same files in a theme whose sheet is light."""
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
    """Every size LG's style sheet hands out multiplied by `percent` / 100: the 200 % text pass the Lucent rules ask of a screen
    (the TV has no text-size setting, so this only shows what a larger face would do to the layout)."""
    path = os.path.join(qml, 'UserInterfaceLayer', 'GlobalStyle.qml')
    with open(path, encoding='utf-8') as f:
        text = f.read()
    k = percent / 100.0
    text, n = re.subn(r'"pixelSize": size,', '"pixelSize": size * %s,' % k, text)
    text, m = re.subn(r'Math\.round\(pixelSize \* _fontRatio\)', 'Math.round(pixelSize * _fontRatio * %s)' % k, text)
    if n < 6 or m != 1:
        raise SystemExit('text scale: %d font functions and %d fontSize found in the style sheet' % (n, m))
    with open(path, 'w', encoding='utf-8') as f:
        f.write(text)


def stage(dest, patched, light=False, text=100):
    """A copy of LG's tree under `dest/qml`, patched the way the set's staging does when asked."""
    qml = os.path.join(dest, 'qml')
    shutil.copytree(STOCK, qml)
    if patched:
        r = subprocess.run([sys.executable, os.path.join(ROOT, 'tools', 'inputcommon_patch.py'), qml, palette(light), FONTS],
                           capture_output=True, text=True)
        if r.returncode != 0:
            raise SystemExit('patch failed: ' + r.stderr)
    if text != 100:
        scale_text(qml, text)
    return qml


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--scene', choices=sorted(SCENES), default='pill')
    ap.add_argument('--patched', action='store_true', help='run tools/inputcommon_patch.py over the copy first')
    ap.add_argument('--light', action='store_true', help='with --patched: stage with the light material as the palette')
    ap.add_argument('--shot', action='append', default=[], metavar='MS:PATH', help='PNG of the window at MS after start (repeatable); a bare PATH is taken at --wait')
    ap.add_argument('--wait', type=int, default=1500)
    ap.add_argument('--backdrop', help='image drawn behind the banner')
    ap.add_argument('--press', action='append', default=[], metavar='MS:KEYS', help='keys sent at MS, comma separated')
    ap.add_argument('--probe', action='append', default=[], metavar='MS:OBJECT:PROP', help='print a property of the item with this objectName at MS, as `[probe] OBJECT.PROP=value`')
    ap.add_argument('--keep', help='keep the staged tree in this directory')
    ap.add_argument('--text', type=int, default=100, help='text size in percent (200 doubles every size of the style sheet)')
    ap.add_argument('--audit', metavar='KINDS', nargs='?', const='overlap', help='run the layout audit of the finished picture (tests/qmlhost/layoutaudit.py) for these kinds, comma separated; `all` for every kind; findings are warnings')
    a = ap.parse_args(argv)
    if not os.path.isdir(STOCK):
        print("needs the census copy of LG's inputcommon QML at", STOCK)
        return 2

    from PyQt5 import QtCore, QtGui, QtQml, QtQuick
    work = os.path.abspath(a.keep) if a.keep else tempfile.mkdtemp(prefix='inputcommon-')
    if a.keep and os.path.exists(work):
        shutil.rmtree(work)
    os.makedirs(work, exist_ok=True)
    qml = stage(work, a.patched, a.light, a.text)
    spec = SCENES[a.scene]
    host = os.path.join(work, 'Host.qml')
    with open(host, 'w') as f:
        f.write(HOST_QML % {'top': spec.get('top', BANNER_TOP), 'size': spec['size'], 'focused': str(spec['focused']).lower(),
                            'slotH': 1080 if 'top' in spec else 600, 'base': '#0d1020' if spec.get('backdrop', True) else '#000000'})

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
    eng = view.engine()
    ctx = view.rootContext()

    def make(name, source, data=None):
        comp = QtQml.QQmlComponent(eng)
        comp.setData((source % (data or {})).encode(), QtCore.QUrl.fromLocalFile(os.path.join(work, name + '.qml')))
        obj = comp.create(ctx)
        if obj is None:
            for e in comp.errors():
                print('ERROR', name, e.toString())
            raise SystemExit(1)
        keep.append(obj)
        ctx.setContextProperty(name, obj)
        return obj

    keep = []
    sysprops = SYSTEM_PROPERTIES
    if spec.get('tv'):
        # Live TV: the tuner is the input and the channel banner is the banner interface
        sysprops = (sysprops.replace('isExternalInput: true', 'isExternalInput: false').replace('isTvInput: false', 'isTvInput: true')
                    .replace('bannerInterfaceName: "inputBanner"', 'bannerInterfaceName: "channelBanner"'))
    make('systemProperties', sysprops)
    make('windowHandler', 'import QtQuick 2.12\nQtObject { property bool isFocusableViewExist: false; signal hideAllViews(var sender) }\n')
    make('stringSheet', STRING_SHEET)
    make('timeManager', TIME_MANAGER)
    ctx.setContextProperty('webOS', WEBOS)
    ctx.setContextProperty('isDevice', True)
    make('logger', LOGGER)
    flags = dict({k: str(spec[k]).lower() for k in ('ok', 'zap', 'detail')}, nsu=spec.get('nsu', 0), photo=str(spec.get('photo', False)).lower(),
                 still=os.path.join(ROOT, 'docs', 'mocks', 'img', 'still.jpg'), model=detail_model() if spec.get('model') else 'null')
    extra = ''.join(EXTRAS[n] % flags + '\n    ' for n in spec.get('extra', ()))
    make('interfaces', INTERFACES, dict(flags, extra=extra))
    fonts = make('fontManager', FONT_MANAGER)
    # LG's own style sheet, instantiated the way UIController does (a window size and the font manager).
    sheet_comp = QtQml.QQmlComponent(eng, QtCore.QUrl.fromLocalFile(os.path.join(qml, 'UserInterfaceLayer', 'GlobalStyle.qml')))
    sheet = sheet_comp.beginCreate(ctx)
    if sheet is None:
        for e in sheet_comp.errors():
            print('ERROR', e.toString())
        return 1
    sheet.setProperty('windowSize', {'width': 1920, 'height': 1080})
    sheet.setProperty('fontManager', fonts)
    sheet_comp.completeCreate()
    keep.append(sheet)
    ctx.setContextProperty('styleSheet', sheet)

    view.setResizeMode(QtQuick.QQuickView.SizeRootObjectToView)
    view.resize(1920, 1080)
    view.setSource(QtCore.QUrl.fromLocalFile(host))
    if view.status() != QtQuick.QQuickView.Ready:
        for e in view.errors():
            print('ERROR', e.toString())
        return 1
    root = view.rootObject()
    root.setProperty('banner', QtCore.QUrl.fromLocalFile(os.path.join(qml, 'UserInterfaceLayer', 'Containers', spec['file'])))
    if a.backdrop and spec.get('backdrop', True):
        root.setProperty('backdrop', QtCore.QUrl.fromLocalFile(os.path.abspath(a.backdrop)))
    view.show()

    def press(keys):
        for name in keys.split(','):
            if not name:
                continue
            key = getattr(QtCore.Qt, KEYS[name.strip().upper()])
            for typ in (QtCore.QEvent.KeyPress, QtCore.QEvent.KeyRelease):
                app.sendEvent(view, QtGui.QKeyEvent(typ, key, QtCore.Qt.NoModifier))
    for s in a.press:
        ms, _, keys = s.partition(':')
        QtCore.QTimer.singleShot(int(ms), lambda k=keys: press(k))

    def probe(name, prop):
        v = QtCore.QMetaObject.invokeMethod(root, 'probe', QtCore.Qt.DirectConnection, QtCore.Q_RETURN_ARG('QVariant'),
                                            QtCore.Q_ARG('QVariant', name), QtCore.Q_ARG('QVariant', prop))
        print('[probe] %s.%s=%s' % (name, prop, v), flush=True)
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
