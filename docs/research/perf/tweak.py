#!/usr/bin/env python3
"""Scheduler and affinity tweaks for the compositor, run on the set.

    tweak.py fifo [PRIO]      GUI thread SCHED_FIFO (default prio 1)
    tweak.py other            back to SCHED_OTHER, nice -19 as stock
    tweak.py affinity MASK    GUI thread CPU mask, e.g. 2 (CPU1) or f
    tweak.py show             current policy, nice and mask

Only the compositor's main (GUI/render) thread is changed; the driver and
helper threads keep their stock settings.
"""
import os
import subprocess
import sys

pid = int(subprocess.check_output(['pidof', 'surface-manager']).split()[0])
cmd = sys.argv[1] if len(sys.argv) > 1 else 'show'

if cmd == 'fifo':
    prio = int(sys.argv[2]) if len(sys.argv) > 2 else 1
    os.sched_setscheduler(pid, os.SCHED_FIFO, os.sched_param(prio))
elif cmd == 'other':
    os.sched_setscheduler(pid, os.SCHED_OTHER, os.sched_param(0))
    os.setpriority(os.PRIO_PROCESS, pid, -19)
elif cmd == 'affinity':
    mask = int(sys.argv[2], 16)
    os.sched_setaffinity(pid, {c for c in range(8) if mask >> c & 1})

pol = os.sched_getscheduler(pid)
names = {os.SCHED_OTHER: 'other', os.SCHED_FIFO: 'fifo', os.SCHED_RR: 'rr'}
print(f'pid {pid} policy {names.get(pol, pol)} prio {os.sched_getparam(pid).sched_priority} '
      f'nice {os.getpriority(os.PRIO_PROCESS, pid)} cpus {sorted(os.sched_getaffinity(pid))}')
