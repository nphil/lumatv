#!/usr/bin/env python3
"""A stand-in for lumatv-helper's v1 API on 127.0.0.1:PORT, for the offline QML host.

    fakehelper.py [--port 18731] [--weather-place NAME] [--no-news]

Serves /v1/state and a long-poll /v1/events in the shape of docs/research/native-helper-result.md:
`topics` (screen, weather, news, receiver), `seq`, `boot`. `POST /__publish/<topic>` with a JSON body
sets a topic and raises an event, so a test can change what the cards draw while the surface runs.
Not a model of the helper's behaviour; the helper's own tests cover that.
"""
import argparse
import json
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlparse

NOW = lambda: int(time.time() * 1000)


def weather(place):
    t = NOW()
    days = [{"day": d, "kind": k, "hi": f"{70 + i}\u00b0", "lo": f"{50 + i}\u00b0"}
            for i, (d, k) in enumerate([("Today", "sun"), ("Mon", "cloud"), ("Tue", "rain"), ("Wed", "partly"), ("Thu", "snow")])]
    doc = {"schema": 1, "fetchedAt": t, "staleAt": t + 900000, "expiresAt": t + 21600000, "zip": "30311", "units": "fahrenheit",
           "place": place, "display": {"place": place, "temp": "66\u00b0", "cond": "Clear", "stats": "Feels like 64\u00b0 \u00b7 40% humidity \u00b7 5 mph",
                                       "nowKind": "sun", "days": days}}
    return {"doc": doc, "status": {"failures": 0, "error": None, "lastAttemptAt": t}, "state": "fresh", "ageMs": 0}


def news(headline):
    t = NOW()
    items = [{"tag": "WORLD", "src": "BBC News", "title": headline, "summary": "A summary of the story.", "link": "https://example.invalid/a",
              "img": "", "pub": "Sun, 25 Oct 2026 04:30:00 GMT", "pubMs": 1792902600000},
             {"tag": "U.S.", "src": "BBC News", "title": "A second story", "summary": "", "link": "", "img": "", "pub": "", "pubMs": None}]
    doc = {"schema": 1, "fetchedAt": t, "staleAt": t + 900000, "expiresAt": t + 21600000, "feeds": ["x", "y"], "items": items}
    return {"doc": doc, "status": {"failures": 0, "error": None, "lastAttemptAt": t}, "state": "fresh", "ageMs": 0}


class State:
    def __init__(self, a):
        self.boot = NOW()
        self.seq = 10
        self.cv = threading.Condition()
        self.topics = {"screen": {"on": True, "state": "Active", "source": "luna-subscribe"},
                       "weather": weather(a.weather_place),
                       "receiver": {"enabled": True, "reachable": True, "fresh": True, "power": "on", "volumeDb": -44.5, "mute": False}}
        if not a.no_news:
            self.topics["news"] = news("HEADLINE FROM THE HELPER")
        self.events = []        # (seq, topic, data)

    def publish(self, topic, data):
        with self.cv:
            self.seq += 1
            self.topics[topic] = data
            self.events.append((self.seq, topic, data))
            self.cv.notify_all()


def handler(st):
    class H(BaseHTTPRequestHandler):
        def log_message(self, *a):
            pass

        def send(self, code, obj):
            body = json.dumps(obj).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            u = urlparse(self.path)
            q = parse_qs(u.query)
            if u.path == "/v1/state":
                self.send(200, {"ok": True, "api": 1, "now": NOW(), "boot": st.boot, "seq": st.seq, "helper": {}, "topics": st.topics})
            elif u.path == "/v1/events":
                after = int(q.get("after", ["0"])[0])
                wait = min(float(q.get("timeout", ["25"])[0]), 30)
                end = time.time() + wait
                with st.cv:
                    while not [e for e in st.events if e[0] > after] and time.time() < end:
                        st.cv.wait(timeout=end - time.time())
                    evs = [{"seq": s, "at": NOW(), "topic": t, "kind": "changed", "data": d} for s, t, d in st.events if s > after]
                    seq = max([e["seq"] for e in evs], default=after)
                self.send(200, {"ok": True, "api": 1, "now": NOW(), "boot": st.boot, "seq": seq, "reset": False, "events": evs})
            else:
                self.send(404, {"ok": False})

        def do_POST(self):
            u = urlparse(self.path)
            n = int(self.headers.get("Content-Length", "0"))
            body = json.loads(self.rfile.read(n) or b"{}")
            if u.path.startswith("/__publish/"):
                st.publish(u.path.rsplit("/", 1)[1], body)
                self.send(200, {"ok": True})
            else:
                self.send(200, {"ok": True, "api": 1})
    return H


def serve(port, a=None):
    a = a or argparse.Namespace(weather_place="HELPERVILLE", no_news=False)
    st = State(a)
    srv = ThreadingHTTPServer(("127.0.0.1", port), handler(st))
    srv.daemon_threads = True
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    return srv, st


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--port", type=int, default=18731)
    ap.add_argument("--weather-place", default="HELPERVILLE")
    ap.add_argument("--no-news", action="store_true")
    a = ap.parse_args()
    srv, st = serve(a.port, a)
    print(f"fake helper on 127.0.0.1:{a.port}", flush=True)
    threading.Event().wait()
