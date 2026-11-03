#!/usr/bin/env python3
"""Shared video-plane clip and bitmap format helpers."""
import hashlib
import json
import os
import shutil
import struct
import subprocess
import tempfile

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
THEME = os.path.join(ROOT, "payload", "theme", "theme.json")
OUT = os.path.join(ROOT, "payload", "vplane", "clips")
SCREEN = (1920, 1080)
CODEC = {"h264": 1, "hevc": 2}
AUD = {"h264": 9, "hevc": 35}


def hexrgb(c):
    c = c.lstrip("#")
    if len(c) == 8:           # AARRGGBB, Qt's order
        a = int(c[0:2], 16) / 255.0
        return tuple(int(c[i:i + 2], 16) for i in (2, 4, 6)), a
    return tuple(int(c[i:i + 2], 16) for i in (0, 2, 4)), 1.0


def read_vpt(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:4] != b"VPT1":
        raise ValueError("not a VPT1 file")
    w, h, x, y, split = struct.unpack_from("<IIiiI", data, 4)
    if len(data) != 24 + w * h or split > w:
        raise ValueError("bad VPT1 size")
    return {"width": w, "height": h, "x": x, "y": y, "split": split, "alpha": data[24:]}


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


def sha(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


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


