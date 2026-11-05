#!/usr/bin/env python3
"""Idle Home with the weather's sky moving, on the set: what the compositor and the UI cost over a window (docs/research/sky-cost-result.md).

Run ON THE SET as root with LumaTV's compositor live (`ctl` answers), Home in front, the screensaver off and nobody at the remote. Stdlib only. Nothing
is written but its own result file: no key, no setting, no draw.

    tv-idle-probe.py run [--secs 60] [--label NAME] [--out DIR]   reads, waits, reads; DIR/NAME.json (default /tmp/lumatv/out/idle/NAME.json) and a summary
    tv-idle-probe.py read                                         one reading, nothing waited for
    tv-idle-probe.py compare BEFORE.json AFTER.json               two results side by side
    tv-idle-probe.py selftest                                     the readers and the arithmetic on text (runs on the host)

What a window gives: the share of one core the compositor and the UI took (user and system apart, and by thread), the two together against the
target of `TARGET_PCT`, the loop's wake-ups and ticks a second, the flips a second and how many of them were frames at every refresh (`animating`),
the UI engine's CPU per frame and where the engine spent it, how the compositor's ticks were spent, the GPU's busy share, the panel's refresh and
what Home was showing (the weather's reading and the theme's sky switch). The expectations of the idle gate with the sky on are in `EXPECT`.
"""
import json
import os
import re
import subprocess
import sys
import time
import urllib.request

ROOT = os.environ.get("IDLE_PROBE_ROOT", "")
OUT = "/tmp/lumatv/out/idle"
CTL = ("/var/lib/lumatv/compositor/lib/ld-linux-aarch64.so.1 --library-path /var/lib/lumatv/compositor/lib "
       "/var/lib/lumatv/compositor/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg ")
THEME = "/var/lib/lumatv/theme/theme.json"
HELPER = "http://127.0.0.1:18731/v1/weather"
MALI = "/sys/kernel/debug/mali0/dvfs_utilization"
#: the compositor and the UI together, in percent of one core, with the sky moving (the owner's target, 2026-11-04)
TARGET_PCT = 8.0
#: what the idle gate expects of a Home at rest with the sky moving, by the sky's cadence (`weather_art::STEPS`): flips a second (the sky's frames, 15 for a calm
#: sky, 30 for snow and wind, 60 for rain, and the few a Home at rest draws for itself) and how many of them are frames at every refresh (an ambient motion asks
#: for none: 120 of 120 before it was paced; a storm's strikes, 4 % of the time, still do)
EXPECT = {"calm": {"flips_per_s": (12.0, 32.0), "animating_per_s": (0.0, 5.0)},
          "fast": {"flips_per_s": (26.0, 45.0), "animating_per_s": (0.0, 5.0)},
          "rain": {"flips_per_s": (55.0, 80.0), "animating_per_s": (0.0, 8.0)},
          "off": {"flips_per_s": (0.0, 15.0), "animating_per_s": (0.0, 5.0)}}
#: `weather_art::Cond::from_doc`: snow is drawn at 30 a second, and so is any sky with wind from `WINDY_MPH`; drizzle, rain and storm (the other codes from 51 to
#: 99) at 60; the rest at 15
SNOW_CODES = {71, 73, 75, 77, 85, 86}
WINDY_MPH = 18.0
CLK_TCK = 100.0
T0 = time.monotonic()


def say(*a):
    print("idle: %6.1f" % (time.monotonic() - T0), *a, flush=True)


# ---- readers (pure; `selftest` runs them) -----------------------------------------------------------------------------------------------------------

def read(path):
    try:
        with open(ROOT + path, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def stat_fields(text):
    """The fields of /proc/PID/stat after the command name (which may hold spaces and brackets): [0] is the state."""
    i = text.rfind(")")
    return text[i + 2:].split() if i >= 0 else []


def utime_stime(text):
    f = stat_fields(text)
    return (int(f[11]), int(f[12])) if len(f) > 12 else None


def ctxt(text):
    """(voluntary, involuntary) context switches of /proc/PID/task/TID/status."""
    v = re.search(r"^voluntary_ctxt_switches:\s*(\d+)", text, re.M)
    n = re.search(r"^nonvoluntary_ctxt_switches:\s*(\d+)", text, re.M)
    return (int(v.group(1)), int(n.group(1))) if v and n else (0, 0)


def parse_dvfs(text):
    m = {k: int(v) for k, v in re.findall(r"(busy_time|idle_time):\s*(\d+)", text)}
    return (m["busy_time"], m["idle_time"]) if len(m) == 2 else None


def interrupts(text, name):
    """The count of one interrupt over all CPUs: the numbers after the label, up to the controller's name."""
    for line in text.splitlines():
        f = line.split()
        if f and f[-1] == name:
            return sum(int(x) for x in f[1:] if x.isdigit())
    return None


def total_jiffies(text):
    return sum(int(x) for x in text.splitlines()[0].split()[1:]) if text else None


def summary_fields(summary):
    """The numbers of `ctl status` `summary` a window needs. `engine` is the UI engine's totals in us since the start, `None` when the compositor is an
    older build that does not print them."""
    def num(pattern, cast=int):
        m = re.search(pattern, summary)
        return cast(m.group(1)) if m else None
    out = {"flips": num(r"\bflips (\d+)"), "ticks": num(r"\bticks (\d+)"), "drawn": num(r"\bdrawn (\d+)"), "late": num(r"\blate (\d+)"),
           "ui_frames": num(r"ui frames (\d+)"), "ui_empty": num(r"ui frames \d+ \(empty (\d+)\)"),
           "px_frame": num(r"px/frame (\d+)"), "changed": num(r"changed (\d+)"), "draws_frame": num(r"draws/frame (\d+)"), "layers_frame": num(r"layers/frame (\d+)"),
           "cpu_mean_us": num(r"cpu (\d+) us/frame"), "refresh_ms": num(r"refresh ([0-9.]+) ms", float)}
    m = re.search(r"total us: cpu (\d+) place (\d+) plan (\d+) backdrops (\d+) draw (\d+) damage (\d+) apply (\d+)(?: scope (\d+) step (\d+) finish (\d+))?", summary)
    out["engine"] = dict(zip(("cpu", "place", "plan", "backdrops", "draw", "damage", "apply", "scope", "step", "finish"), map(int, (g for g in m.groups() if g is not None)))) if m else None
    if out["engine"] is None and out["ui_frames"] and out["cpu_mean_us"] is not None:
        out["engine_cpu_total_approx"] = out["ui_frames"] * out["cpu_mean_us"]
    return out


def sky_of(code, wind_mph):
    """The sky's cadence by the reading: `rain` draws at 60 a second, `fast` (snow, wind) at 30, `calm` at 15; `None` without a reading."""
    if code is None:
        return None
    if 51 <= code <= 99 and code not in SNOW_CODES:
        return "rain"
    return "fast" if code in SNOW_CODES or (wind_mph or 0) >= WINDY_MPH else "calm"


# ---- the set -------------------------------------------------------------------------------------------------------------------------------------------

def ctl(args):
    r = subprocess.run(CTL + args, shell=True, capture_output=True, text=True, timeout=20)
    try:
        return json.loads(r.stdout)
    except ValueError:
        return {}


def find_compositor():
    for d in os.listdir(ROOT + "/proc"):
        if d.isdigit():
            try:
                cmd = read("/proc/%s/cmdline" % d).split("\0")
            except OSError:
                continue
            if any(c.endswith("bin/lumatv-compositor") for c in cmd) and "ctl" not in cmd:
                return int(d)
    return None


def threads(pid):
    out = {}
    for t in os.listdir("%s/proc/%d/task" % (ROOT, pid)):
        us = utime_stime(read("/proc/%d/task/%s/stat" % (pid, t)))
        if us is None:
            continue
        name = read("/proc/%d/task/%s/comm" % (pid, t)).strip()
        out["%s/%s" % (name, t)] = {"user": us[0], "system": us[1], "ctxt": ctxt(read("/proc/%d/task/%s/status" % (pid, t)))}
    return out


def weather():
    """What the helper says Home's sky shows (the reading's code and whether it is day), and the theme's switches for the sky."""
    out = {"code": None, "day": None, "wind_mph": None, "theme_sky_animated": None, "theme_wallpaper_motion": None}
    try:
        doc = json.load(urllib.request.urlopen(HELPER, timeout=3))
        cur = (doc.get("doc") or doc).get("current") or {}
        out["code"], out["day"], out["wind_mph"] = cur.get("code"), cur.get("isDay"), cur.get("windMph")
    except Exception:
        pass
    try:
        theme = json.loads(read(THEME))
        out["theme_sky_animated"] = (theme.get("weather") or {}).get("animated", True)
        out["theme_wallpaper_motion"] = (theme.get("wallpaperMotion") or {}).get("enabled", True)
    except ValueError:
        pass
    return out


def snapshot():
    cp = find_compositor()
    sh = ctl("shell")
    up = sh.get("pid")
    st = ctl("status")
    rf = ctl("refresh")
    keys = ctl("keys")
    ui = sh.get("ui") or {}
    s = {"mono": time.monotonic(), "clk_tck": os.sysconf("SC_CLK_TCK"), "comp_pid": cp, "ui_pid": up,
         "stat_total": total_jiffies(read("/proc/stat")), "dvfs": parse_dvfs(read(MALI)),
         "vsync": interrupts(read("/proc/interrupts"), "DE2VSYNC"), "osd_irq": interrupts(read("/proc/interrupts"), "osd_irq"),
         "stats": st.get("stats"), "summary": st.get("summary", ""), "refresh": {k: rf.get(k) for k in ("hz", "mode", "tick_ms")},
         "measure": (rf.get("measure") or {}), "keys": {k: keys.get(k) for k in ("delivered", "dropped", "foreground")},
         "saver": ui.get("saver"), "home": {k: (ui.get("home") or {}).get(k) for k in ("profile",)}, "loadavg": read("/proc/loadavg").strip()}
    for name, pid in (("comp", cp), ("ui", up)):
        try:
            s[name + "_threads"] = threads(pid)
        except (OSError, TypeError):
            s[name + "_threads"] = None
    return s


# ---- the arithmetic ----------------------------------------------------------------------------------------------------------------------------------

def per(a, b, key, dt):
    return None if a.get(key) is None or b.get(key) is None else round((b[key] - a[key]) / dt, 2)


def share(a, b, dt, hz):
    """User, system and total share of one core of a process between two readings of its threads, and its busiest threads."""
    if not a or not b:
        return None
    user = sum(t["user"] for t in b.values()) - sum(t["user"] for t in a.values())
    system = sum(t["system"] for t in b.values()) - sum(t["system"] for t in a.values())
    top = {}
    for k, t in b.items():
        before = a.get(k) or {"user": 0, "system": 0, "ctxt": (0, 0)}
        pct = 100.0 * (t["user"] + t["system"] - before["user"] - before["system"]) / hz / dt
        if pct >= 0.3:
            switches = [(x - y) / dt for x, y in zip(t["ctxt"], before["ctxt"])]
            top[k] = {"pct": round(pct, 2), "user_pct": round(100.0 * (t["user"] - before["user"]) / hz / dt, 2),
                      "system_pct": round(100.0 * (t["system"] - before["system"]) / hz / dt, 2), "ctxt_per_s": [round(x) for x in switches]}
    return {"pct": round(100.0 * (user + system) / hz / dt, 2), "user_pct": round(100.0 * user / hz / dt, 2), "system_pct": round(100.0 * system / hz / dt, 2),
            "top_threads": dict(sorted(top.items(), key=lambda kv: -kv[1]["pct"]))}


def window(a, b):
    """Everything a window says, from two readings."""
    dt, hz = b["mono"] - a["mono"], a["clk_tck"]
    w = {"seconds": round(dt, 2), "comp": share(a.get("comp_threads"), b.get("comp_threads"), dt, hz), "ui": share(a.get("ui_threads"), b.get("ui_threads"), dt, hz)}
    w["combined_pct"] = round((w["comp"] or {}).get("pct", 0) + (w["ui"] or {}).get("pct", 0), 2) if w["comp"] and w["ui"] else None
    sa, sb = summary_fields(a["summary"]), summary_fields(b["summary"])
    w["flips_per_s"] = per(sa, sb, "flips", dt)
    w["ticks_per_s"] = per(sa, sb, "ticks", dt)
    w["frames_drawn_per_s"] = per(sa, sb, "drawn", dt)
    w["ui_frames_per_s"] = per(sa, sb, "ui_frames", dt)
    w["ui_frames_empty_per_s"] = per(sa, sb, "ui_empty", dt)
    w["panel_vsync_per_s"] = per(a, b, "vsync", dt)
    w["osd_irq_per_s"] = per(a, b, "osd_irq", dt)
    ma, mb = (a.get("measure") or {}), (b.get("measure") or {})
    w["animating_flip_intervals_per_s"] = (round((mb["motion"]["intervals"] - ma["motion"]["intervals"]) / dt, 2)
                                           if ma.get("motion") and mb.get("motion") else None)
    # the loop: wake-ups, what they took, and how the ticks were spent
    ta, tb = a.get("stats") or {}, b.get("stats") or {}
    if "passes" in ta and "passes" in tb:
        passes = tb["passes"] - ta["passes"]
        w["loop"] = {"passes_per_s": round(passes / dt, 1), "busy_pct": round(100.0 * (tb["passUsTotal"] - ta["passUsTotal"]) / 1e6 / dt, 2),
                     "ticks_busy_pct": round(100.0 * (tb["tickUsTotal"] - ta["tickUsTotal"]) / 1e6 / dt, 2),
                     "tick_parts_pct": dict(zip(("plan_and_frame", "present", "callbacks", "releases", "rest"),
                                                [round(100.0 * (y - x) / 1e6 / dt, 2) for x, y in zip(ta["tickPartsUs"], tb["tickPartsUs"])]))}
    ea, eb = sa.get("engine"), sb.get("engine")
    frames = (sb["ui_frames"] or 0) - (sa["ui_frames"] or 0)
    if ea and eb and frames > 0:
        us = {k: (eb[k] - ea[k]) / frames for k in eb if k in ea}
        # `scope` (the compositor's GL state saved and put back round each of the engine's calls), `step` (the scene stepped to the frame's time and its
        # layers listed) and `finish` are in the builds that count them; `other` is what is left: the renderer's work between the phases
        us["other"] = us["cpu"] - sum(us[k] for k in us if k != "cpu")
        w["engine_us_per_frame"] = {k: round(v, 1) for k, v in us.items()}
        w["engine_pct"] = round(100.0 * (eb["cpu"] - ea["cpu"]) / 1e6 / dt, 2)
    w["since_start"] = {k: sb[k] for k in ("px_frame", "changed", "draws_frame", "layers_frame") if sb.get(k) is not None}
    da, db = a.get("dvfs"), b.get("dvfs")
    if da and db and db[0] + db[1] > da[0] + da[1]:
        w["mali_busy_pct"] = round(100.0 * (db[0] - da[0]) / ((db[0] - da[0]) + (db[1] - da[1])), 1)
    w["keys_delivered"] = ((b["keys"].get("delivered") or 0) - (a["keys"].get("delivered") or 0))
    return w


def verdicts(w, a, b, sky):
    """The checks, each with what decided it. `None`: the reading was not there."""
    out = {}
    out["quiet"] = {"ok": w["keys_delivered"] == 0 and not a.get("saver") and not b.get("saver") and (b["keys"].get("foreground") == "com.webos.app.home"),
                    "detail": "keys %s, saver %s/%s, foreground %s" % (w["keys_delivered"], a.get("saver"), b.get("saver"), b["keys"].get("foreground"))}
    out["panel_120"] = {"ok": (w.get("panel_vsync_per_s") or 0) > 110, "detail": "vsync %s a second" % w.get("panel_vsync_per_s")}
    exp = EXPECT.get(sky)
    for name, key in (("flips", "flips_per_s"), ("animating", "animating_flip_intervals_per_s")):
        v = w.get(key)
        if exp is None:
            out[name] = {"ok": None, "detail": "%s a second, no reading of the weather to judge it by" % v}
            continue
        lo, hi = exp["flips_per_s" if name == "flips" else "animating_per_s"]
        out[name] = {"ok": None if v is None else lo <= v <= hi, "detail": "%s a second, %s sky expects %g to %g" % (v, sky, lo, hi)}
    v = w.get("combined_pct")
    out["target"] = {"ok": None if v is None else v <= TARGET_PCT, "detail": "compositor %s + UI %s = %s %% of one core, target %g" % (
        (w["comp"] or {}).get("pct"), (w["ui"] or {}).get("pct"), v, TARGET_PCT)}
    return out


def table(rows):
    width = max(len(r[0]) for r in rows)
    return "\n".join("%-*s  %s" % (width, k, v) for k, v in rows)


def summary_text(res):
    w = res["window"]
    rows = [("window", "%.1f s, %s, sky %s (reading %s, theme sky switch %s)" % (w["seconds"], res["label"], res["sky"], res["weather"].get("code"), res["weather"].get("theme_sky_animated"))),
            ("compositor", "%s %% of a core (user %s, system %s)" % ((w["comp"] or {}).get("pct"), (w["comp"] or {}).get("user_pct"), (w["comp"] or {}).get("system_pct"))),
            ("ui", "%s %% of a core (user %s, system %s)" % ((w["ui"] or {}).get("pct"), (w["ui"] or {}).get("user_pct"), (w["ui"] or {}).get("system_pct"))),
            ("together", "%s %%  (target %g)" % (w["combined_pct"], TARGET_PCT)),
            ("flips", "%s a second, %s of them at every refresh; ui frames %s a second (%s empty); ticks %s; panel %s Hz" % (
                w["flips_per_s"], w["animating_flip_intervals_per_s"], w["ui_frames_per_s"], w["ui_frames_empty_per_s"], w["ticks_per_s"], w["panel_vsync_per_s"]))]
    if w.get("loop"):
        rows.append(("loop", "%s wake-ups a second, busy %s %%, ticks %s %% (%s)" % (w["loop"]["passes_per_s"], w["loop"]["busy_pct"], w["loop"]["ticks_busy_pct"], w["loop"]["tick_parts_pct"])))
    if w.get("engine_us_per_frame"):
        rows.append(("engine", "%s us a frame, %s %% of a core: %s" % (w["engine_us_per_frame"]["cpu"], w.get("engine_pct"), w["engine_us_per_frame"])))
    if w.get("mali_busy_pct") is not None:
        rows.append(("gpu", "%s %% busy" % w["mali_busy_pct"]))
    for name, c in res["verdicts"].items():
        rows.append(("check " + name, "%s  %s" % ({True: "PASS", False: "FAIL", None: "?"}[c["ok"]], c["detail"])))
    return table(rows)


# ---- commands ----------------------------------------------------------------------------------------------------------------------------------------------

def cmd_run(secs, label, out_dir):
    os.makedirs(out_dir, exist_ok=True)
    wx = weather()
    sky = "off" if wx.get("theme_sky_animated") is False else sky_of(wx.get("code"), wx.get("wind_mph"))
    say("window of %g s, sky %s: reading" % (secs, sky))
    a = snapshot()
    wait = max(0.0, secs - (time.monotonic() - a["mono"]))
    time.sleep(wait)
    b = snapshot()
    w = window(a, b)
    res = {"label": label, "when": time.strftime("%Y-%m-%d %H:%M:%S"), "target_pct": TARGET_PCT, "sky": sky, "weather": wx, "refresh": b["refresh"], "window": w,
           "verdicts": verdicts(w, a, b, sky), "readings": {"a": a, "b": b}}
    path = os.path.join(out_dir, label + ".json")
    with open(path, "w") as f:
        json.dump(res, f, indent=1)
    print(summary_text(res))
    say("written", path)
    return 0 if all(c["ok"] is not False for c in res["verdicts"].values()) else 1


def cmd_compare(path_a, path_b):
    a, b = (json.load(open(p)) for p in (path_a, path_b))
    rows = []

    def row(name, get):
        try:
            x, y = get(a), get(b)
        except (KeyError, TypeError):
            return
        rows.append((name, "%-10s %-10s %s" % (x, y, "" if not all(isinstance(v, (int, float)) for v in (x, y)) else "%+.2f" % (y - x))))
    rows.append(("", "%-10s %-10s delta" % (a["label"], b["label"])))
    for name, get in (("compositor %", lambda r: r["window"]["comp"]["pct"]), ("ui %", lambda r: r["window"]["ui"]["pct"]), ("together %", lambda r: r["window"]["combined_pct"]),
                      ("flips/s", lambda r: r["window"]["flips_per_s"]), ("animating flips/s", lambda r: r["window"]["animating_flip_intervals_per_s"]),
                      ("ui frames/s", lambda r: r["window"]["ui_frames_per_s"]), ("ticks/s", lambda r: r["window"]["ticks_per_s"]),
                      ("loop wake-ups/s", lambda r: r["window"]["loop"]["passes_per_s"]), ("engine us/frame", lambda r: r["window"]["engine_us_per_frame"]["cpu"]),
                      ("gpu busy %", lambda r: r["window"]["mali_busy_pct"])):
        row(name, get)
    print(table(rows))
    return 0


# ---- the readers on text -------------------------------------------------------------------------------------------------------------------------------

#: a `ctl status` summary of the set (2026-11-04 19:28, the sky moving at 120 a second) with the totals a newer build prints added
SAMPLE = ("uptime 165.2s clients 2 (total 2) surfaces 10 windows 1 requests 204 commits 15 buffers 0 ticks 19700 drawn 19580 callbacks 26 releases 13 skipped 0 gl errors 0 "
          "render last 955 us max 18597 us prepared 3 frame imports 0 held ticks 0 check ok 0 bad 0 unread 0 flips 19536 late 6 lost 26 reached late 9 (over 1 ms 1155, worst 17.2 ms) "
          "direct 0 split 0 odd stamps 4 (0 missed) refresh 8.33 ms (measured 8.3929, changed 1) ui frames 19536 (empty 1) px/frame 217189 changed 202832 draws/frame 51 layers/frame 30 "
          "cpu 1552 us/frame max 36853 us (total us: cpu 30318000 place 1000 plan 2000 backdrops 3000 draw 4000 damage 5000 apply 6000 scope 7000 step 8000 finish 9000) gl errors 0")


def selftest():
    f = summary_fields(SAMPLE)
    assert (f["flips"], f["ticks"], f["ui_frames"], f["ui_empty"], f["draws_frame"], f["layers_frame"], f["cpu_mean_us"]) == (19536, 19700, 19536, 1, 51, 30, 1552), f
    assert f["engine"] == {"cpu": 30318000, "place": 1000, "plan": 2000, "backdrops": 3000, "draw": 4000, "damage": 5000, "apply": 6000, "scope": 7000, "step": 8000, "finish": 9000}, f
    before_scope = summary_fields(SAMPLE.replace(" scope 7000 step 8000 finish 9000", ""))
    assert "scope" not in before_scope["engine"] and before_scope["engine"]["apply"] == 6000, before_scope
    old = summary_fields(SAMPLE.split(" (total us")[0] + " gl errors 0")
    assert old["engine"] is None and old["engine_cpu_total_approx"] == 19536 * 1552
    assert utime_stime("1234 (ld-linux-aarch6) S 1 2 3 4 5 6 7 8 9 10 11 12 13 14") == (11, 12)
    assert utime_stime("42 (a b) c) S 0 0 0 0 0 0 0 0 0 0 250 50 0 0 0") == (250, 50)
    assert ctxt("voluntary_ctxt_switches:\t12\nnonvoluntary_ctxt_switches:\t3\n") == (12, 3)
    assert parse_dvfs("busy_time: 300\nidle_time: 700\n") == (300, 700)
    assert interrupts("  60:   100  200   0   0   GIC  DE2VSYNC\n  61:  5 6 0 0 GIC osd_irq\n", "DE2VSYNC") == 300
    assert (sky_of(63, 0), sky_of(95, 0), sky_of(0, 3), sky_of(3, 0), sky_of(45, 0)) == ("rain", "rain", "calm", "calm", "calm")
    assert (sky_of(73, 0), sky_of(0, 18), sky_of(61, 40), sky_of(None, 5)) == ("fast", "fast", "rain", None)
    # a window: the compositor's main thread takes 2.4 s of CPU in 60 s, one of the UI's takes 0.6 s, 1800 frames
    def thread(user, system, voluntary=0):
        return {"user": user, "system": system, "ctxt": (voluntary, 0)}
    a = {"mono": 100.0, "clk_tck": 100, "comp_threads": {"main/1": thread(1000, 200), "mali/2": thread(50, 10)}, "ui_threads": {"ui/9": thread(100, 20, 10)},
         "summary": SAMPLE, "stats": {"passes": 1000, "passUsTotal": 1_000_000, "tickUsTotal": 900_000, "tickPartsUs": [1, 2, 3, 4, 5]},
         "vsync": 0, "osd_irq": 1000, "keys": {"delivered": 2, "foreground": "com.webos.app.home"}, "saver": False, "dvfs": (100, 900), "measure": {"motion": {"intervals": 100}}}
    later = SAMPLE.replace("flips 19536", "flips 21336").replace("ui frames 19536", "ui frames 21336").replace("ticks 19700", "ticks 21500").replace("cpu 30318000 place 1000 plan 2000 backdrops 3000 draw 4000 damage 5000 apply 6000", "cpu 32118000 place 2000 plan 4000 backdrops 6000 draw 8000 damage 10000 apply 12000 scope 14000 step 16000 finish 18000")
    b = {"mono": 160.0, "clk_tck": 100, "comp_threads": {"main/1": thread(1180, 260), "mali/2": thread(60, 12)}, "ui_threads": {"ui/9": thread(130, 26, 70)},
         "summary": later, "stats": {"passes": 1000 + 2400, "passUsTotal": 1_000_000 + 1_200_000, "tickUsTotal": 900_000 + 1_000_000, "tickPartsUs": [600_001, 200_002, 3, 4, 5]},
         "vsync": 7200, "osd_irq": 1000 + 7200, "keys": {"delivered": 2, "foreground": "com.webos.app.home"}, "saver": False, "dvfs": (400, 1700), "measure": {"motion": {"intervals": 100}}}
    w = window(a, b)
    assert w["comp"]["pct"] == 4.2 and w["comp"]["user_pct"] == 3.17 and w["comp"]["system_pct"] == 1.03, w["comp"]
    assert list(w["comp"]["top_threads"]) == ["main/1"] and w["ui"]["pct"] == 0.6 and w["combined_pct"] == 4.8, w
    assert w["ui"]["top_threads"]["ui/9"]["ctxt_per_s"] == [1, 0]
    assert w["flips_per_s"] == 30.0 and w["osd_irq_per_s"] == 120.0 and w["animating_flip_intervals_per_s"] == 0.0 and w["keys_delivered"] == 0, w
    assert w["loop"]["passes_per_s"] == 40.0 and w["loop"]["busy_pct"] == 2.0 and w["loop"]["tick_parts_pct"]["plan_and_frame"] == 1.0, w["loop"]
    assert w["engine_us_per_frame"]["cpu"] == 1000.0 and w["engine_us_per_frame"]["draw"] == 2.2 and w["engine_pct"] == 3.0, w
    assert w["engine_us_per_frame"]["scope"] == 3.9 and w["engine_us_per_frame"]["other"] == round(1000.0 - (1000 + 2000 + 3000 + 4000 + 5000 + 6000 + 7000 + 8000 + 9000) / 1800, 1), w
    assert w["mali_busy_pct"] == 27.3
    v = verdicts(w, a, b, "calm")
    assert all(c["ok"] for c in v.values()), v
    assert not verdicts(w, a, b, "rain")["flips"]["ok"], "30 flips a second is not what a sky of rain draws"
    assert verdicts(w, a, b, "fast")["flips"]["ok"] and verdicts(w, a, b, None)["flips"]["ok"] is None
    text = summary_text({"label": "t", "sky": "calm", "weather": {"code": 0, "theme_sky_animated": True}, "window": w, "verdicts": v})
    assert "together" in text and "PASS" in text
    print("selftest ok")
    return 0


def main(argv):
    if not argv or argv[0] not in ("run", "read", "compare", "selftest"):
        print(__doc__)
        return 2
    cmd, args = argv[0], argv[1:]

    def opt(name, default):
        return args[args.index(name) + 1] if name in args else default
    if cmd == "selftest":
        return selftest()
    if cmd == "compare":
        return cmd_compare(args[0], args[1])
    if cmd == "read":
        print(json.dumps(snapshot(), indent=1))
        return 0
    label = opt("--label", time.strftime("idle-%H%M%S"))
    return cmd_run(float(opt("--secs", 60)), label, opt("--out", OUT))


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
