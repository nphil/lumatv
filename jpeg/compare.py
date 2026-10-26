#!/usr/bin/env python3
"""Reads the hardware block's own buffer against libjpeg's picture of the same file: which row and plane alignment the buffer
has, whether it is full or studio range, and how close the converted output is. Run on the workstation on what the TV slot
copied back (docs/research/hwjpeg-result.md, step 6); it needs Pillow and nothing from the TV.

    jpeg/compare.py FILE.jpg --dump hw-s0.bin --report bench.txt [--shift N] [--out out-s0.rgba|out-s0.nv12]

FILE.jpg   the picture the slot decoded
--dump     the library's buffer, written by `lumatv-jpeg -b FILE -o DIR` (hw-s<shift>.bin)
--report   the bench's text (its `out_width=... hw_width=... hw_height=... hw_pixfmt=...` line)
--out      the converted output of the same run (out-s<shift>.rgba or .nv12), compared as well

Prints the `-A`/`-B` (and `-S`, `-L`) arguments that make lumatv-jpeg read the buffer right, as images.decoderArgs for helper.json.
Exit 0 when a layout reproduces the picture, 1 when none does (then the buffer is not laid out as planes this program knows, and the
dump has to be read by hand), 2 for an input that cannot be read.
"""
import argparse
import io
import re
import sys

from PIL import Image

# the pixel_format codes of the library (jpeg/dilejpeg.h): code -> (hshift, vshift, semi-planar, gray)
FORMATS = {
    7: (0, 0, False, False), 8: (0, 0, True, False), 9: (1, 0, False, False), 10: (1, 0, True, False), 12: (1, 1, False, False),
    13: (1, 1, True, False), 14: (0, 0, False, True), 15: (0, 1, False, False), 16: (0, 1, True, False),
}
ALIGNS = (1, 8, 16, 32, 64, 128, 256)
VALIGNS = (1, 8, 16, 32, 64, 128)
GOOD = 6.0  # mean absolute error of a plane, in levels, below which the layout is taken as right


def align_up(v, a):
    return (v + a - 1) // a * a if a > 1 else v


def reference(jpeg, shift):
    """libjpeg's decode of the file at the same scale, as three planes of the picture's size (Y, Cb, Cr), and the size."""
    im = Image.open(io.BytesIO(jpeg))
    gray = im.mode == "L"
    if shift:
        im.draft("L" if gray else "YCbCr", (im.width >> shift, im.height >> shift))
    elif not gray:
        im.draft("YCbCr", im.size)
    if not gray and im.mode != "YCbCr":
        im = im.convert("YCbCr")
    planes = [p.tobytes() for p in im.split()] if not gray else [im.tobytes()]
    return im.size, planes


def stats(pairs):
    """Mean absolute error after the best straight line (hw = a * ref + b), and the line."""
    n = len(pairs)
    sx = sum(r for r, _ in pairs)
    sy = sum(h for _, h in pairs)
    sxx = sum(r * r for r, _ in pairs)
    sxy = sum(r * h for r, h in pairs)
    den = n * sxx - sx * sx
    a = (n * sxy - sx * sy) / den if den else 1.0
    b = (sy - a * sx) / n
    fit = sum(abs(h - (a * r + b)) for r, h in pairs) / n
    raw = sum(abs(h - r) for r, h in pairs) / n
    # A line may be flat (a buffer plane that is all one value fits anything): only a gain near 1 (full range) or near 0.86 (studio)
    # counts as a fit, whatever its error.
    return (fit if 0.7 <= a <= 1.3 else max(fit, raw)), raw, a, b


def sample_points(w, h, n=48):
    xs = sorted({min(w - 1, i * w // n) for i in range(n)} | {w - 1})
    ys = sorted({min(h - 1, j * h // n) for j in range(n)} | {h - 1})
    return xs, ys


def score(dump, hw_size, size, fmt, ref_planes, A, B, swap):
    """The error of reading `dump`, laid out for the size the block reports with row alignment A and plane-height alignment B,
    against the reference picture of `size`."""
    w, h = size
    hs, vs, semi, gray = fmt
    ystride, yrows = align_up(hw_size[0], A), align_up(hw_size[1], B)
    ybytes = ystride * yrows
    out = []
    xs, ys = sample_points(w, h)
    try:
        pairs = [(ref_planes[0][y * w + x], dump[y * ystride + x]) for y in ys for x in xs]
        out.append(stats(pairs))
        if gray:
            return out
        cw = (ystride + (1 << hs) - 1) >> hs
        crows = (yrows + (1 << vs) - 1) >> vs
        sw, sh = (w + (1 << hs) - 1) >> hs, (h + (1 << vs) - 1) >> vs
        cxs, cys = sample_points(sw, sh)
        for plane in (1, 2):
            if semi:
                cstride, step = cw * 2, 2
                base = ybytes
                col = (plane == 1) != bool(swap)
                off = base + (0 if col else 1)
            else:
                cstride, step = cw, 1
                first = (plane == 1) != bool(swap)
                off = ybytes + (0 if first else cw * crows)
            pairs = []
            for cy in cys:
                for cx in cxs:
                    # the reference chroma at the middle of the cell this sample covers (libjpeg upsamples, so any pixel of it will do)
                    rx, ry = min(w - 1, cx << hs), min(h - 1, cy << vs)
                    pairs.append((ref_planes[plane][ry * w + rx], dump[off + cy * cstride + cx * step]))
            out.append(stats(pairs))
    except IndexError:
        return None
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("jpeg")
    ap.add_argument("--dump", required=True)
    ap.add_argument("--report", required=True)
    ap.add_argument("--shift", type=int, default=0)
    ap.add_argument("--out")
    args = ap.parse_args()
    try:
        jpeg = open(args.jpeg, "rb").read()
        dump = open(args.dump, "rb").read()
        report = open(args.report).read()
        m = re.search(r"hw_width=(\d+) hw_height=(\d+) hw_pixfmt=(\d+) hw_bytes=(\d+)", report)
        if not m:
            raise ValueError("no hw_width= line in the report")
        hw_w, hw_h, pf, nbytes = map(int, m.groups())
        size, planes = reference(jpeg, args.shift)
    except (OSError, ValueError) as e:
        print("cannot read the inputs: %s" % e, file=sys.stderr)
        return 2
    if pf not in FORMATS:
        print("the buffer's pixel format %d is not one this program reads" % pf)
        return 1
    fmt = FORMATS[pf]
    print("picture %dx%d at 1/%d; buffer %dx%d format %d, %d bytes (dump %d bytes)" % (size[0], size[1], 1 << args.shift, hw_w, hw_h, pf, nbytes, len(dump)))
    if (hw_w, hw_h) != size:
        print("note: the block reports %dx%d for a %dx%d picture; lumatv-jpeg crops to the picture" % (hw_w, hw_h, size[0], size[1]))
    rows = []
    for A in ALIGNS:
        for B in VALIGNS:
            for swap in (0, 1):
                s = score(dump, (hw_w, hw_h), size, fmt, planes, A, B, swap)
                if s:
                    rows.append((sum(x[0] for x in s) / len(s), A, B, swap, s))
    if not rows:
        print("no alignment fits inside the dump")
        return 1
    rows.sort(key=lambda r: r[0])
    for err, A, B, swap, s in rows[:5]:
        print("A=%-3d B=%-3d swap=%d  mean fit error %.2f levels   per plane (fit, raw, gain, offset): %s" % (
            A, B, swap, err, "  ".join("%.1f/%.1f/%.3f/%+.1f" % p for p in s)))
    err, A, B, swap, s = rows[0]
    if err > GOOD:
        print("NO LAYOUT MATCHES (best %.1f levels): the buffer is not planar or semi-planar rows at these alignments; read the dump by hand" % err)
        return 1
    # Alignments that give the same stride and plane height for this size read the same bytes: a 1920x1080 picture cannot tell 16
    # from 64. The ones that fit as well as the best are listed; a second file whose size is no multiple of 16 settles it.
    same = [r for r in rows if r[3] == swap and r[0] <= err + 0.1]
    As, Bs = sorted({r[1] for r in same}), sorted({r[2] for r in same})
    A, B = As[0], Bs[0]
    fit, raw, gain, offset = s[0]
    limited = abs(gain - 219 / 255) < 0.05 and 8 < offset < 24
    args_out = ["-A", str(A), "-B", str(B)] + (["-S"] if swap else []) + (["-L"] if limited else [])
    print()
    print("consistent: A in %s B in %s%s" % (As, Bs, "  (several: decode a file whose width and height are no multiple of 16 and take the A and B both files allow)" if len(As) > 1 or len(Bs) > 1 else ""))
    print("range: Y = %.3f * Y(libjpeg) %+.1f -> %s" % (gain, offset, "studio swing (16-235): set -L" if limited else "full range"))
    print("chroma order: %s" % ("Cr before Cb: set -S" if swap else "Cb before Cr"))
    print('helper.json: "images": {"decoderArgs": %s}' % str(args_out).replace("'", '"'))
    if args.out:
        out = open(args.out, "rb").read()
        w, h = size
        if args.out.endswith(".rgba"):
            ref = Image.open(io.BytesIO(jpeg))
            if args.shift:
                ref.draft("RGB", (ref.width >> args.shift, ref.height >> args.shift))
            ref = ref.convert("RGB").tobytes()
            tot = n = 0
            for y in range(0, h, max(1, h // 64)):
                for x in range(0, w, max(1, w // 64)):
                    for c in range(3):
                        tot += abs(out[(y * w + x) * 4 + c] - ref[(y * w + x) * 3 + c])
                        n += 1
            print("converted RGBA against libjpeg's RGB: mean error %.2f levels (the output as run, with the arguments of that run)" % (tot / n))
        else:
            tot = n = 0
            stride = (w + 1) & ~1
            for y in range(0, h, max(1, h // 64)):
                for x in range(0, w, max(1, w // 64)):
                    tot += abs(out[y * stride + x] - planes[0][y * w + x])
                    n += 1
            print("converted NV12 luma against libjpeg's: mean error %.2f levels" % (tot / n))
    return 0


if __name__ == "__main__":
    sys.exit(main())
