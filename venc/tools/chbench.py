#!/usr/bin/env python3
"""Channel change as a player sees it: from sending the request to each step of the stream, raw and converted."""
import os, socket, statistics, sys, time

AUDIO = {0x03, 0x04, 0x0F, 0x11, 0x81, 0x87, 0x06}
VIDEO = {0x02: "mpeg2", 0x1B: "h264", 0x24: "hevc"}

class Probe:
    """Feeds on a transport stream as it arrives and notes when the PAT, the PMT, the first picture and the first sound were seen."""
    def __init__(self):
        self.buf = b""
        self.at = {}
        self.pmt_pid = self.vpid = self.apid = self.vtype = None
        self.key_open = False
        self.head = None
        self.head_t = 0.0

    def mark(self, name, t):
        self.at.setdefault(name, t)

    def feed(self, data, t):
        self.buf += data
        if not self.buf.startswith(b"\x47"):
            i = self.buf.find(b"\x47")
            self.buf = self.buf[i:] if i >= 0 else b""
        n = len(self.buf) // 188 * 188
        for o in range(0, n, 188):
            self.packet(self.buf[o:o + 188], t)
        self.buf = self.buf[n:]

    def packet(self, p, t):
        pid, pusi = ((p[1] & 0x1F) << 8) | p[2], bool(p[1] & 0x40)
        afc = (p[3] >> 4) & 3
        body = p[(5 + p[4]) if afc & 2 else 4:] if afc & 1 else b""
        if pid == 0 and pusi and len(body) > 12:
            self.mark("pat", t)
            s = body[1 + body[0]:]
            for i in range(8, 3 + ((s[1] & 15) << 8 | s[2]) - 4, 4):
                prog, pp = (s[i] << 8) | s[i + 1], ((s[i + 2] & 0x1F) << 8) | s[i + 3]
                if prog:
                    self.pmt_pid = pp
                    break
        elif pid == self.pmt_pid and pusi and self.vpid is None:
            s = body[1 + body[0]:]
            end = 3 + ((s[1] & 15) << 8 | s[2]) - 4
            i = 12 + (((s[10] & 15) << 8) | s[11])
            while i + 5 <= end:
                typ, ep, info = s[i], ((s[i + 1] & 0x1F) << 8) | s[i + 2], ((s[i + 3] & 15) << 8) | s[i + 4]
                if typ in VIDEO and self.vpid is None:
                    self.vpid, self.vtype = ep, typ
                elif typ in AUDIO and self.apid is None:
                    self.apid = ep
                i += 5 + info
            if self.vpid is not None:
                self.mark("pmt", t)
        elif pid == self.apid and pusi:
            self.mark("sound", t)
        elif pid == self.vpid:
            if pusi:
                if self.key_open:
                    self.mark("key_whole", t)
                    self.key_open = False
                self.mark("pic_start", t)
                self.head = bytearray(body[9 + body[8]:]) if "key_start" not in self.at else None
                self.head_t = t
            elif self.head is not None:
                self.head += body
            if self.head is not None:
                if self.keyframe(bytes(self.head)):
                    self.mark("key_start", self.head_t)
                    self.key_open, self.head = True, None
                elif len(self.head) > 2048:
                    self.head = None

    def keyframe(self, es):
        if self.vtype == 0x02:
            i = es.find(b"\x00\x00\x01\x00")
            return i >= 0 and i + 6 <= len(es) and ((es[i + 5] >> 3) & 7) == 1
        if self.vtype == 0x1B:
            return any((es[i + 3] & 31) == 5 for i in range(len(es) - 3) if es[i:i + 3] == b"\x00\x00\x01")
        return False

    @property
    def done(self):
        return "key_whole" in self.at

def trial(host, port, channel, profile, limit=9.0):
    path = f"/auto/v{channel}" + (f"?transcode={profile}" if profile else "")
    probe = Probe()
    t0 = time.monotonic()
    s = socket.create_connection((host, port), timeout=limit)
    probe.at["connect"] = time.monotonic() - t0
    s.sendall(f"GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n".encode())
    head = b""
    status = None
    try:
        while time.monotonic() - t0 < limit and not probe.done:
            data = s.recv(65536)
            t = time.monotonic() - t0
            if not data:
                break
            if status is None:
                head += data
                if b"\r\n\r\n" not in head:
                    continue
                head, data = head.split(b"\r\n\r\n", 1)
                status = head.split(b" ", 2)[1].decode()
                probe.at["headers"] = t
                if status != "200":
                    break
            if data:
                probe.mark("first_byte", t)
                probe.feed(data, t)
    except OSError:
        pass
    finally:
        s.close()
    return status, probe

def main():
    host = os.environ.get("TV", "192.168.1.242")
    if sys.argv[1:2] == ["--file"]:
        p = Probe()
        d = open(sys.argv[2], "rb").read()
        for i in range(0, len(d), 188 * 64):
            p.feed(d[i:i + 188 * 64], i / 1e6)
        print({k: round(v, 3) for k, v in p.at.items()}, p.vtype)
        return
    channels, rounds = sys.argv[1].split(","), int(sys.argv[2])
    profile = sys.argv[3] if len(sys.argv) > 3 else "mobile"
    gap = float(os.environ.get("GAP", "1.2"))
    rows = []
    for r in range(rounds):
        for i, ch in enumerate(channels):
            mode = None if (i + r) % 2 == 0 else profile
            status, p = trial(host, 5004, ch, mode)
            row = {"channel": ch, "mode": mode or "raw", "status": status, **{k: round(v, 3) for k, v in p.at.items()}, "vtype": p.vtype}
            if "key_start" in row and "first_byte" in row:
                row["lag"] = round(row["key_start"] - row["first_byte"], 3)
            rows.append(row)
            print(row, flush=True)
            time.sleep(gap)
    for mode in ("raw", profile):
        mine = [r for r in rows if r["mode"] == mode and "key_whole" in r]
        if not mine:
            continue
        print(f"{mode}: {len(mine)} changes")
        for k in ("headers", "first_byte", "pat", "pmt", "pic_start", "key_start", "lag", "key_whole", "sound"):
            v = [r[k] for r in mine if k in r]
            if v:
                print(f"  {k:10} median {statistics.median(v):.2f}  min {min(v):.2f}  max {max(v):.2f}  ({len(v)})")

main()
