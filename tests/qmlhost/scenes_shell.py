"""The shell slice's scenes (ShellSlice): the library behind the Apps tile, search and its keyboard, the voice bar and the
banners, each reached the way Home reaches it (a key where there is one), in dark and light.

    python3 tests/qmlhost/scenes.py --only 12-all-apps,09-search-keyboard --out /tmp/shell-shots

The names follow the mocks in docs/mocks/shots; the ones with no mock (the Hidden section, the dock tile's own menu, the
uninstall question, the rename keyboard) say what they are.
"""
from scenes import READY, scene, js


def apps_open():
    """Home at rest has the Apps tile focused: OK on it opens the library."""
    return [("keys", READY, "ok")]


def streaming():
    """The rail's second entry, and the focus into its grid."""
    return [("keys", READY + 700, "left down right")]


@scene("12-all-apps")
def _():
    return apps_open() + streaming() + [("snap", READY + 1800, "12-all-apps")]


@scene("12-all-apps-all")
def _():
    # three rows are in view; the fifth row of icons scrolls into the third place
    return apps_open() + [("keys", READY + 700, "down down down down"), ("snap", READY + 2000, "12-all-apps-all")]


@scene("12-all-apps-hidden")
def _():
    # the owner hid two apps: the rail ends with Hidden, which lists them and offers to bring them back (the rail clamps at its foot)
    return [("theme", 0, {"apps": {"hidden": ["com.crunchyroll.webapp", "com.limelight.webos"]}})] + apps_open() + \
           [("keys", READY + 700, "left " + "down " * 8), ("snap", READY + 2000, "12-all-apps-hidden")]


@scene("12-all-apps-menu")
def _():
    # the Menu key on an icon (a hold of OK does the same): Open, Move on the dock, Hide, Rename, Uninstall
    return apps_open() + streaming() + [("keys", READY + 1500, "menu"), ("snap", READY + 2600, "12-all-apps-menu")]


@scene("16-context-menu-dock")
def _():
    # Home's dock: Right to Netflix (tile 2), the Menu key: the same menu beside the lit tile
    return [("keys", READY, "right menu"), ("snap", READY + 1400, "16-context-menu-dock")]


VOICE_READY = {"state": "idle", "session": 0, "ready": True}


@scene("06-confirm-uninstall")
def _():
    # library, Down to Crunchyroll, its menu, Down to Uninstall, OK: the question, with Keep it focused
    return apps_open() + [("keys", READY + 700, "down menu"), ("keys", READY + 1800, "down down down down ok"), ("snap", READY + 3000, "06-confirm-uninstall")]


@scene("09-search-keyboard")
def _():
    # the mic hint shows when the helper says voice search is ready
    return [("publish", 300, "voice", VOICE_READY), ("eval", READY, "root.openSearch('Net')"), ("snap", READY + 1500, "09-search-keyboard")]


@scene("09-search-rename")
def _():
    # library, Right to Netflix, its menu, Down to Rename, OK: the same keyboard, no answers, the name to change
    return apps_open() + [("keys", READY + 700, "right menu"), ("keys", READY + 1800, "down down down ok"), ("snap", READY + 3000, "09-search-rename")]


@scene("28-voice-listening")
def _():
    return [("eval", READY, "root.voiceStart()"), ("snap", READY + 1200, "28-voice-listening")]


@scene("28-voice-heard")
def _():
    # one command naming one app acts: the words, "Opening..." (the app opens 700 ms later)
    return [("eval", READY, "root.voiceHeard('open PlxNative')"), ("snap", READY + 300, "28-voice-heard")]


@scene("28-voice-results")
def _():
    return [("eval", READY, "root.voiceHeard('netflix series about chess')"), ("snap", READY + 1200, "28-voice-results")]


@scene("28-voice-working")
def _():
    # the key is up and the words are on their way
    return [("publish", 300, "voice", dict(VOICE_READY, say={"text": "x", "delayMs": 20000})), ("eval", READY, "root.voiceKeyDown()"),
            ("eval", READY + 500, "root.voiceKeyUp()"), ("snap", READY + 1500, "28-voice-working")]


@scene("28-voice-failed")
def _():
    return [("publish", 300, "voice", VOICE_READY), ("eval", READY, "root.voiceKeyDown()"), ("eval", READY + 500, "root.voiceKeyUp()"),
            ("snap", READY + 1500, "28-voice-failed")]


@scene("28-mic-indicator")
def _():
    return [("eval", READY, "root.showMic(true)"), ("snap", READY + 1000, "28-mic-indicator")]


CHANNEL = {"kind": "channel", "number": "11.1", "call": "WXIA", "title": "Antiques Roadshow \u00b7 Atlanta, Hour 2",
           "sub": "8:00 \u2013 9:00 PM  \u00b7  23 min left", "next": "Nova: Secrets of the Shining Knight", "progress": 0.62,
           "chips": ["1080i", "Stereo", "CC"], "holdMs": 60000}
INPUT = {"kind": "input", "icon": "gamepad-2", "name": "HDMI 3 \u00b7 PlayStation 5", "facts": "4K \u00b7 120 Hz \u00b7 HDR10", "holdMs": 60000}


@scene("20-channel-banner")
def _():
    return [("eval", READY, "root.showBanner(%s)" % js(CHANNEL)), ("snap", READY + 1000, "20-channel-banner")]


@scene("25-input-pill")
def _():
    return [("eval", READY, "root.showBanner(%s)" % js(INPUT)), ("snap", READY + 1000, "25-input-pill")]


@scene("25-input-banner")
def _():
    spec = dict(INPUT, facts="4K \u00b7 120 Hz \u00b7 HDR10 \u00b7 Game Optimizer", buttons=True)
    return [("eval", READY, "root.showBanner(%s)" % js(spec)), ("snap", READY + 1000, "25-input-banner")]


@scene("18-no-signal")
def _():
    spec = {"kind": "nosignal", "icon": "unplug", "port": "HDMI 4", "title": "Nothing is connected here",
            "body": "Plug a device into HDMI 4, or pick another input. The screen stays dark to protect the panel."}
    return [("eval", READY, "root.showBanner(%s)" % js(spec)), ("snap", READY + 1000, "18-no-signal")]
