#!/usr/bin/env python3
"""Makes the parameter sets of `helper/app/src/live/rate.rs`'s tests, and what ffmpeg's bitstream filters write over them (the oracle the tests compare to).

    rate-vectors.py x264      x264's sets at 25 and 50 fps, and the same through `h264_metadata=tick_rate=60/1` (120/1 for 50 fps)
    rate-vectors.py x265      x265's sets at 25 and 50 fps, and the same through `hevc_metadata=tick_rate=30/1` (60/1 for 50 fps)
    rate-vectors.py h265      H.265 sets built bit by bit (sub-layers, scaling lists, PCM, reference sets predicted from each other, long-term pictures,
                              a VUI with every part), each checked by `hevc_metadata` (a set it cannot read is reported), and the filter's rewrite of it

Each line is `name set filter's-set` in hex. Needs ffmpeg with libx264, libx265 and the two filters. Stdlib only.
"""
import subprocess
import sys
import tempfile


def run(cmd):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    if r.returncode != 0:
        raise RuntimeError("%s\n%s" % (cmd, r.stderr[-400:]))
    return r


def nals(data):
    """The NAL units of an Annex B stream, each without its start code or the zero bytes after it."""
    starts, i = [], 0
    while i + 3 <= len(data):
        if data[i] == 0 and data[i + 1] == 0 and data[i + 2] == 1:
            starts.append(i + 3)
            i += 3
        else:
            i += 1
    out = []
    for k, s in enumerate(starts):
        e = starts[k + 1] - 3 if k + 1 < len(starts) else len(data)
        while e > s and data[e - 1] == 0:
            e -= 1
        out.append(data[s:e])
    return out


def first(path, kind, types):
    return next(n for n in nals(open(path, "rb").read()) if kind(n) in types)


def h264_type(n):
    return n[0] & 0x1f


def h265_type(n):
    return (n[0] >> 1) & 0x3f


def x264(tmp):
    variants = {
        "baseline 640x360": ("640x360", 25, "-profile:v baseline -level 3.0"),
        "main 1280x720": ("1280x720", 25, "-profile:v main -x264-params keyint=25"),
        "high 1920x1080, cropped": ("1920x1080", 25, "-profile:v high"),
        "main with HRD": ("1280x720", 25, "-profile:v main -x264-params nal-hrd=cbr:bitrate=2000:vbv-maxrate=2000:vbv-bufsize=2000:force-cfr=1:pic-struct=1"),
        "high with scaling lists": ("640x360", 25, "-profile:v high -x264-params cqm=jvt"),
        "high interlaced 720x576": ("720x576", 25, "-profile:v high -flags +ildct+ilme -x264-params tff=1"),
        "high 1280x720 at 50": ("1280x720", 50, "-profile:v high"),
    }
    for name, (size, rate, opts) in variants.items():
        src, out = "%s/a.h264" % tmp, "%s/b.h264" % tmp
        run('ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc=size=%s:rate=%d" -frames:v 3 -c:v libx264 -pix_fmt yuv420p %s -f h264 %s' % (size, rate, opts, src))
        run("ffmpeg -hide_banner -loglevel error -y -i %s -c copy -bsf:v h264_metadata=tick_rate=%s -f h264 %s" % (src, "60/1" if rate == 25 else "120/1", out))
        print(name, first(src, h264_type, (7,)).hex(), first(out, h264_type, (7,)).hex())


def x265(tmp):
    variants = {
        "720p": ("1280x720", 25, "", ""),
        "1080p, B pictures": ("1920x1080", 25, "-x265-params bframes=4:ref=4:keyint=50", ""),
        "4K": ("3840x2160", 25, "-x265-params keyint=25", ""),
        "two temporal layers": ("1280x720", 25, "-x265-params temporal-layers=2:bframes=3", ""),
        "three temporal layers": ("1280x720", 25, "-x265-params temporal-layers=3:bframes=7", ""),
        "scaling lists on": ("1280x720", 25, "-x265-params scaling-list=default", ""),
        "aspect ratio and colour description": ("1280x720", 25, "", "-vf setsar=4/3 -color_primaries bt709 -color_trc bt709 -colorspace bt709 -color_range tv"),
        "50 fps": ("1280x720", 50, "", ""),
    }
    for name, (size, rate, opts, vf) in variants.items():
        src, out = "%s/a.h265" % tmp, "%s/b.h265" % tmp
        run('ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc=size=%s:rate=%d" -frames:v 6 %s -c:v libx265 -pix_fmt yuv420p %s -f hevc %s' % (size, rate, vf, opts, src))
        run("ffmpeg -hide_banner -loglevel error -y -i %s -c copy -bsf:v hevc_metadata=tick_rate=%s -f hevc %s" % (src, "30/1" if rate == 25 else "60/1", out))
        print(name, first(src, h265_type, (33,)).hex(), first(out, h265_type, (33,)).hex())


class W:
    """A bit writer: fixed-width fields and Exp-Golomb codes."""

    def __init__(self):
        self.b = []

    def put(self, n, v):
        for i in range(n - 1, -1, -1):
            self.b.append((v >> i) & 1)

    def ue(self, v):
        x = v + 1
        self.put(x.bit_length() - 1, 0)
        self.put(x.bit_length(), x)

    def se(self, v):
        self.ue(2 * v - 1 if v > 0 else -2 * v)

    def rbsp(self):
        b = list(self.b) + [1]
        while len(b) % 8:
            b.append(0)
        return bytes(int("".join(map(str, b[i:i + 8])), 2) for i in range(0, len(b), 8))


def escape(rbsp):
    out, zeros = bytearray(), 0
    for x in rbsp:
        if zeros >= 2 and x <= 3:
            out.append(3)
            zeros = 0
        out.append(x)
        zeros = zeros + 1 if x == 0 else 0
    return bytes(out)


def profile_tier_level(w, max_sub, profile, level):
    general = lambda: (w.put(2, 0), w.put(1, 0), w.put(5, 1), w.put(32, 0x60000000), w.put(4, 0b1001), w.put(43, 0), w.put(1, 0))
    general()
    w.put(8, 93)
    for i in range(max_sub):
        w.put(1, profile[i])
        w.put(1, level[i])
    if max_sub > 0:
        for _ in range(max_sub, 8):
            w.put(2, 0)
    for i in range(max_sub):
        if profile[i]:
            general()
        if level[i]:
            w.put(8, 90)


def h265_sps(max_sub=0, ordering=True, profile=None, level=None, scaling=None, pcm=False, rps=None, long_term=None, vui=None, timing=(1, 25), poc_bits=8, crop=False):
    """An H.265 sequence parameter set NAL unit. `scaling`: None off, [] on with the default lists, [1] on with coded lists; `rps`: ("plain", negatives, positives)
    or ("inter", delta_rps_sign, abs_delta_rps_minus1, [(used, use_delta)...]); `long_term`: [(lsb, used)...]; `vui`: a dict of the parts to write."""
    profile, level = profile or [0] * 8, level or [0] * 8
    w = W()
    w.put(4, 0)
    w.put(3, max_sub)
    w.put(1, 1)
    profile_tier_level(w, max_sub, profile, level)
    for v in (0, 1, 1280, 720):
        w.ue(v)
    w.put(1, int(crop))
    if crop:
        for v in (0, 0, 0, 4):
            w.ue(v)
    w.ue(0)
    w.ue(0)
    w.ue(poc_bits - 4)
    w.put(1, int(ordering))
    for _ in range(0 if ordering else max_sub, max_sub + 1):
        for v in (4, 2, 0):
            w.ue(v)
    for v in (0, 3, 0, 3, 1, 1):
        w.ue(v)
    if scaling is None:
        w.put(1, 0)
    else:
        w.put(1, 1)
        w.put(1, 1 if scaling else 0)
        for size in range(4) if scaling else ():
            for mid in range(0, 6, 3 if size == 3 else 1):
                if (size + mid) % 2 == 0:
                    w.put(1, 0)
                    w.ue(0)
                else:
                    w.put(1, 1)
                    if size > 1:
                        w.se(-2)
                    for k in range(min(64, 1 << (4 + (size << 1)))):
                        w.se(1 if k % 7 == 0 else 0)
    w.put(1, 1)
    w.put(1, 1)
    w.put(1, int(pcm))
    if pcm:
        w.put(4, 7)
        w.put(4, 7)
        w.ue(0)
        w.ue(2)
        w.put(1, 1)
    rps = rps or []
    w.ue(len(rps))
    deltas = []
    for i, r in enumerate(rps):
        if i != 0:
            w.put(1, 1 if r[0] == "inter" else 0)
        if i != 0 and r[0] == "inter":
            w.put(1, r[1])
            w.ue(r[2])
            kept = 0
            for used, use_delta in r[3][: deltas[i - 1] + 1]:
                w.put(1, used)
                if not used:
                    w.put(1, use_delta)
                kept += 1 if used or use_delta else 0
            deltas.append(kept)
        else:
            w.ue(len(r[1]))
            w.ue(len(r[2]))
            for d, used in r[1] + r[2]:
                w.ue(d)
                w.put(1, used)
            deltas.append(len(r[1]) + len(r[2]))
    if long_term is None:
        w.put(1, 0)
    else:
        w.put(1, 1)
        w.ue(len(long_term))
        for lsb, used in long_term:
            w.put(poc_bits, lsb)
            w.put(1, used)
    w.put(1, 1)
    w.put(1, 1)
    if vui is None:
        w.put(1, 0)
    else:
        w.put(1, 1)
        w.put(1, int(vui.get("sar", False)))
        if vui.get("sar"):
            w.put(8, 255)
            w.put(16, 4)
            w.put(16, 3)
        w.put(1, int(vui.get("overscan", False)))
        if vui.get("overscan"):
            w.put(1, 1)
        w.put(1, int(vui.get("signal", False)))
        if vui.get("signal"):
            w.put(3, 5)
            w.put(1, 0)
            w.put(1, 1)
            for _ in range(3):
                w.put(8, 1)
        w.put(1, int(vui.get("chroma_loc", False)))
        if vui.get("chroma_loc"):
            w.ue(1)
            w.ue(2)
        w.put(2, 0)
        w.put(1, int(vui.get("ffi", False)))
        w.put(1, int(vui.get("window", False)))
        if vui.get("window"):
            for v in (0, 0, 0, 2):
                w.ue(v)
        w.put(1, 1)
        w.put(32, timing[0])
        w.put(32, timing[1])
        w.put(1, int(vui.get("poc_prop", False)))
        if vui.get("poc_prop"):
            w.ue(0)
        w.put(1, 0)
        w.put(1, int(vui.get("restrict", False)))
        if vui.get("restrict"):
            for v in (0, 1, 1):
                w.put(1, v)
            for v in (0, 2, 1, 15, 15):
                w.ue(v)
    w.put(1, 0)
    return bytes([0x42, 0x01]) + escape(w.rbsp())


SHAPES = {
    "two sub-layers": dict(max_sub=2, profile=[1, 0, 0, 0, 0, 0, 0, 0], level=[1, 1, 0, 0, 0, 0, 0, 0], vui={"signal": True}),
    "three sub-layers, one ordering": dict(max_sub=3, ordering=False, profile=[0, 1, 1, 0, 0, 0, 0, 0], level=[1, 0, 1, 0, 0, 0, 0, 0], vui={}),
    "scaling list data": dict(scaling=[1], vui={"sar": True}),
    "PCM": dict(pcm=True, vui={"chroma_loc": True, "window": True}),
    "short-term reference sets": dict(rps=[("plain", [(0, 1)], []), ("plain", [(0, 1), (1, 1)], [(0, 1)])], vui={}),
    "reference sets predicted from each other": dict(rps=[("plain", [(0, 1), (1, 1)], []), ("inter", 1, 0, [(1, 0), (1, 0), (0, 1)]), ("inter", 1, 1, [(1, 0), (0, 0), (0, 1), (1, 0)])], vui={"restrict": True}),
    "long-term pictures": dict(long_term=[(3, 1), (200, 0)], rps=[("plain", [(0, 1)], [])], vui={"ffi": True, "poc_prop": True}),
    "cropped, with every part of the VUI": dict(crop=True, vui={"overscan": True, "sar": True, "signal": True, "chroma_loc": True, "window": True, "restrict": True}),
}


def h265(tmp):
    # a real stream for the filter to carry: x265's own parameter sets and first picture, with the set built here in place of x265's
    base = "%s/base.h265" % tmp
    run('ffmpeg -hide_banner -loglevel error -y -f lavfi -i "testsrc=size=1280x720:rate=25" -frames:v 4 -c:v libx265 -pix_fmt yuv420p -f hevc %s' % base)
    for name, shape in SHAPES.items():
        for rate, tick in ((25, "30/1"), (50, "60/1")):
            sps = h265_sps(timing=(1, rate), **shape)
            data = b"".join(b"\x00\x00\x00\x01" + (sps if h265_type(n) == 33 else n) for n in nals(open(base, "rb").read()))
            src, out = "%s/in.h265" % tmp, "%s/out.h265" % tmp
            open(src, "wb").write(data)
            try:
                run("ffmpeg -hide_banner -loglevel error -y -f hevc -i %s -c copy -bsf:v hevc_metadata=tick_rate=%s -f hevc %s" % (src, tick, out))
            except RuntimeError as e:
                print(name, rate, "REFUSED by hevc_metadata:", str(e)[-120:].replace("\n", " "))
                continue
            print("%s at %d" % (name, rate), sps.hex(), first(out, h265_type, (33,)).hex())


def main(argv):
    if len(argv) != 1 or argv[0] not in ("x264", "x265", "h265"):
        print(__doc__)
        return 2
    with tempfile.TemporaryDirectory() as tmp:
        {"x264": x264, "x265": x265, "h265": h265}[argv[0]](tmp)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
