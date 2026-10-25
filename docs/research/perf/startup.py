#!/usr/bin/env python3
"""Startup A/B on the set: Home's time to first present after a compositor restart, and the worst
frame of the first D-pad sweep, with the warm-up off and on in one session.

    startup.py [--runs N] [--sweep-at S[,S...]] [--tag T] [--json OUT]

Each run restarts the compositor (`lumatv restart`: QML cache kept, the start every boot after the
first sees), watches the panel from blank to first present with 320x180 captures, waits until
S seconds after that first present, then runs the D-pad workload of the gate (8 x RIGHT, 8 x LEFT,
UP, DOWN at 350 ms) in a full trace. The conditions alternate off, on, on, off, ... so drift
cancels. The warm-up is switched by theme.json `warmup.enabled`, read at compositor start.

Prints, per run: first present after the restart call, the warm-up's own summary line from the
compositor log, and for the sweep the animated frames over 20 ms inside the animation, the p90 of
the animated render time and the render time of the first frames after the first key (where the
cost of anything drawn for the first time lands). The window's worst gap is not reported: idle
Home's 10 updates a second make it 100 ms by construction.
Home must be in front and the screen on when it starts. The set ends with the warm-up on.
"""
import argparse
import json
import os
import re
import sys
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import gate  # noqa: E402

TOOL = '/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv'
CAP = ("luna-send -n 1 luna://com.webos.service.capture/executeOneShot "
       "'{\"path\":\"/tmp/startup-probe.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":320,\"height\":180}'")
BLANK_BYTES = 5000
PRESENT_BYTES = 20000
WARMUP_LINE = re.compile(r'\[Warmup\] (.*)')


def panel_bytes(rig):
    _, out, _ = rig.set.sh(CAP, timeout=20, pty=True)
    m = re.search(r'"writtenBytes"\s*:\s*(\d+)', out)
    return int(m.group(1)) if m else 0


class Probe(threading.Thread):
    """Blank to first present on its own SSH session, so the restart call is not slowed."""

    def __init__(self, rig_factory):
        super().__init__(daemon=True)
        self.rig = rig_factory()
        self.t_blank = None
        self.t_present = None

    def run(self):
        t0 = time.time()
        while time.time() - t0 < 40 and panel_bytes(self.rig) > BLANK_BYTES:
            time.sleep(0.05)
        self.t_blank = time.time()
        while time.time() - t0 < 90:
            if panel_bytes(self.rig) > PRESENT_BYTES:
                self.t_present = time.time()
                return
            time.sleep(0.1)


def one_run(rig, warm, sweep_at, tag):
    rig.sh(f"{TOOL} theme set '{json.dumps({'warmup': {'enabled': warm}})}'")
    log = gate.LogWatch(rig)
    mark = log.mark()
    probe = Probe(gate.Rig)
    t_call = time.time()
    probe.start()
    rc, out, err = rig.set.sh(f'{TOOL} restart', timeout=180, pty=True)
    probe.join(100)
    res = {'warmup': warm, 'sweep_at_s': sweep_at, 'restart_rc': rc}
    if probe.t_present is None:
        res['error'] = 'the panel never presented after the restart'
        return res
    res['blank_after_call_s'] = round(probe.t_blank - t_call, 2)
    res['first_present_after_call_s'] = round(probe.t_present - t_call, 2)
    time.sleep(max(0.0, sweep_at - (time.time() - probe.t_present)))
    pid = rig.compositor_pid()
    rig.remember_compositor(pid)
    name = f'startup-{tag}-{"on" if warm else "off"}-{int(time.time())}'
    rig.capture(name, 'full')
    try:
        rig.sh("python3 /tmp/kb.py @sweep RIGHTx8@0.35 LEFTx8@0.35 UP s0.5 DOWN s1", timeout=60)
    finally:
        rig.stop(name)
    report = rig.analyse(name)
    w = next((x for x in report['windows'] if x['name'] == 'sweep'), None)
    if w:
        res['sweep'] = {k: w.get(k) for k in ('frames', 'anim_frames', 'over20', 'worst_ms', 'dropped', 'dropped_in_motion', 'anim_render_ms',
                                              'burst_first_render_ms', 'first_frame_ms')}
    lines, _ = log.since(mark)
    res['warmup_log'] = [m.group(1) for m in (WARMUP_LINE.search(ln) for ln in lines) if m]
    res['errors'] = gate.log_failures(lines)[0][:5]
    return res


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--runs', type=int, default=2, help='runs per condition (default 2)')
    ap.add_argument('--sweep-at', default='10', help='seconds after first present for the sweep, comma-separated')
    ap.add_argument('--tag', default=time.strftime('%m%d-%H%M'))
    ap.add_argument('--json')
    a = ap.parse_args(argv)
    rig = gate.Rig()
    rig.push_tools()
    results = []
    try:
        for at in (float(x) for x in a.sweep_at.split(',')):
            for i in range(a.runs):
                order = (False, True) if i % 2 == 0 else (True, False)
                for warm in order:
                    r = one_run(rig, warm, at, a.tag)
                    results.append(r)
                    print(json.dumps(r), flush=True)
    finally:
        rig.sh(f"{TOOL} theme set '{json.dumps({'warmup': {'enabled': True}})}'")
    if a.json:
        with open(a.json, 'w') as f:
            json.dump(results, f, indent=1)
    for warm in (False, True):
        rows = [r for r in results if r['warmup'] == warm and 'sweep' in r]
        if rows:
            print('warmup', 'on ' if warm else 'off',
                  'first present %s s' % sorted(r['first_present_after_call_s'] for r in rows),
                  'drops in motion %s' % sorted(r['sweep']['dropped_in_motion'] for r in rows),
                  'render p90 %s ms' % sorted(r['sweep']['anim_render_ms']['p90'] for r in rows if r['sweep']['anim_render_ms']),
                  'first frames max %s ms' % sorted(max(r['sweep']['burst_first_render_ms'] or [0]) for r in rows))
    return 0


if __name__ == '__main__':
    sys.exit(main())
