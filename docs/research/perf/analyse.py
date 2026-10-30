#!/usr/bin/env python3
"""Frame pacing and gap attribution from a capture.sh ftrace dump.

    analyse.py TRACE [--sm PID] [--window A B] [--gap MS] [--json OUT]

A presented frame is a drm_vblank_event_delivered event. Windows are cut
at tracing_mark_write markers whose text starts with "P:" (phase name);
the frames between two markers belong to the first. Without markers the
whole trace is one window.

For every interval over --gap ms (default 20) the analyser lists what ran
on each CPU, what the compositor's GUI thread was doing (running,
runnable, sleeping), GPU job activity and power transitions, interrupts,
hot-plug, reclaim and CMA events inside the interval.

Every window also reports the burst that starts at its marker (the response
to a key injected right after it, see kb.py @NAME): first_frame_ms, burst_end_ms,
burst_over20 and burst_anim_p90_ms. A burst continues while each frame is
within 50 ms of the one before it; the first is measured from the marker.
"""
import argparse
import bisect
import collections
import json
import re
import sys

import kwait

LINE = re.compile(r'^\s*(.*?)-(\d+)\s+\[(\d{3})\]\s+(\S+)\s+([\d.]+):\s+(\w+):\s?(.*)$')
SWITCH = re.compile(r'prev_comm=(.*?) prev_pid=(\d+) prev_prio=(-?\d+) prev_state=(\S+) ==> next_comm=(.*?) next_pid=(\d+) next_prio=(-?\d+)')
WAKEUP = re.compile(r'comm=(.*?) pid=(\d+) prio=(-?\d+) target_cpu=(\d+)')
KV = re.compile(r'(\w+)=(\S+)')


def parse(path):
    ev = []
    with open(path, errors='replace') as f:
        for line in f:
            m = LINE.match(line)
            if not m:
                continue
            comm, pid, cpu, flags, ts, name, args = m.groups()
            ev.append((float(ts), int(cpu), name, int(pid), comm, args))
    ev.sort(key=lambda e: e[0])
    return ev


class Runs:
    """Per-CPU run intervals from sched_switch, queryable by time."""

    def __init__(self, ncpu=4):
        self.cur = {}        # cpu -> (t, pid, comm)
        self.runs = collections.defaultdict(list)  # cpu -> [(t0, t1, pid, comm)]
        self.starts = {}

    def switch(self, t, cpu, prev_pid, prev_comm, next_pid, next_comm):
        if cpu in self.cur:
            t0, pid, comm = self.cur[cpu]
            self.runs[cpu].append((t0, t, pid, comm))
        self.cur[cpu] = (t, next_pid, next_comm)

    def finish(self, t_end):
        for cpu, (t0, pid, comm) in self.cur.items():
            self.runs[cpu].append((t0, t_end, pid, comm))
        for cpu in self.runs:
            self.runs[cpu].sort()
            self.starts[cpu] = [r[0] for r in self.runs[cpu]]

    def overlap(self, cpu, a, b):
        """[(pid, comm, seconds)] overlapping [a, b] on cpu, largest first."""
        out = collections.Counter()
        names = {}
        rs = self.runs.get(cpu, [])
        i = bisect.bisect_right(self.starts.get(cpu, []), a) - 1
        i = max(i, 0)
        while i < len(rs) and rs[i][0] < b:
            t0, t1, pid, comm = rs[i]
            o = min(t1, b) - max(t0, a)
            if o > 0:
                out[pid] += o
                names[pid] = comm
            i += 1
        return [(pid, names[pid], s) for pid, s in out.most_common()]


BURST_GAP_MS = 50.0
# A frame interval over this is a dropped frame (the `over20`, `dropped` and `burst_over20` counts): 20 ms is a 60 Hz refresh and a
# fifth; set_refresh() scales it (10 ms at 120, the compositor's refresh test mode).
DROP_MS = 20.0
# The refresh that was asked for, Hz: a window whose vblanks say the panel ran at another rate is judged by that rate (`panel_drop_ms`).
ASKED_HZ = 60.0
# How far from the asked refresh the panel's own may be before the window is judged by it.
PANEL_TOLERANCE = 0.08


def set_refresh(hz):
    """The refresh the frames were presented at: the drop threshold becomes a refresh and a fifth."""
    global DROP_MS, ASKED_HZ
    ASKED_HZ = float(hz)
    DROP_MS = round(1.2 * 1000.0 / float(hz), 2)


def panel_period_ms(vblanks, a, b):
    """The panel's refresh period in [a, b] from the vblank events there: the median interval, None with fewer than 12 of them. Round 22: a camera
    popup (a hardware video window) made the TV re-time the panel from 120 Hz to 100 and it stayed there, so that every flip, 10.0 ms after the
    last, was a "dropped frame" against the 10 ms of 120 Hz, a thousand of them an idle screen."""
    i, j = bisect.bisect_left(vblanks, a), bisect.bisect_right(vblanks, b)
    gaps = sorted((y - x) * 1000.0 for x, y in zip(vblanks[i:j], vblanks[i + 1:j]) if y - x < 0.1)
    return gaps[len(gaps) // 2] if len(gaps) >= 12 else None


def panel_drop_ms(vblanks, a, b):
    """(the drop threshold, the panel's Hz) of [a, b]: a refresh and a fifth of what the panel ran at when that is not the asked refresh, else
    [`DROP_MS`], and the Hz measured (None without vblanks)."""
    period = panel_period_ms(vblanks, a, b)
    if period is None:
        return DROP_MS, None
    hz = 1000.0 / period
    if abs(hz - ASKED_HZ) > PANEL_TOLERANCE * ASKED_HZ:
        return round(1.2 * period, 2), round(hz, 1)
    return DROP_MS, round(hz, 1)


# LumaTV's compositor thread wakes for every vblank and every commit: runs of 0.1 ms that draw nothing (apps-grid at 120 Hz: every flip the walk's keys
# left 25-217 ms apart was read as an animated frame that took a hundred milliseconds, its "render" 0.10-0.13 ms). The ring's own frames cost 0.6 ms
# or more (`render 623 us` for the weather's 43 k px), so what the frame chain below reads as a render is a burst of at least 0.3 ms.
LUMATV_MIN_RENDER_S = 0.0003


def renders(runs, lumatv):
    """The (start, end) bursts of the compositor thread that can have rendered a frame: all of LG's, those of 0.3 ms or more of LumaTV's. The wakeups
    stay in the chain as what the render followed (the idle time before it); they are not what it is read from."""
    return [r for r in runs if r[1] - r[0] >= LUMATV_MIN_RENDER_S] if lumatv else runs
# Processes whose GPU work is the composition itself, not an app's own drawing.
COMPOSITION_COMMS = ('surface-manager', 'gal_renderd')
# An app's buffer reaches the screen up to three frames after its GPU job ends.
APP_QUEUE_S = 0.050


def burst(times, start, gap_ms=BURST_GAP_MS, drop_ms=None):
    """The frame burst that begins at start (a marker) from sorted presentation times.

    Frames chain while each is within gap_ms of the one before, the first measured from start,
    so a first frame later than gap_ms leaves the burst empty. Returns first_frame_ms (None when
    no frame follows start), burst_end_ms and over20 (frame-to-frame intervals over 20 ms inside
    the burst; burst_end_ms is None for an empty burst), plus the burst's frame count and
    first and last times. `drop_ms` is the interval over which a frame is dropped (default `DROP_MS`).
    """
    out = {'first_frame_ms': None, 'burst_end_ms': None, 'over20': 0, 'frames': 0, 'first_t': None, 'last_t': None}
    if not times:
        return out
    out['first_frame_ms'] = round((times[0] - start) * 1000.0, 2)
    prev = start
    n = 0
    for t in times:
        if (t - prev) * 1000.0 > gap_ms:
            break
        if n and (t - prev) * 1000.0 > (DROP_MS if drop_ms is None else drop_ms):
            out['over20'] += 1
        prev = t
        n += 1
    if n:
        out['frames'] = n
        out['first_t'], out['last_t'] = times[0], times[n - 1]
        out['burst_end_ms'] = round((times[n - 1] - start) * 1000.0, 2)
    return out


def loop_pid(ev):
    """The compositor's own process id from the trace's `loop N` marker (`capture.sh start`), None when the trace has none."""
    for e in ev:
        if e[2] == 'tracing_mark_write':
            m = re.match(r'loop (\d+)\s*$', e[5].strip())
            if m:
                return int(m.group(1))
    return None


def analyse(path, sm_pid=None, gap_ms=20.0, window=None):
    ev = parse(path)
    if not ev:
        sys.exit("empty trace")
    t_start, t_end = ev[0][0], ev[-1][0]

    # Discover the compositor pid from the DRM events' owner is unreliable
    # (delivery runs in irq context under whatever task was current), so the
    # caller passes it; default: the most frequent sched_switch comm match.
    if sm_pid is None:
        c = collections.Counter()
        for e in ev:
            if e[2] == 'sched_switch':
                m = SWITCH.match(e[5])
                if m and m.group(5) == 'surface-manager':
                    c[int(m.group(6))] += 1
        sm_pid = c.most_common(1)[0][0] if c else -1

    # LumaTV's compositor is a child of the supervisor that `pidof surface-manager` names, run under its private loader (its threads are called
    # `ld-linux-aarch6`); the supervisor's few runs say nothing of frames. `capture.sh start` writes the compositor's own process id, its loop
    # thread's, into the trace as `loop N` (LG's surface-manager is its own loop: the same pid as --sm): that is the thread whose renders are read.
    lumatv = loop_pid(ev) is not None
    sm_pid = loop_pid(ev) or sm_pid

    # The kernel waits behind each late frame (kwait.py): the D-state intervals of the compositor's thread and of the threads it waited
    # for, labelled from the kernel stacks the capture's trigger left. A trace without scheduler events (a basic capture) has none.
    kctx = None
    if any(e[2] == 'sched_switch' for e in ev):
        kev, kstacks = kwait.parse(path)
        kctx = kwait.Context(kev, kstacks, sm_pid)

    runs = Runs()
    frames = []       # (t, seq)
    vblanks = []      # t
    marks = []        # (t, text)
    wake = []         # (t, pid, comm, cpu)
    sm_wake = []      # (t) wakeups of the compositor thread
    sm_run = []       # (t0, t1, cpu)
    sm_state = {}     # cur run start
    irqs = []         # (t0, t1, cpu, irq, name)
    irq_open = {}
    softirq = []      # (t0, t1, cpu, action)
    soft_open = {}
    gpu_jobs = []     # (t0, t1, slot, pid)
    job_open = {}
    gpu_pm = []       # (t, event, value)
    cpuhp = []        # (t, text)
    reclaim = []      # (t, name, text)
    cma = []
    block = []
    wq = []           # (t0, t1, cpu, fn)
    wq_open = {}
    migr = []         # (t, pid, from, to)
    comms = {}        # pid -> comm, from sched_switch

    for t, cpu, name, pid, comm, args in ev:
        if name == 'drm_vblank_event_delivered':
            m = re.search(r'seq=(\d+)', args)
            frames.append((t, int(m.group(1)) if m else 0))
        elif name == 'drm_vblank_event':
            vblanks.append(t)
        elif name == 'sched_switch':
            m = SWITCH.match(args)
            if not m:
                continue
            pc, pp, _, pstate, nc, np_, _ = m.groups()
            pp, np_ = int(pp), int(np_)
            runs.switch(t, cpu, pp, pc, np_, nc)
            comms[np_] = nc
            if pp == sm_pid and cpu in sm_state:
                sm_run.append((sm_state.pop(cpu), t, cpu, pstate))
            if np_ == sm_pid:
                sm_state[cpu] = t
        elif name == 'sched_wakeup':
            m = WAKEUP.match(args)
            if m:
                wpid = int(m.group(2))
                wake.append((t, wpid, m.group(1), int(m.group(4))))
                if wpid == sm_pid:
                    sm_wake.append(t)
        elif name == 'sched_migrate_task':
            m = re.search(r'pid=(\d+).*orig_cpu=(\d+) dest_cpu=(\d+)', args)
            if m:
                migr.append((t, int(m.group(1)), int(m.group(2)), int(m.group(3))))
        elif name == 'irq_handler_entry':
            m = re.match(r'irq=(\d+) name=(\S+)', args)
            if m:
                irq_open[(cpu, int(m.group(1)))] = (t, m.group(2))
        elif name == 'irq_handler_exit':
            m = re.match(r'irq=(\d+)', args)
            if m and (cpu, int(m.group(1))) in irq_open:
                t0, nm = irq_open.pop((cpu, int(m.group(1))))
                irqs.append((t0, t, cpu, int(m.group(1)), nm))
        elif name == 'softirq_entry':
            m = re.search(r'action=(\w+)', args)
            if m:
                soft_open[(cpu, m.group(1))] = t
        elif name == 'softirq_exit':
            m = re.search(r'action=(\w+)', args)
            if m and (cpu, m.group(1)) in soft_open:
                softirq.append((soft_open.pop((cpu, m.group(1))), t, cpu, m.group(1)))
        elif name == 'mali_job_slots_event':
            # kbase gator encoding: (type << 24) | (slot << 16); type 1 start, 2 stop
            m = re.match(r'gpu=\d+ event=(\d+) tgid=(\d+) pid=(\d+) job_id=(\d+)', args)
            if m:
                e, tg = int(m.group(1)), int(m.group(2))
                typ, slot = e >> 24, (e >> 16) & 0xff
                if typ == 1:
                    job_open[slot] = (t, tg)
                elif slot in job_open:
                    t0, tg0 = job_open.pop(slot)
                    gpu_jobs.append((t0, t, slot, tg0))
        elif name == 'mali_pm_status':
            m = re.match(r'gpu=\d+ event (\d+) = (\d+)', args)
            if m:
                gpu_pm.append((t, int(m.group(1)), int(m.group(2))))
        elif name in ('cpuhp_enter', 'cpuhp_exit'):
            cpuhp.append((t, name + ' ' + args))
        elif name.startswith('mm_vmscan') or name.startswith('mm_compaction'):
            reclaim.append((t, name, args[:60]))
        elif name in ('cma_alloc', 'cma_release'):
            cma.append((t, name, args[:60]))
        elif name == 'block_rq_issue':
            block.append((t, args[:60]))
        elif name == 'workqueue_execute_start':
            m = re.search(r'function (\S+)', args)
            if m:
                wq_open[(cpu, pid)] = (t, m.group(1))
        elif name == 'workqueue_execute_end':
            if (cpu, pid) in wq_open:
                t0, fn = wq_open.pop((cpu, pid))
                wq.append((t0, t, cpu, fn))
        elif name == 'tracing_mark_write':
            marks.append((t, args.strip()))
    runs.finish(t_end)
    for cpu, t0 in sm_state.items():
        sm_run.append((t0, t_end, cpu, 'R'))
    sm_run.sort()
    sm_run_starts = [r[0] for r in sm_run]
    frames.sort()
    ftimes = [f[0] for f in frames]
    gpu_jobs.sort()
    gpu_starts = [g[0] for g in gpu_jobs]
    irqs.sort()
    irq_starts = [i[0] for i in irqs]
    # The display's OSD interrupt: on this set it stops while Dolby Vision playback has nothing drawn over it, and the first
    # frame after a key then waits for it to come back (round 11: 1171 ms, the first flip at 1479 ms).
    osd = [i[0] for i in irqs if i[4] == 'osd_irq']
    wake.sort()
    wake_ts = [w[0] for w in wake]
    sm_wake.sort()
    wq.sort()
    wq_starts = [w[0] for w in wq]

    def in_range(lst, key, a, b):
        i = bisect.bisect_left(key, a)
        j = bisect.bisect_right(key, b)
        return lst[i:j]

    def sm_activity(a, b):
        """seconds running / runnable / sleeping for the compositor thread in [a,b]."""
        running = 0.0
        i = max(bisect.bisect_right(sm_run_starts, a) - 1, 0)
        cpus = set()
        while i < len(sm_run) and sm_run[i][0] < b:
            t0, t1, cpu, st = sm_run[i]
            o = min(t1, b) - max(t0, a)
            if o > 0:
                running += o
                cpus.add(cpu)
            i += 1
        # runnable: from each wakeup inside the window to the next run start
        runnable = 0.0
        max_lat = 0.0
        for wt in in_range(sm_wake, sm_wake, a, b):
            k = bisect.bisect_left(sm_run_starts, wt)
            if k < len(sm_run):
                lat = min(sm_run[k][0], b) - wt
                if lat > 0:
                    runnable += lat
                    max_lat = max(max_lat, lat)
        return running, runnable, max_lat, sorted(cpus)

    # windows from P: markers
    phases = [(t, txt[2:]) for t, txt in marks if txt.startswith('P:')]
    if window:
        windows = [(window[0], window[1], 'window')]
    elif phases:
        windows = []
        for i, (t, nm) in enumerate(phases):
            t_next = phases[i + 1][0] if i + 1 < len(phases) else t_end
            windows.append((t, t_next, nm))
    else:
        windows = [(t_start, t_end, 'all')]

    report = {'trace': path, 'sm_pid': sm_pid, 'span_s': round(t_end - t_start, 3),
              'frames_total': len(frames), 'vblank_hz': round(len(vblanks) / (t_end - t_start), 2) if vblanks else None,
              'cpuhp_events': len(cpuhp), 'gpu_pm_events': len(gpu_pm), 'windows': []}

    for a, b, nm in windows:
        drop_ms, panel_hz = panel_drop_ms(vblanks, a, b)
        fr = in_range(frames, ftimes, a, b)
        ivals = [(fr[i + 1][0] - fr[i][0]) * 1000.0 for i in range(len(fr) - 1)]
        seqd = collections.Counter(fr[i + 1][1] - fr[i][1] for i in range(len(fr) - 1))
        w = {'name': nm, 'start': round(a, 3), 'dur_s': round(b - a, 3), 'frames': len(fr),
             'fps': round(len(fr) / (b - a), 1) if b > a else 0,
             'over17': sum(1 for x in ivals if x > 17.5), 'over20': sum(1 for x in ivals if x > drop_ms), 'panel_hz': panel_hz, 'drop_ms': drop_ms,
             'over33': sum(1 for x in ivals if x > 33.5), 'over50': sum(1 for x in ivals if x > 50),
             'worst_ms': round(max(ivals), 1) if ivals else 0,
             'vblanks_between_presents': dict(sorted(seqd.items())),
             'gaps': []}
        # compositor thread totals in window
        running, runnable, max_lat, cpus = sm_activity(a, b)
        w['sm_run_ms'] = round(running * 1000, 1)
        w['sm_runnable_ms'] = round(runnable * 1000, 1)
        w['sm_max_sched_latency_ms'] = round(max_lat * 1000, 2)
        w['sm_migrations'] = sum(1 for m in migr if a <= m[0] <= b and m[1] == sm_pid)
        # top CPU consumers in window
        tot = collections.Counter()
        names = {}
        for cpu in range(4):
            for pid, comm, s in runs.overlap(cpu, a, b):
                if pid != 0:
                    tot[comm if comm.startswith('kworker') else f'{comm}-{pid}'] += s
        w['top_cpu_ms'] = [(k, round(v * 1000, 1)) for k, v in tot.most_common(12)]
        gj = in_range(gpu_jobs, gpu_starts, a, b)
        w['gpu_jobs'] = len(gj)
        w['gpu_busy_ms'] = round(sum(min(t1, b) - max(t0, a) for t0, t1, _, _ in gj) * 1000, 1)
        w['gpu_pm_transitions'] = sum(1 for g in gpu_pm if a <= g[0] <= b)
        w['cpuhp'] = sum(1 for c in cpuhp if a <= c[0] <= b and 'cpuhp_enter' in c[1] and 'step: 209' in c[1] or (a <= c[0] <= b and 'cpuhp_enter' in c[1] and 'bringup_cpu' in c[1]))
        w['reclaim_events'] = sum(1 for r in reclaim if a <= r[0] <= b)
        w['cma_events'] = sum(1 for r in cma if a <= r[0] <= b)
        w['block_rq'] = sum(1 for r in block if a <= r[0] <= b)
        ic = collections.Counter(nm_ for t0, t1, cpu, irq, nm_ in in_range(irqs, irq_starts, a, b))
        w['irq_counts'] = dict(ic.most_common(8))
        # ms from the marker until the OSD interrupt resumed, when it was silent for the 100 ms before the marker (None: it was
        # running, or the capture has no interrupts); the window's length when it never resumed inside it
        w['osd_wake_ms'] = None
        if osd and not in_range(osd, osd, a - 0.1, a):
            nxt = in_range(osd, osd, a, b)
            w['osd_wake_ms'] = round(((nxt[0] if nxt else b) - a) * 1000.0, 1)

        # Per-frame chain: which frames were requested by the animation driver
        # (the GUI thread's render burst follows the previous commit after
        # Qt's 5 ms update timer) and which by a slower timer (wallpaper tick,
        # clock). Only an animation-driven frame that took 2+ vblanks is a
        # dropped frame; a timer-driven one at 2 vblanks is content at 30 fps.
        frag = [(t0, t1) for t0, t1, slot, tg in gpu_jobs if slot == 0]
        bursts = []
        for t0, t1, cpu, st in sm_run:
            if bursts and t0 - bursts[-1][1] < 0.0003 and bursts[-1][2].startswith('R'):
                bursts[-1][1] = t1
                bursts[-1][2] = st
            else:
                bursts.append([t0, t1, st])
        bstarts = [x[0] for x in bursts]
        anim = []
        anim_t = []       # presentation time of each animated frame, parallel to anim
        anim_late = []    # parallel to anim: the previous presented frame was animated too
        timer = []
        for i in range(1, len(fr)):
            f0, f1 = fr[i - 1][0], fr[i][0]
            gj2 = [(x, y) for x, y in frag if f0 < y <= f1]
            gpu_end = max([y for x, y in gj2], default=None)
            lo = bisect.bisect_left(bstarts, f0)
            bs = [(x, y) for x, y, st in bursts[lo:lo + 40] if x >= f0 and y <= f1]
            after = renders([(x, y) for x, y in bs if gpu_end is None or x > gpu_end + 0.0005], lumatv)
            # With QT_QPA_UPDATE_IDLE_TIME=0 the render follows the commit in
            # the same burst: a burst starting at the GPU completion that lasts
            # longer than a commit (0.6-0.8 ms) is commit plus render.
            fused = [(x, y) for x, y in bs if gpu_end is not None and gpu_end - 0.0002 <= x <= gpu_end + 0.0005 and y - x > 0.0013]
            if fused and (not after or max(p[1] - p[0] for p in after) < 0.001):
                x, y = max(fused, key=lambda p: p[1] - p[0])
                anim.append(((f1 - f0) * 1000, (y - x) * 1000 - 0.7))
                anim_t.append(f1)
                anim_late.append(len(anim_t) > 1 and anim_t[-2] == f0)
                continue
            if not after:
                timer.append(((f1 - f0) * 1000, 0.0))
                continue
            rx, ry = max(after, key=lambda p: p[1] - p[0])
            prev = [y for x, y in bs if y <= rx]
            idle = (rx - max(prev)) * 1000 if prev else None
            rec = ((f1 - f0) * 1000, (ry - rx) * 1000)
            if idle is not None and idle < 7.5:
                anim.append(rec)
                anim_t.append(f1)
                anim_late.append(len(anim_t) > 1 and anim_t[-2] == f0)
            else:
                timer.append(rec)
        w['anim_frames'] = len(anim)
        w['dropped'] = sum(1 for iv, r in anim if iv > drop_ms)
        # A frame over 20 ms that follows another animated frame is a miss inside a running
        # animation; the first animated frame after an idle gap is late by construction (the update
        # that starts it fires when the key does) and is not counted here.
        w['dropped_in_motion'] = sum(1 for (iv, r), late in zip(anim, anim_late) if iv > drop_ms and late)
        w['dropped_pct'] = round(100.0 * w['dropped'] / len(anim), 1) if anim else 0
        w['timer_frames'] = len(timer)
        rends = sorted(r for iv, r in anim)
        w['anim_render_ms'] = {'median': round(rends[len(rends) // 2], 2), 'p90': round(rends[int(0.9 * (len(rends) - 1))], 2), 'max': round(rends[-1], 2)} if rends else None
        w['anim_render_samples'] = [round(r, 2) for iv, r in anim]
        w['dropped_render_ms'] = sorted(round(r, 1) for iv, r in anim if iv > drop_ms)[-12:]

        # The burst that starts at the window's marker: the response to the key
        # injected right after it. Animated frames count when both of their
        # frames lie inside it.
        bu = burst([f[0] for f in fr], a, drop_ms=drop_ms)
        w['first_frame_ms'] = bu['first_frame_ms']
        w['burst_end_ms'] = bu['burst_end_ms']
        w['burst_over20'] = bu['over20']
        # GPU jobs of an app (a film's player redrawing its own controls) that ended while the burst ran: its frames chain into the
        # burst, which then measures the app as well as the surface (round 16: Netflix at 60 Hz for 2.4 s kept a picker closing's
        # burst going to 944 ms)
        w['burst_app_jobs'] = sum(1 for t0, t1, slot, tg in gpu_jobs
                                  if bu['frames'] and a - APP_QUEUE_S <= t1 <= bu['last_t'] and comms.get(tg, '') not in COMPOSITION_COMMS)
        b_rends = sorted(r for (iv, r), t in zip(anim, anim_t) if bu['frames'] and bu['first_t'] < t <= bu['last_t'])
        w['burst_anim_p90_ms'] = round(b_rends[int(0.9 * (len(b_rends) - 1))], 2) if b_rends else None
        w['burst_anim_samples'] = [round(r, 2) for r in b_rends]
        # In time order, the first animated frames of the burst: the ones that pay for anything the
        # surface draws for the first time (glyph sets, programs, textures).
        w['burst_first_render_ms'] = [round(r, 2) for (iv, r), t in zip(anim, anim_t)
                                      if bu['frames'] and bu['first_t'] < t <= bu['last_t']][:6]

        for i, x in enumerate(ivals):
            if x <= gap_ms:
                continue
            g0, g1 = fr[i][0], fr[i + 1][0]
            g = {'at': round(g0 - a, 3), 'ms': round(x, 1), 'vblanks': fr[i + 1][1] - fr[i][1]}
            sr, sq, ml, cp = sm_activity(g0, g1)
            g['sm'] = {'run_ms': round(sr * 1000, 2), 'runnable_ms': round(sq * 1000, 2),
                       'max_sched_lat_ms': round(ml * 1000, 2), 'cpus': cp,
                       'sleep_ms': round((x - (sr + sq) * 1000), 2)}
            g['cpu'] = {}
            for cpu in range(4):
                g['cpu'][cpu] = [(f'{c}-{p}', round(s * 1000, 2)) for p, c, s in runs.overlap(cpu, g0, g1)[:4]]
            gj = in_range(gpu_jobs, gpu_starts, g0, g1)
            g['gpu'] = {'jobs': len(gj), 'busy_ms': round(sum(min(t1, g1) - max(t0, g0) for t0, t1, _, _ in gj) * 1000, 2),
                        'pm': [(round((t - g0) * 1000, 2), e, v) for t, e, v in gpu_pm if g0 <= t <= g1][:8]}
            ii = in_range(irqs, irq_starts, g0, g1)
            g['irq_ms'] = {}
            for t0, t1, cpu, irq, nm_ in ii:
                g['irq_ms'][nm_] = round(g['irq_ms'].get(nm_, 0) + (t1 - t0) * 1000, 3)
            g['softirq_ms'] = round(sum(min(t1, g1) - max(t0, g0) for t0, t1, cpu, act in softirq if t0 < g1 and t1 > g0) * 1000, 2)
            g['cpuhp'] = [c[1][:70] for c in cpuhp if g0 <= c[0] <= g1][:4]
            g['reclaim'] = [r[1] for r in reclaim if g0 <= r[0] <= g1][:6]
            g['cma'] = [r[1] for r in cma if g0 <= r[0] <= g1][:4]
            g['block'] = [r[1] for r in block if g0 <= r[0] <= g1][:4]
            wk = collections.Counter(c for t, p, c, cpu in in_range(wake, wake_ts, g0, g1))
            g['wakeups'] = wk.most_common(8)
            wqf = collections.Counter(fn for t0, t1, cpu, fn in in_range(wq, wq_starts, g0, g1))
            g['workqueue'] = wqf.most_common(5)
            if kctx and x <= kwait.MAX_GAP_MS:
                g['kwait'] = kctx.frame(g0, g1)
            w['gaps'].append(g)
        report['windows'].append(w)
        w['kwait'] = kwait.summarize([g['kwait'] for g in w['gaps'] if 'kwait' in g]) if kctx else None
    return report


def fmt(report, top=10):
    out = [f"trace {report['trace']}  sm_pid {report['sm_pid']}  span {report['span_s']} s  frames {report['frames_total']}  vblank {report['vblank_hz']} Hz  cpuhp {report['cpuhp_events']}  gpu_pm {report['gpu_pm_events']}"]
    for w in report['windows']:
        out.append(f"\n== {w['name']}: {w['dur_s']} s, {w['frames']} frames ({w['fps']} fps), >17 {w['over17']}, >20 {w['over20']}, >33 {w['over33']}, >50 {w['over50']}, worst {w['worst_ms']} ms; vblanks/present {w['vblanks_between_presents']}"
                   + (f"; panel {w['panel_hz']} Hz" if w.get('panel_hz') is not None else ''))
        out.append(f"   animation-driven frames {w['anim_frames']}, dropped {w['dropped']} ({w['dropped_pct']} %), render ms {w['anim_render_ms']}, dropped-frame render {w['dropped_render_ms']}; timer-driven frames {w['timer_frames']}")
        out.append(f"   burst from marker: first frame {w['first_frame_ms']} ms, ends {w['burst_end_ms']} ms, intervals >20 ms {w['burst_over20']}, animated render p90 {w['burst_anim_p90_ms']} ms"
                   + (f"; the OSD was off at the marker, on after {w['osd_wake_ms']} ms" if w.get('osd_wake_ms') is not None else ''))
        out.append(f"   compositor: run {w['sm_run_ms']} ms, runnable {w['sm_runnable_ms']} ms, max sched latency {w['sm_max_sched_latency_ms']} ms, migrations {w['sm_migrations']}; gpu jobs {w['gpu_jobs']} busy {w['gpu_busy_ms']} ms, pm transitions {w['gpu_pm_transitions']}; cpuhp {w['cpuhp']} reclaim {w['reclaim_events']} cma {w['cma_events']} block {w['block_rq']}")
        out.append(f"   top cpu: {w['top_cpu_ms']}")
        out.append(f"   irqs: {w['irq_counts']}")
        gaps = sorted(w['gaps'], key=lambda g: -g['ms'])[:top]
        if len(w['gaps']) > top:
            out.append(f"   ({len(w['gaps'])} gaps over threshold; {top} longest shown)")
        for g in sorted(gaps, key=lambda g: g['at']):
            out.append(f"   gap +{g['at']}s {g['ms']} ms ({g['vblanks']} vbl): sm run {g['sm']['run_ms']} runnable {g['sm']['runnable_ms']} (max lat {g['sm']['max_sched_lat_ms']}) sleep {g['sm']['sleep_ms']} on {g['sm']['cpus']}; gpu jobs {g['gpu']['jobs']} busy {g['gpu']['busy_ms']} pm {g['gpu']['pm'][:4]}; irq {g['irq_ms']} softirq {g['softirq_ms']}")
            for cpu, lst in g['cpu'].items():
                out.append(f"      cpu{cpu}: {lst}")
            if g['cpuhp']: out.append(f"      cpuhp: {g['cpuhp']}")
            if g['reclaim']: out.append(f"      reclaim: {g['reclaim']}")
            if g['cma']: out.append(f"      cma: {g['cma']}")
            if g['block']: out.append(f"      block: {g['block']}")
            if g.get('kwait'):
                out.append(f"      {kwait.describe(g['kwait'])}")
            out.append(f"      wakeups: {g['wakeups']}  wq: {g['workqueue']}")
        if w.get('kwait') and w['kwait']['late']:
            out.append(f"   {kwait.summary_line(w['kwait'])}")
    return '\n'.join(out)


if __name__ == '__main__':
    ap = argparse.ArgumentParser()
    ap.add_argument('trace')
    ap.add_argument('--sm', type=int)
    ap.add_argument('--gap', type=float, default=20.0)
    ap.add_argument('--top', type=int, default=10)
    ap.add_argument('--window', type=float, nargs=2)
    ap.add_argument('--json')
    ap.add_argument('--refresh', type=float, default=60.0, help='the refresh the frames were presented at, Hz: the dropped-frame threshold is a refresh and a fifth (20 ms at 60)')
    a = ap.parse_args()
    set_refresh(a.refresh)
    r = analyse(a.trace, a.sm, a.gap, a.window)
    if a.json:
        with open(a.json, 'w') as f:
            json.dump(r, f, indent=1)
    print(fmt(r, a.top))
