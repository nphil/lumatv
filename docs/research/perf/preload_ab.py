#!/usr/bin/env python3
"""A/B of the preload trim on the Home return, in one session.

    preload_ab.py [--rounds N] [--workload apphome|hog] [--settle S] [--tag T] [--json OUT]

Each round is one state of `lumatv preload`: `on` (YouTube, LG Channels and the browser dropped from
the preload list) or `off` (LG's list restored). The states alternate on, off, off, on, ... so drift
cancels. After a switch the set settles for S seconds (the preload manager loads or drops apps in the
background), then memory is read and the workload runs in a full trace:

  apphome  LG's Settings app launched, then Home (the `home` window is the return)
  hog      the browser in front, 450 MB of anonymous memory touched and released, then Home

Per round it prints MemAvailable, swap in use, how many apps the app manager reports running, and for
the Home return the frames over 20 ms, the worst gap and the animated render p90. A gap that stays
with fewer preloads is page-in cost, not preload contention. The set is left with the preload as it
was found.
"""
import argparse
import json
import re
import sys
import time

import gate
import run

TOOL = '/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv'


def meminfo(rig):
    _, out, _ = rig.sh("grep -E '^(MemAvailable|MemFree|SwapTotal|SwapFree|CmaFree)' /proc/meminfo")
    vals = {k: int(v) for k, v in re.findall(r'(\w+):\s+(\d+) kB', out)}
    _, run_out, _ = rig.sh("luna-send -n 1 luna://com.webos.applicationManager/running '{}'", 40)
    running = re.findall(r'"id"\s*:\s*"([^"]+)"', run_out)
    return {'mem_available_mb': vals.get('MemAvailable', 0) // 1024, 'mem_free_mb': vals.get('MemFree', 0) // 1024,
            'swap_used_mb': (vals.get('SwapTotal', 0) - vals.get('SwapFree', 0)) // 1024,
            'cma_free_mb': vals.get('CmaFree', 0) // 1024, 'running': sorted(running)}


def state_now(rig):
    _, out, _ = rig.sh(f'{TOOL} preload status', 60)
    try:
        return bool(json.loads(out.strip().splitlines()[-1]).get('on'))
    except (ValueError, IndexError):
        return False


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--rounds', type=int, default=4, help='rounds in all, alternating on and off (default 4)')
    ap.add_argument('--workload', default='apphome', choices=('apphome', 'hog'))
    ap.add_argument('--settle', type=int, default=45, help='seconds after a switch before measuring (default 45)')
    ap.add_argument('--tag', default=time.strftime('%m%d-%H%M'))
    ap.add_argument('--json')
    a = ap.parse_args(argv)
    rig = gate.Rig()
    rig.push_tools()
    s = rig.set
    was_on = state_now(rig)
    results = []
    order = [True, False, False, True] * ((a.rounds + 3) // 4)
    try:
        for i, want in enumerate(order[:a.rounds]):
            rc, out, _ = rig.sh(f'{TOOL} preload {"on" if want else "off"}', 90)
            time.sleep(a.settle)
            mem = meminfo(rig)
            name = f'preload-{a.tag}-{"on" if want else "off"}-{i}'
            s.capture(name, 'full')
            time.sleep(1)
            try:
                run.WORKLOADS[a.workload](s)
            finally:
                s.stop(name)
            rep = run.analyse(name, s.sm_pid, 17.5)
            w = next((x for x in rep['windows'] if x['name'] in ('home',)), None)
            row = {'round': i, 'trimmed': want, 'switch': out.strip()[-120:], **mem}
            if w:
                row.update({k: w.get(k) for k in ('frames', 'over20', 'worst_ms', 'dropped_in_motion', 'anim_render_ms', 'first_frame_ms', 'burst_end_ms')})
            results.append(row)
            print(json.dumps(row), flush=True)
    finally:
        rig.sh(f'{TOOL} preload {"on" if was_on else "off"}', 90)
    if a.json:
        with open(a.json, 'w') as f:
            json.dump(results, f, indent=1)
    for trimmed in (True, False):
        rows = [r for r in results if r['trimmed'] == trimmed]
        if rows:
            print('trimmed' if trimmed else 'LG list', 'available MB', [r['mem_available_mb'] for r in rows], 'running apps', [len(r['running']) for r in rows],
                  'over 20 ms', [r.get('over20') for r in rows], 'worst ms', [r.get('worst_ms') for r in rows])
    return 0


if __name__ == '__main__':
    sys.exit(main())
