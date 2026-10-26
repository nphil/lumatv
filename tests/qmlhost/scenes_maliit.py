"""Scenes of LG's on-screen keyboard keys (maliit's `view-global` plugin) restyled by tools/maliit_patch.py, dark and light, at 100 % and at
200 % text (`-200`).

Not Home's surfaces: the keyboard is LG's QML from the census copy of the set (85 files), patched, and drawn by tests/qmlhost/maliit.py (LG's
own style sheets and components behind stand-ins for what is native) over a still of video, which is what the keyboard is drawn over on the
set. Names follow the mock they answer (`09-search-keyboard`) with `lg-keys` for the restyled LG file. "Light" stages the patch with the light
material as its palette: on the set the sheet is always the dark material's (tools/tokens.py), so this shows the same files in a theme whose
sheet is light.

    python3 tests/qmlhost/scenes.py --only 09-lg-keys,09-lg-keys-focus-search,09-lg-keys-focus-shift,09-lg-keys-symbols,09-lg-keys-numbers,09-lg-keys-candidates --out /tmp/ks
"""
import os
import subprocess
import sys

from scenes import scene, png_path, ROOT, HERE
import xdisplay

STILL = os.path.join(ROOT, "docs", "mocks", "img", "still.jpg")


def render(name, out_dir, material, scene_name, text=100, wait=1800):
    path = png_path(out_dir, name, material)
    if os.path.exists(path):
        os.remove(path)
    args = [sys.executable, os.path.join(HERE, "maliit.py"), "--scene", scene_name, "--patched", "--backdrop", STILL,
            "--wait", str(wait), "--shot", "%d:%s" % (wait - 100, path), "--text", str(text)]
    if material == "light":
        args.append("--light")
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=240)
    out = r.stdout + r.stderr
    warn = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "file://" in ln and "Implicitly defined onFoo" not in ln]
    warn = [ln for ln in warn if "ButtonBase.qml:30:5: Unable to assign [undefined] to int" not in ln and "LeftFunctionArea.qml:165:13" not in ln
            and "FocusPanel.qml:27:5" not in ln and "fontfamily is not defined" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    return (r.returncode or (1 if not os.path.exists(path) else 0)), warn, out


for _name, _scene in (("09-lg-keys", "letters"), ("09-lg-keys-focus-search", "focus-enter"), ("09-lg-keys-focus-shift", "focus-shift"),
                      ("09-lg-keys-symbols", "symbols"), ("09-lg-keys-numbers", "numbers"), ("09-lg-keys-candidates", "candidates-focus")):
    for _text in (100, 200):
        def _make(name=_name, scene_name=_scene, text=_text):
            @scene(name + ("" if text == 100 else "-200"), runner=True)
            def _(out_dir, material):
                return render(name + ("" if text == 100 else "-200"), out_dir, material, scene_name, text)
        _make()
