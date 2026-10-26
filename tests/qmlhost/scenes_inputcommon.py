"""Scenes of LG's input banners (`com.webos.app.inputcommon`) restyled by tools/inputcommon_patch.py, dark and light.

These are not Home's surfaces: the banners are LG's QML from the census copy, patched, and drawn by tests/qmlhost/inputcommon.py
(LG's own style sheet and banner components behind stand-ins for what is native) over a still of video, which is what they are
drawn over on the set. Names follow the mocks they answer (`25-input-*`) with `lg-` for the restyled LG file. "Light" stages the
patch with the light material as its palette: on the set the sheet is always the dark material's (tools/tokens.py), so this shows
the same files in a theme whose sheet is light.

    python3 tests/qmlhost/scenes.py --only 25-lg-input-pill,25-lg-input-ok,25-lg-input-info,25-lg-input-update,20-lg-channel-banner,18-lg-no-signal --out /tmp/ic-shots
"""
import os
import subprocess
import sys

from scenes import scene, png_path, ROOT, HERE
import xdisplay

STILL = os.path.join(ROOT, "docs", "mocks", "img", "still.jpg")


def render(name, out_dir, material, scene_name, press=(), wait=1400):
    path = png_path(out_dir, name, material)
    if os.path.exists(path):
        os.remove(path)
    args = [sys.executable, os.path.join(HERE, "inputcommon.py"), "--scene", scene_name, "--patched", "--backdrop", STILL,
            "--wait", str(wait), "--shot", "%d:%s" % (wait, path)]
    if material == "light":
        args.append("--light")
    for ms, keys in press:
        args += ["--press", "%d:%s" % (ms, keys)]
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=240)
    text = r.stdout + r.stderr
    warn = [ln for ln in text.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln and "bFocusScaleEffect" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    return (r.returncode or (1 if not os.path.exists(path) else 0)), warn, text


@scene("25-lg-input-pill", runner=True)
def _(out_dir, material):
    return render("25-lg-input-pill", out_dir, material, "pill")


@scene("25-lg-input-ok", runner=True)
def _(out_dir, material):
    # OK pressed: the options key beside the pill, the input name lit
    return render("25-lg-input-ok", out_dir, material, "ok")


@scene("25-lg-input-options", runner=True)
def _(out_dir, material):
    return render("25-lg-input-options", out_dir, material, "ok", press=[(600, "RIGHT")])


@scene("25-lg-input-info", runner=True)
def _(out_dir, material):
    return render("25-lg-input-info", out_dir, material, "detail")


@scene("25-lg-input-update", runner=True)
def _(out_dir, material):
    return render("25-lg-input-update", out_dir, material, "update")


# Live TV's channel banner (mock 20), the no-signal screen (mock 18), the place the banner goes and the question over the picture.
# The no-signal scenes are black: the photo mode shows LG's setting over a still.
@scene("20-lg-channel-banner", runner=True)
def _(out_dir, material):
    return render("20-lg-channel-banner", out_dir, material, "channel")


@scene("20-lg-channel-ok", runner=True)
def _(out_dir, material):
    return render("20-lg-channel-ok", out_dir, material, "channel-ok")


@scene("20-lg-channel-options", runner=True)
def _(out_dir, material):
    return render("20-lg-channel-options", out_dir, material, "channel-ok", press=[(600, "RIGHT")])


@scene("20-lg-channel-info", runner=True)
def _(out_dir, material):
    return render("20-lg-channel-info", out_dir, material, "channel-detail")


@scene("18-lg-no-signal", runner=True)
def _(out_dir, material):
    return render("18-lg-no-signal", out_dir, material, "nosignal")


@scene("18-lg-no-signal-photo", runner=True)
def _(out_dir, material):
    return render("18-lg-no-signal-photo", out_dir, material, "nosignal-photo")


@scene("25-lg-banner-position", runner=True)
def _(out_dir, material):
    return render("25-lg-banner-position", out_dir, material, "banner-edit")


@scene("35-lg-prompt", runner=True)
def _(out_dir, material):
    return render("35-lg-prompt", out_dir, material, "alert")
