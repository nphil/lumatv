#!/usr/bin/env python3
"""The 1 s system sampler of capture.sh, in one process that forks nothing.

    sample.py OUT PID [DIR]

Writes the same line capture.sh's shell sampler wrote (uptime, per-CPU jiffies, pressure, memory,
vm counters, GPU utilisation and state, the compositor's faults, times and RSS, online CPUs,
temperature and LG's load line, `|`-separated) once a second to OUT until DIR/.running is gone
(DIR defaults to /tmp/perf). The shell version forked about 40 processes a second (grep, tr, sed,
cat, cut, awk); on the set those forks and the scheduler's exec-time migrations used 50-60 ms of
CPU inside the first window of every full capture, which is exactly where the gate measures the
first frame after a key.
"""
import os
import re
import sys
import time

VM_KEYS = ('pgscan_kswapd', 'pgsteal_kswapd', 'pgscan_direct', 'pgsteal_direct', 'allocstall_normal', 'allocstall_movable',
           'pswpin', 'pswpout', 'pgmajfault', 'compact_stall', 'nr_free_cma')
MEM_KEYS = ('MemFree', 'MemAvailable', 'CmaFree', 'SwapFree', 'Dirty')
CPU_LINE = re.compile(r'^cpu\d')
SPACES = re.compile(r' +')


def read(path):
    try:
        with open(path, errors='replace') as f:
            return f.read()
    except OSError:
        return ''


def first_line(text, n=0):
    lines = text.split('\n')
    return lines[n] if n < len(lines) else ''


def one_line(text):
    """What `$(cat file)` gives: the text without its trailing newlines."""
    return text.rstrip('\n')


def sample(pid):
    up = first_line(read('/proc/uptime')).split(' ')[0]
    stat = read('/proc/stat').split('\n')
    cpu = ''.join(ln + ';' for ln in stat if CPU_LINE.match(ln))
    psi = '|'.join((first_line(read('/proc/pressure/cpu')), first_line(read('/proc/pressure/memory')),
                    first_line(read('/proc/pressure/memory'), 1), first_line(read('/proc/pressure/io'))))
    mem = ''
    for ln in read('/proc/meminfo').split('\n'):
        if ln.split(':', 1)[0] in MEM_KEYS and ':' in ln:
            mem += SPACES.sub(' ', ln) + ';'
    vm = ''
    for ln in read('/proc/vmstat').split('\n'):
        if ln.split(' ', 1)[0] in VM_KEYS:
            vm += ln + ';'
    gpu = read('/sys/kernel/debug/mali0/dvfs_utilization').replace('\n', ' ')
    gpust = one_line(read('/sys/class/misc/mali0/device/power/runtime_status'))
    sm = ''
    f = read('/proc/%d/stat' % pid).split()
    if len(f) >= 24:
        sm = 'minflt=%s majflt=%s utime=%s stime=%s rss=%s' % (f[9], f[11], f[13], f[14], f[23])
    online = one_line(read('/sys/devices/system/cpu/online'))
    temp = one_line(read('/proc/lg/pm/temperature'))
    lgload = ''
    for ln in read('/proc/lg/pm/status').split('\n'):
        if ln.startswith('load:'):
            lgload = SPACES.sub(' ', ln)
            break
    return '%s|%s|%s|%s|%s|gpu=%s|gpust=%s|%s|online=%s|temp=%s|%s' % (up, cpu, psi, mem, vm, gpu, gpust, sm, online, temp, lgload)


def main(argv):
    out, pid = argv[1], int(argv[2])
    running = os.path.join(argv[3] if len(argv) > 3 else '/tmp/perf', '.running')
    with open(out, 'w') as f:
        while os.path.exists(running):
            f.write(sample(pid) + '\n')
            f.flush()
            time.sleep(1)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv))
