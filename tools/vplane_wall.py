#!/usr/bin/env python3
"""Writes and checks the moving wallpaper's clip for the video-plane player (vplane/): Home's live wallpaper as a seamless loop.

    python3 tools/vplane_wall.py wall  [--out DIR] [--id ID] [--loop-sec S] [--content-fps N] [--qp N] [--bit-depth 8|10] [--jobs N]
    python3 tools/vplane_wall.py check [--dir DIR] [--deep]
    python3 tools/vplane_wall.py frame --out PNG [--t SECONDS]          one picture of the loop, to look at

What a wall clip is. Home's GPU wallpaper (LumaHome.qml, `liveWall`) is the wallpaper picture through a slow domain warp: a few pixels of
drift from sine fields, with the warp also modulating the brightness, and the wall scrim over it. This writes that picture as video, so
the decoder draws it and the GPU does not (hardware-census.md, rank 4). The warp is `liveWall`'s own vertex shader, evaluated here on its
64 x 36 grid and applied with a mesh transform, so the clip and the GPU wallpaper that takes over when the player fails are the same
picture. The numbers (amplitude, depth, scale, detail, the grade, the scrim, the picture) are read from theme.json, never typed here, and
`check` fails when one has moved since the clip was written.

The loop is seamless because the phases are whole turns: the GPU's phase rates (0.70, 0.50, 0.40, 0.90 rad/s, and 0.8 of the first for
the third field) have no common period shorter than 314 s, so the clip rounds them to 3, 2, 2, 4 and 2 turns of the 30 s loop (0.63,
0.42, 0.42, 0.84 and 0.42 rad/s). The last picture is one step before the first and playback returns to the first through the key
picture. The motion is at most a few pixels a second, so the pictures are made at 20 a second and each is shown three times: the stream
is 60 pictures a second because the UI plane's refresh follows the video pipeline's rate (docs/research/perf-tweaks.md).

How it is encoded, and why. The picture is a dark, smooth gradient that moves a fraction of a level a frame. Measured on six seconds of
it (HEVC, one key picture, predicted pictures, one reference): 8 bits at a constant quality needed 10.7 MB (CRF 14) because the
rounding of each picture to 8 bits is noise the encoder then codes, and a repeated picture was coded again as a refinement (42 KB each);
10 bits at a constant quantiser (QP 18, key and predicted pictures the same) takes 133 KB with the decoded picture within 0.11 of a level
of the exact one on average and no step of the error larger than 0.76 of a level between two pictures. So the clip is Main10 at QP 18,
about 0.7 MB for the loop; `--bit-depth 8 --qp 16` writes the 8-bit one (1.7 MB, steps up to 1.3 levels) for a decoder that refuses
Main10. The pictures are rendered in 16 bits (the GPU's 8-bit framebuffer throws the precision away).

Not baked: anything above the wallpaper (the glass, the widgets, the dock). The clip is the dark material's: the scrim is black, so a
light material, a wallpaper that is not the clip's picture or a look that has been changed draws the GPU wallpaper. `wall.json` carries
what the shell needs to decide that and to draw its glass over a plane it cannot sample: the look the clip has baked, the tint and luma
the wallpaper sampler would find in it (Tint.js's own method, on poster frames spread over the loop), and the tint-only glass's least
alpha for the ink to read at the TV's distance contrast over the brightest thing the clip can put behind a word.
"""
import argparse
import hashlib
import json
import math
import os
import shutil
import subprocess
import sys
import tempfile
from concurrent.futures import ProcessPoolExecutor

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import vplane_clips as vc  # noqa: E402

ROOT = vc.ROOT
THEME = vc.THEME
WALLPAPER_DIR = os.path.join(ROOT, "payload", "wallpaper")
OUT = vc.OUT

WALL_VERSION = 2                # part of what `check` compares: a change here asks for new clips (2: the warp's edge pixels repeat, were black)
SCREEN = vc.SCREEN
GRID = (64, 36)                 # LumaHome.qml's GridMesh resolution: cells of 30 x 30 pixels
FPS = 60
CONTENT_FPS = 20
LOOP_SEC = 30
QP = {10: 18, 8: 16}            # the constant quantiser each bit depth was measured at (see the docstring)
TURNS = {"x": 3, "y": 2, "z": 2, "w": 4, "v": 2}   # whole turns per loop of phase.x, .y, .z, .w and of the third field's own (0.8 phase.x)
LOOK_KEYS = ("amplitude", "speed", "depth", "scale", "detail", "vignette", "specular", "bloom", "saturation", "contrast")
LOOK_DEFAULTS = {"amplitude": 4, "speed": 1, "depth": 0.25, "scale": 1, "detail": 0.35, "vignette": 0, "specular": 0, "bloom": 0, "saturation": 1, "contrast": 1}
POSTERS = 12                    # pictures the sampler and the glass numbers are taken from, spread over the loop
SAMPLE = (32, 18)               # what LumaHome grabs the wallpaper into for the sampler (Tint.js)
WORST_MARGIN = 1.1              # the brightest thing a word can sit over is the poster frames' brightest, and a tenth more


# ---- the look, from the sources that own it -------------------------------------------------------------------------------

def read_theme():
    with open(THEME) as f:
        return json.load(f)


def look_of(theme, override=None):
    """The wallpaper's motion settings as LumaHome.qml reads them (a missing one is the QML's own default), and the dark material's scrim.
    `override` is a partial `wallpaperMotion` laid over the theme's: the look of a clip made for a look the owner has tuned."""
    wm = dict(theme.get("wallpaperMotion", {}), **(override or {}))
    unknown = sorted(set(override or {}) - set(LOOK_KEYS))
    if unknown:
        raise SystemExit("not wallpaperMotion settings a clip bakes: %s (they are %s)" % (", ".join(unknown), ", ".join(LOOK_KEYS)))
    look = {k: wm.get(k, LOOK_DEFAULTS[k]) for k in LOOK_KEYS}
    scrim = [float(v) for v in theme["dark"]["wallScrim"]]
    return {"motion": look, "scrim": scrim, "material": "dark", "wallpaper": theme["wallpaper"]}


def wallpaper_path(name):
    path = os.path.join(WALLPAPER_DIR, os.path.basename(name))
    if os.path.basename(name) != name or not os.path.isfile(path):
        raise SystemExit("the wallpaper %r is not one of the package's (%s): a clip is made for a shipped picture" % (name, WALLPAPER_DIR))
    return path


# ---- the warp: liveWall's vertex shader, on its grid -------------------------------------------------------------------------

def fields(u, v, theta, look):
    """w1, w2, w3 of the vertex shader at (u, v) for the loop angle `theta` (radians, 0..2 pi over the loop)."""
    k = look["scale"]
    px, py, pz, pw, pv = (TURNS[n] * theta for n in "xyzwv")
    w1 = math.sin(v * 6.0 * k + px) * math.cos(u * 4.0 * k - py)
    w2 = math.sin((u + v) * 5.0 * k - pz)
    w3 = math.sin(u * 11.0 * k + pw) * math.sin(v * 9.0 * k - pv)
    return w1, w2, w3


def smoothstep(e0, e1, x):
    t = min(max((x - e0) / (e1 - e0), 0.0), 1.0)
    return t * t * (3.0 - 2.0 * t)


def vertex(u, v, theta, look):
    """The shader's vUv, vLight (the vignette folded in) and the specular lobe for one vertex."""
    w1, w2, w3 = fields(u, v, theta, look)
    amp = look["amplitude"] / 1920.0
    detail, depth = look["detail"], look["depth"]
    dx = w1 + 0.5 * w2 + detail * 0.5 * w3
    dy = 0.7 * w2 - 0.5 * w1 - detail * 0.5 * w3
    light = 1.0 + depth * (0.3 * (w1 - 0.5 * w2) + detail * 0.25 * w3)
    spec = 0.0
    if look["specular"]:
        crest = min(max(0.5 + 0.5 * (w1 + 0.4 * w3), 0.0), 1.0)      # a narrow lobe on the crest of the main field
        spec = look["specular"] * 0.35 * crest ** 6
    if look["vignette"]:
        qx, qy = (u - 0.5), (v - 0.5) * 0.5625                        # quadratic falloff, aspect-corrected, on the undisplaced position
        light *= 1.0 - look["vignette"] * 0.75 * smoothstep(0.12, 0.42, qx * qx + qy * qy)
    return u + amp * dx, v + amp * dy, light, spec


def scrim_alpha(y, stops):
    """The fragment shader's scrim alpha at the warped y: three stops at 0, 0.55 and 1 (black, so the colour is the picture's times 1 - alpha)."""
    a, b, c = stops
    return a + (b - a) * (y / 0.55) if y < 0.55 else b + (c - b) * ((y - 0.55) / 0.45)


def grid(theta, look, scrim, glow=None):
    """The vertices of one picture: (vUv.x, vUv.y), the brightness multiplier (light times the scrim's remainder) and, when the look has specular
    or bloom, the additive RGB (0..255, through the scrim), 65 x 37 of each; `add` is None otherwise."""
    gw, gh = GRID
    pts, mult, add = [], [], []
    bloom = look["bloom"]
    lit = bool(bloom or look["specular"])
    for iy in range(gh + 1):
        row_p, row_m, row_a = [], [], []
        for ix in range(gw + 1):
            ux, uy, light, spec = vertex(ix / gw, iy / gh, theta, look)
            keep = 1.0 - scrim_alpha(uy, scrim)
            row_p.append((ux, uy))
            row_m.append(light * keep)
            if lit:
                bl = glow_at(glow, ux, uy, bloom) if bloom else (0.0, 0.0, 0.0)
                row_a.append(tuple((spec + b) * keep * 255.0 for b in bl))
        pts.append(row_p)
        mult.append(row_m)
        add.append(row_a)
    return pts, mult, (add if lit else None)


def glow_of(base):
    """The texture the GPU wallpaper's bloom reads (`blurTex`): the graded wallpaper at half size, blurred (QtGraphicalEffects' deviation (radius + 1) / 3.3333 at
    the theme's blur radius over two) and saturated by the theme's `saturation`. The blur is PIL's and the saturation a mix with the luma: the glow is a
    soft term (`max(glow - 0.3, 0) * bloom`), so this is within a few levels of the shader's, which the TV's capture measures."""
    from PIL import Image, ImageEnhance, ImageFilter
    theme = read_theme()
    radius = (theme.get("blurRadius") or 30) / 2
    half = base.resize((SCREEN[0] // 2, SCREEN[1] // 2), Image.BOX)
    half = half.filter(ImageFilter.GaussianBlur((radius + 1) / 3.3333))
    sat = theme.get("saturation")
    return ImageEnhance.Color(half).enhance(1.0 + (0.25 if sat is None else sat))


def glow_at(glow, u, v, bloom):
    """`max(texture2DLod(glow, vUv, 0).rgb - 0.3, 0) * bloom` for one vertex: bilinear, clamped at the edges, in 0..1."""
    w, h = glow.size
    x = min(max(u * w - 0.5, 0.0), w - 1.0)
    y = min(max(v * h - 0.5, 0.0), h - 1.0)
    x0, y0 = int(x), int(y)
    x1, y1 = min(x0 + 1, w - 1), min(y0 + 1, h - 1)
    fx, fy = x - x0, y - y0
    px = glow.load()
    out = []
    for c in range(3):
        top = px[x0, y0][c] * (1 - fx) + px[x1, y0][c] * fx
        bot = px[x0, y1][c] * (1 - fx) + px[x1, y1][c] * fx
        out.append(max((top * (1 - fy) + bot * fy) / 255.0 - 0.3, 0.0) * bloom)
    return tuple(out)


def fmath(fn, **images):
    """ImageMath on float images (Pillow 10.3 or newer: `lambda_eval`); `fn` gets the images by name and the functions `min`, `max`, `convert`."""
    from PIL import ImageMath
    if not hasattr(ImageMath, "lambda_eval"):
        raise SystemExit("Pillow 10.3 or newer is needed (ImageMath.lambda_eval)")
    return ImageMath.lambda_eval(fn, **images)


def to_bytes(a, v):
    """`v` (a float image in 0..255) rounded and clamped to an 8-bit band."""
    return a["convert"](a["max"](a["min"](v + 0.5, 255.0), 0.0), "L")


def graded(base, motion):
    """The wallpaper through gradePass: (mix(luma, c, saturation) - 0.5) * contrast + 0.5. Identity (the shipped look) returns the picture itself."""
    from PIL import Image
    sat, con = float(motion["saturation"]), float(motion["contrast"])
    if sat == 1.0 and con == 1.0:
        return base
    r, g, b = (c.convert("F") for c in base.split())
    luma = fmath(lambda a: a["r"] * 0.299 + a["g"] * 0.587 + a["b"] * 0.114, r=r, g=g, b=b)
    out = []
    for ch in (r, g, b):
        out.append(fmath(lambda a: to_bytes(a, (((a["l"] + (a["c"] - a["l"]) * sat) / 255.0 - 0.5) * con + 0.5) * 255.0), l=luma, c=ch))
    return Image.merge("RGB", out)


_PADDED = {}


def clamp_pad(base, pad):
    """`base` with its edge pixels repeated `pad` pixels outward on every side: what the GPU's clamp-to-edge sampling reads outside the picture (PIL's
    mesh transform reads black there, a fringe of a few pixels where the warp pulls the edge inward). Kept for the picture it was made for."""
    from PIL import Image
    key = (id(base), pad)
    if _PADDED.get("key") == key:
        return _PADDED["img"]
    w, h = base.size
    big = Image.new("RGB", (w + 2 * pad, h + 2 * pad))
    big.paste(base, (pad, pad))
    big.paste(base.crop((0, 0, 1, h)).resize((pad, h), Image.NEAREST), (0, pad))
    big.paste(base.crop((w - 1, 0, w, h)).resize((pad, h), Image.NEAREST), (pad + w, pad))
    big.paste(big.crop((0, pad, w + 2 * pad, pad + 1)).resize((w + 2 * pad, pad), Image.NEAREST), (0, 0))
    big.paste(big.crop((0, pad + h - 1, w + 2 * pad, pad + h)).resize((w + 2 * pad, pad), Image.NEAREST), (0, pad + h))
    _PADDED.update(key=key, img=big, base=base)
    return big


def render_float(base, theta, look_doc, glow=None):
    """One picture of the loop as three float bands (R, G, B in 0..255, not rounded): the mesh warp, the brightness multiplier, then the additive
    specular and bloom of the look (`glow`: the bloom's texture, made from `base` when the look has bloom and none is given)."""
    from PIL import Image
    motion, scrim = look_doc["motion"], look_doc["scrim"]
    gw, gh = GRID
    cw, ch = SCREEN[0] // gw, SCREEN[1] // gh
    if glow is None and motion["bloom"]:
        glow = glow_of(base)
    pts, mult, add = grid(theta, motion, scrim, glow)
    pad = int(math.ceil(motion["amplitude"] * 2.5)) + 4          # the warp moves a point by at most about amplitude x 1.75
    source = clamp_pad(base, pad)
    mesh = []
    for iy in range(gh):
        for ix in range(gw):
            quad = []
            for jx, jy in ((ix, iy), (ix, iy + 1), (ix + 1, iy + 1), (ix + 1, iy)):      # NW, SW, SE, NE: PIL's order
                ux, uy = pts[jy][jx]
                quad += [ux * SCREEN[0] + pad, uy * SCREEN[1] + pad]
            mesh.append(((ix * cw, iy * ch, (ix + 1) * cw, (iy + 1) * ch), quad))
    warped = source.transform(SCREEN, Image.MESH, mesh, Image.BILINEAR)
    # the multiplier is the vertices' values interpolated across the picture: a 65 x 37 float image stretched with its centres on the vertices
    m = Image.new("F", (gw + 1, gh + 1))
    m.putdata([v for row in mult for v in row])
    m = m.resize(SCREEN, Image.BILINEAR, box=(0.5, 0.5, 0.5 + gw, 0.5 + gh))
    bands = [band.convert("F") for band in warped.split()]
    if add is None:
        return [fmath(lambda a: a["c"] * a["m"], c=band, m=m) for band in bands]
    out = []
    for c, band in enumerate(bands):
        s = Image.new("F", (gw + 1, gh + 1))
        s.putdata([v[c] for row in add for v in row])
        s = s.resize(SCREEN, Image.BILINEAR, box=(0.5, 0.5, 0.5 + gw, 0.5 + gh))
        out.append(fmath(lambda a: a["c"] * a["m"] + a["s"], c=band, m=m, s=s))
    return out


def render(base, theta, look_doc):
    """The picture as the GPU would draw it: a PIL RGB image of 1920 x 1080, 8 bits."""
    from PIL import Image
    return Image.merge("RGB", [fmath(lambda a: to_bytes(a, a["f"]), f=band) for band in render_float(base, theta, look_doc)])


def planes16(bands):
    """The float bands as 16-bit planes in the order ffmpeg's gbrp16le wants (G, B, R), little endian: the encoder takes the precision the
    GPU's 8-bit framebuffer throws away, which is what keeps a slow change in a dark gradient smooth."""
    r, g, b = bands
    out = []
    for band in (g, b, r):
        i = fmath(lambda a: a["convert"](a["max"](a["min"](a["f"] * 257.0 + 0.5, 65535.0), 0.0), "I"), f=band)
        out.append(i.convert("I;16").tobytes())
    return b"".join(out)


def loop_angle(i, n):
    return 2.0 * math.pi * i / n


# ---- rendering the loop, in parallel and in order ------------------------------------------------------------------------------

_WORK = {}


def _init(base_path, look_doc, n):
    from PIL import Image
    _WORK["base"] = graded(Image.open(base_path).convert("RGB"), look_doc["motion"])
    _WORK["glow"] = glow_of(_WORK["base"]) if look_doc["motion"]["bloom"] else None
    _WORK["look"], _WORK["n"] = look_doc, n
    _WORK["bare"] = dict(look_doc, scrim=[0.0, 0.0, 0.0])       # the picture without the scrim: what the wallpaper sampler is shown


def _planes(i):
    return planes16(render_float(_WORK["base"], loop_angle(i, _WORK["n"]), _WORK["look"], _WORK["glow"]))


def _poster(i):
    """(picture i as drawn, picture i without the scrim), 8-bit RGB bytes."""
    from PIL import Image
    out = []
    for doc in (_WORK["look"], _WORK["bare"]):
        out.append(Image.merge("RGB", [fmath(lambda a: to_bytes(a, a["f"]), f=band) for band in render_float(_WORK["base"], loop_angle(i, _WORK["n"]), doc, _WORK["glow"])]).tobytes())
    return tuple(out)


def render_loop(base_path, look_doc, count, n, jobs):
    """Yields the planes of pictures 0..count-1 in order, `jobs` at a time and no more than twice that in memory."""
    with ProcessPoolExecutor(jobs, initializer=_init, initargs=(base_path, look_doc, n)) as pool:
        pending, nxt = [], 0
        while nxt < count or pending:
            while nxt < count and len(pending) < 2 * jobs:
                pending.append(pool.submit(_planes, nxt))
                nxt += 1
            yield pending.pop(0).result()


def render_posters(base_path, look_doc, n, indices, jobs):
    """{index: (as drawn, without the scrim)} for the poster frames."""
    with ProcessPoolExecutor(jobs, initializer=_init, initargs=(base_path, look_doc, n)) as pool:
        return dict(zip(indices, pool.map(_poster, indices)))



# ---- the stream -----------------------------------------------------------------------------------------------------------

def encoder_args(qp, frames, bits):
    """Like the art clips' (vplane_clips.encoder_args): one key picture, then predicted ones, one reference, no reordering. A constant
    quantiser, the same for every picture (ipratio and pbratio 1: the key picture the loop restarts at is the quality of the one before
    it), and the dark gradient gets the adaptive quantiser's help; why not a constant quality is in the docstring."""
    color = "colorprim=bt709:transfer=bt709:colormatrix=bt709"
    gop = "keyint=%d:min-keyint=%d:scenecut=0:bframes=0:ref=1:aud=1:repeat-headers=1" % (frames, frames)
    return ["-c:v", "libx265", "-preset", "slow", "-x265-params",
            "qp=%d:" % qp + gop + ":ipratio=1.0:pbratio=1.0:aq-mode=3:rc-lookahead=0:no-open-gop=1:info=0:frame-threads=1:pools=none:log-level=error:range=limited:" + color,
            "-f", "hevc"]


def encode_planes(planes_iter, count, qp, bits, content_fps, fps):
    """The pictures (16-bit planes) as an Annex B stream at `fps`, each repeated fps / content_fps times by the filter."""
    repeat = fps // content_fps
    pix = "yuv420p10le" if bits == 10 else "yuv420p"
    vf = "scale=in_range=full:out_range=tv:out_color_matrix=bt709:flags=lanczos+accurate_rnd,format=%s,fps=%d" % (pix, fps)
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "s.bin")
        cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "rawvideo", "-pix_fmt", "gbrp16le", "-s", "%dx%d" % SCREEN,
               "-framerate", str(content_fps), "-i", "-", "-vf", vf, "-pix_fmt", pix] + encoder_args(qp, count * repeat, bits) + [out]
        p = subprocess.Popen(cmd, stdin=subprocess.PIPE)
        for planes in planes_iter:
            p.stdin.write(planes)
        p.stdin.close()
        if p.wait() != 0:
            raise SystemExit("ffmpeg failed")
        with open(out, "rb") as f:
            return f.read()


# ---- what the shell is told: the poster frames' tint, and the glass over the plane -----------------------------------------

def downsample(rgb_bytes, size):
    from PIL import Image
    return Image.frombytes("RGB", SCREEN, rgb_bytes).resize(size, Image.BOX)


def luma_of(data, pixels):
    """Tint.luma: the mean luma 0..1 of the opaque pixels (data is r, g, b, a bytes)."""
    s = n = 0
    for i in range(pixels):
        if data[4 * i + 3] < 128:
            continue
        s += 0.299 * data[4 * i] + 0.587 * data[4 * i + 1] + 0.114 * data[4 * i + 2]
        n += 1
    return s / n / 255.0 if n else -1.0


def dominant_of(data, pixels):
    """Tint.dominant: 8 levels a channel, near-black skipped, the commonest bin wins (a tie goes to the lower bin), the bin's centre."""
    count, best, best_n = {}, -1, 0
    for i in range(pixels):
        r, g, b, a = data[4 * i:4 * i + 4]
        if a < 128 or (r < 32 and g < 32 and b < 32):
            continue
        key = ((r >> 5) << 6) | ((g >> 5) << 3) | (b >> 5)
        count[key] = count.get(key, 0) + 1
        n = count[key]
        if n > best_n or (n == best_n and key < best):
            best_n, best = n, key
    if best < 0:
        return None
    return {"r": ((best >> 6) & 7) * 32 + 16, "g": ((best >> 3) & 7) * 32 + 16, "b": (best & 7) * 32 + 16, "share": best_n / float(pixels)}


def sample_of(small):
    """What LumaHome's sampler finds in a picture it has grabbed at 32 x 18: (luma, dominant colour or None)."""
    data = list(small.convert("RGBA").tobytes())
    px = SAMPLE[0] * SAMPLE[1]
    return luma_of(data, px), dominant_of(data, px)


def hexc(c):
    return "#%02x%02x%02x" % (c["r"], c["g"], c["b"])


def linear(c):
    c = c / 255.0
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def relative_luminance(rgb):
    return 0.2126 * linear(rgb[0]) + 0.7152 * linear(rgb[1]) + 0.0722 * linear(rgb[2])


def contrast(a, b):
    ya, yb = relative_luminance(a), relative_luminance(b)
    return (max(ya, yb) + 0.05) / (min(ya, yb) + 0.05)


def over(top, alpha, below):
    return tuple(top[i] * alpha + below[i] * (1.0 - alpha) for i in range(3))


def grey_of_luminance(y):
    """The grey (0..255) whose relative luminance is `y`: contrast depends on luminance only."""
    g = 12.92 * y if y <= 0.0031308 else 1.055 * y ** (1 / 2.4) - 0.055
    return 255.0 * min(1.0, max(0.0, g))


def extremes(posters):
    """The brightest luminance any poster frame has at a point (the pictures area-averaged to 120 x 68: a word sits well inside a pane, so what
    is behind it is a few hundred pixels of the picture) and the mean colour of all of them."""
    best, total, n = 0.0, [0.0, 0.0, 0.0], 0
    for rgb in posters:
        data = downsample(rgb, (120, 68)).tobytes()
        for i in range(0, len(data), 3):
            best = max(best, relative_luminance((data[i], data[i + 1], data[i + 2])))
            total[0] += data[i]
            total[1] += data[i + 1]
            total[2] += data[i + 2]
            n += 1
    return best, tuple(t / n for t in total)


def glass_numbers(theme, worst, mean):
    """The tint-only glass over the plane: a pane that is `alpha` of its own colour over the video. Its own colour is the dark material's glass
    over the picture's mean colour (the flat backdrop the pane samples in this mode); the ink on it has to read at the TV's distance
    contrast (Lucent contrast.distance, 7) over the brightest thing the clip can put behind a word. `alpha` is the least that does it, in
    steps of 0.02 from 0.5, never below the material's own `readingAlpha`; `ratio` is the ink's contrast at that alpha."""
    dark = theme["dark"]
    ink, _ = vc.hexrgb(dark["ink"])
    glass, ga = vc.hexrgb(dark["glass"])
    pane = over(glass, ga, mean)
    g = grey_of_luminance(worst * WORST_MARGIN)
    video = (g, g, g)
    need = 7.0
    floor = float(dark.get("readingAlpha", 0.8))
    alpha, ratio = 1.0, contrast(ink, pane)
    for step in range(25, 51):
        a = step * 0.02
        r = contrast(ink, over(pane, a, video))
        if r >= need and a >= floor - 1e-9:
            alpha, ratio = round(a, 2), r
            break
    return {"alpha": alpha, "ratio": round(ratio, 2), "need": need, "worstLuminance": round(worst * WORST_MARGIN, 4),
            "pane": "#%02x%02x%02x" % tuple(int(round(v)) for v in pane), "flat": "#%02x%02x%02x" % tuple(int(round(v)) for v in mean)}


# ---- the manifest ----------------------------------------------------------------------------------------------------------------

def inputs_hash(src_sha, look_doc, opts):
    blob = json.dumps({"src": src_sha, "look": look_doc, "turns": TURNS, "grid": GRID, "v": WALL_VERSION, "opts": opts, "screen": SCREEN}, sort_keys=True)
    return hashlib.sha256(blob.encode()).hexdigest()[:16]


def load_manifest(directory):
    try:
        with open(os.path.join(directory, "wall.json")) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def cmd_wall(a):
    if not shutil.which("ffmpeg"):
        raise SystemExit("ffmpeg (with libx265, 10 bits) is needed to write clips")
    theme = read_theme()
    override = None
    if a.look:
        try:
            override = json.loads(a.look)
        except ValueError as e:
            raise SystemExit("--look is JSON, a wallpaperMotion object such as '{\"amplitude\": 12}': %s" % e)
        if not isinstance(override, dict) or not override:
            raise SystemExit("--look is a wallpaperMotion object with at least one setting")
        if a.id == "silk":
            raise SystemExit("a clip for a look of its own needs an id of its own (--id): `silk` is the shipped look's")
    look_doc = look_of(theme, override)
    m = look_doc["motion"]
    if m["speed"] != 1:
        raise SystemExit("the loop's pace is the clip's own: theme.wallpaperMotion.speed must be 1")
    src = wallpaper_path(look_doc["wallpaper"])
    n = a.loop_sec * a.content_fps
    count = a.frames_only or n
    qp = a.qp or QP[a.bit_depth]
    opts = {"codec": "hevc", "bits": a.bit_depth, "qp": qp, "loopSec": a.loop_sec, "contentFps": a.content_fps, "fps": FPS}
    inputs = inputs_hash(vc.sha(src), look_doc, opts)
    os.makedirs(a.out, exist_ok=True)
    name = "wall-%s" % a.id
    path = os.path.join(a.out, name + ".vpc")
    doc = load_manifest(a.out) or {"schema": 1, "walls": []}
    old = next((w for w in doc["walls"] if w["id"] == a.id), None)
    if a.reuse and old and old["inputs"] == inputs and os.path.exists(path) and vc.sha(path) == old["sha256"]:
        units = vc.read_vpc(path)["units"]                       # written from these inputs: the file stays, the numbers about it are made again
    else:
        def planes():
            for i, pl in enumerate(render_loop(src, look_doc, count, n, a.jobs)):
                if i % 60 == 0:
                    print("picture %d of %d" % (i, count), file=sys.stderr, flush=True)
                yield pl

        stream = encode_planes(planes(), count, qp, a.bit_depth, a.content_fps, FPS)
        units = vc.split(stream, "hevc")
        want = count * (FPS // a.content_fps)
        if len(units) != want or not units[0][1] or any(k for _, k in units[1:]):
            raise SystemExit("expected one key unit then %d others, got %d units" % (want - 1, len(units)))
        vc.write_vpc(path, "hevc", SCREEN, FPS, units)
    keep = sorted({round(i * n / POSTERS) % n for i in range(POSTERS)}) if not a.frames_only else list(range(0, count, max(1, count // POSTERS)))
    posters = render_posters(src, look_doc, n, keep, a.jobs)
    # the sampler is shown the picture without the scrim, as LumaHome's is shown the wallpaper's own: its numbers are what it finds there
    samples = [sample_of(downsample(posters[i][1], SAMPLE)) for i in keep]
    lumas = [s[0] for s in samples]
    bins = {}
    for _, t in samples:
        if t:
            bins[(t["r"], t["g"], t["b"])] = bins.get((t["r"], t["g"], t["b"]), 0) + 1
    top = max(bins.items(), key=lambda kv: (kv[1], tuple(-c for c in kv[0])))[0] if bins else None
    worst, mean = extremes([posters[i][0] for i in keep])        # the glass is over the picture as drawn, scrim and all
    tint = {"luma": round(sum(lumas) / len(lumas), 4), "lumaMin": round(min(lumas), 4), "lumaMax": round(max(lumas), 4),
            "tint": hexc({"r": top[0], "g": top[1], "b": top[2]}) if top else None,
            "posters": [{"at": round(i / n * a.loop_sec, 2), "luma": round(s[0], 4), "tint": hexc(s[1]) if s[1] else None} for i, s in zip(keep, samples)]}
    entry = {"id": a.id, "file": name + ".vpc", "codec": "hevc", "width": SCREEN[0], "height": SCREEN[1], "fps": FPS, "units": len(units),
             "loopSec": a.loop_sec, "contentFps": a.content_fps, "keyBytes": len(units[0][0]), "bytes": os.path.getsize(path), "sha256": vc.sha(path),
             "source": {"file": look_doc["wallpaper"], "sha256": vc.sha(src)}, "look": look_doc, "turns": TURNS, "tint": tint,
             "glass": glass_numbers(theme, worst, mean), "opts": opts, "inputs": inputs}
    if override:
        entry["override"] = override                           # the owner's look is laid over the theme's: `check` makes the same look from the theme again
    doc["generator"] = WALL_VERSION
    doc["walls"] = sorted([w for w in doc["walls"] if w["id"] != a.id] + [entry], key=lambda w: w["id"])
    with open(os.path.join(a.out, "wall.json"), "w") as f:
        json.dump(doc, f, indent=2)
        f.write("\n")
    print("%-12s %8d bytes (key unit %d), %d units, %d-bit QP %d, tint %s luma %.3f, tint glass alpha %.2f (%.1f:1)" %
          (name, entry["bytes"], entry["keyBytes"], len(units), a.bit_depth, qp, tint["tint"], tint["luma"], entry["glass"]["alpha"], entry["glass"]["ratio"]))
    return 0


def cmd_frame(a):
    from PIL import Image
    look_doc = look_of(read_theme(), json.loads(a.look) if a.look else None)
    base = graded(Image.open(wallpaper_path(look_doc["wallpaper"])).convert("RGB"), look_doc["motion"])
    render(base, 2.0 * math.pi * (a.t % a.loop_sec) / a.loop_sec, look_doc).save(a.out)
    return 0


# ---- check ----------------------------------------------------------------------------------------------------------------

def check(directory, deep=False):
    """Problems with the wall clips in `directory`, as a list of lines (empty when they are right)."""
    doc = load_manifest(directory)
    if doc is None:
        return ["%s: no wall.json" % directory]
    bad = []
    if doc.get("generator") != WALL_VERSION:
        bad.append("the wall clips were written by generator %s, this is %s" % (doc.get("generator"), WALL_VERSION))
    theme = read_theme()
    for w in doc["walls"]:
        want = look_of(theme, w.get("override"))
        wid = w["id"]
        path = os.path.join(directory, w["file"])
        if not os.path.exists(path):
            bad.append("%s: file missing" % wid)
            continue
        if vc.sha(path) != w["sha256"]:
            bad.append("%s: not the file wall.json was written for" % wid)
            continue
        try:
            v = vc.read_vpc(path)
        except ValueError as e:
            bad.append("%s: %s" % (wid, e))
            continue
        units = v["units"]
        if not units or not units[0][1] or any(k for _, k in units[1:]) or v["loopFrom"] != 0:
            bad.append("%s: expected one key unit first, then predicted ones, looping to unit 0" % wid)
        if (v["width"], v["height"], v["fps"]) != (SCREEN[0], SCREEN[1], FPS) or v["codec"] != "hevc":
            bad.append("%s: %s %dx%d at %s fps, expected hevc %dx%d at %d" % (wid, v["codec"], v["width"], v["height"], v["fps"], SCREEN[0], SCREEN[1], FPS))
        if len(units) != w["loopSec"] * FPS:
            bad.append("%s: %d units for a loop of %d s at %d" % (wid, len(units), w["loopSec"], FPS))
        if w["look"] != want:
            moved = []
            for k in want:
                if w["look"].get(k) == want[k]:
                    continue
                if isinstance(want[k], dict):
                    moved += ["%s.%s" % (k, s) for s in want[k] if (w["look"].get(k) or {}).get(s) != want[k][s]]
                else:
                    moved.append(k)
            bad.append("%s: the wallpaper's look moved since the clip was written (%s): run `wall` again" % (wid, ", ".join(moved)))
        src = os.path.join(WALLPAPER_DIR, w["source"]["file"])
        if not os.path.exists(src) or vc.sha(src) != w["source"]["sha256"]:
            bad.append("%s: the wallpaper %s is not the picture the clip was made from" % (wid, w["source"]["file"]))
        if w["turns"] != TURNS:
            bad.append("%s: the phases' turns are not this generator's" % wid)
        if w["inputs"] != inputs_hash(w["source"]["sha256"], w["look"], w["opts"]):
            bad.append("%s: the picture, the look or the encoder settings changed since this clip was written" % wid)
        if deep:
            bad += deep_check(wid, v, w)
    return bad


def pair_stats(v):
    """What changes between consecutive pictures of two plays of the clip, in levels of 8 bits: ffmpeg decodes the stream twice over (the second
    play begins at the key picture, as the player's loop does), `tblend` takes the difference of each picture with the one before and
    `signalstats` reads its mean (YAVG), its 90th percentile (YHIGH) and its largest (YMAX). Returns one (mean, high, max) per pair; pair
    j is picture j + 1 against picture j, so the seam (picture n against n - 1) is pair n - 1."""
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "s.bin")
        blob = b"".join(u for u, _ in v["units"])
        with open(path, "wb") as f:
            f.write(blob + blob)
        out = subprocess.run(["ffprobe", "-v", "error", "-f", "lavfi", "-i", "movie=%s,tblend=all_mode=difference,signalstats" % path,
                              "-show_entries", "frame_tags=lavfi.signalstats.YAVG,lavfi.signalstats.YHIGH,lavfi.signalstats.YMAX", "-of", "csv=p=0"],
                             capture_output=True, text=True)
    rows = [r.split(",") for r in out.stdout.split() if r.strip()]
    pairs = []
    for r in rows:
        try:
            pairs.append(tuple(float(x) for x in r[:3]))
        except ValueError:
            return None
    return pairs


def deep_check(name, v, w):
    """The stream decodes and the loop is seamless: the step across the seam is a fraction of a level, and no larger than the steps the loop
    makes anywhere else by more than the key picture's own different rounding explains."""
    bad = vc.deep_check(name, v)
    if not shutil.which("ffprobe"):
        return bad
    pairs = pair_stats(v)
    n = len(v["units"])
    if not pairs or len(pairs) < 2 * n - 1:
        return bad + ["%s: ffprobe could not measure the loop's seam" % name]
    scale = 4.0 if w["opts"]["bits"] == 10 else 1.0            # the measurement is in the stream's own levels
    repeat = FPS // w["contentFps"]
    inside = [pairs[k * repeat - 1][0] / scale for k in range(1, n // repeat)]      # the pairs where the content moves
    seam = pairs[n - 1]
    seam_mean, seam_high, seam_max = seam[0] / scale, seam[1] / scale, seam[2] / scale
    inside.sort()
    typical = inside[len(inside) // 2]
    if seam_mean > 0.6 or seam_high > 2.0:
        bad.append("%s: the loop's seam is a step of %.3f levels on average (90th percentile %.2f, largest %.1f); a typical step inside the loop is %.3f" %
                   (name, seam_mean, seam_high, seam_max, typical))
    # the repeats cost nothing: a picture shown again does not change
    flat = [pairs[j][0] / scale for j in range(n - 1) if (j + 1) % repeat]
    if flat and max(flat) > 0.5:
        bad.append("%s: a repeated picture changes by %.2f levels (the predicted pictures were meant to copy)" % (name, max(flat)))
    return bad


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    wl = sub.add_parser("wall")
    wl.add_argument("--out", default=OUT)
    wl.add_argument("--id", default="silk")
    wl.add_argument("--loop-sec", type=int, default=LOOP_SEC)
    wl.add_argument("--content-fps", type=int, default=CONTENT_FPS)
    wl.add_argument("--bit-depth", type=int, choices=(8, 10), default=10)
    wl.add_argument("--qp", type=int, default=0, help="the constant quantiser (default: the one each bit depth was measured at)")
    wl.add_argument("--jobs", type=int, default=max(1, min(8, (os.cpu_count() or 2) - 1)))
    wl.add_argument("--frames-only", type=int, default=0, help="tests: render only this many pictures of the loop")
    wl.add_argument("--reuse", action="store_true", help="keep the clip file when it was written from these inputs; make the numbers about it again")
    wl.add_argument("--look", default="", help="a wallpaperMotion object laid over the theme's, for a clip of a look the owner has tuned (needs its own --id)")
    fr = sub.add_parser("frame")
    fr.add_argument("--out", required=True)
    fr.add_argument("--t", type=float, default=0.0)
    fr.add_argument("--loop-sec", type=int, default=LOOP_SEC)
    fr.add_argument("--look", default="", help="a wallpaperMotion object laid over the theme's")
    ck = sub.add_parser("check")
    ck.add_argument("--dir", default=OUT)
    ck.add_argument("--deep", action="store_true")
    a = ap.parse_args()
    if a.cmd == "wall":
        return cmd_wall(a)
    if a.cmd == "frame":
        return cmd_frame(a)
    bad = check(a.dir, a.deep)
    for line in bad:
        print(line)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
