#!/usr/bin/env python3
"""Stands in for the helper's capture socket for one window of the light-sync probe slot (ambient/capture/tv_probe_slot.py), on the set.

    slot_collect.py SOCKET SECS LABEL OUT.json [--pid PID] [--preset N]

It listens on SOCKET, waits for lumatv-capture to connect, tells it the preset (as the helper does), reads its frames for SECS
seconds (ambient/src/wire.rs: 12 byte header, 12 bytes an area) and writes what they say to OUT.json: how many frames, how many
differed from the one before, how long between them, the capture service's CPU (from /proc/PID/stat) and the last colours of the
areas. Closing the socket at the end is what makes the service give the capture back. Python 3 standard library only: the set has it.
Exit 0 with frames, 2 when the service never connected, 3 when it connected and sent nothing.
"""
import json
import os
import socket
import struct
import sys
import time

HEADER, ZONE = 12, 12
ZONES = {0: 16, 1: 16, 2: 16, 3: 14}


def ticks(pid):
    """utime + stime of the process, in clock ticks (None if it is gone)."""
    try:
        with open("/proc/%d/stat" % pid) as f:
            fields = f.read().rsplit(")", 1)[1].split()
        return int(fields[11]) + int(fields[12])
    except (OSError, ValueError, IndexError):
        return None


def read_exact(conn, n, deadline):
    data = b""
    while len(data) < n:
        left = deadline - time.time()
        if left <= 0:
            return None
        conn.settimeout(min(left, 1.0))
        try:
            chunk = conn.recv(n - len(data))
        except socket.timeout:
            continue
        if not chunk:
            return None
        data += chunk
    return data


def percentile(values, p):
    if not values:
        return 0
    values = sorted(values)
    return values[int(p * (len(values) - 1) + 0.5)]


def main(argv):
    pid = None
    preset = 0
    args = []
    it = iter(argv)
    for a in it:
        if a == "--pid":
            pid = int(next(it))
        elif a == "--preset":
            preset = int(next(it))
        else:
            args.append(a)
    path, secs, label, out = args[0], float(args[1]), args[2], args[3]
    report = {"label": label, "connected": False, "frames": 0, "changed": 0, "seconds": 0.0}
    try:
        os.unlink(path)
    except FileNotFoundError:
        pass
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    listener.bind(path)
    listener.listen(1)
    listener.settimeout(15)
    try:
        conn, _ = listener.accept()
    except socket.timeout:
        with open(out, "w") as f:
            json.dump(report, f)
        return 2
    report["connected"] = True
    conn.sendall(bytes([preset]))
    ticks0, t_start = (ticks(pid) if pid else None), time.time()
    deadline = t_start + secs
    arrivals, last_body, per_zone = [], None, {}
    last_frame = None
    closed_by = "deadline"
    while time.time() < deadline:
        head = read_exact(conn, HEADER, deadline)
        if head is None:
            closed_by = "eof" if time.time() < deadline else "deadline"
            break
        if head[:4] != b"AMB1" or head[4] not in ZONES or head[5] != ZONES[head[4]]:
            closed_by = "bad frame"
            break
        body = read_exact(conn, head[5] * ZONE, deadline + 1)
        if body is None:
            closed_by = "eof"
            break
        arrivals.append(time.time())
        report["frames"] += 1
        if body != last_body:
            report["changed"] += 1
        last_body = body
        last_frame = (head, body)
        for i in range(head[5]):
            r, g, b, black, count = struct.unpack_from("<HHHHI", body, i * ZONE)
            luma = (0.2126 * r + 0.7152 * g + 0.0722 * b) / 65535 * 255
            lo, hi = per_zone.get(i, (1e9, -1e9))
            per_zone[i] = (min(lo, luma), max(hi, luma))
    elapsed = time.time() - t_start
    ticks1 = ticks(pid) if pid else None
    report["seconds"] = round(elapsed, 2)
    report["closed_by"] = closed_by
    report["frames_per_s"] = round(report["frames"] / elapsed, 1) if elapsed else 0
    report["changes_per_s"] = round(report["changed"] / elapsed, 1) if elapsed else 0
    gaps = [round((b - a) * 1000, 1) for a, b in zip(arrivals, arrivals[1:])]
    report["interval_ms"] = {"p50": percentile(gaps, 0.5), "p90": percentile(gaps, 0.9), "p99": percentile(gaps, 0.99), "max": max(gaps) if gaps else 0}
    if ticks0 is not None and ticks1 is not None and elapsed:
        report["service_cpu_pct"] = round(100.0 * (ticks1 - ticks0) / os.sysconf("SC_CLK_TCK") / elapsed, 2)
    if last_frame:
        head, body = last_frame
        report["preset"] = head[4]
        zones = []
        for i in range(head[5]):
            r, g, b, black, count = struct.unpack_from("<HHHHI", body, i * ZONE)
            lo, hi = per_zone[i]
            zones.append({"area": i, "r": round(r / 65535 * 255, 1), "g": round(g / 65535 * 255, 1), "b": round(b / 65535 * 255, 1),
                          "black": round(black / 65535, 3), "count": count, "luma_min": round(lo, 1), "luma_max": round(hi, 1)})
        report["zones"] = zones
    conn.close()      # the service finalizes when it sees this
    listener.close()
    try:
        os.unlink(path)
    except OSError:
        pass
    with open(out, "w") as f:
        json.dump(report, f, indent=1)
    print("frames=%d changed=%d (%.1f/s) cpu=%s%% closed_by=%s" % (report["frames"], report["changed"], report["changes_per_s"],
                                                             report.get("service_cpu_pct", "?"), closed_by))
    return 0 if report["frames"] else 3


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
