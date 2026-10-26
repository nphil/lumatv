#!/usr/bin/env python3
"""The frame-rate switch probe for the C4 (docs/research/framerate-result.md "TV probe steps"): what the display engine, the
decoder's clock and the screen do, every 50 ms, from before an app's launch to well into its playback. One file, three roles:

    python3 docs/research/framerate/probe.py --plan [--run NAME]          # the steps and the on-set commands; touches nothing (default)
    python3 docs/research/framerate/probe.py --go --run NAME [--launch JSON] [--keys TOKENS] [--secs N] [--no-standby] [--no-mute]
    python3 docs/research/framerate/probe.py --report DIR                  # summary.txt from a fetched run

    python3 /tmp/fr-probe-<stamp>/probe.py --sample --out DIR --secs N      # on the set: the sampler (uploaded by --go)

`--go` is read-only on the set apart from the app launch (`luna://com.webos.applicationManager/launch`), the key presses of the run
(kb.py on the SSAP key device) and the files under /tmp/fr-probe-<stamp>/ (samples, screenshots, logs). It never writes under
/usr or /var, never sets a picture setting, and never writes /proc/lg/de/fr_rate (the one lever the note names: that write is a
separate experiment, and `--allow-fr-rate-write` is refused here). The receiver is muted through
Home Assistant before anything plays and put back as it was only once Home is in front again; the set is left in Active Standby
(`--no-standby` leaves it on). Picture Mode is read before and after, never set.

What the sampler logs each tick (samples.jsonl, one JSON object a line, `up` = /proc/uptime seconds, the common clock):
  irq        DE2VSYNC and osd_irq totals from /proc/interrupts (the DE's output vsync: 120/s at 120 Hz, 100/s at 100 Hz)
  flip       /proc/lg/fbdev/fb0/flip_cnt (the UI plane's flips)
  win0       /proc/lg/de/win0/status: vFreq (the source rate the DE sees, 0.1 Hz), localmute (the DE firmware's internal mute reason,
             0 = showing), lowdelaymode, hdr_type, h/v active
  scaler     /proc/lgtv-driver/scaler/main.p0: connect_src, mute, hdr, adaptive, out_w/out_h
  vsync      /proc/lxvo/vdisp/vsync: each decoder vsync port's rate and its fixed flag (HFR pins it)
  vrate      /proc/lxvo/vdisp/vrate: each instance's decided rate and where it came from (CODEC, CONF, PTS, FIXED)
  vdisp      /proc/lxvo/vdisp/vdisp: film mode, frame count, paused, the frame's own rate and size
  vo         /proc/lxvo/<vo>/status: flags (hfr, dolby, svp, lowdelay), frames matched / dropped / freerun
Beside it, as their own files: shots.jsonl (a 320x180 PNG of the screen through com.webos.service.capture as fast as it goes,
with its size: under BLANK_BYTES is a blank screen; luma per shot is computed by --report when PIL is there), videooutput.jsonl
(a getStatus subscription: frameRate, muted, connected, appId per sink), monitor.jsonl (ls-monitor -j, the videooutput, avmutemanager,
pqcontroller and avconnector lines: the app's own mute on and off), marks.txt (launch, keys, home, written by --go), kernel.txt
(the DE and decoder log lines of the switch: set_frate, timing_changed, 4k HFR mode, mute-on/off, FrameDuration90k, [VSYNC) and
before.txt (ovi/de status, cmdline, the OSD vsync divider, picture mode).

Protected video (Netflix) comes back black in a screen capture, so for `protected` runs the shot detector reports only "blank
from the capture's view" and the verdict rests on localmute, the vsync count and videooutput's mute. Nothing here is proven on the
set: tests/test_framerate_probe.py runs the parsers and the report on text written from LG's kernel source and the X4 capture.
"""
import argparse
import json
import os
import re
import shlex
import signal
import subprocess
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", "..", ".."))
NETFLIX_MERIDIAN = "m=https://www.netflix.com/watch/80141336&source_type=4"    # 59.94 fps 4K Dolby Vision (ambient-result.md, X4)
NETFLIX_24 = "m=https://www.netflix.com/watch/80049832&source_type=4"          # Our Planet: 23.976 fps 4K DV [the id is a guess; vFreq says what played]
YOUTUBE_50 = "https://www.youtube.com/results?search_query=4k+50fps+test"
HA = "http://192.168.1.146"
RECEIVER = "media_player.living_room_avr"
PERIOD = 0.05
SHOT_W, SHOT_H = 320, 180
BLANK_BYTES = 5000               # a 320x180 PNG of a black screen (perf/startup.py: blank under 5000, a presented frame over 20000)
BLANK_LUMA = 12.0                # mean luma (0-255) under which a shot is black
FROZEN_DIFF = 1.5                # mean absolute luma difference under which two consecutive shots are the same picture
RATE_WINDOW_S = 1.0              # the output rate is the vsync count over this window
# osd_irq follows the panel's refresh (120 Hz steady on the set through Home and a 23.976 Netflix title, 2026-10-26);
# DE2VSYNC also counts input-side events (150-245 Hz on the same runs), so it is kept for the record only.
DE_IRQ, OSD_IRQ = "DE2VSYNC", "osd_irq"

# The runs: the launch, the keys after `settle` seconds (kb.py tokens: KEY, sS), the playback length, what follows.
RUNS = {
    "netflix24": {"launch": {"id": "netflix", "params": {"contentTarget": NETFLIX_24}}, "settle": 14, "keys": "OK", "after_keys": 2,
                  "secs": 45, "protected": True,
                  "what": "Netflix, a 23.976 fps 4K Dolby Vision title: the no-switch case (120 Hz stays) and the DCO slew to 119.88"},
    "netflix60": {"launch": {"id": "netflix", "params": {"contentTarget": NETFLIX_MERIDIAN}}, "settle": 14, "keys": "OK", "after_keys": 2,
                  "secs": 45, "protected": True,
                  "what": "Netflix, Meridian (59.94 fps 4K Dolby Vision): the 2:2 case at 119.88 Hz"},
    "youtube50": {"launch": {"id": "youtube.leanback.v4", "params": {"contentTarget": YOUTUBE_50}}, "settle": 12, "keys": "OK", "after_keys": 2,
                  "secs": 45, "then_home": 10,
                  "what": "YouTube, a 50 fps clip (the first search result; --launch names a known one): the 120 -> 100 Hz switch at the "
                          "first frame, and whether 100 Hz holds over on Home afterwards"},
    "plex25after24": {"launch": {"id": "com.beb.plxnative"}, "settle": 4, "keys": "OK s4 OK s5 OK", "after_keys": 2, "secs": 30,
                      "second": {"keys": "BACK s2 DOWN s1 OK s4 OK", "after_keys": 2, "secs": 30},
                      "what": "Plex, a 24 fps item then a 25 fps item without going Home (the library order picks them; --keys and "
                              "--second-keys change the path): the mid-play 120 -> 100 switch"},
    "gfn120": {"launch": {"id": "com.geforcenow.play"}, "settle": 20, "keys": "", "after_keys": 0, "secs": 45,
               "what": "GeForce NOW at 4K120 (a session must be started by hand once the app is up): both HFR switches on, 120 Hz 1:1"},
}
MARKS = ("launch", "keys", "second", "home", "end")
KERNEL_PATTERNS = re.compile(r"set_frate|timing_changed|HFR mode|mute-on|mute-off|FrameDuration90k|\[VSYNC|hfr mode|"
                             r"KWRAP_DE_SetHighFrameRate|SetFrRate|uHighFrameRate|Current frame rate|rate limit|direct_vid")


# ---- parsers (pure; tests/test_framerate_probe.py) -------------------------------------------------------------------------

def parse_interrupts(text, names=(DE_IRQ, OSD_IRQ)):
    """/proc/interrupts: the total count (all CPUs) of each line whose name is in `names`."""
    out = {}
    for line in text.splitlines():
        parts = line.split()
        if len(parts) < 2 or not parts[0].endswith(":"):
            continue
        counts = []
        for p in parts[1:]:
            if p.isdigit():
                counts.append(int(p))
            else:
                break
        for name in names:
            if name in line and name not in out:
                out[name] = sum(counts)
    return out


def parse_kv(text):
    """`key=value` and `key : value` lines (the DE's win status, scaler status, vo status) as {key: value}, numbers as ints."""
    out = {}
    for line in text.splitlines():
        m = re.match(r"\s*([A-Za-z0-9_./ -]+?)\s*[=:]\s*(.*?)\s*$", line)
        if not m:
            continue
        key, val = m.group(1).strip(), m.group(2)
        if key in out:
            continue
        out[key] = int(val) if re.fullmatch(r"-?\d+", val) else val
    return out


def _framerate(val):
    """`24000/1001(23.97)` -> (24000, 1001, 23.97)."""
    m = re.match(r"(\d+)/(\d+)\((\d+\.\d+)\)", val or "")
    return (int(m.group(1)), int(m.group(2)), float(m.group(3))) if m else None


def parse_vsync(text):
    """/proc/lxvo/vdisp/vsync: [{ch, framerate: (res, div, hz), interlaced, intv_us, vsyncs: [{id, fixed_frame_rate, framerate}]}]."""
    ports, port = [], None
    for line in text.splitlines():
        m = re.match(r"=== VSYNC PORT\[(\d+)\] ===", line)
        if m:
            port = {"ch": int(m.group(1)), "framerate": None, "interlaced": None, "intv_us": None, "vsyncs": []}
            ports.append(port)
            continue
        if port is None:
            continue
        m = re.match(r"\s*-- VSYNC\[(\d+)\] --", line)
        if m:
            port["vsyncs"].append({"id": int(m.group(1)), "fixed_frame_rate": None, "framerate": None})
            continue
        m = re.match(r"(\s*)([a-z_]+)\s*:\s*(.*?)\s*$", line)
        if not m:
            continue
        indented, key, val = bool(m.group(1)), m.group(2), m.group(3)
        target = port["vsyncs"][-1] if indented and port["vsyncs"] else port
        if key == "framerate":
            target["framerate"] = _framerate(val)
        elif key == "fixed_frame_rate" and target is not port:
            target["fixed_frame_rate"] = int(val)
        elif key == "interlaced" and target is port:
            port["interlaced"] = int(val)
        elif key == "intv":
            m2 = re.match(r"\d+\((\d+)us\)", val)
            port["intv_us"] = int(m2.group(1)) if m2 else None
    return ports


def parse_vrate(text):
    """/proc/lxvo/vdisp/vrate: [{id, rate_src, status: (res, div, hz), duration, use_fixed, fixed: (res, div, hz), framerate_max}]."""
    out, cur, section = [], None, None
    for line in text.splitlines():
        m = re.match(r"===\s+VRATE\[(\d+)\] ===", line)
        if m:
            cur = {"id": int(m.group(1)), "rate_src": None, "status": None, "duration": None, "use_fixed": None, "fixed": None, "framerate_max": None}
            out.append(cur)
            section = None
            continue
        if cur is None:
            continue
        m = re.match(r"- ([A-Z]+) -", line.strip())
        if m:
            section = m.group(1)
            continue
        m = re.match(r"\s*([a-z_]+)\s*:\s*(.*?)\s*$", line)
        if not m:
            continue
        key, val = m.group(1), m.group(2)
        if section == "STATUS":
            if key == "rate_src":
                cur["rate_src"] = val
            elif key == "framerate":
                cur["status"] = _framerate(val)
            elif key == "duration":
                cur["duration"] = int(val)
        elif section == "FIXED":
            if key == "use_fixed":
                cur["use_fixed"] = int(val)
            elif key == "framerate":
                cur["fixed"] = _framerate(val)
        elif section is None and key == "framerate_max":
            cur["framerate_max"] = int(val)
    return out


def parse_vdisp(text):
    """/proc/lxvo/vdisp/vdisp: [{id, port, is_film_mode, frame_cnt, paused, qlen, frame: {frame_rate, width, height, variable_framerate}}]."""
    out, cur, in_frame = [], None, False
    for line in text.splitlines():
        m = re.match(r"=== VDISP\[(\d+)\]", line)
        if m:
            cur = {"id": int(m.group(1)), "port": None, "is_film_mode": None, "frame_cnt": None, "paused": None, "qlen": None,
                   "frame": {"frame_rate": None, "width": None, "height": None, "variable_framerate": None}}
            out.append(cur)
            in_frame = False
            continue
        if cur is None:
            continue
        if line.strip() == "- FRAME -":
            in_frame = True
            continue
        m = re.match(r"\s*([A-Za-z_().,]+)\s*:\s*(.*?)\s*$", line)
        if not m:
            continue
        key, val = m.group(1), m.group(2)
        if in_frame:
            if key == "frame_rate":
                cur["frame"]["frame_rate"] = _framerate(val)
            elif key in ("width", "height", "variable_framerate"):
                cur["frame"][key] = int(val)
        else:
            if key == "port":
                m2 = re.match(r"(-?\d+)\((.*)\)", val)
                cur["port"] = m2.group(2) if m2 else val
            elif key in ("is_film_mode", "frame_cnt", "paused", "qlen"):
                cur[key] = int(val)
    return out


def parse_vo_status(text):
    """/proc/lxvo/<vo>/status: the k=v lines, `flags` split into a list."""
    d = parse_kv(text)
    if isinstance(d.get("flags"), str):
        d["flags"] = [f for f in re.split(r"[|, ]+", d["flags"]) if f and f != "none"]
    return d


def parse_capture_reply(text):
    """luna-send's answer to executeOneShot: writtenBytes, or None when it did not say."""
    m = re.search(r'"writtenBytes"\s*:\s*(\d+)', text)
    return int(m.group(1)) if m else None


def json_objects(text):
    """Every JSON object in luna-send's output (one a line, or pretty-printed over lines), in order."""
    out, depth, start = [], 0, None
    for i, ch in enumerate(text):
        if ch == "{":
            if depth == 0:
                start = i
            depth += 1
        elif ch == "}" and depth:
            depth -= 1
            if depth == 0 and start is not None:
                try:
                    out.append(json.loads(text[start:i + 1]))
                except ValueError:
                    pass
                start = None
    return out


def parse_marks(text):
    """marks.txt: `<uptime> <name>` lines -> [(uptime, name)]."""
    out = []
    for line in text.splitlines():
        p = line.split(None, 1)
        if len(p) == 2:
            try:
                out.append((float(p[0]), p[1].strip()))
            except ValueError:
                pass
    return out


def kernel_lines(text):
    """The log lines about the switch (dmesg or /var/log/messages), kept in order."""
    return [l for l in text.splitlines() if KERNEL_PATTERNS.search(l)]


# ---- the sampler (on the set) -----------------------------------------------------------------------------------------------

def read(path):
    try:
        # /var/log/messages carries stray non-UTF-8 bytes (0xf7 seen on the set).
        with open(path, encoding="utf-8", errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def uptime():
    try:
        return float(read("/proc/uptime").split()[0])
    except (IndexError, ValueError):
        return time.time()


def vo_paths():
    try:
        return sorted(os.path.join("/proc/lxvo", d, "status") for d in os.listdir("/proc/lxvo") if d != "vdisp")
    except OSError:
        return []


def tick(vos):
    """One sample of everything the note names, from /proc only."""
    win0 = parse_kv(read("/proc/lg/de/win0/status"))
    scaler = parse_kv(read("/proc/lgtv-driver/scaler/main.p0"))
    row = {"t": round(time.time(), 3), "up": uptime(),
           "irq": parse_interrupts(read("/proc/interrupts")),
           "flip": parse_kv(read("/proc/lg/fbdev/fb0/flip_cnt")) or read("/proc/lg/fbdev/fb0/flip_cnt").strip()[:40],
           "win0": {k: win0.get(k) for k in ("vFreq", "localmute", "lowdelaymode", "hdr_type", "h/v active", "scanType", "svp")},
           "scaler": {k: scaler.get(k) for k in ("connect_src", "mute", "hdr", "adaptive", "out_w", "out_h", "freeze")},
           "vsync": parse_vsync(read("/proc/lxvo/vdisp/vsync")),
           "vrate": parse_vrate(read("/proc/lxvo/vdisp/vrate")),
           "vdisp": parse_vdisp(read("/proc/lxvo/vdisp/vdisp")),
           "vo": []}
    for p in vos:
        d = parse_vo_status(read(p))
        if d:
            row["vo"].append({k: d.get(k) for k in ("proc_name", "flags", "framerate_es_num", "framerate_es_den", "framerate_container_num",
                                                    "framerate_container_den", "width", "height", "frames_written", "frames_matched",
                                                    "frames_dropped", "frames_freerun", "queued")})
    return row


class Shots(threading.Thread):
    """Screenshots through LG's capture service, back to back, each with its size and how long the call took."""

    def __init__(self, out, stop):
        super().__init__(daemon=True)
        self.out, self.stop, self.n = out, stop, 0

    def run(self):
        with open(os.path.join(self.out, "shots.jsonl"), "a") as log:
            while not self.stop.is_set():
                path = os.path.join(self.out, "shot-%04d.png" % self.n)
                payload = json.dumps({"path": path, "method": "DISPLAY", "format": "PNG", "width": SHOT_W, "height": SHOT_H})
                t0, up0 = time.time(), uptime()
                try:
                    reply = subprocess.run(["luna-send", "-n", "1", "luna://com.webos.service.capture/executeOneShot", payload],
                                           capture_output=True, text=True, timeout=10).stdout
                except (subprocess.TimeoutExpired, OSError) as e:
                    reply = str(e)
                row = {"n": self.n, "up": up0, "t": round(t0, 3), "ms": round((time.time() - t0) * 1000, 1), "bytes": parse_capture_reply(reply)}
                log.write(json.dumps(row) + "\n")
                log.flush()
                self.n += 1
                time.sleep(0.02)


class Stream(threading.Thread):
    """A long-running command whose output lines are logged with the uptime they arrived at (luna-send -i, ls-monitor -j)."""

    def __init__(self, out, name, cmd, keep=None):
        super().__init__(daemon=True)
        self.out, self.name, self.cmd, self.keep = out, name, cmd, keep
        self.proc = None

    def run(self):
        try:
            self.proc = subprocess.Popen(self.cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, errors="replace", bufsize=1)
        except OSError as e:
            with open(os.path.join(self.out, self.name + ".jsonl"), "a") as log:
                log.write(json.dumps({"up": uptime(), "error": str(e)}) + "\n")
            return
        with open(os.path.join(self.out, self.name + ".jsonl"), "a") as log:
            for line in self.proc.stdout:
                line = line.rstrip("\n")
                if self.keep and not self.keep.search(line):
                    continue
                log.write(json.dumps({"up": uptime(), "line": line}) + "\n")
                log.flush()

    def end(self):
        if self.proc and self.proc.poll() is None:
            self.proc.terminate()


def sample(out, secs, period=PERIOD, shots=True, streams=True):
    """The sampler: `secs` seconds of ticks to samples.jsonl, the side streams, then the kernel lines. SIGTERM ends it early."""
    os.makedirs(out, exist_ok=True)
    stop = threading.Event()
    signal.signal(signal.SIGTERM, lambda *_: stop.set())
    with open(os.path.join(out, "before.txt"), "w") as f:
        for path in ("/proc/cmdline", "/proc/lg/ovi/status", "/proc/lg/de/status", "/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip",
                     "/proc/lg/fbdev/fb0/status"):
            f.write("--- %s\n%s\n" % (path, read(path)))
        f.write("--- raw /proc/lg/de/win0/status\n%s\n" % read("/proc/lg/de/win0/status"))
        f.write("--- raw /proc/lgtv-driver/scaler/main.p0\n%s\n" % read("/proc/lgtv-driver/scaler/main.p0"))
        f.write("--- raw /proc/lxvo/vdisp/vsync\n%s\n" % read("/proc/lxvo/vdisp/vsync"))
    log0 = len(read("/var/log/messages"))
    threads = []
    if shots:
        threads.append(Shots(out, stop))
    if streams:
        threads.append(Stream(out, "videooutput", ["luna-send", "-i", "luna://com.webos.service.videooutput/getStatus", '{"subscribe":true}']))
        threads.append(Stream(out, "monitor", ["ls-monitor", "-j", "-e"], re.compile(r"videooutput|avmutemanager|pqcontroller|avconnector")))
    for t in threads:
        t.start()
    t_end = time.time() + secs
    vos = vo_paths()
    with open(os.path.join(out, "samples.jsonl"), "a") as log:
        n = 0
        while time.time() < t_end and not stop.is_set():
            if n % 40 == 0:
                vos = vo_paths()                     # a pipeline that starts after the launch brings its vo node with it
            row = tick(vos)
            log.write(json.dumps(row, separators=(",", ":")) + "\n")
            if n % 20 == 0:
                log.flush()
            n += 1
            time.sleep(max(0.0, period - (time.time() - row["t"])))
    stop.set()
    for t in threads:
        if isinstance(t, Stream):
            t.end()
    for t in threads:
        t.join(5)
    with open(os.path.join(out, "kernel.txt"), "w") as f:
        try:
            dmesg = subprocess.run(["dmesg"], capture_output=True, text=True, errors="replace", timeout=20).stdout
        except (OSError, subprocess.TimeoutExpired):
            dmesg = ""
        f.write("--- dmesg\n" + "\n".join(kernel_lines(dmesg)) + "\n")
        f.write("--- /var/log/messages since the start\n" + "\n".join(kernel_lines(read("/var/log/messages")[log0:])) + "\n")
    with open(os.path.join(out, "done.txt"), "w") as f:
        f.write("%d\n" % n)
    return n


# ---- the report (pure over a run directory) ----------------------------------------------------------------------------------

def load_jsonl(path):
    out = []
    for line in read(path).splitlines():
        try:
            out.append(json.loads(line))
        except ValueError:
            pass
    return out


def rates(samples, name=DE_IRQ, window=RATE_WINDOW_S):
    """The interrupt's rate in Hz at each sample, over the last `window` seconds of samples: [(up, hz)]."""
    out = []
    for i, s in enumerate(samples):
        c1, u1 = s["irq"].get(name), s["up"]
        j = i
        while j > 0 and u1 - samples[j - 1]["up"] < window:
            j -= 1
        if j == i or c1 is None or samples[j]["irq"].get(name) is None:
            out.append((u1, None))
            continue
        dt = u1 - samples[j]["up"]
        out.append((u1, round((c1 - samples[j]["irq"][name]) / dt, 1) if dt > 0 else None))
    return out


def rate_band(hz):
    """The output rate family a measured DE vsync rate belongs to."""
    if hz is None:
        return None
    for band in (120, 100, 96, 60, 50, 48):
        if abs(hz - band) <= 3.5:
            return band
    return None


def switches(samples, name=OSD_IRQ, short_window=0.25):
    """Where the DE's output rate band changes: [(up, from_band, to_band)]. The band is read over the 1 s window (robust); the
    moment is the last sample whose `short_window` rate was still in the old band, so the change lies within the next quarter
    second. None bands (a window still filling, or the crossing itself) are skipped."""
    long_, short = rates(samples, name), rates(samples, name, short_window)
    out, last = [], None
    for i, (up, hz) in enumerate(long_):
        band = rate_band(hz)
        if band is None:
            continue
        if last is not None and band != last:
            at = up
            for j in range(i, -1, -1):
                if rate_band(short[j][1]) == last:
                    at = short[j][0]
                    break
            out.append((at, last, band))
        last = band
    return out


def vsync_gaps(samples, name=OSD_IRQ, max_gap_s=0.03):
    """Spans where the DE vsync count did not advance between samples for at least `max_gap_s` (no refresh at all): [(up_from, up_to)]."""
    out, start, end = [], None, None
    for a, b in zip(samples, samples[1:]):
        ca, cb = a["irq"].get(name), b["irq"].get(name)
        if ca is None or cb is None:
            continue
        if cb == ca:
            start = a["up"] if start is None else start
            end = b["up"]
        else:
            if start is not None and end - start >= max_gap_s:
                out.append((start, end))
            start = None
    if start is not None and end - start >= max_gap_s:
        out.append((start, end))
    return out


def mute_windows(samples):
    """Spans where the DE firmware's internal mute (win0 localmute) was not 0: [(up_from, up_to, reason)]."""
    out, open_ = [], None
    for s in samples:
        v = s.get("win0", {}).get("localmute")
        if v not in (None, 0, "0"):
            if open_ is None:
                open_ = [s["up"], s["up"], v]
            else:
                open_[1] = s["up"]
        elif open_ is not None:
            out.append(tuple(open_))
            open_ = None
    if open_ is not None:
        out.append(tuple(open_))
    return out


def source_rates(samples):
    """The source rate the DE saw, as it changed: [(up, vFreq_tenths)]."""
    out, last = [], None
    for s in samples:
        v = s.get("win0", {}).get("vFreq")
        if v != last:
            out.append((s["up"], v))
            last = v
    return out


def decoder_rates(samples):
    """The decoder's own vsync rate and fixed flag as they changed: [(up, hz, fixed)] from the first port with a rate."""
    out, last = [], None
    for s in samples:
        cur = None
        for port in s.get("vsync", []):
            fr = port.get("framerate")
            if fr and fr[0]:
                fixed = next((v.get("fixed_frame_rate") for v in port.get("vsyncs", []) if v.get("fixed_frame_rate") is not None), None)
                cur = (round(fr[2], 2), fixed)
                break
        if cur != last:
            out.append((s["up"], cur[0] if cur else None, cur[1] if cur else None))
            last = cur
    return out


def videooutput_events(rows):
    """The subscription's updates as (up, [{sink, frameRate, muted, connected, appId}])."""
    out = []
    for r in rows:
        for obj in json_objects(r.get("line", "")):
            video = obj.get("video")
            if not isinstance(video, list):
                continue
            out.append((r["up"], [{k: v.get(k) for k in ("sink", "frameRate", "muted", "connected", "appId", "contentType")} for v in video]))
    return out


def app_mute_windows(rows):
    """From ls-monitor: videooutput's own video mute spans, [(up_on, up_off)], from setAvMute calls (muteType videoMute)."""
    out, on_at = [], None
    for r in rows:
        for obj in json_objects(r.get("line", "")):
            if obj.get("method") != "setAvMute" or obj.get("type") != "call" or obj.get("transport") not in (None, "TX"):
                continue
            p = obj.get("payload") or {}
            if p.get("muteType") != "videoMute":
                continue
            at = obj.get("timeStart", r["up"])
            if p.get("muteOn"):
                on_at = at if on_at is None else on_at
            elif on_at is not None:
                out.append((on_at, at))
                on_at = None
    if on_at is not None:
        out.append((on_at, None))
    return out


def shot_luma(path):
    """Mean luma 0-255 of a PNG, or None without PIL."""
    try:
        from PIL import Image, ImageStat
        return ImageStat.Stat(Image.open(path).convert("L")).mean[0]
    except Exception:
        return None


def shot_spans(shots, run_dir=None, protected=False):
    """Black and frozen spans from the screenshot log: {"black": [(up_from, up_to)], "frozen": [...], "with_luma": n}."""
    black, frozen, n_luma = [], [], 0
    prev_img, prev_up = None, None
    cur_black, cur_frozen = None, None
    for s in shots:
        up, is_black, same = s["up"], (s.get("bytes") or 0) < BLANK_BYTES, False
        if run_dir:
            path = os.path.join(run_dir, "shot-%04d.png" % s["n"])
            try:
                from PIL import Image, ImageChops, ImageStat
                img = Image.open(path).convert("L").resize((64, 36))
                n_luma += 1
                is_black = is_black or ImageStat.Stat(img).mean[0] < BLANK_LUMA
                if prev_img is not None:
                    same = ImageStat.Stat(ImageChops.difference(img, prev_img)).mean[0] < FROZEN_DIFF
                prev_img = img
            except Exception:
                pass
        if is_black:
            cur_black = [cur_black[0], up] if cur_black else [up, up]
        elif cur_black:
            black.append(tuple(cur_black))
            cur_black = None
        if same and not is_black:
            cur_frozen = [cur_frozen[0], up] if cur_frozen else [prev_up, up]
        elif cur_frozen:
            frozen.append(tuple(cur_frozen))
            cur_frozen = None
        prev_up = up
    if cur_black:
        black.append(tuple(cur_black))
    if cur_frozen:
        frozen.append(tuple(cur_frozen))
    return {"black": black, "frozen": frozen, "with_luma": n_luma, "protected": protected}


def inside(up, spans, slack=0.1):
    return any(a - slack <= up <= (b if b is not None else 1e12) + slack for a, b in spans)


def verdict(samples, shots, vo_rows, mon_rows, marks, run_dir=None, protected=False):
    """What the run says, as a dict for summary.txt: the rates, the switches, the mutes and gaps around them, and the words."""
    de = rates(samples)
    osd = rates(samples, OSD_IRQ)
    sw = switches(samples)
    gaps = vsync_gaps(samples)
    fw_mute = mute_windows(samples)
    app_mute = app_mute_windows(mon_rows)
    spans = shot_spans(shots, run_dir, protected)
    first_up = samples[0]["up"] if samples else 0.0
    rel = lambda u: None if u is None else round(u - first_up, 2)
    out = {"samples": len(samples), "seconds": rel(samples[-1]["up"]) if samples else 0,
           "marks": [(rel(u), n) for u, n in marks],
           "de_hz_first": de[min(len(de) - 1, 25)][1] if de else None, "de_hz_last": de[-1][1] if de else None,
           "osd_hz_last": osd[-1][1] if osd else None,
           "switches": [(rel(u), a, b) for u, a, b in sw],
           "vsync_gaps": [(rel(a), rel(b)) for a, b in gaps],
           "firmware_mute": [(rel(a), rel(b), r) for a, b, r in fw_mute],
           "app_video_mute": [(rel(a), rel(b)) for a, b in app_mute],
           "source_rate": [(rel(u), v) for u, v in source_rates(samples)],
           "decoder_rate": [(rel(u), hz, fixed) for u, hz, fixed in decoder_rates(samples)],
           "shots": len(shots), "black": [(rel(a), rel(b)) for a, b in spans["black"]], "frozen": [(rel(a), rel(b)) for a, b in spans["frozen"]],
           "videooutput": [(rel(u), v) for u, v in videooutput_events(vo_rows)][:40]}
    hfr = any(any(v.get("fixed_frame_rate") for v in p.get("vsyncs", [])) for s in samples for p in s.get("vsync", []))
    out["decoder_hfr_fixed"] = hfr
    out["de_hfr"] = any("hfr" in (vo.get("flags") or []) for s in samples for vo in s.get("vo", []))
    words = []
    if not sw:
        words.append("no output-rate switch: the panel stayed in the %s Hz band for the whole run" % (rate_band(out["osd_hz_last"]) or "?"))
    for u, a, b in out["switches"]:
        hidden = inside(u + first_up, app_mute)
        muted = inside(u + first_up, [(x, y) for x, y, _ in fw_mute], 0.3)
        gap = inside(u + first_up, gaps, 0.3)
        black = inside(u + first_up, spans["black"], 0.3) if not protected else None
        words.append("switch %s -> %s Hz at %.2f s: %s; firmware mute %s; vsync gap %s; screen %s" % (
            a, b, u, "under the app's own video mute" if hidden else "with the app's video unmuted",
            "yes" if muted else "none seen", "yes" if gap else "none",
            "protected (capture black by design)" if protected else "black" if black else "not black"))
    clean = all(not inside(u + first_up, [(x, y) for x, y, _ in fw_mute], 0.3) and not inside(u + first_up, gaps, 0.3) for u, _, _ in out["switches"])
    if not samples:
        clean = False
        words.append("no samples: the sampler recorded nothing, so the run proves nothing")
    elif not shots and not protected:
        clean = False
        words.append("no screen shots: blanking cannot be judged")
    else:
        played = any(v for _, v in out["source_rate"]) or bool(out["decoder_rate"])
        last = samples[-1]["up"]
        muted_throughout = any(a <= first_up + 0.5 and (b is None or b >= last - 0.5) for a, b, _ in fw_mute)
        if not played:
            clean = False
            words.append("no video: the DE saw no source rate and no decoder vsync, so nothing played")
        elif muted_throughout:
            clean = False
            words.append("the firmware mute never cleared: no picture was shown")
    out["evidence"] = not any(w.startswith(("no samples", "no screen shots", "no video", "the firmware mute never"))
                              for w in words)
    out["clean"] = clean
    out["words"] = words
    return out


def report(run_dir, protected=None):
    samples = load_jsonl(os.path.join(run_dir, "samples.jsonl"))
    shots = load_jsonl(os.path.join(run_dir, "shots.jsonl"))
    vo_rows = load_jsonl(os.path.join(run_dir, "videooutput.jsonl"))
    mon_rows = load_jsonl(os.path.join(run_dir, "monitor.jsonl"))
    marks = parse_marks(read(os.path.join(run_dir, "marks.txt")))
    if protected is None:
        meta = {}
        try:
            meta = json.loads(read(os.path.join(run_dir, "run.json")) or "{}")
        except ValueError:
            pass
        protected = bool(meta.get("protected"))
    v = verdict(samples, shots, vo_rows, mon_rows, marks, run_dir, protected)
    lines = ["frame-rate probe: %s" % os.path.basename(run_dir.rstrip("/")),
             "samples %d over %s s; DE vsync %s Hz at the start, %s Hz at the end; OSD vsync %s Hz at the end" % (
                 v["samples"], v["seconds"], v["de_hz_first"], v["de_hz_last"], v["osd_hz_last"]),
             "marks: " + ", ".join("%s at %s s" % (n, u) for u, n in v["marks"]),
             "source rate seen by the DE (0.1 Hz): " + ", ".join("%s at %s s" % (r, u) for u, r in v["source_rate"][:12]),
             "decoder vsync (hz, fixed): " + ", ".join("%s/%s at %s s" % (hz, f, u) for u, hz, f in v["decoder_rate"][:12]),
             "decoder HFR fixed: %s; vo hfr flag: %s" % (v["decoder_hfr_fixed"], v["de_hfr"]),
             "switches: %s" % (v["switches"] or "none"),
             "firmware mute (localmute) spans: %s" % (v["firmware_mute"] or "none"),
             "app video mute spans (setAvMute): %s" % (v["app_video_mute"] or "none seen"),
             "vsync gaps over 30 ms: %s" % (v["vsync_gaps"] or "none"),
             "screen: %d shots; black %s; frozen %s%s" % (v["shots"], v["black"] or "none", v["frozen"] or "none",
                                                          " (protected video: black by design)" if protected else "")]
    lines += v["words"]
    lines.append("VERDICT: %s" % ("PASS: every switch clean (no firmware mute, no vsync gap)" if v["clean"]
                                  else "FAIL: no evidence" if not v["evidence"]
                                  else "FAIL: a switch muted or gapped"))
    text = "\n".join(lines) + "\n"
    with open(os.path.join(run_dir, "summary.txt"), "w") as f:
        f.write(text)
    with open(os.path.join(run_dir, "verdict.json"), "w") as f:
        json.dump(v, f, indent=1)
    return text


def shots_to_keep(shots, moments, every=20, around=1.5):
    """Which shot numbers stay in the repository: every `every`th, and all within `around` seconds of a moment (uptime)."""
    keep = set()
    for i, s in enumerate(shots):
        if i % every == 0 or any(abs(s["up"] - m) <= around for m in moments):
            keep.add(s["n"])
    return keep


def prune_shots(run_dir):
    """Delete the screenshots that are not worth keeping (the verdict names the moments); the log keeps every row."""
    shots = load_jsonl(os.path.join(run_dir, "shots.jsonl"))
    samples = load_jsonl(os.path.join(run_dir, "samples.jsonl"))
    first_up = samples[0]["up"] if samples else 0.0
    try:
        v = json.loads(read(os.path.join(run_dir, "verdict.json")) or "{}")
    except ValueError:
        v = {}
    moments = [first_up + u for u, _, _ in v.get("switches", [])] + [first_up + a for a, _ in v.get("black", [])]
    moments += [first_up + a for a, _, _ in v.get("firmware_mute", [])] + [first_up + u for u, _ in v.get("marks", []) if u is not None]
    keep = shots_to_keep(shots, moments)
    removed = 0
    for s in shots:
        if s["n"] not in keep:
            try:
                os.unlink(os.path.join(run_dir, "shot-%04d.png" % s["n"]))
                removed += 1
            except OSError:
                pass
    return removed


# ---- the slot (workstation) ---------------------------------------------------------------------------------------------

def total_seconds(run):
    secs = 3 + run["settle"] + run.get("after_keys", 0) + run["secs"]
    if run.get("second"):
        secs += run["second"].get("after_keys", 0) + run["second"]["secs"]
    if run.get("then_home"):
        secs += run["then_home"]
    return secs + 3


def plan(run, launch=None, keys=None):
    steps = [("preflight", "read-only apart from waking the set: it answers and is Active, Home is in front, Picture Mode, failed units, "
                           "ovi/de status and the boot line are recorded"),
             ("upload", "probe.py and kb.py to /tmp/fr-probe-<stamp>/"),
             ("mute", "the receiver muted through Home Assistant and read back"),
             ("sample", "the sampler starts on the set for %d s (50 ms ticks, screenshots, videooutput subscription, bus monitor)" % total_seconds(run)),
             ("launch", "3 s of Home, then %s; the mark `launch`" % json.dumps(launch or run["launch"])),
             ("keys", "after %d s: %s; the mark `keys`" % (run["settle"], keys or run["keys"] or "no keys"))]
    if run.get("second"):
        steps.append(("second", "after %d s of playback: %s; the mark `second`" % (run["secs"], run["second"]["keys"])))
    if run.get("then_home"):
        steps.append(("home", "after playback: Home for %d s with the sampler running; the mark `home`" % run["then_home"]))
    steps += [("close", "the app closed, Home in front and seen (getForegroundAppInfo)"),
              ("unmute", "the receiver put back as it was, after Home is back"),
              ("fetch", "the run directory copied to docs/research/framerate/<run>-<stamp>/ and summary.txt written"),
              ("standby", "the set left in Active Standby (--no-standby leaves it on)")]
    return steps


def run_slot(a):
    run = dict(RUNS[a.run])
    if a.launch:
        run["launch"] = json.loads(a.launch)
    if a.keys is not None:
        run["keys"] = a.keys
    if a.second_keys is not None and run.get("second"):
        run["second"] = dict(run["second"], keys=a.second_keys)
    if a.secs:
        run["secs"] = a.secs
    sys.path[:0] = [os.path.join(REPO, "compositor", "tools"), os.path.join(REPO, "docs", "research", "arch")]
    import tv
    import tv_gate
    stamp = time.strftime("%Y%m%d-%H%M")
    remote = "/tmp/fr-probe-" + stamp
    out = a.out or os.path.join(HERE, "%s-%s" % (a.run, stamp))
    os.makedirs(out, exist_ok=True)
    problems, muted_before, started = [], None, False

    def sh(cmd, timeout=60):
        return tv.tv(cmd, timeout)

    def luna(uri, payload):
        return tv_gate.pty("luna-send -n 1 %s %s" % (uri, shlex.quote(json.dumps(payload))), 30)

    def mark(name):
        sh("echo \"$(cut -d' ' -f1 /proc/uptime) %s\" >> %s/marks.txt" % (name, remote))
        print("%s mark %s" % (time.strftime("%H:%M:%S"), name), flush=True)

    def foreground():
        text = tv_gate.pty("luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}'", 20)
        m = re.search(r'"appId"\s*:\s*"([^"]*)"', text)
        return m.group(1) if m else ""

    def ha(path, body=None):
        import urllib.request
        req = urllib.request.Request(HA + path, data=json.dumps(body).encode() if body is not None else None,
                                     headers={"Authorization": "Bearer " + tv_gate.ha_token(), "Content-Type": "application/json"})
        with urllib.request.urlopen(req, timeout=15) as r:
            return json.loads(r.read() or b"null")

    def receiver():
        st = ha("/api/states/" + RECEIVER)
        return st.get("state"), st.get("attributes", {}).get("is_volume_muted")

    try:
        if not tv_gate.wake():
            raise SystemExit("the set does not answer")
        picture_before = tv_gate.pty("luna-send -n 1 luna://com.webos.settingsservice/getSystemSettings '{\"category\":\"picture\",\"keys\":[\"pictureMode\",\"realCinema\",\"truMotionMode\"]}'", 20)
        fg = foreground()
        if fg != "com.webos.app.home":
            luna("luna://com.webos.applicationManager/launch", {"id": "com.webos.app.home"})
            time.sleep(3)
        with open(os.path.join(out, "run.json"), "w") as f:
            json.dump({"run": a.run, "launch": run["launch"], "keys": run["keys"], "secs": run["secs"], "protected": bool(run.get("protected")),
                       "picture_before": picture_before.strip(), "foreground_before": fg}, f, indent=1)
        sh("mkdir -p %s" % remote)
        tv.put(os.path.abspath(__file__), remote + "/probe.py")
        tv.put(os.path.join(REPO, "docs", "research", "perf", "kb.py"), remote + "/kb.py")
        if not a.no_mute:
            state, muted_before = receiver()
            ha("/api/services/media_player/volume_mute", {"entity_id": RECEIVER, "is_volume_muted": True})
            time.sleep(1.5)
            if receiver()[1] is not True:
                raise SystemExit("the receiver is not shown muted: nothing is played")
            print("receiver muted (was %s, muted %s)" % (state, muted_before))
        total = total_seconds(run)
        sh("nohup python3 %s/probe.py --sample --out %s --secs %d > %s/sampler.out 2>&1 < /dev/null &" % (remote, remote, total, remote))
        started = True
        time.sleep(3)
        mark("launch")
        luna("luna://com.webos.applicationManager/launch", run["launch"])
        time.sleep(run["settle"])
        if run["keys"]:
            mark("keys")
            sh("python3 %s/kb.py %s" % (remote, run["keys"]), 120)
        time.sleep(run.get("after_keys", 0) + run["secs"])
        if run.get("second"):
            mark("second")
            sh("python3 %s/kb.py %s" % (remote, run["second"]["keys"]), 120)
            time.sleep(run["second"].get("after_keys", 0) + run["second"]["secs"])
        if run.get("then_home"):
            mark("home")
            luna("luna://com.webos.applicationManager/launch", {"id": "com.webos.app.home"})
            time.sleep(run["then_home"])
        mark("end")
        for _ in range(20):
            if "yes" in sh("test -e %s/done.txt && echo yes || echo no" % remote):
                break
            time.sleep(1)
        else:
            sh("kill -TERM $(pgrep -f 'probe.py --sample') 2>/dev/null; sleep 2; true")
    except Exception as e:
        problems.append("the run stopped early: %s" % e)
    finally:
        try:
            luna("luna://com.webos.applicationManager/closeByAppId", {"id": run["launch"]["id"]})
            luna("luna://com.webos.applicationManager/launch", {"id": "com.webos.app.home"})
            time.sleep(3)
            home = foreground() == "com.webos.app.home"
            if not home:
                problems.append("Home is not in front at the end (foreground %s)" % foreground())
            if muted_before is not None:
                if home:
                    ha("/api/services/media_player/volume_mute", {"entity_id": RECEIVER, "is_volume_muted": bool(muted_before)})
                    print("receiver put back (muted %s)" % muted_before)
                else:
                    problems.append("the receiver stays muted: Home was not confirmed in front")
            if started:
                sh("cd %s && tar czf run.tgz *.jsonl *.txt *.png *.out 2>/dev/null; true" % remote, 120)
                tv.get(remote + "/run.tgz", os.path.join(out, "run.tgz"))
                subprocess.run(["tar", "xzf", "run.tgz"], cwd=out, check=False)
                print(report(out, bool(run.get("protected"))))
                print("shots pruned: %d removed" % prune_shots(out))
                try:
                    os.unlink(os.path.join(out, "run.tgz"))
                except OSError:
                    pass
            picture_after = tv_gate.pty("luna-send -n 1 luna://com.webos.settingsservice/getSystemSettings '{\"category\":\"picture\",\"keys\":[\"pictureMode\"]}'", 20)
            with open(os.path.join(out, "after.txt"), "w") as f:
                f.write(picture_after)
            if not a.no_standby:
                tv_gate.pty("luna-send -n 1 -f luna://com.webos.service.tvpower/power/powerOff '{\"reason\":\"remoteKey\"}'", 20)
        except Exception as e:
            problems.append("the end of the run failed: %s" % e)
    for p in problems:
        print("PROBLEM:", p, file=sys.stderr)
    return 1 if problems else 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--plan", action="store_true")
    ap.add_argument("--go", action="store_true")
    ap.add_argument("--sample", action="store_true", help="on the set")
    ap.add_argument("--report", metavar="DIR")
    ap.add_argument("--run", default="youtube50", choices=sorted(RUNS))
    ap.add_argument("--launch", help="the launch payload (JSON) in place of the run's")
    ap.add_argument("--keys", help="kb.py tokens after the settle, in place of the run's")
    ap.add_argument("--second-keys")
    ap.add_argument("--secs", type=int, help="playback seconds, in place of the run's")
    ap.add_argument("--out")
    ap.add_argument("--no-mute", action="store_true")
    ap.add_argument("--no-standby", action="store_true")
    ap.add_argument("--no-shots", action="store_true", help="sampler: no screenshots")
    ap.add_argument("--no-streams", action="store_true", help="sampler: no videooutput subscription, no bus monitor")
    ap.add_argument("--protected", action="store_true", help="report: treat the video as protected (capture black by design)")
    ap.add_argument("--allow-fr-rate-write", action="store_true", help="refused here: the fr_rate experiment is a separate slot")
    a = ap.parse_args(argv)
    if a.allow_fr_rate_write:
        raise SystemExit("--allow-fr-rate-write is not part of this probe (framerate-result.md, Steer points)")
    if a.sample:
        return 0 if sample(a.out or "/tmp/fr-probe", a.secs or 60, shots=not a.no_shots, streams=not a.no_streams) else 1
    if a.report:
        print(report(a.report, True if a.protected else None), end="")
        return 0
    if a.go:
        return run_slot(a)
    run = RUNS[a.run]
    print("run %s: %s" % (a.run, run["what"]))
    for step, why in plan(run, a.launch, a.keys):
        print("  %-9s %s" % (step, why))
    return 0


if __name__ == "__main__":
    sys.exit(main())
