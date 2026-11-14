#!/usr/bin/env python3
"""LG's shared JavaScript service host on the set (/usr/bin/node, titled ss.gateway): when it starts, what starts it, how long it stays.

    watch.py sample --hours N --out FILE.jsonl [--window 60] [--lane DIR]
    watch.py summary FILE.jsonl

`sample` appends one JSON line per window (60 s) to FILE, each window one read-only SSH call through tvs.ro of the lane's btv.py
(no lease). Once a second the set checks the pids created since the second before (the last field of /proc/loadavg) for the host
and whether a host it knows has exited. At the end of the window it reads each host's start, CPU, VmRSS, Pss and the services it
holds (ls-monitor), the SSAP clients on ports 3000 and 3001, the nsl processes, and ls-hubd's launches into the host with their
requesters from LG's debug log (/var/log/dbg-log). A window the set did not answer is a line with "error". Nothing here calls the
host's services: a call would start it.

`summary` joins the windows into instances of the host (boot, pid, start): resident time, the guest whose launch started it and the
launch's requester, the guests seen, peak RSS and Pss, CPU; then starts per hour and the resident share of the time covered.
"""
import argparse
import collections
import datetime
import ipaddress
import json
import os
import re
import sys
import time

HZ = 100  # USER_HZ on the set: the unit of /proc/PID/stat's times
LANE = os.environ.get("BTV_DIR", "/data/home/tmp/lumatv-valueelide-20261112")
GAP = 90  # seconds between windows that count as a gap in the record
TRIGGER_SPAN = (-3.0, 2.0)  # a launch this close to an instance's start (s) is what started it

# One window on the set (busybox ash). Tags: B boot, R running at the start, S started, X exited, E end, P host, G guest,
# C SSAP client, N nsl pids, L ls-hubd launch into the host.
WINDOW = r'''W=__W__
PIDMAX=$(cat /proc/sys/kernel/pid_max)
read -r up _ < /proc/uptime
end=$(( ${up%.*} + W ))
echo "B $(cat /proc/sys/kernel/random/boot_id) $up $(date +%s)"
host() {
  c=""
  { read -r c < "/proc/$1/comm"; } 2>/dev/null
  case "$c" in
    ss.gateway|unified_servic*) return 0 ;;
    node) tr '\0' ' ' < "/proc/$1/cmdline" 2>/dev/null | grep -q unified_service_server ;;
    *) return 1 ;;
  esac
}
began() { cut -d' ' -f22 "/proc/$1/stat" 2>/dev/null; }
cur=""
for p in $(pidof ss.gateway unified_service_server node 2>/dev/null); do
  case " $cur " in *" $p "*) continue ;; esac
  host "$p" && { cur="$cur $p"; echo "R $p $up $(began "$p")"; }
done
read -r _ _ _ _ last < /proc/loadavg
while :; do
  sleep 1
  read -r up _ < /proc/uptime
  read -r _ _ _ _ now < /proc/loadavg
  p=$last
  while [ "$p" != "$now" ]; do
    p=$((p + 1)); [ "$p" -gt "$PIDMAX" ] && p=1
    [ -d "/proc/$p" ] || continue
    tg=""
    { while read -r k v; do [ "$k" = Tgid: ] && { tg=$v; break; }; done < "/proc/$p/status"; } 2>/dev/null
    [ "$tg" = "$p" ] && host "$p" && { cur="$cur $p"; echo "S $p $up $(began "$p")"; }
  done
  last=$now
  keep=""
  for p in $cur; do
    if [ -d "/proc/$p" ]; then keep="$keep $p"; else echo "X $p $up"; fi
  done
  cur=$keep
  [ "${up%.*}" -ge "$end" ] && break
done
echo "E $up"
for p in $cur; do
  read -r c < "/proc/$p/comm"
  echo "P $p|$c|$(tr '\0' ' ' < "/proc/$p/cmdline")|$(cut -d' ' -f14,15,22 "/proc/$p/stat")|$(sed -n 's/^VmRSS:[^0-9]*\([0-9]*\).*/\1/p' "/proc/$p/status")|$(sed -n 's/^Pss:[^0-9]*\([0-9]*\).*/\1/p' "/proc/$p/smaps_rollup" | head -n1)"
done 2>/dev/null
[ -n "$cur" ] && ls-monitor -l 2>/dev/null | awk -v s="$cur" 'BEGIN {n = split(s, a, " "); for (i = 1; i <= n; i++) h[a[i]] = 1} ($1 in h) {print "G " $1 " " $2}'
awk '$4 == "01" && ($2 ~ /:0BB8$/ || $2 ~ /:0BB9$/) {print "C " $2 " " $3}' /proc/net/tcp /proc/net/tcp6 2>/dev/null
echo "N $(pidof nsl)"
{ [ -n "$(find /var/log/dbg-log.0.gz -mmin -5 2>/dev/null)" ] && zcat /var/log/dbg-log.0.gz; cat /var/log/dbg-log; } 2>/dev/null \
  | grep LSHUB_SRV_LNCH | grep -e 'run-js-service -u' -e run-unified-service-server \
  | sed -n 's/^[^[]*\[\([0-9.]*\)\].*"SERVICE_NAME":"\([^"]*\)","EXE":"\([^"]*\)","REQUESTER":"\([^"]*\)".*/L \1|\2|\4|\3/p'
'''


def addr(s):
    """An address of /proc/net/tcp or tcp6 (hex, little-endian words) as ip:port."""
    h, _, port = s.partition(":")
    raw = b"".join(bytes.fromhex(h[i:i + 8])[::-1] for i in range(0, len(h), 8))
    ip = ipaddress.ip_address(raw)
    ip = getattr(ip, "ipv4_mapped", None) or ip
    return "%s:%d" % (ip, int(port, 16))


def parse(text):
    """One window's lines from the set as a JSON-ready dict; lines it does not know (a message of the set's shell) are skipped."""
    w = {"events": [], "hosts": [], "ssap": [], "nsl": [], "launches": []}
    guests = collections.defaultdict(list)
    for line in text.splitlines():
        tag, _, rest = line.partition(" ")
        f = rest.split()
        try:
            if tag == "B":
                w["boot"], w["up0"], w["epoch0"] = f[0], float(f[1]), int(f[2])
            elif tag in ("R", "S"):
                w["events"].append({"ev": "running" if tag == "R" else "start", "pid": int(f[0]), "up": float(f[1]),
                                    "startUp": int(f[2]) / HZ})
            elif tag == "X":
                w["events"].append({"ev": "exit", "pid": int(f[0]), "up": float(f[1])})
            elif tag == "E":
                w["up1"] = float(f[0])
            elif tag == "P":
                pid, comm, title, ticks, rss, pss = rest.split("|")
                ut, st, start = (int(x) for x in ticks.split())
                w["hosts"].append({"pid": int(pid), "comm": comm, "title": title.strip(), "startUp": start / HZ,
                                   "cpuS": (ut + st) / HZ, "rssKb": int(rss) if rss else None, "pssKb": int(pss) if pss else None})
            elif tag == "G":
                guests[int(f[0])].append(f[1])
            elif tag == "C":
                w["ssap"].append({"local": addr(f[0]), "remote": addr(f[1])})
            elif tag == "N":
                w["nsl"] = [int(x) for x in f]
            elif tag == "L":
                ts, service, requester, exe = rest.split("|", 3)
                w["launches"].append({"up": float(ts), "service": service, "requester": requester,
                                      "host": "run-unified-service-server" in exe})
        except (ValueError, IndexError):
            continue
    for h in w["hosts"]:
        h["guests"] = guests.get(h["pid"], [])
    return w


def sample(hours, out, window, lane):
    sys.path.insert(0, lane)
    from btv import tvs  # the lane's own paths and environment are set on import
    script = WINDOW.replace("__W__", str(int(window)))
    end = time.time() + hours * 3600
    while time.time() < end:
        t0 = time.time()
        try:
            text, rc = tvs.ro(script, window + 60)
            rec = parse(text) if rc == 0 else {"error": "exit %d" % rc, "tail": text[-300:]}
        except (OSError, EOFError, tvs.paramiko.SSHException) as e:  # off, rebooting or out of reach
            rec = {"error": "%s: %s" % (type(e).__name__, e)}
        rec["t"] = round(t0)
        with open(out, "a") as f:
            f.write(json.dumps(rec, separators=(",", ":")) + "\n")
        if "error" in rec:
            time.sleep(max(0.0, min(30.0, end - time.time())))


def requester_kind(name):
    """A requester without its pid suffix: com.webos.service.nsl-3813 and com.webos.lunasend-9313 name a program, not one run."""
    return re.sub(r"-\d+$", "", name)


def summarize(windows):
    """Instances of the host and the totals over the windows that answered."""
    ok = [w for w in windows if "error" not in w and "up1" in w]
    boots, launches = {}, {}
    for w in ok:
        b = boots.setdefault(w["boot"], {"first": w["up0"], "last": w["up1"], "windows": []})
        b["first"], b["last"] = min(b["first"], w["up0"]), max(b["last"], w["up1"])
        b["windows"].append(w)
        for la in w["launches"]:
            launches.setdefault(w["boot"], {})[(la["up"], la["service"])] = la
    order = list(boots)
    insts, live = {}, {}
    for w in ok:
        b = w["boot"]

        def inst(pid, start):
            key = (b, pid, round(start, 2))
            if key not in insts:
                insts[key] = {"boot": b, "pid": pid, "start": start, "epoch": w["epoch0"] - (w["up0"] - start), "end": None,
                              "lastSeen": start, "guests": [], "rssKb": None, "pssKb": None, "cpuS": None, "title": None}
            live[(b, pid)] = key
            return insts[key]
        for e in w["events"]:
            if e["ev"] == "exit":
                key = live.pop((b, e["pid"]), None)
                if key:
                    insts[key]["end"] = insts[key]["lastSeen"] = e["up"]
            else:
                i = inst(e["pid"], e["startUp"])
                i["lastSeen"] = max(i["lastSeen"], e["up"])
        for h in w["hosts"]:
            i = inst(h["pid"], h["startUp"])
            i["lastSeen"] = max(i["lastSeen"], w["up1"])
            i["title"] = h["title"] or h["comm"]
            for k in ("rssKb", "pssKb", "cpuS"):
                if h[k] is not None:
                    i[k] = max(i[k] or 0, h[k])
            i["guests"] += [g for g in h["guests"] if g not in i["guests"]]
    out = []
    for i in insts.values():
        b = boots[i["boot"]]
        i["counted"] = i["start"] >= b["first"] - 1
        i["censored"] = None if i["end"] is not None else ("end of the record" if i["boot"] == order[-1] else "reboot")
        i["residentS"] = max(0.0, min(i["end"] if i["end"] is not None else i["lastSeen"], b["last"]) - max(i["start"], b["first"]))
        near = sorted((la for la in launches.get(i["boot"], {}).values()
                       if TRIGGER_SPAN[0] <= la["up"] - i["start"] <= TRIGGER_SPAN[1]), key=lambda la: la["up"])
        first = next((la for la in near if not la["host"]), None) or next(iter(near), None)
        i["firstGuest"] = first["service"] if first and not first["host"] else None
        i["trigger"] = requester_kind(first["requester"]) if first else None
        out.append(i)
    out.sort(key=lambda i: (order.index(i["boot"]), i["start"]))
    span = sum(b["last"] - b["first"] for b in boots.values())
    gaps = sum(1 for b in boots.values() for p, n in zip(b["windows"], b["windows"][1:]) if n["up0"] - p["up1"] > GAP)
    starts = [i for i in out if i["counted"]]
    return {"windows": len(windows), "unanswered": len(windows) - len(ok), "boots": len(boots), "spanS": span, "gaps": gaps,
            "starts": len(starts), "startsPerHour": len(starts) / (span / 3600) if span else None,
            "residentS": sum(i["residentS"] for i in out), "instances": out,
            "triggers": collections.Counter(i["trigger"] or "not in the log" for i in starts),
            "firstGuests": collections.Counter(i["firstGuest"] or "unknown" for i in starts),
            "ssapClients": sorted({c["remote"].rsplit(":", 1)[0] for w in ok for c in w["ssap"]})}


def report(s):
    mb = lambda kb: "%.1f MB" % (kb / 1024) if kb else "?"  # noqa: E731
    lines = ["%d windows, %d unanswered; %.2f h covered over %d boot(s), %d gap(s) over %d s" % (
        s["windows"], s["unanswered"], s["spanS"] / 3600, s["boots"], s["gaps"], GAP)]
    if s["spanS"]:
        lines.append("starts %d (%.2f per hour); resident %.2f h, %.0f%% of the time covered" % (
            s["starts"], s["startsPerHour"], s["residentS"] / 3600, 100 * s["residentS"] / s["spanS"]))
    for i in s["instances"]:
        when = datetime.datetime.fromtimestamp(i["epoch"]).strftime("%m-%d %H:%M:%S")
        end = "exited after" if i["end"] is not None else "still there (%s) after" % i["censored"]
        lines.append("pid %d %s (uptime %.0f s)%s: %s %.0f s; first guest %s, started by %s; guests %s; peak RSS %s, Pss %s; "
                     "CPU %s s" % (i["pid"], when, i["start"], "" if i["counted"] else ", already running", end, i["residentS"],
                                   i["firstGuest"] or "unknown", i["trigger"] or "not in the log",
                                   ", ".join(i["guests"]) or "none seen", mb(i["rssKb"]), mb(i["pssKb"]),
                                   "?" if i["cpuS"] is None else "%.1f" % i["cpuS"]))
    for name in ("triggers", "firstGuests"):
        if s[name]:
            lines.append("%s: %s" % (name, ", ".join("%s %d" % kv for kv in s[name].most_common())))
    lines.append("SSAP clients seen: %s" % (", ".join(s["ssapClients"]) or "none"))
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    sub = ap.add_subparsers(dest="cmd", required=True)
    sp = sub.add_parser("sample")
    sp.add_argument("--hours", type=float, required=True)
    sp.add_argument("--out", required=True)
    sp.add_argument("--window", type=int, default=60)
    sp.add_argument("--lane", default=LANE)
    sm = sub.add_parser("summary")
    sm.add_argument("file")
    a = ap.parse_args(argv)
    if a.cmd == "sample":
        sample(a.hours, a.out, a.window, a.lane)
    else:
        with open(a.file) as f:
            print(report(summarize([json.loads(line) for line in f if line.strip()])))


if __name__ == "__main__":
    main()
