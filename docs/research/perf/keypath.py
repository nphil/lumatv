#!/usr/bin/env python3
"""From a key (or any marker) to the first frame: where the time goes, from a capture.sh `full` trace.

    keypath.py TRACE [--marker NAME] [--table] [--upto MS] [--cycle N]

Without --table, one marker's timeline: the compositor's main thread per 5 ms (how long it ran), the
tasks of other processes that ran while the key was in flight, the wakeups of the compositor's own
threads, the GPU job, the page flip and the presented frames. With --table, one row per window
(`P:NAME` marker) of the trace:

  first      first presented frame after the marker
  response   first presented frame that starts an animation run (the next frame follows within
             20 ms) or, when nothing animates, the first frame: an idle tick of the wallpaper
             (10 a second) or the clock can come before the response and is skipped by the run rule
  busy       milliseconds the compositor's main thread ran between the marker and the response's GPU job
  waits      times it was woken again before that job: each is a message arriving (luna reply, a
             timer, a thread) that the next step of the open waited for
  gpu        offset of the response's first GPU job (the render), flip: the page-flip commit
  noise      CPU the capture's own sampler and other processes' forks used in the window

The marker is where kb.py wrote it: at the press for a surface that acts on the press, at the
release for one that acts on the release (LG's Quick Settings opens on the release of the settings
key), at the spawn of luna-send for a surface a luna call opens (`Q:NAME-sent`, written when the
call returns, splits the spawn and the hub from what the compositor does).

Reads the trace with analyse.parse, so it needs the same events capture.sh `full` arms: sched_switch,
sched_wakeup, drm_vblank_event_delivered, mali_job_slots_event, workqueue_execute_start.
"""
import argparse
import collections
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import analyse  # noqa: E402

COMPOSITOR_COMM = 'surface-manager'
ANIMATION_GAP_MS = 20.0       # the next frame within this: the frame belongs to a running animation
NOISE_COMM = ('capture.sh', 'grep', 'cat', 'tr', 'sed', 'sleep', 'cut', 'awk', 'date', 'python3')


def find_compositor_pid(ev):
    """The compositor's main thread: the pid that ran most under the name surface-manager."""
    count = collections.Counter()
    for t, cpu, name, pid, comm, args in ev:
        if name == 'sched_switch':
            m = analyse.SWITCH.search(args)
            if m and m.group(5) == COMPOSITOR_COMM:
                count[int(m.group(6))] += 1
    return count.most_common(1)[0][0] if count else None


def markers(ev, prefix='P:'):
    return [(e[0], e[5].strip()[len(prefix):]) for e in ev if e[2] == 'tracing_mark_write' and e[5].strip().startswith(prefix)]


def frame_times(ev):
    return [e[0] for e in ev if e[2] == 'drm_vblank_event_delivered']


def response_frame(frames, t0, gap_ms=ANIMATION_GAP_MS):
    """(first frame, response frame) after t0, as times. The response is the first frame followed by
    another within gap_ms (an animation has begun), else the first frame."""
    after = [f for f in frames if f >= t0]
    if not after:
        return None, None
    for a, b in zip(after, after[1:]):
        if (b - a) * 1000.0 <= gap_ms:
            return after[0], a
    return after[0], after[0]


def run_intervals(ev, a, b):
    """{(pid, comm): [(t0, t1)]}: when each task ran within [a, b], from sched_switch."""
    cur, out = {}, collections.defaultdict(list)
    for t, cpu, name, pid, comm, args in ev:
        if name != 'sched_switch':
            continue
        m = analyse.SWITCH.search(args)
        if not m:
            continue
        _pc, ppid, _pp, _st, nc, npid, _np = m.groups()
        if cpu in cur:
            s, p, c = cur[cpu]
            if p != 0 and t >= a and s <= b:
                out[(p, c)].append((max(s, a), min(t, b)))
        cur[cpu] = (t, int(npid), nc)
    return out


def overlap(intervals, a, b):
    return sum(max(0.0, min(y, b) - max(x, a)) for x, y in intervals) * 1000.0


def analyse_window(ev, t0, upto_s, sm_pid, frames):
    first, resp = response_frame(frames, t0)
    out = {'t0': t0, 'first_ms': None, 'response_ms': None}
    if first is None:
        return out
    out['first_ms'] = (first - t0) * 1000.0
    out['response_ms'] = (resp - t0) * 1000.0
    end = t0 + upto_s
    jobs = [e[0] for e in ev if t0 <= e[0] <= end and e[2] == 'mali_job_slots_event' and 'tgid=%d' % sm_pid in e[5] and 'event=16842752' in e[5]]
    # the render that was presented at `resp`: the last job start before it
    job = max([j for j in jobs if j <= resp], default=None)
    out['gpu_ms'] = (job - t0) * 1000.0 if job is not None else None
    flips = [e[0] for e in ev if t0 <= e[0] <= resp and e[2] == 'workqueue_execute_start' and 'commit_work' in e[5]]
    out['flip_ms'] = (flips[-1] - t0) * 1000.0 if flips else None
    stop = job if job is not None else resp
    runs = run_intervals(ev, t0, stop)
    main = runs.get((sm_pid, COMPOSITOR_COMM), [])
    out['busy_ms'] = overlap(main, t0, stop)
    wakes = [e[0] for e in ev if t0 <= e[0] <= stop and e[2] == 'sched_wakeup' and ('pid=%d ' % sm_pid) in e[5] + ' ']
    out['waits'] = len(wakes)
    noise = 0.0
    others = {}
    for (p, c), iv in runs.items():
        if p == sm_pid:
            continue
        ms = overlap(iv, t0, stop)
        if c.startswith(NOISE_COMM):
            noise += ms
        elif ms >= 0.4:
            others[c] = others.get(c, 0.0) + ms
    out['noise_ms'] = noise
    out['others'] = sorted(others.items(), key=lambda kv: -kv[1])[:6]
    return out


def format_table(rows):
    head = '%-26s %8s %9s %8s %5s %7s %7s %7s  %s' % ('window', 'first', 'response', 'busy', 'waits', 'gpu', 'flip', 'noise', 'other tasks that ran (ms)')
    lines = [head]
    for name, w in rows:
        if w['first_ms'] is None:
            lines.append('%-26s %8s' % (name, 'no frame'))
            continue
        f = lambda v: '%.1f' % v if v is not None else '-'
        others = ' '.join('%s %.0f' % (c[:14], ms) for c, ms in w['others'])
        lines.append('%-26s %8s %9s %8s %5d %7s %7s %7s  %s' % (name, f(w['first_ms']), f(w['response_ms']), f(w['busy_ms']), w['waits'],
                                                              f(w['gpu_ms']), f(w['flip_ms']), f(w['noise_ms']), others))
    return '\n'.join(lines)


def timeline(ev, t0, upto_s, sm_pid, frames, bucket_ms=5):
    nb = int(upto_s * 1000 / bucket_ms)
    runs = run_intervals(ev, t0, t0 + upto_s)
    main = [0.0] * nb
    for x, y in runs.get((sm_pid, COMPOSITOR_COMM), []):
        i = int((x - t0) * 1000 / bucket_ms)
        t = x
        while t < y - 1e-9 and i < nb:
            edge = t0 + (i + 1) * bucket_ms / 1000.0
            main[i] += (min(y, edge) - t) * 1000.0
            t = min(y, edge)
            i += 1
    lines = ['%-14s%s' % ('ms', ' '.join('%4d' % (i * bucket_ms) for i in range(nb))),
             '%-14s%s' % ('main thread', ' '.join('%4.1f' % v for v in main))]
    for (p, c), iv in sorted(runs.items(), key=lambda kv: -sum(y - x for x, y in kv[1])):
        if p == sm_pid:
            continue
        tot = overlap(iv, t0, t0 + upto_s)
        if tot < 1.5:
            continue
        row = [0.0] * nb
        for x, y in iv:
            i = int((x - t0) * 1000 / bucket_ms)
            t = x
            while t < y - 1e-9 and i < nb:
                edge = t0 + (i + 1) * bucket_ms / 1000.0
                row[i] += (min(y, edge) - t) * 1000.0
                t = min(y, edge)
                i += 1
        lines.append('%-14s%s  [%.1f]' % (c[:13], ' '.join('%4.1f' % v for v in row), tot))
    lines.append('')
    end = t0 + upto_s
    for e in ev:
        if not (t0 <= e[0] <= end):
            continue
        t, cpu, name, pid, comm, args = e
        ms = (t - t0) * 1000.0
        if name == 'sched_wakeup':
            m = analyse.WAKEUP.search(args)
            if m and int(m.group(2)) == sm_pid:
                lines.append('%7.1f ms  main thread woken by %s-%d' % (ms, comm, pid))
        elif name == 'mali_job_slots_event' and 'tgid=%d' % sm_pid in args and 'event=16842752' in args:
            lines.append('%7.1f ms  GPU job' % ms)
        elif name == 'workqueue_execute_start' and 'commit_work' in args:
            lines.append('%7.1f ms  page flip committed' % ms)
        elif name == 'drm_vblank_event_delivered':
            lines.append('%7.1f ms  ---- frame presented' % ms)
        elif name == 'tracing_mark_write' and args.strip().startswith('Q:'):
            lines.append('%7.1f ms  marker %s' % (ms, args.strip()))
    return '\n'.join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('trace')
    ap.add_argument('--marker', help='window name (the text after P:), e.g. quick-settings-open')
    ap.add_argument('--cycle', type=int, default=0, help='which occurrence of the marker (default the first)')
    ap.add_argument('--table', action='store_true', help='one row per window of the trace')
    ap.add_argument('--upto', type=float, default=200.0, help='ms after the marker to look at (default 200)')
    a = ap.parse_args(argv)
    ev = analyse.parse(a.trace)
    if not ev:
        sys.exit('empty trace')
    sm = find_compositor_pid(ev)
    if sm is None:
        sys.exit('no surface-manager in the trace: capture.sh full records sched_switch')
    frames = frame_times(ev)
    marks = [m for m in markers(ev) if not a.marker or m[1] == a.marker]
    if a.table or not a.marker:
        rows = [(n, analyse_window(ev, t, a.upto / 1000.0, sm, frames)) for t, n in marks]
        print(format_table(rows))
        return 0
    if a.cycle >= len(marks):
        sys.exit('only %d windows named %s' % (len(marks), a.marker))
    t0 = marks[a.cycle][0]
    w = analyse_window(ev, t0, a.upto / 1000.0, sm, frames)
    print('%s #%d at %.3f: first frame +%s ms, response +%s ms' % (a.marker, a.cycle, t0,
          '%.1f' % w['first_ms'] if w['first_ms'] is not None else '-', '%.1f' % w['response_ms'] if w['response_ms'] is not None else '-'))
    print(timeline(ev, t0, a.upto / 1000.0, sm, frames))
    return 0


if __name__ == '__main__':
    sys.exit(main())
