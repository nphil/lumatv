#!/usr/bin/env python3
"""One probe slot for the ambient light's picture capture, on the set: Home, HDMI 2 and Netflix, 60 s each.

    python3 ambient/capture/tv_probe_slot.py --plan                    # the steps and the script that runs on the set; touches nothing
    python3 ambient/capture/tv_probe_slot.py --go                      # the slot, from the repository root, with Main's go
    python3 ambient/capture/tv_probe_slot.py --go --scenarios home,hdmi2 --secs 30 --no-mute

LG's library refuses the histogram to any process that is not LG's own Hue Sync service unless the file
/var/luna/preferences/vt_devmode exists (docs/research/ambient-result.md, "Permission gate"). Nitin allowed that file for
short probe slots, removed at the end of each (docs/PLAN.md, Decided). The slot is built around the removal:

- the file exists only while a probe runs: it is created straight before each 60 s capture and removed straight after,
  never while an app is launching or the receiver is being muted. Each window's create and remove time (seconds since the
  epoch, from the set's clock) is in `slot-log.json` under `flag.windows`, with who removed it;
- three things remove it, so a dropped SSH session or a crashed script cannot leave it: the shell script that runs on the set
  (EXIT/INT/TERM/HUP trap; it carries on and finishes without this script), a dead-man timer started before each window that
  removes the file `secs + DEADMAN_MARGIN` (25) s later even if that script is killed, and this script's own `finally`;
- the slot ends by reading the set: the file is gone (`test ! -e`), or the exit status is non-zero and the first line printed
  says so;
- it never starts on a set where the file already exists (an earlier slot left it), unless `--adopt-flag` says to remove it
  at the end.

A slot is Main's: "TV slot for PluginsSlice-N: 10 min, measure", wait for "go", "TV back" when this returns. It writes only under
/tmp on the set, apart from the flag. The receiver is muted through Home Assistant first and put back as it was; the set is left
in Active Standby (`--no-standby` leaves it on), as the earlier slots leave it; Picture Mode is read before and after, never set.
A new failed systemd unit while it runs ends the captures early. The one thing the script cannot make right by itself is a
receiver that fails to unmute: that is printed last, as a PROBLEM.

Results: docs/research/ambient/probe-<date>-<time>/ (or `--out`): `<scenario>.txt` (the probe's report), `.csv` (one line per
poll), `.bin` (the raw histograms, replay with `ambient/examples/replay.rs`), `interrupts-<scenario>-{before,after}.txt`,
`slot-log.json` (every step, with times) and `summary.txt`, which has what goes in the note's "Probe results".
"""
import argparse
import json
import os
import re
import shlex
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(HERE, "..", ".."))
PROBE = os.path.join(HERE, "build", "arm", "ambient-probe")
HA = "http://192.168.1.146"
RECEIVER = "media_player.living_room_avr"
TV_ENTITY = "media_player.living_room_tv"
FLAG = "/var/luna/preferences/vt_devmode"
LIBRARY = "/usr/lib/libvtcapture.so.1.0.0"
LIBRARY_MD5 = "35d3fec6"          # the prefix recorded at the read-only slot (ambient-result.md)
NETFLIX_TARGET = "m=https://www.netflix.com/watch/80141336&source_type=4"
DEADMAN_MARGIN = 25               # seconds past a capture's length at which the dead-man timer removes the flag

# How each scenario gets its picture on the screen: LG's own services, nothing installed. `settle` is the wait after the
# launch, `key` a button pressed through Home Assistant once `settle` is over (the title page's Play), `after_key` the wait
# for the first frame.
SCENARIOS = {
    "home": {"launch": {"id": "com.webos.app.home"}, "settle": 5,
             "what": "Home: no video; the zones show what the shell draws, or nothing if the tap is off there"},
    "hdmi2": {"launch": {"id": "com.webos.app.hdmi2"}, "settle": 8,
              "what": "HDMI 2 (the PC): the zones follow the desktop if the port has a signal"},
    "netflix": {"launch": {"id": "netflix", "params": {"contentTarget": NETFLIX_TARGET}}, "settle": 14, "key": "ENTER", "after_key": 14,
                "what": "Netflix, the 4K title (protected video): whether the zones are real or black"},
}


# ---- the plan (pure; tests/test_ambient_slot.py reads it) ------------------------------------------------------------------

def plan(scenarios, secs, mute=True, standby=True):
    """The slot as ordered (step, why, always): `always` marks what runs even after a failure."""
    steps = [("preflight", "read-only: the set answers and is Active, the library is the one the probe was written for, nobody "
                           "holds /dev/video60, %s is absent, the failed units and Picture Mode are recorded" % FLAG, False),
             ("upload", "the probe and the on-set script to /tmp/ambient-slot-<time>/, and a dry run that resolves the four "
                        "library functions without taking the capture", False)]
    if mute:
        steps.append(("mute", "the receiver is muted through Home Assistant (its state is read first)", False))
    steps.append(("start", "the on-set script runs the scenarios one after the other; this script watches it", False))
    for s in scenarios:
        steps.append(("scenario:" + s, "%s (launch, %d s settle, then flag created, %d s capture, flag removed)" % (
            SCENARIOS[s]["what"], SCENARIOS[s]["settle"] + SCENARIOS[s].get("after_key", 0), secs), False))
    steps.append(("flag-check", "%s is gone, from this script's own look at the set; it is removed here if not" % FLAG, True))
    if mute:
        steps.append(("unmute", "the receiver is put back as it was", True))
    steps += [("fetch", "the results are copied here and summary.txt is written", True),
              ("home", "back to Home", True)]
    if standby:
        steps.append(("standby", "the set is left in Active Standby", True))
    return steps


def remote_script(remote, scenarios, secs, hz=30):
    """The shell script that runs on the set: the captures one after the other; the flag lives only inside each capture."""
    head = """#!/bin/sh
# written by tv_probe_slot.py; runs on the set, under @R@ only (and the flag file, which is created and removed below)
R=@R@
FLAG=@FLAG@
cd "$R" || exit 9
mkdir "$R/run.lock" || exit 8
echo $$ > "$R/slot.pid"
DM=
now() { date +%s; }
removed() { if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed $1 $(now)" >> "$R/flag-events.txt"; fi; }
finish() { removed "$1"; if [ -n "$DM" ]; then kill "$DM" 2>/dev/null; DM=; fi; }
waitfor() { n=0; while [ ! -e "$1" ] && [ "$n" -lt "$2" ]; do sleep 1; n=$((n + 1)); done; }
trap 'finish trap' EXIT
trap 'exit 130' INT TERM HUP
"""
    body = ""
    for name in scenarios:
        s = SCENARIOS[name]
        block = """echo "== @N@ $(now)"
cat /proc/interrupts > "$R/interrupts-@N@-before.txt"
luna-send -n 1 luna://com.webos.applicationManager/launch @LAUNCH@ > "$R/@N@-launch.txt" 2>&1
sleep @SETTLE@
"""
        if s.get("key"):
            block += """now > "$R/@N@-key.txt"
waitfor "$R/@N@-key-sent.txt" 30
sleep @AFTER@
"""
        block += """luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' > "$R/@N@-foreground.txt" 2>&1
( sleep @DEAD@; if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed deadman $(now)" >> "$R/flag-events.txt"; fi ) >/dev/null 2>&1 &
DM=$!
touch "$FLAG" && echo "created @N@ $(now)" >> "$R/flag-events.txt"
"$R/ambient-probe" -s @SECS@ -r @HZ@ -L @N@ -c "$R/@N@.csv" -d "$R/@N@.bin" > "$R/@N@.txt" 2>&1
echo "exit=$?" >> "$R/@N@.txt"
finish end
cat /proc/interrupts > "$R/interrupts-@N@-after.txt"
"""
        block = (block.replace("@N@", name).replace("@LAUNCH@", shlex.quote(json.dumps(s["launch"], separators=(",", ":"))))
                 .replace("@SETTLE@", str(s["settle"])).replace("@AFTER@", str(s.get("after_key", 0)))
                 .replace("@DEAD@", str(secs + DEADMAN_MARGIN)).replace("@SECS@", str(secs)).replace("@HZ@", str(hz)))
        body += block
    return head.replace("@R@", remote).replace("@FLAG@", FLAG) + body + 'echo done > "$R/done.txt"\n'


# ---- reading what the set wrote --------------------------------------------------------------------------------------------

def parse_report(text):
    """The numbers a note wants from one probe report."""
    out = {}
    m = re.search(r"summary preset (\d+): wall=([\d.]+)s calls=(\d+) ok=(\d+) changed=(\d+) \(([\d.]+)/s\) rc:((?: \d+ x\d+)*)", text)
    if m:
        out.update(preset=int(m.group(1)), wall_s=float(m.group(2)), calls=int(m.group(3)), ok=int(m.group(4)), changed=int(m.group(5)),
                   changes_per_s=float(m.group(6)), rc={int(a): int(b) for a, b in re.findall(r"(\d+) x(\d+)", m.group(7))})
    m = re.search(r"call latency us: p50=(\d+) p90=(\d+) p99=(\d+) max=(\d+)", text)
    if m:
        out["latency_us"] = dict(zip(("p50", "p90", "p99", "max"), map(int, m.groups())))
    m = re.search(r"change interval us: n=(\d+) p50=(\d+) p90=(\d+) p99=(\d+) max=(\d+)", text)
    if m:
        out["change_interval_us"] = dict(zip(("n", "p50", "p90", "p99", "max"), map(int, m.groups())))
    m = re.search(r"cpu: probe ([\d.]+)% of one core .*system busy ([\d.]+)%", text)
    if m:
        out["cpu_probe_pct"], out["cpu_system_pct"] = float(m.group(1)), float(m.group(2))
    m = re.search(r"initHistogram ok in (\d+) ms", text)
    if m:
        out["init_ms"] = int(m.group(1))
    m = re.search(r"^exit=(\d+)$", text, re.M)
    if m:
        out["exit"] = int(m.group(1))
    out["init_refused"] = "initHistogram returned NULL" in text
    out["flag_seen"] = bool(re.search(r"dev flag \S+: present", text))
    return out


def verdict(report):
    """What the numbers say about that picture, in words for the note."""
    if report.get("init_refused"):
        return "LG refused the histogram (initHistogram returned NULL): not allowed, capture busy or no driver"
    if "exit" in report and report["exit"] != 0:
        return "no capture (exit %d)" % report["exit"]
    if not report.get("calls"):
        return "no report"
    note = "" if report.get("flag_seen") else "; the flag was not there when the probe started"
    if report.get("ok", 0) == 0:
        return "every call failed (return codes %s)%s" % (report.get("rc"), note)
    if report.get("changed", 0) < 5:
        return "the zones did not change: a still picture, no signal, or protected video that comes back black%s" % note
    cpu = report.get("cpu_probe_pct", 0)
    return "the zones change %.1f times a second, probe CPU %.2f%% of a core%s%s" % (
        report.get("changes_per_s", 0), cpu, "" if cpu < 1 else " (over the plan's 1 %)", note)


def seconds(text):
    """An epoch time as the set wrote it. The set's busybox `date` has no %N and prints it literally, so only the whole seconds count."""
    m = re.match(r"\d+(\.\d+)?", text)
    return float(m.group()) if m else None


def parse_events(text):
    """The flag's windows from the on-set events file: `created <scenario> <t>` then `removed <by> <t>`."""
    windows, open_ = [], None
    for line in text.splitlines():
        p = line.split()
        if len(p) == 3 and p[0] == "created":
            if open_:
                windows.append(open_)
            open_ = {"scenario": p[1], "created": seconds(p[2]), "removed": None, "removedBy": None}
        elif len(p) == 3 and p[0] == "removed":
            if open_:
                open_.update(removed=seconds(p[2]), removedBy=p[1])
                windows.append(open_)
                open_ = None
            else:
                windows.append({"scenario": None, "created": None, "removed": seconds(p[2]), "removedBy": p[1]})
    if open_:
        windows.append(open_)
    return windows


# ---- the slot --------------------------------------------------------------------------------------------------------------

def slurp(path):
    with open(path) as f:
        return f.read()


class Log:
    """Every step with its time, written out as it happens, so a slot that dies still says what it did."""

    def __init__(self, out):
        self.out = out
        self.doc = {"started": time.time(), "steps": [], "flag": {"path": FLAG, "windows": []}}
        os.makedirs(out, exist_ok=True)

    def step(self, name, **info):
        self.doc["steps"].append(dict(step=name, at=time.time(), **info))
        self.save()
        print("%s %-18s %s" % (time.strftime("%H:%M:%S"), name, json.dumps(info) if info else ""), flush=True)

    def save(self):
        with open(os.path.join(self.out, "slot-log.json"), "w") as f:
            json.dump(self.doc, f, indent=2)


FAILED = r"""systemctl --failed --no-legend 2>/dev/null | awk '{for (i = 1; i <= NF; i++) if ($i ~ /\.(service|socket|mount|timer|path|target|slice)$/) { print $i; break }}'"""
PICTURE = """luna-send -n 1 luna://com.webos.settingsservice/getSystemSettings '{"category":"picture","keys":["pictureMode"]}' | tr -d '\\n'"""


def run(a):
    scenarios = a.scenarios.split(",")
    unknown = [s for s in scenarios if s not in SCENARIOS]
    if unknown:
        raise SystemExit("unknown scenario %s (home, hdmi2, netflix)" % unknown)
    if not os.path.exists(PROBE):
        raise SystemExit("%s is not built: run ambient/capture/build.sh first" % PROBE)
    # The same helpers the compositor slots use: tv (ssh, sftp), tv_gate (wake, receiver mute, power state).
    sys.path[:0] = [os.path.join(REPO, "compositor", "tools"), os.path.join(REPO, "docs", "research", "arch")]
    import tv
    import tv_gate

    stamp = time.strftime("%Y-%m-%d-%H%M")
    out = a.out or os.path.join(REPO, "docs", "research", "ambient", "probe-" + stamp)
    remote = "/tmp/ambient-slot-" + stamp.replace("-", "")
    log = Log(out)
    problems = []
    muted_before = None     # None: the receiver was not touched
    owns_flag = False       # this slot may remove the flag (it was absent at the start, or --adopt-flag)
    woke = False            # the set was in standby when the slot began
    uploaded = started = False

    def sh(cmd, timeout=60):
        return tv.tv(cmd, timeout)

    def has(path):
        return sh("test -e %s && echo yes || echo no" % shlex.quote(path)).split()[0] == "yes"

    def ha(path, body=None):
        req = urllib.request.Request(HA + path, data=json.dumps(body).encode() if body is not None else None,
                                     headers={"Authorization": "Bearer " + tv_gate.ha_token(), "Content-Type": "application/json"})
        for attempt in range(4):
            try:
                with urllib.request.urlopen(req, timeout=15) as r:
                    return json.loads(r.read() or b"null")
            except urllib.error.HTTPError as e:
                if e.code != 500 or attempt == 3:       # HA's webostv link drops now and then
                    raise
                time.sleep(2)

    def stop_remote():
        # The pid file, not pkill -f: a pattern naming the script would match the shell that runs this very command line.
        # The probe is stopped as well, because the script's own trap waits for its foreground command.
        sh("kill -TERM $(cat %s/slot.pid 2>/dev/null) 2>/dev/null; pkill -TERM ambient-probe 2>/dev/null; sleep 2; true" % remote)

    try:
        # ---- preflight: read-only, apart from waking the set
        try:
            before = tv_gate.power_state()
        except Exception:
            before = "unreachable"
        log.step("power-state", state=before)
        if not tv_gate.wake():
            raise SystemExit("the set does not answer; nothing was changed")
        woke = before != "active"
        state = tv_gate.power_state()
        if state != "active":
            raise SystemExit("the set is %r, not active" % state)
        existed = has(FLAG)
        log.doc["flag"]["existedBefore"] = existed
        log.step("preflight", flag_existed_before=existed, woke=woke)
        if existed and not a.adopt_flag:
            raise SystemExit("%s exists already: an earlier slot left it. Not starting (--adopt-flag runs anyway and removes it at the end)." % FLAG)
        owns_flag = True
        md5 = sh("md5sum %s" % LIBRARY).split()[0]
        log.step("library", md5=md5)
        if not md5.startswith(LIBRARY_MD5):
            raise SystemExit("libvtcapture is not the one the probe was written for (md5 %s)" % md5)
        holders = sh("for p in /proc/[0-9]*; do ls -l $p/fd 2>/dev/null | grep -q /dev/video60 && echo ${p#/proc/}; done").split()
        log.step("holders", pids=holders)
        if holders:
            raise SystemExit("/dev/video60 is held by pid %s: another capture owner" % holders)
        failed_before = sh(FAILED).split()
        log.step("baseline", failed_units=failed_before, picture=sh(PICTURE).strip()[:200])

        # ---- upload, dry run
        sh("mkdir -p %s" % remote)
        uploaded = True
        tv.put(PROBE, remote + "/ambient-probe")
        script = os.path.join(out, "slot.sh")
        with open(script, "w") as f:
            f.write(remote_script(remote, scenarios, a.secs))
        tv.put(script, remote + "/slot.sh")
        sh("chmod +x %s/ambient-probe %s/slot.sh" % (remote, remote))
        dry = sh("%s/ambient-probe -n" % remote)
        log.step("dry-run", output=dry.strip()[:400])
        if "library: init=0x" not in dry:
            raise SystemExit("the dry run did not resolve the library functions: " + dry[:200])

        # ---- the receiver
        if not a.no_mute:
            st = ha("/api/states/" + RECEIVER)
            muted_before = bool(st["attributes"].get("is_volume_muted"))
            log.step("receiver-before", muted=muted_before, state=st.get("state"), volume=st["attributes"].get("volume_level"))
            ha("/api/services/media_player/volume_mute", {"entity_id": RECEIVER, "is_volume_muted": True})
            log.step("receiver-muted")

        # ---- the captures: one script on the set; the button a scenario needs is pressed from here when it asks
        started = True
        sh("nohup sh %s/slot.sh > %s/slot.out 2>&1 < /dev/null &" % (remote, remote))
        log.step("started", remote=remote)
        budget = sum(SCENARIOS[s]["settle"] + SCENARIOS[s].get("after_key", 0) + 30 for s in scenarios) + len(scenarios) * (a.secs + 15) + 60
        deadline = time.time() + budget
        pressed = set()
        while time.time() < deadline:
            time.sleep(4)
            listing = sh("ls %s; echo ---; %s" % (remote, FAILED))
            names, _, failed = listing.partition("---")
            files = names.split()
            for name in scenarios:
                key = SCENARIOS[name].get("key")
                if key and name not in pressed and name + "-key.txt" in files:
                    pressed.add(name)
                    try:
                        ha("/api/services/webostv/button", {"entity_id": TV_ENTITY, "button": key})
                        log.step("key", scenario=name, button=key)
                    except Exception as e:           # the script waits 30 s for the marker, then captures whatever is on screen
                        problems.append("the %s button for %s was not sent: %s" % (key, name, e))
                    sh("touch %s/%s-key-sent.txt" % (remote, name))
            new = [u for u in failed.split() if u not in failed_before]
            if new:
                problems.append("a new failed unit during the slot: %s" % new)
                log.step("abort", new_failed_units=new)
                break
            if "done.txt" in files:
                break
        else:
            problems.append("the captures did not finish in %d s" % budget)
        log.step("captures-over")
    except SystemExit as e:
        problems.append(str(e))
        raise
    finally:
        # ---- stop what is still running, then the flag: whatever happened above
        try:
            if started and not has(remote + "/done.txt"):
                stop_remote()
                log.step("script-stopped")
        except Exception as e:
            problems.append("could not stop the script on the set (%s): kill it and rm -f %s" % (e, FLAG))
        if owns_flag:
            try:
                if has(FLAG):
                    sh("rm -f %s; echo \"removed finally $(date +%%s)\" >> %s/flag-events.txt" % (FLAG, remote))
                    problems.append("the flag was still there when the captures were over; this script's own check removed it")
                gone = not has(FLAG)
                windows = parse_events(sh("cat %s/flag-events.txt 2>/dev/null" % remote))
                log.doc["flag"].update(windows=windows, gone=gone,
                                       createdAt=next((w["created"] for w in windows if w["created"]), None),
                                       removedAt=next((w["removed"] for w in reversed(windows) if w["removed"]), None))
                log.step("flag-check", gone=gone, windows=windows)
                if not gone:
                    problems.append("THE FLAG IS STILL THERE: rm -f %s" % FLAG)
                if any(w["removed"] is None for w in windows):
                    problems.append("a flag window has no removal time: %s" % windows)
            except Exception as e:
                problems.append("COULD NOT CONFIRM THE FLAG IS GONE (%s): on the set, rm -f %s" % (e, FLAG))
        # ---- the receiver, while the set is still on
        try:
            if muted_before is False:
                ha("/api/services/media_player/volume_mute", {"entity_id": RECEIVER, "is_volume_muted": False})
                log.step("receiver-unmuted")
        except Exception as e:
            problems.append("THE RECEIVER IS STILL MUTED (%s): unmute %s" % (e, RECEIVER))
        # ---- the results, then back as found: Home, and standby unless the set was left on
        try:
            if started:
                fetch(tv, sh, remote, scenarios, out, log)
                log.step("picture-after", reply=sh(PICTURE).strip()[:200])
            elif uploaded:
                sh("rm -rf %s" % remote)
        except Exception as e:
            problems.append("results not fetched (they stay on the set under %s): %s" % (remote, e))
        try:
            if started or woke:
                sh("luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.home\"}'")
                if not a.no_standby:
                    time.sleep(3)
                    tv_gate.pty("luna-send -n 1 -f luna://com.webos.service.tvpower/power/powerOff '{\"reason\":\"remoteKey\"}'", 20)
                    log.step("standby")
        except Exception as e:
            problems.append("the set was not put back (%s)" % e)
        log.doc["problems"] = problems
        log.save()
        for p in problems:
            print("PROBLEM:", p, file=sys.stderr)
    return 1 if problems else 0


def fetch(tv, sh, remote, scenarios, out, log):
    lines = ["probe slot %s" % time.strftime("%Y-%m-%d %H:%M"), ""]
    for w in log.doc["flag"].get("windows", []):
        lines.append("flag %s: created %s, removed %s by %s" % (w["scenario"], w["created"], w["removed"], w["removedBy"]))
    lines.append("")
    for name in scenarios:
        for ext in ("txt", "csv", "bin"):
            if sh("test -e %s/%s.%s && echo yes || echo no" % (remote, name, ext)).split()[0] == "yes":
                tv.get("%s/%s.%s" % (remote, name, ext), os.path.join(out, "%s.%s" % (name, ext)))
        for extra in ("launch", "foreground"):
            with open(os.path.join(out, "%s-%s.txt" % (name, extra)), "w") as f:
                f.write(sh("cat %s/%s-%s.txt 2>/dev/null" % (remote, name, extra)))
        for when in ("before", "after"):
            with open(os.path.join(out, "interrupts-%s-%s.txt" % (name, when)), "w") as f:
                f.write(sh("cat %s/interrupts-%s-%s.txt 2>/dev/null" % (remote, name, when)))
        path = os.path.join(out, name + ".txt")
        report = parse_report(slurp(path)) if os.path.exists(path) else {}
        lines += ["%s: %s" % (name, verdict(report)), "  " + json.dumps(report)]
        fg = os.path.join(out, name + "-foreground.txt")
        m = re.search(r'"appId"\s*:\s*"([^"]*)"', slurp(fg)) if os.path.exists(fg) else None
        lines.append("  foreground when the capture started: %s" % (m.group(1) if m else "unknown"))
    with open(os.path.join(out, "summary.txt"), "w") as f:
        f.write("\n".join(lines) + "\n")
    print("\n".join(lines))
    sh("rm -rf %s" % remote)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--plan", action="store_true", help="print the steps and the script that runs on the set; touch nothing (the default)")
    ap.add_argument("--go", action="store_true", help="run the slot (only with Main's go)")
    ap.add_argument("--scenarios", default="home,hdmi2,netflix")
    ap.add_argument("--secs", type=int, default=60)
    ap.add_argument("--out", help="where the results go (default docs/research/ambient/probe-<date>-<time>)")
    ap.add_argument("--no-mute", action="store_true", help="leave the receiver alone")
    ap.add_argument("--no-standby", action="store_true", help="leave the set on instead of Active Standby")
    ap.add_argument("--adopt-flag", action="store_true", help="run although the flag exists already, and remove it at the end")
    a = ap.parse_args(argv)
    if not 5 <= a.secs <= 600:
        ap.error("--secs is 5 to 600")
    if a.go:
        return run(a)
    for n, (step, why, always) in enumerate(plan(a.scenarios.split(","), a.secs, not a.no_mute, not a.no_standby), 1):
        print("%2d. %-16s %s%s" % (n, step, why, "   [always, even after a failure]" if always else ""))
    print("\nruns on the set:\n")
    print(remote_script("/tmp/ambient-slot-<time>", a.scenarios.split(","), a.secs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
