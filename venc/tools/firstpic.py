#!/usr/bin/env python3
"""Time from request to the first I picture of a station's programme, measured here.

tune.py HOST:PORT STATION OTHER [ROUNDS] [SIBLING]

Each round, for each of `cold` and `same-rf`, alternates `plain` and `heavy`: the tuner is first moved (cold: to OTHER on another RF; same-rf:
to SIBLING on STATION's RF), the stream is dropped, and STATION is requested. The clock starts when the request is sent and stops at the TS
packet that carries the first I picture of the programme, and again at the end of that picture.
Plain (MPEG-2): the stream opens with the helper's slate, an I picture of about 30 kB every half second with small P pictures between, on the
programme's clock; the programme is the first I picture larger than BIG bytes (its own are 100-250 kB at 720p). Heavy (H.264): no slate; the
first IDR slice.
"""
import os, socket, statistics, sys, time

BIG = int(os.environ.get("BIG", 60000))
LIMIT = 8.0


class Probe:
    """Finds the first I picture of the programme in one video PID's bytes, and when it arrived."""

    def __init__(self, codec):
        self.codec = codec
        self.buf = bytearray()
        self.times = []  # (offset where a chunk starts, arrival time)
        self.scan = 0
        self.cur = None  # mpeg2: (offset, type, arrival) of the picture being read
        self.start = None  # h264: offset of the IDR slice
        self.t_start = self.t_end = None

    def at(self, offset):
        t = self.times[0][1]
        for off, when in self.times:
            if off <= offset:
                t = when
        return t

    def add(self, payload, now):
        self.times.append((len(self.buf), now))
        self.buf += payload
        b = self.buf
        i = max(self.scan - 3, 0)
        while True:
            j = b.find(b"\x00\x00\x01", i)
            if j < 0 or j + 5 >= len(b):
                break
            code = b[j + 3]
            if self.codec == "mpeg2":
                if code == 0x00:
                    if self.cur and self.cur[1] == 1 and j - self.cur[0] > BIG:
                        self.t_start, self.t_end = self.cur[2], self.at(j)
                        return True
                    self.cur = (j, (b[j + 5] >> 3) & 7, self.at(j))
            elif self.start is None:
                if code & 0x1F == 5 and code & 0x80 == 0:  # H.264 IDR slice
                    self.start, self.t_start = j, self.at(j)
            elif self.t_end is None and j > self.start and code & 0x1F not in (5, 7, 8) and code & 0x80 == 0:
                self.t_end = self.at(j)
                return True
            i = j + 1
        self.scan = len(b)
        return False


def first_picture(host, port, path, codec, limit=LIMIT):
    """(seconds to the first I picture's first byte, seconds to its end) or None."""
    s = socket.create_connection((host, port), timeout=limit)
    t0 = time.monotonic()
    s.sendall(f"GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n".encode())
    raw = bytearray()
    probes = {}
    header_done = False
    try:
        while time.monotonic() - t0 < limit:
            try:
                chunk = s.recv(65536)
            except socket.timeout:
                break
            if not chunk:
                break
            now = time.monotonic()
            raw += chunk
            if not header_done:
                k = raw.find(b"\r\n\r\n")
                if k < 0:
                    continue
                if not raw.startswith(b"HTTP/1.1 200") and not raw.startswith(b"HTTP/1.0 200"):
                    return None
                del raw[: k + 4]
                header_done = True
            while len(raw) >= 188 and raw[0] != 0x47:
                del raw[0]
            n = len(raw) // 188
            for p in range(n):
                pk = raw[p * 188 : p * 188 + 188]
                if pk[0] != 0x47:
                    continue
                pid = ((pk[1] & 0x1F) << 8) | pk[2]
                afc = (pk[3] >> 4) & 3
                if not afc & 1:
                    continue
                off = 4 + (1 + pk[4] if afc & 2 else 0)
                payload = bytes(pk[off:])
                if pk[1] & 0x40 and payload[:3] == b"\x00\x00\x01" and 0xE0 <= payload[3] <= 0xEF:
                    payload = payload[9 + payload[8] :]
                    if pid not in probes:
                        probes[pid] = Probe(codec)
                if pid in probes and probes[pid].add(payload, now):
                    pr = probes[pid]
                    return pr.t_start - t0, pr.t_end - t0
            del raw[: n * 188]
        for pr in probes.values():
            if pr.t_start is not None:
                return pr.t_start - t0, (pr.t_end - t0) if pr.t_end else None
        return None
    finally:
        s.close()


def stats(xs):
    xs = [x for x in xs if x is not None]
    if not xs:
        return "n/a"
    return f"median {statistics.median(xs):.2f} s, worst {max(xs):.2f} s, best {min(xs):.2f} s (n={len(xs)})"


def main():
    host, port = sys.argv[1].rsplit(":", 1)
    port = int(port)
    station, other = sys.argv[2], sys.argv[3]
    rounds = int(sys.argv[4]) if len(sys.argv) > 4 else 5
    sibling = sys.argv[5] if len(sys.argv) > 5 else None
    results = {}
    for mode, away in (("cold", other), ("same-rf", sibling)):
        if away is None:
            continue
        for rnd in range(rounds):
            for kind, query, codec in (("plain", "", "mpeg2"), ("heavy", "?transcode=heavy", "h264")):
                first_picture(host, port, f"/auto/v{away}", "mpeg2", limit=3)  # moves the tuner, then lets go of it
                time.sleep(1.0)
                r = first_picture(host, port, f"/auto/v{station}{query}", codec)
                results.setdefault((mode, kind), []).append(r)
                print(f"{mode} round {rnd + 1} {kind}: {r}", flush=True)
                time.sleep(1.5)
    print()
    for (mode, kind), rs in results.items():
        ok = [r for r in rs if r]
        print(f"{mode:8} {kind:5} first I byte: {stats([r[0] for r in ok])}; picture complete: {stats([r[1] for r in ok])}; failed {len(rs) - len(ok)}")
    for mode in ("cold", "same-rf"):
        a = [r[0] for r in results.get((mode, "plain"), []) if r]
        b = [r[0] for r in results.get((mode, "heavy"), []) if r]
        if a and b:
            print(f"{mode:8} heavy minus plain: median {statistics.median(b) - statistics.median(a):+.2f} s, worst {max(b) - max(a):+.2f} s")


if __name__ == "__main__":
    main()
