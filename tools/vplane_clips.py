#!/usr/bin/env python3
"""Writes and checks the clips the video-plane player (vplane/) plays: the art mode's prints as stills on a video window.

    python3 tools/vplane_clips.py art    [--out DIR] [--only ID,ID] [--codec hevc|h264] [--crf N] [--loop-sec S] [--fps N]
    python3 tools/vplane_clips.py glyphs [--out DIR]
    python3 tools/vplane_clips.py check  [--dir DIR] [--deep]

What a clip is. The art mode is a still: a print in its matte and frame on a dark wall (panels/ArtPiece.qml). On a video plane a
still costs the decoder one picture, so a clip is one IDR picture of the composition followed by predicted pictures that copy it
(about 170 bytes each), `--loop-sec` seconds of them at the display's own rate; the player feeds them in order and starts again from
the IDR. The picture is 1856 x 1024, not 1920 x 1080: the player drifts the picture by moving its window, a few pixels every few
seconds (the art mode's burn-in rule), and the 32 and 28 pixel margins are what it drifts in. The outermost 40 pixels of the
picture fade to pure black so the window's edge never shows against the black around it, wherever it is. The rate is 60: the UI
plane's refresh follows the video pipeline's rate (docs/research/perf-tweaks.md: a browser page held the output at 50 Hz), and a
24 or 30 fps clip may hold it there.

The composition is the art mode's own: the numbers (print box, matte, frame, colours, wall gradient, safe margins, brightness) are
read from ArtPiece.qml, Screensaver.qml and theme.json, never typed here, and `check` fails when one of them has moved since the
clips were written. The caption and the time in the corner are not in the picture: the dim is a black veil over the picture, and
text under it would dim with it (the caption's contrast against the wall would fall below Lucent's 3:1 for large text at the deepest
dim; the GPU saver keeps its caption above the dimming layer). The player draws the caption from a coverage bitmap per print
(`<id>.vpt`) and the time from the glyph atlas (`glyphs`), both above the veil, in the colours the request gives them.

The container (`.vpc`, read by vplane/src/vpl_vpc.c): little endian,
    0   "VPC1"
    4   u8 version (1), u8 codec (1 H.264, 2 H.265: NDL's NDL_VIDEO_TYPE), u16 flags (0)
    8   u32 width, u32 height, u32 fps numerator, u32 fps denominator
    24  u32 access units, u32 loop_from (the key access unit playback returns to), u32 payload offset (40 + 8 x units), u32 0
    40  per access unit: u32 size, u32 flags (bit 0: key)
        then every access unit, Annex B, the parameter sets in front of each key one.
The glyph atlas (`.vpg`): "VPG1", u32 count, u32 right edge, u32 baseline (screen pixels of the corner clock), u32 line height, then per glyph u32 codepoint,
i32 advance, i32 left, i32 top, u32 width, u32 height, u32 offset into the alpha plane that follows.
The caption (`.vpt`): "VPT1", u32 width, u32 height, i32 x, i32 y (screen pixels of its top left), u32 split (columns left of it are the artist's name,
in the first ink, the rest in the second), then width x height bytes of coverage.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
QML = os.path.join(ROOT, "payload", "compositor", "lumatv")
ART = os.path.join(QML, "art")
THEME = os.path.join(ROOT, "payload", "theme", "theme.json")
FONTS = os.path.join(ROOT, "payload", "fonts")
OUT = os.path.join(ROOT, "payload", "vplane", "clips")

VERSION = 1                     # the generator's: part of what `check` compares, so a change here asks for new clips
SCREEN = (1920, 1080)
CANVAS = (1856, 1024)           # the picture; margins of 32 and 28 pixels for the player's drift
FADE = 40                       # the picture's outer pixels fade to black
CODEC = {"h264": 1, "hevc": 2}
CLOCK_PX = 32                   # the corner clock, Screensaver.qml: role label, pixelSize 32, Medium
CAPTION_PX = 24                 # the caption: role label, pixelSize 24
GLYPHS = "0123456789:"

AUD = {"h264": 9, "hevc": 35}   # access unit delimiter NAL types: each access unit starts with one (the encoders are told to)


# ---- the numbers, from the sources that own them ----------------------------------------------------------------------------

def _int(src, name):
    m = re.search(r"property int %s:\s*(-?\d+)" % name, src)
    if not m:
        raise SystemExit("no `property int %s` in the QML" % name)
    return int(m.group(1))


def geometry():
    """What the composition is made of, read from the QML and the theme. (The ink colours are the player's request's, not the clip's.)"""
    with open(os.path.join(QML, "panels", "ArtPiece.qml")) as f:
        piece = f.read()
    with open(os.path.join(QML, "panels", "Screensaver.qml")) as f:
        saver = f.read()
    with open(THEME) as f:
        theme = json.load(f)
    colors = re.findall(r'color:\s*"(#[0-9A-Fa-f]{6})"', piece)
    if len(colors) < 2:
        raise SystemExit("ArtPiece.qml: the frame and matte colours were not found")
    stops = re.findall(r'GradientStop\s*\{\s*position:\s*([0-9.]+);\s*color:\s*"(#[0-9A-Fa-f]{6})"', saver)
    if len(stops) != 3:
        raise SystemExit("Screensaver.qml: the wall's three gradient stops were not found")
    ss = theme["screensaver"]
    return {
        "maxW": _int(piece, "maxW"), "maxH": _int(piece, "maxH"), "matte": _int(piece, "matte"), "frame": _int(piece, "frame"),
        "frameColor": colors[0], "matteColor": colors[1],
        "wall": [[float(p), c] for p, c in stops],
        "safeX": _int(saver, "safeX"), "safeY": _int(saver, "safeY"), "edgeX": _int(saver, "edgeX"),
        "artLevel": ss["artLevel"],
    }


def drift_numbers():
    """The art mode's shift, from Screensaver.qml's shift timer: round(ampX sin(2 pi t / periodX)), round(ampY sin(2 pi t / periodY)).
    The picture has to have the room to move in: a margin of the screen around it at least as wide as the reach."""
    with open(os.path.join(QML, "panels", "Screensaver.qml")) as f:
        saver = f.read()
    mx = re.search(r"art\.dx\s*=\s*Math\.round\((\d+)\s*\*\s*Math\.sin\(2 \* Math\.PI \* t / (\d+)\)\)", saver)
    my = re.search(r"art\.dy\s*=\s*Math\.round\((\d+)\s*\*\s*Math\.sin\(2 \* Math\.PI \* t / (\d+)\)\)", saver)
    if not mx or not my:
        raise SystemExit("Screensaver.qml: the shift timer's art.dx and art.dy were not found")
    return {"ampX": int(mx.group(1)), "periodX": int(mx.group(2)), "ampY": int(my.group(1)), "periodY": int(my.group(2))}


def hexrgb(c):
    c = c.lstrip("#")
    if len(c) == 8:           # AARRGGBB, Qt's order
        a = int(c[0:2], 16) / 255.0
        return tuple(int(c[i:i + 2], 16) for i in (2, 4, 6)), a
    return tuple(int(c[i:i + 2], 16) for i in (0, 2, 4)), 1.0


def font(weight, px):
    from PIL import ImageFont
    return ImageFont.truetype(os.path.join(FONTS, "Manrope-%s.ttf" % weight), px)


# ---- the picture -------------------------------------------------------------------------------------------------------------

def smoothstep(a, b, x):
    t = min(1.0, max(0.0, (x - a) / float(b - a)))
    return t * t * (3 - 2 * t)


def wall_color(geo, y):
    """Screensaver.qml's wall: the gradient over the whole screen height."""
    t = (y + 0.5) / SCREEN[1]
    stops = [(p, hexrgb(c)[0]) for p, c in geo["wall"]]
    for (p0, c0), (p1, c1) in zip(stops, stops[1:]):
        if t <= p1:
            k = 0.0 if p1 == p0 else (t - p0) / (p1 - p0)
            return tuple(int(round(c0[i] + (c1[i] - c0[i]) * max(0.0, min(1.0, k)))) for i in range(3))
    return stops[-1][1]


def compose(entry, geo):
    """The art mode's picture for one print: wall, framed print at the art level, black edges. A PIL RGB image of CANVAS."""
    from PIL import Image, ImageChops, ImageDraw
    sw, sh = SCREEN
    img = Image.new("RGB", SCREEN)
    d = ImageDraw.Draw(img)
    for y in range(sh):
        d.line([(0, y), (sw, y)], fill=wall_color(geo, y))
    fit = min(geo["maxW"] / entry["width"], geo["maxH"] / entry["height"])
    pw, ph = round(entry["width"] * fit), round(entry["height"] * fit)
    m, fr = geo["matte"], geo["frame"]
    fw, fh = pw + 2 * (m + fr), ph + 2 * (m + fr)
    fx, fy = (sw - fw) // 2, (sh - fh) // 2             # anchors.centerIn: the QML centres on the screen
    d.rectangle([fx, fy, fx + fw - 1, fy + fh - 1], fill=hexrgb(geo["frameColor"])[0])
    d.rectangle([fx + fr, fy + fr, fx + fw - fr - 1, fy + fh - fr - 1], fill=hexrgb(geo["matteColor"])[0])
    with Image.open(os.path.join(ART, entry["file"])) as src:
        print_ = src.convert("RGB").resize((pw, ph), Image.LANCZOS)
    img.paste(print_, (fx + fr + m, fy + fr + m))
    level = geo["artLevel"]
    img = img.point([round(v * level) for v in range(256)] * 3)      # the QML's black layer over the pieces: the art level
    cw, ch = CANVAS
    mx, my = (sw - cw) // 2, (sh - ch) // 2
    img = img.crop((mx, my, mx + cw, my + ch))
    ramp_x = Image.new("L", (cw, 1))
    ramp_x.putdata([round(255 * smoothstep(0, FADE, min(i, cw - 1 - i))) for i in range(cw)])
    ramp_y = Image.new("L", (1, ch))
    ramp_y.putdata([round(255 * smoothstep(0, FADE, min(j, ch - 1 - j))) for j in range(ch)])
    mask = ImageChops.multiply(ramp_x.resize((cw, ch)), ramp_y.resize((cw, ch)))
    return ImageChops.multiply(img, Image.merge("RGB", (mask, mask, mask)))


def caption_text(entry):
    """The art mode's caption, as Screensaver.qml writes it: the artist in bold, then the title, the year and the licence."""
    return entry["artist"], " \u00b7 %s, %s \u00b7 %s" % (entry["title"], entry["year"], entry["licence"])


def caption_bitmap(entry, geo):
    """The caption as coverage: (PIL 'L' image, split, x, y) with the bitmap's top left in screen pixels. The box is the QML's: left at
    edgeX, the line ending safeY above the bottom of the screen (the baseline 8 pixels above that, a 24 px face)."""
    from PIL import Image, ImageDraw
    bold, plain = font("Bold", CAPTION_PX), font("Medium", CAPTION_PX)
    artist, rest = caption_text(entry)
    split = round(bold.getlength(artist))
    width = split + round(plain.getlength(rest)) + 4
    asc, desc = CAPTION_PX, 8                      # room above and below the baseline for the face's ascenders and descenders
    im = Image.new("L", (width, asc + desc), 0)
    d = ImageDraw.Draw(im)
    d.text((0, asc), artist, font=bold, fill=255, anchor="ls")
    d.text((split, asc), rest, font=plain, fill=255, anchor="ls")
    base = SCREEN[1] - geo["safeY"] - 8
    return im, split, geo["edgeX"], base - asc


def write_vpt(path, bitmap):
    im, split, x, y = bitmap
    with open(path, "wb") as f:
        f.write(b"VPT1" + struct.pack("<IIiiI", im.width, im.height, x, y, split) + im.tobytes())


def read_vpt(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"VPT1":
        raise ValueError("not a VPT1 file")
    w, h, x, y, split = struct.unpack_from("<IIiiI", data, 4)
    if len(data) != 24 + w * h or split > w:
        raise ValueError("bad VPT1 size")
    return {"width": w, "height": h, "x": x, "y": y, "split": split, "alpha": data[24:]}


# ---- the stream ---------------------------------------------------------------------------------------------------------------

def encoder_args(codec, crf, loop_frames, fps):
    color = "colorprim=bt709:transfer=bt709:colormatrix=bt709"
    gop = "keyint=%d:min-keyint=%d:scenecut=0:bframes=0:ref=1:aud=1:repeat-headers=1" % (loop_frames, loop_frames)
    if codec == "hevc":
        return ["-c:v", "libx265", "-preset", "slow", "-crf", str(crf), "-x265-params",
                gop + ":rc-lookahead=0:no-open-gop=1:info=0:frame-threads=1:pools=none:log-level=error:range=limited:" + color, "-f", "hevc"]
    return ["-c:v", "libx264", "-preset", "slow", "-crf", str(crf), "-profile:v", "high", "-level", "4.1", "-x264-params",
            gop + ":rc-lookahead=0:range=tv:" + color, "-f", "h264"]


def encode(png, codec, crf, frames, fps):
    """One picture repeated `frames` times, as an Annex B stream. The encoder sees a still, so everything after the IDR is a skip."""
    vf = "scale=in_range=full:out_range=tv:out_color_matrix=bt709:flags=lanczos+accurate_rnd,format=yuv420p"
    with tempfile.TemporaryDirectory() as d:
        out = os.path.join(d, "s.bin")
        cmd = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-loop", "1", "-framerate", str(fps), "-i", png,
               "-frames:v", str(frames), "-vf", vf, "-pix_fmt", "yuv420p"] + encoder_args(codec, crf, frames, fps) + [out]
        subprocess.run(cmd, check=True, stderr=subprocess.DEVNULL if os.environ.get("VPLANE_QUIET") else None)
        with open(out, "rb") as f:
            return f.read()


def nals(data):
    """(offset of the start code, offset of the NAL header byte) for every NAL unit of an Annex B stream."""
    out, i, n = [], 0, len(data)
    while True:
        j = data.find(b"\x00\x00\x01", i)
        if j < 0:
            return out
        start = j - 1 if j > 0 and data[j - 1] == 0 else j
        out.append((start, j + 3))
        i = j + 3
        if i >= n:
            return out


def nal_type(codec, data, header):
    return (data[header] >> 1) & 0x3F if codec == "hevc" else data[header] & 0x1F


def split(data, codec):
    """[(bytes, key)] one entry per access unit: a unit starts at an access unit delimiter and runs to the next one."""
    marks = nals(data)
    cuts = [s for s, h in marks if nal_type(codec, data, h) == AUD[codec]]
    if not cuts or cuts[0] != 0:
        raise SystemExit("the stream does not start with an access unit delimiter (the encoder was not told to write them)")
    cuts.append(len(data))
    key_types = (19, 20) if codec == "hevc" else (5,)
    units = []
    for a, b in zip(cuts, cuts[1:]):
        chunk = data[a:b]
        key = any(nal_type(codec, data, h) in key_types for s, h in marks if a <= s < b)
        units.append((chunk, key))
    return units


def write_vpc(path, codec, size, fps, units, loop_from=0):
    w, h = size
    head = b"VPC1" + struct.pack("<BBH", 1, CODEC[codec], 0) + struct.pack("<IIII", w, h, fps, 1)
    head += struct.pack("<IIII", len(units), loop_from, 40 + 8 * len(units), 0)
    table = b"".join(struct.pack("<II", len(u), 1 if key else 0) for u, key in units)
    with open(path, "wb") as f:
        f.write(head + table + b"".join(u for u, _ in units))


def read_vpc(path):
    """The header, the table and the units of a .vpc: a dict with `units` a list of (bytes, key)."""
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"VPC1":
        raise ValueError("not a VPC1 file")
    version, codec, flags = struct.unpack_from("<BBH", data, 4)
    w, h, num, den = struct.unpack_from("<IIII", data, 8)
    count, loop_from, off, _ = struct.unpack_from("<IIII", data, 24)
    if version != 1 or codec not in (1, 2) or off != 40 + 8 * count or off > len(data):
        raise ValueError("bad VPC1 header")
    units, pos = [], off
    for i in range(count):
        size, fl = struct.unpack_from("<II", data, 40 + 8 * i)
        if pos + size > len(data):
            raise ValueError("access unit %d runs past the file" % i)
        units.append((data[pos:pos + size], bool(fl & 1)))
        pos += size
    if pos != len(data):
        raise ValueError("%d bytes after the last access unit" % (len(data) - pos))
    return {"codec": "h264" if codec == 1 else "hevc", "width": w, "height": h, "fps": num / den, "fpsNum": num, "fpsDen": den,
            "loopFrom": loop_from, "units": units}


# ---- the corner clock's glyphs -----------------------------------------------------------------------------------------------

def write_glyphs(path, geo):
    """The digits and the colon of the corner clock, rendered the way the QML does it (Manrope Medium, 32 px), as one alpha plane."""
    f = font("Medium", CLOCK_PX)
    glyphs, plane = [], bytearray()
    from PIL import Image, ImageDraw
    for ch in GLYPHS:
        l, t, r, b = f.getbbox(ch, anchor="ls")
        w, h = max(1, r - l), max(1, b - t)
        im = Image.new("L", (w, h), 0)
        ImageDraw.Draw(im).text((-l, -t), ch, font=f, fill=255, anchor="ls")
        glyphs.append((ord(ch), int(round(f.getlength(ch))), l, -t, w, h, len(plane)))
        plane += im.tobytes()
    right = SCREEN[0] - geo["edgeX"]
    baseline = SCREEN[1] - geo["safeY"] - 11
    head = b"VPG1" + struct.pack("<IIII", len(glyphs), right, baseline, round(CLOCK_PX * 1.3))
    body = b"".join(struct.pack("<IiiiIII", *g) for g in glyphs)
    with open(path, "wb") as fh:
        fh.write(head + body + bytes(plane))


def read_glyphs(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"VPG1":
        raise ValueError("not a VPG1 file")
    count, right, baseline, height = struct.unpack_from("<IIII", data, 4)
    base = 20 + 28 * count
    out = {}
    for i in range(count):
        cp, adv, left, top, w, h, off = struct.unpack_from("<IiiiIII", data, 20 + 28 * i)
        out[chr(cp)] = {"advance": adv, "left": left, "top": top, "w": w, "h": h, "alpha": data[base + off:base + off + w * h]}
    return {"right": right, "baseline": baseline, "height": height, "glyphs": out}


# ---- index --------------------------------------------------------------------------------------------------------------------

def sha(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def inputs_hash(entry, geo, opts):
    """What a clip was made from: the picture's file, the composition's numbers, the generator's version and its encoder settings."""
    blob = json.dumps({"sha256": entry["sha256"], "geo": geo, "v": VERSION, "opts": opts, "canvas": CANVAS}, sort_keys=True)
    return hashlib.sha256(blob.encode()).hexdigest()[:16]


def caption_inputs(entry, geo):
    """What a caption bitmap was made from: its words, the face and size, and where the QML puts it."""
    blob = json.dumps({"words": caption_text(entry), "px": CAPTION_PX, "edgeX": geo["edgeX"], "safeY": geo["safeY"], "screen": SCREEN, "v": VERSION}, sort_keys=True)
    return hashlib.sha256(blob.encode()).hexdigest()[:16]


def pictures():
    with open(os.path.join(ART, "index.json")) as f:
        doc = json.load(f)
    # the art mode shows public-domain pictures only (Screensaver.qml filters the same way)
    return [p for p in doc["pictures"] if p["licence"] == "Public domain"]


def cmd_art(a):
    if not shutil.which("ffmpeg"):
        raise SystemExit("ffmpeg (with libx265 or libx264) is needed to write clips")
    geo = geometry()
    only = set(a.only.split(",")) if a.only else None
    os.makedirs(a.out, exist_ok=True)
    frames = a.loop_sec * a.fps
    opts = {"codec": a.codec, "crf": a.crf, "frames": frames, "fps": a.fps}
    old = {}
    if os.path.exists(os.path.join(a.out, "index.json")):
        with open(os.path.join(a.out, "index.json")) as f:
            old = {c["id"]: c for c in json.load(f).get("clips", [])}
    clips = []
    for entry in pictures():
        if only and entry["id"] not in only:
            if entry["id"] in old:
                clips.append(old[entry["id"]])
            continue
        with tempfile.TemporaryDirectory() as d:
            png = os.path.join(d, "canvas.png")
            compose(entry, geo).save(png)
            stream = encode(png, a.codec, a.crf, frames, a.fps)
        units = split(stream, a.codec)
        if len(units) != frames or not units[0][1] or any(k for _, k in units[1:]):
            raise SystemExit("%s: expected one key unit then %d others, got %d units" % (entry["id"], frames - 1, len(units)))
        path = os.path.join(a.out, entry["id"] + ".vpc")
        write_vpc(path, a.codec, CANVAS, a.fps, units)
        cap_path = os.path.join(a.out, entry["id"] + ".vpt")
        bm = caption_bitmap(entry, geo)
        write_vpt(cap_path, bm)
        clips.append({"id": entry["id"], "file": entry["id"] + ".vpc", "codec": a.codec, "width": CANVAS[0], "height": CANVAS[1],
                      "fps": a.fps, "units": len(units), "keyBytes": len(units[0][0]), "bytes": os.path.getsize(path), "sha256": sha(path),
                      "opts": opts, "inputs": inputs_hash(entry, geo, opts),
                      "caption": {"file": entry["id"] + ".vpt", "bytes": os.path.getsize(cap_path), "sha256": sha(cap_path), "width": bm[0].width,
                                  "height": bm[0].height, "split": bm[1], "x": bm[2], "y": bm[3], "inputs": caption_inputs(entry, geo)}})
        print("%-20s %6d bytes, key unit %6d, %d units, caption %dx%d" % (entry["id"], clips[-1]["bytes"], clips[-1]["keyBytes"], len(units), bm[0].width, bm[0].height))
    glyphs = os.path.join(a.out, "clock.vpg")
    write_glyphs(glyphs, geo)
    with open(os.path.join(a.out, "index.json"), "w") as f:
        json.dump({"schema": 1, "generator": VERSION, "canvas": list(CANVAS), "geometry": geo, "drift": drift_numbers(), "clips": clips,
                   "glyphs": {"file": "clock.vpg", "sha256": sha(glyphs)}}, f, indent=2)
        f.write("\n")
    return 0


def cmd_glyphs(a):
    os.makedirs(a.out, exist_ok=True)
    write_glyphs(os.path.join(a.out, "clock.vpg"), geometry())
    return 0


def check(directory, deep=False):
    """Problems with the clips in `directory`, as a list of lines (empty when they are right)."""
    bad = []
    try:
        with open(os.path.join(directory, "index.json")) as f:
            doc = json.load(f)
    except OSError:
        return ["%s: no index.json" % directory]
    geo = geometry()
    room = ((SCREEN[0] - CANVAS[0]) // 2, (SCREEN[1] - CANVAS[1]) // 2)
    drift = drift_numbers()
    if doc.get("drift") != drift:
        bad.append("the shift in Screensaver.qml is %s, the index has %s: run `art` again" % (drift, doc.get("drift")))
    if drift["ampX"] > room[0] or drift["ampY"] > room[1]:
        bad.append("the picture has %dx%d pixels of room and the shift reaches %dx%d: the clips need a smaller picture" % (room[0], room[1], drift["ampX"], drift["ampY"]))
    if doc.get("generator") != VERSION:
        bad.append("the clips were written by generator %s, this is %s" % (doc.get("generator"), VERSION))
    if doc.get("geometry") != geo:
        moved = [k for k in geo if doc.get("geometry", {}).get(k) != geo[k]]
        bad.append("the art mode's numbers moved since the clips were written (%s): run `art` again" % ", ".join(moved))
    ids = {p["id"]: p for p in pictures()}
    have = {c["id"] for c in doc["clips"]}
    for missing in sorted(set(ids) - have):
        bad.append("%s: a public-domain print with no clip" % missing)
    for extra in sorted(have - set(ids)):
        bad.append("%s: a clip of a print the art mode does not show" % extra)
    for c in doc["clips"]:
        path = os.path.join(directory, c["file"])
        if not os.path.exists(path):
            bad.append("%s: file missing" % c["id"])
            continue
        if sha(path) != c["sha256"]:
            bad.append("%s: not the file the index was written for" % c["id"])
            continue
        try:
            v = read_vpc(path)
        except ValueError as e:
            bad.append("%s: %s" % (c["id"], e))
            continue
        units = v["units"]
        if not units or not units[0][1] or any(k for _, k in units[1:]) or v["loopFrom"] != 0:
            bad.append("%s: expected one key unit first, then predicted ones, looping to unit 0" % c["id"])
        if (v["width"], v["height"]) != CANVAS or v["fps"] != 60:
            bad.append("%s: %dx%d at %s fps, expected %dx%d at 60" % (c["id"], v["width"], v["height"], v["fps"], CANVAS[0], CANVAS[1]))
        if c["id"] in ids and c["inputs"] != inputs_hash(ids[c["id"]], geo, c["opts"]):
            bad.append("%s: the print, the composition or the encoder settings changed since this clip was written" % c["id"])
        if deep:
            bad += deep_check(c["id"], v)
        cap = c.get("caption")
        if not cap:
            bad.append("%s: no caption in the index" % c["id"])
            continue
        cpath = os.path.join(directory, cap["file"])
        if not os.path.exists(cpath):
            bad.append("%s: caption file missing" % c["id"])
        elif sha(cpath) != cap["sha256"]:
            bad.append("%s: the caption is not the file the index was written for" % c["id"])
        else:
            try:
                got = read_vpt(cpath)
                if (got["width"], got["height"], got["x"], got["y"], got["split"]) != (cap["width"], cap["height"], cap["x"], cap["y"], cap["split"]):
                    bad.append("%s: the caption's size or place differs from the index" % c["id"])
            except ValueError as e:
                bad.append("%s: caption: %s" % (c["id"], e))
        if c["id"] in ids and cap["inputs"] != caption_inputs(ids[c["id"]], geo):
            bad.append("%s: the caption's words or place changed since it was written" % c["id"])
    try:
        g = read_glyphs(os.path.join(directory, doc["glyphs"]["file"]))
        if sha(os.path.join(directory, doc["glyphs"]["file"])) != doc["glyphs"]["sha256"]:
            bad.append("clock.vpg: not the file the index was written for")
        if set(g["glyphs"]) != set(GLYPHS):
            bad.append("clock.vpg: glyphs %r, expected %r" % ("".join(sorted(g["glyphs"])), GLYPHS))
    except (OSError, ValueError, KeyError) as e:
        bad.append("clock.vpg: %s" % e)
    return bad


def deep_check(name, v):
    """The stream decodes: ffprobe counts the pictures and reads the size, the profile and the reference depth."""
    if not shutil.which("ffprobe"):
        return []
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, "s.bin")
        with open(path, "wb") as f:
            f.write(b"".join(u for u, _ in v["units"]))
        out = subprocess.run(["ffprobe", "-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries",
                              "stream=codec_name,width,height,nb_read_frames,has_b_frames,refs,profile", "-of", "json", "-f",
                              "hevc" if v["codec"] == "hevc" else "h264", path], capture_output=True, text=True)
        try:
            s = json.loads(out.stdout)["streams"][0]
        except (ValueError, KeyError, IndexError):
            return ["%s: ffprobe could not read the stream: %s" % (name, out.stderr.strip()[:200])]
    bad = []
    if int(s["nb_read_frames"]) != len(v["units"]):
        bad.append("%s: %s pictures decoded, %d units written" % (name, s["nb_read_frames"], len(v["units"])))
    if (s["width"], s["height"]) != (v["width"], v["height"]):
        bad.append("%s: the stream is %dx%d, the header says %dx%d" % (name, s["width"], s["height"], v["width"], v["height"]))
    if s.get("has_b_frames", 0) != 0 or int(s.get("refs", 1)) > 1:
        bad.append("%s: reordering or more than one reference picture (the decoder would hold more frames than it needs)" % name)
    return bad


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    art = sub.add_parser("art")
    art.add_argument("--out", default=OUT)
    art.add_argument("--only", default="")
    art.add_argument("--codec", choices=sorted(CODEC), default="hevc")
    art.add_argument("--crf", type=int, default=16)
    art.add_argument("--loop-sec", type=int, default=2)
    art.add_argument("--fps", type=int, default=60)
    gl = sub.add_parser("glyphs")
    gl.add_argument("--out", default=OUT)
    ck = sub.add_parser("check")
    ck.add_argument("--dir", default=OUT)
    ck.add_argument("--deep", action="store_true", help="also decode every clip with ffprobe")
    a = ap.parse_args()
    if a.cmd == "art":
        return cmd_art(a)
    if a.cmd == "glyphs":
        return cmd_glyphs(a)
    bad = check(a.dir, a.deep)
    for line in bad:
        print(line)
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
