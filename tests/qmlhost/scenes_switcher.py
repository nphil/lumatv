"""Scenes of the app switcher (docs/mocks/switcher), dark and light, at 100 % and 200 % text:

    python3 tests/qmlhost/scenes.py --only switcher-open,switcher-confirm --out /tmp/switcher-shots

LumaOverlay.qml with the switcher surface, loaded beside Home as the shell loads it (qmlhost.py `--overlay`), Home hidden. Under LumaTV's
compositor what stands under the shell's window is the compositor's: the dim and the pictures of the apps' windows. The host has no
compositor, so a picture generated here stands under the surface: the dim in the palette's `scrimHide` and a rounded stand-in for each live
picture at the place the shell sends the compositor (panels/SwitcherLayout.js), which is exactly the layering of the set. What is drawn is what
the set draws, up to the look of a pixel; software GL says nothing of time. The 200 % renders go through the layout audit for overlap.
"""
import json
import math
import os
import struct
import subprocess
import sys
import tempfile
import zlib

from scenes import HERE, doubled_type, png_path, scene
import xdisplay

_DIR = tempfile.mkdtemp(prefix="switcher-scenes-")

# the palette's scrimHide (theme.json), 8-bit RGB
SCRIM = {"dark": (0x12, 0x0E, 0x1A), "light": (0xFA, 0xF4, 0xED)}
ALPHA = 0xF8 / 255.0
W, H, GAP, TOP, RADIUS = 400, 225, 32, 340, 22
# stand-ins for what the windows drew last: top colour, bottom colour
SHOTS = {"plex": ((38, 52, 78), (214, 126, 56)), "netflix": ((26, 11, 15), (80, 18, 26)), "youtube.leanback.v4": ((46, 52, 60), (24, 24, 24)),
         "com.geforcenow.play": ((70, 110, 60), (28, 40, 28)), "browser": ((205, 214, 232), (232, 237, 246)), "crunchyroll": ((120, 60, 20), (40, 20, 8)),
         "lgmovies": ((44, 44, 88), (20, 20, 40))}


def apps_of(*ids, picture="live"):
    names = {"plex": "Plex", "netflix": "Netflix", "youtube.leanback.v4": "YouTube", "com.geforcenow.play": "GeForce NOW", "browser": "Web Browser",
             "crunchyroll": "Crunchyroll", "lgmovies": "LG Movies & TV", "com.webos.app.hdmi2": "HDMI 2", "com.webos.app.livetv": "Live TV"}
    return [{"id": i, "title": names.get(i, i), "picture": picture if i in SHOTS else "video", "state": ""} for i in ids]


def backdrop(name, material, apps, first=0, wide=False):
    """1920x1080 PNG: the dim and, where the shell places them, the stand-in pictures of the live apps."""
    path = os.path.join(_DIR, "%s-%s.png" % (name, material))
    sr, sg, sb = SCRIM[material]
    base = (28, 24, 38) if material == "dark" else (233, 226, 238)             # what Home shows under the dim
    bg = tuple(round(c * (1 - ALPHA) + s * ALPHA) for c, s in zip(base, (sr, sg, sb)))
    n = len(apps)
    shown = min(n, 4)
    x0 = round((1920 - (shown * W + (shown - 1) * GAP)) / 2)
    rects = []
    for i, a in enumerate(apps):
        if a["picture"] != "live" or not (first <= i < first + 4):
            continue
        rects.append((x0 + (i - first) * (W + GAP), TOP, SHOTS.get(a["id"], ((60, 60, 60), (30, 30, 30)))))
    rows = bytearray()
    for y in range(1080):
        row = bytearray(bg * 1920)
        for x, top, (c0, c1) in rects:
            if not (top <= y < top + H):
                continue
            t = (y - top) / (H - 1)
            col = tuple(round(a * (1 - t) + b * t) for a, b in zip(c0, c1))
            dy = min(y - top, top + H - 1 - y)
            inset = 0 if dy >= RADIUS else round(RADIUS - math.sqrt(max(0.0, RADIUS ** 2 - (RADIUS - dy - 0.5) ** 2)))
            for xx in range(x + inset, x + W - inset):
                row[xx * 3:xx * 3 + 3] = bytes(col)
        rows += b"\x00" + row
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 1920, 1080, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(rows), 6)) + chunk(b"IEND", b"")
    with open(path, "wb") as f:
        f.write(png)
    return path


def feed(apps, is_open=True, n=1, frm=""):
    return "overlay.switcherFeed = %s" % json.dumps({"apps": apps, "open": is_open, "from": frm, "n": n})


def key(name):
    code = {"left": "Qt.Key_Left", "right": "Qt.Key_Right", "up": "Qt.Key_Up", "down": "Qt.Key_Down", "ok": "Qt.Key_Return", "back": "Qt.Key_Back"}[name]
    return "overlay.key(%s, true, false, 0)" % code


def render(name, out_dir, material, apps, keys=(), first=0, text200=False, snap_at=2600):
    path = png_path(out_dir, name, material)
    if os.path.exists(path):
        os.remove(path)
    user = {"perf": {"syncResidents": True}, "material": material, "remote": {"appSwitcher": True}}
    if text200:
        user.update(doubled_type())
    args = [sys.executable, os.path.join(HERE, "qmlhost.py"), "LumaHome.qml", "--fake-helper", "--overlay", "--bgimage", backdrop(name, material, apps, first),
            "--prop", "hostActive=false", "--theme", json.dumps(user),
            "--eval", "100:overlay.warm = true", "--eval", "100:overlay.warmSwitcher = true", "--eval", "700:" + feed(apps)]
    for i, k in enumerate(keys):
        args += ["--eval", "%d:%s" % (1200 + 150 * i, key(k))]
    audit = path[:-4] + ".json"
    args += ["--snap", "%d:%s" % (snap_at, path), "--audit", "%d:%s:overlap" % (snap_at, audit), "--wait", str(snap_at + 600)]
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=240)
    text = r.stdout + r.stderr
    warn = [ln for ln in text.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    if os.path.exists(audit):
        import layoutaudit
        with open(audit) as f:
            warn += layoutaudit.lines(json.load(f), "layout 200%" if text200 else "layout")
    return (r.returncode or (1 if not os.path.exists(path) or any(w.startswith("[layout") for w in warn) else 0)), warn, text


FOUR = apps_of("plex", "netflix", "youtube.leanback.v4", "com.geforcenow.play")


@scene("switcher-open", runner=True)
def _(out_dir, material):
    return render("switcher-open", out_dir, material, FOUR)


@scene("switcher-open-text200", runner=True)
def _(out_dir, material):
    return render("switcher-open-text200", out_dir, material, FOUR, text200=True)


@scene("switcher-plane", runner=True)
def _(out_dir, material):
    return render("switcher-plane", out_dir, material, apps_of("com.webos.app.livetv", "plex", "com.webos.app.hdmi2", "youtube.leanback.v4"))


@scene("switcher-quit", runner=True)
def _(out_dir, material):
    # Right, then Down: the focus is on YouTube's... on Netflix's own Quit; YouTube is closing
    apps = FOUR[:]
    apps[2] = dict(apps[2], state="quitting")
    return render("switcher-quit", out_dir, material, apps, keys=["right", "down"])


@scene("switcher-quit-text200", runner=True)
def _(out_dir, material):
    apps = FOUR[:]
    apps[2] = dict(apps[2], state="quitting")
    return render("switcher-quit-text200", out_dir, material, apps, keys=["right", "down"], text200=True)


@scene("switcher-confirm", runner=True)
def _(out_dir, material):
    return render("switcher-confirm", out_dir, material, FOUR, keys=["up", "ok"])


@scene("switcher-confirm-text200", runner=True)
def _(out_dir, material):
    return render("switcher-confirm-text200", out_dir, material, FOUR, keys=["up", "ok"], text200=True)


@scene("switcher-many", runner=True)
def _(out_dir, material):
    # seven apps, the fifth in focus: the window of four has moved by one
    apps = apps_of("plex", "netflix", "youtube.leanback.v4", "com.geforcenow.play", "browser", "crunchyroll", "lgmovies")
    return render("switcher-many", out_dir, material, apps, keys=["right"] * 4, first=1, snap_at=3200)


@scene("switcher-empty", runner=True)
def _(out_dir, material):
    return render("switcher-empty", out_dir, material, [])
