"""The volume display over the pictures it meets, on one contact sheet, and the time from a volume key to the bar's first frame.

    python3 tests/qmlhost/volume_sheet.py --out docs/research/volume-osd/renders           # renders and the sheet
    python3 tests/qmlhost/volume_sheet.py --latency                                       # key to first presented frame, offline

Home dark and light (the glass is the frosted wallpaper's), then over an app's picture, where the glass is see-through: a bright daytime frame
(light glass), a dark film frame, a busy game frame (dark glass), and the mute over the bright frame. Needs an X display (tests/xdisplay.py) and PyQt5, PIL.
"""
import argparse
import json
import os
import re
import subprocess
import sys

from scenes import HERE, png_path
import scenes_volume as sv
import xdisplay

ROOT = os.path.dirname(os.path.dirname(HERE))
IMG = os.path.join(ROOT, "docs", "mocks", "img")
B = sv.B
PRESS = [sv.sound_out(B), sv.receiver_at(B + 200, sv.RX_FMT), sv.key_at(B + 2000, "volumeUp", 32)]
MUTE = [sv.sound_out(B), sv.receiver_at(B + 200, sv.RX_FMT), sv.receiver_at(B + 2000, dict(sv.RX_FMT, mute=True)), sv.key_at(B + 2020, "mute", None, True)]
# without the helper (no lock to wait for): the TV's own volume, the bar with no format
TV = [sv.sound_out(B, "tv_speaker", 46, True), sv.key_at(B + 2000, "volumeUp", 47)]
TVMUTE = [sv.sound_out(B, "tv_speaker", 46, True), sv.key_at(B + 2000, "mute", None, True)]
# (name, caption, material, over, picture, steps, grab at)
SHEET = [
    ("sheet-home-dark", "Home, dark", "dark", "home", None, PRESS, B + 2700),
    ("sheet-home-light", "Home, light", "light", "home", None, PRESS, B + 2700),
    ("sheet-bright", "Bright daytime frame, light glass", "light", "video", os.path.join(IMG, "art-hokusai.jpg"), PRESS, B + 2700),
    ("sheet-film", "Dark film frame, dark glass", "dark", "video", os.path.join(IMG, "still.jpg"), PRESS, B + 2700),
    ("sheet-game", "Busy game frame, dark glass", "dark", "video", os.path.join(IMG, "game.jpg"), PRESS, B + 2700),
    ("sheet-muted", "Muted, bright frame, light glass", "light", "video", os.path.join(IMG, "art-hokusai.jpg"), MUTE, B + 3300),
]


def sheet(out_dir, helper=True):
    from PIL import Image, ImageDraw
    os.makedirs(out_dir, exist_ok=True)
    shots = []
    for name, caption, material, over, picture, steps, at in SHEET:
        if not helper:
            steps = TVMUTE if steps is MUTE else TV
        rc, warn, text = sv.render(name, out_dir, material, steps, at, over=over, bg=picture, helper=helper)
        ours = [w for w in warn if re.search(r"VolumeOsd|TvVolume|AudioFormat|LumaOverlay|Glass\.qml|LuPane|ReceiverGate|overlay volume|\[snap\]", w)]
        print("%-18s rc=%s %s" % (name, rc, ours or "ok"), flush=True)
        shots.append((caption, png_path(out_dir, name, material)))
    tile_w, tile_h = 640, 360
    crop = (560, 860, 1360, 1040)                    # the bar's place at the bottom centre, at full size
    cols = 3
    rows = (len(shots) + cols - 1) // cols
    sheet_img = Image.new("RGB", (cols * tile_w, rows * (tile_h + 90 + 30)), (24, 24, 28))
    draw = ImageDraw.Draw(sheet_img)
    for i, (caption, path) in enumerate(shots):
        im = Image.open(path).convert("RGB")
        x, y = (i % cols) * tile_w, (i // cols) * (tile_h + 120)
        sheet_img.paste(im.resize((tile_w, tile_h)), (x, y))
        close = im.crop(crop).resize((tile_w, int((crop[3] - crop[1]) * tile_w / (crop[2] - crop[0]))))
        sheet_img.paste(close, (x, y + tile_h))
        draw.rectangle((x, y, x + 8 + 7 * len(caption), y + 16), fill=(0, 0, 0))
        draw.text((x + 4, y + 2), caption, fill=(255, 255, 255))
    path = os.path.join(out_dir, "volume-sheet.png")
    sheet_img.save(path)
    print("sheet", path)


def latency():
    """A key's report (the audio service's volumeUp) to the next frame the window presents, three presses, as the host runs in software GL on a loaded
    machine: the order of magnitude and that nothing waits for the receiver, not the set's number."""
    find = "var f=function(p){if(p.staleMs!==undefined)return p;for(var i=0;i<p.children.length;i++){var r=f(p.children[i]);if(r)return r}return null}; var o=f(overlay);"
    press = ("(function(){ %s var t0=Date.now(); var w=overlay.window; var g=function(){ w.frameSwapped.disconnect(g); "
             "console.info('[latency] key to next frame ' + (Date.now()-t0) + ' ms, up=' + o.paneUp + ' level=' + o.level + ' fill=' + o.fraction.toFixed(3)) }; w.frameSwapped.connect(g); "
             "Qt.createQmlObject('import QtQuick 2.4\\nimport WebOSCompositorBase 1.0\\nimport WebOSServices 1.0\\nQtObject { Component.onCompleted: { LS.audioService.report(\"%s\", null, null) } }', overlay, 'audio'); return 1 })()")
    steps = [sv.sound_out(B), sv.receiver_at(B + 200, sv.RX_FMT), sv.osd_at(B + 300, "o.holdMs = 600; o.settleMs = 300")]
    for i, why in enumerate(("volumeUp", "volumeUp", "volumeDown")):
        steps.append((B + 2000 + i * 2500, press % (find, why)))
    # a held key: ten repeats 100 ms apart, then the level the keys made it and the one the receiver reads
    held = [(B + 9600 + i * 100, sv.key_at(0, "volumeUp")[1]) for i in range(10)]
    steps += held
    steps.append(sv.osd_at(B + 11000, "console.info('[held] level=' + o.level + ' fill=' + o.fraction.toFixed(3) + ' keyed=' + o.keyed)"))
    steps.append(sv.osd_at(B + 12500, "console.info('[settled] level=' + o.level + ' keyed=' + o.keyed)"))
    user = {"perf": {"syncResidents": True}, "material": "dark"}
    args = [sys.executable, os.path.join(HERE, "qmlhost.py"), "LumaHome.qml", "--fake-helper", "--overlay", "--theme", json.dumps(user), "--eval", "100:overlay.warm = true",
            "--bgimage", os.path.join(IMG, "still.jpg"), "--prop", "hostActive=false", "--eval", "800:overlay.appFront = true"]
    for ms, code in steps:
        args += ["--eval", "%d:%s" % (ms, code)]
    args += ["--wait", str(B + 13200)]
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=280)
    for line in (r.stdout + r.stderr).splitlines():
        if line.startswith("[latency]") or line.startswith("[held]") or line.startswith("[settled]"):
            print(line)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--out", default=os.path.join(ROOT, "docs", "research", "volume-osd", "renders"))
    ap.add_argument("--latency", action="store_true")
    ap.add_argument("--no-helper", action="store_true", help="the TV's own volume, no fake helper (nothing to queue for)")
    a = ap.parse_args()
    latency() if a.latency else sheet(a.out, helper=not a.no_helper)
