"""Scenes of the pages that cover the rest of LG's Settings (docs/research/settings-coverage.md): one scene per page and tab, dark and light.

Each opens Settings the way the gear does, takes the rail to the section the page hangs under, and presses the D-pad through the
rows to the page (a child page is opened by the row that leads to it), so what is drawn is what a person gets.
"""
from scenes import scene, k, READY
from scenes_settings import page

DOWN = "down"


def register(name, section, tab, *after, at=2800):
    @scene(name)
    def _():
        return page(section, tab, *after) + [("snap", READY + at, name)]


# System › Accessibility: the tab, and the pages it opens (rows in order: Audio Guidance, Text with High Contrast, Focus with High
# Contrast, Grayscale, Invert Colors, Screen Zoom, Closed Caption, Audio Description, Audio options)
# (the tab itself is scenes_settings' 30-settings-system-accessibility)
register("30-settings-audioguide", "system", 3, "ok")
register("30-settings-captions", "system", 3, *[DOWN] * 6, "ok")
register("30-settings-captions-text", "system", 3, *[DOWN] * 6, "ok", "up", "right", DOWN)
register("30-settings-captions-background", "system", 3, *[DOWN] * 6, "ok", "up", "right", "right", DOWN)
register("30-settings-captions-subtitles", "system", 3, *[DOWN] * 6, "ok", "up", "right", "right", "right", DOWN)
register("30-settings-captions-choice", "system", 3, *[DOWN] * 6, "ok", DOWN, "ok")
register("30-settings-audio-options", "system", 3, *[DOWN] * 8, "ok")


def direct(name, page_id, *after, at=2800, before=()):
    """A child page opened the way a quick-panel tile or a search result opens it, so no rows are counted to reach it. `before` are steps
    that put the TV in the state the page needs (a different sound out)."""
    @scene(name)
    def _():
        steps = list(before) + [("eval", 1500, "root.openSettings()"), ("eval", READY, "root.searchPicked('page','%s','')" % page_id)]
        if after:
            steps.append(("keys", READY + 900, k(*after)))
        return steps + [("snap", READY + at, name)]


def tv_setting(category, settings):
    """A step that changes one of the offline TV's settings before Settings opens."""
    js = ("(function(){var o=Qt.createQmlObject('import QtQuick 2.4; import WebOSCompositorBase 1.0; import WebOSServices 1.0; QtObject { function set() { "
          "LS.adhoc.call(\"luna://com.webos.service.settings\", \"/setSystemSettings\", JSON.stringify({category: \"%s\", settings: %s})) } }', root);o.set();return 1})()" % (category, settings))
    return ("eval", 1000, js)


# System › Time & power's pages, Bluetooth, Join Network and IP Settings
direct("30-settings-clock", "clock")
direct("30-settings-power-on-timer", "poweron")
direct("30-settings-power-off-timer", "poweroff")
direct("30-settings-bluetooth-speakers", "btaudio")
direct("30-settings-bluetooth-controllers", "bthid")
direct("30-settings-wifi", "wifi")
direct("30-settings-wifi-ask", "wifi", DOWN, "ok")
direct("30-settings-wifi-password", "wifi", DOWN, DOWN, "ok")
direct("30-settings-ip", "ipsettings")

# Sound: the chooser for the sound out, and the pages under it (the equalizer answers only with the TV's own speakers)
register("30-settings-sound-choice", "sound", 0, "ok")
direct("30-settings-hdmi-audio", "hdmiaudio", at=4500)
direct("30-settings-equalizer", "equalizer", at=4500, before=[tv_setting("sound", '{soundOutput: "tv_speaker"}')])

# System › Language & location: the language list, the chooser of the menu language, the keyboards and the location
direct("30-settings-language", "language", at=4500)
direct("30-settings-language-choice", "language", "ok", at=4500)
direct("30-settings-language-keyboards", "language", "up", "right", DOWN, at=4500)
direct("30-settings-language-location", "language", "up", "right", "right", DOWN, at=4500)

# System › Support (tabs TV info, Update, Privacy, Tools, Reset), Safety, and Inputs › Channels
direct("30-settings-support", "support", at=4500)
direct("30-settings-support-update", "support", "up", "right", DOWN, at=4500)
direct("30-settings-support-privacy", "support", "up", "right", "right", DOWN, at=4500)
direct("30-settings-support-tools", "support", "up", "right", "right", "right", DOWN, at=4500)
direct("30-settings-support-reset", "support", "up", "right", "right", "right", "right", DOWN, at=4500)
direct("30-settings-support-reset-ask", "support", "up", "right", "right", "right", "right", DOWN, "ok", at=4500)
direct("30-settings-safety", "safety", at=4500)
direct("30-settings-channels", "channels", at=4500)

# Picture › Colour: White balance (22 points, at the 100 % level) and Fine Tune (user mode, so its rows answer)
direct("30-settings-white-balance", "whitebalance", at=4500)
direct("30-settings-fine-tune", "finetune", at=4500, before=[tv_setting("picture", '{dynamicColor: "user"}')])

# Picture › Expert: HDMI Signalling; System › Accessibility: Emergency Information; Support › Privacy: App Permissions (the camera, then the microphone tab) and
# Delete My Personal Information
direct("30-settings-signalling", "signalling", at=4500)
direct("30-settings-emergency", "emergency", at=4500)
direct("30-settings-permissions", "permissions", at=4500)
direct("30-settings-permissions-microphone", "permissions", "up", "right", DOWN, at=4500)
direct("30-settings-delete-data", "deletedata", at=4500)
