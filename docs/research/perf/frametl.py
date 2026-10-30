import pickle, sys, collections, bisect
sys.path.insert(0, '/tmp')
import tr
d = pickle.load(open(sys.argv[1], 'rb'))
A = float(sys.argv[2]); lo = float(sys.argv[3]); hi = float(sys.argv[4])
runs = d['runs']; flips = d['flips']; jobs = d['jobs']
COMP, UI = 8969, 9478
def segs(pid, a, b):
    out = []
    for cpu, rr in runs.items():
        for t0, t1, p, c in tr.overlap(rr, a, b):
            if p == pid: out.append((t0, t1, cpu))
    out.sort()
    # merge adjacent
    m = []
    for s in out:
        if m and s[0] - m[-1][1] < 0.0002: m[-1] = (m[-1][0], s[1], m[-1][2])
        else: m.append(s)
    return m
# GPU jobs: event 16842752 = start? 16777216 = ? Inspect: pair start/end per slot by job_id
evs = collections.Counter(j[1] for j in jobs)
print('job events', evs.most_common(6), file=sys.stderr)
fl = [f for f in flips if A + lo <= f <= A + hi]
prev = None
for f in fl:
    if prev is not None:
        a, b = prev, f
        c = segs(COMP, a, b); u = segs(UI, a, b)
        fmt = lambda s: ' '.join('%.1f-%.1f(%.1f)' % ((x[0] - A) * 1000 % 100000 - 0, (x[1] - A) * 1000, (x[1] - x[0]) * 1000) for x in s)
        print('flip %+.4f  +%5.1f ms | comp %s | ui %s' % (f - A, (f - prev) * 1000,
              ' '.join('[%+.1f..%+.1f %.1fms]' % ((x[0] - prev) * 1000, (x[1] - prev) * 1000, (x[1] - x[0]) * 1000) for x in c),
              ' '.join('[%+.1f..%+.1f %.1fms]' % ((x[0] - prev) * 1000, (x[1] - prev) * 1000, (x[1] - x[0]) * 1000) for x in u)))
    prev = f
