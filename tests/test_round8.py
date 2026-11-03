"""Run with: python3 -m unittest tests.test_round8

docs/research/round8/tv_round8.py, Round 8's TV slot, run against a simulated set: the power state, SAM's foreground and launches, the
helper's loopback API with the plugin roster, the development door, the remote's keys, the X4 monitor, the maliit tree, the logo record and
a receiver that Home Assistant mutes. It says the script's flow and its parsing agree with one another and with the lines the notes say
the surfaces, the helper and LG's services give (the strings it looks for are the ones the notes, the X4 census and the QML name). It says
nothing about the TV: the simulation is written from the same reading of those notes as the script is. The plugin's own conversation is
run for real against a stand-in broker on a unix socket.
"""
import gzip
import hashlib
import importlib.util
import io
import json
import os
import socket
import struct
import subprocess
import sys
import tarfile
import tempfile
import threading
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
ROUND8 = os.path.join(ROOT, "docs", "research", "round8")
PLEX = "com.beb.plxnative"
HOME = "com.webos.app.home"
DATA = "/var/lib/lumatv"
ROSTER = DATA + "/plugins"
# What LumaShow's plugin-api crate computes for docs/research/round8/plugin/plugin.json (Manifest::consent_document and its SHA-256,
# 2026-10-25; the document is below). The script rebuilds it in Python, so the two must agree.
RUST_CONSENT = ('{"consent":1,"id":"round8","api":{"min":1,"target":1},"exe":"bin/round8","args":[],"class":"S","requires":[],'
                '"permissions":["cards","tiles","notify","background"],"limits":{}}')
RUST_APPROVAL = "8bb95c0e71907b3d09fa838af10a87265be6ce8b78e453d0af34c8f56ca697d5"

try:
    from PIL import Image
except ImportError:      # the picture checks then say "could not compare" and the tests that need pictures are skipped
    Image = None


def load():
    spec = importlib.util.spec_from_file_location("tv_round8", os.path.join(ROUND8, "tv_round8.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)

    class Clock:        # the script's waits cost nothing here: sleeping moves a clock that nothing else moves
        now = 1000.0

        @classmethod
        def sleep(cls, seconds):
            cls.now += seconds

        @classmethod
        def time(cls):
            return cls.now

        strftime = staticmethod(__import__("time").strftime)

    mod.time = Clock
    return mod


def png(color, box=None):
    """A 96x54 picture: `color` all over, and a lighter block where `box` (x0, y0, x1, y1) is."""
    if Image is None:
        return b"png"
    im = Image.new("RGB", (96, 54), color)
    if box:
        im.paste((230, 230, 230), box)
    out = io.BytesIO()
    im.save(out, "PNG")
    return out.getvalue()


class Events:
    """One ordered record of what happened on the receiver, the screen and the power, across the three stand-ins."""

    def __init__(self):
        self.list = []

    def add(self, *e):
        self.list.append(e)

    def index(self, *e):
        return self.list.index(e)

    def has(self, *e):
        return e in self.list


class FakeHa:
    def __init__(self, events, works=True):
        self.events = events
        self.works = works
        self.muted = False
        self.stale = False

    def state(self):
        return {"state": "on", "muted": self.muted or self.stale, "volume": 0.4}

    def mute(self, on):
        self.events.add("mute" if on else "unmute")
        if self.works:
            self.muted = bool(on)


class FakeGate:
    def __init__(self, fake, wakes=True):
        self.fake, self.wakes = fake, wakes

    def wake(self):
        if not self.wakes:
            return False
        self.fake.events.add("wake")
        if self.fake.power != "Active":
            for word in ("prepare power on", "prepare power on"):
                self.fake.msg("com.webos.service.tvpower", "com.webos.surfacemanager", "?", {"sub processing": word}, type="return")
        self.fake.power = "Active"
        return True


class FakeSet:
    """Just enough of a TV and its helper: what the script asks and the answers the notes say come back."""

    def __init__(self, ha, events, logo_on=False, plays=True, closes_plex=True, busy="", fg=HOME, staged_record=True, live=True,
                 roster='{"plugins":[{"id":"other","uid":20000,"approve":"00"}]}', maliit=True, powered="Active"):
        self.ha, self.events = ha, events
        self.power = powered
        self.fg = fg
        self.plex_running = False
        self.plays, self.closes_plex = plays, closes_plex
        self.ok_presses = 0
        self.playing = False
        self.picker = False
        self.door = ""
        self.busy = busy
        self.logo_on, self.staged_record, self.live = logo_on, staged_record, live
        self.logo_events = "up=50.1 wall=2026-10-25T10:00:00Z power-off logo 2 launch reason=old\n"
        self.files = {}
        self.roster = roster
        self.roster_dir_existed = True
        self.roster_listed = False
        self.plugin_up = False
        self.pings = 0
        self.notes = 1
        self.plugin_log = ""
        self.lane_inner = False
        self.monitor = False
        self.capture = []
        self.gz = {}
        self.log = ""
        self.shots = {}
        self.puts = []
        self.cmds = []
        self.picture = "filmMaker"
        self.maliit = maliit
        self.mute_at_play = None
        self.t = 100.0
        self.fail_shot = False

    # ---- the capture the X4 monitor would hold
    def msg(self, sender, destination, method, payload=None, **kw):
        m = {"type": "call", "transport": "TX", "sender": sender, "destination": destination, "method": method, "payload": payload or {}}
        m.update(kw)
        if self.monitor:
            self.capture.append(m)

    def logline(self, text):
        self.log += "[ %.3f] %s\n" % (self.t, text)

    # ---- the helper's loopback API as curl prints it
    def helper(self, cmd):
        path = cmd.split("http://127.0.0.1:18731")[1].split()[0].strip("'")
        post = "-X POST" in cmd

        def answer(code, doc):
            return json.dumps(doc) + "\n%d" % code

        if path == "/v1/device":
            return answer(200, {"screen": {"class": "tv"}, "input": ["remote"]})
        if path == "/v1/plugins/rescan":
            self.roster_listed = "round8" in (self.roster or "")
            return answer(202, {"ok": True})
        if path == "/v1/plugins":
            plugins = []
            if self.roster_listed:
                plugins.append({"id": "round8", "state": "connected" if self.plugin_up else "waiting"})
            return answer(200, {"plugins": plugins, "firstParty": [{"id": "ambient"}]})
        if path == "/v1/plugins/round8/actions/ping" and post:
            if not self.plugin_up:
                return answer(404, {"error": "not on"})
            self.ping()
            return answer(202, {"op": 7})
        if path.startswith("/v1/state/"):
            name = path.split("/v1/state/")[1]
            value = self.topic(name)
            return answer(200, {"topic": name, "value": value}) if value is not None else answer(404, {"error": "no topic"})
        if path in ("/v1/settings/base.ambient", "/v1/ambient"):
            return answer(200, {"enabled": True})
        return answer(404, {"error": path})

    def topic(self, name):
        if name == "receiver":
            return {"fresh": True, "reachable": True, "mute": self.ha.muted}
        if name == "plugin.ambient.tiles":
            return {"items": [{"id": "toggle", "state": {"type": "off"}}]}
        if name in ("plugins", "plugin.weather.cards", "plugin.news.cards", "plugin.clock.cards"):
            return {"items": []}
        if name == "plugin.round8.cards" and self.plugin_up:
            return {"items": [{"id": "r8-card", "revision": 1 + self.pings}]}
        if name == "plugin.round8.tiles" and self.plugin_up:
            return {"items": [{"id": "r8-tile", "state": {"type": "on" if self.pings % 2 else "off"}}]}
        if name == "plugin.round8.notifications" and self.plugin_up:
            return {"items": [{"id": "r8-note-%d" % n} for n in range(1, self.notes + 1)]}
        return None

    def ping(self):
        self.pings += 1
        self.plugin_log += "%.3f action ping %d\n" % (self.t, self.pings)

    # ---- the shell
    def tv(self, cmd, timeout=60):
        self.cmds.append(cmd)
        self.t += 1.0
        if "http://127.0.0.1:18731" in cmd:
            return self.helper(cmd)
        if "getPowerState" in cmd:
            return '{"returnValue": true, "state": "%s"}' % self.power
        if "/power/powerOff" in cmd:
            self.events.add("powerOff")
            self.power = "Active Standby"
            for word in ("request power off", "request power off", "request active standby", "prepare power off"):
                self.msg("com.webos.service.tvpower", "com.webos.surfacemanager", "?", {"sub processing": word}, type="return")
            if self.logo_on and self.staged_record:
                self.logo_events += "up=%.1f wall=2026-10-25T20:00:00Z power-off logo 2 launch reason=remoteKey\n" % self.t
                self.logo_events += "up=%.1f wall=2026-10-25T20:00:01Z power-off answered tvpowerd\n" % (self.t + 0.6)
                self.logline("[Logo] power-off logo 2")
            self.shots["/tmp/round8/poweroff.png"] = png((10, 10, 10))
            return ""
        if "getForegroundAppInfo" in cmd:
            return json.dumps({"returnValue": True, "foregroundAppInfo": [{"appId": self.fg}]})
        if "applicationManager/launch" in cmd:
            app = json.loads(cmd.split("'")[1])["id"]
            self.fg = app
            self.events.add("launch", app)
            if app == PLEX:
                self.plex_running = True
                self.mute_at_play = self.ha.muted
            if app == HOME:
                self.events.add("home")
            return '{"returnValue": true}'
        if "applicationManager/closeByAppId" in cmd:
            app = json.loads(cmd.split("'")[1])["id"]
            self.events.add("close", app)
            if app == PLEX and self.closes_plex:
                self.plex_running, self.playing = False, False
                if self.fg == PLEX:
                    self.fg = ""          # nothing is said to be in front until Home is asked for
            return '{"returnValue": true}'
        if "applicationManager/running" in cmd:
            return json.dumps({"running": [{"id": PLEX}] if self.plex_running else [{"id": "netflix"}]})
        if "showInputPicker" in cmd:
            self.picker = True
            self.msg("com.webos.lunasend-1", "com.webos.surfacemanager", "showInputPicker")
            return '{"returnValue": true}'
        if "systemUIManager/closeByAppId" in cmd:
            if self.picker:
                self.msg("com.webos.lunasend-1", "com.webos.service.systemUIManager", "closeByAppId", {"appId": "com.webos.app.quickinputpicker"})
            self.picker = False
            return '{"returnValue": true}'
        if "getForegroundVideoWindow" in cmd:
            return json.dumps({"returnValue": True, "appId": PLEX, "fullScreen": True}) if self.playing else '{"returnValue": true}'
        if "getSystemSettings" in cmd:
            return '{"pictureMode": "%s"}' % self.picture
        if "python3 /tmp/round8/kb.py" in cmd:
            return self.keys(cmd.split("kb.py", 1)[1].split())
        if "executeOneShot" in cmd:
            path = cmd.split('"path":"')[1].split('"')[0]
            if not self.fail_shot:
                scene = self.door or ("plex" if self.playing else "home")
                self.shots[path] = png((20, 20, 30), {"quick": (10, 0, 60, 30), "drawer": (60, 0, 96, 54), "settings": (6, 4, 90, 50)}.get(self.door))
            return ""
        if cmd.startswith("wc -c < /tmp/round8/") and cmd.endswith("2>/dev/null"):
            path = cmd.split("< ")[1].split()[0]
            return "%d\n" % len(self.shots[path]) if path in self.shots else ""
        if "wc -c < /var/log/messages" in cmd:
            return "%d\n" % len(self.log)
        if "tail -c +" in cmd and "/var/log/messages" in cmd:
            n = int(cmd.split("tail -c +")[1].split()[0])
            return self.log[n - 1:]
        if cmd.startswith("printf '%s' ") and "/tmp/lumatv-layer" in cmd:
            self.door = cmd.split()[2]
            self.logline("[LumaHome] door %s" % self.door)
            return ""
        if cmd.startswith(": > /tmp/lumatv-layer"):
            self.door = ""
            return ""
        if "pgrep -fl" in cmd:
            return self.busy or "none\nnothing\n"
        if cmd.startswith("mkdir /tmp/round8.lock"):
            return "locked\n"
        if "tools/lumatv status" in cmd:
            return json.dumps({"ok": True, "live": self.live, "applied": True, "helper": {"running": True}, "compositor": {"live": False}})
        if "tools/lumatv logo status" in cmd:
            return json.dumps({"ok": True, "on": self.logo_on, "main": None})
        if "tools/lumatv logo on" in cmd:
            self.logo_on = True
            self.events.add("logo", "on")
            return '{"ok":true,"logo":"on"}'
        if "tools/lumatv logo off" in cmd:
            self.logo_on = False
            self.events.add("logo", "off")
            return '{"ok":true,"logo":"off"}'
        if "MasterBrand.qml" in cmd and "grep -c" in cmd:
            return "3\n" if self.staged_record else "0\n"
        if cmd.startswith("cat %s/logo-events" % DATA):
            return self.logo_events
        if cmd.startswith("cat /var/lib/lumatv/helper.json"):
            return ""
        if cmd.startswith("cat %s/enabled.json" % ROSTER):
            return self.roster if self.roster is not None else ""
        if cmd.startswith("test -e %s/enabled.json" % ROSTER):
            return "yes\n" if self.roster is not None else "no\n"
        if cmd.startswith("test -d %s" % ROSTER):
            return "yes\n" if self.roster_dir_existed else "no\n"
        if cmd.startswith("mkdir -p %s/round8" % ROSTER):
            self.plugin_log = ""
            return ""
        if "plugin.py --user" in cmd:
            if self.roster_listed:
                self.plugin_up = True
                self.ping_before = self.pings
            return ""
        if "/tmp/round8/plugin.log" in cmd and cmd.startswith("grep -c"):
            return "%d\n" % self.plugin_log.count(" action ping ")
        if cmd.startswith("cat /tmp/round8/plugin.log"):
            return self.plugin_log
        if "plugin.cmd" in cmd and cmd.startswith("echo"):
            word = cmd.split("'")[1] if "'" in cmd else cmd.split()[1]
            if word.startswith("note"):
                self.notes = int(word.split()[1])
            if word == "quit":
                self.plugin_up = False
            return ""
        if cmd.startswith("rm -f %s/enabled.json" % ROSTER):
            self.roster = None
            return ""
        if cmd.startswith("rm -rf %s/round8" % ROSTER):
            return ""
        if cmd.startswith("rmdir %s" % ROSTER):
            return ""
        if "x4-monitor.sh start" in cmd:
            self.monitor = True
            for n in range(3):
                self.msg("com.webos.service.audio", "com.webos.service.audiooutput", "noise%d" % n)
            return "started pid 99: 99 ls-monitor\n"
        if "x4-monitor.sh stop" in cmd:
            self.monitor = False
            return "stopped; remaining: none; lines %d bytes 1\n" % len(self.capture)
        if cmd.startswith("rm -f /tmp/round8/x4/all.jsonl"):
            self.capture = []
            return ""
        if cmd.startswith("gzip -c /tmp/round8/x4/all.jsonl"):
            name = cmd.split("> /tmp/round8/x4-")[1].split(".jsonl.gz")[0]
            self.gz[name] = gzip.compress("\n".join(json.dumps(m) for m in self.capture).encode() + b"\n")
            return ""
        if "maliit" in cmd:
            return self.maliit_cmd(cmd)
        if cmd.startswith("rm -rf /tmp/round8"):
            self.events.add("cleanup")
            return ""
        return ""

    def maliit_cmd(self, cmd):
        files = ["/usr/share/maliit/plugins/Keyboard.qml", "/usr/share/maliit/plugins/Key.qml", "/usr/share/maliit/plugins/qmldir"] if self.maliit else []
        if cmd.startswith("find / -xdev -ipath"):
            return "\n".join(files) + "\n"
        if "tar cf -" in cmd:
            return "2048\n"
        if "tar czf /tmp/round8/maliit.tgz" in cmd:
            return ""
        if "grep -a -o" in cmd:
            return "== /usr/lib/libplugin-global.so\nqrc:/Keyboard.qml\n"
        if cmd.startswith("find / -xdev -iname"):
            return "\n".join(files) + "\n"
        return ""

    def keys(self, tokens):
        if self.fg == PLEX:
            for t in tokens:
                if t == "OK":
                    self.ok_presses += 1
            if self.plays and self.ok_presses >= 2:
                self.playing = True
                self.msg(PLEX, "com.webos.service.videooutput", "setDisplayWindow", {"context": "x"}, methodCategory="/display")
            return ""
        if tokens[:4] == ["UP", "s1", "OK", "s1"] and self.plugin_up:
            self.lane_inner = True
            return ""
        if tokens == ["OK", "s1.5"] and self.lane_inner:
            self.ping()
            return ""
        if tokens[:1] == ["BACK"]:
            self.lane_inner = False
        return ""

    def put(self, local, remote):
        self.puts.append((os.path.basename(local), remote))
        if remote.endswith("enabled.json"):
            with open(local) as f:
                self.roster = f.read()
        if remote.endswith("/plugin.json"):
            self.roster_manifest = remote

    def get(self, remote, local):
        if remote in self.shots:
            data = self.shots[remote]
        elif remote.endswith(".jsonl.gz"):
            data = self.gz[remote.rsplit("x4-", 1)[1][:-len(".jsonl.gz")]]
        elif remote.endswith("maliit.tgz"):
            buf = io.BytesIO()
            with tarfile.open(fileobj=buf, mode="w:gz") as t:
                for name in ("usr/share/maliit/plugins/Keyboard.qml", "usr/share/maliit/plugins/Key.qml", "usr/share/maliit/plugins/qmldir"):
                    body = b"import QtQuick 2.0\n"
                    info = tarfile.TarInfo(name)
                    info.size = len(body)
                    t.addfile(info, io.BytesIO(body))
            data = buf.getvalue()
        elif remote.endswith("maliit.md5"):
            data = b"abc  /usr/share/maliit/plugins/Keyboard.qml\n"
        else:
            raise IOError("no such file on the set: " + remote)
        with open(local, "wb") as f:
            f.write(data)


class Round8(unittest.TestCase):
    def setUp(self):
        self.mod = load()
        self.tmp = tempfile.TemporaryDirectory()
        self.out = os.path.join(self.tmp.name, "out")

    def tearDown(self):
        self.tmp.cleanup()

    def rig(self, ha_works=True, **kw):
        events = Events()
        ha = FakeHa(events, ha_works)
        fake = FakeSet(ha, events, **kw)
        return fake, ha, events

    def go(self, fake, ha, *args):
        parser_args = ["--go", "--out", self.out] + list(args)
        self.rc = self.mod.main(parser_args, tv=fake, gate_module=FakeGate(fake), ha=ha)
        with open(os.path.join(self.out, "summary.txt")) as f:
            self.lines = f.read().splitlines()
        return self.rc

    def fails(self):
        return [l for l in self.lines if l.startswith("FAIL")]

    def passed(self, text):
        return any(l.startswith("PASS") and text in l for l in self.lines)

    # ---- the whole slot -----------------------------------------------------------------------------------------------------------------
    def test_a_slot_that_goes_to_plan_passes_every_step_and_leaves_the_set_as_found(self):
        fake, ha, ev = self.rig(roster='{"plugins":[{"id":"other","uid":20000,"approve":"00"}]}')
        before_roster = fake.roster
        rc = self.go(fake, ha, "--approve-test-plugin")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertEqual(rc, 0)
        for text in ("receiver muted", "test plugin: the helper switched it on", "lane: the plugin's card", "lane: OK on the plugin's chip", "03-quick: the screen shows it",
                     "quick: the plugin was told", "05-settings-ambient: the screen shows it", "06-drawer: the screen shows it",
                     "drawer: the helper holds both notes", "test plugin: switched off, roster as it was", "maliit: the QML tree is copied",
                     "picker: showInputPicker answered", "picker: closeByAppId answered", "plex: PlxNative reaches the foreground",
                     "plex: playback happened", "plex: playback closed and Home back", "standby: the set reaches Active Standby",
                     "standby: the power rounds reached", "wake: the set is Active again", "logo: the power-off window ran",
                     "logo: the window answered tvpowerd", "end: Home in front", "end: PlxNative is not running", "end: the receiver is put back",
                     "end: the set is in Active Standby"):
            self.assertTrue(self.passed(text), text + "\n" + "\n".join(self.lines))
        # what was found is what is left
        self.assertEqual(fake.roster, before_roster)
        self.assertEqual(fake.door, "")
        self.assertFalse(fake.logo_on)
        self.assertFalse(fake.monitor)
        self.assertFalse(fake.plugin_up)
        self.assertFalse(ha.muted)
        self.assertEqual(fake.power, "Active Standby")
        self.assertTrue(ev.has("cleanup"))
        # the order the brief fixes: muted before anything plays; playback closed and Home back before the receiver is put back; standby last
        self.assertLess(ev.index("mute"), ev.index("launch", PLEX))
        self.assertTrue(fake.mute_at_play)
        self.assertLess(ev.list.index(("close", PLEX)), ev.index("unmute"))
        closing = [i for i, e in enumerate(ev.list) if e == ("close", PLEX)][0]
        home_after = [i for i, e in enumerate(ev.list) if e == ("home",) and i > closing]
        self.assertTrue(home_after, ev.list)
        self.assertLess(home_after[0], ev.index("unmute"))
        self.assertEqual(ev.list[-1], ("powerOff",))
        self.assertLess(ev.index("unmute"), len(ev.list) - 1)
        # the evidence
        d = self.out
        for rel in ("summary.txt", "summary-final.txt", "plugin/00-home.png", "plugin/01-lane.png", "plugin/03-quick.png", "plugin/05-settings-ambient.png",
                    "plugin/06-drawer.png", "plugin/plugin.log", "plugin/enabled.json.before", "x4/picker/census.md", "x4/plex/census.md", "x4/standby/census.md",
                    "x4/plex/all.jsonl.gz", "x4/plex/facts.json", "x4/standby/power-phases.json", "maliit/qml-files.txt", "maliit/md5sum.txt",
                    "maliit/tree/usr/share/maliit/plugins/Keyboard.qml", "logo/logo-events.new.txt", "logo/log-logo-lines.txt"):
            self.assertTrue(os.path.exists(os.path.join(d, rel)), rel)
        self.assertFalse(os.path.exists(os.path.join(d, "x4", "plex", "all.jsonl")), "the plain copy is not kept")
        phases = json.load(open(os.path.join(d, "x4", "standby", "power-phases.json")))
        self.assertEqual(phases, ["request power off", "request active standby", "prepare power off", "prepare power on"])

    def test_without_the_approval_no_plugin_is_switched_on_and_the_roster_is_never_written(self):
        fake, ha, ev = self.rig()
        before = fake.roster
        self.go(fake, ha, "--skip", "x4,logo,maliit")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertTrue(any(l.startswith("SKIP plugin: the test plugin") for l in self.lines))
        self.assertFalse(any(remote.endswith("enabled.json") or "plugins/round8" in remote for _, remote in fake.puts), fake.puts)
        self.assertFalse(any("plugin.py --user" in c for c in fake.cmds))
        self.assertEqual(fake.roster, before)
        self.assertTrue(self.passed("03-quick: the screen shows it"))

    def test_every_step_can_be_skipped_and_the_rest_still_ends_cleanly(self):
        fake, ha, ev = self.rig()
        self.go(fake, ha, "--skip", "plugin,x4,maliit,logo")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertEqual([l for l in self.lines if l.startswith("SKIP")], ["SKIP plugin: --skip", "SKIP maliit: --skip", "SKIP x4: --skip", "SKIP logo: --skip"])
        self.assertFalse(any("kb.py" in c for c in fake.cmds))
        self.assertEqual(fake.power, "Active Standby")
        self.assertFalse(ha.muted)
        for step, marker in (("plugin", "/tmp/lumatv-layer"), ("maliit", "maliit"), ("x4", "x4-monitor.sh start"), ("logo", "logo status")):
            fake2, ha2, _ = self.rig()
            out2 = os.path.join(self.tmp.name, "only-" + step)
            others = ",".join(s for s in ("plugin", "x4", "maliit", "logo") if s != step)
            self.mod.main(["--go", "--out", out2, "--skip", others, "--approve-test-plugin"], tv=fake2, gate_module=FakeGate(fake2), ha=ha2)
            self.assertTrue(any(marker in c for c in fake2.cmds), step)
            for other, other_marker in (("plugin", "/tmp/lumatv-layer"), ("x4", "x4-monitor.sh start"), ("logo", "logo status")):
                if other != step:
                    self.assertFalse(any(other_marker in c for c in fake2.cmds), "%s ran with only %s" % (other, step))

    def test_a_skipped_standby_leaves_the_set_on(self):
        fake, ha, ev = self.rig()
        self.go(fake, ha, "--skip", "plugin,x4,maliit,logo", "--no-standby")
        self.assertEqual(fake.power, "Active")
        self.assertFalse(ev.has("powerOff"))

    def test_the_plan_names_the_steps_and_touches_nothing(self):
        fake, ha, ev = self.rig()
        steps = self.mod.plan()
        names = [s[0] for s in steps]
        self.assertEqual(names, ["preflight", "mute", "plugin", "maliit", "x4", "logo", "close", "unmute", "summary", "standby"])
        self.assertTrue(all(always for step, _, always in steps if step in ("close", "unmute", "summary", "standby")))
        self.assertNotIn("x4", [s[0] for s in self.mod.plan(skip={"x4"})])
        self.assertNotIn("standby", [s[0] for s in self.mod.plan(standby=False)])
        self.assertEqual(self.mod.main([], tv=fake, gate_module=FakeGate(fake), ha=ha), 0)
        self.assertEqual(fake.cmds, [])
        with self.assertRaises(SystemExit):
            self.mod.main(["--skip", "plugin,bogus"])

    # ---- the set must be free, and awake, with Home in front ------------------------------------------------------------------------------
    def test_a_set_that_is_not_free_is_not_touched(self):
        fake, ha, ev = self.rig(busy="1234 ls-monitor -j -e\n")
        with self.assertRaises(SystemExit) as caught:
            self.go(fake, ha)
        self.assertIn("not free", str(caught.exception))
        self.assertEqual(ev.list, [("wake",)])
        self.assertFalse(any("kb.py" in c or "launch" in c for c in fake.cmds))

    def test_a_set_with_something_else_in_front_is_not_started_over(self):
        fake, ha, ev = self.rig(fg="netflix")
        with self.assertRaises(SystemExit) as caught:
            self.go(fake, ha)
        self.assertIn("Home is not in front", str(caught.exception))
        self.assertFalse(ev.has("mute"))

    def test_a_set_that_was_asleep_is_put_back_to_sleep_when_the_run_is_refused(self):
        fake, ha, ev = self.rig(powered="Active Standby", busy="9 [slot.sh]\n")
        with self.assertRaises(SystemExit):
            self.go(fake, ha)
        self.assertEqual(ev.list[-1], ("powerOff",))

    def test_a_set_that_does_not_answer_is_left_alone(self):
        fake, ha, ev = self.rig()
        with self.assertRaises(SystemExit) as caught:
            self.mod.main(["--go", "--out", self.out], tv=fake, gate_module=FakeGate(fake, wakes=False), ha=ha)
        self.assertIn("does not answer", str(caught.exception))
        self.assertEqual(ev.list, [])

    # ---- the receiver ----------------------------------------------------------------------------------------------------------------------
    def test_a_receiver_that_cannot_be_shown_muted_gets_nothing_played_and_no_standby_cycle(self):
        fake, ha, ev = self.rig(ha_works=False)
        rc = self.go(fake, ha, "--skip", "plugin,maliit")
        self.assertEqual(rc, 1)
        self.assertTrue(any(l.startswith("FAIL receiver muted") for l in self.lines), "\n".join(self.lines))
        self.assertFalse(ev.has("launch", PLEX))
        self.assertFalse(any("showInputPicker" in c or "x4-monitor.sh start" in c for c in fake.cmds))
        self.assertFalse(ev.has("powerOff") and ev.index("powerOff") < len(ev.list) - 1, "no standby cycle in the middle")

    def test_a_receiver_that_was_muted_already_is_left_muted(self):
        fake, ha, ev = self.rig()
        ha.muted = True
        self.go(fake, ha, "--skip", "plugin,maliit,logo")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertTrue(ha.muted)
        self.assertFalse(ev.has("unmute"))

    def test_a_home_assistant_that_still_says_muted_does_not_stand_in_for_the_helpers_reading(self):
        fake, ha, ev = self.rig()
        ha.stale = True          # the entity says muted; the receiver (the helper's reading) is not
        self.go(fake, ha, "--skip", "plugin,maliit,logo")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertTrue(ev.has("mute"), "muted although the entity said it was")
        self.assertTrue(fake.mute_at_play)
        self.assertTrue(ev.has("unmute"))
        self.assertFalse(ha.muted)

    def test_a_receiver_that_the_wake_unmuted_is_muted_again_before_playback(self):
        fake, ha, ev = self.rig()
        original = fake.tv

        def tv(cmd, timeout=60):
            out = original(cmd, timeout)
            if "showInputPicker" in cmd:
                ha.muted = False       # the receiver lost its mute between the picker and PlxNative (it follows the set over CEC)
            return out

        fake.tv = tv
        self.go(fake, ha, "--skip", "plugin,maliit")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertGreaterEqual(ev.list.count(("mute",)), 2)
        self.assertEqual(fake.mute_at_play, True)

    # ---- PlxNative ---------------------------------------------------------------------------------------------------------------------------
    def test_playback_that_never_starts_is_a_failure_that_names_the_keys_and_the_set_is_still_put_back(self):
        fake, ha, ev = self.rig(plays=False)
        rc = self.go(fake, ha, "--skip", "plugin,maliit,logo")
        self.assertEqual(rc, 1)
        bad = [l for l in self.fails() if "plex: playback happened" in l]
        self.assertEqual(len(bad), 1, "\n".join(self.lines))
        self.assertIn("--plex-keys", bad[0])
        self.assertEqual(len(self.fails()), 1, "\n".join(self.lines))
        self.assertFalse(fake.plex_running)
        self.assertFalse(ha.muted)
        self.assertEqual(fake.power, "Active Standby")

    def test_plex_keys_are_the_ones_asked_for(self):
        fake, ha, ev = self.rig(plays=False)
        self.go(fake, ha, "--skip", "plugin,maliit,logo", "--plex-keys", "DOWN|OK s1")
        sent = [c.split("kb.py", 1)[1].strip() for c in fake.cmds if "kb.py" in c]
        self.assertEqual(sent, ["DOWN", "OK s1"])

    def test_a_plex_that_will_not_close_keeps_the_receiver_muted_and_says_so(self):
        fake, ha, ev = self.rig(closes_plex=False)
        rc = self.go(fake, ha, "--skip", "plugin,maliit,logo")
        self.assertEqual(rc, 1)
        self.assertFalse(ev.has("unmute"), "PlxNative would be heard")
        self.assertTrue(ha.muted)
        self.assertTrue(any(l.startswith("FAIL end: PlxNative is not running") for l in self.lines), "\n".join(self.lines))

    # ---- the plugin step ---------------------------------------------------------------------------------------------------------------------
    def test_a_step_that_dies_halfway_still_takes_the_test_plugin_off_and_puts_the_roster_back(self):
        fake, ha, ev = self.rig()
        before = fake.roster
        original = fake.tv

        def tv(cmd, timeout=60):
            if "/tmp/lumatv-layer" in cmd and "quick" in cmd:
                raise IOError("ssh dropped")
            return original(cmd, timeout)

        fake.tv = tv
        rc = self.go(fake, ha, "--skip", "x4,maliit,logo", "--approve-test-plugin")
        self.assertEqual(rc, 1)
        self.assertTrue(any(l.startswith("FAIL plugin: ran to its end") for l in self.lines), "\n".join(self.lines))
        self.assertEqual(fake.roster, before)
        self.assertFalse(fake.plugin_up)
        self.assertTrue(os.path.exists(os.path.join(self.out, "plugin", "traceback.txt")))

    def test_a_roster_that_did_not_exist_is_not_left_behind(self):
        fake, ha, ev = self.rig(roster=None)
        fake.roster_dir_existed = False
        self.go(fake, ha, "--skip", "x4,maliit,logo", "--approve-test-plugin")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertIsNone(fake.roster)
        self.assertTrue(any(c.startswith("rmdir %s" % ROSTER) for c in fake.cmds))

    def test_a_helper_that_refuses_the_plugin_is_a_failure_and_nothing_is_left(self):
        fake, ha, ev = self.rig()
        original = fake.helper

        def helper(cmd):
            if "/v1/plugins" in cmd and "rescan" not in cmd and "actions" not in cmd:
                return json.dumps({"plugins": [{"id": "round8", "state": "inert", "reason": "its manifest is not the one that was approved"}]}) + "\n200"
            return original(cmd)

        fake.helper = helper
        rc = self.go(fake, ha, "--skip", "x4,maliit,logo", "--approve-test-plugin")
        self.assertEqual(rc, 1)
        self.assertTrue(any(l.startswith("FAIL test plugin: the helper switched it on") for l in self.lines), "\n".join(self.lines))
        self.assertFalse(any("plugin.py --user" in c for c in fake.cmds), "an inert plugin is not started")

    def test_a_home_that_is_not_lumatvs_is_said_before_any_surface_is_asked_for(self):
        fake, ha, ev = self.rig(live=False)
        rc = self.go(fake, ha, "--skip", "x4,maliit,logo", "--approve-test-plugin")
        self.assertEqual(rc, 1)
        self.assertTrue(any(l.startswith("FAIL plugin: LumaTV's Home is the live one") for l in self.lines))
        self.assertFalse(any("/tmp/lumatv-layer" in c for c in fake.cmds))

    def test_a_surface_that_does_not_draw_is_a_failure(self):
        if Image is None:
            self.skipTest("Pillow")
        fake, ha, ev = self.rig()
        fake.door_scene = None
        original = fake.tv

        def tv(cmd, timeout=60):
            if cmd.startswith("printf '%s' ") and "/tmp/lumatv-layer" in cmd:
                self.mod.time.now += 0     # the door is written but the layer does not show: the scene stays Home's
                return ""
            return original(cmd, timeout)

        fake.tv = tv
        rc = self.go(fake, ha, "--skip", "x4,maliit,logo")
        self.assertEqual(rc, 1)
        self.assertTrue(any(l.startswith("FAIL 03-quick: the screen shows it") for l in self.lines), "\n".join(self.lines))

    def test_a_log_line_that_the_gate_would_fail_fails_the_surface(self):
        fake, ha, ev = self.rig()
        original = fake.tv

        def tv(cmd, timeout=60):
            if "/tmp/lumatv-layer" in cmd and "drawer" in cmd:
                fake.logline("file:///x/Drawer.qml:12: TypeError: Cannot read property 'title' of undefined")
            return original(cmd, timeout)

        fake.tv = tv
        rc = self.go(fake, ha, "--skip", "x4,maliit,logo")
        self.assertEqual(rc, 1)
        self.assertTrue(any(l.startswith("FAIL drawer: no failure line in the log") and "TypeError" in l for l in self.lines), "\n".join(self.lines))
        self.assertTrue(os.path.exists(os.path.join(self.out, "plugin", "drawer.log")))

    # ---- maliit ---------------------------------------------------------------------------------------------------------------------------------
    def test_keys_compiled_into_the_plugin_are_reported_not_failed(self):
        fake, ha, ev = self.rig(maliit=False)
        self.go(fake, ha, "--skip", "plugin,x4,logo")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertTrue(any(l.startswith("INFO maliit: QML on disk") and "compiled in" in l for l in self.lines), "\n".join(self.lines))
        self.assertFalse(os.path.exists(os.path.join(self.out, "maliit", "tree")))

    def test_the_maliit_step_only_reads(self):
        fake, ha, ev = self.rig()
        self.go(fake, ha, "--skip", "plugin,x4,logo")
        writers = ("rm ", "mv ", "cp ", "chmod", "sed -i", "> /usr", "> /var", "> /etc", "mount", "tee ")
        for c in fake.cmds:
            if "maliit" in c and "x4-monitor" not in c:
                for w in writers:
                    self.assertNotIn(w, c.replace("> /tmp/round8/", ""), c)

    # ---- the logo record ------------------------------------------------------------------------------------------------------------------------
    def test_the_logo_switch_is_turned_on_for_the_record_and_back_off_and_said(self):
        fake, ha, ev = self.rig(logo_on=False)
        self.go(fake, ha, "--skip", "plugin,x4,maliit")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertEqual([e for e in ev.list if e[0] == "logo"], [("logo", "on"), ("logo", "off")])
        self.assertLess(ev.index("logo", "on"), ev.index("powerOff"), "the switch is on before the power-off it records")
        self.assertTrue(self.passed("logo: the switch turned on for the record"))

    def test_a_logo_that_was_on_stays_on(self):
        fake, ha, ev = self.rig(logo_on=True)
        self.go(fake, ha, "--skip", "plugin,x4,maliit")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertEqual([e for e in ev.list if e[0] == "logo"], [])
        self.assertTrue(fake.logo_on)

    def test_keep_logo_off_refuses_to_turn_it_on(self):
        fake, ha, ev = self.rig(logo_on=False)
        self.go(fake, ha, "--skip", "plugin,x4,maliit", "--keep-logo-off")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        self.assertTrue(any(l.startswith("SKIP logo: the power-off record") for l in self.lines))
        self.assertFalse(ev.has("logo", "on"))
        self.assertFalse(ev.has("powerOff") and ev.index("powerOff") < len(ev.list) - 1)

    def test_a_window_that_leaves_no_record_is_a_failure_that_says_where_to_look(self):
        fake, ha, ev = self.rig(logo_on=True, staged_record=False)
        rc = self.go(fake, ha, "--skip", "plugin,x4,maliit")
        self.assertEqual(rc, 1)
        bad = [l for l in self.fails() if "logo: the staged power-off window" in l]
        self.assertEqual(len(bad), 1, "\n".join(self.lines))
        self.assertIn("lumatv apply", bad[0])

    def test_with_the_x4_step_the_logo_record_reads_the_same_power_off_and_there_is_one_cycle(self):
        fake, ha, ev = self.rig(logo_on=True)
        self.go(fake, ha, "--skip", "plugin,maliit")
        self.assertEqual(self.fails(), [], "\n".join(self.lines))
        cycles = [e for e in ev.list if e == ("powerOff",)]
        self.assertEqual(len(cycles), 2, "one standby cycle for both steps, then the one that ends the run: %s" % ev.list)
        new = open(os.path.join(self.out, "logo", "logo-events.new.txt")).read()
        self.assertIn("power-off answered tvpowerd", new)
        self.assertNotIn("reason=old", new)

    # ---- the readers --------------------------------------------------------------------------------------------------------------------------------
    def test_the_consent_hash_is_the_one_the_plugin_api_crate_computes(self):
        with open(os.path.join(ROUND8, "plugin", "plugin.json")) as f:
            manifest = json.load(f)
        self.assertEqual(self.mod.consent_document(manifest), RUST_CONSENT)
        self.assertEqual(self.mod.approval(manifest), RUST_APPROVAL)
        self.assertEqual(hashlib.sha256(RUST_CONSENT.encode()).hexdigest(), RUST_APPROVAL)
        again = dict(manifest, permissions=["background", "notify", "tiles", "cards"])
        self.assertEqual(self.mod.approval(again), RUST_APPROVAL, "the order the author wrote is not what is approved")
        more = dict(manifest, permissions=manifest["permissions"] + ["settings"])
        self.assertNotEqual(self.mod.approval(more), RUST_APPROVAL)
        with self.assertRaises(ValueError):
            self.mod.consent_document(dict(manifest, permissions=["net.internet"]))

    def test_the_manifest_asks_for_nothing_the_tv_does_not_serve(self):
        with open(os.path.join(ROUND8, "plugin", "plugin.json")) as f:
            manifest = json.load(f)
        served = {"settings", "cards", "tiles", "notify", "background", "process.spawn", "ui.view"}     # helper/app/src/plugins/roster.rs unserved()
        self.assertLessEqual(set(manifest["permissions"]), served)
        self.assertEqual(manifest["requires"], [])
        self.assertEqual(manifest["id"], "round8")

    def test_the_roster_entry_goes_under_a_free_user_id_and_keeps_the_others(self):
        text, uid = self.mod.merged_roster('{"plugins":[{"id":"a","uid":20000,"approve":"x"},{"id":"b","uid":20001,"approve":"y"}]}', {"id": "round8", "approve": "z"})
        self.assertEqual(uid, 20002)
        doc = json.loads(text)
        self.assertEqual([p["id"] for p in doc["plugins"]], ["a", "b", "round8"])
        self.assertEqual(doc["plugins"][0], {"id": "a", "uid": 20000, "approve": "x"})
        text, uid = self.mod.merged_roster("", {"id": "round8", "approve": "z"}, first_uid=21000)
        self.assertEqual(uid, 21000)
        with self.assertRaises(ValueError):
            self.mod.merged_roster(text, {"id": "round8", "approve": "z"})
        full = json.dumps({"plugins": [{"id": "p%d" % i, "uid": 20000 + i, "approve": "x"} for i in range(64)]})
        with self.assertRaises(ValueError):
            self.mod.merged_roster(full, {"id": "round8", "approve": "z"})

    def test_the_power_rounds_of_the_recorded_standby_are_read_in_order(self):
        path = os.path.join(ROOT, "docs", "research", "arch", "x4", "pass2.jsonl.gz")
        with gzip.open(path, "rt", errors="replace") as f:
            phases = self.mod.power_phases(self.mod.load_messages(f.read()))
        for want in ("request power off", "request power off logo", "request active standby", "prepare power off", "prepare power on"):
            self.assertIn(want, phases)
        self.assertLess(phases.index("request power off"), phases.index("prepare power off"))
        self.assertLess(phases.index("prepare power off"), phases.index("prepare power on"))

    def test_plex_facts_count_its_placements_and_the_compositors_own_calls(self):
        msgs = [{"type": "call", "transport": "TX", "sender": PLEX, "destination": "com.webos.service.videooutput", "method": "setDisplayWindow", "payload": {}},
                {"type": "call", "transport": "TX", "sender": PLEX, "destination": "com.webos.applicationManager", "method": "registerApp", "payload": {}},
                {"type": "call", "transport": "TX", "sender": "com.webos.surfacemanager", "destination": "com.webos.service.videooutput", "method": "x", "payload": {}},
                {"type": "return", "transport": "TX", "sender": PLEX, "method": "setDisplayWindow", "payload": {}}]
        f = self.mod.plex_facts(msgs)
        self.assertEqual(f["placements"], 1)
        self.assertEqual(f["calls"], {"com.webos.service.videooutput/setDisplayWindow": 1, "com.webos.applicationManager/registerApp": 1})
        self.assertEqual(f["compositor_calls"], {"com.webos.service.videooutput/x": 1})
        self.assertFalse(f["window"])
        msgs.append({"type": "call", "transport": "TX", "method": "getForegroundVideoWindow", "payload": {"appId": PLEX}})
        self.assertTrue(self.mod.plex_facts(msgs)["window"])

    def test_logo_events_are_read_and_only_the_new_lines_count(self):
        before = "up=1.0 wall=a power-off logo 2 launch reason=old\nup=1.5 wall=b power-off answered tvpowerd\n"
        after = before + "up=90.0 wall=c power-off logo 2 launch reason=remoteKey\nup=90.55 wall=d power-off answered tvpowerd\n"
        fresh = self.mod.new_lines(before, after)
        self.assertEqual(len(fresh), 2)
        launch, answered, gap = self.mod.power_off_record(self.mod.parse_logo_events("\n".join(fresh)))
        self.assertEqual((launch["up"], answered["up"], gap), (90.0, 90.55, 0.55))
        launch, answered, gap = self.mod.power_off_record(self.mod.parse_logo_events("up=- wall=c power-off logo 1 launch type=x\nup=- wall=d power-off answered tvpowerd"))
        self.assertIsNotNone(launch)
        self.assertIsNone(gap)
        self.assertEqual(self.mod.power_off_record([]), (None, None, None))

    def test_the_busy_list_names_other_slots_and_not_the_packages_own_services(self):
        self.assertEqual(self.mod.free_problems("none\nnothing\n"), [])
        self.assertEqual(self.mod.free_problems("12 ls-monitor -j -e\n/tmp/ambient-slot-x/run.lock\n"), ["12 ls-monitor -j -e", "/tmp/ambient-slot-x/run.lock"])
        for own in ("lumatv-helper", "lumatv-capture"):
            self.assertNotIn(own, self.mod.BUSY)

    @unittest.skipIf(Image is None, "Pillow")
    def test_picture_comparisons(self):
        a, b, flat = (os.path.join(self.tmp.name, n) for n in ("a.png", "b.png", "flat.png"))
        open(a, "wb").write(png((20, 20, 30)))
        open(b, "wb").write(png((20, 20, 30), (0, 0, 48, 54)))
        open(flat, "wb").write(png((0, 0, 0)))
        self.assertAlmostEqual(self.mod.png_change(a, b), 0.5, places=2)
        self.assertEqual(self.mod.png_change(a, a), 0.0)
        self.assertTrue(self.mod.png_flat(flat))
        self.assertFalse(self.mod.png_flat(b))
        self.assertIsNone(self.mod.png_change(a, os.path.join(self.tmp.name, "missing.png")))


# ---- the plugin's own conversation, for real, against a stand-in broker -------------------------------------------------------------------------

def frame(message):
    body = json.dumps(message).encode()
    return struct.pack("<I", len(body)) + body


class Broker(threading.Thread):
    """What the helper's broker says to a plugin (hello -> welcome, register -> registered) on a unix socket, and what the plugin sends."""

    def __init__(self, path):
        super().__init__(daemon=True)
        self.path = path
        self.server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.server.bind(path)
        self.server.listen(1)
        self.received = []
        self.conn = None
        self.buf = b""
        self.cond = threading.Condition()

    def run(self):
        self.conn, _ = self.server.accept()
        while True:
            data = self.conn.recv(65536)
            if not data:
                return
            self.buf += data
            while len(self.buf) >= 4:
                n = struct.unpack("<I", self.buf[:4])[0]
                if len(self.buf) < 4 + n:
                    break
                message = json.loads(self.buf[4:4 + n])
                self.buf = self.buf[4 + n:]
                with self.cond:
                    self.received.append(message)
                    self.cond.notify_all()
                if message["t"] == "hello":
                    self.conn.sendall(frame({"t": "welcome", "api": 1, "id": "round8", "class": "S", "features": ["cards:1"], "permissions": [], "limits": {}}))
                elif message["t"] == "register":
                    self.conn.sendall(frame({"t": "registered", "owner": "round8"}))

    def wait_for(self, predicate, seconds=10):
        with self.cond:
            return self.cond.wait_for(lambda: any(predicate(m) for m in self.received), seconds)


class PluginWire(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        spec = importlib.util.spec_from_file_location("round8_plugin", os.path.join(ROUND8, "plugin", "plugin.py"))
        self.plugin = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.plugin)
        self.sock = os.path.join(self.tmp.name, "b.sock")
        self.ctl = os.path.join(self.tmp.name, "plugin.cmd")
        self.log = os.path.join(self.tmp.name, "plugin.log")
        open(self.ctl, "w").close()
        self.broker = Broker(self.sock)
        self.broker.start()
        self.thread = threading.Thread(target=lambda: self.plugin.main(["--socket", self.sock, "--ctl", self.ctl, "--log", self.log, "--wait", "5"]), daemon=True)
        self.thread.start()

    def tearDown(self):
        with open(self.ctl, "a") as f:
            f.write("quit\n")
        self.thread.join(10)
        self.tmp.cleanup()

    def test_it_registers_puts_a_card_and_a_note_answers_a_ping_and_says_bye(self):
        b = self.broker
        self.assertTrue(b.wait_for(lambda m: m["t"] == "notify.post"), b.received)
        kinds = [m["t"] for m in b.received]
        self.assertEqual(kinds[:2], ["hello", "register"])
        register = b.received[1]
        self.assertEqual([t["id"] for t in register["tiles"]], ["r8-tile"])
        self.assertEqual([c["id"] for c in register["channels"]], ["r8"])
        self.assertEqual(register["tiles"][0]["action"], "ping")
        self.assertIn("ping", [a["key"] for a in register["actions"]])
        self.assertIn("card.put", kinds)
        card = next(m for m in b.received if m["t"] == "card.put")
        self.assertEqual(card["open"], {"view": "main"})
        self.assertIn("main", [v["id"] for v in register["views"]])
        note = next(m for m in b.received if m["t"] == "notify.post")
        self.assertEqual((note["id"], note["channel"]), ("r8-note-1", "r8"))
        # an action the helper hands it
        b.conn.sendall(frame({"t": "action.invoke", "op": 5, "action": "ping"}))
        self.assertTrue(b.wait_for(lambda m: m["t"] == "action.result" and m["op"] == 5 and m["outcome"] == {"type": "ok"}), b.received)
        self.assertTrue(b.wait_for(lambda m: m["t"] == "tile.put" and m["state"] == {"type": "on"}), b.received)
        self.assertTrue(b.wait_for(lambda m: m["t"] == "card.put" and {"label": "Pings", "value": "1"} in m["content"]["facts"]), b.received)
        # the harness's commands
        with open(self.ctl, "a") as f:
            f.write("note 2\nend 2\nquit\n")
        self.assertTrue(b.wait_for(lambda m: m["t"] == "notify.post" and m["id"] == "r8-note-2"), b.received)
        self.assertTrue(b.wait_for(lambda m: m["t"] == "notify.end" and m["id"] == "r8-note-2"), b.received)
        self.assertTrue(b.wait_for(lambda m: m["t"] == "bye"), b.received)
        self.thread.join(10)
        self.assertFalse(self.thread.is_alive())
        log = open(self.log).read()
        self.assertIn("action ping 1", log)
        self.assertIn("bye", log)

    def test_a_shutdown_from_the_broker_ends_it_with_a_bye(self):
        b = self.broker
        self.assertTrue(b.wait_for(lambda m: m["t"] == "notify.post"))
        b.conn.sendall(frame({"t": "shutdown", "reason": "disabled", "grace_ms": 2000}))
        self.assertTrue(b.wait_for(lambda m: m["t"] == "bye"), b.received)

    def test_every_message_it_sends_is_one_of_the_plugin_apis_and_within_a_frame(self):
        for m in (self.plugin.REGISTER, self.plugin.card(3), self.plugin.tile(True), self.plugin.note(9)):
            data = self.plugin.encode(m)
            self.assertLessEqual(len(data) - 4, self.plugin.MAX_FRAME)
            self.assertEqual(self.plugin.Reader().feed(data), [m])
        with self.assertRaises(ValueError):
            self.plugin.encode({"t": "x", "pad": "y" * 70000})
        pieces = self.plugin.encode(self.plugin.note(1))
        reader = self.plugin.Reader()
        self.assertEqual(reader.feed(pieces[:3]), [])
        self.assertEqual(reader.feed(pieces[3:10]), [])
        self.assertEqual(len(reader.feed(pieces[10:])), 1)


class Wiring(unittest.TestCase):
    def test_the_script_is_a_program_and_the_plugin_dir_holds_what_it_uploads(self):
        for rel in ("tv_round8.py", "plugin/plugin.py", "plugin/plugin.json"):
            self.assertTrue(os.path.exists(os.path.join(ROUND8, rel)), rel)
        self.assertTrue(os.access(os.path.join(ROUND8, "tv_round8.py"), os.X_OK))
        p = subprocess.run([sys.executable, os.path.join(ROUND8, "tv_round8.py"), "--plan"], capture_output=True, text=True, timeout=60)
        self.assertEqual(p.returncode, 0, p.stderr)
        self.assertIn("preflight", p.stdout)

    def test_the_tools_it_uses_are_where_it_looks(self):
        for rel in ("docs/research/arch/x4/x4-monitor.sh", "docs/research/arch/x4/x4-analyse.py", "docs/research/perf/kb.py", "docs/research/perf/gate.py",
                    "compositor/tools/tv_gate.py", "docs/research/arch/tv.py", "brand.json"):
            self.assertTrue(os.path.exists(os.path.join(ROOT, rel)), rel)
        with open(os.path.join(ROOT, "docs/research/arch/x4/x4-monitor.sh")) as f:
            self.assertIn("D=/tmp/x4", f.read(), "the script rewrites this line to keep its capture out of /tmp/x4")


if __name__ == "__main__":
    unittest.main()
