"""Run with: cd tests && python3 -m unittest test_saver_own   (needs Xvfb and PyQt5; skipped without them)

LumaTV's saver as the only one the TV shows (docs/research/saver-own-result.md): while Home is in front the saver is on tvpower's screen
saver list (`registerScreenSaverRequest`, on `/power2` and `/power`), and a question from tvpower ("may LG's saver start?") is answered
no, so LG's Flutter saver is never started, the power state stays Active, and the saver draws. The offline host stands in for tvpower: it
records the calls (LS.calls) and the test makes the subscription answer by calling the Service's own signal.

What must hold: nothing is voted unless Home is in front and the saver is wanted (style not off, `own` not false); a question while it
cannot draw is answered yes; a key (and its release), a volume key (which goes on to LG), the pointer, the TV going off, Home leaving and
the owner's switch each end it, without asking tvpower for anything (its state never left Active); a subscription tvpower refuses is asked
for again for as long as it is wanted, and every time the TV comes on; a saver that LG started (power state `Screen Saver`) is what it
was; the video-plane player taking a key in its own window ends the saver and does not send it to the GPU's art; and nothing in the saver
grows over many cycles.
"""
import json
import os
import re
import shutil
import tempfile
import threading
import time
import unittest

from test_plugins_surfaces import AWAKE, SAVER, Surfaces, js, probe as _probe, probes, publish, READY, BYPROP, BYNAME
from xdisplay import DISPLAY, HAVE_QT

APP = "org.nphil.lumatv"                 # brand.json's appId: the vote's clientName
RES = "byProp(root, 'videoUsable')"      # Home's own saver (Layers builds one)
# The calls the stand-in recorded that are the vote's: tvpower's, and its cancels. A cancel is recorded under the uri of the service its Service
# was made for; the vote's names none (it calls tvpower by uri), so its cancels have an empty uri, while the Inputs readings cancel the reading
# before each new one (kit/Inputs.qml: eim and applicationmanager, once for each time a panel, a page or the 30 s watch asks) under theirs.
LUNA = ("var L = Qt.createQmlObject('import QtQuick 2.4; import WebOSCompositorBase 1.0; import WebOSServices 1.0; QtObject { property var calls: LS.calls }', root);"
        "var calls = L.calls.filter(function(c) { return c.uri.indexOf('tvpower') > 0 || (c.method === 'cancel' && c.uri === '') }).map(function(c) { var p = {}; try { p = JSON.parse(c.payload) } catch (e) {} return {m: c.method, p: p} });")
SNAP = LUNA + ("var d = JSON.parse(s.snapshot()); return {up: d.up, owned: d.owned, active: d.active, timers: d.timers, vote: d.vote, calls: calls, shown: d.art.shown, video: d.video.state, reason: d.video.reason};")


def probe(at, label, body=SNAP):
    return _probe(at, label, body, find=RES)


def say(at, text):
    return js(at, BYPROP + "var s = byProp(root, 'videoUsable'); " + text)


def subscribed(list_name, token=1):
    """JS: tvpower's first answer to the saver's subscription on one list."""
    return "s.voter.service.response('%s/registerScreenSaverRequest', JSON.stringify({returnValue: true, subscribed: true}), %d);" % (list_name, token)


def asked(list_name, stamp, token=1):
    """JS: tvpower's question on one list (the subscription's later answer, with the timestamp every client is asked with)."""
    return "s.voter.service.response('%s/registerScreenSaverRequest', JSON.stringify({returnValue: true, subscribed: true, timestamp: '%s'}), %d);" % (list_name, stamp, token)


def registers(calls):
    return [c for c in calls if c["m"].endswith("/registerScreenSaverRequest")]


def answers(calls):
    return [(c["m"], c["p"].get("timestamp"), c["p"].get("ack")) for c in calls if c["m"].endswith("/responseScreenSaverRequest")]


def cancels(calls):
    """The vote's own cancels (see LUNA): the call each one lets go is named by its token, which the stand-in hands out 1, 2, 3... in order."""
    return [c for c in calls if c["m"] == "cancel"]


class Player:
    """The video player's end of a mailbox of its own (the saver is pointed at it with `videoPlayer.dirUrl`; /tmp/lumatv-vplane is the other video
    tests' and the real player's): it reads `req.json`, answers a `start` with `playing`, keeps its status fresh while it plays, and can be woken
    the way the real one is (a key in its window: status `idle` with its reason)."""

    def __init__(self):
        self.dir = tempfile.mkdtemp(prefix="saver-own-")
        self.lock = threading.Lock()
        self.state, self.last_seq, self.seen, self.requests = "idle", 0, None, []
        self.running = True
        self.status("idle")
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    @property
    def url(self):
        return "file://" + self.dir + "/"

    def status(self, state, reason=""):
        with self.lock:
            self.state = state
            doc = {"v": 1, "pid": 1, "ts": int(time.time() * 1000), "state": state, "reason": reason, "seq": self.last_seq, "clip": "great-wave"}
            tmp = os.path.join(self.dir, "status.json.tmp")
            with open(tmp, "w") as f:
                json.dump(doc, f)
            os.replace(tmp, os.path.join(self.dir, "status.json"))

    def wake(self):
        self.status("idle", "a key or the pointer: the screen was asked for")

    def loop(self):
        beat, answer_at = time.monotonic(), None
        while self.running:
            time.sleep(0.02)
            try:
                with open(os.path.join(self.dir, "req.json")) as f:
                    text = f.read()
            except OSError:
                text = ""
            if text and text != self.seen:
                self.seen = text
                try:
                    req = json.loads(text)
                except ValueError:
                    continue
                self.requests.append(req)
                self.last_seq = req["seq"]
                if req["op"] == "start":
                    answer_at = time.monotonic() + 0.12
                elif req["op"] == "stop":
                    answer_at = None
                    self.status("idle", "stopped")
            if answer_at and time.monotonic() >= answer_at:
                answer_at = None
                self.status("playing")
            if self.state == "playing" and time.monotonic() - beat > 0.5:
                beat = time.monotonic()
                self.status("playing")

    def ops(self):
        return [r["op"] for r in self.requests]

    def close(self):
        self.running = False
        self.thread.join(2)
        shutil.rmtree(self.dir, ignore_errors=True)


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class OwnSaver(Surfaces):
    MINE = Surfaces.MINE + ("SaverVote.qml", "SaverVideo.qml", "Layers.qml")
    THEME = {"screensaver": {"style": "clock"}}

    def run_own(self, theme, events, wait=4500, allow=()):
        at = READY + 300
        rc, out = self.run_host(theme, *publish(300, "screen", AWAKE), *events(at), "--wait", str(at + wait))
        self.assertEqual(self.own_warnings(out, allow=allow), [], out[-3000:])
        return probes(out), out

    def test_it_asks_tvpower_for_its_list_answers_no_and_ends_on_a_key_without_asking_tvpower_for_anything(self):
        def events(at):
            return [
                *probe(at, "start"),
                *say(at + 100, subscribed("/power2", 1) + subscribed("/power", 2)),
                *probe(at + 300, "subscribed"),
                *say(at + 400, asked("/power2", "111")),
                *probe(at + 900, "claimed"),
                # a key: Home takes it (the saver is up), the release goes with it, and nothing else asks for anything
                *js(at + 1000, "console.info('[key] ' + [root.key(Qt.Key_Left, true, false, 0), root.key(Qt.Key_Left, false, false, 0)].join(','));"),
                *probe(at + 1400, "keyed"),
                # the idle timer ran out again (it repeats), on the other list this time
                *say(at + 1500, asked("/power", "222", 2)),
                *probe(at + 2000, "again"),
                # Home goes away while it is up: down at once, and the TV's list is left
                *js(at + 2100, "root.hostActive = false;"),
                *probe(at + 2500, "left"),
                # a question that was already on its way: not ours to draw, so yes
                *say(at + 2600, asked("/power2", "333")),
                *probe(at + 3100, "late"),
            ]
        p, out = self.run_own(self.THEME, events)
        s = p["start"]
        self.assertEqual((s["up"], s["owned"], s["vote"]["want"]), (False, False, True), "Home is in front and the saver is wanted: on the list")
        reg = registers(s["calls"])
        # (the stand-in does not answer until the test makes it, so a subscription that went unanswered for three seconds was asked for again, as it should be)
        self.assertEqual(sorted({c["m"] for c in reg}), ["/power/registerScreenSaverRequest", "/power2/registerScreenSaverRequest"], "both of tvpower's lists")
        for c in reg:
            self.assertEqual(c["p"], {"clientName": APP, "subscribe": True})
        self.assertEqual(answers(s["calls"]), [], "nothing is voted before it is asked")
        self.assertTrue(p["subscribed"]["vote"]["registered"])
        c = p["claimed"]
        self.assertEqual((c["up"], c["owned"], c["active"]), (True, True, True), "tvpower asked and was told no: the saver draws, the power state is Active")
        self.assertEqual((c["vote"]["asked"], c["vote"]["refused"]), (1, 1))
        self.assertEqual(answers(c["calls"]), [("/power2/responseScreenSaverRequest", "111", False)], "no, with tvpower's own timestamp, on the list it asked on")
        self.assertTrue(c["timers"], "it runs while it is up")
        self.assertIn("[key] true,true", out, "the press and its release are Home's: nothing else sees them")
        k = p["keyed"]
        self.assertEqual((k["up"], k["owned"], k["timers"]), (False, False, False), "a key ends it and nothing runs")
        self.assertNotIn("/power/turnOnScreen", [x["m"] for x in k["calls"]], "the TV never left Active: there is no screen to turn on")
        a = p["again"]
        self.assertEqual((a["up"], a["owned"]), (True, True), "the next question brings it up again")
        self.assertEqual(answers(a["calls"])[-1], ("/power/responseScreenSaverRequest", "222", False))
        self.assertEqual(a["vote"]["refused"], 2)
        l = p["left"]
        self.assertEqual((l["up"], l["owned"], l["vote"]["want"], l["vote"]["registered"]), (False, False, False, False), "Home left: down, and off tvpower's list")
        self.assertEqual(len(cancels(l["calls"])) - len(cancels(a["calls"])), 2, "one cancel for each list")
        z = p["late"]
        self.assertEqual(answers(z["calls"])[-1], ("/power2/responseScreenSaverRequest", "333", True), "a question while it cannot draw is answered yes: LG's saver goes on as it would")
        self.assertEqual((z["up"], z["owned"], z["vote"]["refused"]), (False, False, 2))

    def test_a_volume_key_ends_a_saver_it_owns_and_goes_on_to_lg(self):
        def events(at):
            return [
                *say(at + 100, subscribed("/power2", 1) + asked("/power2", "901")),
                *probe(at + 600, "up"),
                *js(at + 700, "console.info('[vol] ' + root.key(Qt.Key_VolumeUp, true, false, 0));"),
                *probe(at + 1100, "after"),
            ]
        p, out = self.run_own(self.THEME, events, wait=1600)
        self.assertEqual((p["up"]["up"], p["up"]["owned"]), (True, True))
        self.assertIn("[vol] false", out, "the press is LG's: Home does not take it")
        a = p["after"]
        self.assertEqual((a["up"], a["owned"]), (False, False), "and it ends the saver, which nothing else would")
        self.assertNotIn("/power/turnOnScreen", [x["m"] for x in a["calls"]], "the TV never left Active")

    def test_nothing_is_voted_unless_the_saver_is_wanted_and_the_switch_follows_the_owner(self):
        def events(at):
            return [
                *probe(at, "own_off"),
                # the owner turns it on, then the saver off
                *say(at + 200, "root.applyTheme({screensaver: {style: 'clock', own: true}}, root.lastThemeText);"),
                *probe(at + 700, "own_on"),
                *say(at + 800, "root.applyTheme({screensaver: {style: 'off', own: true}}, root.lastThemeText);"),
                *probe(at + 1300, "style_off"),
                *say(at + 1400, asked("/power2", "444")),
                *probe(at + 1900, "asked_off"),
                # the way LG's own state is handled is what it was: `Screen Saver` brings it up and a key asks the TV for the screen
                *say(at + 2000, "root.applyTheme({screensaver: {style: 'clock', own: false}}, root.lastThemeText);"),
                *publish(at + 2100, "screen", SAVER),
                *probe(at + 2600, "lg_state"),
                *say(at + 2700, "s.key('');"),
                *probe(at + 3000, "lg_key"),
            ]
        p, out = self.run_own({"screensaver": {"style": "clock", "own": False}}, events, wait=3600)
        self.assertEqual(registers(p["own_off"]["calls"]), [], "the owner left LG's saver alone: tvpower is not asked for a place on its list")
        self.assertFalse(p["own_off"]["vote"]["want"])
        self.assertEqual(len(registers(p["own_on"]["calls"])), 2, "turned on at run time: on the list")
        self.assertTrue(p["own_on"]["vote"]["want"])
        o = p["style_off"]
        self.assertEqual((o["vote"]["want"], o["vote"]["timers"]), (False, False), "a saver the owner turned off leaves the list, and nothing of the vote runs")
        self.assertEqual(sorted(c["p"]["token"] for c in cancels(o["calls"])), list(range(1, len(registers(o["calls"])) + 1)), "each subscription it had made is let go, once")
        self.assertEqual(answers(p["asked_off"]["calls"])[-1], ("/power2/responseScreenSaverRequest", "444", True), "and a question that is still answered is yes")
        self.assertFalse(p["asked_off"]["owned"])
        g = p["lg_state"]
        self.assertEqual((g["up"], g["owned"]), (True, False), "LG's own state still brings it up")
        self.assertIn(("/power/turnOnScreen"), [x["m"] for x in p["lg_key"]["calls"]], "and a key still asks the TV to turn the screen on")
        self.assertFalse(p["lg_key"]["owned"])

    def test_the_pointer_the_tv_going_off_and_on_and_a_refusing_tvpower(self):
        def events(at):
            return [
                *say(at + 50, "s.voter.retryMs = 300; s.voter.retryFast = 2; s.voter.retrySlowMs = 700;"),
                # tvpower refuses the subscription on /power (not in a state to take it): it is asked again, quickly a few times, then slowly, and goes on being asked
                *say(at + 100, "s.voter.service.response('/power/registerScreenSaverRequest', JSON.stringify({returnValue: false, errorText: 'not yet'}), 2);" + subscribed("/power2", 1)),
                *probe(at + 1900, "retried"),
                *probe(at + 3400, "slow"),
                *say(at + 3500, asked("/power2", "555")),
                *probe(at + 3800, "up"),
                # the pointer: where it already is does nothing, a small move does nothing, a real one ends it
                *say(at + 3900, "console.info('[ptr] ' + JSON.stringify([(s.pointerAt(500, 400), s.owned), (s.pointerAt(504, 403), s.owned), (s.pointerAt(900, 700), s.owned)]));"),
                *probe(at + 4300, "pointed"),
                *say(at + 4400, asked("/power2", "666")),
                *probe(at + 4700, "up_again"),
                *publish(at + 4800, "screen", {"on": False, "state": "Active Standby", "since": 1, "source": "luna-subscribe", "error": None}),
                *probe(at + 5300, "off"),
                # the TV comes on: the subscriptions are made again (a list tvpower did not keep through standby has the client)
                *publish(at + 5400, "screen", AWAKE),
                *probe(at + 5900, "on"),
            ]
        p, out = self.run_own(self.THEME, events, wait=6400, allow=("refused the subscription",))
        power_asks = lambda k: len([c for c in registers(p[k]["calls"]) if c["m"].startswith("/power/")])
        self.assertIn(2, [c["p"]["token"] for c in cancels(p["retried"]["calls"])], "the refused call (token 2) is let go: the compositor holds a subscription's tag until it is cancelled")
        self.assertGreaterEqual(power_asks("retried"), 3, "a refused subscription is asked for again, quickly at first")
        self.assertTrue(1 <= power_asks("slow") - power_asks("retried") <= 3, "then slowly (a 300 ms round would have asked five times), and for as long as it is wanted: %d, %d" % (power_asks("retried"), power_asks("slow")))
        self.assertEqual((p["up"]["up"], p["up"]["owned"]), (True, True))
        self.assertIn("[ptr][true,true,false]", out.replace(" ", ""), "the first position, a four pixel move, then a real move")
        self.assertEqual((p["pointed"]["up"], p["pointed"]["owned"]), (False, False))
        self.assertNotIn("/power/turnOnScreen", [x["m"] for x in p["pointed"]["calls"]])
        self.assertEqual((p["up_again"]["up"], p["up_again"]["owned"]), (True, True))
        self.assertEqual((p["off"]["up"], p["off"]["owned"]), (False, False), "the TV went off: the saver is not left waiting for Home")
        on, off = p["on"], p["off"]
        self.assertGreaterEqual(len(registers(on["calls"])) - len(registers(off["calls"])), 2, "the TV came on: both lists asked for again")
        self.assertGreaterEqual(len(cancels(on["calls"])) - len(cancels(off["calls"])), 2, "after the old subscriptions were let go")
        self.assertEqual((on["up"], on["owned"], on["vote"]["want"]), (False, False, True))

    def test_the_player_taking_a_key_in_its_own_window_ends_the_saver_and_the_gpu_does_not_take_over(self):
        theme = {"screensaver": {"style": "art", "video": True, "art": {"order": "sequence"}}}
        player = Player()
        self.addCleanup(player.close)

        def wake_when_playing():
            # (this thread starts before the host does, and a loaded machine starts it slowly: it waits as long as the host may take)
            until = time.monotonic() + float(os.environ.get("HOST_TIMEOUT", "240"))
            while time.monotonic() < until:
                if player.state == "playing":
                    time.sleep(2.0)                    # (the probe of "playing" is 0.8 s after the player says so)
                    player.wake()
                    return
                time.sleep(0.02)
        waker = threading.Thread(target=wake_when_playing, daemon=True)
        waker.start()

        def events(at):
            return [*say(at - 200, "s.videoPlayer.dirUrl = '%s'; s.videoStartMs = 3000; s.videoRenewMs = 700; s.videoStaleMs = 2500;" % player.url),
                    *say(at + 300, subscribed("/power2", 1) + asked("/power2", "777")),
                    *probe(at + 1200, "playing"),
                    *probe(at + 5000, "after")]
        p, out = self.run_own(theme, events, wait=5600)
        waker.join(1)
        self.assertEqual((p["playing"]["video"], p["playing"]["owned"], p["playing"]["shown"]), ("playing", True, ""), "the player has the picture; the GPU holds none of it")
        a = p["after"]
        self.assertEqual((a["owned"], a["up"]), (False, False), "the player's own wake ended the saver")
        self.assertEqual((a["video"], a["shown"]), ("off", ""), "not a failure: no fall back to the GPU's art")
        self.assertEqual(player.ops()[0], "start")
        self.assertNotIn("/power/turnOnScreen", [x["m"] for x in a["calls"]], "the player asked tvpower itself; the shell has nothing to add")

    def test_an_old_wake_in_the_players_status_does_not_end_the_next_saver(self):
        """The player's status file still says it was woken (from the last time): the new session is not another viewer's wake."""
        theme = {"screensaver": {"style": "art", "video": True, "art": {"order": "sequence"}}}
        player = Player()
        self.addCleanup(player.close)
        player.wake()

        def events(at):
            return [*say(at - 200, "s.videoPlayer.dirUrl = '%s'; s.videoStartMs = 3000; s.videoRenewMs = 700; s.videoStaleMs = 2500;" % player.url),
                    *say(at + 300, subscribed("/power2", 1) + asked("/power2", "888")),
                    *probe(at + 2000, "playing")]
        p, out = self.run_own(theme, events, wait=2600)
        self.assertEqual((p["playing"]["video"], p["playing"]["owned"], p["playing"]["up"]), ("playing", True, True))


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Cycles(Surfaces):
    """Many saver cycles in the offline host: the host process's resident size after each (what a QML or image leak in the GPU saver would show).
    The host is Qt on the workstation with software GL: it says nothing of the TV's graphics driver, only whether the saver's own objects,
    pixmaps and timers are let go."""

    RSS = ("var t = Qt.createQmlObject('import QtQuick 2.4; import TextFile 1.0; QtObject { property var f: TextFile { name: \"file:///proc/self/status\" } }', root);"
           "var m = /VmRSS:\\s+(\\d+)/.exec(t.f.read()); console.info('[rss] %s ' + (m ? m[1] : 0));")
    CYCLES = int(os.environ.get("SAVER_CYCLES", "20"))
    STYLE = os.environ.get("SAVER_STYLE", "art")                     # "off" is the control: the same publishes, no saver

    def test_the_gpu_saver_gives_back_what_it_takes(self):
        theme = {"screensaver": {"style": self.STYLE, "art": {"order": "sequence"}}}
        step, up, down = 2600, 1500, 400                                 # ms a cycle; in the saver; awake before the sample
        at = READY + 500
        events = []
        for n in range(self.CYCLES):
            t = at + n * step
            events += [*publish(t, "screen", SAVER), *publish(t + up, "screen", AWAKE), *js(t + up + down + 400, self.RSS % n)]
        rc, out = self.run_host(theme, *publish(300, "screen", AWAKE), *say(at - 300, "s.periodMs = 1200; s.leadMs = 500; s.shiftMs = 300;"), *events,
                                "--wait", str(at + self.CYCLES * step + 500))
        rss = {int(m.group(1)): int(m.group(2)) for m in re.finditer(r"\[rss\] (\d+) (\d+)", out)}
        self.assertEqual(len(rss), self.CYCLES, out[-1500:])
        series = [rss[i] for i in range(self.CYCLES)]
        print("\nVmRSS kB after each cycle: %s" % series)
        # one reading swings by 10 MB (allocator and collector), so the windows are compared, not single readings: cycles 5-11 against the last seven
        # (centres 8 cycles apart: the two means differed by up to 3 MB in runs without a leak; a 2.9 MB print kept back each cycle would make it 23 MB)
        mid, last = sum(series[5:12]) / 7, sum(series[-7:]) / 7
        self.assertLess(last - mid, 6 * 1024, "the host grew %d kB between cycles 5-11 and the last seven: %s" % (last - mid, series))


if __name__ == "__main__":
    unittest.main()
