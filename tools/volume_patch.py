#!/usr/bin/env python3
"""Patches a copy of LG's `StarfishVolumeView.qml` (the compositor view that draws the volume and mute popup) so that it never opens:
LumaTV's own display (payload/compositor/lumatv/panels/VolumeOsd.qml) is the only one.

LG's popup is this one view. It hosts `views/volume/StarfishVolume.qml` (the level bar and its number, or on an ARC output the speaker
with a highlighted + or -) and `views/volume/StarfishVolumeMute.qml` (the bubble that stays up while the TV is muted and moves 94 px
every ten seconds), and opens either from the audio service's `volumeUpdated` signal:

    Connections { target: LS.audioService; onVolumeUpdated: { if (available) updateVisibility(); else console.info("volume view is not available") } }

`available` is the view's own switch. LG builds it from the power-off state, the hotel-mode setting and the configd key
`com.webos.surfacemanager.volumeOSD`, and a view that is not available closes what it has open, ignores the signal and does not start
the mute bubble (`volume.onClosed` asks `available` too). The patch puts `false &&` in front of that expression, so the signal is answered
with LG's own "volume view is not available" line (in /var/log/messages, once per change: the proof on the set that the popup was asked
for and declined) and nothing opens. One path does not ask `available`: a change of the country setting to Ukraine (where LG keeps the mute
bubble up) calls `updateVisibility()` directly, so that function returns at once when the view is not available.

Why this and not the configd key: the key is a persistent setting of the TV and would outlive the shadow module that carries LumaTV's
display, so a TV that fell back to LG's module would have no volume display at all. The patched view exists only in the shadow module
(`tools/lumatv build_shadow`), so LG's popup is back whenever that module is not the one loaded, and the popup goes off in the same
restart that brings LumaTV's display. Nothing the compositor relies on is touched: the contract with the rest of LG's code
(`setLocaleInfo`, `handleVolumeBannerTimeout`, `muteStr`, `requestTts`, `opened`) and BaseView's open and close are as LG wrote them.
Multi View's window decoration has a volume bar of its own (`StarfishServerDecoration.qml`, `volumeUI`); it draws only in Multi View,
which the set does not use (PLAN), and is left as LG's.

All three anchors must be found exactly once: the first proves that `available` is what opens the popup (a firmware that stops gating the
signal on it would make the patch a lie), the second is the property itself, the third the function. A firmware update that rewrites any of
them fails here, loudly, and the caller leaves LG's file in force.

usage: volume_patch.py <StarfishVolumeView.qml>
Exit status: 0 patched in place, 1 an anchor is missing, 2 usage.
"""
import pathlib
import sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from notification_patch import sub  # noqa: E402

MARKER = "Luma"

# the signal handler that opens the popup, and only while `available`
GATE = ("        onVolumeUpdated: {\n"
        "            if (available)\n"
        "                updateVisibility();\n")
AVAILABLE = "    property bool available: !((root.powerProcessing"
UPDATE = "    function updateVisibility() {\n"


def patch(text):
    name = "StarfishVolumeView.qml"
    if MARKER + ": patched" in text:
        raise ValueError("already patched: start from a fresh copy of LG's file")
    sub(name, text, GATE, GATE)
    text = sub(name, text, AVAILABLE,
               "    // Luma: LumaTV's own display answers every volume and mute change, so this view is never available (tools/volume_patch.py).\n"
               "    property bool available: false && !((root.powerProcessing")
    text = sub(name, text, UPDATE, UPDATE + "        if (!available) return;   // Luma: the country path asks no one; a view that is not available opens nothing\n")
    return "// Luma: patched, see tools/volume_patch.py\n" + text


def main(argv):
    if len(argv) != 2:
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        print("usage: volume_patch.py <StarfishVolumeView.qml>", file=sys.stderr)
        return 2
    try:
        path = pathlib.Path(argv[1])
        path.write_text(patch(path.read_text()))
    except (ValueError, OSError) as e:
        print("volume_patch: " + str(e), file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
