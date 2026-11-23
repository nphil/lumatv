#!/usr/bin/env python3
"""Splits an MPEG-2 video or H.264 Annex B elementary stream into access units (one picture each) and writes
them as the records `lumatv-venc vdec` reads and the transcode child takes (xcode_proto.h):

    "XAUN" + <IQII flags (bit 0: random access point), pts (90 kHz, decode-order placeholder), size, 0> + bytes

    es2au.py mpeg2|h264 in.es out.au [--fps 29.97]
"""
import struct
import sys


def start_codes(d):
    """Yields (offset of the 00 00 01 prefix, code byte)."""
    i = d.find(b"\x00\x00\x01")
    while i >= 0 and i + 3 < len(d):
        yield i, d[i + 3]
        i = d.find(b"\x00\x00\x01", i + 3)


def split_mpeg2(d):
    """An access unit runs from the first of sequence header (B3), GOP header (B8) or picture header (00)
    that follows a picture, up to the next such start code."""
    cuts = []
    have_pic = False
    for off, code in start_codes(d):
        if have_pic and code in (0xB3, 0xB8, 0x00):
            cuts.append(off)
            have_pic = False
        if code == 0x00:
            have_pic = True
    starts = [0] + cuts
    ends = cuts + [len(d)]
    out = []
    for s, e in zip(starts, ends):
        unit = d[s:e]
        # random access: a unit that carries a sequence header and an I picture
        is_key = False
        for off, code in start_codes(unit):
            if code == 0x00 and off + 5 < len(unit):
                coding_type = (unit[off + 5] >> 3) & 7
                is_key = coding_type == 1 and b"\x00\x00\x01\xb3" in unit[:off]
                break
        out.append((is_key, unit))
    return out


def nal_units(d):
    pos = [(off, d[off + 3] & 0x1F) for off, _ in start_codes(d)]
    for i, (off, t) in enumerate(pos):
        end = pos[i + 1][0] if i + 1 < len(pos) else len(d)
        yield off, end, t


def split_h264(d):
    """A new access unit starts at an AUD, or at SPS/PPS/SEI after a picture, or at a slice with
    first_mb_in_slice == 0 after a slice."""
    cuts = []
    have_vcl = False
    for off, end, t in nal_units(d):
        first_slice = t in (1, 5) and (d[off + 4] & 0x80) != 0
        if have_vcl and (t in (9, 6, 7, 8) or first_slice):
            cuts.append(off)
            have_vcl = False
        if t in (1, 5):
            have_vcl = True
    starts = [0] + cuts
    ends = cuts + [len(d)]
    out = []
    for s, e in zip(starts, ends):
        unit = d[s:e]
        out.append((any(t == 5 for _, _, t in nal_units(unit)), unit))
    return out


def main():
    if len(sys.argv) < 4 or sys.argv[1] not in ("mpeg2", "h264"):
        sys.exit(__doc__)
    fps = 30000 / 1001
    if "--fps" in sys.argv:
        fps = float(sys.argv[sys.argv.index("--fps") + 1])
    d = open(sys.argv[2], "rb").read()
    units = split_mpeg2(d) if sys.argv[1] == "mpeg2" else split_h264(d)
    with open(sys.argv[3], "wb") as f:
        for i, (key, unit) in enumerate(units):
            f.write(b"XAUN" + struct.pack("<IQII", 1 if key else 0, round(i * 90000 / fps), len(unit), 0) + unit)
    print(f"{len(units)} access units, {sum(1 for k, _ in units if k)} random access points, largest {max(len(u) for _, u in units)} bytes")


main()
