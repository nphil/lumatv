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
import os
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


LEVELS = ["never", "ask", "always"]
GROUPS = [("state", "See what is playing, power and input state", "always"), ("playback", "Play, pause, volume, mute", "always"),
          ("launch", "Open apps, switch inputs", "ask"), ("power", "Turn the TV off", "ask")]


def ai_doc(enabled=False, assistants=None, pairing=None):
    return {"listener": {"enabled": enabled, "bound": enabled, "addr": "192.168.1.242:18732" if enabled else None, "port": 18732, "since": NOW() if enabled else None, "error": None},
            "pairing": pairing, "assistants": assistants or [], "limits": {"maxVolume": 60},
            "groups": [{"id": g, "title": t, "default": d} for g, t, d in GROUPS],
            "tools": []}


def assistant(i, name, levels=None, paused=False, calls=0):
    return {"id": i, "name": name, "createdAt": NOW() - 86400000, "lastSeenAt": NOW() - 120000, "paused": paused, "calls": calls,
            "levels": levels or {g: d for g, _, d in GROUPS}}


def ambient_schema():
    """The `base.ambient` settings owner, from the helper's own schema file, so what the fake accepts is what the real store accepts."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "helper", "app", "src", "schemas", "base.ambient.json")
    with open(path) as f:
        doc = json.load(f)
    return {fl["key"]: fl for s in doc["sections"] for fl in s["fields"]}


def voice_schema():
    """The `base.voice` settings owner, from the helper's own schema file."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..", "helper", "app", "src", "schemas", "base.voice.json")
    with open(path) as f:
        doc = json.load(f)
    return {fl["key"]: fl for s in doc["sections"] for fl in s["fields"]}


def ambient_check(schema, key, value):
    """Why the helper's schema would refuse `value` for `key`, or None."""
    f = schema.get(key)
    if f is None:
        return "no such setting " + key
    kind, v = f["kind"], value.get("value")
    if value.get("type") != kind["type"]:
        return "%s takes a %s" % (key, kind["type"])
    if kind["type"] == "integer":
        if not isinstance(v, int) or isinstance(v, bool) or v < kind["min"] or v > kind["max"] or (v - kind["min"]) % kind["step"]:
            return "%s must be %d to %d in steps of %d" % (key, kind["min"], kind["max"], kind["step"])
    if kind["type"] == "choice" and v not in [o["id"] for o in kind["options"]["inline"]]:
        return "%s is not one of the listed choices" % v
    if kind["type"] == "bool" and not isinstance(v, bool):
        return "%s is a switch" % key
    return None


HA = "home-assistant"


def ha_fixture():
    """What the real Home Assistant plugin and the helper make for a house with a room, a doorbell that rang and a notification
    (tests/qmlhost/fixtures/ha-plugin.json, written by helper/app/tests/ha_plugin.rs, which fails when it is not what they make
    now): the settings page, the pickers' pages and the topics. None if the file is not there."""
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "fixtures", "ha-plugin.json")
    try:
        with open(path) as f:
            return json.load(f)
    except OSError:
        return None


def ha_view(st):
    """`GET /v1/settings/home-assistant`: the plugin's page exists only while it is connected, as in the helper."""
    roster = (st.topics.get("plugins") or {}).get("plugins") or []
    if st.ha is None or not any(p.get("id") == HA and p.get("state") == "connected" for p in roster):
        return None
    return {"ok": True, "api": 1, "owner": HA, "version": st.ha_version, "schemaRevision": 1, "schema": st.ha["settings"]["schema"],
            "values": st.ha_effective, "effective": st.ha_effective, "invalid": {}, "unknown": []}


def ha_check(st, key, value):
    """Why the helper would refuse this change of the plugin's page, or None (the schema's kind is the rule)."""
    fields = {f["key"]: f for sec in st.ha["settings"]["schema"]["sections"] for f in sec["fields"]}
    if key not in fields:
        return "no such setting " + key
    want = {"choice": "choice", "ordered-set": "ordered", "bool": "bool", "integer": "integer", "text": "text"}[fields[key]["kind"]["type"]]
    if value.get("type") != want:
        return "%s takes a %s" % (key, want)
    return None


def updateblock(state):
    """The helper's `updateblock` topic (helper/app/src/updateblock.rs) in one of its four states."""
    flag, live = {"blocked": (True, True), "armed": (True, False), "hosts-only": (False, True), "open": (False, False)}[state]
    return {"flag": flag, "hosts": {n: live for n in ("snu.lge.com", "su.lge.com", "su-ssl.lge.com", "su-dev.lge.com")}, "live": live,
            "blocking": flag or live, "state": state}


def panel(count=12, last=1700.0, usage=1705.0, interval=4.0):
    """The helper's `panel` topic (helper/app/src/refreshguard.rs) with the screen on."""
    return {"read": True, "offrsCount": count, "lastRunHours": last, "intervalHours": interval, "usageHours": usage, "due": usage >= last + interval,
            "lastStandbyMinutes": 42, "standby": None, "guard": {"enabled": False, "deferrals": 0, "last": None}, "error": None, "readAt": NOW()}


# A 32x18 baseline JPEG: what the frame route of the cameras answers (the shell decodes it as any camera picture).
JPEG = bytes.fromhex(
    "ffd8ffe000104a46494600010100000100010000ffdb0043000a07070807060a0808080b0a0a0b0e18100e0d0d0e1d15161118231f2524221f222126"
    "2b372f26293429212230413134393b3e3e3e252e4449433c48373d3e3bffdb0043010a0b0b0e0d0e1c10101c3b2822283b3b3b3b3b3b3b3b3b3b3b3b"
    "3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3bffc00011080012002003012200021101031101ffc400"
    "1f0000010501010101010100000000000000000102030405060708090a0bffc400b5100002010303020403050504040000017d010203000411051221"
    "31410613516107227114328191a1082342b1c11552d1f02433627282090a161718191a25262728292a3435363738393a434445464748494a53545556"
    "5758595a636465666768696a737475767778797a838485868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6"
    "c7c8c9cad2d3d4d5d6d7d8d9dae1e2e3e4e5e6e7e8e9eaf1f2f3f4f5f6f7f8f9faffc4001f0100030101010101010101010000000000000102030405"
    "060708090a0bffc400b51100020102040403040705040400010277000102031104052131061241510761711322328108144291a1b1c109233352f015"
    "6272d10a162434e125f11718191a262728292a35363738393a434445464748494a535455565758595a636465666768696a737475767778797a828384"
    "85868788898a92939495969798999aa2a3a4a5a6a7a8a9aab2b3b4b5b6b7b8b9bac2c3c4c5c6c7c8c9cad2d3d4d5d6d7d8d9dae2e3e4e5e6e7e8e9ea"
    "f2f3f4f5f6f7f8f9faffda000c03010002110311003f00f35a28a2ba8a0a28a2800a28a2800a28a2803fffd9")


def openapps(enabled, listed, state=None):
    """The helper's `openapps` topic (helper/app/src/openapps.rs): the setting and LG's list of apps waiting for an update. On, the helper has emptied
    the list (`holding`) unless the test says LG's updater is running (`waiting`) or the list was just written (`clearing`)."""
    shown = [] if enabled and state in (None, "holding") else list(listed)
    return {"enabled": enabled, "entries": len(shown), "ids": shown[:8], "state": state or ("holding" if enabled else "off"), "saved": enabled and bool(listed)}


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
        self.topics["ai"] = ai_doc()
        self.topics["ai.prompt"] = None
        self.topics["updateblock"] = updateblock("blocked")
        self.openapps_listed = ["com.webos.app.lifeonscreen", "dazn"]
        self.topics["openapps"] = openapps(False, self.openapps_listed)
        self.topics["panel"] = panel()
        self.requests = []      # (method, path, body) of every /v1/ai call, for the tests to read back
        self.posts = []         # (path, body) of every other POST (plugin actions, receiver nudge and set), read back at /__posts
        self.audit = []
        self.ambient_schema = ambient_schema()
        self.ambient = {k: f["default"] for k, f in self.ambient_schema.items()}      # key -> {type, value}: the owner's stored values over its defaults
        self.voice_fields = voice_schema()
        self.voice_values = {k: f["default"] for k, f in self.voice_fields.items()}    # key -> {type, value}
        self.ha = ha_fixture()
        self.ha_effective = json.loads(json.dumps(self.ha["settings"]["effective"])) if self.ha else {}
        self.ha_version = 1
        self.ambient_state = "idle"
        self.ambient_strip = True
        self.pending = None     # (prompt doc, what to do when a physical remote approves it)
        self.voice_session = 0
        self.frames = []        # the camera pictures asked for: (camera, w, h, when in ms), read back at /__frames
        self.crash = None       # the newest crash bundle as helper/app/src/crash.rs says it ({pending, kind, at, reason, ...}); set at POST /__crash
        self.capture = None     # the `capture` object of helper/app/src/capture.rs's GET /v1/capture (None: switched off); set at POST /__capture
        self.topics["cameras.popup"] = None
        # the cameras' routes (helper/app/src/scrypted/routes.rs): a test sets what they answer
        self.pictures = {}      # camera -> JPEG bytes the stills and the thumbnails answer (else the 32x18 stand-in)
        self.event_pictures = {}    # event id -> JPEG bytes of `/events/{event}/thumbnail` (else 404 event_not_found)
        self.camera_routes = {}     # (camera, "recordings" | "events" | "clips") -> the JSON the route answers
        self.no_picture = set()     # cameras whose stills are refused with 404 no_picture
        self.refusals = {}          # path of a POST -> (status, code, message): the helper refuses it as the cameras routes do
        self.thumb_cycle = []       # JPEGs the thumbnails of the timeline answer in turn (else the camera's still)
        self.camera_asks = []       # [path, query] of every timeline and thumbnail ask, read back at /__camera_asks
        self.cameras_settings = {"version": 1, "schemaRevision": 1, "effective": {}}   # GET and POST /v1/settings/scrypted-cameras

    def publish(self, topic, data):
        if topic == "__audit":              # not a topic: the recent-activity list of /v1/ai/audit
            self.audit = data
            return
        with self.cv:
            self.seq += 1
            self.topics[topic] = data
            self.events.append((self.seq, topic, data))
            self.cv.notify_all()


def ambient_view(st):
    return {"ok": True, "api": 1, "owner": "base.ambient", "version": 1, "schema": {"sections": [{"id": "all", "fields": list(st.ambient_schema.values())}]},
            "values": st.ambient, "effective": st.ambient, "invalid": {}, "unknown": []}


def voice_view(st):
    return {"ok": True, "api": 1, "owner": "base.voice", "version": 1, "schema": {"sections": [{"id": "all", "fields": list(st.voice_fields.values())}]},
            "values": st.voice_values, "effective": st.voice_values, "invalid": {}, "unknown": []}


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

        def jpeg(self, data):
            self.send_response(200)
            self.send_header("Content-Type", "image/jpeg")
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(data)

        def camera_get(self, u, q):
            """The timeline and the thumbnails of a camera: `/v1/cameras/{id}/{recordings,events,clips,thumbnail,clip-thumbnail}` and
            `/v1/cameras/events/{event}/thumbnail`."""
            parts = u.path.split("/")[3:]
            st.camera_asks.append([u.path, {k: v[0] for k, v in q.items()}])
            if parts[0] == "events" and len(parts) == 3 and parts[2] == "thumbnail":
                pic = st.event_pictures.get(parts[1])
                if pic is None:
                    return self.send(404, {"ok": False, "api": 1, "error": {"code": "event_not_found", "message": "no such event"}})
                return self.jpeg(pic)
            camera, kind = parts[0], parts[1] if len(parts) > 1 else ""
            if kind in ("thumbnail", "clip-thumbnail"):
                # a test can set `thumb_cycle` (JPEGs) so the pictures of one camera differ from ask to ask
                if st.thumb_cycle:
                    return self.jpeg(st.thumb_cycle[(len(st.camera_asks) - 1) % len(st.thumb_cycle)])
                return self.jpeg(st.pictures.get(camera, JPEG))
            if kind in ("recordings", "events", "clips"):
                doc = st.camera_routes.get((camera, kind))
                if doc is None:
                    doc = {"recordings": {"camera": camera, "ranges": [], "unknown": []}, "events": {"camera": camera, "events": []}, "clips": {"camera": camera, "clips": []}}[kind]
                return self.send(200, dict({"ok": True, "api": 1, "now": NOW()}, **doc))
            self.send(404, {"ok": False, "api": 1, "error": {"code": "not_found", "message": "no such route"}})

        def ai_reply(self, code, **extra):
            self.send(code, dict({"ok": True, "api": 1, "now": NOW(), "ai": st.topics["ai"]}, **extra))

        def ask(self, title, detail, kind, apply_fn, assistant=None):
            """A change that gives more: a prompt opens and waits for the test's `/__approve` (the physical remote)."""
            if st.pending:
                self.send(409, {"ok": False, "api": 1, "error": {"code": "busy", "message": "another prompt is open on the TV"}})
                return
            doc = {"id": "p%d" % NOW(), "kind": kind, "assistant": assistant, "title": title, "detail": detail, "reason": None,
                   "shownAt": NOW(), "expiresAt": NOW() + 30000, "allow": "OK", "deny": "Back"}
            st.pending = (doc, apply_fn)
            st.publish("ai.prompt", doc)
            self.send(202, {"ok": True, "api": 1, "now": NOW(), "pending": True, "promptId": doc["id"]})

        def ai_route(self, method, path, body):
            st.requests.append((method, path, body))
            doc = json.loads(json.dumps(st.topics["ai"]))
            parts = [p for p in path.split("/") if p][2:]
            by_id = lambda i: next((a for a in doc["assistants"] if a["id"] == i), None)

            def commit(d):
                st.publish("ai", d)

            if method == "GET" and parts == []:
                return self.ai_reply(200)
            if method == "GET" and parts == ["audit"]:
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "entries": st.audit})
            if method == "POST" and parts == ["listener"]:
                if not body.get("enabled"):
                    doc["listener"].update({"enabled": False, "bound": False, "addr": None, "since": None})
                    commit(doc)
                    return self.ai_reply(200, changed=True, pending=False)

                def on(d):
                    d["listener"].update({"enabled": True, "bound": True, "addr": "192.168.1.242:18732", "since": NOW()})
                    return d
                return self.ask("Turn on AI access?", "AI assistants on your home network will be able to connect to this TV and use the tools you allow them. Press OK to turn it on, Back to cancel.",
                                "enable_listener", on)
            if method == "POST" and parts == ["pairing"]:
                def open_window(d):
                    d["pairing"] = {"code": "ABCD-EFGH", "expiresAt": NOW() + 120000, "attemptsLeft": 5}
                    return d
                return self.ask("Pair a new assistant?", "A one-time code will show on this screen. Press OK to show it, Back to cancel.", "pair", open_window)
            if method == "DELETE" and parts == ["pairing"]:
                doc["pairing"] = None
                commit(doc)
                return self.ai_reply(200, changed=True, pending=False)
            if parts[:1] == ["assistants"] and len(parts) >= 2:
                a = by_id(parts[1])
                if not a:
                    return self.send(404, {"ok": False, "api": 1, "error": {"code": "no_such_assistant", "message": "no such assistant"}})
                if method == "DELETE" and len(parts) == 2:
                    doc["assistants"] = [x for x in doc["assistants"] if x["id"] != parts[1]]
                    commit(doc)
                    return self.ai_reply(200, changed=True, pending=False)
                if method == "POST" and parts[2:] == ["pause"]:
                    a["paused"] = True
                    commit(doc)
                    return self.ai_reply(200, changed=True, pending=False)
                if method == "POST" and parts[2:] == ["resume"]:
                    def resume(d, i=parts[1]):
                        next(x for x in d["assistants"] if x["id"] == i)["paused"] = False
                        return d
                    return self.ask("Resume %s?" % a["name"], "It will be able to use the tools you allow it again.", "resume", resume, {"id": a["id"], "name": a["name"]})
                if method == "POST" and parts[2:] == ["levels"]:
                    want = body.get("levels", {})
                    wider = any(LEVELS.index(want[g]) > LEVELS.index(a["levels"][g]) for g in want)
                    if not wider:
                        a["levels"].update(want)
                        commit(doc)
                        return self.ai_reply(200, changed=True, pending=False)

                    def widen(d, i=parts[1], w=want):
                        next(x for x in d["assistants"] if x["id"] == i)["levels"].update(w)
                        return d
                    return self.ask("Give %s more?" % a["name"], "It will be able to do more without asking. Press OK to allow it, Back to cancel.", "widen", widen, {"id": a["id"], "name": a["name"]})
            if method == "POST" and parts == ["lockout"]:
                doc["assistants"] = []
                doc["pairing"] = None
                doc["listener"].update({"enabled": False, "bound": False, "addr": None, "since": None})
                commit(doc)
                return self.ai_reply(200, changed=True, pending=False)
            return self.send(404, {"ok": False, "api": 1, "error": {"code": "not_found", "message": "no such route"}})

        def voice_route(self, path, body):
            """Voice search as the helper's /v1/voice/* answers it. Not a model of the speech: the `voice` topic a test publishes
            says whether it is `ready` and, in a `say` key that only this fake reads, what the next request hears: {"text": ...} or
            {"error": {"code", "message"}}, after `delayMs` (150); a `refuse` {code, message} makes listen answer 409 with it."""
            st.posts.append([path, body])
            cur = st.topics.get("voice") or {"state": "idle", "session": 0, "ready": False}
            base = {k: v for k, v in cur.items() if k not in ("text", "error")}
            if path == "/v1/voice/listen":
                if not cur.get("ready") or cur.get("refuse"):
                    return self.send(409, {"ok": False, "api": 1, "error": cur.get("refuse") or {"code": "voice_off", "message": "voice search is switched off"}})
                st.voice_session += 1
                st.publish("voice", dict(base, state="listening", session=st.voice_session))
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "session": st.voice_session})
            if path == "/v1/voice/stop":
                session, say = cur.get("session", 0), dict(cur.get("say") or {"error": {"code": "nothing_heard", "message": "Nothing was heard"}})
                delay = say.pop("delayMs", 150) / 1000
                st.publish("voice", dict(base, state="thinking"))
                answer = dict(base, state="heard", **say) if "text" in say else dict(base, state="error", **say)
                threading.Timer(delay, lambda: st.publish("voice", answer) if (st.topics.get("voice") or {}).get("session") == session else None).start()
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "stopped": True})
            if path == "/v1/voice/cancel":
                st.publish("voice", dict(base, state="idle"))
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "cancelled": True})
            return self.send(200, {"ok": True, "api": 1})

        def do_DELETE(self):
            u = urlparse(self.path)
            if u.path.startswith("/v1/ai"):
                return self.ai_route("DELETE", u.path, {})
            self.send(404, {"ok": False})

        def do_GET(self):
            u = urlparse(self.path)
            q = parse_qs(u.query)
            if u.path.startswith("/v1/ai"):
                return self.ai_route("GET", u.path, {})
            if u.path == "/v1/settings/base.ambient":
                return self.send(200, ambient_view(st))
            if u.path == "/v1/settings/base.voice":
                return self.send(200, voice_view(st))
            if u.path == "/v1/settings/" + HA:
                view = ha_view(st)
                return self.send(200, view) if view else self.send(404, {"ok": False, "api": 1, "error": {"code": "no_such_owner", "message": "no such settings owner"}})
            if u.path.startswith("/v1/plugins/" + HA + "/choices/"):
                provider = u.path.rsplit("/", 1)[1]
                if st.ha is None or provider not in st.ha["choices"]:
                    return self.send(404, {"ok": False, "api": 1, "error": {"code": "no_such_provider", "message": "no such provider"}})
                page = json.loads(json.dumps(st.ha["choices"][provider]))
                link = (st.topics.get("ha.link") or {}).get("state", "off")
                if link != "connected":              # the plugin asks Home Assistant, which cannot answer: the page says why and has no rows
                    page.update({"state": "offline" if link == "offline" else "unconfigured", "rows": [], "more": False})
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "plugin": HA, "provider": provider, "page": page})
            if u.path == "/v1/ambient":
                on = st.ambient["enabled"]["value"]
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "enabled": on, "mode": st.ambient["mode"]["value"], "state": st.ambient_state if on or st.ambient_state == "testing" else "off",
                                       "sections": sum(st.ambient[k]["value"] for k in ("top", "right", "bottom", "left")), "strip": st.ambient_strip, "hue": False, "problem": None})
            if u.path == "/__requests":
                return self.send(200, {"requests": st.requests})
            if u.path == "/__frames":
                return self.send(200, {"frames": st.frames})
            if u.path.startswith("/v1/cameras/") and u.path.endswith("/frame"):
                # a camera's picture, as the helper's GET /v1/cameras/{id}/frame?w=&h= (the file itself: no decoder on the host)
                camera = u.path[len("/v1/cameras/"):-len("/frame")]
                st.frames.append([camera, int(q.get("w", ["0"])[0]), int(q.get("h", ["0"])[0]), NOW()])
                if camera in st.no_picture:
                    return self.send(404, {"ok": False, "api": 1, "error": {"code": "no_picture", "message": "no picture yet"}})
                return self.jpeg(st.pictures.get(camera, JPEG))
            if u.path.startswith("/v1/cameras/") and u.path.count("/") >= 4:
                return self.camera_get(u, q)
            if u.path == "/v1/settings/scrypted-cameras":
                return self.send(200, dict({"ok": True, "api": 1, "now": NOW(), "owner": "scrypted-cameras"}, **st.cameras_settings))
            if u.path == "/__camera_asks":
                return self.send(200, {"asks": st.camera_asks})
            if u.path == "/v1/crash":
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "crash": st.crash})
            if u.path == "/v1/capture":
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "capture": st.capture or {"enabled": False, "available": False}})
            if u.path == "/__posts":
                return self.send(200, {"posts": st.posts})
            if u.path.startswith("/v1/state/"):         # one topic's newest copy, as the helper's GET /v1/state/{topic}
                name = u.path[len("/v1/state/"):]
                if name not in st.topics:
                    return self.send(404, {"ok": False, "error": {"code": "no_such_topic"}})
                return self.send(200, {"ok": True, "api": 1, "now": NOW(), "boot": st.boot, "seq": st.seq, "topic": name, "value": st.topics[name]})
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
            elif u.path.startswith("/__quiet/"):
                # a topic's retained value changes with no event, as a stamp that moves on every poll does (hub.set_ignoring)
                with st.cv:
                    st.topics[u.path.rsplit("/", 1)[1]] = body
                self.send(200, {"ok": True})
            elif u.path in ("/__approve", "/__deny"):
                # the physical remote: only this test door answers a prompt, as only the real remote does
                if st.pending:
                    doc, apply_fn = st.pending
                    st.pending = None
                    st.publish("ai.prompt", None)
                    if u.path == "/__approve":
                        st.publish("ai", apply_fn(json.loads(json.dumps(st.topics["ai"]))))
                self.send(200, {"ok": True})
            elif u.path == "/v1/settings/base.ambient":
                changes = body.get("changes", [])
                for c in changes:
                    why = ambient_check(st.ambient_schema, c["key"], c["value"])
                    if why:
                        st.posts.append([u.path, body])
                        return self.send(400, {"ok": False, "api": 1, "error": {"code": "invalid", "message": why}})
                for c in changes:
                    st.ambient[c["key"]] = c["value"]
                st.posts.append([u.path, body])
                self.send(200, {"ok": True, "api": 1, "changed": [c["key"] for c in changes], "version": 1, "settings": ambient_view(st)})
            elif u.path == "/v1/settings/base.voice":
                changes = body.get("changes", [])
                st.posts.append([u.path, body])
                for c in changes:
                    if c["key"] not in st.voice_fields or c["value"].get("type") != st.voice_fields[c["key"]]["kind"]["type"]:
                        return self.send(422, {"ok": False, "api": 1, "error": {"code": "invalid", "message": "a setting was refused"}})
                for c in changes:
                    st.voice_values[c["key"]] = c["value"]
                self.send(200, {"ok": True, "api": 1, "changed": True, "version": 2, "settings": voice_view(st)})
            elif u.path == "/v1/settings/scrypted-cameras":
                st.posts.append([u.path, body])
                cur = st.cameras_settings
                if body.get("ifVersion") not in (None, cur["version"]):
                    return self.send(409, {"ok": False, "api": 1, "error": {"code": "conflict", "message": "the settings changed since they were read", "version": cur["version"]}})
                for c in body.get("changes", []):
                    cur["effective"][c["key"]] = c["value"]
                cur["version"] += 1
                self.send(200, {"ok": True, "api": 1, "changed": [c["key"] for c in body.get("changes", [])], "version": cur["version"],
                                "settings": dict({"owner": "scrypted-cameras"}, **cur)})
            elif u.path == "/v1/settings/" + HA:
                st.posts.append([u.path, body])
                view = ha_view(st)
                if not view:
                    return self.send(404, {"ok": False, "api": 1, "error": {"code": "no_such_owner", "message": "no such settings owner"}})
                changes = body.get("changes", [])
                for c in changes:
                    why = ha_check(st, c["key"], c["value"])
                    if why:
                        return self.send(422, {"ok": False, "api": 1, "error": {"code": "invalid", "message": why}})
                for c in changes:
                    st.ha_effective[c["key"]] = c["value"]
                st.ha_version += 1
                self.send(200, {"ok": True, "api": 1, "changed": True, "version": st.ha_version, "settings": ha_view(st)})
            elif u.path == "/v1/plugins/" + HA + "/actions/lease.ha.forget":
                # the helper's own action: the saved link goes and the lease says it is not linked
                st.posts.append([u.path, body])
                link = dict(st.topics.get("ha.link") or {}, state="unconfigured", detail="Home Assistant is not linked yet", address=None, token=False)
                st.publish("ha.link", link)
                self.send(202, {"ok": True, "api": 1, "op": 1})
            elif u.path == "/__crash":
                st.crash = body or None
                self.send(200, {"ok": True})
            elif u.path == "/__capture":
                st.capture = body or None
                self.send(200, {"ok": True})
            elif u.path in ("/v1/capture/screenshot", "/v1/capture/record/start", "/v1/capture/record/stop"):
                # the helper's answers: 403 while the switch is off, the compositor's refusals when the test set one in `refuse`, else done
                st.posts.append([u.path, body])
                cap = st.capture or {}
                refuse = cap.get("refuse")
                if not cap.get("enabled"):
                    return self.send(403, {"ok": False, "api": 1, "error": {"code": "capture_disabled", "message": "screen capture is off"}})
                if refuse:
                    return self.send(refuse.get("status", 409), {"ok": False, "api": 1, "error": {"code": refuse["code"], "message": "refused"}})
                if u.path.endswith("/start"):
                    st.capture = dict(cap, recording=True, recorder={"stage": "recording", "seconds": 3.2})
                    return self.send(200, {"ok": True, "api": 1, "recording": True, "name": "rec-test.mp4"})
                if u.path.endswith("/stop"):
                    st.capture = dict(cap, recording=False, recorder=None)
                    return self.send(200, {"ok": True, "api": 1, "recording": False, "name": "rec-test.mp4", "seconds": 4.1})
                self.send(200, {"ok": True, "api": 1, "name": "shot-test.png", "bytes": 812345})
            elif u.path == "/v1/crash/ack":
                st.posts.append([u.path, body])
                cleared = bool(st.crash and st.crash.get("pending"))
                if st.crash:
                    st.crash = dict(st.crash, pending=False)
                self.send(200, {"ok": True, "api": 1, "cleared": cleared})
            elif u.path == "/v1/openapps":
                # the helper's own: the flag is set (or removed) and the list is emptied (or put back) at once
                st.posts.append([u.path, body])
                if set(body) != {"enabled"} or not isinstance(body["enabled"], bool):
                    return self.send(400, {"ok": False, "api": 1, "error": {"code": "bad_request", "message": "the body is {\"enabled\": true or false}"}})
                st.publish("openapps", openapps(body["enabled"], st.openapps_listed))
                self.send(200, {"ok": True, "api": 1, "openapps": st.topics["openapps"]})
            elif u.path == "/v1/ambient/test":
                st.ambient_state = "testing" if body.get("run") else "idle"
                st.posts.append([u.path, body])
                self.send(200, {"ok": True, "api": 1})
            elif u.path.startswith("/v1/ai"):
                self.ai_route("POST", u.path, body)
            elif u.path.startswith("/v1/voice/"):
                self.voice_route(u.path, body)
            else:
                st.posts.append([u.path, body])
                refuse = st.refusals.get(u.path)
                if refuse:
                    return self.send(refuse[0], {"ok": False, "api": 1, "error": {"code": refuse[1], "message": refuse[2]}})
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
