#!/usr/bin/env python3
"""On-device helper for tools/declutter.sh: JSON through luna-send, and measurements.

    dc.py get <category.key>          current value of one setting, as JSON, or __missing__
    dc.py set <category.key> <json>   set one setting; prints ok or the error
    dc.py apps                        installed app ids, one per line (SAM's listApps)
    dc.py strip <file> <id>...        drop app ids from every list in a JSON file; prints how many
    dc.py deleted <file> <id>...      add app ids to a deletedList JSON file; prints how many
    dc.py measure [seconds]           one JSON object: memory, resident sizes, CPU over the interval
"""
import json
import os
import subprocess
import sys
import time


def luna(uri, payload, timeout=15):
    try:
        p = subprocess.run(['timeout', str(timeout), 'luna-send', '-n', '1', uri, json.dumps(payload)],
                           stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, stdin=subprocess.DEVNULL)
        return json.loads(p.stdout.decode('utf-8', 'replace') or '{}')
    except Exception as e:  # a missing service or bad JSON is an answer, not a crash
        return {'returnValue': False, 'errorText': str(e)}


def split_key(ck):
    cat, _, key = ck.partition('.')
    return cat, key


def cmd_get(ck):
    cat, key = split_key(ck)
    r = luna('luna://com.webos.settingsservice/getSystemSettings', {'category': cat, 'keys': [key]})
    s = r.get('settings') or {}
    print(json.dumps(s[key]) if key in s else '__missing__')


def cmd_set(ck, value):
    cat, key = split_key(ck)
    r = luna('luna://com.webos.settingsservice/setSystemSettings',
             {'category': cat, 'settings': {key: json.loads(value)}})
    print('ok' if r.get('returnValue') else 'error: ' + str(r.get('errorText') or r))


def cmd_apps():
    r = luna('luna://com.webos.applicationManager/listApps', {})
    for a in r.get('apps') or []:
        print(a.get('id', ''))


def drop_ids(node, ids):
    n = 0
    if isinstance(node, dict):
        for k in list(node):
            n += drop_ids(node[k], ids)
    elif isinstance(node, list):
        keep = [x for x in node if not (isinstance(x, str) and x in ids)]
        n += len(node) - len(keep)
        node[:] = keep
        for x in node:
            n += drop_ids(x, ids)
    return n


def cmd_strip(path, ids):
    with open(path) as f:
        d = json.load(f)
    n = drop_ids(d, set(ids))
    if n:
        with open(path, 'w') as f:
            json.dump(d, f, separators=(',', ':'))
    print(n)


def cmd_deleted(path, ids):
    try:
        with open(path) as f:
            d = json.load(f)
    except (OSError, ValueError):
        d = {}
    lst = d.setdefault('deletedList', [])
    add = [i for i in ids if i not in lst]
    if add:
        lst.extend(add)
        with open(path, 'w') as f:
            json.dump(d, f, separators=(',', ':'))
    print(len(add))


def rd(path):
    try:
        with open(path) as f:
            return f.read()
    except OSError:
        return ''


def sample():
    """pid -> (comm, unit, cpu ticks, pss kB, swap kB)."""
    out = {}
    for d in os.listdir('/proc'):
        if not d.isdigit():
            continue
        st = rd('/proc/%s/stat' % d)
        if not st:
            continue
        try:
            rp = st.rindex(')')
            comm = st[st.index('(') + 1:rp]
            f = st[rp + 2:].split()
            ticks = int(f[11]) + int(f[12])
        except (ValueError, IndexError):
            continue
        pss = swap = 0
        for line in rd('/proc/%s/smaps_rollup' % d).splitlines():
            if line.startswith('Pss:'):
                pss = int(line.split()[1])
            elif line.startswith('Swap:'):
                swap = int(line.split()[1])
        cg = rd('/proc/%s/cgroup' % d).strip().split('\n')[-1].split(':')[-1].rsplit('/', 1)[-1]
        out[int(d)] = (comm, cg or '-', ticks, pss, swap)
    return out


def busy_ticks():
    f = rd('/proc/stat').split('\n')[0].split()[1:]
    v = [int(x) for x in f]
    idle = v[3] + (v[4] if len(v) > 4 else 0)
    return sum(v[:8]) - idle, sum(v[:8])


def cmd_measure(secs):
    hz = os.sysconf('SC_CLK_TCK')
    a, (b0, t0) = sample(), busy_ticks()
    time.sleep(secs)
    b, (b1, t1) = sample(), busy_ticks()
    mem = {}
    for line in rd('/proc/meminfo').splitlines():
        k, _, v = line.partition(':')
        mem[k] = int(v.split()[0]) if v.split() else 0
    units = {}
    cpu = {}
    comms = {}
    for pid, (comm, cg, ticks, pss, swap) in b.items():
        u = units.setdefault(cg, [0, 0, 0])
        u[0] += pss
        u[1] += swap
        d = ticks - a[pid][2] if pid in a else 0
        u[2] += d
        cpu[comm] = cpu.get(comm, 0) + d
        if pss or swap:
            c = comms.setdefault(comm, [0, 0, 0])
            c[0] += pss
            c[1] += swap
            c[2] += 1
    span = secs * hz
    res = {
        'secs': secs,
        'memAvailableKb': mem.get('MemAvailable'),
        'memFreeKb': mem.get('MemFree'),
        'swapUsedKb': mem.get('SwapTotal', 0) - mem.get('SwapFree', 0),
        'cachedKb': mem.get('Cached'),
        'procs': len(b),
        'pssTotalKb': sum(u[0] for u in units.values()),
        'pssLsHubdKb': units.get('ls-hubd.service', [0])[0],
        # CPUs go offline and online; their counters can step backwards, so a wild value is dropped
        'cpuBusyPct': round(100.0 * (b1 - b0) / max(1, t1 - t0), 2) if 0 <= b1 - b0 <= t1 - t0 else None,
        'cpuProcsPct': round(100.0 * sum(cpu.values()) / span, 2),
        'topCpuPct': {k: round(100.0 * v / span, 2) for k, v in sorted(cpu.items(), key=lambda x: -x[1])[:8] if v},
        'units': units,
        'comms': comms,
    }
    print(json.dumps(res, separators=(',', ':')))


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    c = argv[1]
    if c == 'get' and len(argv) == 3:
        cmd_get(argv[2])
    elif c == 'set' and len(argv) == 4:
        cmd_set(argv[2], argv[3])
    elif c == 'apps':
        cmd_apps()
    elif c == 'strip' and len(argv) >= 4:
        cmd_strip(argv[2], argv[3:])
    elif c == 'deleted' and len(argv) >= 4:
        cmd_deleted(argv[2], argv[3:])
    elif c == 'measure':
        cmd_measure(int(argv[2]) if len(argv) > 2 else 30)
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
