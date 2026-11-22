#!/usr/bin/env python3
"""Regenerates fixtures/tagged-bframes.hevc and fixtures/tagged-bframes.order, the stream the presenter tests decode.

The tests read the committed files; run this only to change the stream. It needs ffmpeg with libx265 and ffprobe.

Stream: 80 pictures, 128x64, HEVC Main10 in Annex B with access unit delimiters, parameter sets repeated at each IDR, up to four
B pictures between references (pyramid), a closed GOP of 40, and a 90 kHz clock of 750 ticks per picture.

Every picture carries a 16 bit tag in its luma, 16 cells of 8x8 samples at the top left, bit k in cell k (940 for 1, 64 for 0),
and a smooth moving body so the pictures differ everywhere (the tests also checksum the luma). The tag is not the picture's display index.

The `.order` file has one line per access unit, in decode order: `display_index tag`. It is the only link between decode order and
display order that the tests use besides the pixels themselves.
"""
import json
import os
import subprocess
import sys
import tempfile

W, H, N = 128, 64, 80
TICKS = 750                     # 90 kHz / 120 pictures per second
TAG_CELL = 8
HERE = os.path.dirname(os.path.abspath(__file__))


def tag_of(i):
    return (i * 37 + 0x5A5A) & 0xFFFF


def tri(t):
    t %= 512
    return t if t < 256 else 512 - t


def frame(i):
    """One yuv420p10le picture as bytes."""
    tag = tag_of(i)
    luma = bytearray()
    for y in range(H):
        for x in range(W):
            if y < 2 * TAG_CELL and x < 8 * TAG_CELL:
                v = 940 if (tag >> ((y // TAG_CELL) * 8 + x // TAG_CELL)) & 1 else 64
            else:
                v = 120 + 2 * tri(3 * x + 2 * y + 9 * i) + tri(5 * y - 4 * i)
            luma += v.to_bytes(2, "little")
    cb, cr = bytearray(), bytearray()
    for y in range(H // 2):
        for x in range(W // 2):
            tagged = y < TAG_CELL and x < 4 * TAG_CELL
            cb += (512 if tagged else 480 + tri(2 * x + 5 * i) // 4).to_bytes(2, "little")
            cr += (512 if tagged else 480 + tri(2 * y + 7 * i) // 4).to_bytes(2, "little")
    return bytes(luma + cb + cr)


def run(cmd, **kw):
    return subprocess.run(cmd, check=True, **kw)


def main():
    out = os.path.join(HERE, "fixtures")
    os.makedirs(out, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        ts = os.path.join(tmp, "s.ts")
        enc = subprocess.Popen(
            ["ffmpeg", "-y", "-loglevel", "error", "-f", "rawvideo", "-pix_fmt", "yuv420p10le", "-s", f"{W}x{H}",
             "-framerate", "120", "-i", "pipe:0", "-c:v", "libx265", "-pix_fmt", "yuv420p10le", "-crf", "14",
             "-x265-params",
             "bframes=4:b-adapt=0:b-pyramid=1:keyint=40:min-keyint=40:scenecut=0:open-gop=0:repeat-headers=1:aud=1:"
             "frame-threads=1:pools=1:log-level=error:no-info=1",
             "-f", "mpegts", ts],
            stdin=subprocess.PIPE)
        for i in range(N):
            enc.stdin.write(frame(i))
        enc.stdin.close()
        if enc.wait() != 0:
            sys.exit("ffmpeg failed")
        probe = run(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries", "packet=pts,size",
                     "-of", "json", ts], capture_output=True, text=True)
        packets = json.loads(probe.stdout)["packets"]
        hevc = os.path.join(out, "tagged-bframes.hevc")
        run(["ffmpeg", "-y", "-loglevel", "error", "-i", ts, "-map", "0:v:0", "-c:v", "copy", "-f", "hevc", hevc])
    if len(packets) != N:
        sys.exit(f"expected {N} access units, ffprobe saw {len(packets)}")
    first = min(int(p["pts"]) for p in packets)
    order = [(int(p["pts"]) - first) // TICKS for p in packets]
    if sorted(order) != list(range(N)) or order == list(range(N)):
        sys.exit("pictures are not reordered")
    if os.path.getsize(hevc) != sum(int(p["size"]) for p in packets):
        sys.exit("stream size and packet sizes disagree")
    with open(os.path.join(out, "tagged-bframes.order"), "w") as f:
        for i in order:
            f.write(f"{i} {tag_of(i)}\n")
    print(f"{N} pictures, {os.path.getsize(hevc)} bytes, decode order {order[:12]} ...")


if __name__ == "__main__":
    main()
