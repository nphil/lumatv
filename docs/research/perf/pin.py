#!/usr/bin/env python3
"""Pin the compositor's resident file pages so reclaim cannot evict them.

    pin.py [--all]      lock, then sleep until killed (run with nohup &)

Maps every file the compositor has mapped, finds the ranges that are
resident right now (mincore), and mlocks those ranges in this process.
Page cache pages are shared, so locking them here keeps them resident for
the compositor too. --all locks the whole of each mapping instead
(pulls everything in). Prints what it locked; SIGTERM releases it.
"""
import ctypes
import mmap
import os
import signal
import subprocess
import sys
import time

libc = ctypes.CDLL(None, use_errno=True)
libc.mlock.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
libc.mincore.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_char_p]
PAGE = mmap.PAGESIZE

pid = subprocess.check_output(['pidof', 'surface-manager']).split()[0].decode()
ranges = {}
for line in open(f'/proc/{pid}/maps'):
    f = line.split()
    if len(f) < 6 or not f[5].startswith('/') or f[5].startswith('/dev/'):
        continue
    a, b = (int(x, 16) for x in f[0].split('-'))
    off = int(f[2], 16)
    ranges.setdefault(f[5], []).append((off, b - a))

keep = []          # (mmap object) kept alive so the locks persist
locked = 0
for path, segs in ranges.items():
    try:
        fd = os.open(path, os.O_RDONLY)
        size = os.fstat(fd).st_size
    except OSError:
        continue
    for off, length in segs:
        length = min(length, max(size - off, 0))
        if length <= 0:
            continue
        try:
            m = mmap.mmap(fd, length, mmap.MAP_SHARED, mmap.PROT_READ, offset=off)
        except (OSError, ValueError):
            continue
        base = ctypes.addressof(ctypes.c_char.from_buffer(m))
        npages = (length + PAGE - 1) // PAGE
        if '--all' in sys.argv:
            if libc.mlock(base, length) == 0:
                locked += length
                keep.append(m)
            continue
        vec = ctypes.create_string_buffer(npages)
        if libc.mincore(base, length, vec) != 0:
            continue
        # lock runs of resident pages
        i = 0
        got = 0
        while i < npages:
            if vec[i] == b'\x00':
                i += 1
                continue
            j = i
            while j < npages and vec[j] != b'\x00':
                j += 1
            if libc.mlock(base + i * PAGE, (j - i) * PAGE) == 0:
                got += (j - i) * PAGE
            i = j
        if got:
            locked += got
            keep.append(m)
    os.close(fd)

print(f'pinned {locked // 1024} kB of the compositor\'s file pages across {len(keep)} mappings', flush=True)
signal.signal(signal.SIGTERM, lambda *a: sys.exit(0))
while True:
    time.sleep(3600)
