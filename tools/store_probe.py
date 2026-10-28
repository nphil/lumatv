#!/usr/bin/env python3
"""The Content Store theme's probe: the receiver it posts to, and the report that reads what it wrote. Python 3 standard library only
(sockets, json): runs on the set (python 3.10) and on the workstation.

usage: store_probe.py serve --dir DIR [--port N] [--seconds S] [--pidfile F] [--portfile F]
       store_probe.py report DIR [--json]

WAM's inspector does not listen on this firmware, so the store's markup cannot be read from the set (docs/research/store-result.md, TV round
of 2026-10-28). `lumatv store probe` mounts the theme's script with its probe part (the page then posts what it is made of: class names,
structure, computed styles of what matters, none of its words, text becomes its length) and starts `serve` on a loopback port of the set.
`serve` writes each post to DIR as <n>-<kind>.json, answers the page's cross-origin and private-network preflights, accepts at most
MAX_FILES posts and MAX_BYTES bytes, listens on 127.0.0.1 only and ends by itself after --seconds (default 1800). `report` prints what a
person needs from those files to fix the theme's selectors: the families and variables the page uses, where its focus plates are, its
black bands, its stylesheets.
"""
import json
import os
import re
import select
import signal
import socket
import sys
import time
import urllib.parse

MAX_FILES = 400         # the page stops at 150 posts per document, and the loader and the cloud page are two
MAX_BODY = 4 * 1024 * 1024
MAX_BYTES = 24 * 1024 * 1024
CORS = ("Access-Control-Allow-Origin: *\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Allow-Methods: POST, GET, OPTIONS\r\n"
        "Access-Control-Allow-Headers: *\r\nAccess-Control-Max-Age: 600\r\n")


def response(status, body=b"", ctype="text/plain"):
    reasons = {200: "OK", 204: "No Content", 400: "Bad Request", 404: "Not Found", 413: "Payload Too Large", 429: "Too Many Requests"}
    head = "HTTP/1.1 %d %s\r\n%sContent-Type: %s\r\nContent-Length: %d\r\nConnection: close\r\n\r\n" % (status, reasons.get(status, "OK"), CORS, ctype, len(body))
    return head.encode() + body


def read_request(conn):
    """(method, path, headers, body) or None. Reads at most 16 KB of headers and MAX_BODY of body."""
    data = b""
    while b"\r\n\r\n" not in data:
        chunk = conn.recv(8192)
        if not chunk:
            return None
        data += chunk
        if len(data) > 16384 and b"\r\n\r\n" not in data:
            return None
    head, _, body = data.partition(b"\r\n\r\n")
    lines = head.decode("latin1").split("\r\n")
    parts = lines[0].split(" ")
    if len(parts) < 2:
        return None
    headers = {}
    for line in lines[1:]:
        k, _, v = line.partition(":")
        headers[k.strip().lower()] = v.strip()
    want = int(headers.get("content-length", "0") or 0)
    if want > MAX_BODY:
        return parts[0], parts[1], headers, None
    while len(body) < want:
        chunk = conn.recv(65536)
        if not chunk:
            break
        body += chunk
    return parts[0], parts[1], headers, body[:want]


class Receiver:
    def __init__(self, out_dir):
        self.dir = out_dir
        self.files = len([n for n in os.listdir(out_dir) if n.endswith(".json") or n.endswith(".txt")])
        self.bytes = 0
        os.makedirs(out_dir, exist_ok=True)
        self.log_path = os.path.join(out_dir, "receiver.log")

    def log(self, text):
        with open(self.log_path, "a") as f:
            f.write("%s %s\n" % (time.strftime("%H:%M:%S"), text))

    def full(self):
        return self.files >= MAX_FILES or self.bytes >= MAX_BYTES

    def handle(self, conn):
        try:
            conn.settimeout(5)
            req = read_request(conn)
            if req is None:
                return
            method, path, headers, body = req
            origin = headers.get("origin", "")
            if method == "OPTIONS":
                conn.sendall(response(204))
                self.log("OPTIONS %s origin=%s" % (path, origin))
            elif method == "POST" and path == "/p":
                if body is None:
                    conn.sendall(response(413))
                elif self.full():
                    conn.sendall(response(429, b'{"ok":false,"full":true}', "application/json"))
                else:
                    self.files += 1
                    self.bytes += len(body)
                    kind = "raw"
                    ext = "txt"
                    try:
                        kind = re.sub(r"[^a-z0-9-]", "", str(json.loads(body.decode("utf-8", "replace")).get("kind", "raw")).lower())[:24] or "raw"
                        ext = "json"
                    except ValueError:
                        pass
                    name = "%03d-%s.%s" % (self.files, kind, ext)
                    with open(os.path.join(self.dir, name), "wb") as f:
                        f.write(body)
                    conn.sendall(response(200, json.dumps({"ok": True, "n": self.files, "file": name}).encode(), "application/json"))
                    self.log("POST %s %d bytes origin=%s -> %s" % (path, len(body), origin, name))
            elif method == "GET" and path.startswith("/b"):
                q = urllib.parse.parse_qs(urllib.parse.urlparse(path).query)
                with open(os.path.join(self.dir, "beacons.log"), "a") as f:
                    f.write("%s %s %s\n" % (time.strftime("%H:%M:%S"), (q.get("k") or [""])[0], (q.get("d") or [""])[0][:2000]))
                conn.sendall(response(204))
                self.log("BEACON %s origin=%s" % ((q.get("k") or [""])[0], origin))
            elif method == "GET" and path == "/ping":
                conn.sendall(response(200, b"ok"))
            else:
                conn.sendall(response(404))
        except (OSError, ValueError):
            pass
        finally:
            try:
                conn.close()
            except OSError:
                pass


def serve(argv):
    opts = {"--port": "0", "--seconds": "1800", "--dir": None, "--pidfile": None, "--portfile": None}
    rest = argv[:]
    while rest:
        k = rest.pop(0)
        if k not in opts or not rest:
            print(__doc__.split("\n\n")[0], file=sys.stderr)
            return 2
        opts[k] = rest.pop(0)
    if not opts["--dir"]:
        print("serve needs --dir", file=sys.stderr)
        return 2
    os.makedirs(opts["--dir"], exist_ok=True)
    srv = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    try:
        srv.bind(("127.0.0.1", int(opts["--port"])))
    except OSError as e:
        print("cannot listen: %s" % e, file=sys.stderr)
        return 1
    srv.listen(8)
    port = srv.getsockname()[1]
    rx = Receiver(opts["--dir"])
    stop = []
    signal.signal(signal.SIGTERM, lambda *a: stop.append(1))
    signal.signal(signal.SIGINT, lambda *a: stop.append(1))
    if opts["--pidfile"]:
        with open(opts["--pidfile"], "w") as f:
            f.write("%d\n" % os.getpid())
    if opts["--portfile"]:
        tmp = opts["--portfile"] + ".tmp"
        with open(tmp, "w") as f:
            f.write("%d\n" % port)
        os.replace(tmp, opts["--portfile"])           # written once it is listening: a reader that sees it can connect
    rx.log("listening on 127.0.0.1:%d for %s s" % (port, opts["--seconds"]))
    deadline = time.time() + float(opts["--seconds"])
    try:
        while not stop and time.time() < deadline:
            ready, _, _ = select.select([srv], [], [], 0.25)
            if ready:
                conn, _ = srv.accept()
                rx.handle(conn)
    finally:
        rx.log("ended: %d posts, %d bytes" % (rx.files, rx.bytes))
        srv.close()
        for p in (opts["--pidfile"], opts["--portfile"]):
            if p:
                try:
                    os.remove(p)
                except OSError:
                    pass
    return 0


# ------------------------------------------------------------------------------------------------------------------ the report

def rgba(s):
    m = re.findall(r"[\d.]+", str(s))
    if len(m) < 3:
        return None
    a = float(m[3]) if len(m) > 3 else 1.0
    if len(m) > 3 and re.search(r"%\s*\)\s*$", str(s)):
        a /= 100.0
    return float(m[0]), float(m[1]), float(m[2]), a


def lum(c):
    def ch(v):
        v /= 255.0
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
    return 0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2])


def light(bg):
    c = rgba(bg)
    return bool(c) and c[3] >= 0.6 and lum(c) >= 0.35


def black(bg):
    c = rgba(bg)
    return bool(c) and c[3] >= 0.9 and lum(c) < 0.01


def name(n):
    cls = (n.get("c") or "").split()
    return "%s%s" % (n.get("t", "?"), ("." + ".".join(cls[:3])) if cls else "")


def load(directory):
    out = []
    for fn in sorted(os.listdir(directory)):
        if fn.endswith(".json"):
            try:
                with open(os.path.join(directory, fn)) as f:
                    d = json.load(f)
                d["_file"] = fn
                out.append(d)
            except ValueError:
                pass
    return out


def summarize(records):
    s = {"posts": len(records), "kinds": {}, "env": None, "dom": [], "focus": [], "classes": [], "csp": [], "errors": []}
    for r in records:
        s["kinds"][r.get("kind")] = s["kinds"].get(r.get("kind"), 0) + 1
        k, d = r.get("kind"), r.get("data")
        if k == "env" and s["env"] is None:
            s["env"] = d
        elif k == "dom":
            s["dom"].append(dom_summary(r, d))
        elif k == "focus":
            s["focus"].append(focus_summary(r, d))
        elif k == "classes":
            s["classes"].append({"file": r["_file"], "key": d.get("key"), "changes": d.get("changes", [])})
        elif k == "csp":
            s["csp"].append(d)
        elif k == "error":
            s["errors"].append(d)
    return s


def dom_summary(rec, d):
    nodes = d.get("nodes", [])
    classes, comps, band, lights, shadows, willc, durations, marks = {}, {}, [], [], 0, 0, {}, 0
    for n in nodes:
        sty = n.get("s") or {}
        for c in (n.get("c") or "").split():
            classes[c] = classes.get(c, 0) + 1
            m = re.match(r"^enact_sandstone_([A-Za-z]+)_\1_([A-Za-z0-9]+)__", c)
            if m:
                comps["%s_%s" % m.groups()] = comps.get("%s_%s" % m.groups(), 0) + 1
        r = n.get("r") or [0, 0, 0, 0]
        if black(sty.get("bg")) and r[2] >= 1000 and r[3] >= 24:
            band.append("%s %s" % (name(n), r))
        if light(sty.get("bg")) and r[2] * r[3] > 2000:
            lights.append("%s %s bg=%s" % (name(n), r, sty.get("bg")))
        for side in ("b", "f"):
            p = n.get(side)
            if p and light(p.get("bg")):
                lights.append("%s::%s %s bg=%s opacity=%s" % (name(n), "before" if side == "b" else "after", r, p.get("bg"), p.get("op", "1")))
        if sty.get("bs"):
            shadows += 1
        if sty.get("wc"):
            willc += 1
        if sty.get("tr"):
            durations[sty["tr"].split(" ")[-1]] = durations.get(sty["tr"].split(" ")[-1], 0) + 1
        marks += sum(1 for a in (n.get("a") or {}) if a.startswith("data-lu-"))
    top = sorted(classes.items(), key=lambda kv: -kv[1])[:40]
    return {"file": rec["_file"], "page": rec.get("page"), "reason": d.get("reason"), "nodes": len(nodes), "truncated": d.get("truncated"),
            "components": sorted(comps.items(), key=lambda kv: -kv[1])[:30], "topClasses": top, "blackBands": band[:12], "lightPlates": lights[:25],
            "boxShadows": shadows, "willChange": willc, "transitions": sorted(durations.items(), key=lambda kv: -kv[1])[:8], "marks": marks}


def focus_summary(rec, d):
    chain = d.get("chain") or []
    sub = d.get("sub") or []
    f = chain[0] if chain else {}
    plates = []
    for n in sub:
        sty = n.get("s") or {}
        r = n.get("r") or [0, 0, 0, 0]
        if light(sty.get("bg")):
            plates.append("self %s %s bg=%s words-inside=%s" % (name(n), r, sty.get("bg"), "?"))
        for side, label in (("b", "::before"), ("f", "::after")):
            p = n.get(side)
            if p and light(p.get("bg")):
                plates.append("%s%s %s bg=%s opacity=%s %sx%s" % (name(n), label, r, p.get("bg"), p.get("op", "1"), p.get("w"), p.get("h")))
    words = [(name(n), (n.get("s") or {}).get("co"), (n.get("s") or {}).get("op")) for n in sub if n.get("x")]
    return {"file": rec["_file"], "focused": "%s %s" % (name(f), f.get("r")), "found": d.get("found"), "lightPlatesSeen": plates[:8], "words": words[:8],
            "ancestors": [name(n) for n in chain[1:5]]}


def report(directory, as_json=False):
    s = summarize(load(directory))
    if as_json:
        print(json.dumps(s, indent=1))
        return 0
    out = ["posts: %d %s" % (s["posts"], json.dumps(s["kinds"]))]
    e = s["env"]
    if e:
        out.append("page: %s  viewport %s  ready %s  theme parts %s" % (e.get("origin"), e.get("vp"), e.get("ready"), e.get("parts")))
        out.append("user agent: %s" % e.get("ua"))
        out.append("font families used by text (count): %s" % json.dumps(sorted(e.get("families", {}).items(), key=lambda kv: -kv[1])[:8]))
        out.append("font faces declared: %s" % "; ".join(e.get("fonts", [])[:24]))
        out.append("stylesheets: %s" % "; ".join("%s (%s)" % (x.get("href"), x.get("rules")) for x in e.get("sheets", [])[:12]))
        out.append("variables (html / body / first child): %s" % json.dumps({k: v for k, v in (e.get("vars") or {}).items() if v}) if any((e.get("vars") or {}).values()) else "variables: none of the --sand-* ones the probe asks for are set")
        out.append("meta CSP: %s" % (e.get("csp") or "none"))
    for d in s["dom"]:
        out.append("\nDOM %s (%s, %s nodes%s) from %s" % (d["file"], d["reason"], d["nodes"], ", truncated" if d["truncated"] else "", d["page"]))
        out.append("  Sandstone components: %s" % json.dumps(d["components"]))
        out.append("  black bands: %s" % (d["blackBands"] or "none"))
        out.append("  light plates at rest: %s" % (d["lightPlates"] or "none"))
        out.append("  box-shadows %d, will-change %d, transitions %s, theme marks %d" % (d["boxShadows"], d["willChange"], json.dumps(d["transitions"]), d["marks"]))
        out.append("  top classes: %s" % json.dumps(d["topClasses"][:20]))
    for fcs in s["focus"]:
        out.append("\nFOCUS %s: %s  (ancestors: %s)" % (fcs["file"], fcs["focused"], ", ".join(fcs["ancestors"])))
        out.append("  the theme found: %s" % json.dumps(fcs["found"]))
        out.append("  light plates in the subtree: %s" % (fcs["lightPlatesSeen"] or "none"))
        out.append("  words (element, colour, opacity): %s" % json.dumps(fcs["words"]))
    for c in s["classes"]:
        out.append("\nCLASS CHANGES after a %s key (%s): how the page marks what the remote moved to" % (c["key"], c["file"]))
        for line in c["changes"][:14]:
            out.append(("  %s %s %s" % (line["el"], ("+" + ",".join(line["add"])) if line["add"] else "", ("-" + ",".join(line["rem"])) if line["rem"] else "")).rstrip())
    if s["csp"]:
        out.append("\nCSP violations: %s" % json.dumps(s["csp"][:10]))
    if s["errors"]:
        out.append("\nerrors: %s" % json.dumps(s["errors"][:10]))
    print("\n".join(out))
    return 0


def main(argv):
    if len(argv) >= 2 and argv[1] == "serve":
        return serve(argv[2:])
    if len(argv) >= 3 and argv[1] == "report":
        return report(argv[2], "--json" in argv[3:])
    print(__doc__.split("\n\n")[0], file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
