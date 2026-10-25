#!/usr/bin/env python3
"""One probe slot for the ambient light's picture capture, on the set: Home, HDMI 2 and Netflix, 60 s each, through the package's
own colour reader (lumatv-capture) with the LS2 role it ships with.

    python3 ambient/capture/tv_probe_slot.py --plan                    # the steps and the script that runs on the set; touches nothing
    python3 ambient/capture/tv_probe_slot.py --go                      # the slot, from the repository root, with Main's go
    python3 ambient/capture/tv_probe_slot.py --go --scenarios home,hdmi2 --secs 30 --no-mute --role-type privileged

What a slot installs is what the package installs, by the same script (payload/capture/capture.sh): the 32-bit service staged
under the data directory (`/var/lib/lumatv/bin/lumatv-capture`) and its LS2 role written for that path (LG's library registers a bus
client once its permission gate has passed, and ls-hubd refuses an executable it has no role for: the slot of 2026-10-25 12:35,
docs/research/ambient-result.md). The set's own LumaTV package need not carry it yet: the slot puts the same files there and takes
them away again if they were not there (`capture.sh remove`), or starts the service again if it was running. The slot's copy reads a
socket of the slot's own (`slot_collect.py` on the set stands in for the helper on it), so the helper's capture socket is left alone.

LG's library refuses the histogram to any process that is not LG's own Hue Sync service unless the file
/var/luna/preferences/vt_devmode exists (docs/research/ambient-result.md, "Permission gate"). Nitin allowed that file for
short probe slots, removed at the end of each (docs/PLAN.md, Decided). The slot is built around the removal:

- the file exists only while a capture runs: created straight before the service starts for a window and removed straight after
  the service has given the capture back, never while an app is launching or the receiver is being muted. Each window's create and
  remove time (whole seconds since the epoch, the set's busybox `date` has no fractions) is in `slot-log.json` under `flag.windows`,
  with who removed it;
- three things remove it, so a dropped SSH session or a crashed script cannot leave it: the shell script that runs on the set
  (EXIT/INT/TERM/HUP trap; it carries on and finishes without this script), a dead-man timer started before each window that
  removes the file `secs + DEADMAN_MARGIN` (45) s later even if that script is killed, and this script's own `finally`;
- the slot ends by reading the set: the file is gone (`test ! -e`), or the exit status is non-zero and the first line printed
  says so;
- it never starts on a set where the file already exists (an earlier slot left it), unless `--adopt-flag` says to remove it
  at the end.

A slot is Main's: "TV slot for PluginsSlice-N: 10 min, measure", wait for "go", "TV back" when this returns. It writes under /tmp on the
set, the role file and the staged service (put back as found), and the flag. The receiver is muted through Home Assistant first and
put back as it was; the set is left in Active Standby (`--no-standby` leaves it on); Picture Mode is read before and after, never
set. A new failed systemd unit while it runs ends the captures early. The one thing the script cannot make right by itself is a
receiver that fails to unmute: that is printed last, as a PROBLEM.

If a window says ls-hubd refused the registration (`LSHUB_` lines in `<scenario>.log.txt`, "LSRegisterPubPriv FAILED" in
`<scenario>.service.log`), the role is what to change: `--role-type privileged` writes the other type of role (payload/capture/role.json
ships `regular`; which one LG's library needs is not known offline).

Results: docs/research/ambient/probe-<date>-<time>/ (or `--out`): `<scenario>.json` (frames, changes, intervals, the service's CPU,
the last colour of every area), `.service.log` (what the service said), `.log.txt` (ls-hubd's and LG's capture lines), `.txt`
(the collector's line), `interrupts-<scenario>-{before,after}.txt`, `slot-log.json` (every step, with times) and `summary.txt`,
which has what goes in the note's "Probe results".
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
SERVICE = os.path.join(HERE, "build", "arm", "lumatv-capture")
COLLECT = os.path.join(HERE, "slot_collect.py")
PACKAGE = os.path.join(REPO, "payload", "capture")
CAPTURE_SH = os.path.join(PACKAGE, "capture.sh")
ROLE = os.path.join(PACKAGE, "role.json")
HA = "http://192.168.1.146"
RECEIVER = "media_player.living_room_avr"
TV_ENTITY = "media_player.living_room_tv"
FLAG = "/var/luna/preferences/vt_devmode"
LIBRARY = "/usr/lib/libvtcapture.so.1.0.0"
LIBRARY_MD5 = "35d3fec6"          # the prefix recorded at the read-only slot (ambient-result.md)
NETFLIX_TARGET = "m=https://www.netflix.com/watch/80141336&source_type=4"
DEADMAN_MARGIN = 45               # seconds past a capture's length at which the dead-man timer removes the flag

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


def data_dir():
    """The brand's data directory on the set (brand.json), where the package stages the service."""
    with open(os.path.join(REPO, "brand.json")) as f:
        return json.load(f)["dataDir"]


# ---- the plan (pure; tests/test_ambient_slot.py reads it) ------------------------------------------------------------------

def plan(scenarios, secs, mute=True, standby=True):
    """The slot as ordered (step, why, always): `always` marks what runs even after a failure."""
    steps = [("preflight", "read-only: the set answers and is Active, the library is the one the reader was written for, nobody "
                           "holds /dev/video60, %s is absent, the failed units and Picture Mode are recorded" % FLAG, False),
             ("upload", "the service, its role template, capture.sh and the collector to /tmp/ambient-slot-<time>/, and a dry run that "
                        "resolves the four library functions without taking the capture", False),
             ("install", "capture.sh stages the service under the data directory and writes its LS2 role for that path (what the package "
                         "does at apply); whether it was installed or running before is recorded", False)]
    if mute:
        steps.append(("mute", "the receiver is muted through Home Assistant (its state is read first)", False))
    steps.append(("start", "the on-set script runs the scenarios one after the other; this script watches it", False))
    for s in scenarios:
        steps.append(("scenario:" + s, "%s (launch, %d s settle, then flag created, the service started, %d s collected, the capture "
                                       "given back, flag removed)" % (SCENARIOS[s]["what"], SCENARIOS[s]["settle"] + SCENARIOS[s].get("after_key", 0), secs), False))
    steps.append(("flag-check", "%s is gone, from this script's own look at the set; it is removed here if not" % FLAG, True))
    steps.append(("package", "the service and its role are removed again if they were not there before, and the service started again "
                             "if it was running", True))
    if mute:
        steps.append(("unmute", "the receiver is put back as it was", True))
    steps += [("fetch", "the results are copied here and summary.txt is written", True),
              ("home", "back to Home", True)]
    if standby:
        steps.append(("standby", "the set is left in Active Standby", True))
    return steps


def remote_script(remote, state, scenarios, secs, hz=30):
    """The shell script that runs on the set: the captures one after the other; the flag lives only inside each capture."""
    head = """#!/bin/sh
# written by tv_probe_slot.py; runs on the set, under @R@ only, with the staged service under @STATE@ and the flag file, which is
# created and removed below
R=@R@
STATE=@STATE@
FLAG=@FLAG@
cd "$R" || exit 9
mkdir "$R/run.lock" || exit 8
echo $$ > "$R/slot.pid"
DM=
SVC=
COL=
now() { date +%s; }
removed() { if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed $1 $(now)" >> "$R/flag-events.txt"; fi; }
stop_service() {
  if [ -n "$COL" ]; then kill "$COL" 2>/dev/null; wait "$COL" 2>/dev/null; COL=; fi
  if [ -n "$SVC" ]; then kill "$SVC" 2>/dev/null; wait "$SVC" 2>/dev/null; SVC=; fi
}
finish() { removed "$1"; if [ -n "$DM" ]; then kill "$DM" 2>/dev/null; DM=; fi; }
waitfor() { n=0; while [ ! -e "$1" ] && [ "$n" -lt "$2" ]; do sleep 1; n=$((n + 1)); done; }
trap 'stop_service; finish trap' EXIT
trap 'exit 130' INT TERM HUP
# the helper's own copy of the service, if one runs, leaves the capture to this one
sh "$R/capture.sh" stop "$STATE" >/dev/null 2>&1
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
LOG0=$(wc -c < /var/log/messages 2>/dev/null || echo 0)
( sleep @DEAD@; if [ -e "$FLAG" ]; then rm -f "$FLAG"; echo "removed deadman $(now)" >> "$R/flag-events.txt"; fi ) >/dev/null 2>&1 &
DM=$!
touch "$FLAG" && echo "created @N@ $(now)" >> "$R/flag-events.txt"
"$STATE/bin/lumatv-capture" -s "$R/cap.sock" -r @HZ@ -i 100 -b 1000 > "$R/@N@.service.log" 2>&1 &
SVC=$!
echo $SVC > "$R/svc.pid"
python3 "$R/slot_collect.py" "$R/cap.sock" @SECS@ @N@ "$R/@N@.json" --pid $SVC > "$R/@N@.txt" 2>&1 &
COL=$!
wait $COL
echo "exit=$?" >> "$R/@N@.txt"
COL=
n=0; while [ "$n" -lt 40 ] && ! grep -q "capture ended" "$R/@N@.service.log" 2>/dev/null; do sleep 0.1; n=$((n + 1)); done
stop_service
finish end
tail -c +$((LOG0 + 1)) /var/log/messages 2>/dev/null | grep -E 'LSHUB|capture|vtCapture' | tail -n 40 > "$R/@N@.log.txt"
cat /proc/interrupts > "$R/interrupts-@N@-after.txt"
"""
        block = (block.replace("@N@", name).replace("@LAUNCH@", shlex.quote(json.dumps(s["launch"], separators=(",", ":"))))
                 .replace("@SETTLE@", str(s["settle"])).replace("@AFTER@", str(s.get("after_key", 0)))
                 .replace("@DEAD@", str(secs + DEADMAN_MARGIN)).replace("@SECS@", str(secs)).replace("@HZ@", str(hz)))
        body += block
    return head.replace("@R@", remote).replace("@STATE@", state).replace("@FLAG@", FLAG) + body + 'echo done > "$R/done.txt"\n'


# ---- reading what the set wrote --------------------------------------------------------------------------------------------

def parse_report(text):
    """The collector's JSON report of one window ({} when there is none)."""
    try:
        doc = json.loads(text)
    except ValueError:
        return {}
    return doc if isinstance(doc, dict) else {}


def seconds(text):
    """An epoch time as the set wrote it. The set's busybox `date` has no %N and prints it literally, so only the whole seconds count."""
    m = re.match(r"\d+(\.\d+)?", text)
    return float(m.group()) if m else None


def verdict(report, service_log="", hub_log=""):
    """What the numbers say about that picture, in words for the note."""
    if not report:
        return "no report"
    if not report.get("connected"):
        return "the service never connected to the slot's socket (it did not start: see the service log)"
    if not report.get("frames"):
        if "initHistogram returned NULL" in service_log:
            why = "ls-hubd has no role for the executable (LSHUB_NO_ROLE_FILE)" if "LSHUB_NO_ROLE_FILE" in hub_log else "see the log lines"
            return "LG refused the histogram (initHistogram returned NULL): %s" % why
        return "the service connected and sent no frames (the polls failed: see the service log)"
    if report.get("changed", 0) < 5:
        return "the zones did not change: a still picture, no signal, or protected video that comes back black"
    cpu = report.get("service_cpu_pct")
    return "%.1f frames a second, the zones change %.1f times a second, service CPU %s%% of a core%s" % (
        report.get("frames_per_s", 0), report.get("changes_per_s", 0), "?" if cpu is None else cpu, "" if cpu is None or cpu < 1 else " (over the plan's 1 %)")


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
    for path in (SERVICE, COLLECT, CAPTURE_SH, ROLE):
        if not os.path.exists(path):
            raise SystemExit("%s is not there%s" % (path, ": run ambient/capture/build.sh first" if path == SERVICE else ""))
    # The same helpers the compositor slots use: tv (ssh, sftp), tv_gate (wake, receiver mute, power state).
    sys.path[:0] = [os.path.join(REPO, "compositor", "tools"), os.path.join(REPO, "docs", "research", "arch")]
    import tv
    import tv_gate

    stamp = time.strftime("%Y-%m-%d-%H%M")
    out = a.out or os.path.join(REPO, "docs", "research", "ambient", "probe-" + stamp)
    remote = "/tmp/ambient-slot-" + stamp.replace("-", "")
    state = data_dir()
    log = Log(out)
    problems = []
    muted_before = None     # None: the receiver was not touched
    owns_flag = False       # this slot may remove the flag (it was absent at the start, or --adopt-flag)
    woke = False            # the set was in standby when the slot began
    uploaded = started = installed = False
    before = {}             # capture.sh status before the slot put anything there

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
        # The pid files, not pkill -f: a pattern naming the script would match the shell that runs this very command line.
        # The service is stopped as well, because the script's own trap waits for the collector.
        sh("kill -TERM $(cat %s/slot.pid 2>/dev/null) 2>/dev/null; kill -TERM $(cat %s/svc.pid 2>/dev/null) 2>/dev/null; sleep 2; true" % (remote, remote))

    try:
        # ---- preflight: read-only, apart from waking the set
        try:
            before_power = tv_gate.power_state()
        except Exception:
            before_power = "unreachable"
        log.step("power-state", state=before_power)
        if not tv_gate.wake():
            raise SystemExit("the set does not answer; nothing was changed")
        woke = before_power != "active"
        power = tv_gate.power_state()
        if power != "active":
            raise SystemExit("the set is %r, not active" % power)
        existed = has(FLAG)
        log.doc["flag"]["existedBefore"] = existed
        log.step("preflight", flag_existed_before=existed, woke=woke)
        if existed and not a.adopt_flag:
            raise SystemExit("%s exists already: an earlier slot left it. Not starting (--adopt-flag runs anyway and removes it at the end)." % FLAG)
        owns_flag = True
        md5 = sh("md5sum %s" % LIBRARY).split()[0]
        log.step("library", md5=md5)
        if not md5.startswith(LIBRARY_MD5):
            raise SystemExit("libvtcapture is not the one the reader was written for (md5 %s)" % md5)
        holders = sh("for p in /proc/[0-9]*; do ls -l $p/fd 2>/dev/null | grep -q /dev/video60 && echo ${p#/proc/}; done").split()
        log.step("holders", pids=holders)
        if holders:
            raise SystemExit("/dev/video60 is held by pid %s: another capture owner" % holders)
        failed_before = sh(FAILED).split()
        log.step("baseline", failed_units=failed_before, picture=sh(PICTURE).strip()[:200])

        # ---- upload, dry run, install
        sh("mkdir -p %s" % remote)
        uploaded = True
        role_local = os.path.join(out, "role.json")
        with open(role_local, "w") as f:
            f.write(slurp(ROLE).replace('"type": "regular"', '"type": "%s"' % a.role_type))
        for local, name in ((SERVICE, "lumatv-capture"), (COLLECT, "slot_collect.py"), (CAPTURE_SH, "capture.sh"), (role_local, "role.json")):
            tv.put(local, "%s/%s" % (remote, name))
        sh("chmod +x %s/lumatv-capture %s/capture.sh" % (remote, remote))
        dry = sh("%s/lumatv-capture -n" % remote)
        log.step("dry-run", output=dry.strip()[:300])
        if "library: init=0x" not in dry:
            raise SystemExit("the dry run did not resolve the library functions: " + dry[:200])
        try:
            before = json.loads(sh("sh %s/capture.sh status %s" % (remote, shlex.quote(state))).strip().splitlines()[0])
        except (ValueError, IndexError):
            before = {}
        log.step("package-before", status=before)
        script = os.path.join(out, "slot.sh")
        with open(script, "w") as f:
            f.write(remote_script(remote, state, scenarios, a.secs))
        tv.put(script, remote + "/slot.sh")
        installed = True          # from here the package state is this slot's to put back
        res = sh("sh %s/capture.sh install %s/lumatv-capture %s/role.json %s; echo rc=$?" % (remote, remote, remote, shlex.quote(state)))
        log.step("installed", role_type=a.role_type, output=res.strip()[:300])
        if "rc=0" not in res:
            raise SystemExit("capture.sh install failed: " + res[:200])

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
        budget = sum(SCENARIOS[s]["settle"] + SCENARIOS[s].get("after_key", 0) + 30 for s in scenarios) + len(scenarios) * (a.secs + 45) + 60
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
        # ---- the package as it was: the service and its role gone if they were not there, the service running again if it was
        if installed:
            try:
                if before.get("installed") or before.get("role"):
                    if before.get("running"):
                        sh("sh %s/capture.sh start %s %s/ambient.sock" % (remote, shlex.quote(state), state))
                        log.step("service-restarted")
                else:
                    sh("sh %s/capture.sh remove %s" % (remote, shlex.quote(state)))
                    left = sh("sh %s/capture.sh status %s" % (remote, shlex.quote(state))).strip().splitlines()
                    log.step("package-removed", status=left[0] if left else "")
                    if left and ('"installed":true' in left[0] or '"role":true' in left[0] or '"running":true' in left[0]):
                        problems.append("the service or its role is still on the set: sh %s/capture.sh remove %s" % (remote, state))
            except Exception as e:
                problems.append("could not put the package back as it was (%s): capture.sh remove %s" % (e, state))
        # ---- the receiver, while the set is still on
        try:
            if muted_before is False:
                ha("/api/services/media_player/volume_mute", {"entity_id": RECEIVER, "is_volume_muted": False})
                log.step("receiver-unmuted")
        except Exception as e:
            problems.append("THE RECEIVER IS STILL MUTED (%s): unmute %s" % (e, RECEIVER))
        # ---- the results, then home and standby unless the set was left on
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
        for suffix in ("json", "txt", "service.log", "log.txt", "launch.txt", "foreground.txt"):
            remote_name = name + ("-" + suffix if suffix in ("launch.txt", "foreground.txt") else "." + suffix)
            with open(os.path.join(out, remote_name), "w") as f:
                f.write(sh("cat %s/%s 2>/dev/null" % (remote, remote_name)))
        for when in ("before", "after"):
            with open(os.path.join(out, "interrupts-%s-%s.txt" % (name, when)), "w") as f:
                f.write(sh("cat %s/interrupts-%s-%s.txt 2>/dev/null" % (remote, name, when)))
        report = parse_report(slurp(os.path.join(out, name + ".json")))
        service_log, hub_log = slurp(os.path.join(out, name + ".service.log")), slurp(os.path.join(out, name + ".log.txt"))
        lines += ["%s: %s" % (name, verdict(report, service_log, hub_log)), "  " + json.dumps({k: v for k, v in report.items() if k != "zones"})]
        fg = slurp(os.path.join(out, name + "-foreground.txt"))
        m = re.search(r'"appId"\s*:\s*"([^"]*)"', fg)
        lines.append("  foreground when the capture started: %s" % (m.group(1) if m else "unknown"))
        lines += ["  ls-hubd / capture: " + l for l in hub_log.strip().splitlines()[-6:]]
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
    ap.add_argument("--role-type", choices=("regular", "privileged"), default="regular", help="the `type` of the LS2 role the slot writes")
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
    print(remote_script("/tmp/ambient-slot-<time>", data_dir(), a.scenarios.split(","), a.secs))
    return 0


if __name__ == "__main__":
    sys.exit(main())
