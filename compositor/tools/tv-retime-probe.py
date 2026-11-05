#!/usr/bin/env python3
"""The retime clip on the set (`compositor/.../retime.rs`, docs/research/vwin-refresh-result.md "Panel left at 100 Hz after a full view").

Run ON THE SET as root, under a PTY (luna-send needs one), with LumaTV's compositor live, Home in front and the receiver muted. Stdlib only.

    tv-retime-probe.py read                         the state, nothing changed
    tv-retime-probe.py play [--secs N]              the clip by hand (`ctl retime now`) with the panel as it is: does the decoder connect under the UI,
                                                    does the display engine read 30 fps, does nothing show, is everything gone afterwards
    tv-retime-probe.py restore --clip25 FILE        puts the panel at 100 Hz (a 25 fps clip in a 480x270 window over Home for a few seconds: a black box is
                                                    SEEN for that time) and waits for the compositor to bring it back
    tv-retime-probe.py restore --manual             the same without the box: the run waits for the panel to be held at 100 Hz (open the Ring's full view
                                                    and press Back, or any 25/50 fps camera) and watches the compositor bring it back
    tv-retime-probe.py all --clip25 FILE            read, play, restore
    tv-retime-probe.py selftest                     the readers on text and a PNG made here (runs on the host)

Evidence goes to /tmp/lumatv/out/retime/ (samples.jsonl, result.json, the two captures and the sink status of `play`); fetch the directory. The 25 fps clip is made
on the workstation and put on the set (/tmp/gfn/retime25.h264):
    ffmpeg -f lavfi -i color=c=black:size=640x360:rate=25 -t 1 -c:v libx264 -profile:v high -pix_fmt yuv420p -preset veryslow \\
      -x264-params aud=1:repeat-headers=1:slices=1:bframes=0:keyint=25:scenecut=0:ref=1:force-cfr=1 -bsf:v filter_units=remove_types=6 -f h264 retime25.h264

The retime is off by default; `play` and `restore` switch it on (`ctl retime on`) and put it back as it was found. What it never does: write /proc/lg/de/fr_rate (the
write the firmware ignores), launch an app, touch a picture setting, change the vwin mode or leave a claim, a player or the clip's `show` switch behind. What decides
each verdict is in `result.json` next to its number; a verdict that needs eyes says so (`null`).
"""
import json
import os
import re
import signal
import struct
import subprocess
import sys
import time
import zlib

OUT = "/tmp/lumatv/out/retime"
CTL = ("/var/lib/lumatv/compositor/lib/ld-linux-aarch64.so.1 --library-path /var/lib/lumatv/compositor/lib "
       "/var/lib/lumatv/compositor/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg ")
FEEDER = "/var/lib/lumatv/bin/lumatv-gfn-feeder"
SCALER = "/proc/lgtv-driver/scaler/main.p0"
WIN0 = "/proc/lg/de/win0/status"
HOME = "com.webos.app.home"
APP25 = "org.nphil.lumatv.retime25"
# the clip's window in the UI's pixels (retime.rs RECT), and a 960x540 capture's
RECT_UI = (16, 16, 128, 72)
T0 = time.monotonic()


def say(*a):
    print("retime: %6.1f" % (time.monotonic() - T0), *a, flush=True)


# ---- readers (pure; `selftest` runs them) -------------------------------------------------------------------------------------------------------------

def osd_count(text):
    """The osd_irq count of /proc/interrupts: the CPU columns before the controller's name."""
    for line in text.splitlines():
        f = line.split()
        if f and f[-1] == "osd_irq":
            n = 0
            for x in f[1:]:
                if not x.isdigit():
                    break
                n += int(x)
            return n
    return None


def band(hz):
    for b in (120, 100):
        if hz is not None and abs(hz - b) <= 3.5:
            return b
    return None


def scaler_of(text):
    """connect_src and the source and output sizes of the main window's status."""
    kv = dict(l.strip().split("=", 1) for l in text.splitlines() if "=" in l)
    return {"src": kv.get("connect_src"), "source": (kv.get("source_w"), kv.get("source_h")), "out": (kv.get("out_w"), kv.get("out_h"))}


def win0_of(text):
    """The timing window 0's source has, as the kernel read it: size and rate in 0.1 Hz (300 is 30 fps)."""
    m = re.search(r"h/v active\s*:\s*(\d+)\s*/\s*(\d+)", text)
    f = re.search(r"vFreq\s*:\s*(\d+)", text)
    return {"active": (int(m.group(1)), int(m.group(2))) if m else None, "vfreq": int(f.group(1)) if f else None}


def png_region(path, x0, y0, x1, y1):
    """Mean luma of a rectangle of an 8-bit RGB or RGBA PNG (the captures), or None."""
    d = open(path, "rb").read()
    if d[:8] != b"\x89PNG\r\n\x1a\n":
        return None
    pos, idat, ihdr = 8, b"", None
    while pos + 8 <= len(d):
        n, kind = struct.unpack(">I4s", d[pos:pos + 8])
        body = d[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            ihdr = struct.unpack(">IIBBBBB", body)
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    if not ihdr or ihdr[2] != 8 or ihdr[3] not in (2, 6) or ihdr[6] != 0:
        return None
    w, h, bpp = ihdr[0], ihdr[1], 3 if ihdr[3] == 2 else 4
    raw, stride, prev, rows = zlib.decompress(idat), w * bpp, bytearray(w * bpp), []
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
        for i in range(stride):
            a = line[i - bpp] if i >= bpp else 0
            b, c = prev[i], (prev[i - bpp] if i >= bpp else 0)
            if f == 1:
                line[i] = (line[i] + a) & 255
            elif f == 2:
                line[i] = (line[i] + b) & 255
            elif f == 3:
                line[i] = (line[i] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(line)
        prev = line
    tot = n = 0
    for y in range(max(0, y0), min(h, y1)):
        for x in range(max(0, x0), min(w, x1)):
            r, g, b = rows[y][x * bpp:x * bpp + 3]
            tot += 0.299 * r + 0.587 * g + 0.114 * b
            n += 1
    return tot / n if n else None


# ---- the set --------------------------------------------------------------------------------------------------------------------------------------

def sh(cmd, timeout=30):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout).stdout
    except subprocess.TimeoutExpired:
        return ""


def first_json(text):
    """The first JSON object in a command's output (luna-send and the ctl print notes around it), or {}."""
    i = text.find("{")
    try:
        return json.JSONDecoder().raw_decode(text[i:])[0] if i >= 0 else {}
    except ValueError:
        return {}


def ctl(args):
    return first_json(sh(CTL + args, 25))


def read(path):
    try:
        with open(path) as f:
            return f.read(65536)
    except OSError:
        return ""


def luna(uri, payload="{}"):
    return first_json(sh("luna-send -n 1 %s '%s' 2>&1" % (uri, payload), 20))


def foreground():
    r = luna("luna://com.webos.applicationManager/getForegroundAppInfo")
    e = r.get("foregroundAppInfo")
    if isinstance(e, list):
        e = next((x for x in e if x.get("primary")), e[0] if e else {})
    return (e or r).get("appId")


def capture(name, method):
    p = "%s/%s-%s.png" % (OUT, name, method.lower())
    r = luna("luna://com.webos.service.capture/executeOneShot",
             json.dumps({"path": p, "method": method, "format": "PNG", "width": 960, "height": 540}))
    return p if r.get("returnValue") and os.path.exists(p) else None


def feeders():
    """(pid, arguments) of every feeder running."""
    out = []
    for d in os.listdir("/proc"):
        if d.isdigit():
            try:
                with open("/proc/%s/cmdline" % d, "rb") as f:
                    argv = [a.decode(errors="replace") for a in f.read().split(b"\0") if a]
            except OSError:
                continue
            if argv and argv[0].endswith("lumatv-gfn-feeder"):
                out.append((int(d), argv))
    return out


def players():
    return ["%d %s" % (pid, " ".join(argv)) for pid, argv in feeders()]


def is_for(argv, app):
    """Whether a feeder's arguments say it is the window of `app` (`-a APP`)."""
    return "-a" in argv and argv[argv.index("-a") + 1:][:1] == [app]


def end_player(app):
    """Ends the feeder of one app id (not `pkill -f`: the shell that runs it carries the pattern)."""
    for pid, argv in feeders():
        if is_for(argv, app):
            try:
                os.kill(pid, signal.SIGTERM)
            except OSError:
                pass


class Meter:
    """The OSD interrupt's rate between two looks."""

    def __init__(self):
        self.last = None

    def look(self):
        n, t = osd_count(read("/proc/interrupts")), time.monotonic()
        hz = None
        if self.last and n is not None:
            hz = round((n - self.last[0]) / (t - self.last[1]), 1)
        self.last = (n, t)
        return hz


def sample(meter, tag, log, with_ctl=True):
    s = {"t": round(time.monotonic() - T0, 2), "tag": tag, "hz": meter.look(), "scaler": scaler_of(read(SCALER)), "win0": win0_of(read(WIN0))}
    if with_ctl:
        r = ctl("retime").get("retime", {})
        s["retime"] = {k: r.get(k) for k in ("phase", "playing", "pid", "tries", "enabled", "show")}
    log.write(json.dumps(s) + "\n")
    log.flush()
    return s


def watch(meter, tag, log, seconds, until=None, every=0.5):
    """Samples for `seconds` (or until `until(sample)` says so); the samples."""
    out, end = [], time.monotonic() + seconds
    while time.monotonic() < end:
        time.sleep(every)
        s = sample(meter, tag, log)
        out.append(s)
        if until and until(s, out):
            break
    return out


def journal(pattern):
    return [l for l in ctl("journal").get("lines", []) if re.search(pattern, l)]


# ---- the stages -----------------------------------------------------------------------------------------------------------------------------------

def stage_read(res):
    r = ctl("retime").get("retime", {})
    m = Meter()
    m.look()
    time.sleep(2.0)
    hz = m.look()
    refresh = ctl("refresh")
    # the player's usage text names `--play` when the build can play a file (a bad codec ends it at once, before it opens anything)
    prog = r.get("player") or FEEDER
    plays = os.path.exists(prog) and "--play" in sh("%s --codec none 2>&1" % prog, 10)
    st = {"retime": r, "foreground": foreground(), "refresh": {k: refresh.get(k) for k in ("hz", "mode")}, "panel_hz": hz,
          "panel_band": band(hz), "scaler": scaler_of(read(SCALER)), "win0": win0_of(read(WIN0)), "players": players(),
          "vwin_mode": ctl("vwin").get("vwin", {}).get("mode"), "player_plays_files": plays}
    st["ready"] = bool(r.get("playerPresent") and plays and st["foreground"] == HOME and st["scaler"]["src"] == "NONE" and not st["players"])
    res["read"] = st
    say("read:", json.dumps(st))
    return st


def stage_play(res, secs):
    ctl("retime hide")
    pre = stage_read(res) if "read" not in res else res["read"]
    if not pre["ready"]:
        res["play"] = {"error": "not ready (the player installed, Home in front, no source, no player): see read"}
        say("play: not ready")
        return
    log = open(OUT + "/samples.jsonl", "a")
    m = Meter()
    m.look()
    start_band = pre["panel_band"]
    ctl("retime on")
    now = ctl("retime now 10")
    say("play: ctl retime now ->", json.dumps(now.get("ok")), now.get("error", ""))
    p = {"started": bool(now.get("ok")), "start_band": start_band}
    during = watch(m, "play", log, 2.0)
    p["status"] = ctl("status")
    p["vwin"] = ctl("vwin").get("vwin", {})
    p["sinks"] = luna("luna://com.webos.service.videooutput/getStatus")
    json.dump(p["sinks"], open(OUT + "/play-videooutput-status.json", "w"), indent=1)
    p["display_during"] = capture("play-during", "DISPLAY")
    p["video_during"] = capture("play-during", "VIDEO")
    p["foreground_video_window"] = luna("luna://com.webos.surfacemanager/getForegroundVideoWindow")
    during += watch(m, "play", log, max(secs - 2.0, 0.5))
    # the run ends the clip itself (the capture below is of Home without it), and waits for the player to go
    ctl("retime stop")
    after = watch(m, "play-after", log, 6.0, until=lambda s, all_: s["scaler"]["src"] == "NONE" and not s["retime"]["playing"] and len(all_) >= 3)
    p["display_after"] = capture("play-after", "DISPLAY")
    last = after[-1] if after else {}
    p["players_after"] = players()
    p["retime_after"] = ctl("retime").get("retime", {})
    p["vwin_after"] = ctl("vwin").get("vwin", {})
    p["journal"] = journal(r"retime:|vwin: window .* retime clip")
    winq = [w for w in p["vwin"].get("windows", []) if w.get("app") == "org.nphil.lumatv.retime"]
    sinks = [s for s in p["sinks"].get("video", []) if s.get("connected")]
    main = next((s for s in sinks if s.get("sink") == "MAIN"), None)
    # what videooutputd says the clip is: the source's size and rate (640 by 360, 30) and where it shows (32,32 256 by 144 in the panel's pixels)
    p["sink"] = {k: main.get(k) for k in ("appId", "width", "height", "frameRate", "displayOutput", "fullScreen", "zOrder")} if main else None
    v = {
        "started": p["started"],
        "decoder_connected": any(s["scaler"]["src"] not in (None, "NONE") for s in during),
        "kernel_read_30fps": any(s["win0"]["vfreq"] in range(295, 306) and s["win0"]["active"] == (640, 360) for s in during),
        "no_hole": p["status"].get("summary", {}).get("holes") == 0 and bool(winq) and all(w.get("quiet") and not w.get("live") for w in winq),
        "placed_small": bool(main) and (main.get("displayOutput", {}).get("width", 9999) <= 400),
        "rate_kept": all(band(s["hz"]) == start_band for s in during if s["hz"] is not None and s["t"] > during[0]["t"]) if start_band else None,
        "cleaned_up": last.get("scaler", {}).get("src") == "NONE" and not p["players_after"] and not p["retime_after"].get("playing"),
    }
    # nothing shows: the region of the clip in the DISPLAY capture is the same with the clip and without it (Home's pixels, not a black window)
    x0, y0, w, h = RECT_UI
    box = (x0 // 2, y0 // 2, (x0 + w) // 2, (y0 + h) // 2)
    a = png_region(p["display_during"], *box) if p["display_during"] else None
    b = png_region(p["display_after"], *box) if p["display_after"] else None
    p["region_luma"] = {"during": a, "after": b}
    v["hidden"] = None if a is None or b is None or b < 40 else abs(a - b) < 8
    v["hidden_note"] = ("Home's own pixels there are dark (or a capture is missing): look at play-during-display.png at 8,8 to 72,44"
                        if v["hidden"] is None else "the region's mean luma is the same with the clip and without it")
    p["verdict"] = v
    res["play"] = p
    say("play verdict:", json.dumps(v))


def stage_restore(res, clip25, manual, wait):
    pre = stage_read(res) if "read" not in res else res["read"]
    log = open(OUT + "/samples.jsonl", "a")
    m = Meter()
    m.look()
    ctl("retime on")
    r = {"provoke": "manual" if manual else "clip25"}
    if not manual:
        if not (clip25 and os.path.exists(clip25) and os.path.exists(FEEDER)):
            res["restore"] = {"error": "--clip25 FILE and the feeder are needed (see the header)"}
            say("restore: no clip or no feeder")
            return
        if pre["panel_band"] != 120 or pre["foreground"] != HOME or pre["vwin_mode"] not in ("idle", "apps"):
            res["restore"] = {"error": "start from 120 Hz, Home in front and vwin idle or apps (see read)", "read": pre}
            say("restore: not at 120 Hz on Home")
            return
        env = "XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0"
        sh("%s %s --play %s --size 640x360 --codec h264 --fps 25 --seconds 9 --loop -w video -a %s --ndl-app %s --stats %s/p25-stats.jsonl "
           "> %s/p25.log 2>&1 &" % (env, FEEDER, clip25, APP25, HOME, OUT, OUT))
        time.sleep(1.5)
        say("restore: claim p25:", json.dumps(ctl("vwin claim p25 pip 1280 720 480 270 %s h264 640x360@25" % APP25).get("claim", {}).get("state")))
        seen = watch(m, "provoke", log, 10.0, until=lambda s, all_: band(s["hz"]) == 100 and sum(1 for x in all_ if band(x["hz"]) == 100) >= 3)
        r["provoked"] = any(band(s["hz"]) == 100 for s in seen)
        say("restore: provoked:", r["provoked"])
        time.sleep(1.0)
        ctl("vwin release p25")
        end_player(APP25)
    else:
        say("restore: waiting up to %d s for the panel to be held at 100 Hz with Home in front: open the Ring's full view and press Back now" % wait)
        seen = watch(m, "provoke", log, wait, until=lambda s, all_: band(s["hz"]) == 100 and s["scaler"]["src"] == "NONE" and foreground() == HOME)
        r["provoked"] = bool(seen) and band(seen[-1]["hz"]) == 100
    if not r["provoked"]:
        r["error"] = "the panel was not held at 100 Hz: nothing to restore"
        res["restore"] = r
        return
    t_release = time.monotonic()
    watched = watch(m, "restore", log, wait, until=lambda s, all_: sum(1 for x in all_[-3:] if band(x["hz"]) == 120) >= 3 and not s["retime"]["playing"])
    back = next((s for s in watched if band(s["hz"]) == 120), None)
    r["seconds_to_120"] = round(back["t"] - (t_release - T0), 1) if back else None
    # it stays: ten more seconds with no player and no second switch
    stay = watch(m, "stay", log, 10.0)
    r["journal"] = journal(r"retime:|vwin: window .* retime clip")
    r["retime_after"] = ctl("retime").get("retime", {})
    r["players_after"] = players()
    r["verdict"] = {
        "restored": back is not None,
        "by_the_retime": any("the panel is back at 120 Hz" in l for l in r["journal"]),
        "stays_120": all(band(s["hz"]) == 120 for s in stay if s["hz"] is not None),
        "cleaned_up": not r["players_after"] and not r["retime_after"].get("playing") and scaler_of(read(SCALER))["src"] == "NONE",
        "no_blink_seen": None,
        "no_blink_note": "needs eyes: the owner watches the screen at the moment the journal says the clip began and the moment the panel is back",
    }
    res["restore"] = r
    say("restore verdict:", json.dumps(r["verdict"]))


def finish(res, found):
    """Everything this run did is undone: the clip stopped and hidden, no claim of ours, no player of ours, the retime as it was found."""
    ctl("retime stop")
    ctl("retime hide")
    ctl("vwin release p25")
    end_player(APP25)
    if found is not None:
        ctl("retime on" if found else "retime off")
    time.sleep(1.0)
    r = ctl("retime").get("retime", {})
    res["end"] = {"retime": {k: r.get(k) for k in ("enabled", "show", "phase", "playing")}, "players": players(), "foreground": foreground(),
                  "refresh": ctl("refresh").get("mode"), "claims": ctl("vwin").get("vwin", {}).get("claims")}
    say("end:", json.dumps(res["end"]))


def selftest():
    assert osd_count(" 68:     240000      12       0       3     GICv2 104 Edge      osd_irq\n 9: 5 GICv2 Edge other\n") == 240015
    assert osd_count("9: 5 GICv2 Edge other") is None
    assert (band(99.5), band(120.4), band(110), band(None)) == (100, 120, None, None)
    assert scaler_of("[main.p0]\nconnect_src=VDEC\nsource_w=640\nsource_h=360\nout_w=3840\nout_h=2160\n") == {"src": "VDEC", "source": ("640", "360"), "out": ("3840", "2160")}
    assert win0_of("localmute=777\nh/v active : 640 / 360\nvFreq      : 300\n") == {"active": (640, 360), "vfreq": 300}
    assert first_json('ERROR: ld.so: preload ignored\n{"ok": true, "retime": {"phase": "idle"}}\nnote') == {"ok": True, "retime": {"phase": "idle"}}
    assert first_json("no json here") == {} and first_json("{broken") == {}
    assert is_for(["/v/lumatv-gfn-feeder", "--play", "c.h264", "-a", "org.a"], "org.a") and not is_for(["f", "-a", "org.ab"], "org.a") and not is_for(["f"], "org.a")
    # a PNG of the kinds the capture service writes: RGB, filters 0, 1, 2, 4 on four rows
    w, h = 8, 4
    rows = [bytes([(x * 30 + y * 7) % 256 for x in range(w) for _ in range(3)]) for y in range(h)]
    raw, prev = b"", bytes(w * 3)
    for y, row in enumerate(rows):
        f = (0, 1, 2, 4)[y]
        enc = bytearray()
        for i, v in enumerate(row):
            a = row[i - 3] if i >= 3 else 0
            b, c = prev[i], (prev[i - 3] if i >= 3 else 0)
            if f == 1:
                pred = a
            elif f == 2:
                pred = b
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if pa <= pb and pa <= pc else b if pb <= pc else c
            else:
                pred = 0
            enc.append((v - pred) & 255)
        raw += bytes([f]) + bytes(enc)
        prev = row

    def chunk(k, body):
        return struct.pack(">I", len(body)) + k + body + struct.pack(">I", zlib.crc32(k + body) & 0xffffffff)

    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    path = "/tmp/retime-probe-selftest.png"
    open(path, "wb").write(png)
    want = sum(0.299 * r + 0.587 * g + 0.114 * b for y in range(1, 3) for x in range(2, 5) for r, g, b in [rows[y][x * 3:x * 3 + 3]]) / 6
    got = png_region(path, 2, 1, 5, 3)
    assert got is not None and abs(got - want) < 1e-6, (got, want)
    os.remove(path)
    print("selftest ok")


def main(argv):
    if not argv or argv[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    stage = argv[0]
    if stage == "selftest":
        selftest()
        return 0
    opt = lambda name, default=None: argv[argv.index(name) + 1] if name in argv else default
    os.makedirs(OUT, exist_ok=True)
    res = {"stage": stage, "time": time.strftime("%Y-%m-%d %H:%M:%S")}
    found = ctl("retime").get("retime", {}).get("enabled")
    try:
        if stage in ("read", "play", "restore", "all"):
            stage_read(res)
        if stage in ("play", "all"):
            stage_play(res, float(opt("--secs", 5.0)))
        if stage in ("restore", "all"):
            stage_restore(res, opt("--clip25"), "--manual" in argv or not opt("--clip25"), float(opt("--wait", 40)))
    finally:
        if stage != "read":
            finish(res, found)
        json.dump(res, open(OUT + "/result.json", "w"), indent=1)
        say("result:", OUT + "/result.json")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
