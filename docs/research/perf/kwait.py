#!/usr/bin/env python3
"""Which kernel waits made a frame late (LPC 2026, "Kernel Lock Contention Hotspots Causing Frame Drops on Android").

    kwait.py TRACE [--tid TID | --comm NAME] [--gap MS] [--period MS] [--hops N] [--x1 X1LOG] [--json OUT]

A late frame is a `drm_vblank_event_delivered` more than --gap ms (default 20) after the one before it. The compositor's loop thread
missed it; the loop is blamed on the kernel when the time it, or a thread it was waiting for, spent blocked in the kernel covers
most of the overrun (the frame's interval less one refresh). From a capture.sh `full` or `kwait` trace:

  - a kernel wait is an interval of `sched_switch` with `prev_state` D (uninterruptible, blocked in a lock, I/O or the driver) to the
    `sched_wakeup` that ends it; only waits of 3 ms or more count (--min-wait);
  - the threads the loop waited for are found through the wakeups: the thread that woke the loop is asked what it was blocked in
    while the loop waited, and the thread that woke that one in turn, up to 3 hops;
  - the union of those intervals inside the late frame, over the overrun, is the score; 0.5 or more is "kernel-caused";
  - each wait is labelled: from the kernel stack that capture.sh's `stacktrace` trigger leaves after a D switch (`kbase` for the Mali
    driver's own symbols, `mmap_sem` for a read-write semaphore sleep in a page fault or an mmap/munmap/mprotect/brk, `io`, `swap`,
    `memory` for reclaim, compaction and CMA, `fence`, `mutex`, `futex`, `cgroup`), else from what the trace saw around it (a block
    request issued, reclaim, a GPU job running: the label then ends in `?`), else `other`;
  - a loop that waited for a CPU is no kernel wait: its run-queue time over 3 ms is reported apart as `sched`, with who held the CPU.

The loop thread is the id capture.sh wrote into the trace when it started (a `loop TID` marker); for a trace without one --comm names it
(a thread whose name starts so), else the compositor's own names are tried in turn: LumaTV's runs under its private loader, so its
threads are `ld-linux-aarch6` (the loader's file name cut to 15 characters), not the binary's name.

A client that commits every frame, as the X1 probe does, can be late on its own side while the flips are not: the compositor's frame callbacks
start the client's next frame, and they go out when a flip lands (`ctl callbacks clients landing`) or with the frame's draw (the default since
112d9cc). The same trace says when the client's first GPU job of that frame began after that moment (its reaction: the client waking, drawing
and submitting), taken from the flips or from the compositor's own job starts, whichever the client's jobs follow more closely. A reaction that
is 3.3 ms longer than the one before it is a swap gap of 20 ms with no late flip. The reactions over 3 ms are listed with the tasks seen on the
loop's CPU meanwhile, and with --x1 the client's own gap lines are set beside the gaps the trace implies.

`analyse.py` calls this for every gap it lists; `gate.py` prints the result under each surface.
"""
import argparse
import bisect
import collections
import gzip
import json
import re
import sys

LINE = re.compile(r'^\s*(.*?)-(\d+)\s+\[(\d{3})\]\s+(\S+)\s+([\d.]+):\s+(\w+):\s?(.*)$')
STACK_HEAD = re.compile(r'^\s*(.*?)-(\d+)\s+\[(\d{3})\]\s+(\S+)\s+([\d.]+):\s+<stack trace>')
STACK_LINE = re.compile(r'^\s*=>\s+([A-Za-z_][\w.$]*)')
SWITCH = re.compile(r'prev_comm=(.*?) prev_pid=(\d+) prev_prio=(-?\d+) prev_state=(\S+) ==> next_comm=(.*?) next_pid=(\d+) next_prio=(-?\d+)')
WAKEUP = re.compile(r'comm=(.*?) pid=(\d+) prio=(-?\d+) target_cpu=(\d+)')

MIN_WAIT_MS = 3.0
HOPS = 3
MAX_GAP_MS = 250.0     # a longer silence between presented frames is a still screen, not a late frame
KERNEL_SHARE = 0.5
PERIOD_MS = 1000.0 / 60.0

# Labels from a kernel stack, first match wins; a stack is its symbols from the schedule() call outwards.
_MM_CALLERS = re.compile(r'^(do_page_fault|__do_page_fault|do_translation_fault|do_mem_abort|do_el0_\w+|el0_\w+|handle_mm_fault|__handle_mm_fault|'
                         r'vm_mmap_pgoff|ksys_mmap_pgoff|__vm_munmap|__do_munmap|do_munmap|do_mprotect_pkey|__arm64_sys_(mmap|munmap|mprotect|brk|mremap)|'
                         r'mmap_region|do_mmap|vm_brk_flags|do_brk_flags|__mm_populate|populate_vma_page_range|__get_user_pages|get_user_pages\w*|'
                         r'access_remote_vm|__access_remote_vm|move_vma|mm_populate)$')
_RWSEM = re.compile(r'^(rwsem_down_\w+|call_rwsem_down_\w+|__down_read\w*|down_read\w*|down_write\w*|__down_write\w*)$')
STACK_RULES = [
    ('mmap_sem', lambda s: any(_RWSEM.match(x) for x in s) and any(_MM_CALLERS.match(x) for x in s)),
    ('cgroup', lambda s: any(re.match(r'^(percpu_down_read\w*|__percpu_down_read|percpu_rwsem_\w+|cgroup_\w+|cgroup_threadgroup\w*|copy_process|_?do_fork|kernel_clone)$', x) for x in s)),
    ('kbase', lambda s: any(re.match(r'^(kbase\w*|kctx\w*|mali\w*|gpu_\w+)$', x) for x in s)),
    ('swap', lambda s: any(re.match(r'^(swap_readpage|do_swap_page|swapin_readahead|swap_cluster_readahead|zram_\w+|zacc\w*|f2io\w*)$', x) for x in s)),
    ('memory', lambda s: any(re.match(r'^(alloc_contig_range|cma_alloc|compact_zone\w*|try_to_free_pages|do_try_to_free_pages|shrink_\w+|__alloc_pages_slowpath|'
                                      r'__alloc_pages_direct_\w+|wait_iff_congested|congestion_wait|drain_all_pages|__lru_add_drain_all)$', x) for x in s)),
    ('io', lambda s: any(re.match(r'^(io_schedule\w*|wait_on_page_bit\w*|__lock_page\w*|lock_page\w*|filemap_fault|__filemap_fault|do_read_cache_page|submit_bio_wait|'
                                  r'wait_for_completion_io\w*|blk_\w+|bio_\w+|mmc_\w+|ext4_\w+|f2fs_\w+|jbd2_\w+|__wait_on_buffer|wait_on_buffer|squashfs\w*|erofs_\w+|'
                                  r'generic_file_\w+|filemap_\w+|mpage_\w+|do_fault|do_read_fault|__do_fault)$', x) for x in s)),
    ('fence', lambda s: any(re.match(r'^(dma_fence_\w+|sync_file_\w+|sync_fence_\w+|fence_wait\w*|reservation_object_\w+|dma_resv_\w+)$', x) for x in s)),
    ('futex', lambda s: any(re.match(r'^(futex_wait\w*|do_futex)$', x) for x in s)),
    ('mutex', lambda s: any(re.match(r'^(mutex_lock\w*|__mutex_lock\w*|ww_mutex_lock\w*|rwsem_down_\w+|down_read\w*|down_write\w*|down_interruptible|down)$', x) for x in s)),
]


def label_stack(symbols):
    """The class a kernel stack says a wait is, or None."""
    for name, rule in STACK_RULES:
        if rule(symbols):
            return name
    return None


class Wait:
    """One interval a thread was off the CPU: kind 'D' (blocked in the kernel) or 'R' (runnable, waiting for a CPU)."""
    __slots__ = ('tid', 'comm', 't0', 't1', 'kind', 'waker', 'waker_comm', 'stack', 'by')

    def __init__(self, tid, comm, t0, t1, kind, waker=None, waker_comm='', stack=None, by=''):
        self.tid, self.comm, self.t0, self.t1, self.kind = tid, comm, t0, t1, kind
        self.waker, self.waker_comm, self.stack, self.by = waker, waker_comm, stack, by

    @property
    def ms(self):
        return (self.t1 - self.t0) * 1000.0

    def __repr__(self):
        return f'Wait({self.kind} {self.comm}-{self.tid} {self.t0:.6f}..{self.t1:.6f} {self.ms:.2f} ms)'


def parse(path):
    """(events, stacks) from a trace dump (plain text, or gzip when the name ends in .gz): events as analyse.py's
    `(t, cpu, name, pid, comm, args)` sorted by time, stacks as {index into events of a sched_switch: [symbol, ...]} for the switches a
    `stacktrace` trigger followed."""
    with (gzip.open(path, 'rt', errors='replace') if path.endswith('.gz') else open(path, errors='replace')) as f:
        return parse_lines(f)


def parse_lines(lines):
    events, stacks = [], {}
    last_switch = {}       # cpu -> index in `events` of the cpu's latest sched_switch, as it was appended
    open_stack = None      # the stack being read: (index, symbols)
    for line in lines:
        m = LINE.match(line)
        if m:
            comm, pid, cpu, _flags, ts, name, args = m.groups()
            events.append((float(ts), int(cpu), name, int(pid), comm, args))
            if name == 'sched_switch':
                last_switch[int(cpu)] = len(events) - 1
            open_stack = None
            continue
        h = STACK_HEAD.match(line)
        if h:
            pid, cpu = int(h.group(2)), int(h.group(3))
            i = last_switch.get(cpu)
            open_stack = None
            if i is not None:
                sm = SWITCH.match(events[i][5])
                if sm and int(sm.group(2)) == pid:
                    open_stack = stacks.setdefault(i, [])
            continue
        s = STACK_LINE.match(line)
        if s and open_stack is not None:
            open_stack.append(s.group(1))
    # Sorting moves events: the stacks follow their switches.
    order = sorted(range(len(events)), key=lambda i: events[i][0])
    rank = {old: new for new, old in enumerate(order)}
    return [events[i] for i in order], {rank[i]: syms for i, syms in stacks.items()}


def build_waits(events, stacks):
    """{tid: [Wait, ...]} in time order: every D wait (from the switch out to the wakeup, or to the switch in when the wakeup is not in the
    trace) and every run-queue wait (from a wakeup, or a preemption, to the switch in)."""
    waits = collections.defaultdict(list)
    blocked = {}       # tid -> (t0, comm, stack)
    runnable = {}      # tid -> (t, comm, waker tid, waker comm)
    preempted = {}     # tid -> (t0, comm, who took the CPU)
    for i, (t, cpu, name, pid, comm, args) in enumerate(events):
        if name == 'sched_switch':
            m = SWITCH.match(args)
            if not m:
                continue
            pc, pp, _, pstate, nc, np_, _ = m.groups()
            pp, np_ = int(pp), int(np_)
            if np_ in runnable:
                t0, c0, w, wc = runnable.pop(np_)
                waits[np_].append(Wait(np_, nc, t0, t, 'R', w, wc))
            if np_ in preempted:
                t0, c0, by = preempted.pop(np_)
                waits[np_].append(Wait(np_, nc, t0, t, 'R', by=by))
            if np_ in blocked:
                t0, c0, st = blocked.pop(np_)
                waits[np_].append(Wait(np_, nc, t0, t, 'D', stack=st))
            if 'D' in pstate:
                blocked[pp] = (t, pc, stacks.get(i))
            elif pstate.startswith('R'):
                preempted[pp] = (t, pc, nc)
        elif name == 'sched_wakeup':
            m = WAKEUP.match(args)
            if not m:
                continue
            wc, wp = m.group(1), int(m.group(2))
            if wp in blocked:
                t0, c0, st = blocked.pop(wp)
                waits[wp].append(Wait(wp, wc, t0, t, 'D', pid, comm, st))
            runnable[wp] = (t, wc, pid, comm)
    return waits


def label_wait(w, events_by_name):
    """(class, source) of a D wait: from its stack, else from what the trace shows inside it for the same thread, else 'other'."""
    if w.stack:
        c = label_stack(w.stack)
        if c:
            return c, 'stack'
        return 'other', 'stack'
    for name, cls in (('block_rq_issue', 'io?'), ('mm_vmscan_direct_reclaim_begin', 'memory?'), ('cma_alloc', 'memory?'),
                      ('mm_compaction_begin', 'memory?')):
        for t, pid, _args in events_by_name.get(name, ()):
            if w.t0 <= t <= w.t1 and (pid == w.tid or name == 'block_rq_issue'):
                return cls, 'context'
    for t0, t1 in events_by_name.get('gpu_jobs', ()):
        if t0 < w.t1 and t1 > w.t0:
            return 'kbase?', 'context'
    return 'other', 'none'


def index_events(events):
    """What `label_wait` looks inside a wait for: the block requests, reclaim, CMA and compaction events by name, and the GPU jobs' spans."""
    by = collections.defaultdict(list)
    open_job = {}
    for t, cpu, name, pid, comm, args in events:
        if name in ('block_rq_issue', 'mm_vmscan_direct_reclaim_begin', 'cma_alloc', 'mm_compaction_begin'):
            by[name].append((t, pid, args))
        elif name == 'mali_job_slots_event':
            m = re.match(r'gpu=\d+ event=(\d+) ', args)
            if m:
                e = int(m.group(1))
                typ, slot = e >> 24, (e >> 16) & 0xff
                if typ == 1:
                    open_job[slot] = t
                elif slot in open_job:
                    by['gpu_jobs'].append((open_job.pop(slot), t))
    return by


def union_ms(spans):
    """Total length in ms of the union of (t0, t1) spans in seconds."""
    total, end = 0.0, None
    for a, b in sorted(spans):
        if end is None or a > end:
            total += b - a
            end = b
        elif b > end:
            total += b - end
            end = b
    return total * 1000.0


class Context:
    """A trace read once, asked about many late frames."""

    def __init__(self, events, stacks, loop_tid):
        self.loop = loop_tid
        self.waits = build_waits(events, stacks)
        self.index = index_events(events)

    def frame(self, g0, g1, period_ms=PERIOD_MS, hops=HOPS, min_wait_ms=MIN_WAIT_MS, share=KERNEL_SHARE):
        """The attribution of one late frame, the interval from vblank delivery `g0` to `g1` (seconds)."""
        interval = (g1 - g0) * 1000.0
        overrun = max(interval - period_ms, 1.0)
        found, spans, seen = [], [], set()
        frontier = collections.deque([(self.loop, g0, g1, 0)])
        while frontier:
            tid, a, b, hop = frontier.popleft()
            for w in self.waits.get(tid, ()):
                if w.kind != 'D' or w.t1 <= a or w.t0 >= b:
                    continue
                s, e = max(w.t0, a), min(w.t1, b)
                if (e - s) * 1000.0 < min_wait_ms or (tid, w.t0) in seen:
                    continue
                seen.add((tid, w.t0))
                cls, src = label_wait(w, self.index)
                found.append({'tid': tid, 'comm': w.comm, 'hop': hop, 'from': round(s - g0, 6), 'ms': round((e - s) * 1000.0, 2), 'class': cls,
                              'source': src, 'woken_by': f'{w.waker_comm}-{w.waker}' if w.waker is not None else None,
                              'stack': (w.stack or [])[:6]})
                spans.append((s, e))
                if hop < hops and w.waker not in (None, 0, tid):
                    frontier.append((w.waker, s, e, hop + 1))
        by_class = collections.Counter()
        for x in found:
            by_class[x['class']] += x['ms']
        covered = union_ms(spans)
        # The loop's own run-queue time over the limit, and who had its CPU.
        sched = [w for w in self.waits.get(self.loop, ()) if w.kind == 'R' and w.t1 > g0 and w.t0 < g1 and (min(w.t1, g1) - max(w.t0, g0)) * 1000.0 >= min_wait_ms]
        sched_ms = union_ms([(max(w.t0, g0), min(w.t1, g1)) for w in sched])
        score = min(1.0, covered / overrun)
        top = max(by_class, key=by_class.get) if by_class else None
        return {'ms': round(interval, 1), 'overrun_ms': round(overrun, 1), 'kernel_ms': round(covered, 2), 'score': round(score, 2),
                'kernel': score >= share, 'class': top if score >= share else None, 'by_class': {k: round(v, 2) for k, v in by_class.most_common()},
                'waits': sorted(found, key=lambda x: (x['hop'], x['from'])), 'sched_ms': round(sched_ms, 2),
                'sched_behind': sorted({w.by or w.waker_comm for w in sched if (w.by or w.waker_comm)})[:4],
                'verdict': 'kernel' if score >= share else ('sched' if sched_ms >= overrun * share else 'unexplained')}


def late_frames(events, gap_ms=20.0, max_gap_ms=MAX_GAP_MS, span=None):
    """[(g0, g1)] of the presented frames more than gap_ms and no more than max_gap_ms after the one before (a longer silence is a still screen,
    not a frame that was late); with `span` (from, to) only the frames inside it: the run of a client that draws every frame, not the still
    screen before it starts and after it ends."""
    t = [e[0] for e in events if e[2] == 'drm_vblank_event_delivered']
    return [(a, b) for a, b in zip(t, t[1:]) if gap_ms < (b - a) * 1000.0 <= max_gap_ms and (span is None or span[0] <= a and b <= span[1])]


# The names the compositor's threads have in a trace that carries no `loop` marker, tried in turn: its binary's (started directly), the
# private loader's (`ld-linux-aarch64.so.1` cut to the 15 characters of a comm: how LumaTV's compositor runs on the set, and the X1 client too),
# LG's.
LOOP_COMMS = ('lumatv-composit', 'ld-linux', 'surface-manager')
LOOP_MARK = re.compile(r'^loop (\d+)\s*$')
MARK_EVENTS = ('tracing_mark_write', 'print')     # what a trace_marker write is called: the set's 5.4 kernel, newer kernels


def marked_loop(events):
    """The thread id capture.sh marked when it started (`loop TID`, the last such marker when there are several), None without one."""
    tid = None
    for t, cpu, name, pid, comm, args in events:
        if name in MARK_EVENTS:
            m = LOOP_MARK.match(args)
            if m:
                tid = int(m.group(1))
    return tid


def threads_seen(events):
    """Counter of (comm, tid) over the trace's sched_switch events, each switch counting for both threads."""
    c = collections.Counter()
    for t, cpu, name, pid, ec, args in events:
        if name == 'sched_switch':
            m = SWITCH.match(args)
            if m:
                c[(m.group(1), int(m.group(2)))] += 1
                c[(m.group(5), int(m.group(6)))] += 1
    return c


def busiest(seen, prefix):
    """The thread id, among those whose name starts with `prefix`, that switched most; None when none did."""
    best = [(n, tid) for (comm, tid), n in seen.items() if comm.startswith(prefix)]
    return max(best)[1] if best else None


def find_loop(events, comm=None):
    """(tid, how): the compositor's loop thread and how it was found; (None, what was tried) when it cannot be. A name given is tried first,
    then the id capture.sh marked, then the compositor's own names. Among threads of one name the busiest is taken: the X1 client runs under
    the same loader and is `ld-linux-aarch6` as well, but the loop switches hundreds of times a second and the client a few times."""
    seen = threads_seen(events)
    if comm:
        tid = busiest(seen, comm)
        if tid is not None:
            return tid, f'a thread named {comm}'
    tid = marked_loop(events)
    if tid is not None:
        return tid, 'the loop marker in the trace'
    for name in LOOP_COMMS:
        tid = busiest(seen, name)
        if tid is not None:
            return tid, f'the busiest thread named {name}'
    top = ', '.join(f'{c}-{t} ({n})' for (c, t), n in seen.most_common(4)) or 'none'
    return None, f"no loop marker, no thread named {', '.join(([comm] if comm else []) + list(LOOP_COMMS))}; busiest threads: {top}"


JOB = re.compile(r'gpu=\d+ event=(\d+) tgid=(\d+) ')
REACTION_SLOW_MS = 3.0
REACTION_START = 8     # the client's first job starts are its start-up (X1 primes its swapchain with four frames of two jobs): flips before the ninth are not read


def job_starts(events):
    """({tgid: [t, ...]}, {tid, ...}): when each process's GPU jobs began (`mali_job_slots_event` of type 1, a job slot starting; tgid 0 is
    the interrupt's stop events), and the threads that started them (the Mali threads and the workers the driver hands the jobs to)."""
    starts, plumbing = collections.defaultdict(list), set()
    for t, cpu, name, pid, comm, args in events:
        if name == 'mali_job_slots_event':
            m = JOB.match(args)
            if m and int(m.group(1)) >> 24 == 1 and int(m.group(2)):
                starts[int(m.group(2))].append(t)
                plumbing.add(pid)
    return starts, plumbing


def loop_cpu(events, times, t, loop_tid, before=False, within=0.002):
    """The CPU the loop thread was switched in on, the first time after `t` (the last time before it with `before`), within `within` seconds;
    None when it was not."""
    if before:
        i = bisect.bisect_right(times, t) - 1
        while i >= 0 and times[i] >= t - within:
            if events[i][2] == 'sched_switch':
                m = SWITCH.match(events[i][5])
                if m and int(m.group(6)) == loop_tid:
                    return events[i][1]
            i -= 1
        return None
    i = bisect.bisect_left(times, t)
    while i < len(events) and times[i] <= t + within:
        if events[i][2] == 'sched_switch':
            m = SWITCH.match(events[i][5])
            if m and int(m.group(6)) == loop_tid:
                return events[i][1]
        i += 1
    return None


def clusters(ts, within=0.002):
    """The first of each run of times less than `within` seconds from the one before it: a frame's GPU jobs make one start."""
    out, last = [], None
    for t in ts:
        if last is None or t - last >= within:
            out.append(t)
        last = t
    return out


def occupants(events, times, cpu, t0, t1, skip_tids, skip_comms=('swapper', '<idle>')):
    """[(comm-tid, count)], most seen first: who the trace shows on `cpu` from t0 to t1. An event names the task that was running when it
    fired and a switch the two it changed between; threads in `skip_tids` (the loop, the client, the GPU plumbing) and idle are left out."""
    seen = collections.Counter()
    for i in range(bisect.bisect_left(times, t0), bisect.bisect_right(times, t1)):
        t, c, name, pid, comm, args = events[i]
        if c != cpu:
            continue
        who = [(comm.strip(), pid)]
        if name == 'sched_switch':
            m = SWITCH.match(args)
            if m:
                who = [(m.group(1), int(m.group(2))), (m.group(5), int(m.group(6)))]
        for cm, tid in who:
            if tid and tid not in skip_tids and not cm.startswith(skip_comms):
                seen[f'{cm}-{tid}'] += 1
    return seen.most_common()


def reactions(events, loop_tid, gap_ms=20.0, slow_ms=REACTION_SLOW_MS, period_ms=PERIOD_MS):
    """How the client answered the compositor's frame callbacks: how long after the moment they go out the client's next GPU job began (its
    wake-up, its draw and its submission). They go out when a flip lands, or, in builds that send them with the frame's draw (`ctl callbacks
    clients early`, the default since 112d9cc), when the compositor's own job for the frame starts. The trace has no event for the callbacks, so
    the moment taken is the one the client's jobs follow most closely, the flips' or the draws' (`trigger`).
    The client is the process, other than the compositor, that started a job for at least every second flip; None when no process did.
    Two consecutive answers r0 and r1 make the client's jobs `interval + r1 - r0` apart: a pair more than gap_ms apart is a swap gap of the
    client's own, whether the flips were late or not (`implied`). The reactions over slow_ms are listed (`slow`) with the CPU the loop's pass
    ran on and who else the trace shows there until the job began."""
    flips = [e[0] for e in events if e[2] == 'drm_vblank_event_delivered']
    starts, plumbing = job_starts(events)
    cands = [(len(v), tg) for tg, v in starts.items() if tg != loop_tid]
    if len(flips) < 30 or not cands or max(cands)[0] < len(flips) // 2:
        return None
    client = max(cands)[1]
    jobs = starts[client]
    first = jobs[REACTION_START] if len(jobs) > REACTION_START else jobs[-1]
    t0 = events[0][0]
    times = [e[0] for e in events]

    def answers(moments):
        """(index in moments, moment, the client's next job start, ms between them) for each moment a job followed within a period."""
        out = []
        for k, m in enumerate(moments):
            i = bisect.bisect_right(jobs, m)
            if m >= first and i < len(jobs) and (jobs[i] - m) * 1000.0 < period_ms:
                out.append((k, m, jobs[i], (jobs[i] - m) * 1000.0))
        return out

    best = None
    for name, moments in (('flip', flips), ('draw', clusters(starts.get(loop_tid, [])))):
        got = answers(moments)
        if len(got) >= 30:
            median = sorted(r[3] for r in got)[len(got) // 2]
            if best is None or median < best[0]:
                best = (median, name, got)
    if best is None:
        return None
    _, trigger, rows = best
    ms = sorted(r[3] for r in rows)
    n = len(ms)

    def pick(p):
        return round(ms[min(n - 1, int(n * p))], 2)

    def detail(row, prev):
        k, m, j, r = row
        out = {'at': round(m - t0, 3), 't': round(m, 3), 'trigger': trigger, 'ms': round(r, 2)}
        if prev and prev[0] == k - 1:
            out['prev_ms'] = round(prev[3], 2)
            out['gap_ms'] = round((j - prev[2]) * 1000.0, 1)
        cpu = loop_cpu(events, times, m, loop_tid, before=trigger == 'draw')
        if cpu is not None:
            out['cpu'] = cpu
            out['held_by'] = [f'{who} x{c}' for who, c in occupants(events, times, cpu, m, j, {loop_tid, client} | plumbing)[:4]]
        return out

    slow, implied = [], []
    for x, row in enumerate(rows):
        prev = rows[x - 1] if x else None
        if row[3] >= slow_ms:
            slow.append(detail(row, prev))
        if prev and prev[0] == row[0] - 1 and (row[2] - prev[2]) * 1000.0 > gap_ms:
            implied.append(detail(row, prev))
    span = [r[1] for r in rows]
    inside = [f for f in flips if span[0] <= f <= span[-1]]
    return {'client': client, 'trigger': trigger, 'frames': n, 'p50': pick(0.5), 'p90': pick(0.9), 'p99': pick(0.99), 'p999': pick(0.999),
            'max': round(ms[-1], 2), 'over2': sum(1 for v in ms if v > 2.0), 'over3': sum(1 for v in ms if v > 3.0),
            'flips': len(inside), 'flip_max_ms': round(max((b - a) * 1000.0 for a, b in zip(inside, inside[1:])), 2) if len(inside) > 1 else 0.0,
            'slow': slow[:20], 'implied': implied[:20], 'span': (round(span[0], 6), round(span[-1], 6))}


X1_GAP = re.compile(r'^\[x1\] gap ([\d.]+) ms before swap (\d+) returned at monotonic ([\d.]+)')


def x1_gaps(path):
    """[(ms, swap, monotonic time)] of the gap lines the X1 client wrote (`[x1] gap G ms before swap K returned at monotonic T`)."""
    with open(path, errors='replace') as f:
        return [(float(m.group(1)), int(m.group(2)), float(m.group(3))) for m in map(X1_GAP.match, f) if m]


def describe(r):
    """One line for a late frame's attribution."""
    head = f"{r['ms']} ms (overrun {r['overrun_ms']}): "
    if r['kernel']:
        parts = [f"{c} {ms} ms" for c, ms in r['by_class'].items()]
        chain = ', '.join(f"{x['class']} on {x['comm']}-{x['tid']}" + (f' (hop {x["hop"]})' if x['hop'] else '') for x in r['waits'][:4])
        return head + f"kernel wait {r['kernel_ms']} ms = {int(r['score'] * 100)} % of it: {'; '.join(parts)} [{chain}]"
    why = f"run-queue wait {r['sched_ms']} ms" + (f" behind {', '.join(r['sched_behind'])}" if r['sched_behind'] else '') if r['verdict'] == 'sched' else 'no kernel wait of 3 ms or more'
    return head + f"not the kernel ({why}; kernel {r['kernel_ms']} ms)"


def summarize(frames):
    """Counts over a list of `Context.frame` results, for a gate line."""
    n = len(frames)
    kernel = [f for f in frames if f['verdict'] == 'kernel']
    by = collections.Counter(f['class'] for f in kernel)
    return {'late': n, 'kernel': len(kernel), 'sched': sum(1 for f in frames if f['verdict'] == 'sched'),
            'unexplained': sum(1 for f in frames if f['verdict'] == 'unexplained'), 'by_class': dict(by.most_common())}


def summary_line(s):
    if not s['late']:
        return 'kernel waits: no late frame'
    classes = ', '.join(f'{c} {n}' for c, n in s['by_class'].items())
    return (f"kernel waits: {s['late']} late frame(s): {s['kernel']} kernel-caused" + (f' ({classes})' if classes else '')
            + f", {s['sched']} waiting for a CPU, {s['unexplained']} unexplained")


WHAT = {'flip': 'a flip', 'draw': "the compositor's draw"}


def describe_reaction(x):
    """One line for a reaction the client took long over: when, how long, the frames it left apart, and who else the trace shows on the loop's CPU."""
    s = f"+{x['at']} s: {x['trigger']} {x['t']:.3f}, the client's next GPU job began {x['ms']} ms after it"
    if 'prev_ms' in x:
        s += f" (the frame before: {x['prev_ms']} ms), its jobs {x['gap_ms']} ms apart"
    if x.get('held_by'):
        s += f"; CPU {x['cpu']} also ran {', '.join(x['held_by'])}"
    elif 'cpu' in x:
        s += f"; CPU {x['cpu']} ran nothing else the trace shows"
    return s


def client_lines(c, gap_ms):
    """The lines for the client's reactions: the spread, then each reaction over 3 ms."""
    out = [f"client {c['client']}: its next GPU job began after {WHAT[c['trigger']]} (where its frame callbacks go out) p50 {c['p50']} ms, "
           f"p90 {c['p90']}, p99 {c['p99']}, p99.9 {c['p999']}, max {c['max']} ({c['frames']} answered, the flips in that stretch at most "
           f"{c['flip_max_ms']} ms apart); {c['over3']} over 3 ms, {len(c['implied'])} would leave its jobs more than {gap_ms:g} ms apart"]
    return out + ['  ' + describe_reaction(x) for x in c['slow']]


def client_summary(c, gap_ms):
    """The client's part of the one line a gate prints."""
    return (f"client {c['client']} reaction after {WHAT[c['trigger']]}: p50 {c['p50']} ms, p99.9 {c['p999']}, max {c['max']}; {c['over3']} over 3 ms, "
            f"{len(c['implied'])} frame gap(s) over {gap_ms:g} ms implied; answered {c['frames']}, flips at most {c['flip_max_ms']} ms apart")


def x1_lines(gaps, c, gap_ms):
    """The X1 client's own gap lines beside the gaps the trace implies: paired in order when there are as many of one as of the other."""
    implied = c['implied'] if c else []
    out = [f"x1 log: {len(gaps)} gap(s) over {gap_ms:g} ms" + (f" ({', '.join(f'{g[0]} ms' for g in gaps)})" if gaps else '')
           + f"; the trace implies {len(implied)}" + (f" ({', '.join(str(x['gap_ms']) + ' ms' for x in implied)})" if implied else '')]
    if gaps and len(gaps) == len(implied):
        out += [f"  gap {ms} ms before swap {swap} = {describe_reaction(x)}" for (ms, swap, _t), x in zip(gaps, implied)]
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('trace')
    ap.add_argument('--tid', type=int, help='the loop thread (the compositor process id)')
    ap.add_argument('--comm', help="or its name's start: tried before the trace's own `loop` marker")
    ap.add_argument('--gap', type=float, default=20.0)
    ap.add_argument('--period', type=float, default=PERIOD_MS)
    ap.add_argument('--hops', type=int, default=HOPS)
    ap.add_argument('--min-wait', type=float, default=MIN_WAIT_MS)
    ap.add_argument('--x1', help="the X1 client's log: its gap lines are set beside the gaps the trace implies")
    ap.add_argument('--json')
    a = ap.parse_args(argv)
    events, stacks = parse(a.trace)
    if not events:
        sys.exit('empty trace')
    if a.tid is not None:
        tid, how = a.tid, '--tid'
    else:
        tid, how = find_loop(events, a.comm)
        if tid is None:
            sys.exit(f'the loop thread is not in the trace ({how}): pass --tid')
    print(f'loop thread {tid} ({how})')
    ctx = Context(events, stacks, tid)
    client = reactions(events, tid, a.gap)
    frames = []
    for g0, g1 in late_frames(events, a.gap, span=client['span'] if client else None):
        r = ctx.frame(g0, g1, a.period, a.hops, a.min_wait)
        r['at'] = round(g0 - events[0][0], 3)
        frames.append(r)
        print(f"+{r['at']} s: {describe(r)}")
    if client:
        print('\n'.join(client_lines(client, a.gap)))
    gaps = None
    if a.x1:
        try:
            gaps = x1_gaps(a.x1)
        except OSError as e:
            print(f'x1 log not read: {e}')
    if gaps is not None:
        print('\n'.join(x1_lines(gaps, client, a.gap)))
    print(summary_line(summarize(frames)) + (f' | {client_summary(client, a.gap)}' if client else ''))
    if a.json:
        with open(a.json, 'w') as f:
            json.dump({'tid': tid, 'how': how, 'frames': frames, 'summary': summarize(frames), 'client': client,
                       'x1': [{'ms': g[0], 'swap': g[1], 'at': g[2]} for g in gaps] if gaps is not None else None}, f, indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
