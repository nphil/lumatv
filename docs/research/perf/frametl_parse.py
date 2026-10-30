import re, sys, collections, bisect

LINE = re.compile(r'^\s*(.+?)-(\d+)\s+\[(\d+)\]\s+\S+\s+([0-9.]+):\s+(\w+):\s*(.*)$')
SW = re.compile(r'prev_comm=(.*?) prev_pid=(\d+) prev_prio=\d+ prev_state=(\S+) ==> next_comm=(.*?) next_pid=(\d+) next_prio=\d+')
JOB = re.compile(r'gpu=(\d+) event=(\d+) tgid=(\d+) pid=(\d+) job_id=(\d+)')

class Trace:
    def __init__(self, path):
        self.runs = collections.defaultdict(list)   # cpu -> [(t0,t1,pid,comm)]
        self.flips = []     # vblank_event_delivered times
        self.vblanks = []
        self.jobs = []      # (t, event, tgid, pid, job)
        self.marks = []     # (t, text)
        self.comm = {}
        cur = {}
        with open(path, errors='replace') as f:
            for line in f:
                m = LINE.match(line)
                if not m:
                    continue
                task, pid, cpu, ts, ev, rest = m.groups()
                t = float(ts); cpu = int(cpu)
                if ev == 'sched_switch':
                    s = SW.search(rest)
                    if not s: continue
                    pc, pp, ps, nc, np_, = s.group(1), int(s.group(2)), s.group(3), s.group(4), int(s.group(5))
                    self.comm[pp] = pc; self.comm[np_] = nc
                    if cpu in cur:
                        t0, p0, c0 = cur[cpu]
                        if p0 != 0:
                            self.runs[cpu].append((t0, t, p0, c0))
                    cur[cpu] = (t, np_, nc)
                elif ev == 'drm_vblank_event_delivered':
                    self.flips.append(t)
                elif ev == 'drm_vblank_event':
                    self.vblanks.append(t)
                elif ev == 'mali_job_slots_event':
                    j = JOB.search(rest)
                    if j:
                        self.jobs.append((t, int(j.group(2)), int(j.group(3)), int(j.group(4)), int(j.group(5)), int(j.group(1))))
                elif ev == 'tracing_mark_write':
                    self.marks.append((t, rest.strip()))
        for cpu in self.runs:
            self.runs[cpu].sort()
        self.flips.sort()

def overlap(runs, a, b):
    out = []
    for t0, t1, pid, comm in runs:
        if t1 <= a or t0 >= b: continue
        out.append((max(t0, a), min(t1, b), pid, comm))
    return out
