#!/usr/bin/env python3
"""Offline probe streams for the GeForce NOW decode probe (docs/research/gfn-app-result.md, Phase 0 item 1).

`gen` writes Annex B elementary streams into gfn/build/streams/ (not in git) with ffmpeg: x265 Main10 HDR10 at 4K120 in the
level, bitrate and reference-frame variants the probe matrix needs, the lower tiers, an 8-bit SDR 4K120, an H.264 1080p60
control, the 4:4:4 negative, an SVT-AV1 4K120 10-bit, and an Opus packet file for the audio path. The source is synthetic
(testsrc2 under temporal noise): the decoder's work scales with resolution and bitrate, not with what the picture shows, and
the noise keeps the encoder at its bitrate cap instead of letting a flat test card compress to nothing.

Every stream is written with access unit delimiters, repeated parameter sets at each IDR, one slice per picture and no
B-frames, which is the shape GFN's encoder sends and what the feeder splits on. `manifest.json` beside the files records the
settings and what the headers say (profile, tier, level, chroma), read back from the SPS, so a wrong encode cannot pass as a
right one. `check` compares the files with the manifest.

Usage: gfn_streams.py gen [--out DIR] [--only NAME,...] [--seconds N]; gfn_streams.py check [--out DIR]; gfn_streams.py list
"""
import argparse
import hashlib
import json
import os
import struct
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.normpath(os.path.join(HERE, "..", "build", "streams"))

MASTER = "G(13250,34500)B(7500,3000)R(34000,16000)WP(15635,16450)L(10000000,1)"   # BT.2020 primaries, D65, 1000 nits, 0.0001
MAX_CLL = "1000,400"

HEVC_LEVELS = {"4.1": 123, "5.0": 150, "5.1": 153, "5.2": 156, "6.0": 180, "6.1": 183, "6.2": 186}
# Main tier bitrate ceilings in Mbit/s (H.265 table A.9); above them x265 writes the high tier flag, below it stays main even when allowed.
MAIN_TIER_MBPS = {"4.1": 20, "5.0": 25, "5.1": 40, "5.2": 60, "6.0": 60, "6.1": 120, "6.2": 240}


def hevc(name, w, h, fps, mbps, level, ref, hdr=True, bits=10, chroma="420", seconds=None, declare_level=None):
    tier = "high" if mbps > MAIN_TIER_MBPS[level] else "main"
    return {"name": name, "codec": "hevc", "width": w, "height": h, "fps": fps, "mbps": mbps, "level": level, "tier": tier,
            "ref": ref, "hdr": hdr, "bits": bits, "chroma": chroma, "seconds": seconds, "declare_level": declare_level}


def h264(name, w, h, fps, mbps, level, seconds=None):
    return {"name": name, "codec": "h264", "width": w, "height": h, "fps": fps, "mbps": mbps, "level": level, "tier": "main",
            "ref": 1, "hdr": False, "bits": 8, "chroma": "420", "seconds": seconds, "declare_level": None}


def av1(name, w, h, fps, mbps, seconds=None):
    return {"name": name, "codec": "av1", "width": w, "height": h, "fps": fps, "mbps": mbps, "level": "5.3", "tier": "main",
            "ref": 1, "hdr": True, "bits": 10, "chroma": "420", "seconds": seconds}


# The matrix of opennow-result.md Phase 0 item 1, named by what they test.
STREAMS = [
    hevc("4k120_main10_hdr_l52_80m_ref1", 3840, 2160, 120, 80, "5.2", 1),      # the headline mode
    hevc("4k120_main10_hdr_l52_50m_ref1", 3840, 2160, 120, 50, "5.2", 1),      # the capability file's 50 Mbit/s
    hevc("4k120_main10_hdr_l52_100m_ref1", 3840, 2160, 120, 100, "5.2", 1),    # NVIDIA's "up to 100 Mbps"
    hevc("4k120_main10_hdr_l52_80m_ref4", 3840, 2160, 120, 80, "5.2", 4),      # Aurora's finding: more references, slower decode
    # x265 refuses 4K120 at level 5.1 (995 Msamples/s against the level's 535), so the stream is encoded at 5.2 and the VPS/SPS level byte
    # rewritten to 5.1: the content a server would send if it declared the level webOS advertises.
    hevc("4k120_main10_hdr_l51_80m_ref1", 3840, 2160, 120, 80, "5.2", 1, declare_level="5.1"),
    hevc("4k120_main_sdr_l52_80m_ref1", 3840, 2160, 120, 80, "5.2", 1, hdr=False, bits=8),
    hevc("4k60_main10_hdr_l51_50m_ref1", 3840, 2160, 60, 50, "5.1", 1),        # fallback tier
    hevc("1440p120_main10_hdr_l51_50m_ref1", 2560, 1440, 120, 50, "5.1", 1),   # fallback tier; HFR below 1920x1440 needs the pipeline flag
    hevc("1080p120_main10_hdr_l51_30m_ref1", 1920, 1080, 120, 30, "5.1", 1),
    h264("1080p60_high_sdr_l41_20m", 1920, 1080, 60, 20, "4.1"),               # the control: NDL's documented H.264 path
    hevc("4k120_main444_10_hdr_l52_80m_negative", 3840, 2160, 120, 80, "5.2", 1, chroma="444", seconds=2),   # E2, expected to fail
    av1("4k120_av1_main10_hdr_50m", 3840, 2160, 120, 50),                      # capped at 4K60 by the capability file; expected to fail
]


def x265_params(s, seconds):
    keyint = s["fps"]
    p = ["bitrate=%d" % (s["mbps"] * 1000), "vbv-maxrate=%d" % (s["mbps"] * 1000), "vbv-bufsize=%d" % (s["mbps"] * 500), "hrd=1",
         "level-idc=%s" % s["level"], "high-tier=%d" % (1 if s["tier"] == "high" else 0),
         "keyint=%d" % keyint, "min-keyint=%d" % keyint, "scenecut=0", "bframes=0", "ref=%d" % s["ref"], "slices=1",
         "aud=1", "repeat-headers=1", "range=limited", "rc-lookahead=0", "frame-threads=4", "log-level=error"]
    if s["hdr"]:
        p += ["hdr10=1", "hdr10-opt=1", "master-display=" + MASTER, "max-cll=" + MAX_CLL,
              "colorprim=bt2020", "transfer=smpte2084", "colormatrix=bt2020nc"]
    else:
        p += ["colorprim=bt709", "transfer=bt709", "colormatrix=bt709"]
    return ":".join(p)


def pix_fmt(s):
    return {("420", 8): "yuv420p", ("420", 10): "yuv420p10le", ("444", 10): "yuv444p10le", ("444", 8): "yuv444p"}[(s["chroma"], s["bits"])]


def source(s, seconds):
    # testsrc2 moves (a scrolling scene) and the noise is a new draw per frame, so nothing is a free skip for the encoder.
    return "testsrc2=size=%dx%d:rate=%d,noise=alls=12:allf=t+u,format=%s" % (s["width"], s["height"], s["fps"], pix_fmt(s))


def ffmpeg_cmd(s, seconds, out):
    frames = s["fps"] * seconds
    base = ["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i", source(s, seconds), "-frames:v", str(frames),
            "-pix_fmt", pix_fmt(s)]
    if s["hdr"]:
        base += ["-color_primaries", "bt2020", "-color_trc", "smpte2084", "-colorspace", "bt2020nc"]
    else:
        base += ["-color_primaries", "bt709", "-color_trc", "bt709", "-colorspace", "bt709"]
    if s["codec"] == "hevc":
        profile = {("420", 8): "main", ("420", 10): "main10", ("444", 10): "main444-10", ("444", 8): "main444-8"}[(s["chroma"], s["bits"])]
        return base + ["-c:v", "libx265", "-preset", "faster", "-profile:v", profile, "-x265-params", x265_params(s, seconds), "-f", "hevc", out]
    if s["codec"] == "h264":
        kb = s["mbps"] * 1000
        params = "keyint=%d:min-keyint=%d:scenecut=0:bframes=0:ref=1:slices=1:aud=1:repeat-headers=1:nal-hrd=cbr:vbv-maxrate=%d:vbv-bufsize=%d:range=tv:colorprim=bt709:transfer=bt709:colormatrix=bt709:log-level=error" % (
            s["fps"], s["fps"], kb, kb // 2)
        return base + ["-c:v", "libx264", "-preset", "faster", "-profile:v", "high", "-level", s["level"], "-b:v", "%dk" % kb,
                       "-x264-params", params, "-f", "h264", out]
    if s["codec"] == "av1":
        params = "rc=1:tbr=%d:keyint=%d:enable-hdr=1:mastering-display=%s:content-light=%s:color-primaries=9:transfer-characteristics=16:matrix-coefficients=9:lp=16" % (
            s["mbps"] * 1000, s["fps"], MASTER, MAX_CLL)
        return base + ["-c:v", "libsvtav1", "-preset", "10", "-svtav1-params", params, "-f", "obu", out]
    raise ValueError(s["codec"])


def ext(s):
    return {"hevc": ".hevc", "h264": ".h264", "av1": ".obu"}[s["codec"]]


def unescape(rbsp):
    """Removes emulation prevention bytes (00 00 03 -> 00 00)."""
    out, i, n = bytearray(), 0, len(rbsp)
    while i < n:
        if i + 2 < n and rbsp[i] == 0 and rbsp[i + 1] == 0 and rbsp[i + 2] == 3:
            out += b"\x00\x00"
            i += 3
        else:
            out.append(rbsp[i])
            i += 1
    return bytes(out)


def escape(rbsp):
    """Inserts emulation prevention bytes (00 00 0x, x <= 3 -> 00 00 03 0x)."""
    out, zeros = bytearray(), 0
    for b in rbsp:
        if zeros >= 2 and b <= 3:
            out.append(3)
            zeros = 0
        out.append(b)
        zeros = zeros + 1 if b == 0 else 0
    return bytes(out)


def hevc_declare_level(data, level_idc):
    """Rewrites general_level_idc in every VPS and SPS: the profile_tier_level block sits 4 bytes into the VPS payload and 1 byte into the
    SPS payload, its level byte is its 12th. Emulation prevention is undone around the edit and redone after it."""
    out, pos = bytearray(), 0
    for h, e in nal_units(data):
        t = (data[h] >> 1) & 0x3F
        out += data[pos:h + 2]
        if t in (32, 33):
            rbsp = bytearray(unescape(data[h + 2:e]))
            rbsp[(4 if t == 32 else 1) + 11] = level_idc
            out += escape(bytes(rbsp))
        else:
            out += data[h + 2:e]
        pos = e
    return bytes(out)


def nal_units(data):
    """(header offset, next start) for each NAL unit of an Annex B stream."""
    out, i = [], 0
    while True:
        j = data.find(b"\x00\x00\x01", i)
        if j < 0:
            break
        out.append(j + 3)
        i = j + 3
    return [(h, (out[k + 1] - 3 if k + 1 < len(out) else len(data))) for k, h in enumerate(out)]


class Bits:
    def __init__(self, data):
        self.d, self.p = data, 0

    def u(self, n):
        v = 0
        for _ in range(n):
            v = (v << 1) | ((self.d[self.p >> 3] >> (7 - (self.p & 7))) & 1)
            self.p += 1
        return v

    def ue(self):
        z = 0
        while self.u(1) == 0:
            z += 1
        return (1 << z) - 1 + self.u(z)


def hevc_sps_facts(data):
    """profile, tier, level and chroma_format_idc from the first SPS of an HEVC Annex B stream."""
    for h, e in nal_units(data):
        if (data[h] >> 1) & 0x3F != 33:
            continue
        b = Bits(unescape(data[h + 2:e]))
        b.u(4)              # sps_video_parameter_set_id
        sub = b.u(3)        # sps_max_sub_layers_minus1
        b.u(1)              # temporal_id_nesting
        b.u(2)              # general_profile_space
        tier = b.u(1)
        profile = b.u(5)
        b.u(32)             # compatibility flags
        b.u(48)             # constraint flags
        level = b.u(8)
        if sub > 0:         # sub-layer flags (2 bits each, padded to 16) then their PTL blocks: not written by these encodes
            raise ValueError("sub-layers present")
        b.ue()              # sps_seq_parameter_set_id
        chroma = b.ue()
        return {"profile_idc": profile, "tier": "high" if tier else "main", "level_idc": level, "chroma_format_idc": chroma}
    raise ValueError("no SPS")


def h264_sps_facts(data):
    for h, e in nal_units(data):
        if data[h] & 0x1F != 7:
            continue
        b = Bits(unescape(data[h + 1:e]))
        profile = b.u(8)
        b.u(8)
        level = b.u(8)
        b.ue()
        chroma = 1
        if profile in (100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135):
            chroma = b.ue()
        return {"profile_idc": profile, "tier": "main", "level_idc": level, "chroma_format_idc": chroma}
    raise ValueError("no SPS")


def count_access_units(data, codec):
    aud = 35 if codec == "hevc" else 9
    n = 0
    for h, e in nal_units(data):
        t = (data[h] >> 1) & 0x3F if codec == "hevc" else data[h] & 0x1F
        if t == aud:
            n += 1
    return n


def sha(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def opus_packets(out, seconds):
    """A raw Opus packet file (u16 LE length + packet, 20 ms frames, 48 kHz stereo): a tone sweep, what the audio path probe plays."""
    ogg = out + ".ogg"
    subprocess.run(["ffmpeg", "-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi", "-i",
                    "sine=frequency=440:beep_factor=4:duration=%d:sample_rate=48000" % seconds, "-ac", "2", "-c:a", "libopus",
                    "-b:a", "128k", "-frame_duration", "20", "-application", "lowdelay", "-f", "ogg", ogg], check=True)
    data = open(ogg, "rb").read()
    os.unlink(ogg)
    packets, pos, serial_packets = [], 0, []
    carry = b""
    while pos + 27 <= len(data):
        if data[pos:pos + 4] != b"OggS":
            raise ValueError("not an Ogg page at %d" % pos)
        segs = data[pos + 26]
        table = data[pos + 27:pos + 27 + segs]
        body = pos + 27 + segs
        for seg in table:
            carry += data[body:body + seg]
            body += seg
            if seg < 255:
                serial_packets.append(carry)
                carry = b""
        pos = body
    # The first two packets are OpusHead and OpusTags.
    for p in serial_packets[2:]:
        packets.append(p)
    with open(out, "wb") as f:
        for p in packets:
            f.write(struct.pack("<H", len(p)) + p)
    return len(packets)


def gen(args):
    os.makedirs(args.out, exist_ok=True)
    only = set(filter(None, args.only.split(",")))
    manifest_path = os.path.join(args.out, "manifest.json")
    manifest = {"streams": {}, "audio": {}}
    if os.path.exists(manifest_path):
        manifest = json.load(open(manifest_path))
    for s in STREAMS:
        if only and s["name"] not in only:
            continue
        seconds = s["seconds"] or args.seconds
        out = os.path.join(args.out, s["name"] + ext(s))
        cmd = ffmpeg_cmd(s, seconds, out)
        t0 = time.time()
        print("encoding %s (%d s)" % (s["name"], seconds), flush=True)
        r = subprocess.run(cmd, stderr=subprocess.PIPE, text=True)
        err = "\n".join(l for l in r.stderr.splitlines() if "set_mempolicy" not in l)
        if r.returncode != 0:
            print("  FAILED: " + err, flush=True)
            manifest["streams"][s["name"]] = dict(s, file=os.path.basename(out), error=err[-400:])
            continue
        data = open(out, "rb").read()
        if s.get("declare_level"):
            data = hevc_declare_level(data, HEVC_LEVELS[s["declare_level"]])
            with open(out, "wb") as f:
                f.write(data)
        facts = {}
        if s["codec"] == "hevc":
            facts = hevc_sps_facts(data)
            facts["access_units"] = count_access_units(data, "hevc")
        elif s["codec"] == "h264":
            facts = h264_sps_facts(data)
            facts["access_units"] = count_access_units(data, "h264")
        entry = dict(s, file=os.path.basename(out), bytes=len(data), sha256=sha(out), seconds=seconds, headers=facts,
                     measured_mbps=round(len(data) * 8 / seconds / 1e6, 1), encode_seconds=round(time.time() - t0, 1))
        manifest["streams"][s["name"]] = entry
        print("  %d bytes, %.1f Mbit/s, headers %s, %.0f s" % (len(data), entry["measured_mbps"], facts, entry["encode_seconds"]), flush=True)
        json.dump(manifest, open(manifest_path, "w"), indent=1, sort_keys=True)
    if not only or "opus" in only:
        out = os.path.join(args.out, "tone_48k_stereo_20ms.opuspkts")
        n = opus_packets(out, args.seconds)
        manifest["audio"]["tone_48k_stereo_20ms"] = {"file": os.path.basename(out), "packets": n, "sample_rate": 48000, "channels": 2,
                                                      "frame_ms": 20, "bytes": os.path.getsize(out), "sha256": sha(out)}
        print("opus: %d packets" % n, flush=True)
    json.dump(manifest, open(manifest_path, "w"), indent=1, sort_keys=True)


def check(args):
    manifest = json.load(open(os.path.join(args.out, "manifest.json")))
    bad = 0
    for name, e in sorted(manifest["streams"].items()):
        path = os.path.join(args.out, e["file"])
        problems = []
        if "error" in e:
            problems.append("encode failed")
        elif not os.path.exists(path):
            problems.append("missing")
        elif os.path.getsize(path) != e["bytes"]:
            problems.append("size differs")
        else:
            h = e.get("headers", {})
            if e["codec"] == "hevc":
                want_level = HEVC_LEVELS[e.get("declare_level") or e["level"]]
                if h.get("level_idc") != want_level:
                    problems.append("level_idc %s, wanted %d" % (h.get("level_idc"), want_level))
                if h.get("tier") != e["tier"]:
                    problems.append("tier %s, wanted %s" % (h.get("tier"), e["tier"]))
                want_chroma = 3 if e["chroma"] == "444" else 1
                if h.get("chroma_format_idc") != want_chroma:
                    problems.append("chroma_format_idc %s" % h.get("chroma_format_idc"))
                want_profile = {("420", 8): 1, ("420", 10): 2, ("444", 10): 4, ("444", 8): 4}[(e["chroma"], e["bits"])]
                if h.get("profile_idc") != want_profile:
                    problems.append("profile_idc %s, wanted %d" % (h.get("profile_idc"), want_profile))
                if h.get("access_units") != e["fps"] * e["seconds"]:
                    problems.append("%s access units, wanted %d" % (h.get("access_units"), e["fps"] * e["seconds"]))
            if e["codec"] in ("hevc", "h264") and abs(e["measured_mbps"] - e["mbps"]) > e["mbps"] * 0.25:
                problems.append("measured %.1f Mbit/s against %d" % (e["measured_mbps"], e["mbps"]))
        print("%-44s %s" % (name, "ok" if not problems else "; ".join(problems)))
        bad += bool(problems)
    for name, e in sorted(manifest.get("audio", {}).items()):
        path = os.path.join(args.out, e["file"])
        ok = os.path.exists(path) and os.path.getsize(path) == e["bytes"]
        print("%-44s %s" % (name, "ok" if ok else "missing or size differs"))
        bad += not ok
    return 1 if bad else 0


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("gen")
    g.add_argument("--out", default=OUT)
    g.add_argument("--only", default="")
    g.add_argument("--seconds", type=int, default=10)
    c = sub.add_parser("check")
    c.add_argument("--out", default=OUT)
    sub.add_parser("list")
    a = ap.parse_args()
    if a.cmd == "gen":
        gen(a)
    elif a.cmd == "check":
        sys.exit(check(a))
    else:
        for s in STREAMS:
            print("%-44s %s %dx%d@%d %d Mbit/s L%s %s ref%d %s %d-bit %s" % (s["name"], s["codec"], s["width"], s["height"], s["fps"], s["mbps"],
                                                                        s["level"], s["tier"], s["ref"], "HDR10" if s["hdr"] else "SDR", s["bits"], s["chroma"]))


if __name__ == "__main__":
    main()
