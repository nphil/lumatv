#!/usr/bin/env python3
"""TV check of the camera streams' re-declared rate (docs/research/vwin-refresh-result.md, "TV check"). Runs on the set as root; stdlib only.

A camera stream whose parameter set says 25 fps took the panel to 100 Hz and left it there. The helper now writes 30 (60 for 50) over the rate in the
parameter sets it hands the feeder (`helper/app/src/live/rate.rs`), so the decoder reports a rate the panel keeps 120 Hz for. This reads whether it did.

    tv-camera-rate-probe.py read                                       the set as the other stages need it
    tv-camera-rate-probe.py picture --clip30 FILE                      the picture mode before, during and after one clip through NDL, the ES player and the ES player without low delay
    tv-camera-rate-probe.py clips --clip25 FILE --clip30 FILE          the decoder's rate and the panel's, from two files that differ only in the rate they say
    tv-camera-rate-probe.py rates --native30 FILE --native60 FILE      H.264 made at 30 and at 60 pictures a second and fed at that pace: the rule this relies on, about 30 s
    tv-camera-rate-probe.py camera [--downs N] [--secs S]              a real camera's full view and Back, from Home (the camera N rows down the Cameras grid)
    tv-camera-rate-probe.py ring [--downs N] [--secs S]                the Ring door camera on its battery: the full view's time to the first picture, and the panel's rate while it plays (S defaults to 60)
    tv-camera-rate-probe.py all --clip25 FILE --clip30 FILE [--native30 FILE --native60 FILE] [--downs N] [--secs S]
    tv-camera-rate-probe.py selftest                                   the readers on text (runs on the host)

The clips are made on the workstation, 640x360, one second of black at 25 pictures a second; the second is the first with the rate its parameter set says
changed by ffmpeg's bitstream filter, which writes the same bytes the helper writes (`live::rate` tests). Both are fed at 25 a second:
    ffmpeg -f lavfi -i color=c=black:size=640x360:rate=25 -t 1 -c:v libx264 -profile:v high -pix_fmt yuv420p -preset veryslow \\
      -x264-params aud=1:repeat-headers=1:slices=1:bframes=0:keyint=25:scenecut=0:ref=1:force-cfr=1 -bsf:v filter_units=remove_types=6 -f h264 clip25.h264
    ffmpeg -i clip25.h264 -c copy -bsf:v h264_metadata=tick_rate=60/1 -f h264 clip30.h264
    scp clip25.h264 clip30.h264 root@TV:/tmp/gfn/

`rates` plays one second of black made at 30 and at 60 pictures a second (what a server sending 30 or 60 would send, and what the sentence "H.264 at 30 or 60 keeps the
panel at 120 Hz" claims; unconfirmed until this runs): the same recipe with the rate and the key interval changed, fed at that rate:
    ffmpeg -f lavfi -i color=c=black:size=640x360:rate=30 -t 1 -c:v libx264 -profile:v high -pix_fmt yuv420p -preset veryslow \\
      -x264-params aud=1:repeat-headers=1:slices=1:bframes=0:keyint=30:scenecut=0:ref=1:force-cfr=1 -bsf:v filter_units=remove_types=6 -f h264 native30.h264
    the same with rate=60 and keyint=60 into native60.h264
    scp native30.h264 native60.h264 root@TV:/tmp/gfn/

What it reads each half second: the rate of the OSD interrupt (`osd_irq`, the panel's: 120 a second at 120 Hz), the kernel's reading of window 0 (`/proc/lg/de/win0/status`
`vFreq`, tenths of a Hz: 250 and 300; `lowdelaymode`, the display's low delay: 1 for NDL's pipelines), the scaler's source (`connect_src`) and the decoder's rate and where it
came from (`/proc/lxvo/vdisp/vrate`, STATUS: `rate_src CODEC`, `framerate 30/1`). For `camera`, the helper's lines in the RAM log (`/tmp/lumatv-logs/current.log`): the stream it
dialled, the line that says the rate was re-declared, and how many pictures it fed. Evidence goes to /tmp/lumatv/out/camera-rate/ (samples.jsonl, result.json, the helper's lines);
fetch the directory.

The picture mode: NDL's own load tells the TV the video is a game (`videoInfo.isGameMode` and `lowDelayMode` true), and the TV takes its game picture mode while the video plays (`game`,
dimension `sdrALLM`), which shifts the colours; the ES player's load, the one a camera's view uses (`gfn/feeder/src/gf_pipe.h`), leaves the game out. `picture` plays the 30 clip
through NDL (the control: expected to go to game), the ES player, and the ES player with `--low-delay 0`, which says which of the two flags the TV takes the mode for. `camera` reads
it over the real view. Both read `getSystemSettings` of the picture category about ten times a second from a thread of their own, and the load each pipeline got from the system
log (`smp.api LOAD` of the feeder). Pass: the mode read before the video is every mode read while it plays, the load says no game, and the display's low delay stays on. `picture`
also sets the render queue each pipeline held (the feeder's stats: the pictures that wait there are the latency the pipeline adds, 40 ms each at 25 a second) against NDL's.

What it never does: write /proc/lg/de/fr_rate, change a setting, launch an app other than by the keys a hand would press, or leave a claim or a player behind.
A verdict that needs eyes says so (`null`): the picture is smooth and shows no lag against the real scene, nothing flashes at the start or at Back.
"""
import argparse
import json
import os
import re
import signal
import struct
import subprocess
import sys
import threading
import time

OUT = "/tmp/lumatv/out/camera-rate"
CTL = ("/var/lib/lumatv/compositor/lib/ld-linux-aarch64.so.1 --library-path /var/lib/lumatv/compositor/lib "
       "/var/lib/lumatv/compositor/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg ")
FEEDER = "/var/lib/lumatv/bin/lumatv-gfn-feeder"
SCALER = "/proc/lgtv-driver/scaler/main.p0"
WIN0 = "/proc/lg/de/win0/status"
VRATE = "/proc/lxvo/vdisp/vrate"
RAMLOG = "/tmp/lumatv-logs/current.log"
MESSAGES = "/var/log/messages"
PICTURE = ("luna-send -n 1 -f luna://com.webos.service.settingsservice/getSystemSettings "
           "'{\"category\":\"picture\",\"keys\":[\"pictureMode\"]}' 2>&1")
GAME = ("game", "hdrGame", "dolbyHdrGame")
HOME = "com.webos.app.home"
APP = "org.nphil.lumatv.rateprobe"
KEYS = {"UP": 103, "DOWN": 108, "LEFT": 105, "RIGHT": 106, "OK": 28, "BACK": 412}
T0 = time.monotonic()


def say(*a):
    print("camera-rate: %6.1f" % (time.monotonic() - T0), *a, flush=True)


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
    """120 or 100 when the panel's rate is within 3.5 Hz of it, else None."""
    for b in (120, 100):
        if hz is not None and abs(hz - b) <= 3.5:
            return b
    return None


def scaler_of(text):
    """connect_src and the source and output sizes of the main window's status."""
    kv = dict(l.strip().split("=", 1) for l in text.splitlines() if "=" in l)
    return {"src": kv.get("connect_src"), "source": (kv.get("source_w"), kv.get("source_h"))}


def win0_of(text):
    """The timing window 0's source has, as the kernel read it: size and rate in 0.1 Hz (300 is 30 fps), and the display's low delay (`lowdelaymode`, 1 or 0)."""
    m = re.search(r"h/v active\s*:\s*(\d+)\s*/\s*(\d+)", text)
    f = re.search(r"vFreq\s*:\s*(\d+)", text)
    d = re.search(r"lowdelaymode\s*=\s*(\d+)", text)
    return {"active": (int(m.group(1)), int(m.group(2))) if m else None, "vfreq": int(f.group(1)) if f else None, "lowdelay": int(d.group(1)) if d else None}


def fps_of(word):
    """`25/1(25.00)` -> 25.0; None for no rate (`0/4294967295(0.00)`)."""
    m = re.search(r"\(([\d.]+)\)", word or "")
    v = float(m.group(1)) if m else None
    return v if v else None


def vrate_of(text):
    """The decoder's rate from `/proc/lxvo/vdisp/vrate` (the last VRATE block, which is the main window's while one video plays): the rate it settled on and where
    from (`rate_src`: CODEC is the stream's own, CONF the container's, PTS the pictures' times), and each source's reading."""
    blocks = re.split(r"^===\s+VRATE\[\d+\]\s+===", text, flags=re.M)[1:]
    if not blocks:
        return None
    sections, name = {}, None
    for line in blocks[-1].splitlines():
        h = re.match(r"^- (CONFIG|CODEC|PTS|FIXED|STATUS) -", line.strip())
        if h:
            name = h.group(1)
            sections[name] = {}
            continue
        if name and ":" in line and not line.strip().startswith("--"):
            k, v = [x.strip() for x in line.split(":", 1)]
            sections[name][k] = v          # a CODEC or CONFIG block has the original then the current: the current is the later one
    st = sections.get("STATUS", {})
    return {"rate_src": st.get("rate_src"), "status_fps": fps_of(st.get("framerate")), "codec_fps": fps_of(sections.get("CODEC", {}).get("framerate")),
            "config_fps": fps_of(sections.get("CONFIG", {}).get("framerate")), "pts_fps": fps_of(sections.get("PTS", {}).get("framerate")),
            "use_fixed": sections.get("FIXED", {}).get("use_fixed")}


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


def helper_lines(text):
    """What the helper's camera lines say, from a stretch of the RAM log: the stream it dialled, the re-declaration, and how the stream ended."""
    out = {"stream": None, "redeclared": None, "ended": None}
    for line in text.splitlines():
        if "lumatv-helper:" not in line:
            continue
        m = re.search(r"cameras: live(?: for [^:]*?)? on camera (\d+): (\w*) ?(\d+)x(\d+) at (\d+) a second declared", line)
        if m and not out["stream"]:
            out["stream"] = {"camera": m.group(1), "codec": m.group(2) or None, "size": (int(m.group(3)), int(m.group(4))), "declared_fps": int(m.group(5))}
        m = re.search(r"cameras: live(?: for [^:]*?)? on camera (\d+): the stream's parameter set says ([\d.]+) pictures a second.*the decoder is told (\d+)", line)
        if m and not out["redeclared"]:
            out["redeclared"] = {"camera": m.group(1), "was": float(m.group(2)), "now": int(m.group(3))}
        m = re.search(r"cameras: live(?: for [^:]*?)? on camera (\d+) ended: first picture after (\d+) ms, (\d+) pictures in ([\d.]+) s = ([\d.]+) a second, (\d+) refused, (\d+) lost", line)
        if m:
            out["ended"] = {"camera": m.group(1), "first_ms": int(m.group(2)), "pictures": int(m.group(3)), "per_second": float(m.group(5)), "refused": int(m.group(6)), "lost": int(m.group(7))}
    return out


def first_picture_after(samples, opened):
    """Seconds from `opened` (the probe's own monotonic clock) to the first reading with a source on the main window; None when there was none."""
    for s in video(samples):
        return round(s["t"] - (opened - T0), 1)
    return None


def in_band(samples, b, minimum=1):
    """Whether every reading of the panel in `samples` is in the band `b` (None when there are fewer than `minimum` readings)."""
    bands = [band(s["hz"]) for s in samples if s["hz"] is not None]
    return all(x == b for x in bands) if len(bands) >= minimum else None


def commonest(xs):
    """The value that comes most often in `xs` (the first of them on a tie), None for none: the set's python has no `statistics`."""
    counts = {}
    for x in xs:
        counts[x] = counts.get(x, 0) + 1
    best = None
    for x, n in counts.items():
        if best is None or n > counts[best]:
            best = x
    return best


def picture_of(text):
    """The picture mode and dynamic range from LG's settings service (`getSystemSettings`, picture category): `{"mode": "game", "range": "sdrALLM"}` while a video says it is
    a game and `{"mode": "filmMaker", "range": "sdr"}` otherwise on the owner's settings; None for no answer."""
    d = first_json(text)
    mode = (d.get("settings") or {}).get("pictureMode") or (d.get("dimension") or {}).get("pictureMode")
    return {"mode": mode, "range": (d.get("dimension") or {}).get("dynamicRange")} if mode and d.get("returnValue", True) else None


def feeder_loads(text):
    """The loads LG's media pipeline got from the feeder, from a stretch of the system log (`smp.api LOAD`, which the pipeline prints whatever library made the call): the
    transport, the app, `low_delay` (`lowDelayMode`) and `game` (`videoInfo.isGameMode`, None when the load has none)."""
    out = []
    for line in text.splitlines():
        if "lumatv-gfn-feeder" not in line or "smp.api LOAD" not in line:
            continue
        try:
            msg = json.loads(json.JSONDecoder().raw_decode(line[line.find("{", line.find("smp.api LOAD")):])[0]["msg"])
            arg = msg["args"][0]
        except (ValueError, KeyError, IndexError, TypeError):
            continue                    # the pipeline's other LOAD lines say `appId[...]` and `load completed`
        option = arg.get("option") or {}
        out.append({"transport": arg.get("mediaTransportType"), "app": option.get("appId"), "low_delay": option.get("lowDelayMode"),
                    "game": (option.get("videoInfo") or {}).get("isGameMode")})
    return out


def picture_verdict(before, during, after, minimum=8):
    """What the picture mode did, from readings `{"mode", "range"}`: the mode before the video, the modes while it played and in the last three readings after it, whether
    a game mode was seen (the mode, or a dynamic range of ALLM), and whether the mode held: every reading while it played was the one before (None with too few to say)."""
    base = commonest([s["mode"] for s in before])
    modes = lambda xs: sorted({s["mode"] for s in xs})
    settled = after[-3:]
    return {"before": base, "during": modes(during), "after": modes(settled), "readings_during": len(during),
            "game_seen": any(s["mode"] in GAME or (s["range"] or "").endswith("ALLM") for s in during) if during else None,
            "kept": all(s["mode"] == base for s in during) if base and len(during) >= minimum else None,
            "back_after": all(s["mode"] == base for s in settled) if base and len(settled) == 3 else None}


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


def read(path, n=65536):
    try:
        with open(path, errors="replace") as f:
            return f.read(n)
    except OSError:
        return ""


def size_of(path):
    try:
        return os.path.getsize(path)
    except OSError:
        return None


def read_since(path, at, limit=8 << 20):
    """What a log has gained since it was `at` bytes long (all of it, up to `limit`, when it has been rotated to a shorter file or had no size to start from)."""
    try:
        with open(path, "rb") as f:
            f.seek(at if at is not None and at <= os.path.getsize(path) else 0)
            return f.read(limit).decode(errors="replace")
    except OSError:
        return ""


class PictureMode:
    """Reads the picture mode from a thread of its own, one luna-send after another (about ten a second): `{"t", "mode", "range"}`, `t` on this program's monotonic clock.
    A reading that got no answer is left out. It only reads."""

    def __init__(self):
        self.samples = []
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._run, daemon=True)

    def _run(self):
        while not self._stop.is_set():
            t = time.monotonic()
            p = picture_of(sh(PICTURE, 8))
            if p:
                self.samples.append({"t": t, **p})

    def start(self):
        self._thread.start()
        return self

    def stop(self):
        self._stop.set()
        self._thread.join(15)
        return self.samples

    def between(self, a, b):
        return [s for s in list(self.samples) if a <= s["t"] <= b]


def foreground():
    r = first_json(sh("luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' 2>&1", 20))
    e = r.get("foregroundAppInfo")
    if isinstance(e, list):
        e = next((x for x in e if x.get("primary")), e[0] if e else {})
    return (e or r).get("appId")


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


def end_player(app):
    """Ends the feeder of one app id (not `pkill -f`: the shell that runs it carries the pattern)."""
    for pid, argv in feeders():
        if is_for(argv, app):
            try:
                os.kill(pid, signal.SIGTERM)
            except OSError:
                pass


def key_device():
    blocks = read("/proc/bus/input/devices").split("\n\n")
    for name in ("Smart Remote RCU Input", "LGE M-RCU - Builtin [0]"):
        for block in blocks:
            if 'Name="%s"' % name in block:
                for word in block.split():
                    if word.startswith("event"):
                        return "/dev/input/" + word
    return None


def press(name, device):
    fd = os.open(device, os.O_WRONLY)
    try:
        for kind, code, value in ((1, KEYS[name], 1), (0, 0, 0), (1, KEYS[name], 0), (0, 0, 0)):
            os.write(fd, struct.pack("<llHHi", 0, 0, kind, code, value))
            time.sleep(0.03)
    finally:
        os.close(fd)


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


def sample(meter, tag, log):
    s = {"t": round(time.monotonic() - T0, 2), "up": uptime(), "tag": tag, "hz": meter.look(), "scaler": scaler_of(read(SCALER)), "win0": win0_of(read(WIN0)),
         "vrate": vrate_of(read(VRATE))}
    log.write(json.dumps(s) + "\n")
    log.flush()
    return s


def watch(meter, tag, log, seconds, every=0.5):
    out, end = [], time.monotonic() + seconds
    while time.monotonic() < end:
        time.sleep(every)
        out.append(sample(meter, tag, log))
    return out


def video(samples):
    """The samples with a source on the main window."""
    return [s for s in samples if s["scaler"]["src"] not in (None, "NONE")]


def decoder_reading(samples):
    """What the decoder and the kernel said while a video played: the commonest `vFreq`, STATUS rate and its source, and the display's low delay."""
    v = video(samples)
    pick = commonest
    return {"samples": len(v), "vfreq": pick([s["win0"]["vfreq"] for s in v if s["win0"]["vfreq"]]),
            "lowdelay": pick([s["win0"]["lowdelay"] for s in v if s["win0"]["lowdelay"] is not None]),
            "status_fps": pick([s["vrate"]["status_fps"] for s in v if s["vrate"] and s["vrate"]["status_fps"]]),
            "rate_src": pick([s["vrate"]["rate_src"] for s in v if s["vrate"] and s["vrate"]["rate_src"]]),
            "codec_fps": pick([s["vrate"]["codec_fps"] for s in v if s["vrate"] and s["vrate"]["codec_fps"]]),
            "pts_fps": pick([s["vrate"]["pts_fps"] for s in v if s["vrate"] and s["vrate"]["pts_fps"]])}


# ---- the stages -----------------------------------------------------------------------------------------------------------------------------------

def stage_read(res):
    m = Meter()
    m.look()
    time.sleep(2.0)
    hz = m.look()
    vwin = ctl("vwin").get("vwin", {})
    log_bytes = os.path.getsize(RAMLOG) if os.path.exists(RAMLOG) else None
    st = {"foreground": foreground(), "panel_hz": hz, "panel_band": band(hz), "refresh": ctl("refresh").get("mode"), "scaler": scaler_of(read(SCALER)),
          "vrate_readable": os.path.exists(VRATE), "vwin_mode": vwin.get("mode"), "claims": vwin.get("claims"), "feeder_present": os.path.exists(FEEDER),
          "feeder_plays_files": os.path.exists(FEEDER) and "--play" in sh("%s --codec none 2>&1" % FEEDER, 10), "ramlog_bytes": log_bytes, "key_device": key_device(),
          "feeders_running": [pid for pid, _ in feeders()]}
    st["ready_for_clips"] = bool(st["foreground"] == HOME and st["panel_band"] == 120 and st["scaler"]["src"] == "NONE" and st["feeder_plays_files"] and st["vrate_readable"]
                                 and st["vwin_mode"] in ("idle", "apps") and not st["claims"])
    st["ready_for_camera"] = bool(st["foreground"] == HOME and st["panel_band"] == 120 and st["scaler"]["src"] == "NONE" and st["ramlog_bytes"] is not None and st["key_device"])
    res["read"] = st
    say("read:", json.dumps(st))
    return st


def play_clip(meter, log, name, path, seconds=7.0, after=6.0, fps=25, pipe="es", extra=""):
    """One clip, fed at `fps` pictures a second through the pipeline `pipe` (the ES player, what a camera's view uses; `ndl` for the control) as a full-screen claim (black, as
    the Ring's view is): the samples while it plays and after it is released, and the monotonic times of the claim and of the release."""
    env = "XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0"
    sh("%s %s --play %s --size 640x360 --codec h264 --fps %d --seconds 12 --loop -w video -a %s --ndl-app %s --pipe %s %s --stats %s/%s-stats.jsonl > %s/%s.log 2>&1 &"
       % (env, FEEDER, path, fps, APP, HOME, pipe, extra, OUT, name, OUT, name))
    time.sleep(1.5)
    meter.look()
    claimed = time.monotonic()
    state = ctl("vwin claim probe camera-full 0 0 1920 1080 %s h264 640x360@%d" % (APP, fps)).get("claim", {}).get("state")
    say("%s: claim probe:" % name, json.dumps(state))
    during = watch(meter, name, log, seconds)
    released = time.monotonic()
    ctl("vwin release probe")
    end_player(APP)
    post = watch(meter, name + "-after", log, after)
    return {"pipe": (pipe + " " + extra).strip(), "claim": state, "during": during, "after": post, "decoder": decoder_reading(during), "panel_after": [band(s["hz"]) for s in post],
            "t": [claimed, released]}


def stage_clips(res, clip25, clip30, pre):
    if not (clip25 and clip30 and os.path.exists(clip25) and os.path.exists(clip30)):
        res["clips"] = {"error": "--clip25 FILE and --clip30 FILE are needed (see the header)"}
        say("clips: no clips")
        return
    if not pre["ready_for_clips"]:
        res["clips"] = {"error": "start from 120 Hz, Home in front, no source, no claim and vwin idle or apps (see read)", "read": pre}
        say("clips: the set is not ready")
        return
    meter = Meter()
    with open(OUT + "/samples.jsonl", "a") as log:
        fix = play_clip(meter, log, "clip30", clip30)                     # the rate the helper writes, at 25 pictures a second, from 120 Hz
        control = play_clip(meter, log, "clip25", clip25)                 # what a camera did: 25, and the panel goes to 100
        back = play_clip(meter, log, "clip30-again", clip30, 6.0, 5.0)    # and a 30 brings it back, as any video at 30 does
    leftover = [pid for pid, argv in feeders() if is_for(argv, APP)]
    d = lambda r: r["decoder"]
    res["clips"] = {
        "fix": {k: v for k, v in fix.items() if k not in ("during", "after")}, "control": {k: v for k, v in control.items() if k not in ("during", "after")},
        "again": {k: v for k, v in back.items() if k not in ("during", "after")},
        "verdict": {
            "fix_decoder_reads_30": d(fix)["vfreq"] in range(295, 306) and d(fix)["status_fps"] == 30.0 if d(fix)["samples"] else None,
            "fix_rate_from_the_stream": d(fix)["rate_src"] == "CODEC" if d(fix)["samples"] else None,
            "fix_panel_stays_120": in_band(fix["during"] + fix["after"], 120, 8),
            "control_decoder_reads_25": d(control)["vfreq"] in range(245, 256) and d(control)["status_fps"] == 25.0 if d(control)["samples"] else None,
            "control_panel_goes_100": sum(1 for s in control["during"] if band(s["hz"]) == 100) >= 3,
            "control_holds_100_after": all(b == 100 for b in control["panel_after"][-3:]) if len(control["panel_after"]) >= 3 else None,
            "again_panel_back_120": all(b == 120 for b in back["panel_after"][-3:]) if len(back["panel_after"]) >= 3 else None,
            "cleaned_up": not leftover and scaler_of(read(SCALER))["src"] == "NONE",
            "no_flash_seen": None,
            "eyes_note": "needs eyes: the screen is black while a clip plays; nothing but black shows at the start, at the end, and when the panel changes rate",
        },
    }
    say("clips verdict:", json.dumps(res["clips"]["verdict"]))


def rates_verdict(run, fps):
    """What a clip that says `fps` pictures a second did: the decoder's reading of it and whether the panel stayed at 120 Hz while it played and after it ended."""
    d, after = run["decoder"], run["panel_after"][-3:]
    return {
        "decoder_reads_%d" % fps: (abs(d["vfreq"] - fps * 10) <= 5 and d["status_fps"] == float(fps)) if d["samples"] and d["vfreq"] else None,
        "rate_from_the_stream": d["rate_src"] == "CODEC" if d["samples"] else None,
        "panel_120_during": in_band(run["during"], 120, 3),
        "panel_120_after": all(b == 120 for b in after) if len(after) == 3 else None,
    }


def stage_rates(res, native30, native60, pre):
    if not (native30 and native60 and os.path.exists(native30) and os.path.exists(native60)):
        res["rates"] = {"error": "--native30 FILE and --native60 FILE are needed (see the header)"}
        say("rates: no clips")
        return
    if not pre["ready_for_clips"]:
        res["rates"] = {"error": "start from 120 Hz, Home in front, no source, no claim and vwin idle or apps (see read)", "read": pre}
        say("rates: the set is not ready")
        return
    meter = Meter()
    runs = {}
    with open(OUT + "/samples.jsonl", "a") as log:
        for fps, path in ((30, native30), (60, native60)):
            runs[fps] = play_clip(meter, log, "native%d" % fps, path, 6.0, 6.0, fps)
    leftover = [pid for pid, argv in feeders() if is_for(argv, APP)]
    verdict = {"fps%d" % fps: rates_verdict(run, fps) for fps, run in runs.items()}
    verdict["held_120"] = all(v is True for per in list(verdict.values()) for v in per.values())
    verdict["cleaned_up"] = not leftover and scaler_of(read(SCALER))["src"] == "NONE"
    verdict["no_flash_seen"] = None
    verdict["eyes_note"] = "needs eyes: the screen is black while a clip plays; nothing but black shows at the start, at the end, and when the panel changes rate"
    res["rates"] = {"runs": {str(fps): {k: v for k, v in run.items() if k not in ("during", "after")} for fps, run in runs.items()}, "verdict": verdict}
    say("rates verdict:", json.dumps(verdict))


def queue_of(text):
    """The render queue the pipeline held while a clip played, from the feeder's per-second stats (`--stats`): the mean depth in pictures over the seconds that were read, the
    deepest p99, and the readings behind them (`None` when the pipeline reports no queue). The pictures that wait there are the latency the pipeline adds: 40 ms each at 25 a second."""
    means, deepest, readings = [], 0, 0
    for line in text.splitlines():
        try:
            queue = json.loads(line).get("queue")
        except (ValueError, AttributeError):
            continue
        if isinstance(queue, dict) and queue.get("samples"):
            means.append(queue["mean"])
            deepest = max(deepest, queue["p99"])
            readings += queue["samples"]
    return {"mean": round(sum(means) / len(means), 2) if means else None, "p99": deepest if means else None, "readings": readings}


def picture_run(pq, run, loads, queue, since=0.0):
    """One clip's picture-mode readings: before the feeder started (after `since`), while the claim held the video and just after its release; the loads the pipeline got and
    the render queue it held."""
    claimed, released = run["t"]
    return {"pipe": run["pipe"], "loads": loads, "queue": queue, "low_delay": run["decoder"]["lowdelay"], "decoder_samples": run["decoder"]["samples"],
            "picture": picture_verdict(pq.between(max(claimed - 5.0, since), claimed - 1.7), pq.between(claimed, released), pq.between(released + 1.0, released + 4.0))}


def picture_stage_verdict(runs):
    """What the three clips say. The control, NDL's own load, is expected to declare a game and take the game mode (when it does not, the rest proves nothing); the ES player
    is the fix if its load says no game and the mode holds. `low_delay_alone_takes_game` says which flag the TV takes the mode for when the ES player's does not hold: True when the
    mode holds only without `lowDelayMode`, False when it holds with it (the game flag was the one) or when it fails either way (neither is)."""
    ndl, es, plain = runs["ndl"], runs["es"], runs["es-no-low-delay"]
    alone = None
    if es["picture"]["kept"] is True:
        alone = False
    elif es["picture"]["kept"] is False and plain["picture"]["kept"] is not None:
        alone = plain["picture"]["kept"]
    return {
        "control_ndl_declares_game": any(l["game"] is True for l in ndl["loads"]) if ndl["loads"] else None,
        "control_ndl_goes_game": ndl["picture"]["game_seen"],
        "es_declares_no_game": all(l["game"] is not True for l in es["loads"]) if es["loads"] else None,
        "es_keeps_picture_mode": es["picture"]["kept"],
        "es_back_after": es["picture"]["back_after"],
        "es_display_low_delay": es["low_delay"] == 1 if es["decoder_samples"] else None,
        "es_queue_pictures": es["queue"]["mean"],
        "ndl_queue_pictures": ndl["queue"]["mean"],
        "es_queue_no_deeper_than_ndl": (es["queue"]["mean"] <= ndl["queue"]["mean"] + 1.0) if es["queue"]["mean"] is not None and ndl["queue"]["mean"] is not None else None,
        "es_without_low_delay_keeps_picture_mode": plain["picture"]["kept"],
        "low_delay_alone_takes_game": alone,
    }


def stage_picture(res, clip30, pre):
    if not (clip30 and os.path.exists(clip30)):
        res["picture"] = {"error": "--clip30 FILE is needed (see the header)"}
        say("picture: no clip")
        return
    if not pre["ready_for_clips"]:
        res["picture"] = {"error": "start from 120 Hz, Home in front, no source, no claim and vwin idle or apps (see read)", "read": pre}
        say("picture: the set is not ready")
        return
    meter = Meter()
    pq = PictureMode().start()
    runs, since = {}, 0.0
    try:
        time.sleep(5.0)                                             # the mode before any video
        with open(OUT + "/samples.jsonl", "a") as log:
            for key, pipe, extra in (("ndl", "ndl", ""), ("es", "es", ""), ("es-no-low-delay", "es", "--low-delay 0")):
                at = size_of(MESSAGES)
                run = play_clip(meter, log, "picture-" + key, clip30, 6.0, 4.0, 25, pipe, extra)
                runs[key] = picture_run(pq, run, feeder_loads(read_since(MESSAGES, at)), queue_of(read("%s/picture-%s-stats.jsonl" % (OUT, key), 1 << 20)), since)
                since = run["t"][1] + 1.0                           # what the next run takes as "before" starts when this one's mode has gone back
    finally:
        pq.stop()
    leftover = [pid for pid, argv in feeders() if is_for(argv, APP)]
    verdict = picture_stage_verdict(runs)
    verdict["cleaned_up"] = not leftover and scaler_of(read(SCALER))["src"] == "NONE"
    verdict["eyes_note"] = "needs eyes: the screen is black while a clip plays, so the colours cannot be judged here; the readings are the mode the TV reports"
    res["picture"] = {"runs": runs, "readings": len(pq.samples), "verdict": verdict}
    say("picture verdict:", json.dumps(verdict))


def stage_camera(res, downs, secs, pre, ring=False):
    """`ring`: the camera is on a battery. The view waits up to 45 s for its first picture (Ring joins a call 1 to 20 s after it starts, and only about half of them), so the
    time from OK to the first picture is the result, and the panel's rate is read over the seconds the picture plays."""
    key = "ring" if ring else "camera"
    dev = pre["key_device"]
    if not pre["ready_for_camera"]:
        res[key] = {"error": "start from 120 Hz, Home in front, no source, the RAM log and the remote's key device there (see read)", "read": pre}
        say(key + ": the set is not ready")
        return
    log_at = os.path.getsize(RAMLOG)
    messages_at = size_of(MESSAGES)
    pq = PictureMode().start()
    try:
        ctl("note camera-rate probe: camera full view")
        shown = ctl("shell show cameras")
        time.sleep(2.5)
        for _ in range(downs):
            press("DOWN", dev)
            time.sleep(0.4)
        time.sleep(2.0)
        meter = Meter()
        meter.look()
        with open(OUT + "/samples.jsonl", "a") as log:
            opened = time.monotonic()
            press("OK", dev)
            view = watch(meter, "camera", log, secs)
            closed = time.monotonic()
            press("BACK", dev)
            after = watch(meter, "camera-after", log, 9.0)
    finally:
        pq.stop()
    time.sleep(1.0)
    picture = picture_verdict(pq.between(0.0, opened - 0.3), pq.between(opened, closed), pq.between(closed + 1.0, closed + 9.0))
    loads = feeder_loads(read_since(MESSAGES, messages_at))
    with open(RAMLOG, errors="replace") as f:
        f.seek(log_at if os.path.getsize(RAMLOG) >= log_at else 0)
        text = f.read()
    lines = [l for l in text.splitlines() if "lumatv-helper:" in l and "cameras:" in l]
    with open(OUT + "/helper-lines.txt", "w") as f:
        f.write("\n".join(lines) + "\n")
    h = helper_lines(text)
    dec = decoder_reading(view)
    settled = [s for s in video(view) if s["t"] - view[0]["t"] >= 2.0]
    in_family = bool(h["redeclared"]) or dec["codec_fps"] in (25.0, 50.0)
    first = first_picture_after(view, opened)
    res[key] = {
        "shown": shown, "helper": h, "decoder": dec, "panel_during": [band(s["hz"]) for s in video(view)], "panel_after": [band(s["hz"]) for s in after],
        "picture": picture, "loads": loads,
        "verdict": {
            "stream_seen": h["stream"] is not None,
            "first_picture_s": first,
            "first_picture_within_45s": first is not None and first <= 45.0,
            "helper_first_picture_ms": h["ended"]["first_ms"] if h["ended"] else None,
            "in_family": in_family if h["stream"] else None,
            "redeclared": bool(h["redeclared"]),
            "decoder_reads_what_was_declared": (dec["status_fps"] == float(h["redeclared"]["now"]) and dec["rate_src"] == "CODEC") if h["redeclared"] and dec["samples"] else None,
            "panel_120_during": in_band(settled, 120, 4),
            "panel_120_after": all(band(s["hz"]) == 120 for s in after[-6:] if s["hz"] is not None) if after else None,
            "pictures_flow": (h["ended"]["pictures"] > 0 and h["ended"]["refused"] == 0) if h["ended"] else None,
            "pictures_a_second": h["ended"]["per_second"] if h["ended"] else None,
            "load_declares_no_game": all(l["game"] is not True for l in loads) if loads else None,
            "load_is_the_es_player": all(l["transport"] == "DIRECTMEDIA-ES-PLAYER" for l in loads) if loads else None,
            "picture_mode_kept": picture["kept"],
            "picture_mode_back_after": picture["back_after"],
            "display_low_delay": dec["lowdelay"] == 1 if dec["samples"] else None,
            "note": None if in_family or not h["stream"] else "this camera's stream is not 25 or 50 fps: nothing to re-declare, nothing proven about the rate (the picture mode's readings stand); choose another with --downs",
            "smooth_and_no_lag_seen": None,
            "eyes_note": "needs eyes: the picture moves smoothly with no stall at the repeats, shows no more lag against the real scene than before, and nothing flashes at the start or at Back",
        },
    }
    say(key + " verdict:", json.dumps(res[key]["verdict"]))


def finish(res):
    ctl("vwin release probe")
    end_player(APP)
    time.sleep(1.0)
    res["end"] = {"players": [pid for pid, argv in feeders() if is_for(argv, APP)], "foreground": foreground(), "scaler": scaler_of(read(SCALER)),
                  "claims": ctl("vwin").get("vwin", {}).get("claims"), "refresh": ctl("refresh").get("mode")}
    say("end:", json.dumps(res["end"]))


def selftest():
    assert osd_count(" 68:     240000      12       0       3     GICv2 104 Edge      osd_irq\n 9: 5 GICv2 Edge other\n") == 240015
    assert osd_count("nothing") is None
    assert (band(120.4), band(99.1), band(110), band(None), band(123.6)) == (120, 100, None, None, None)
    assert scaler_of("[main.p0]\nopen=1\nconnect_src=VDEC\nsource_w=640\nsource_h=360\n") == {"src": "VDEC", "source": ("640", "360")}
    assert win0_of("localmute=777\nlowdelaymode=1\nh/v active : 640 / 360\nvFreq      : 300\n") == {"active": (640, 360), "vfreq": 300, "lowdelay": 1}
    assert win0_of("h/v active : 0 / 0\nvFreq      : 0\n") == {"active": (0, 0), "vfreq": 0, "lowdelay": None}
    assert (fps_of("25/1(25.00)"), fps_of("0/4294967295(0.00)"), fps_of("3240000/54000(60.00)"), fps_of(None)) == (25.0, None, 60.0, None)
    # the camera pop-up's capture of round 22 (`framerate-r22/vr-A.txt`, 47334.72): the stream says 25, the pictures' times read 60, and the decoder follows the stream
    block = """== /proc/lxvo/vdisp/vrate
===  VRATE[2] ===
speed              : 1000
framerate_max      : 120

- CONFIG -
    -- original --
    framerate          : 0/4294967295(0.00)
    duration           : 0
    -- current --
    framerate          : 0/4294967295(0.00)
    duration           : 0

- CODEC -
    -- original --
    framerate          : 25/1(25.00)
    duration           : 3600
    -- current --
    framerate          : 25/1(25.00)
    duration           : 3600

- PTS -
    prev_stc           : 0
    buf_cnt            : 36
    framerate          : 3240000/54000(60.00)
    duration           : 1500
    stable             : 1

- FIXED -
    use_fixed          : 0
    framerate          : 0/0(0.00)

- STATUS -
    rate_src           : CODEC
    framerate          : 25/1(25.00)
    duration           : 3600


#ENDOFREAD_0027
"""
    assert vrate_of(block) == {"rate_src": "CODEC", "status_fps": 25.0, "codec_fps": 25.0, "config_fps": None, "pts_fps": 60.0, "use_fixed": "0"}, vrate_of(block)
    assert vrate_of("uid : 5\n  display_period : 1\n#ENDOFREAD_0002\n") is None
    # a second instance (a sub window) comes last: the reading is of the last block, as `decoder_reading` takes the samples of the main window's source
    two = block + block.replace("VRATE[2]", "VRATE[3]").replace("25/1(25.00)", "30/1(30.00)")
    assert vrate_of(two)["status_fps"] == 30.0
    log = "\n".join([
        "<14>lumatv-helper: 1793849859201 cameras: live for full view on camera 240: h264 1920x1080 at 25 a second declared, prebuffered, quality native, window video, decoder opened for com.webos.app.home, the bridge answered in 3517 ms",
        "<14>lumatv-helper: 1793849860300 cameras: live for full view on camera 240: the stream's parameter set says 25 pictures a second, which would take the panel to 100 Hz; the decoder is told 30",
        "<14>lumatv-compositor: [srv 1.0] info something else",
        "<14>lumatv-helper: 1793849905859 cameras: live for full view on camera 240 ended: first picture after 816 ms, 35 pictures in 1.7 s = 20.7 a second, 0 refused, 0 lost",
    ])
    h = helper_lines(log)
    assert h["stream"] == {"camera": "240", "codec": "h264", "size": (1920, 1080), "declared_fps": 25}, h
    assert h["redeclared"] == {"camera": "240", "was": 25.0, "now": 30}, h
    assert h["ended"] == {"camera": "240", "first_ms": 816, "pictures": 35, "per_second": 20.7, "refused": 0, "lost": 0}, h
    assert helper_lines("<14>lumatv-helper: 1 cameras: live for pop-up 3 on camera 9:  2560x1920 at 0 a second declared, x")["stream"]["codec"] is None
    assert helper_lines("nothing") == {"stream": None, "redeclared": None, "ended": None}
    assert is_for(["/v/lumatv-gfn-feeder", "--play", "c.h264", "-a", "org.a"], "org.a") and not is_for(["f", "-a", "org.ab"], "org.a") and not is_for(["f"], "org.a")
    s = lambda hz: {"hz": hz}
    assert (in_band([s(120.1), s(119.7), s(None), s(120.5)], 120, 3), in_band([s(120.1), s(99.7)], 120), in_band([s(120.1)], 120, 2)) == (True, False, None)
    assert decoder_reading([])["samples"] == 0
    # a clip that says 30 or 60, as `rates` reads it: the kernel's tenths of a Hz, the decoder's STATUS rate and where it came from, and the panel before, during and after
    seen = lambda vfreq, fps, during, after: {"decoder": {"samples": 5, "vfreq": vfreq, "status_fps": fps, "rate_src": "CODEC"}, "during": [{"hz": h} for h in during],
                                              "panel_after": [band(h) for h in after]}
    assert rates_verdict(seen(300, 30.0, [120.2] * 4, [120.1] * 4), 30) == {"decoder_reads_30": True, "rate_from_the_stream": True, "panel_120_during": True, "panel_120_after": True}
    assert rates_verdict(seen(600, 60.0, [120.2] * 4, [100.1] * 4), 60)["panel_120_after"] is False
    assert rates_verdict(seen(250, 25.0, [100.1] * 4, [100.0] * 4), 60)["decoder_reads_60"] is False
    assert rates_verdict({"decoder": decoder_reading([]), "during": [], "panel_after": []}, 30) == dict.fromkeys(
        ["decoder_reads_30", "rate_from_the_stream", "panel_120_during", "panel_120_after"])
    assert (commonest([1, 2, 2, 3]), commonest([3, 1]), commonest([])) == (2, 3, None)
    # the picture mode as LG's settings service answers while a video says it is a game (TV 2026-11-05, the camera's full view: `game`, `sdrALLM`; 2026-10-26: `hdrGame`) and while none does
    game = '{"subscribed":false,"category":"picture","method":"getSystemSettings","dimension":{"pictureMode":"game","dynamicRange":"sdrALLM","input":"default"},"settings":{"pictureMode":"game"},"returnValue":true}'
    film = '{"subscribed":false,"category":"picture","method":"getSystemSettings","dimension":{"pictureMode":"filmMaker","dynamicRange":"sdr","input":"default"},"settings":{"pictureMode":"filmMaker"},"returnValue":true}'
    hdr = '{"subscribed":false,"category":"picture","method":"getSystemSettings","dimension":{"dynamicRange":"hdrALLM","input":"default"},"settings":{"pictureMode":"hdrGame"},"returnValue":true}'
    assert (picture_of(game), picture_of(film), picture_of(hdr)) == ({"mode": "game", "range": "sdrALLM"}, {"mode": "filmMaker", "range": "sdr"}, {"mode": "hdrGame", "range": "hdrALLM"})
    assert picture_of("") is None and picture_of('{"returnValue":false,"errorText":"no"}') is None
    reading = lambda mode, rng="sdr": {"mode": mode, "range": rng}
    f, g = reading("filmMaker"), reading("game", "sdrALLM")
    held = picture_verdict([f] * 5, [f] * 20, [f] * 4)
    assert held == {"before": "filmMaker", "during": ["filmMaker"], "after": ["filmMaker"], "readings_during": 20, "game_seen": False, "kept": True, "back_after": True}, held
    flipped = picture_verdict([f] * 5, [f] * 3 + [g] * 17, [g, f, f, f])
    assert (flipped["kept"], flipped["game_seen"], flipped["during"], flipped["back_after"]) == (False, True, ["filmMaker", "game"], True), flipped
    assert picture_verdict([f] * 5, [f] * 3, [f] * 3)["kept"] is None and picture_verdict([], [f] * 20, [f] * 3)["kept"] is None       # too few readings, or nothing to compare with
    assert picture_verdict([f] * 5, [reading("filmMaker", "sdrALLM")] * 20, [])["game_seen"] is True                                   # a dynamic range of ALLM is a game whatever the mode is called
    # what the pipeline's log says of the loads (TV 2026-10-30, `smp.api LOAD`): NDL's own, with `lowDelayMode` and `videoInfo.isGameMode` true, and the ES player's, which leaves the game out
    ndl_load = ('{"args":[{"mediaTransportType":"DIRECTMEDIA-ES-PLAYER","option":{"appId":"org.nphil.lumatv.cameras","lowDelayMode":true,'
                '"externalStreamingInfo":{"contents":{"codec":{"video":"H264"},"esInfo":{"videoHeight":1080,"videoWidth":1920,"pauseAtDecodeTime":true,"ptsToDecode":0}}},'
                '"adaptiveStreaming":{"maxHeight":1080,"maxFrameRate":60,"maxWidth":1920},"windowId":"_Window_Id_17","videoInfo":{"isGameMode":true}}}]}')
    es_load = ndl_load.replace(',"videoInfo":{"isGameMode":true}', "")
    said = lambda who, tag, msg: '2026-10-30T05:23:49.976109Z [30830.833777820] user.info %s [] smp.api %s {"uid":"_fsw5qY8gjdVUGR", "tid": "9631", "msg":%s} ' % (who, tag, json.dumps(msg))
    log = "\n".join([said("lumatv-gfn-feeder", "LOAD", ndl_load), said("lumatv-gfn-feeder", "LOAD", "appId[org.nphil.lumatv.cameras]"), said("lumatv-gfn-feeder", "LOAD", "load completed"),
                     said("lumatv-gfn-feeder", "SVU_SMP_LOAD", ndl_load), said("cobalt", "LOAD", ndl_load), said("lumatv-gfn-feeder", "LOAD", es_load), "not a log line"])
    expected = {"transport": "DIRECTMEDIA-ES-PLAYER", "app": "org.nphil.lumatv.cameras", "low_delay": True, "game": True}
    assert feeder_loads(log) == [expected, dict(expected, game=None)], feeder_loads(log)
    # a log that has been rotated is read from its start; one that has grown, from where it was
    import tempfile
    with tempfile.NamedTemporaryFile("w") as lg:
        lg.write("0123456789")
        lg.flush()
        assert (read_since(lg.name, 4), read_since(lg.name, 10), read_since(lg.name, 99), read_since(lg.name, None), read_since("/nonexistent/log", 0)) == (
            "456789", "", "0123456789", "0123456789", "")
    # the windows `picture_run` reads: the mode before the feeder started, while the claim held the video, after the release
    class Readings:
        def __init__(self, xs):
            self.xs = xs

        def between(self, a, b):
            return [s for s in self.xs if a <= s["t"] <= b]
    through = Readings([dict(reading("game", "sdrALLM") if 10.5 <= t / 10 <= 18.5 else f, t=t / 10) for t in range(0, 260)])
    ran = picture_run(through, {"pipe": "ndl", "decoder": {"lowdelay": 1, "samples": 6}, "t": [10.0, 18.0]}, [], {"mean": 1.4, "p99": 2, "readings": 150}, 0.0)
    assert (ran["picture"]["before"], ran["picture"]["kept"], ran["picture"]["game_seen"], ran["picture"]["back_after"], ran["low_delay"]) == ("filmMaker", False, True, True, 1), ran
    # the render queue, from the feeder's per-second stats of a clip (the NDL run of 2026-10-26, `1080p60-card.out`): the pictures that wait there are the latency the pipeline adds
    stats = "\n".join(['{"start":true,"file":"c.h264"}', '{"t":1,"fed":61,"queue":{"min":0,"max":3,"mean":1.86,"p99":3,"samples":61}}', '{"t":2,"fed":60,"queue":{"min":1,"max":2,"mean":1.50,"p99":2,"samples":60}}',
                       '{"t":3,"fed":0,"queue":{"min":0,"max":0,"mean":0.00,"p99":0,"samples":0}}', "not json", '{"summary":true,"units":121}'])
    assert queue_of(stats) == {"mean": 1.68, "p99": 3, "readings": 121}, queue_of(stats)
    assert queue_of("") == queue_of('{"t":1,"queue":{"min":0,"max":0,"mean":0.00,"p99":0,"samples":0}}') == {"mean": None, "p99": None, "readings": 0}
    # the three clips: NDL's own goes game (the control), the ES player holds the mode, and without low delay it holds too
    load = lambda game, low=True: {"transport": "DIRECTMEDIA-ES-PLAYER", "app": "a", "low_delay": low, "game": game}
    clip = lambda kept, seen, loads, mean=1.5: {"loads": loads, "queue": {"mean": mean, "p99": 3, "readings": 100}, "low_delay": 1, "decoder_samples": 6,
                                                "picture": {"kept": kept, "game_seen": seen, "back_after": True}}
    control = clip(False, True, [load(True)])
    fixed = picture_stage_verdict({"ndl": control, "es": clip(True, False, [load(None)]), "es-no-low-delay": clip(True, False, [load(None, False)])})
    assert fixed == {"control_ndl_declares_game": True, "control_ndl_goes_game": True, "es_declares_no_game": True, "es_keeps_picture_mode": True, "es_back_after": True,
                     "es_display_low_delay": True, "es_queue_pictures": 1.5, "ndl_queue_pictures": 1.5, "es_queue_no_deeper_than_ndl": True,
                     "es_without_low_delay_keeps_picture_mode": True, "low_delay_alone_takes_game": False}, fixed
    assert picture_stage_verdict({"ndl": control, "es": clip(True, False, [load(None)], 3.0), "es-no-low-delay": control})["es_queue_no_deeper_than_ndl"] is False
    assert picture_stage_verdict({"ndl": control, "es": clip(True, False, [load(None)], None), "es-no-low-delay": control})["es_queue_no_deeper_than_ndl"] is None
    assert picture_stage_verdict({"ndl": control, "es": clip(False, True, [load(None)]), "es-no-low-delay": clip(True, False, [load(None, False)])})["low_delay_alone_takes_game"] is True
    assert picture_stage_verdict({"ndl": control, "es": clip(False, True, [load(None)]), "es-no-low-delay": clip(False, True, [load(None, False)])})["low_delay_alone_takes_game"] is False
    assert picture_stage_verdict({"ndl": control, "es": clip(None, None, []), "es-no-low-delay": clip(None, None, [])})["low_delay_alone_takes_game"] is None
    # the time to the first picture: the first reading with a source on the main window, counted from the OK
    lit = lambda t, src: {"t": t, "scaler": {"src": src, "source": (None, None)}}
    assert first_picture_after([lit(11.0, "NONE"), lit(19.5, "NONE"), lit(21.5, "VDEC"), lit(22.0, "VDEC")], T0 + 10.0) == 11.5
    assert first_picture_after([lit(11.0, "NONE")], T0 + 10.0) is None and first_picture_after([], T0) is None
    print("selftest ok")


def main(argv):
    ap = argparse.ArgumentParser(add_help=True, description="TV check of the camera streams' re-declared rate")
    ap.add_argument("stage", choices=["read", "picture", "clips", "rates", "camera", "ring", "all", "selftest"])
    ap.add_argument("--clip25")
    ap.add_argument("--clip30")
    ap.add_argument("--native30")
    ap.add_argument("--native60")
    ap.add_argument("--downs", type=int, default=2, help="rows down the Cameras grid to the camera (the Ring door camera is two)")
    ap.add_argument("--secs", type=float, default=None, help="seconds of the camera's view that are sampled (16; 60 for ring, whose first picture may be 45 s away)")
    a = ap.parse_args(argv)
    if a.stage == "selftest":
        selftest()
        return 0
    os.makedirs(OUT, exist_ok=True)
    res = {"started": time.strftime("%Y-%m-%d %H:%M:%S")}
    try:
        pre = stage_read(res)
        if a.stage in ("picture", "all"):
            stage_picture(res, a.clip30, pre)
            if a.stage == "all":
                pre = stage_read({})
        if a.stage in ("clips", "all"):
            stage_clips(res, a.clip25, a.clip30, pre)
        if a.stage == "rates" or (a.stage == "all" and (a.native30 or a.native60)):
            if a.stage == "all":
                pre = stage_read({})
            stage_rates(res, a.native30, a.native60, pre)
        if a.stage in ("camera", "all") and (a.stage == "camera" or not res.get("clips", {}).get("error")):
            if a.stage == "all":
                pre = stage_read({})
            stage_camera(res, a.downs, a.secs or 16.0, pre)
        if a.stage == "ring":
            stage_camera(res, a.downs, a.secs or 60.0, pre, ring=True)
    finally:
        finish(res)
        with open(OUT + "/result.json", "w") as f:
            json.dump(res, f, indent=1, default=str)
        say("result:", OUT + "/result.json")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
