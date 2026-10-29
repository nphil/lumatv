"""Run with: cd tests && python3 -m unittest test_camera_popup   (needs Xvfb and PyQt5; skipped without them)

The camera pop-up and viewer (payload/compositor/lumatv/panels/Cameras.qml) in the overlay over Home and over an app, against the fake
helper and the topics of the Scrypted design (docs/research/scrypted-cameras-design.md 3.4: `cameras` with a `state`, `cameras.popup` with times
in milliseconds, `cameras.live`): a `cameras.popup` topic shows the card within a moment and its preview asks the helper for pictures of its own
size; the card goes by itself at `untilMs`; Back dismisses it and tells the helper; OK expands it to the full view, which takes every key, goes to
the grid and back; over an app the D-pad passes through a pop-up and is taken by the viewer; over Home the keys reach the surface through the bus; a
pop-up waits while the host is busy and arrives when it is free; nothing asks for a picture once the surface is down; live video is asked for over
Home while the link is `ready`, and over an app only when the theme says so.
"""
import json
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

READY = 7000                                      # Home has presented and built the chain; the overlay is warm


def camera(ident, name, room="", **extra):
    doc = {"id": ident, "name": name, "room": room, "type": "camera", "online": True, "live": True, "recordings": True, "events": True, "clips": False,
           "since": 0, "native": {"codec": "h264", "width": 1920, "height": 1080, "fps": 15}, "prebuffered": True}
    doc.update(extra)
    return doc


# the `cameras` topic of the Scrypted helper: the cameras are already ordered and filtered by what Settings shows
CAMERAS = {"state": "ready", "reason": "", "now_skew_ms": 0, "rev": 1,
           "cameras": [camera("274", "Back Door"), camera("273", "Front Door", "Entry", type="doorbell"), camera("275", "Garden")]}
# Qt's key codes, as the key filter hands them to the overlay and to Home
OK, BACK, LEFT, RIGHT, DOWN = 0x01000004, 0x01000061, 0x01000012, 0x01000014, 0x01000015


def publish(topic, doc):
    return ("var d=%s; var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/%s');"
            "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d));" % (json.dumps(doc), topic))


def popup(camera="273", kind="person", lasts=12000, ident=1, label=None, age=5000):
    doc = {"id": ident, "event": "e%d" % ident, "camera": camera, "name": "Front Door" if camera == "273" else camera, "kind": kind, "label": label,
           "at": 0, "untilMs": 0, "seq": ident}
    # `at` and `untilMs` are epoch milliseconds, set in the host's own clock
    return ("var d=%s; d.at = Date.now() - %d; d.untilMs = Date.now() + %d; var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/cameras.popup');"
            "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d));" % (json.dumps(doc), age, lasts))


def frames():
    return "var fx=new XMLHttpRequest(); fx.open('GET','http://127.0.0.1:18731/__frames', false); fx.send(); var frames=JSON.parse(fx.responseText).frames;"


# the surface, as the overlay holds it
C = "overlay.cameraItem"
STATE = ("JSON.stringify({mode: %s.mode, up: %s.up, wants: %s.wants, popupUp: %s.popupUp, current: %s.current, focus: %s.focusIndex, "
         "shownId: %s.shownId, overlayWants: overlay.wants, cameraUp: overlay.cameraUp})" % ((C,) * 7))


def key(code):
    return "overlay.key(%d, true, false, 0); overlay.key(%d, false, false, 0);" % (code, code)


def hkey(name):
    """A key over Home, which reaches the surface through the bus (`root.keyAction`)."""
    return "root.keyAction('%s');" % name


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class CameraPopup(HostCase):
    def run_overlay(self, steps, wait, app=True):
        args = ["--eval", "100:(function(){ overlay.warm = true; return 1 })()"]
        if app:
            args += ["--eval", "800:(function(){ overlay.appFront = true; return 1 })()"]
        for at, js in steps:
            args += ["--eval", "%d:(function(){%s return 1})()" % (at, js)]
        args += ["--eval", "%d:(function(){return %s})()" % (wait - 200, STATE), "--wait", str(wait)]
        rc, out = self.host(*args, "--overlay")
        docs = self.docs(out)
        self.assertTrue(docs, out[-3000:])
        return docs[-1], out

    def test_a_popup_shows_within_a_moment_asks_for_pictures_of_its_own_size_and_goes_by_itself(self):
        steps = [(READY, publish("cameras", CAMERAS)),
                 (READY + 300, popup(lasts=4000)),
                 (READY + 2300, "var c = %s; console.info('[p1] ' + JSON.stringify({mode: c.mode, popupUp: c.popupUp, wants: c.wants, overlayWants: overlay.wants, "
                               "name: c.popup ? c.popup.name : '', what: c.kindWord(c.popup), ago: c.ago, "
                               "shownAt: c.shownAt - (c.popup ? c.popup.untilMs - 4000 : 0), pad: c.padX, pw: c.previewW, ph: c.previewH}));" % C),
                 (READY + 2800, frames() + "console.info('[f1] ' + JSON.stringify(frames));")]
        doc, out = self.run_overlay(steps, READY + 6000)
        p1 = self.tagged(out, "p1")[0]
        self.assertEqual(p1["mode"], "popup", out[-2000:])
        self.assertTrue(p1["popupUp"] and p1["overlayWants"], "the overlay takes OK and Back for the pop-up over an app")
        self.assertFalse(p1["wants"], "the surface asks for no other key")
        self.assertEqual((p1["name"], p1["what"], p1["ago"]), ("Front Door", "Person", "Just now"))
        # the host's clock, from the publish to the show: one long-poll wake. The bound is loose because the host's main thread can still be
        # building Home when the step runs; the helper's own event-to-pop-up time is held under a second by helper/app/tests/cameras.rs.
        self.assertLess(p1["shownAt"], 2000, "the card is up within a moment of the topic")
        self.assertEqual(p1["pad"], 20)
        self.assertEqual((p1["pw"], p1["ph"]), (520, 292))
        frames_asked = self.tagged(out, "f1")[0]
        self.assertTrue(frames_asked, "the preview asks the helper for pictures")
        self.assertEqual(frames_asked[0], ["273", 520, 292], "a picture of the preview's own size")
        self.assertEqual(doc["mode"], "", "gone by itself at untilMs")
        self.assertFalse(doc["overlayWants"])

    def test_over_an_app_the_dpad_passes_through_a_popup_back_dismisses_and_tells_the_helper(self):
        steps = [(READY, publish("cameras", CAMERAS)),
                 (READY + 300, popup()),
                 (READY + 900, "var took = overlay.key(%d, true, false, 0); console.info('[d1] ' + JSON.stringify({took: took, mode: %s.mode}));" % (RIGHT, C)),
                 (READY + 1100, key(BACK) + "console.info('[b1] ' + JSON.stringify({mode: %s.mode, shownId: %s.shownId}));" % (C, C)),
                 (READY + 1600, "var x=new XMLHttpRequest(); x.open('GET','http://127.0.0.1:18731/__posts', false); x.send(); console.info('[posts] ' + x.responseText);")]
        doc, out = self.run_overlay(steps, READY + 2200)
        d1 = self.tagged(out, "d1")[0]
        self.assertFalse(d1["took"], "Right goes on to the app")
        self.assertEqual(d1["mode"], "popup")
        b1 = self.tagged(out, "b1")[0]
        self.assertEqual((b1["mode"], b1["shownId"]), ("", 1))
        posts = self.tagged(out, "posts")[0]["posts"]
        self.assertIn(["/v1/cameras/popup/clear", {}], posts, "the helper is told, so every reader sees it go")
        self.assertEqual(doc["mode"], "", "the same topic value, re-read, is not a second pop-up")

    def test_ok_expands_to_the_full_view_which_takes_every_key_goes_to_the_grid_and_closes_on_back(self):
        steps = [(READY, publish("cameras", CAMERAS)),
                 (READY + 300, popup(label="Domestic Cat", kind="animal")),
                 (READY + 800, "var c = %s; console.info('[w1] ' + JSON.stringify({what: c.kindWord(c.popup)}));" % C + key(OK) +
                               "console.info('[o1] ' + JSON.stringify({mode: c.mode, wants: c.wants, overlayWants: overlay.wants, current: c.current, name: c.nameOf(c.current)}));"),
                 (READY + 1100, key(RIGHT) + key(RIGHT) + "console.info('[r1] ' + JSON.stringify({current: %s.current}));" % C + key(OK) +
                                "console.info('[g1] ' + JSON.stringify({mode: %s.mode, focus: %s.focusIndex, cols: %s.gridCols, n: %s.gridN, tw: %s.tileW, th: %s.tileH}));" % ((C,) * 6)),
                 (READY + 1400, key(RIGHT) + key(DOWN) + key(LEFT) + "console.info('[g2] ' + JSON.stringify({focus: %s.focusIndex}));" % C + key(OK) +
                                "console.info('[f2] ' + JSON.stringify({mode: %s.mode, current: %s.current}));" % (C, C)),
                 (READY + 1700, key(BACK))]
        doc, out = self.run_overlay(steps, READY + 2300)
        self.assertEqual(self.tagged(out, "w1")[0]["what"], "Animal \u00b7 Domestic Cat")
        o1 = self.tagged(out, "o1")[0]
        self.assertEqual(o1["mode"], "full")
        self.assertTrue(o1["wants"] and o1["overlayWants"], "the viewer takes every key")
        self.assertEqual((o1["current"], o1["name"]), (1, "Front Door"))
        self.assertEqual(self.tagged(out, "r1")[0]["current"], 0, "right twice from the second of three wraps to the first")
        g1 = self.tagged(out, "g1")[0]
        self.assertEqual((g1["mode"], g1["focus"], g1["cols"], g1["n"]), ("grid", 0, 2, 3))
        self.assertEqual((g1["tw"], g1["th"]), (764, 430), "two rows of two fit the safe area with their names")
        self.assertEqual(self.tagged(out, "g2")[0]["focus"], 2, "right, down lands on the lone tile of the row below, left stays")
        f2 = self.tagged(out, "f2")[0]
        self.assertEqual((f2["mode"], f2["current"]), ("full", 2))
        self.assertEqual(doc["mode"], "", "Back closes the viewer")
        self.assertFalse(doc["up"] or doc["overlayWants"])

    def test_over_home_the_keys_reach_the_surface_through_the_bus(self):
        steps = [(READY, publish("cameras", CAMERAS)),
                 (READY + 300, popup()),
                 (READY + 900, "console.info('[h1] ' + JSON.stringify({mode: %s.mode, overlayWants: overlay.wants, took: root.keyAction('ok'), after: %s.mode}));" % (C, C)),
                 (READY + 1200, "console.info('[h2] ' + JSON.stringify({took: root.keyAction('back'), after: %s.mode}));" % C)]
        doc, out = self.run_overlay(steps, READY + 1800, app=False)
        h1 = self.tagged(out, "h1")[0]
        self.assertEqual(h1["mode"], "popup")
        self.assertFalse(h1["overlayWants"], "over Home the overlay asks the filter for nothing; Home routes through the bus")
        self.assertTrue(h1["took"])
        self.assertEqual(h1["after"], "full", "OK on Home expands")
        h2 = self.tagged(out, "h2")[0]
        self.assertTrue(h2["took"])
        self.assertEqual(h2["after"], "")
        self.assertEqual(doc["mode"], "")

    def test_a_popup_waits_while_the_host_is_busy_and_nothing_asks_for_pictures_once_it_is_down(self):
        steps = [(READY, publish("cameras", CAMERAS) + " overlay.surface = 'quick';"),
                 (READY + 300, popup(lasts=6000)),
                 (READY + 900, "console.info('[busy] ' + JSON.stringify({mode: %s.mode})); overlay.surface = '';" % C),
                 (READY + 1700, "console.info('[free] ' + JSON.stringify({mode: %s.mode}));" % C + key(BACK)),
                 (READY + 2000, frames() + "console.info('[n1] ' + JSON.stringify({n: frames.length}));"),
                 (READY + 3500, frames() + "console.info('[n2] ' + JSON.stringify({n: frames.length}));")]
        doc, out = self.run_overlay(steps, READY + 3900)
        self.assertEqual(self.tagged(out, "busy")[0]["mode"], "", "a panel is up: the pop-up waits")
        self.assertEqual(self.tagged(out, "free")[0]["mode"], "popup", "and arrives when the host is free")
        before, after = self.tagged(out, "n1")[0]["n"], self.tagged(out, "n2")[0]["n"]
        self.assertGreater(before, 0)
        self.assertEqual(after, before, "nothing fetches once the surface is down")
        self.assertEqual(doc["mode"], "")


    def test_over_home_the_full_view_asks_for_the_live_stream_opens_a_hole_while_it_plays_and_stops_it_on_close(self):
        steps = [(READY, publish("cameras", CAMERAS)),
                 (READY + 300, popup()),
                 (READY + 900, hkey("ok") + "console.info('[l1] ' + JSON.stringify({mode: %s.mode, asked: %s.liveAsked, hole: %s.hole, frame: %s.children.length > 0}));" % ((C,) * 4)),
                 (READY + 1300, publish("cameras.live", {"camera": "273", "mode": "live", "state": "playing", "reason": "", "codec": "h265", "width": 3840, "height": 2160, "fps": 15,
                                                        "startedAt": 0, "positionMs": 0, "rate": 1, "paused": False, "fed": 12, "dropped": 0, "lost": 0, "delayMs": 4, "window": "card", "since": 0})),
                 (READY + 1900, "console.info('[l2] ' + JSON.stringify({hole: %s.hole, playing: %s.livePlaying}));" % (C, C) + hkey("right") +
                                "console.info('[l3] ' + JSON.stringify({asked: %s.liveAsked, hole: %s.hole, current: %s.current, id: %s.currentId, avail: %s.liveAvailable, mode: %s.mode}));" % ((C,) * 6)),
                 (READY + 2300, hkey("back") + "console.info('[l4] ' + JSON.stringify({asked: %s.liveAsked, mode: %s.mode}));" % (C, C)),
                 (READY + 2800, "var x=new XMLHttpRequest(); x.open('GET','http://127.0.0.1:18731/__posts', false); x.send(); console.info('[posts] ' + x.responseText);")]
        doc, out = self.run_overlay(steps, READY + 3300, app=False)
        l1 = self.tagged(out, "l1")[0]
        self.assertEqual((l1["mode"], l1["asked"], l1["hole"]), ("full", "273", False), "asked for the stream, pictures until it plays")
        l2 = self.tagged(out, "l2")[0]
        self.assertTrue(l2["hole"] and l2["playing"], "the layer is a hole over the video plane while the stream plays")
        l3 = self.tagged(out, "l3")[0]
        self.assertEqual((l3["asked"], l3["hole"]), ("275", False), "the next camera is asked for; the hole waits for its stream")
        l4 = self.tagged(out, "l4")[0]
        self.assertEqual((l4["asked"], l4["mode"]), ("", ""))
        posts = [p for p in self.tagged(out, "posts")[0]["posts"] if p[0].startswith("/v1/cameras/live")]
        self.assertEqual(posts, [["/v1/cameras/live", {"camera": "273"}], ["/v1/cameras/live", {"camera": "275"}], ["/v1/cameras/live/stop", {}]])
        self.assertEqual(doc["mode"], "")

    def test_over_an_app_the_full_view_is_pictures_only_and_asks_for_no_live_stream(self):
        steps = [(READY, publish("cameras", CAMERAS)),
                 (READY + 300, popup()),
                 (READY + 900, key(OK) + "console.info('[a1] ' + JSON.stringify({mode: %s.mode, avail: %s.liveAvailable, asked: %s.liveAsked, hole: %s.hole}));" % ((C,) * 4)),
                 (READY + 1500, frames() + "console.info('[f1] ' + JSON.stringify(frames));"),
                 (READY + 1800, "var x=new XMLHttpRequest(); x.open('GET','http://127.0.0.1:18731/__posts', false); x.send(); console.info('[posts] ' + x.responseText);")]
        doc, out = self.run_overlay(steps, READY + 2300)
        a1 = self.tagged(out, "a1")[0]
        self.assertEqual((a1["mode"], a1["avail"], a1["asked"], a1["hole"]), ("full", False, "", False), "an app is playing: the feeder's window over it is unmeasured")
        self.assertTrue([f for f in self.tagged(out, "f1")[0] if f[0] == "273"], "the full view draws the camera's pictures")
        posts = [p for p in self.tagged(out, "posts")[0]["posts"] if p[0].startswith("/v1/cameras/live")]
        self.assertEqual(posts, [])

    def test_over_an_app_live_is_asked_for_only_when_the_theme_offers_it(self):
        offer = "root.theme = root.mergeDeep(root.theme, {cameras: {liveOverApps: true}});"
        steps = [(READY, publish("cameras", CAMERAS) + offer),
                 (READY + 300, popup()),
                 (READY + 900, key(OK) + "console.info('[a1] ' + JSON.stringify({mode: %s.mode, avail: %s.liveAvailable, asked: %s.liveAsked}));" % ((C,) * 3))]
        doc, out = self.run_overlay(steps, READY + 1500)
        a1 = self.tagged(out, "a1")[0]
        self.assertEqual((a1["mode"], a1["avail"], a1["asked"]), ("full", True, "273"))

    def test_a_link_that_is_not_ready_asks_for_no_live_stream(self):
        offline = dict(CAMERAS, state="offline", reason="Scrypted does not answer")
        steps = [(READY, publish("cameras", offline)),
                 (READY + 300, popup()),
                 (READY + 900, hkey("ok") + "console.info('[o1] ' + JSON.stringify({mode: %s.mode, avail: %s.liveAvailable, asked: %s.liveAsked}));" % ((C,) * 3))]
        doc, out = self.run_overlay(steps, READY + 1500, app=False)
        o1 = self.tagged(out, "o1")[0]
        self.assertEqual((o1["mode"], o1["avail"], o1["asked"]), ("full", False, ""))

    def test_the_card_says_what_was_seen_without_repeating_the_class_and_how_long_ago_from_milliseconds(self):
        steps = [(READY, publish("cameras", CAMERAS)),
                 (READY + 300, popup(label="person", age=125000, lasts=20000)),
                 (READY + 900, "var c = %s; console.info('[w1] ' + JSON.stringify({what: c.kindWord(c.popup), ago: c.ago}));" % C +
                               "console.info('[w2] ' + JSON.stringify({animal: c.kindWord({kind: 'animal', label: 'Animal'}), named: c.kindWord({kind: 'animal', label: 'Domestic Cat'}), bell: c.kindWord({kind: 'doorbell', label: 'doorbell'})}));")]
        doc, out = self.run_overlay(steps, READY + 1500)
        self.assertEqual(self.tagged(out, "w1")[0], {"what": "Person", "ago": "2 min ago"})
        self.assertEqual(self.tagged(out, "w2")[0], {"animal": "Animal", "named": "Animal \u00b7 Domestic Cat", "bell": "Doorbell rang"})


if __name__ == "__main__":
    unittest.main()
