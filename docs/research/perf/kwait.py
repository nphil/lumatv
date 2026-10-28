#!/usr/bin/env python3
"""Which kernel waits made a frame late (LPC 2026, "Kernel Lock Contention Hotspots Causing Frame Drops on Android").

    kwait.py TRACE (--tid TID | --comm NAME) [--gap MS] [--period MS] [--hops N] [--json OUT]

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

`analyse.py` calls this for every gap it lists; `gate.py` prints the result under each surface.
"""
import argparse
import collections
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
    """(events, stacks) from a trace dump: events as analyse.py's `(t, cpu, name, pid, comm, args)` sorted by time, stacks as
    {index into events of a sched_switch: [symbol, ...]} for the switches a `stacktrace` trigger followed."""
    with open(path, errors='replace') as f:
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


def late_frames(events, gap_ms=20.0, max_gap_ms=MAX_GAP_MS):
    """[(g0, g1)] of the presented frames more than gap_ms and no more than max_gap_ms after the one before (a longer silence is a still screen,
    not a frame that was late)."""
    t = [e[0] for e in events if e[2] == 'drm_vblank_event_delivered']
    return [(a, b) for a, b in zip(t, t[1:]) if gap_ms < (b - a) * 1000.0 <= max_gap_ms]


def find_loop(events, comm):
    """The thread id a comm has in the trace (the busiest, when several), None when it never ran."""
    c = collections.Counter()
    for t, cpu, name, pid, ec, args in events:
        if name == 'sched_switch':
            m = SWITCH.match(args)
            if m:
                if m.group(5).startswith(comm):
                    c[int(m.group(6))] += 1
                if m.group(1).startswith(comm):
                    c[int(m.group(2))] += 1
    return c.most_common(1)[0][0] if c else None


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


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('trace')
    ap.add_argument('--tid', type=int, help='the loop thread (the compositor process id)')
    ap.add_argument('--comm', help="or its name's start (lumatv-composit)")
    ap.add_argument('--gap', type=float, default=20.0)
    ap.add_argument('--period', type=float, default=PERIOD_MS)
    ap.add_argument('--hops', type=int, default=HOPS)
    ap.add_argument('--min-wait', type=float, default=MIN_WAIT_MS)
    ap.add_argument('--json')
    a = ap.parse_args(argv)
    events, stacks = parse(a.trace)
    if not events:
        sys.exit('empty trace')
    tid = a.tid if a.tid is not None else find_loop(events, a.comm or 'lumatv-composit')
    if tid is None:
        sys.exit('the loop thread is not in the trace: pass --tid')
    ctx = Context(events, stacks, tid)
    frames = []
    for g0, g1 in late_frames(events, a.gap):
        r = ctx.frame(g0, g1, a.period, a.hops, a.min_wait)
        r['at'] = round(g0 - events[0][0], 3)
        frames.append(r)
        print(f"+{r['at']} s: {describe(r)}")
    print(summary_line(summarize(frames)))
    if a.json:
        with open(a.json, 'w') as f:
            json.dump({'tid': tid, 'frames': frames, 'summary': summarize(frames)}, f, indent=1)
    return 0


if __name__ == '__main__':
    sys.exit(main())
