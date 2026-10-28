#!/usr/bin/env python3
"""Reads a pulled slot directory (venc/tv_slot.sh on the set): every <step>.out's JSON summary line, every
<step>.h264 through ffprobe (codec, profile, level, size, frame count) and ffmpeg (the first, a middle and
the last frame as PNG beside it), and writes summary.md with one row per step. Also usable on a single
.h264 file.

    python3 venc/tools/decode_check.py docs/research/venc/20261028-0100
    python3 venc/tools/decode_check.py /tmp/vt.h264
"""
import json
import os
import re
import subprocess
import sys


def ffprobe(path):
    try:
        out = subprocess.run(
            ["ffprobe", "-v", "error", "-select_streams", "v:0", "-count_frames", "-show_entries",
             "stream=codec_name,profile,level,width,height,nb_read_frames,pix_fmt", "-of", "json", path],
            capture_output=True, text=True, timeout=600)
    except (OSError, subprocess.TimeoutExpired) as e:
        return {"error": str(e)}
    try:
        s = json.loads(out.stdout)["streams"][0]
    except (ValueError, KeyError, IndexError):
        return {"error": (out.stderr or "no stream").strip()[:200]}
    return s


def frames_png(path, nframes):
    """first, middle and last frame as PNG next to the stream"""
    base = path[:-5] if path.endswith(".h264") else path
    picks = sorted({0, max(0, nframes // 2), max(0, nframes - 1)})
    made = []
    for i in picks:
        png = f"{base}-f{i:04d}.png"
        r = subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", path, "-vf", f"select=eq(n\\,{i})", "-vframes", "1", png],
                           capture_output=True, text=True, timeout=600)
        if r.returncode == 0 and os.path.exists(png):
            made.append(os.path.basename(png))
    return made


def nal_counts(path):
    d = open(path, "rb").read()
    types = [d[m.start() + 3] & 0x1F for m in re.finditer(rb"\x00\x00\x01", d) if m.start() + 3 < len(d)]
    return {"bytes": len(d), "nals": len(types), "sps": types.count(7), "pps": types.count(8), "idr": types.count(5), "p": types.count(1)}


def summary_line(out_path):
    if not os.path.exists(out_path):
        return None
    for line in reversed(open(out_path, errors="replace").read().splitlines()):
        if line.startswith("{") and line.endswith("}"):
            try:
                return json.loads(line)
            except ValueError:
                return None
    return None


def one(path):
    row = {"file": os.path.basename(path)}
    row.update(nal_counts(path))
    p = ffprobe(path)
    row["ffprobe"] = p
    n = int(p.get("nb_read_frames", 0) or 0) if "error" not in p else 0
    row["png"] = frames_png(path, n) if n else []
    return row


def main():
    if len(sys.argv) != 2:
        print(__doc__)
        return 2
    target = sys.argv[1]
    if os.path.isfile(target):
        print(json.dumps(one(target), indent=1))
        return 0
    rows = []
    for name in sorted(os.listdir(target)):
        if not name.endswith(".h264"):
            continue
        step = name[:-5]
        path = os.path.join(target, name)
        row = one(path)
        row["step"] = step
        row["summary"] = summary_line(os.path.join(target, step + ".out"))
        rc_path = os.path.join(target, step + ".rc")
        row["rc"] = open(rc_path).read().strip() if os.path.exists(rc_path) else "?"
        rows.append(row)
    lines = ["# venc slot " + os.path.basename(target.rstrip("/")), "",
             "| step | rc | frames (probe) | fps got | kbps got | cpu % | ioctl ms avg/max | hw ms avg/max | wait ms avg | refused | ffprobe | nals sps/pps/idr/p | png |",
             "|---|---|---|---|---|---|---|---|---|---|---|---|---|"]
    for r in rows:
        s = r["summary"] or {}
        p = r["ffprobe"]
        pf = p.get("error") or f"{p.get('codec_name')} {p.get('profile')} L{p.get('level')} {p.get('width')}x{p.get('height')} {p.get('nb_read_frames')} frames {p.get('pix_fmt')}"
        lines.append("| %s | %s | %s | %s | %s | %s | %s/%s | %s/%s | %s | %s | %s | %s/%s/%s/%s | %s |" % (
            r["step"], r["rc"], s.get("frames", "-"), s.get("fps_got", "-"), s.get("kbps_got", "-"), s.get("cpu_pct", "-"),
            s.get("ioctl_ms_avg", "-"), s.get("ioctl_ms_max", "-"), s.get("hw_ms_avg", "-"), s.get("hw_ms_max", "-"),
            s.get("wait_ms_avg", "-"), s.get("refused", "-"), pf, r["sps"], r["pps"], r["idr"], r["p"], " ".join(r["png"]) or "-"))
    text = "\n".join(lines) + "\n"
    open(os.path.join(target, "summary.md"), "w").write(text)
    print(text)
    return 0


if __name__ == "__main__":
    sys.exit(main())
