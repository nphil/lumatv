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


def direct(name, page_id, *after, at=2800):
    """A child page opened the way a quick-panel tile or a search result opens it, so no rows are counted to reach it."""
    @scene(name)
    def _():
        steps = [("eval", 1500, "root.openSettings()"), ("eval", READY, "root.searchPicked('page','%s','')" % page_id)]
        if after:
            steps.append(("keys", READY + 900, k(*after)))
        return steps + [("snap", READY + at, name)]


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
