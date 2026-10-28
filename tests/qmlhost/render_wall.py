#!/usr/bin/env python3
"""Renders of Home over the moving wallpaper, on the host: what the display engine shows when LumaHome draws nothing where the wallpaper was and the
video plane is under it.

    python3 tests/qmlhost/render_wall.py [--out DIR] [--only gpu,backdrop,tint,tint-worst]

The video plane is a picture behind the QML (qmlhost.py `--bgimage`): a frame decoded from the shipped clip (`wall-silk.vpc`), and for the glass
check the worst case, the brightest every poster frame of the clip has at each point (what a word of a pane could sit over). LumaHome runs as it
does under LumaTV's compositor, with a stand-in player that says it plays and the shell's two properties set (`videoWallBackend`, `videoWallHole`),
so the GPU wallpaper is faded out and the window is transparent there. Scenes:

    gpu          the GPU wallpaper (the default): Home as it was
    backdrop     the video wallpaper, the glass as it is on the GPU: the blur of the picture the clip is made from
    tint         the video wallpaper, tint-only glass: a flat tint over the video, opaque as the clip's `glass.alpha` says
    tint-worst   tint-only glass over the worst case: the brightest the clip gets at every point

Needs Xvfb, PyQt5, ffmpeg and Pillow 10.3. The pictures are written to DIR (default docs/research/videowall).
"""
import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
sys.path.insert(0, os.path.join(ROOT, "tests"))
sys.path.insert(0, os.path.join(ROOT, "tools"))

READY = 7000
SHIPPED = os.path.join(ROOT, "payload", "vplane", "clips")
HOST = os.path.join(HERE, "qmlhost.py")


def decoded_frame(unit, out):
    """Unit `unit` of the shipped clip decoded to a 1920 x 1080 PNG (the picture the video plane shows)."""
    import vplane_clips as vc
    v = vc.read_vpc(os.path.join(SHIPPED, "wall-silk.vpc"))
    with tempfile.TemporaryDirectory() as d:
        stream = os.path.join(d, "s.hevc")
        with open(stream, "wb") as f:
            f.write(b"".join(u for u, _ in v["units"][:unit + 1]))
        subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-i", stream, "-vf",
                        "select=eq(n\\,%d),scale=in_range=tv:out_range=full:in_color_matrix=bt709:flags=accurate_rnd+full_chroma_int,format=rgb24" % unit,
                        "-frames:v", "1", out], check=True)


def worst_frame(out):
    """The brightest, point by point, of the clip's poster frames: the worst picture a pane's words can sit over."""
    import vplane_wall as vw
    from PIL import Image, ImageChops
    look = vw.look_of(vw.read_theme())
    base = vw.graded(Image.open(vw.wallpaper_path(look["wallpaper"])).convert("RGB"), look["motion"])
    n = vw.LOOP_SEC * vw.CONTENT_FPS
    best = None
    for i in sorted({round(k * n / vw.POSTERS) % n for k in range(vw.POSTERS)}):
        im = vw.render(base, vw.loop_angle(i, n), look)
        best = im if best is None else ImageChops.lighter(best, im)
    best.save(out)


def render(out_png, theme, bg=None, wait_after=3200):
    """One run of the host: Home with `theme`, the wall player answering, `bg` behind it; the picture is taken `wait_after` ms after Home is ready."""
    from test_vplane_wall import FakeWall, manifest_dir, configure
    mailbox = tempfile.mkdtemp(prefix="wall-render-mb-")
    clips = manifest_dir()
    player = FakeWall(mailbox)
    try:
        at = READY + 300
        props = {"startMs": 4000, "settleMs": 150, "endMs": 150}
        cmd = [sys.executable, HOST, "LumaHome.qml", "--fake-helper", "--prop", "hostActive=true", "--prop", "videoWallBackend=own", "--prop", "videoWallHole=true",
               "--wallpaper", os.path.join(ROOT, "payload", "wallpaper", "wall_1080.png"), "--theme", json.dumps(dict(theme, perf={"syncResidents": True}))]
        if bg:
            cmd += ["--bgimage", bg]
        eval_js = ("(function(){var w = root.videoWall; w.dirUrl = %s; w.clipsUrl = %s; w.loadManifest(); var c = %s; for (var k in c) w[k] = c[k]; return 1})()"
                   % (json.dumps("file://" + mailbox + "/"), json.dumps("file://" + clips + "/"), json.dumps(props)))
        cmd += ["--eval", "%d:%s" % (at - 200, eval_js), "--snap", "%d:%s" % (at + wait_after, out_png), "--wait", str(at + wait_after + 400)]
        r = subprocess.run(cmd, env=dict(os.environ), capture_output=True, text=True, timeout=600)
        if r.returncode != 0 and not os.path.exists(out_png):
            raise SystemExit("the host failed:\n" + (r.stdout + r.stderr)[-3000:])
    finally:
        player.close()
        shutil.rmtree(mailbox, ignore_errors=True)
        shutil.rmtree(clips, ignore_errors=True)
    return out_png


SCENES = {
    "gpu": ({}, "frame"),
    "backdrop": ({"wallpaperMotion": {"source": "video"}}, "frame"),
    "tint": ({"wallpaperMotion": {"source": "video", "glass": "tint"}}, "frame"),
    "tint-worst": ({"wallpaperMotion": {"source": "video", "glass": "tint"}}, "worst"),
}


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", default=os.path.join(ROOT, "docs", "research", "videowall"))
    ap.add_argument("--only", default="")
    ap.add_argument("--unit", type=int, default=900, help="the unit of the clip the plane shows (900: fifteen seconds in)")
    a = ap.parse_args()
    os.makedirs(a.out, exist_ok=True)
    frame, worst = os.path.join(a.out, "plane-frame.png"), os.path.join(a.out, "plane-worst.png")
    want = [s for s in SCENES if not a.only or s in a.only.split(",")]
    if any(SCENES[s][1] == "frame" for s in want):
        decoded_frame(a.unit, frame)
    if any(SCENES[s][1] == "worst" for s in want):
        worst_frame(worst)
    for name in want:
        theme, plane = SCENES[name]
        t0 = time.time()
        render(os.path.join(a.out, "home-%s.png" % name), theme, bg=frame if plane == "frame" else worst)
        print("%-11s %5.1f s" % (name, time.time() - t0), flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
