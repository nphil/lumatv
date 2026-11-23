#!/usr/bin/env python3
"""Soak test of the on-TV transcoder: a player that keeps asking the Live TV Tuner for H.264 while the set is made to take the hardware away.

    python3 venc/tools/soak.py --out DIR [--minutes 30] [--channels 5.1,11.11,2.1,30.1] [--profile mobile]
        [--switch-every 180] [--kill-helper-at 8] [--app-at 16 --app com.webos.app.settings --app-hold 20]

The player is a loop that opens `http://<tv>:5004/auto/v<channel>?transcode=<profile>`, reads the transport stream as it arrives and, when the
stream ends, asks again a second later. It reads the stream itself (PAT, PMT, the picture's PES times, continuity counters) and, for the first
connection after every change, keeps 15 s for ffmpeg to decode. The set is sampled every 15 s over ssh (load, memory, the helper's and the child's
CPU and memory). On a timeline the script switches channel, kills the helper's service process (its supervisor starts it again) and opens an app
over Home, then opens Home again; each is marked in `events.jsonl` with the time it was done.

Nothing is left on the set. Needs `ssh` to the set (`--ssh`, default Marquee's `tv-ssh`; the address comes from $TV) and ffmpeg and ffprobe here.
Hold the TV lock (`tools/tv-lock.sh with --why ... --`) around a run, and read the app step's notes before using `--app-at`: it covers the screen.

Writes to DIR: events.jsonl, connections.jsonl, samples.jsonl, decode.jsonl and summary.json.
"""

import argparse
import http.client
import json
import os
import shlex
import socket
import statistics
import subprocess
import sys
import threading
import time

TS = 188
CLK_TCK = 100
VIDEO_TYPES = {0x02: "mpeg2", 0x1B: "h264", 0x24: "hevc"}
AUDIO_TYPES = {0x81: "ac3", 0x06: "ac3", 0x03: "mp2", 0x04: "mp2", 0x0F: "aac"}


class Log:
    """JSON lines, one file each, written as they happen."""

    def __init__(self, out):
        self.out, self.lock, self.t0 = out, threading.Lock(), time.monotonic()

    def at(self):
        return round(time.monotonic() - self.t0, 3)

    def put(self, name, **fields):
        row = {"t": self.at(), **fields}
        with self.lock, open(os.path.join(self.out, name), "a") as f:
            f.write(json.dumps(row) + "\n")
        return row


class Mux:
    """Reads a transport stream as it comes: the program's PIDs and the PTS and arrival time of every picture."""

    def __init__(self):
        self.rest = b""
        self.pmt_pid = None
        self.video = self.audio = None
        self.codec = None
        self.last_cc = {}
        self.cc_errors = {"video": 0, "audio": 0}
        self.pictures = []
        self.audio_packets = 0
        self.packets = 0
        self.lost_sync = 0

    def feed(self, data, now):
        data = self.rest + data
        n = len(data) // TS * TS
        for i in range(0, n, TS):
            self.packet(data[i : i + TS], now)
        self.rest = data[n:]

    def packet(self, p, now):
        if p[0] != 0x47:
            self.lost_sync += 1
            return
        self.packets += 1
        pid = ((p[1] & 0x1F) << 8) | p[2]
        pusi = bool(p[1] & 0x40)
        afc = (p[3] >> 4) & 3
        payload = p[4:] if afc == 1 else p[5 + p[4] :] if afc == 3 and p[4] < 183 else b""
        if pid == 0 and pusi and payload:
            self.pat(payload)
        elif pid == self.pmt_pid and pusi and payload:
            self.pmt(payload)
        elif pid == self.video or pid == self.audio:
            kind = "video" if pid == self.video else "audio"
            if afc & 1:
                cc, last = p[3] & 15, self.last_cc.get(pid)
                disc = afc == 3 and p[4] > 0 and bool(p[5] & 0x80)
                if last is not None and cc != (last + 1) & 15 and not disc:
                    self.cc_errors[kind] += 1
                self.last_cc[pid] = cc
            if kind == "audio" and pusi:
                self.audio_packets += 1
            if kind == "video" and pusi and payload[:3] == b"\x00\x00\x01" and len(payload) >= 14 and payload[7] & 0x80:
                b = payload
                pts = ((b[9] >> 1) & 7) << 30 | b[10] << 22 | (b[11] >> 1) << 15 | b[12] << 7 | b[13] >> 1
                self.pictures.append((now, pts))

    def pat(self, payload):
        s = payload[1 + payload[0] :]
        if len(s) < 12 or s[0] != 0:
            return
        end = 3 + ((s[1] & 0x0F) << 8 | s[2]) - 4
        for i in range(8, min(end, len(s) - 3), 4):
            number, pid = s[i] << 8 | s[i + 1], (s[i + 2] & 0x1F) << 8 | s[i + 3]
            if number != 0:
                self.pmt_pid = pid
                return

    def pmt(self, payload):
        s = payload[1 + payload[0] :]
        if len(s) < 16 or s[0] != 2:
            return
        end = 3 + ((s[1] & 0x0F) << 8 | s[2]) - 4
        i = 12 + ((s[10] & 0x0F) << 8 | s[11])
        while i + 5 <= min(end, len(s)):
            kind, pid, info = s[i], (s[i + 1] & 0x1F) << 8 | s[i + 2], (s[i + 3] & 0x0F) << 8 | s[i + 4]
            if kind in VIDEO_TYPES and self.video is None:
                self.video, self.codec = pid, VIDEO_TYPES[kind]
            elif kind in AUDIO_TYPES and self.audio is None:
                self.audio = pid
            i += 5 + info


def clock_span(pts):
    """The seconds the picture times advance, over the 33-bit clock, and the largest step back."""
    steps_back, total = 0, 0
    for a, b in zip(pts, pts[1:]):
        d = (b - a + (1 << 32)) % (1 << 33) - (1 << 32)
        total += d
        steps_back += d < 0
    return total / 90000, steps_back


class Player(threading.Thread):
    """Asks for a channel as a phone would and asks again when the stream ends."""

    def __init__(self, args, log, decode):
        super().__init__(daemon=True)
        self.args, self.log, self.decode = args, log, decode
        self.channel = args.channels[0]
        self.stop, self.change = threading.Event(), threading.Event()
        self.keep_next = True
        self.connections = []

    def run(self):
        while not self.stop.is_set():
            self.change.clear()
            self.watch(self.channel)
            if not self.stop.is_set() and not self.change.is_set():
                time.sleep(1.0)

    def watch(self, channel):
        a = self.args
        started, mux, bytes_in, ended, status = time.monotonic(), Mux(), 0, "stopped", None
        sample = open(os.path.join(a.out, f"cap-{self.log.at():.0f}.ts"), "wb") if self.keep_next else None
        self.keep_next = False
        conn = http.client.HTTPConnection(a.tv, 5004, timeout=10)
        try:
            conn.request("GET", f"/auto/v{channel}?transcode={a.profile}", headers={"User-Agent": "xcode-soak"})
            resp = conn.getresponse()
            status = resp.status
            while True:
                if self.stop.is_set() or self.change.is_set():
                    ended = "switch" if self.change.is_set() else "stopped"
                    break
                data = resp.read(65536)
                now = time.monotonic()
                if not data:
                    ended = "eof"
                    break
                bytes_in += len(data)
                mux.feed(data, now)
                if sample and now - started < 15:
                    sample.write(data)
                elif sample:
                    sample.close()
                    self.decode.submit(sample.name, channel)
                    sample = None
        except (OSError, http.client.HTTPException) as e:
            ended = "error: " + type(e).__name__ + (": timed out" if isinstance(e, socket.timeout) else "")
        finally:
            conn.close()
            if sample:
                # the stream ended inside the 15 s: what there is of it is worth decoding only when it is most of them
                sample.close()
                if mux.pictures and time.monotonic() - started > 10:
                    self.decode.submit(sample.name, channel)
                else:
                    os.unlink(sample.name)
        self.record(channel, started, bytes_in, mux, ended, status)

    def record(self, channel, started, bytes_in, mux, ended, status):
        seconds = time.monotonic() - started
        times = [t for t, _ in mux.pictures]
        span, back = clock_span([p for _, p in mux.pictures])
        gaps = [b - a for a, b in zip(times, times[1:])]
        row = {
            "channel": channel,
            "at": round(started - self.log.t0, 3),
            "status": status,
            "ended": ended,
            "seconds": round(seconds, 2),
            "bytes": bytes_in,
            "kbps": round(bytes_in * 8 / 1000 / seconds) if seconds else 0,
            "codec": mux.codec,
            "pictures": len(mux.pictures),
            "first_picture_s": round(times[0] - started, 2) if times else None,
            "fps": round((len(times) - 1) / (times[-1] - times[0]), 2) if len(times) > 1 and times[-1] > times[0] else None,
            "pts_span_s": round(span, 2),
            "pts_back_steps": back,
            "max_gap_s": round(max(gaps), 3) if gaps else None,
            "cc_errors": mux.cc_errors,
            "lost_sync": mux.lost_sync,
            "audio": mux.audio is not None,
        }
        self.log.put("connections.jsonl", **row)
        self.connections.append(row)


class Decode:
    """ffmpeg decodes what the player kept and says how many pictures it got and what it complained of."""

    def __init__(self, log):
        self.log, self.threads = log, []

    def submit(self, path, channel):
        t = threading.Thread(target=self.run, args=(path, channel), daemon=True)
        t.start()
        self.threads.append(t)

    def run(self, path, channel):
        try:
            probe = subprocess.run(
                ["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                 "stream=codec_name,profile,level,width,height,avg_frame_rate,pix_fmt", "-of", "json", path],
                capture_output=True, text=True, timeout=60)
            stream = (json.loads(probe.stdout or "{}").get("streams") or [{}])[0]
            count = subprocess.run(
                ["ffprobe", "-v", "error", "-count_frames", "-select_streams", "v:0", "-show_entries", "stream=nb_read_frames",
                 "-of", "csv=p=0", path], capture_output=True, text=True, timeout=120).stdout.split()
            # a stream that is in a program is listed twice: the first line is the count
            frames = int(count[0]) if count and count[0].isdigit() else None
            # the player cut the capture at 15 s wherever the bytes fell, so its last picture is partial: the last two are not judged
            upto = ["-frames:v", str(frames - 2)] if frames and frames > 2 else []
            run = subprocess.run(["ffmpeg", "-v", "error", "-nostdin", "-i", path, "-map", "0:v:0", *upto, "-f", "null", "-"],
                                 capture_output=True, text=True, timeout=120)
            errors = [l for l in run.stderr.splitlines() if l.strip()]
            self.log.put("decode.jsonl", channel=channel, stream=stream, frames=frames, errors=len(errors), first_errors=errors[:5])
        finally:
            os.unlink(path)


class Set:
    """The TV, over ssh."""

    def __init__(self, ssh):
        self.ssh = shlex.split(ssh)

    def run(self, script, timeout=30):
        p = subprocess.run(self.ssh + ["sh -s"], input=script, capture_output=True, text=True, timeout=timeout)
        return p.returncode, p.stdout


SAMPLE = r"""
cat /proc/loadavg
grep -E '^(MemAvailable|CmaFree):' /proc/meminfo
for p in $(ps | awk '/lumatv-venc|lumatv-helper/ && !/awk/ {print $1}'); do
  [ -r /proc/$p/stat ] && echo "P $p $(cut -d' ' -f2,4,14,15 /proc/$p/stat) $(awk '/VmRSS/{print $2}' /proc/$p/status)"
done
"""

SERVICE_PID = r"""
for p in $(ps | awk '/lumatv-helper/ && !/awk/ {print $1}'); do
  pp=$(cut -d' ' -f4 /proc/$p/stat); grep -q lumatv-helper /proc/$pp/comm 2>/dev/null && echo $p
done
"""


class Sampler(threading.Thread):
    def __init__(self, tv, log, every):
        super().__init__(daemon=True)
        self.tv, self.log, self.every, self.stop, self.last = tv, log, every, threading.Event(), {}

    def run(self):
        while not self.stop.wait(self.every):
            try:
                rc, out = self.tv.run(SAMPLE, timeout=20)
            except subprocess.TimeoutExpired:
                self.log.put("samples.jsonl", error="timeout")
                continue
            now, row, procs = time.monotonic(), {}, {}
            for line in out.splitlines():
                f = line.split()
                if line.startswith(("MemAvailable", "CmaFree")):
                    row[f[0].rstrip(":")] = int(f[1])
                elif line.startswith("P ") and len(f) >= 7:
                    pid, comm, ppid, ut, st, rss = f[1], f[2].strip("()"), f[3], int(f[4]), int(f[5]), int(f[6])
                    prev = self.last.get(pid)
                    cpu = round((ut + st - prev[0]) / CLK_TCK / (now - prev[1]) * 100, 1) if prev else None
                    self.last[pid] = (ut + st, now)
                    procs[f"{comm}:{pid}"] = {"cpu": cpu, "rss_kb": rss}
                elif f and f[0].replace(".", "").isdigit():
                    row["load"] = float(f[0])
            self.log.put("samples.jsonl", **row, procs=procs)


def timeline(args, log, tv, player):
    """What is done to the set, and when: the events are in the receipt with the time each one was finished."""
    due = []
    for k in range(1, int(args.minutes * 60 // args.switch_every) + 1):
        due.append((k * args.switch_every, "switch", None))
    if args.kill_helper_at >= 0:
        due.append((args.kill_helper_at * 60, "kill-helper", None))
    if args.app_at >= 0:
        due.append((args.app_at * 60, "app", args.app))
    switches = 0
    for at, what, arg in sorted(due):
        while time.monotonic() - log.t0 < at:
            if player.stop.wait(0.5):
                return
        if what == "switch":
            switches += 1
            player.channel = args.channels[switches % len(args.channels)]
            player.keep_next = True
            player.change.set()
            log.put("events.jsonl", event="switch", channel=player.channel)
        elif what == "kill-helper":
            rc, out = tv.run(SERVICE_PID)
            pids = out.split()
            if len(pids) == 1:
                rc, _ = tv.run(f"kill -9 {pids[0]}")
                player.keep_next = True
                log.put("events.jsonl", event="kill-helper", pid=pids[0], rc=rc)
            else:
                log.put("events.jsonl", event="kill-helper", skipped=f"expected one service process, found {pids}")
        elif what == "app":
            rc, _ = tv.run(f"luna-send -n 1 luna://com.webos.applicationManager/launch '{{\"id\":\"{arg}\"}}' </dev/null")
            log.put("events.jsonl", event="app-open", app=arg, rc=rc)
            player.stop.wait(args.app_hold)
            rc, _ = tv.run("luna-send -n 1 luna://com.webos.applicationManager/launch '{\"id\":\"com.webos.app.home\"}' </dev/null")
            player.keep_next = True
            log.put("events.jsonl", event="home", rc=rc)


def summarize(args, log, player, samples_path):
    rows = player.connections
    total = max(log.at(), 1)
    h264 = [r for r in rows if r["codec"] == "h264"]
    summary = {
        "minutes": round(total / 60, 1),
        "connections": len(rows),
        "h264_connections": len(h264),
        "mpeg2_connections": len([r for r in rows if r["codec"] == "mpeg2"]),
        "no_picture_connections": len([r for r in rows if not r["pictures"]]),
        "h264_seconds": round(sum(r["seconds"] for r in h264), 1),
        "ended": {k: len([r for r in rows if r["ended"] == k]) for k in sorted({r["ended"] for r in rows})},
        "fps_h264": sorted({r["fps"] for r in h264 if r["fps"]})[:1] + sorted({r["fps"] for r in h264 if r["fps"]})[-1:],
        "kbps_h264_mean": round(statistics.mean([r["kbps"] for r in h264])) if h264 else None,
        "first_picture_s": {
            "h264_max": max([r["first_picture_s"] for r in h264 if r["first_picture_s"] is not None] or [None]),
            "h264_median": statistics.median([r["first_picture_s"] for r in h264 if r["first_picture_s"] is not None] or [0]),
        },
        "max_gap_s_h264": max([r["max_gap_s"] for r in h264 if r["max_gap_s"] is not None] or [None]),
        "cc_errors_h264": sum(sum(r["cc_errors"].values()) for r in h264),
        "pts_back_steps_h264": sum(r["pts_back_steps"] for r in h264),
        "lost_sync": sum(r["lost_sync"] for r in rows),
    }
    try:
        samples = [json.loads(l) for l in open(samples_path)]
    except OSError:
        samples = []
    venc = [p["cpu"] for s in samples for k, p in s.get("procs", {}).items() if k.startswith("lumatv-venc") and p["cpu"] is not None]

    def service(s):
        """The helper is two processes, a supervisor of 0.2 MB and the service; the service is the bigger one."""
        return max((p["rss_kb"] for k, p in s.get("procs", {}).items() if k.startswith("lumatv-helper") and p["rss_kb"]), default=None)

    rss = [r for r in map(service, samples) if r is not None]
    summary["child_cpu_pct"] = {"mean": round(statistics.mean(venc), 1), "max": max(venc)} if venc else None
    summary["helper_rss_kb"] = {"first": rss[0], "last": rss[-1], "max": max(rss)} if rss else None
    summary["load_max"] = max([s["load"] for s in samples if "load" in s] or [None])
    summary["cma_free_kb_min"] = min([s["CmaFree"] for s in samples if "CmaFree" in s] or [None])
    with open(os.path.join(args.out, "summary.json"), "w") as f:
        json.dump(summary, f, indent=1)
    return summary


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--out", required=True)
    ap.add_argument("--tv", default=os.environ.get("TV", ""), help="the set's address (default $TV)")
    ap.add_argument("--ssh", default="/data/home/Marquee/tools/tv-ssh ssh tv", help="how to run a command on the set")
    ap.add_argument("--minutes", type=float, default=30)
    ap.add_argument("--channels", default="5.1,11.11,2.1,30.1")
    ap.add_argument("--profile", default="mobile")
    ap.add_argument("--switch-every", type=int, default=180, help="seconds between channel switches")
    ap.add_argument("--kill-helper-at", type=float, default=-1, help="minute; -1 none")
    ap.add_argument("--app-at", type=float, default=-1, help="minute; -1 none")
    ap.add_argument("--app", default="com.webos.app.settings", help="the app opened over Home")
    ap.add_argument("--app-hold", type=int, default=20, help="seconds the app stays in front")
    ap.add_argument("--sample-every", type=int, default=15)
    args = ap.parse_args()
    args.channels = args.channels.split(",")
    if not args.tv:
        sys.exit("soak.py: give the set's address with --tv or $TV")
    os.makedirs(args.out, exist_ok=True)
    log, tv = Log(args.out), Set(args.ssh)
    decode = Decode(log)
    player = Player(args, log, decode)
    sampler = Sampler(tv, log, args.sample_every)
    log.put("events.jsonl", event="start", args={k: v for k, v in vars(args).items() if k != "ssh"})
    sampler.start()
    player.start()
    threading.Thread(target=timeline, args=(args, log, tv, player), daemon=True).start()
    try:
        while log.at() < args.minutes * 60:
            time.sleep(1)
    except KeyboardInterrupt:
        log.put("events.jsonl", event="interrupted")
    player.stop.set()
    sampler.stop.set()
    player.join(timeout=15)
    for t in decode.threads:
        t.join(timeout=120)
    log.put("events.jsonl", event="end")
    print(json.dumps(summarize(args, log, player, os.path.join(args.out, "samples.jsonl")), indent=1))


if __name__ == "__main__":
    main()
