"""A fake Hue bridge for the helper's tests: HTTPS (CLIP v2 entertainment areas, the v1 registration) and a DTLS-PSK stream endpoint.

Loopback only. The HTTPS side is python's `ssl`; the DTLS side is OpenSSL's own `s_server` (DTLS 1.2, PSK-AES128-GCM-SHA256), which
python cannot speak, started when an area is started over HTTPS and killed when it is stopped, so the stream exists exactly while
the bridge would accept one.

    fake_hue_bridge.py <config.json>

The config: cert, key (PEM files), dir (where the logs go), udp_port, app_key, app_id, client_key (32 hex), area (UUID), name,
channels ([{"channel_id", "x", "y", "z"}]), link_button (true: the registration succeeds at once).

Prints the HTTPS port on its first line, then reads commands on stdin, one per line:
    press                  the link button is pressed (the registration succeeds from now on)
    takeover <app id>      another application starts streaming to the area (the DTLS side goes away)
    release                ... and lets go
    close_stream           the bridge ends the DTLS session (close_notify) but leaves the area started
    hold                   the next start is answered with an error, as if the area were busy

Files in `dir`: requests.log (one JSON object per HTTPS request: method, path, body, key_ok, key as seen is never written),
dtls.out (what the DTLS side received: raw application data), dtls.err.
"""
import http.server
import json
import ssl
import subprocess
import sys
import threading
import time

cfg = json.load(open(sys.argv[1]))
LOCK = threading.Lock()
state = {
    "active": False,
    "streamer": None,
    "link": bool(cfg.get("link_button")),
    "hold": False,
    "dtls": None,
}


def log(entry):
    with LOCK:
        with open(cfg["dir"] + "/requests.log", "a") as f:
            f.write(json.dumps(entry) + "\n")


def area():
    out = {
        "id": cfg["area"],
        "type": "entertainment_configuration",
        "metadata": {"name": cfg.get("name", "TV")},
        "configuration_type": "screen",
        "status": "active" if state["active"] else "inactive",
        "stream_proxy": {"mode": "auto", "node": {"rid": "11111111-1111-4111-8111-111111111111", "rtype": "entertainment"}},
        "channels": [
            {
                "channel_id": c["channel_id"],
                "position": {"x": c["x"], "y": c["y"], "z": c["z"]},
                "members": [],
            }
            for c in cfg["channels"]
        ],
        "locations": {"service_locations": []},
    }
    if state["active"]:
        out["active_streamer"] = {"rid": state["streamer"], "rtype": "auth_v1"}
    return out


def start_dtls():
    stop_dtls()
    out = open(cfg["dir"] + "/dtls.out", "ab")
    err = open(cfg["dir"] + "/dtls.err", "ab")
    state["dtls"] = subprocess.Popen(
        [
            "openssl", "s_server", "-dtls1_2", "-accept", "127.0.0.1:%d" % cfg["udp_port"], "-nocert",
            "-psk", cfg["client_key"], "-psk_identity", cfg["app_id"], "-cipher", "PSK-AES128-GCM-SHA256",
        ],
        stdin=subprocess.PIPE, stdout=out, stderr=err,
    )
    time.sleep(0.3)


def stop_dtls():
    p = state["dtls"]
    state["dtls"] = None
    if p is not None:
        p.kill()
        p.wait()


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def reply(self, status, body, headers=()):
        data = json.dumps(body).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "close")
        for k, v in headers:
            self.send_header(k, v)
        self.end_headers()
        self.wfile.write(data)
        self.close_connection = True

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n).decode() if n else ""

    def handle_any(self, method):
        body = self.body()
        key_ok = self.headers.get("hue-application-key") == cfg["app_key"]
        log({"method": method, "path": self.path, "body": body, "key_ok": key_ok,
             "has_key": self.headers.get("hue-application-key") is not None})
        path = self.path
        if method == "POST" and path == "/api":
            req = json.loads(body or "{}")
            if not state["link"]:
                return self.reply(200, [{"error": {"type": 101, "address": "", "description": "link button not pressed"}}])
            assert req.get("generateclientkey") is True, req
            return self.reply(200, [{"success": {"username": cfg["app_key"], "clientkey": cfg["client_key"].upper()}}])
        if not key_ok:
            return self.reply(403, {"errors": [{"description": "Forbidden"}]})
        if method == "GET" and path == "/auth/v1":
            return self.reply(200, {}, [("hue-application-id", cfg["app_id"])])
        base = "/clip/v2/resource/entertainment_configuration"
        if method == "GET" and path == base:
            data = [area()]
            if cfg.get("extra_area"):
                other = area()
                other["id"], other["metadata"] = cfg["extra_area"], {"name": "Desk"}
                other["status"] = "inactive"
                other.pop("active_streamer", None)
                data.append(other)
            return self.reply(200, {"errors": [], "data": data})
        if path == base + "/" + cfg["area"]:
            if method == "GET":
                return self.reply(200, {"errors": [], "data": [area()]})
            if method == "PUT":
                action = json.loads(body).get("action")
                ok = {"errors": [], "data": [{"rid": cfg["area"], "rtype": "entertainment_configuration"}]}
                with LOCK:
                    if action == "start":
                        if state["hold"] or state["active"]:
                            state["hold"] = False
                            return self.reply(200, {"errors": [{"description": "the area is busy"}], "data": []})
                        state["active"], state["streamer"] = True, cfg["app_id"]
                        start_dtls()
                        return self.reply(200, ok)
                    if action == "stop":
                        stop_dtls()
                        state["active"], state["streamer"] = False, None
                        return self.reply(200, ok)
                return self.reply(400, {"errors": [{"description": "unknown action"}]})
        return self.reply(404, {"errors": [{"description": "not found"}]})

    def do_GET(self):
        self.handle_any("GET")

    def do_PUT(self):
        self.handle_any("PUT")

    def do_POST(self):
        self.handle_any("POST")


def commands():
    for line in sys.stdin:
        word = line.split()
        if not word:
            continue
        with LOCK:
            if word[0] == "press":
                state["link"] = True
            elif word[0] == "takeover":
                stop_dtls()
                state["active"], state["streamer"] = True, word[1]
            elif word[0] == "release":
                state["active"], state["streamer"] = False, None
            elif word[0] == "close_stream":
                p = state["dtls"]
                if p is not None:
                    p.stdin.write(b"q\n")
                    p.stdin.flush()
            elif word[0] == "hold":
                state["hold"] = True
    stop_dtls()
    sys.exit(0)


ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(cfg["cert"], cfg["key"])
server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
server.socket = ctx.wrap_socket(server.socket, server_side=True)
print(server.server_address[1], flush=True)
threading.Thread(target=commands, daemon=True).start()
try:
    server.serve_forever()
finally:
    stop_dtls()
