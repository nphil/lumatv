#!/usr/bin/env python3
"""The retime clip on the set (`compositor/.../retime.rs`, docs/research/vwin-refresh-result.md "Panel left at 100 Hz after a full view").

Run ON THE SET as root, under a PTY (luna-send needs one), with LumaTV's compositor live, Home in front and the receiver muted. Stdlib only.

    tv-retime-probe.py read                         the state, nothing changed
    tv-retime-probe.py play [--secs N] [--control]  the clip by hand (`ctl retime now`) with the panel as it is: does the decoder connect under the UI, does the
                                                    display engine read 30 fps, is the window the whole output with no hole, and does the TV's picture controller
                                                    leave the picture alone (the bus, below). --control then plays the clip small, with a hole, for 4 s: the
                                                    positive control, where the picture controller must change the colours and the run must say so
    tv-retime-probe.py restore --clip25 FILE        puts the panel at 100 Hz (a 25 fps clip as a claim of the whole screen for a few seconds: the screen is black
                                                    for that time, as the Ring's view is) and waits for the compositor to bring it back
    tv-retime-probe.py restore --manual             the same without the claim: the run waits for the panel to be held at 100 Hz (open the Ring's full view and
                                                    press Back) and watches the compositor bring it back
    tv-retime-probe.py all --clip25 FILE            read, play, restore
    tv-retime-probe.py selftest                     the readers on text (runs on the host)

The picture path. A video that connects to the main sink makes videooutputd write `dimensionInfo` and call pqcontroller's `setPQ`; pqcontroller then applies the
whole picture menu again. A small video window (picture-in-graphics, "PIG") makes it apply colour gamut 0 and local dimming 0 over the owner's values and grey out
six menu rows (`setSystemSettingDescActive`, `ui.active` false, where a whole-screen video gets `resetSystemSettingDesc`): that is the colour shift Nitin saw with
the 25 fps box. This run reads both: `ls-monitor` (setPQ with the window it was told, the `dimensionInfo` writes, the picture settings pqcontroller read, every
`setSystemSettingDescActive`) into monitor.jsonl, and `getSystemSettingDesc` for the six rows every sample. A verdict `pq_no_pig` is true when no row went from
active to inactive and the bus shows no `setSystemSettingDescActive` with `active` false in the clip's seconds.

Evidence goes to /tmp/lumatv/out/retime/ (samples.jsonl, monitor.jsonl, result.json, the captures and the sink status of `play`); fetch the directory. The 25 fps
clip is made on the workstation and put on the set (/tmp/gfn/retime25.h264):
    ffmpeg -f lavfi -i color=c=black:size=640x360:rate=25 -t 1 -c:v libx264 -profile:v high -pix_fmt yuv420p -preset veryslow \\
      -x264-params aud=1:repeat-headers=1:slices=1:bframes=0:keyint=25:scenecut=0:ref=1:force-cfr=1 -bsf:v filter_units=remove_types=6 -f h264 retime25.h264

The retime is off by default; `play` and `restore` switch it on (`ctl retime on`) and put it back as it was found. What it never does: write /proc/lg/de/fr_rate (the
write the firmware ignores), launch an app, touch a picture setting, change the vwin mode or leave a claim, a player or the clip's `show` switch behind. What decides
each verdict is in `result.json` next to its number; a verdict that needs eyes says so (`null`): nothing black or coloured shows at the clip's start, during it or
at its end, and the colours do not move.
"""
import json
import os
import re
import signal
import subprocess
import sys
import threading
import time

OUT = "/tmp/lumatv/out/retime"
CTL = ("/var/lib/lumatv/compositor/lib/ld-linux-aarch64.so.1 --library-path /var/lib/lumatv/compositor/lib "
       "/var/lib/lumatv/compositor/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg ")
FEEDER = "/var/lib/lumatv/bin/lumatv-gfn-feeder"
SCALER = "/proc/lgtv-driver/scaler/main.p0"
WIN0 = "/proc/lg/de/win0/status"
HOME = "com.webos.app.home"
APP25 = "org.nphil.lumatv.retime25"
# the picture rows the controller greys out while a small video window plays (picture-in-graphics; TV 2026-10-30 `luna-e1` 05:23:44.73)
PIG_KEYS = ("colorTemperature", "whiteBalanceActive", "eyeComfortMode", "gamma", "dynamicContrast", "dynamicColor")
# the services of the picture path, as ls-monitor names them
MONITOR_RE = re.compile(r"videooutput|pqcontroller|settings|oledepl|panelcontroller")
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


def first_json(text):
    """The first JSON object in a command's output (luna-send and the ctl print notes around it), or {}."""
    i = text.find("{")
    try:
        return json.JSONDecoder().raw_decode(text[i:])[0] if i >= 0 else {}
    except ValueError:
        return {}


def is_for(argv, app):
    """Whether a feeder's arguments say it is the window of `app` (`-a APP`)."""
    return "-a" in argv and argv[argv.index("-a") + 1:][:1] == [app]


def holes_of(status):
    """The video holes the scene holds, from `ctl status` (`held.holes`; its `summary` is a line of text)."""
    held = status.get("held")
    return held.get("holes") if isinstance(held, dict) else None


def desc_active_of(reply):
    """key -> `ui.active` of a `getSystemSettingDesc` reply (None where the row does not say)."""
    out = {}
    for e in reply.get("results") or []:
        if isinstance(e, dict) and e.get("key"):
            out[e["key"]] = (e.get("ui") or {}).get("active")
    return out


def pq_summary(rows, t0, t1):
    """What the picture path did between the uptimes t0 and t1. `rows` are (uptime, ls-monitor line); a message is counted once, on the side that sent it."""
    s = {"setpq": [], "dimension_info": [], "picture_writes": [], "menu_disabled": [], "menu_enabled": [], "picture_modes": [], "dynamic_ranges": [], "lines": 0}
    modes, ranges = set(), set()
    for up, line in rows:
        if not t0 <= up <= t1:
            continue
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if not isinstance(m, dict) or m.get("transport", "TX") != "TX":
            continue
        p = m.get("payload") if isinstance(m.get("payload"), dict) else {}
        method = m.get("method") or p.get("method")
        s["lines"] += 1
        if method == "setPQ":
            v = p.get("video") if isinstance(p.get("video"), dict) else {}
            out, src = v.get("displayOutput") or {}, v.get("sourceInput") or {}
            s["setpq"].append({"t": round(up - t0, 2), "out": [out.get("x"), out.get("y"), out.get("width"), out.get("height")], "src": [src.get("width"), src.get("height")],
                               "full": v.get("fullScreen"), "fps": v.get("frameRate"), "connected": v.get("connected")})
        elif method == "setSystemSettings" and p.get("category") == "dimensionInfo":
            s["dimension_info"].append(p.get("settings"))
        elif method == "setSystemSettings" and p.get("category") == "picture":
            s["picture_writes"].append(p.get("settings"))
        elif method == "setSystemSettingDescActive":
            (s["menu_enabled"] if (p.get("ui") or {}).get("active") is not False else s["menu_disabled"]).append(p.get("key"))
        d = p.get("dimension")
        if p.get("category") == "picture" and isinstance(d, dict):
            if d.get("pictureMode"):
                modes.add(d["pictureMode"])
            if d.get("dynamicRange"):
                ranges.add(d["dynamicRange"])
    s["picture_modes"], s["dynamic_ranges"] = sorted(modes), sorted(ranges)
    return s


def pig_in(summary, desc_samples, base, t0, t1):
    """Picture-in-graphics between t0 and t1: the rows that went from active to inactive, and what the bus said. None when nothing was looked at."""
    flipped = sorted({k for s in desc_samples if t0 <= s["up"] <= t1 for k, a in s["desc"].items() if a is False and base.get(k) is True})
    disabled = [k for k in summary["menu_disabled"] if k in PIG_KEYS]
    looked = bool(desc_samples) or summary["lines"] > 0
    return {"pig": (bool(flipped) or bool(disabled)) if looked else None, "rows_off": flipped, "bus_menu_disabled": disabled}


# ---- the set --------------------------------------------------------------------------------------------------------------------------------------

def uptime():
    try:
        with open("/proc/uptime") as f:
            return float(f.read().split()[0])
    except (OSError, ValueError):
        return time.monotonic()


def sh(cmd, timeout=30):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout).stdout
    except subprocess.TimeoutExpired:
        return ""


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


def desc_active():
    """The picture rows a small video window greys out, as the settings service says they are now."""
    return desc_active_of(luna("luna://com.webos.service.settings/getSystemSettingDesc", json.dumps({"category": "picture", "keys": list(PIG_KEYS)})))


def capture(name, method):
    p = "%s/%s-%s.png" % (OUT, name, method.lower())
    r = luna("luna://com.webos.service.capture/executeOneShot", json.dumps({"path": p, "method": method, "format": "PNG", "width": 960, "height": 540}))
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


class Monitor:
    """`ls-monitor -j -e` (the way round 11's probe ran it): the lines that name a service of the picture path, with the uptime they arrived at."""

    def __init__(self):
        self.rows, self.proc, self.thread, self.error = [], None, None, None

    def start(self):
        try:
            self.proc = subprocess.Popen(["ls-monitor", "-j", "-e"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
        except OSError as e:
            self.error = str(e)
            return False
        self.thread = threading.Thread(target=self._read, daemon=True)
        self.thread.start()
        return True

    def _read(self):
        for line in self.proc.stdout:
            if MONITOR_RE.search(line):
                self.rows.append((uptime(), line.strip()))

    def stop(self):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(3)
            except subprocess.TimeoutExpired:
                self.proc.kill()
        if self.thread:
            self.thread.join(2)

    def save(self, path):
        with open(path, "a") as f:
            for up, line in self.rows:
                f.write(json.dumps({"up": up, "line": line}) + "\n")


def sample(meter, tag, log, pq=False):
    s = {"t": round(time.monotonic() - T0, 2), "up": uptime(), "tag": tag, "hz": meter.look(), "scaler": scaler_of(read(SCALER)), "win0": win0_of(read(WIN0))}
    r = ctl("retime").get("retime", {})
    s["retime"] = {k: r.get(k) for k in ("phase", "playing", "pid", "tries", "enabled", "show")}
    if pq:
        s["desc"] = desc_active()
    log.write(json.dumps(s) + "\n")
    log.flush()
    return s


def watch(meter, tag, log, seconds, until=None, every=0.5, pq=False):
    """Samples for `seconds` (or until `until(sample, all)` says so); the samples."""
    out, end = [], time.monotonic() + seconds
    while time.monotonic() < end:
        time.sleep(every)
        s = sample(meter, tag, log, pq)
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
          "vwin_mode": ctl("vwin").get("vwin", {}).get("mode"), "player_plays_files": plays, "picture_rows_active": desc_active(),
          "ls_monitor": bool(sh("command -v ls-monitor", 5).strip())}
    st["ready"] = bool(r.get("playerPresent") and plays and st["foreground"] == HOME and st["scaler"]["src"] == "NONE" and not st["players"])
    res["read"] = st
    say("read:", json.dumps(st))
    return st


def clip_window(res, tag, secs, small, mon, log, meter, base):
    """One clip by hand: `ctl retime now`, shown small with its hole when `small`. The samples and what the picture path did."""
    ctl("retime show" if small else "retime hide")
    t0 = uptime()
    now = ctl("retime now %d" % max(2, int(secs)))
    say(tag, ": ctl retime now ->", json.dumps(now.get("ok")), now.get("error", ""))
    p = {"started": bool(now.get("ok")), "small": small}
    during = watch(meter, tag, log, 2.0, pq=True)
    p["status"] = ctl("status")
    p["vwin"] = ctl("vwin").get("vwin", {})
    p["sinks"] = luna("luna://com.webos.service.videooutput/getStatus")
    p["display_during"] = capture(tag, "DISPLAY")
    p["video_during"] = capture(tag, "VIDEO")
    during += watch(meter, tag, log, max(secs - 2.0, 0.5), pq=True)
    ctl("retime stop")
    t1 = uptime()
    after = watch(meter, tag + "-after", log, 6.0, until=lambda s, all_: s["scaler"]["src"] == "NONE" and not s["retime"]["playing"] and len(all_) >= 3, pq=True)
    ctl("retime hide")
    p["players_after"] = players()
    p["retime_after"] = ctl("retime").get("retime", {})
    p["journal"] = journal(r"retime:|vwin: window .* retime clip")
    winq = [w for w in p["vwin"].get("windows", []) if w.get("app") == "org.nphil.lumatv.retime"]
    main = next((s for s in p["sinks"].get("video", []) if s.get("connected") and s.get("sink") == "MAIN"), None)
    # what videooutputd says the clip is: the source's size and rate (640 by 360, 30) and where it shows (the panel's whole 3840 by 2160, full screen)
    p["sink"] = {k: main.get(k) for k in ("appId", "width", "height", "frameRate", "displayOutput", "fullScreen", "zOrder")} if main else None
    out = (main or {}).get("displayOutput") or {}
    whole = bool(main) and (out.get("x"), out.get("y"), out.get("width"), out.get("height")) == (0, 0, 3840, 2160) and main.get("fullScreen") is True
    last = after[-1] if after else {}
    p["pq"] = pq_summary(mon.rows, t0 - 0.5, t1 + 2.0)
    p["pig"] = pig_in(p["pq"], during + after, base, t0 - 0.5, t1 + 2.0)
    p["verdict"] = {
        "started": p["started"],
        "decoder_connected": any(s["scaler"]["src"] not in (None, "NONE") for s in during),
        "kernel_read_30fps": any(s["win0"]["vfreq"] in range(295, 306) and s["win0"]["active"] == (640, 360) for s in during),
        "no_hole": (holes_of(p["status"]) == 0 and bool(winq) and all(w.get("quiet") and not w.get("live") for w in winq)) if not small else None,
        "window_whole_output": whole if not small else None,
        "pig": p["pig"]["pig"],
        "picture_modes_read": p["pq"]["picture_modes"],
        "cleaned_up": last.get("scaler", {}).get("src") == "NONE" and not p["players_after"] and not p["retime_after"].get("playing"),
    }
    return p, during


def stage_play(res, secs, control):
    ctl("retime hide")
    pre = stage_read(res) if "read" not in res else res["read"]
    if not pre["ready"]:
        res["play"] = {"error": "not ready (the player installed, Home in front, no source, no player): see read"}
        say("play: not ready")
        return
    log = open(OUT + "/samples.jsonl", "a")
    meter = Meter()
    meter.look()
    base = pre["picture_rows_active"]
    mon = Monitor()
    started = mon.start()
    ctl("retime on")
    p, during = clip_window(res, "play", secs, False, mon, log, meter, base)
    v = p["verdict"]
    v["rate_kept"] = all(band(s["hz"]) == pre["panel_band"] for s in during if s["hz"] is not None and s["t"] > during[0]["t"]) if pre["panel_band"] else None
    v["pq_no_pig"] = None if p["pig"]["pig"] is None else not p["pig"]["pig"]
    v["pq_picture_mode_kept"] = (set(p["pq"]["picture_modes"]) <= {"filmMaker"}) if p["pq"]["picture_modes"] else None
    v["no_colour_shift_seen"] = None
    v["no_colour_shift_note"] = "needs eyes: nothing black or coloured shows at the clip's start, during it or at its end, and the colours do not move"
    res["play"] = p
    if control:
        say("control: the clip small, with a hole, for 4 s: the colours are meant to shift now")
        time.sleep(3.0)
        c, _ = clip_window(res, "control", 4, True, mon, log, meter, base)
        c["verdict"]["pig_expected"] = True
        c["verdict"]["detector_works"] = c["verdict"]["pig"]
        c["verdict"]["colour_shift_seen"] = None
        c["verdict"]["colour_shift_note"] = "needs eyes: the colours shifted while the small clip was up (the 25 fps box's shift); null here"
        res["control"] = c
    mon.stop()
    mon.save(OUT + "/monitor.jsonl")
    res["monitor"] = {"started": started, "error": mon.error, "lines": len(mon.rows)}
    say("play verdict:", json.dumps(v))


def stage_restore(res, clip25, manual, wait):
    pre = stage_read(res) if "read" not in res else res["read"]
    log = open(OUT + "/samples.jsonl", "a")
    meter = Meter()
    meter.look()
    base = pre["picture_rows_active"]
    mon = Monitor()
    started = mon.start()
    ctl("retime on")
    r = {"provoke": "manual" if manual else "clip25"}
    t_claim = t_release = None
    if not manual:
        if not (clip25 and os.path.exists(clip25) and os.path.exists(FEEDER)):
            res["restore"] = {"error": "--clip25 FILE and the feeder are needed (see the header)"}
            say("restore: no clip or no feeder")
            mon.stop()
            return
        if pre["panel_band"] != 120 or pre["foreground"] != HOME or pre["vwin_mode"] not in ("idle", "apps"):
            res["restore"] = {"error": "start from 120 Hz, Home in front and vwin idle or apps (see read)", "read": pre}
            say("restore: not at 120 Hz on Home")
            mon.stop()
            return
        env = "XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0"
        sh("%s %s --play %s --size 640x360 --codec h264 --fps 25 --seconds 9 --loop -w video -a %s --ndl-app %s --stats %s/p25-stats.jsonl "
           "> %s/p25.log 2>&1 &" % (env, FEEDER, clip25, APP25, HOME, OUT, OUT))
        time.sleep(1.5)
        # the whole screen, as the Ring's full view is: black for these seconds, and a window the picture controller does not take for picture-in-graphics
        t_claim = uptime()
        say("restore: claim p25:", json.dumps(ctl("vwin claim p25 camera-full 0 0 1920 1080 %s h264 640x360@25" % APP25).get("claim", {}).get("state")))
        seen = watch(meter, "provoke", log, 10.0, until=lambda s, all_: band(s["hz"]) == 100 and sum(1 for x in all_ if band(x["hz"]) == 100) >= 3, pq=True)
        r["provoked"] = any(band(s["hz"]) == 100 for s in seen)
        say("restore: provoked:", r["provoked"])
        time.sleep(1.0)
        t_release = uptime()
        ctl("vwin release p25")
        end_player(APP25)
    else:
        say("restore: waiting up to %d s for the panel to be held at 100 Hz with Home in front: open the Ring's full view and press Back now" % wait)
        seen = watch(meter, "provoke", log, wait, until=lambda s, all_: band(s["hz"]) == 100 and s["scaler"]["src"] == "NONE" and foreground() == HOME, pq=True)
        r["provoked"] = bool(seen) and band(seen[-1]["hz"]) == 100
        t_release = uptime()
    if not r["provoked"]:
        r["error"] = "the panel was not held at 100 Hz: nothing to restore"
        res["restore"] = r
        mon.stop()
        return
    t_release_mono = time.monotonic()
    watched = watch(meter, "restore", log, wait, until=lambda s, all_: sum(1 for x in all_[-3:] if band(x["hz"]) == 120) >= 3 and not s["retime"]["playing"], pq=True)
    back = next((s for s in watched if band(s["hz"]) == 120), None)
    r["seconds_to_120"] = round(back["t"] - (t_release_mono - T0), 1) if back else None
    # it stays: ten more seconds with no player and no second switch
    stay = watch(meter, "stay", log, 10.0, pq=True)
    mon.stop()
    mon.save(OUT + "/monitor.jsonl")
    allsamples = watched + stay
    playing = [s["up"] for s in watched if s["retime"]["playing"]]
    r["journal"] = journal(r"retime:|vwin: window .* retime clip")
    r["retime_after"] = ctl("retime").get("retime", {})
    r["players_after"] = players()
    r["monitor"] = {"started": started, "error": mon.error, "lines": len(mon.rows)}
    if t_claim is not None:
        r["pq_provoke"] = pig_in(pq_summary(mon.rows, t_claim - 0.5, t_release + 2.0), seen, base, t_claim - 0.5, t_release + 2.0)
    if playing:
        w0, w1 = min(playing) - 0.5, max(playing) + 1.5
        r["pq_retime"] = {"summary": pq_summary(mon.rows, w0, w1), **pig_in(pq_summary(mon.rows, w0, w1), allsamples, base, w0, w1)}
    r["verdict"] = {
        "restored": back is not None,
        "by_the_retime": any("the panel is back at 120 Hz" in l for l in r["journal"]),
        "stays_120": all(band(s["hz"]) == 120 for s in stay if s["hz"] is not None),
        "cleaned_up": not r["players_after"] and not r["retime_after"].get("playing") and scaler_of(read(SCALER))["src"] == "NONE",
        "pq_no_pig_retime": None if "pq_retime" not in r or r["pq_retime"]["pig"] is None else not r["pq_retime"]["pig"],
        "pq_no_pig_provoke": None if "pq_provoke" not in r or r["pq_provoke"]["pig"] is None else not r["pq_provoke"]["pig"],
        "no_blink_seen": None,
        "no_colour_shift_seen": None,
        "eyes_note": "needs eyes: the owner watches the screen at the moment the journal says the clip began and the moment the panel is back: no blink, nothing black or "
                     "coloured, and the colours do not move",
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
    assert desc_active_of({"results": [{"key": "colorTemperature", "ui": {"active": False}}, {"key": "gamma"}, {"nokey": 1}]}) == {"colorTemperature": False, "gamma": None}
    assert holes_of({"ok": True, "summary": "uptime 1.0s clients 2 surfaces 3", "held": {"surfaces": 3, "holes": 1}}) == 1 and holes_of({"ok": True, "summary": "uptime 1.0s"}) is None
    # the picture path as round 11's monitor and the 2026-10-30 logs have it: a small window, then a whole-screen one
    def msg(method, payload, sender, transport="TX"):
        return json.dumps({"method": method, "sender": sender, "transport": transport, "payload": payload})
    vo, pqc = "com.webos.service.videooutput", "com.webos.service.pqcontroller.plugin"
    small = {"video": {"displayOutput": {"width": 1440, "y": 486, "x": 312, "height": 810}, "sourceInput": {"width": 1080, "y": 0, "x": 0, "height": 608}, "frameRate": 24, "fullScreen": False, "connected": True}}
    whole = {"video": {"displayOutput": {"width": 3840, "y": 0, "x": 0, "height": 2160}, "sourceInput": {"width": 640, "y": 0, "x": 0, "height": 360}, "frameRate": 30, "fullScreen": True, "connected": True}}
    rows = [
        (10.0, msg("setPQ", small, vo)),
        (10.0, msg("setPQ", small, "com.webos.service.pqcontroller", "RX")),
        (10.1, msg("setSystemSettings", {"notifySelf": False, "category": "dimensionInfo", "settings": {"resolution": "sd", "input": "default"}}, vo)),
        (10.2, msg("setSystemSettingDescActive", {"category": "picture", "key": "colorTemperature", "ui": {"active": False}}, pqc)),
        (10.2, msg("setSystemSettingDescActive", {"category": "picture", "key": "gamma", "ui": {"active": False}}, pqc)),
        (10.3, json.dumps({"transport": "TX", "sender": "com.webos.service.settings", "payload": {"returnValue": True, "method": "getSystemSettings", "category": "picture",
                                                                                                   "dimension": {"pictureMode": "filmMaker", "dynamicRange": "sdr", "input": "default"}}})),
        (10.4, msg("setSystemSettings", {"category": "picture", "settings": {"memcDelayLevel": "mid"}}, "com.webos.service.pqcontroller")),
        (20.0, msg("setPQ", whole, vo)),
        (20.1, "not json"),
        (99.0, msg("setPQ", small, vo)),
    ]
    a = pq_summary(rows, 9.5, 11.0)
    assert [x["out"] for x in a["setpq"]] == [[312, 486, 1440, 810]] and a["setpq"][0]["full"] is False and a["lines"] == 6, a
    assert a["dimension_info"] == [{"resolution": "sd", "input": "default"}] and a["picture_writes"] == [{"memcDelayLevel": "mid"}]
    assert a["menu_disabled"] == ["colorTemperature", "gamma"] and a["picture_modes"] == ["filmMaker"] and a["dynamic_ranges"] == ["sdr"]
    b = pq_summary(rows, 19.5, 21.0)
    assert b["setpq"][0]["out"] == [0, 0, 3840, 2160] and b["setpq"][0]["full"] is True and b["menu_disabled"] == [] and b["lines"] == 1, b
    base = dict.fromkeys(PIG_KEYS, True)
    during = [{"up": 10.5, "desc": {"colorTemperature": False, "eyeComfortMode": True}}, {"up": 30.0, "desc": {"dynamicColor": False}}]
    assert pig_in(a, during, base, 9.5, 11.0) == {"pig": True, "rows_off": ["colorTemperature"], "bus_menu_disabled": ["colorTemperature", "gamma"]}
    assert pig_in(b, [{"up": 20.2, "desc": dict(base)}], base, 19.5, 21.0) == {"pig": False, "rows_off": [], "bus_menu_disabled": []}
    assert pig_in(pq_summary([], 0, 1), [], base, 0, 1)["pig"] is None, "nothing looked at is no answer"
    assert pig_in(pq_summary([], 0, 1), [{"up": 0.5, "desc": {"colorTemperature": False}}], {}, 0, 1)["pig"] is False, "a row that was off before is not the clip's"
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
            stage_play(res, float(opt("--secs", 5.0)), "--control" in argv)
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
