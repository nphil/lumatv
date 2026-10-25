#!/usr/bin/env python3
"""The next C4 slot in one run (about 25 min, exclusive use of the set, from the repository root):

    python3 compositor/tools/tv_next_slot.py [--skip-gate] [--gate-args "--skip netflix"] [--no-standby]

1. Preflight: the set answers and is woken (tv_gate.wake), nothing of ours runs, LG's compositor is live, Picture
   Mode is filmMaker. The installed tools/lumatv must know LumaTV's compositor (its Home stub guard, `comp_live`),
   or the stub guard gives LG's Home back 30 s into every switch: without it the stub is turned off for the session
   and on again at the end.
2. Deploy the current bundle, `cutover install` (records the firmware: whether this build is on verified.list),
   `cutover probe` (12 names allowed).
3. Receiver muted, `cutover trial 120` (a session: any firmware, this boot only); the trial's evidence is fetched and
   checked: SAM's foreground subscription reached the compositor, the 7 s client is named, no protocol errors, the panel
   picture (`panel.png`, upright), and the trial's verdict (trial_problems): no fallback, nothing over 20 ms in the
   compositor's log, the watchdog reading its inputs.
4. The gate (tools/tv_gate.py: firmware, homeheld, enable --session, live, volume, x1, netflix, hdmi, evidence,
   standby/wake, homekey, wd-hang, wd-crash, escape, then disable), without --enable-after: the surfaces LumaTV draws
   do not exist under its compositor yet.
5. End state: LG's compositor live, the enabled flag absent, Home stub as it was, receiver unmuted, Active Standby.

Everything lands in docs/research/compositor/slot-<date>-<HHMM>/ (summary.txt, trial/, gate/)."""
import argparse, os, re, subprocess, sys, tarfile, time

here = os.path.dirname(os.path.abspath(__file__))
repo = os.path.abspath(os.path.join(here, "..", ".."))
sys.path.insert(0, here)
sys.path.insert(0, os.path.join(repo, "docs", "research", "arch"))
import tv
import tv_gate

TOOL = "/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv"
F = os.path.join(here, "tv-first-light.sh")
# The frame rule (PLAN, C4): nothing the compositor logs over this many milliseconds, which is a frame the panel missed.
GAP_LIMIT_MS = 20.0
LIMIT_LINE = re.compile(r"loop: ([0-9.]+) ms in one pass|frame gap ([0-9.]+) ms on surface|page-flip stamp ([0-9.]+) ms after the last")


def sh(*args, timeout=900):
    p = subprocess.run(["sh", F, *args], capture_output=True, text=True, timeout=timeout)
    out = p.stdout + (("\n[stderr]\n" + p.stderr) if p.stderr.strip() else "")
    print(out)
    return p.returncode, out


def fetch(remote_dir, local_dir):
    tv.tv("cd %s/.. && tar czf /tmp/lumatv-slot.tgz %s" % (remote_dir, os.path.basename(remote_dir)))
    os.makedirs(local_dir, exist_ok=True)
    tv.get("/tmp/lumatv-slot.tgz", os.path.join(local_dir, "x.tgz"))
    with tarfile.open(os.path.join(local_dir, "x.tgz")) as t:
        t.extractall(local_dir)
    os.remove(os.path.join(local_dir, "x.tgz"))


def read(path):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def run_lines(sm_log):
    """The lines of the supervisor's log that belong to its last start: the file is appended to by every start."""
    lines = sm_log.splitlines()
    last = 0
    for i, l in enumerate(lines):
        if "] start: " in l:
            last = i
    return lines[last:]


def fallbacks_of_run(sm_log):
    return [l for l in run_lines(sm_log) if "fallback:" in l]


def over_limit(comp_log):
    """The compositor's own record of frames the panel missed: a loop pass, a client's frame gap or a page-flip interval
    over GAP_LIMIT_MS (the lines tools/tv-gate.sh counts the same way)."""
    found = []
    for l in comp_log.splitlines():
        m = LIMIT_LINE.search(l)
        if m and float(next(g for g in m.groups() if g)) > GAP_LIMIT_MS:
            found.append(l.split("] ", 1)[-1][:100])
    return found


def trial_problems(sm_log, comp_log):
    """What the trial's logs say against it: none of these may be there. A fallback of the run (the supervisor handing the
    screen back), a protocol error, a loop pass or frame gap or page-flip interval over 20 ms, the supervisor counting a
    run that never settled, or a watchdog that never read its inputs (no `progress rules on` line)."""
    run = run_lines(sm_log)
    problems = []
    fb = fallbacks_of_run(sm_log)
    if fb:
        problems.append("%d fallback(s): %s" % (len(fb), fb[0].split("] ", 1)[-1][:100]))
    errors = comp_log.count("message too short") + comp_log.count("error in client communication")
    if errors:
        problems.append("%d protocol error(s)" % errors)
    over = over_limit(comp_log)
    if over:
        problems.append("%d frame(s) over %g ms: %s" % (len(over), GAP_LIMIT_MS, over[0]))
    if any("never settled or ended cleanly" in l for l in run):
        problems.append("the supervisor counted a run that never settled")
    if not any("watchdog: progress rules on (frozen output watched, empty panel watched)" in l for l in run):
        problems.append("the watchdog never read the compositor's counters and windows")
    return problems


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--skip-gate", action="store_true")
    ap.add_argument("--gate-args", default="")
    ap.add_argument("--no-standby", action="store_true", help="leave the set on at the end")
    a = ap.parse_args()
    stamp = time.strftime("%Y-%m-%d-%H%M")
    out = os.path.join(repo, "docs", "research", "compositor", "slot-" + stamp)
    os.makedirs(out, exist_ok=True)
    summary = []

    def note(line):
        print(">>", line)
        summary.append(line)

    # 1. Preflight.
    if not tv_gate.wake():
        sys.exit("the set does not answer; nothing was changed")
    # The bracket keeps the pattern from matching the shell that runs this very command line.
    pre = tv.tv("pgrep -fl 'bin/lumatv-[c]ompositor|bin/lumatv-[s]m' || echo none; grep -c ' /usr/bin/surface-manager ' /proc/self/mountinfo; "
                "readlink /proc/$(pgrep -x surface-manager | head -1)/exe; grep -c comp_live %s 2>/dev/null || echo 0; "
                "%s homestub status 2>&1 | head -3" % (TOOL, TOOL))
    print(pre)
    lines = pre.split("\n")
    if lines[0].strip() != "none" or (len(lines) > 1 and lines[1].strip() != "0"):
        sys.exit("something of LumaTV's compositor is running or mounted; run `cutover.sh status` and `disable` first")
    has_guard = len(lines) > 3 and lines[3].strip().isdigit() and int(lines[3].strip()) > 0
    stub_on = '"state":"stub"' in pre
    note("preflight: guard %s, home stub %s, picture %s" % (has_guard, "on" if stub_on else "off/unknown",
         tv_gate.pty("luna-send -n 1 -f luna://com.webos.settingsservice/getSystemSettings '{\"category\":\"picture\",\"keys\":[\"pictureMode\"]}' | grep pictureMode", 20).strip()))
    stub_off_for_session = stub_on and not has_guard
    if stub_off_for_session:
        print(tv_gate.pty(TOOL + " homestub off", 120))
        note("home stub off for the session (the installed tool has no comp_live guard)")

    try:
        # 2. Deploy, install, probe.
        rc, o = sh("deploy", timeout=900)
        note("deploy: exit %d" % rc)
        rc, o = sh("cutover", "install")
        note("install: " + next((l for l in o.splitlines() if l.startswith("firmware:")), "no firmware line"))
        rc, o = sh("cutover", "probe")
        note("probe: " + next((l for l in o.splitlines() if l.startswith("probe:")), "no summary line"))
        if "0 refused" not in o:
            raise SystemExit("the hub refused a bus name; no trial")

        # 3. Trial.
        tv_gate.avr_mute(True)
        rc, o = sh("cutover", "trial", "120", timeout=600)
        tv_gate.avr_mute(False)
        tdir = os.path.join(out, "trial")
        fetch("/tmp/lumatv/out/trial", out)
        sm = read(os.path.join(tdir, "sm.log"))
        comp = read(os.path.join(tdir, "compositor.log"))
        note("trial: " + next((l for l in o.splitlines() if l.startswith("trial evidence")), "no evidence line (fell back?)"))
        note("trial: SAM subscribed to getForegroundAppInfo: %s" % ("yes" if "getForegroundAppInfo from com.webos.applicationManager" in sm else "NO"))
        note("trial: calls queued before the compositor connected: %s" % next((l.split("] ", 1)[-1] for l in sm.splitlines() if "arrived before the compositor" in l), "none"))
        gone = [l.split("] ", 1)[-1] for l in comp.splitlines() if "client disconnected" in l]
        note("trial: %d client disconnects; the first five: %s" % (len(gone), " | ".join(gone[:5])))
        note("trial: protocol errors %d, loop stalls %d, frame gaps %d, fallbacks %d" % (
            comp.count("message too short") + comp.count("error in client communication"), comp.count("loop:"),
            comp.count("frame gap"), len(fallbacks_of_run(sm))))
        problems = trial_problems(sm, comp)
        note("trial verdict: %s" % ("PASS: no fallback, no protocol error, nothing over %g ms in the compositor's log, the watchdog reading its inputs" % GAP_LIMIT_MS if not problems else "FAIL: " + "; ".join(problems)))
        note("trial: SAM's foreground: %s" % read(os.path.join(tdir, "sam-foreground.json")).replace("\n", " ")[:200])

        # 4. The gate.
        if not a.skip_gate:
            cmd = [sys.executable, os.path.join(here, "tv_gate.py")] + a.gate_args.split()
            p = subprocess.run(cmd, capture_output=True, text=True, timeout=1800)
            print(p.stdout[-6000:], p.stderr[-2000:])
            for l in p.stdout.splitlines():
                if l.startswith("GATE"):
                    note(l)
            if os.path.isdir("/tmp/lumatv-tv-out/out/gate"):
                subprocess.run(["cp", "-r", "/tmp/lumatv-tv-out/out/gate", os.path.join(out, "gate")])
    finally:
        # 5. End state.
        print(tv_gate.pty("sh /tmp/lumatv/cutover.sh status", 60))
        if stub_off_for_session:
            print(tv_gate.pty(TOOL + " homestub on", 120))
            note("home stub back on")
        tv_gate.avr_mute(False)
        if not a.no_standby:
            tv_gate.pty("luna-send -n 1 -f luna://com.webos.service.tvpower/power/powerOff '{\"reason\":\"remoteKey\"}'", 20)
            note("set put in Active Standby")
        with open(os.path.join(out, "summary.txt"), "w") as f:
            f.write("# C4 slot %s\n\n" % stamp + "\n".join(summary) + "\n")
        print("wrote", os.path.relpath(os.path.join(out, "summary.txt"), repo))


if __name__ == "__main__":
    main()
