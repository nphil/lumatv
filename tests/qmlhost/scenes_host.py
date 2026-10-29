"""Scenes of the overlay over an app (ShellHost): the banners that say what is on the TV and the notification drawer a held gear opens,
drawn over a picture standing in for video, dark and light.

    python3 tests/qmlhost/scenes.py --only 25-overlay-input-pill,13-overlay-drawer --out /tmp/host-shots

Each is `LumaOverlay.qml` loaded beside Home as the fullscreen container loads it (qmlhost.py `--overlay`), Home hidden and an app
"in front": what is drawn is what the set would draw over Live TV or an HDMI input, up to the look of a pixel (software GL says nothing
about time). The picture behind is generated here: a horizon, a sun and a dark sea, so the glass has something to be seen against.
"""
import json
import os
import struct
import subprocess
import sys
import tempfile
import zlib

from scenes import AI, AUDIT, NOW, HERE, js, png_path, scene
import xdisplay

_DIR = tempfile.mkdtemp(prefix="host-scenes-")


def video_png():
    """A 384x216 picture, stretched to the window behind the overlay."""
    path = os.path.join(_DIR, "video.png")
    if os.path.exists(path):
        return path
    w, h = 384, 216
    horizon = int(h * 0.62)
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        for x in range(w):
            if y < horizon:
                t = y / horizon
                r, g, b = 250 * (1 - t) + 40 * t, 150 * (1 - t) + 60 * t, 60 * (1 - t) + 110 * t
                d = ((x - w * 0.7) ** 2 + (y - horizon * 0.78) ** 2) ** 0.5
                if d < 22:
                    r, g, b = 255, 232, 170
                elif d < 60:
                    k = (60 - d) / 38 * 0.35
                    r, g, b = r + (255 - r) * k, g + (210 - g) * k, b + (120 - b) * k
            else:
                t = (y - horizon) / (h - horizon)
                r, g, b = 20 + 30 * (1 - t), 40 + 30 * (1 - t), 70 + 30 * (1 - t)
                if abs(x - w * 0.7) < 14 * (1 - t) + 2 and (y + x) % 5 < 3:
                    r, g, b = r + 90, g + 70, b + 20
            raw += bytes((int(r), int(g), int(b)))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    return path


def render(name, out_dir, material, steps, snap_at, publish=()):
    """steps: [(ms, js)] run in the host; one grab at `snap_at`. The overlay is built at once and an app is in front from 800 ms."""
    path = png_path(out_dir, name, material)
    if os.path.exists(path):
        os.remove(path)
    user = {"perf": {"syncResidents": True}, "material": material, "banners": {"own": True}}
    args = [sys.executable, os.path.join(HERE, "qmlhost.py"), "LumaHome.qml", "--fake-helper", "--overlay", "--bgimage", video_png(),
            "--prop", "hostActive=false", "--theme", json.dumps(user),
            "--eval", "100:overlay.warm = true", "--eval", "100:overlay.warmQuick = true", "--eval", "100:overlay.warmPicker = true",
            "--eval", "800:overlay.appFront = true"]
    for ms, topic, doc in publish:
        args += ["--publish", "%d:%s:%s" % (ms, topic, json.dumps(doc))]
    for ms, code in steps:
        args += ["--eval", "%d:%s" % (ms, code)]
    args += ["--snap", "%d:%s" % (snap_at, path), "--wait", str(snap_at + 600)]
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=240)
    text = r.stdout + r.stderr
    warn = [ln for ln in text.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    return (r.returncode or (1 if not os.path.exists(path) else 0)), warn, text


INPUT = {"kind": "input", "icon": "gamepad-2", "name": "HDMI 3 \u00b7 PlayStation 5", "facts": "4K \u00b7 120 Hz \u00b7 HDR10", "holdMs": 60000}
CHANNEL = {"kind": "channel", "number": "11.1", "call": "WXIA", "title": "Antiques Roadshow \u00b7 Atlanta, Hour 2",
           "sub": "8:00 \u2013 9:00 PM  \u00b7  23 min left", "next": "Nova: Secrets of the Shining Knight", "progress": 0.62,
           "chips": ["1080i", "Stereo", "CC"], "holdMs": 60000}
NO_SIGNAL = {"kind": "nosignal", "icon": "unplug", "port": "HDMI 4", "title": "Nothing is connected here",
             "body": "Plug a device into HDMI 4, or pick another input. The screen stays dark to protect the panel."}
TV_UPDATE = {"source": "TV", "icon": "download", "title": "TV update ready", "body": "webOS 10.2.3, 412 MB. Installs only when you say so, from Settings > System.",
             "time": NOW - 26 * 3600 * 1000}
POST = ("(function(){Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'kit\"\\nQtObject { Component.onCompleted: {NoteLog.post(%s);} }', root); return 1})()"
        % json.dumps(TV_UPDATE).replace("\\", "\\\\").replace("'", "\\'"))


@scene("25-overlay-input-pill", runner=True)
def _(out_dir, material):
    return render("25-overlay-input-pill", out_dir, material, [(1000, "overlay.showBanner(%s)" % js(INPUT))], 1800)


@scene("25-overlay-input-banner", runner=True)
def _(out_dir, material):
    spec = dict(INPUT, facts="4K \u00b7 120 Hz \u00b7 HDR10 \u00b7 Game Optimizer", buttons=True, actions=["inputs", "picture"])
    return render("25-overlay-input-banner", out_dir, material, [(1000, "overlay.showBanner(%s)" % js(spec)), (1400, "overlay.key(Qt.Key_Right, true, false, 0)")], 2200)


@scene("20-overlay-channel-banner", runner=True)
def _(out_dir, material):
    return render("20-overlay-channel-banner", out_dir, material, [(1000, "overlay.showBanner(%s)" % js(CHANNEL))], 1800)


@scene("18-overlay-no-signal", runner=True)
def _(out_dir, material):
    return render("18-overlay-no-signal", out_dir, material, [(1000, "overlay.showBanner(%s)" % js(NO_SIGNAL))], 1800)


@scene("13-overlay-drawer", runner=True)
def _(out_dir, material):
    return render("13-overlay-drawer", out_dir, material, [(500, POST), (1000, "overlay.gearHold('notifications')")], 2400, publish=[(300, "ai", AI), (300, "__audit", AUDIT)])


def quick_scene(name, out_dir, material, keys=()):
    """The quick panel the gear's tap shows over an app: the receiver read just before, an ambient tile from the plugin."""
    from scenes_panels import AMBIENT_ROSTER, AMBIENT_TILES, again
    steps = [(900, again(0, controls=True)[2]), (1000, "overlay.panel('quick')")] + [(1500 + 100 * i, "overlay.key(%s, true, false, 0)" % k) for i, k in enumerate(keys)]
    return render(name, out_dir, material, steps, 2300,
                  publish=[(300, "ai", AI), (300, "plugins", AMBIENT_ROSTER), (300, "plugin.ambient.tiles", AMBIENT_TILES)])


@scene("10-overlay-quick", runner=True)
def _(out_dir, material):
    return quick_scene("10-overlay-quick", out_dir, material)


@scene("10-overlay-quick-foot", runner=True)
def _(out_dir, material):
    # focus on the button under the list: Down past the last tile
    return quick_scene("10-overlay-quick-foot", out_dir, material, keys=["Qt.Key_Down"] * 7)


@scene("08-overlay-picker", runner=True)
def _(out_dir, material):
    return render("08-overlay-picker", out_dir, material, [(1000, "overlay.panel('picker')"), (1500, "overlay.key(Qt.Key_Right, true, false, 0)")], 2300)
