#!/usr/bin/env python3
"""The UI engine's frame cost under LumaTV's compositor, measured on the set in one maintenance window (about 5 min with the
build and deploy, exclusive use of the set, from the repository root):

    python3 compositor/tools/tv_ui_measure.py [--no-deploy] [--no-standby] [--gate]

1. Preflight: the set answers and is woken (tv_gate.wake), nothing of ours runs, LG's compositor is live.
2. Build the bundle with the engine host (LUMATV_FEATURES=ui) and deploy it; the set's /tmp/lumatv/FEATURES must say `ui`.
3. tools/tv-ui-measure.sh run on the set (LG's compositor stopped for about 2.5 min, started again by the script): the
   demo surface on KMS in four phases (idle10, anim60, volume, direct), the GPU's busy/idle counters sampled every second,
   the compositor's status and a capture of the page per phase.
4. The figures: for each phase the GPU busy share over the phase (mali0 dvfs_utilization), from it the GPU ms per frame at the
   phase's frame rate, the compositor's CPU share, the summary's "ui frames ... px/frame ... cpu ... us/frame" and "direct N".
   The idle gate: idle10's GPU ms per frame under 1 ms (the RustUi spike's prerequisite). anim60 and volume are reported;
   direct must show direct frames (the plane took x2cli's buffer while the bar was down) and no GL error.
5. End state: LG's compositor live (the on-set script restores it), Active Standby unless --no-standby.

--gate would run perf/gate.py (the per-surface frame gate) under LumaTV's compositor; that needs LumaTV's QML surfaces as a
client of this compositor, which do not exist yet ("Surfaces under LumaTV's compositor" in compositor-c2-result.md), so it
reports the blocker and runs nothing.

Everything lands in docs/research/compositor/ui-measure-<date>-<HHMM>/ (summary.txt, the set's out/ui/)."""
import argparse, json, os, re, subprocess, sys, tarfile, time

here = os.path.dirname(os.path.abspath(__file__))
repo = os.path.abspath(os.path.join(here, "..", ".."))
sys.path.insert(0, here)
sys.path.insert(0, os.path.join(repo, "docs", "research", "arch"))

# The phases of tools/tv-ui-measure.sh and the frame rate each draws at (the demo's ticker commits at that rate; direct's
# page flips follow x2cli at the refresh).
PHASES = [("idle10", 10.0), ("anim60", 60.0), ("volume", 10.0), ("direct", 60.0)]
IDLE_GATE_MS = 1.0
# The first seconds of a phase are the start (shader warm-up, the first whole frame) and are left out of the shares.
WARM_S = 3.0
CLK_TCK = 100.0


def parse_gpu(text):
    """`PHASE.gpu` lines: `UPTIME busy_time: B idle_time: I cpu=TICKS`. Returns (gpu busy share, compositor cpu share, seconds)
    over the samples after the warm-up, or None without enough samples."""
    rows = []
    for line in text.splitlines():
        m = re.match(r"\s*([\d.]+)\s+busy_time:\s*(\d+)\s+idle_time:\s*(\d+)\s*cpu=(\d*)", line)
        if m:
            rows.append((float(m.group(1)), int(m.group(2)), int(m.group(3)), int(m.group(4) or 0)))
    if len(rows) < 3:
        return None
    start = rows[0][0] + WARM_S
    rows = [r for r in rows if r[0] >= start] or rows
    if len(rows) < 2:
        return None
    first, last = rows[0], rows[-1]
    busy, idle = last[1] - first[1], last[2] - first[2]
    seconds = last[0] - first[0]
    gpu = busy / float(busy + idle) if busy + idle > 0 else 0.0
    cpu = (last[3] - first[3]) / CLK_TCK / seconds if seconds > 0 else 0.0
    return gpu, cpu, seconds


def summary_of(status_text):
    try:
        return json.loads(status_text).get("summary", "")
    except ValueError:
        return ""


def field(summary, name):
    m = re.search(r"\b%s (\d+)" % re.escape(name), summary)
    return int(m.group(1)) if m else None


def report(out_dir):
    """The figures of a fetched run (also usable on its own: `tv_ui_measure.py --report DIR`)."""
    lines = []
    verdicts = []
    for name, hz in PHASES:
        gpu = parse_gpu(read(os.path.join(out_dir, name + ".gpu")))
        summary = summary_of(read(os.path.join(out_dir, name + ".status.json")))
        if gpu is None:
            lines.append("%s: no GPU samples" % name)
            verdicts.append(False)
            continue
        share, cpu, seconds = gpu
        ms_per_frame = share * 1000.0 / hz
        line = "%s: GPU busy %.2f %% over %.0f s = %.2f ms per frame at %g fps, compositor CPU %.1f %%" % (name, share * 100, seconds, ms_per_frame, hz, cpu * 100)
        ui = re.search(r"ui frames .*?gl errors \d+", summary)
        if ui:
            line += "; " + ui.group(0)
        direct = field(summary, "direct")
        if direct is not None:
            line += "; direct frames %d" % direct
        gl = field(summary, "gl errors")
        if name == "idle10":
            ok = ms_per_frame < IDLE_GATE_MS
            line += "; IDLE GATE %s (under %.0f ms)" % ("PASS" if ok else "FAIL", IDLE_GATE_MS)
            verdicts.append(ok)
        if name == "direct":
            ok = bool(direct) and (gl or 0) == 0
            line += "; DIRECT %s" % ("PASS" if ok else "FAIL (no direct frame, or a GL error)")
            verdicts.append(ok)
        if gl:
            line += "; GL ERRORS %d" % gl
            verdicts.append(False)
        lines.append(line)
    return lines, all(verdicts) and bool(verdicts)


def read(path):
    try:
        with open(path, errors="replace") as f:
            return f.read()
    except OSError:
        return ""


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--no-deploy", action="store_true", help="the set already has a bundle built with LUMATV_FEATURES=ui")
    ap.add_argument("--no-standby", action="store_true", help="leave the set on at the end")
    ap.add_argument("--gate", action="store_true", help="perf/gate.py under LumaTV's compositor (not runnable yet; reports why)")
    ap.add_argument("--report", metavar="DIR", help="only the figures of a fetched run")
    a = ap.parse_args()
    if a.report:
        lines, ok = report(a.report)
        print("\n".join(lines))
        sys.exit(0 if ok else 1)

    import tv
    import tv_gate

    stamp = time.strftime("%Y-%m-%d-%H%M")
    out = os.path.join(repo, "docs", "research", "compositor", "ui-measure-" + stamp)
    os.makedirs(out, exist_ok=True)
    summary = []

    def note(line):
        print(">>", line)
        summary.append(line)

    # 1. Preflight.
    if not tv_gate.wake():
        sys.exit("the set does not answer; nothing was changed")
    pre = tv.tv("pgrep -fl 'bin/lumatv-[c]ompositor|bin/lumatv-[s]m|bin/x2[c]li' || echo none; systemctl is-active surface-manager-daemon")
    print(pre)
    lines = pre.split("\n")
    if lines[0].strip() != "none":
        sys.exit("something of LumaTV's runs on the set; stop it first")
    if len(lines) < 2 or lines[1].strip() != "active":
        sys.exit("LG's compositor is not active; run kms-window.sh restore / recover-home.sh first")
    note("preflight: LG's compositor live, nothing of ours running")

    try:
        # 2. Build with the engine host, deploy.
        if not a.no_deploy:
            env = dict(os.environ, LUMATV_FEATURES="ui")
            p = subprocess.run(["sh", os.path.join(here, "tv-first-light.sh"), "deploy"], capture_output=True, text=True, timeout=1800, env=env)
            print(p.stdout[-3000:], p.stderr[-2000:])
            note("deploy (features ui): exit %d" % p.returncode)
            if p.returncode != 0:
                raise SystemExit("the deploy failed")
        features = tv.tv("cat /tmp/lumatv/FEATURES 2>/dev/null").strip()
        if "ui" not in features:
            raise SystemExit("the set's bundle has no ui feature (FEATURES=%r); build with LUMATV_FEATURES=ui" % features)

        # 3. The measurement window on the set.
        tv.put(os.path.join(here, "tv-ui-measure.sh"), "/tmp/lumatv/ui-measure.sh")
        o = tv_gate.pty("sh /tmp/lumatv/ui-measure.sh run", 600)
        print(o)
        note("measure: %s" % ("restored LG's compositor" if "restore: surface-manager-daemon started (active)" in o else "CHECK the end state (restore line missing)"))

        # 4. Fetch and read.
        tv.tv("cd /tmp/lumatv/out && tar czf /tmp/lumatv-ui.tgz ui")
        tv.get("/tmp/lumatv-ui.tgz", os.path.join(out, "x.tgz"))
        with tarfile.open(os.path.join(out, "x.tgz")) as t:
            t.extractall(out)
        os.remove(os.path.join(out, "x.tgz"))
        lines, ok = report(os.path.join(out, "ui"))
        for l in lines:
            note(l)
        note("RESULT %s" % ("PASS" if ok else "FAIL"))

        # 5. The surface gate under this compositor.
        if a.gate:
            note("GATE: not run; perf/gate.py drives LumaTV's QML surfaces, which have no client under LumaTV's compositor yet "
                 "(compositor-c2-result.md, Surfaces under LumaTV's compositor)")
    finally:
        print(tv_gate.pty("systemctl is-active surface-manager-daemon; pgrep -fl 'bin/lumatv-[c]ompositor' || echo 'nothing of ours'", 30))
        if not a.no_standby:
            tv_gate.pty("luna-send -n 1 -f luna://com.webos.service.tvpower/power/powerOff '{\"reason\":\"remoteKey\"}'", 20)
            note("set put in Active Standby")
        with open(os.path.join(out, "summary.txt"), "w") as f:
            f.write("# UI engine measurement %s\n\n" % stamp + "\n".join(summary) + "\n")
        print("wrote", os.path.relpath(os.path.join(out, "summary.txt"), repo))


if __name__ == "__main__":
    main()
