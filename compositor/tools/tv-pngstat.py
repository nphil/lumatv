#!/usr/bin/env python3
"""What a picture shows, from a PNG, for the gate (tools/tv-gate.sh runs it on the set as /tmp/lumatv/pngstat.py; Python's standard library only,
the set has no imaging package).

    pngstat.py FILE [X0 Y0 X1 Y1]       the mean luma, the spread and the share of bright pixels of a region (default: the middle half of the
                                        picture, from a quarter to three quarters in each direction; the bounds are fractions of the size)

prints one line: `w=960 h=540 mean=87.3 std=41.2 bright=0.62 black=no`. `black=yes` is a picture with nothing in the region: a mean luma under 6
and a spread under 4 (a dark film scene has a spread; the display engine's opaque black under a video hole has none), or a file that cannot be
read (then `error=...` says why). Reads 8-bit grey, RGB and RGBA pictures, not interlaced: what LG's capture service writes.
"""
import struct
import sys
import zlib

BLACK_MEAN = 6.0
BLACK_STD = 4.0
BRIGHT = 24


def read_png(path):
    with open(path, "rb") as f:
        data = f.read()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        raise ValueError("not a PNG")
    pos, idat, head = 8, [], None
    while pos + 8 <= len(data):
        size, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + size]
        pos += 12 + size
        if kind == b"IHDR":
            head = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat.append(body)
        elif kind == b"IEND":
            break
    if head is None or not idat:
        raise ValueError("no image data")
    w, h, depth, ctype, _, _, interlace = head
    channels = {0: 1, 2: 3, 6: 4}.get(ctype)
    if depth != 8 or channels is None or interlace:
        raise ValueError("unsupported PNG (depth %d, colour type %d, interlace %d)" % (depth, ctype, interlace))
    raw = zlib.decompress(b"".join(idat))
    stride = w * channels
    if len(raw) < (stride + 1) * h:
        raise ValueError("truncated image data")
    rows, prev = [], bytearray(stride)
    for y in range(h):
        base = y * (stride + 1)
        ftype = raw[base]
        line = bytearray(raw[base + 1:base + 1 + stride])
        if ftype == 1:
            for i in range(channels, stride):
                line[i] = (line[i] + line[i - channels]) & 255
        elif ftype == 2:
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 255
        elif ftype == 3:
            for i in range(stride):
                left = line[i - channels] if i >= channels else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 255
        elif ftype == 4:
            for i in range(stride):
                a = line[i - channels] if i >= channels else 0
                b = prev[i]
                c = prev[i - channels] if i >= channels else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 255
        elif ftype != 0:
            raise ValueError("bad filter %d" % ftype)
        rows.append(line)
        prev = line
    return w, h, channels, rows


def region_stats(w, h, channels, rows, box=(0.25, 0.25, 0.75, 0.75)):
    x0, y0, x1, y1 = int(box[0] * w), int(box[1] * h), max(int(box[2] * w), int(box[0] * w) + 1), max(int(box[3] * h), int(box[1] * h) + 1)
    n = total = total2 = bright = 0
    for y in range(y0, y1, 2):
        row = rows[y]
        for x in range(x0, x1, 2):
            i = x * channels
            luma = row[i] if channels == 1 else (299 * row[i] + 587 * row[i + 1] + 114 * row[i + 2]) / 1000
            n += 1
            total += luma
            total2 += luma * luma
            bright += luma > BRIGHT
    mean = total / n
    std = max(total2 / n - mean * mean, 0) ** 0.5
    return mean, std, bright / n


def main(argv):
    if len(argv) not in (2, 6):
        print("usage: pngstat.py FILE [X0 Y0 X1 Y1]", file=sys.stderr)
        return 2
    box = tuple(float(v) for v in argv[2:6]) if len(argv) == 6 else (0.25, 0.25, 0.75, 0.75)
    try:
        w, h, channels, rows = read_png(argv[1])
        mean, std, bright = region_stats(w, h, channels, rows, box)
    except (OSError, ValueError, zlib.error) as e:
        print("w=0 h=0 mean=0 std=0 bright=0 black=yes error=%s" % str(e).replace(" ", "_"))
        return 0
    black = "yes" if mean < BLACK_MEAN and std < BLACK_STD else "no"
    print("w=%d h=%d mean=%.1f std=%.1f bright=%.2f black=%s" % (w, h, mean, std, bright, black))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
