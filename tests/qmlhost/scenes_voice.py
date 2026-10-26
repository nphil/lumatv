"""Scenes of LG's Search app (`com.webos.app.voice`) restyled by tools/voice_patch.py, dark and light.

Not Home's surfaces: the screens are LG's QML from the census copy, patched, and drawn by tests/qmlhost/voice.py (LG's own style sheet and
views behind stand-ins for what is native) over a still of video. Names follow the mock they answer (`09-search-keyboard`) with `lg-` for the
restyled LG file. "Light" stages the patch with the light material as its palette: on the set the sheet is always the dark material's
(tools/tokens.py), so this shows the same files in a theme whose sheet is light.

    python3 tests/qmlhost/scenes.py --only 09-lg-search,09-lg-search-focus,09-lg-search-auto,09-lg-search-noti,09-lg-search-option,09-lg-search-confirm --out /tmp/vs
"""
import os
import subprocess
import sys

from scenes import scene, png_path, ROOT, HERE
import xdisplay

STILL = os.path.join(ROOT, "docs", "mocks", "img", "still.jpg")


def render(name, out_dir, material, scene_name, wait=3000):
    path = png_path(out_dir, name, material)
    if os.path.exists(path):
        os.remove(path)
    args = [sys.executable, os.path.join(HERE, "voice.py"), "--scene", scene_name, "--patched", "--backdrop", STILL,
            "--wait", str(wait), "--shot", "%d:%s" % (wait, path)]
    if material == "light":
        args.append("--light")
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=240)
    text = r.stdout + r.stderr
    warn = [ln for ln in text.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln and "QML Image: Cannot open" not in ln
            and "Unable to assign [undefined] to QQuickItem" not in ln and "' of null" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    return (r.returncode or (1 if not os.path.exists(path) else 0)), warn, text


@scene("09-lg-search", runner=True)
def _(out_dir, material):
    # the field focused, the keyword rows under it
    return render("09-lg-search", out_dir, material, "search")


@scene("09-lg-search-focus", runner=True)
def _(out_dir, material):
    return render("09-lg-search-focus", out_dir, material, "search-focus")


@scene("09-lg-search-auto", runner=True)
def _(out_dir, material):
    return render("09-lg-search-auto", out_dir, material, "auto")


@scene("09-lg-search-noti", runner=True)
def _(out_dir, material):
    return render("09-lg-search-noti", out_dir, material, "noti")


@scene("09-lg-search-option", runner=True)
def _(out_dir, material):
    return render("09-lg-search-option", out_dir, material, "option")


@scene("09-lg-search-confirm", runner=True)
def _(out_dir, material):
    return render("09-lg-search-confirm", out_dir, material, "confirm")
