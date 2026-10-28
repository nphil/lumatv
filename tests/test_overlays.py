"""Run with: cd tests && python3 -m unittest test_overlays   (needs Xvfb and PyQt5; skipped without them)

The overlay system (payload/compositor/lumatv/panels/Overlays.qml, OverlayCard.qml, the queue in Overlays.js) in the overlay over Home and over an app,
against the fake helper that publishes the plugin broker's records: a new notification is a card within a moment (and no LG toast); the most
important class first; at most two cards and none below 75 % of the height; the camera pop-up on top with a card under it; nothing while the host is
busy (and what waited comes back); OK acts and Back dismisses once the top card has been up a moment, every other key passes to the app; the
layer asks the compositor for OK and Back only while a card has them; a dismissed record is not shown again; and with the switch off nothing of it
is there and the notification is the toast it always was.
"""
import json
import os
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT, own_tmp

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
DOOR = "%s-overlay" % json.load(open(os.path.join(ROOT, "brand.json")))["shortId"]       # a name in the host's /tmp (own_tmp)

READY = 7000                                      # Home has presented and built the chain; the overlay is warm
OK, BACK, LEFT, RIGHT = 0x01000004, 0x01000061, 0x01000012, 0x01000014
ON = {"perf": {"syncResidents": True}, "overlays": {"enabled": True, "armMs": 300}}
ROSTER = {"plugins": [{"id": "ha", "name": "Home Assistant", "version": "0.2.0", "class": "L", "state": "connected", "registered": True},
                      {"id": "sports", "name": "Scores", "version": "1", "class": "S", "state": "connected", "registered": True}],
          "firstParty": [], "problem": None}
CAMERAS = {"enabled": True, "paired": True, "port": 6054, "state": "connected", "detail": "", "entities": 3, "problem": None,
           "popupKinds": ["person", "doorbell"], "popupSeconds": 12,
           "cameras": [{"id": "camera.front_door", "name": "Front door", "area": "entry", "picture": True, "stream": False, "kinds": ["person", "doorbell"]}]}


def rec(ident, title, body="", channel="ha", icon="base.icons/home", ctitle="Home Assistant", revision=1, actions=None, expires=None, **post):
    p = {"id": ident, "channel": channel, "title": title, "body": body, "privacy": "public", "ongoing": False}
    if actions:
        p["actions"] = actions
    p.update(post)
    return {"id": ident, "channel": channel, "channelTitle": ctitle, "channelIcon": icon, "revision": revision, "receivedAt": 0, "expiresAt": expires, "post": p}


def publish(topic, doc):
    return ("var d=%s; var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/%s');"
            "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d));" % (json.dumps(doc), topic))


def notes(plugin, *items):
    return publish("plugin.%s.notifications" % plugin, {"items": list(items)})


def popup(camera="camera.front_door", kind="person", lasts=12000, ident=1):
    doc = {"id": ident, "camera": camera, "name": "Front door", "kind": kind, "label": None, "at": 0, "source": "binary_sensor.x", "lastsMs": lasts, "untilMs": 0, "test": False}
    return ("var d=%s; d.at = Date.now() / 1000 - 5; d.untilMs = Date.now() + %d; var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/cameras.popup');"
            "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d));" % (json.dumps(doc), lasts))


def key(code):
    return "overlay.key(%d, true, false, 0); overlay.key(%d, false, false, 0);" % (code, code)


# the surface, as the overlay holds it
STATE = ("(function(){ var ov = overlay.overlaysItem; return JSON.stringify({on: !!ov && ov.enabled, ids: !ov ? [] : ov.shown.map(function(e) { return e.id }), "
         "cls: !ov ? [] : ov.shown.map(function(e) { return e.cls }), armed: !!ov && ov.armed, up: !!ov && ov.up, wants: overlay.wants, wantsFocus: overlay.wantsFocus, "
         "takes: overlay.takes, cardsUp: overlay.cardsUp, dnd: overlay.dnd, live: overlay.live, geometry: !ov ? [] : ov.geometry(), popupUp: overlay.popupUp, "
         "queued: !ov ? 0 : ov.queue.list.length, hubOwns: !!ov && ov.hubOwns}) })()")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Overlays(HostCase):
    def run_overlay(self, steps, wait, app=True, theme=ON):
        args = ["--theme", json.dumps(theme), "--eval", "100:(function(){ overlay.warm = true; return 1 })()"]
        if app:
            args += ["--eval", "800:(function(){ overlay.appFront = true; return 1 })()"]
        for at, js in steps:
            args += ["--eval", "%d:(function(){%s return 1})()" % (at, js)]
        args += ["--eval", "%d:%s" % (wait - 200, STATE), "--wait", str(wait)]
        rc, out = self.host(*args, "--overlay")
        docs = self.docs(out)
        self.assertTrue(docs, out[-3000:])
        return docs[-1], out

    def at(self, ms, tag):
        """A step that logs the surface's state under `tag`."""
        return (ms, "console.info('[%s] ' + %s);" % (tag, STATE))

    def test_a_new_notification_is_a_card_within_a_moment_and_no_toast_is_made(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 self.at(READY + 900, "s1"),
                 (READY + 900, "console.info('[luna] ' + JSON.stringify(root.lunaCalls.filter(function(c) { return c.method === '/createToast' })));")]
        doc, out = self.run_overlay(steps, READY + 2000)
        s1 = self.tagged(out, "s1")[0]
        self.assertEqual(s1["ids"], ["low"], out[-2500:])
        self.assertEqual(s1["cls"], ["alert"])
        self.assertTrue(s1["on"] and s1["hubOwns"], "the overlay system owns what a plugin says while it is on")
        self.assertTrue(s1["live"] and s1["wants"], "the layer is drawn and, for LG's key filter, has keys to answer")
        self.assertFalse(s1["wantsFocus"], "no card takes the keyboard focus: the app keeps its arrows")
        self.assertEqual(self.tagged(out, "luna")[0], [], "no LG toast for what the overlay shows")

    def test_the_keys_are_the_cards_only_after_armMs_and_only_ok_and_back(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 (READY + 450, "console.info('[early] ' + JSON.stringify({took: overlay.key(%d, true, false, 0), takes: overlay.takes}));" % OK),
                 self.at(READY + 900, "armed"),
                 (READY + 900, "console.info('[dpad] ' + JSON.stringify({left: overlay.key(%d, true, false, 0), right: overlay.key(%d, true, false, 0)}));" % (LEFT, RIGHT)),
                 self.at(READY + 1000, "still")]
        doc, out = self.run_overlay(steps, READY + 1800)
        early = self.tagged(out, "early")[0]
        self.assertFalse(early["took"], "a press made as the card arrives was the app's")
        self.assertEqual(early["takes"], [])
        armed = self.tagged(out, "armed")[0]
        self.assertTrue(armed["armed"])
        self.assertEqual(armed["takes"], ["ok", "back"], "the layer asks the compositor for OK and Back, and for nothing else")
        dpad = self.tagged(out, "dpad")[0]
        self.assertEqual(dpad, {"left": False, "right": False}, "the arrows pass to the app")
        self.assertEqual(self.tagged(out, "still")[0]["ids"], ["low"])

    def test_back_dismisses_and_the_same_record_is_not_shown_again_ok_runs_the_records_button(self):
        clear = [{"action": "clear_doorbell", "label": "Clear", "target": "ring-1"}]
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 (READY + 900, "console.info('[back] ' + JSON.stringify({took: overlay.key(%d, true, false, 0), up: overlay.overlaysItem.up}));" % BACK),
                 # the same record again (a revision of the hub's lists, nothing new): not a second card
                 (READY + 1100, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"), rec("other", "Leak", "Basement"))),
                 (READY + 1200, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"), rec("other", "Leak", "Basement"), rec("ring", "Doorbell", "Front door", channel="doorbell", icon="base.icons/doorbell", actions=clear))),
                 self.at(READY + 1800, "ring"),
                 (READY + 2300, "console.info('[ok] ' + JSON.stringify({took: overlay.key(%d, true, false, 0)}));" % OK),
                 (READY + 2800, "var x=new XMLHttpRequest(); x.open('GET','http://127.0.0.1:18731/__posts', false); x.send(); console.info('[posts] ' + x.responseText);"),
                 self.at(READY + 2900, "after")]
        doc, out = self.run_overlay(steps, READY + 3600)
        back = self.tagged(out, "back")[0]
        self.assertTrue(back["took"])
        self.assertFalse(back["up"])
        ring = self.tagged(out, "ring")[0]
        self.assertEqual(ring["ids"], ["ring"], "the doorbell first; the leak waits for it and the dismissed one is not back: " + out[-1500:])
        self.assertEqual(ring["cls"], ["doorbell"])
        self.assertTrue(self.tagged(out, "ok")[0]["took"])
        posts = self.tagged(out, "posts")[0]["posts"]
        self.assertIn(["/v1/plugins/ha/actions/clear_doorbell", {"target": "ring-1"}], posts, "OK ran the record's own button")
        after = self.tagged(out, "after")[0]
        self.assertNotIn("low", after["ids"])

    def test_two_cards_at_most_the_most_important_first_and_never_below_the_limit(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("a1", "One", "first alert"), rec("a2", "Two", "second alert"), rec("a3", "Three", "third alert"),
                                     rec("t", "Pasta", "Time's up", channel="timers", icon="base.icons/timer"))),
                 self.at(READY + 1200, "shown")]
        doc, out = self.run_overlay(steps, READY + 2000)
        s = self.tagged(out, "shown")[0]
        self.assertEqual(s["ids"], ["t"], "a timer that finished comes before alerts, and alerts wait while it is up: " + out[-2000:])
        self.assertEqual(s["cls"], ["timer"])
        for g in s["geometry"]:
            self.assertLessEqual(g["y"] + g["h"], 810, "no card ends below 75 % of the height")
            self.assertEqual(g["x"] + g["w"], 1920 - 96, "inside the 5 % safe area")
            self.assertGreaterEqual(g["y"], 134)

    def test_two_alerts_stack_one_under_the_other_inside_the_zone(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("a1", "One", "first alert"), rec("a2", "Two", "second alert"), rec("a3", "Three", "third alert"))),
                 self.at(READY + 1200, "shown")]
        doc, out = self.run_overlay(steps, READY + 2000)
        s = self.tagged(out, "shown")[0]
        self.assertEqual(s["ids"], ["a1", "a2"], out[-2000:])
        g = s["geometry"]
        self.assertEqual(len(g), 2)
        self.assertEqual(g[0]["y"], 134)
        self.assertGreater(g[1]["y"], g[0]["y"] + g[0]["h"], "the second is under the first, with a gap")
        self.assertEqual(g[1]["y"] - (g[0]["y"] + g[0]["h"]), 16)
        self.assertLessEqual(g[1]["y"] + g[1]["h"], 810)

    def test_a_card_waits_while_the_host_is_busy_and_comes_back(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 200, "overlay.systemBusy = true;"),
                 (READY + 400, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 self.at(READY + 1200, "busy"),
                 (READY + 1300, "overlay.systemBusy = false;"),
                 self.at(READY + 2000, "free")]
        doc, out = self.run_overlay(steps, READY + 2600)
        busy = self.tagged(out, "busy")[0]
        self.assertTrue(busy["dnd"])
        self.assertEqual(busy["ids"], [], "a PIN, an alert or the keyboard is up: nothing shows")
        self.assertEqual(busy["queued"], 1, "it waits")
        self.assertFalse(busy["live"])
        self.assertEqual(self.tagged(out, "free")[0]["ids"], ["low"])

    def test_a_card_that_is_up_steps_aside_for_a_prompt_and_is_back_with_the_time_it_had_left(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 self.at(READY + 900, "up"),
                 (READY + 1000, "overlay.systemBusy = true;"),
                 self.at(READY + 1500, "away"),
                 (READY + 1600, "overlay.systemBusy = false;"),
                 self.at(READY + 2100, "back")]
        doc, out = self.run_overlay(steps, READY + 2600)
        self.assertEqual(self.tagged(out, "up")[0]["ids"], ["low"])
        self.assertEqual(self.tagged(out, "away")[0]["ids"], [])
        self.assertEqual(self.tagged(out, "back")[0]["ids"], ["low"])

    def test_the_camera_popup_is_on_top_and_a_card_goes_under_it_inside_the_zone(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha") + publish("cameras", CAMERAS)),
                 (READY + 300, popup()),
                 (READY + 800, notes("ha", rec("t", "Pasta", "Time's up", channel="timers", icon="base.icons/timer"))),
                 (READY + 1300, "console.info('[cam] ' + JSON.stringify({up: overlay.popupUp, bottom: overlay.cameraItem.popupBottom, ph: overlay.cameraItem.previewH}));"),
                 self.at(READY + 1500, "both")]
        doc, out = self.run_overlay(steps, READY + 2200)
        cam = self.tagged(out, "cam")[0]
        both = self.tagged(out, "both")[0]
        self.assertTrue(cam["up"], out[-2500:])
        self.assertEqual(cam["ph"], 292)
        self.assertEqual(both["ids"], ["t"])
        g = both["geometry"][0]
        self.assertEqual(g["y"], cam["bottom"] + 16, "under the camera's card with the gap")
        self.assertLessEqual(g["y"] + g["h"], 810)
        self.assertTrue(both["cardsUp"] and both["wants"] and not both["wantsFocus"], "the pop-up too leaves the keyboard focus to the app")

    def test_an_alert_waits_under_the_camera_popup_it_is_a_lesser_class_and_comes_when_it_is_gone(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha") + publish("cameras", CAMERAS)),
                 (READY + 300, popup(lasts=2500)),
                 (READY + 700, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 self.at(READY + 1500, "during"),
                 self.at(READY + 4200, "after")]
        doc, out = self.run_overlay(steps, READY + 4800)
        self.assertEqual(self.tagged(out, "during")[0]["ids"], [], "a doorbell moment is not shared with an alert: " + out[-2000:])
        self.assertEqual(self.tagged(out, "after")[0]["ids"], ["low"])

    def test_a_doorbell_record_and_the_cameras_doorbell_popup_are_one_card(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha") + publish("cameras", CAMERAS)),
                 (READY + 300, popup(kind="doorbell")),
                 (READY + 700, notes("ha", rec("ring", "Doorbell", "Front door", channel="doorbell", icon="base.icons/doorbell"))),
                 self.at(READY + 1400, "one")]
        doc, out = self.run_overlay(steps, READY + 2000)
        one = self.tagged(out, "one")[0]
        self.assertEqual(one["ids"], [], "the record is the camera's card, not a second one: " + out[-2000:])
        self.assertTrue(one["popupUp"])

    def test_a_doorbell_record_alone_is_a_card_and_the_camera_that_comes_after_takes_it_over(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha") + publish("cameras", CAMERAS)),
                 (READY + 300, notes("ha", rec("ring", "Doorbell", "Front door", channel="doorbell", icon="base.icons/doorbell"))),
                 self.at(READY + 800, "alone"),
                 (READY + 900, popup(kind="doorbell")),
                 self.at(READY + 1500, "taken")]
        doc, out = self.run_overlay(steps, READY + 2100)
        self.assertEqual(self.tagged(out, "alone")[0]["ids"], ["ring"])
        taken = self.tagged(out, "taken")[0]
        self.assertEqual(taken["ids"], [])
        self.assertTrue(taken["popupUp"])

    def test_a_record_the_plugin_ends_takes_its_card_with_it(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 self.at(READY + 900, "up"),
                 (READY + 1000, notes("ha")),
                 self.at(READY + 1500, "gone")]
        doc, out = self.run_overlay(steps, READY + 2000)
        self.assertEqual(self.tagged(out, "up")[0]["ids"], ["low"])
        self.assertEqual(self.tagged(out, "gone")[0]["ids"], [])

    def test_a_score_reads_its_two_sides_and_waits_for_an_alert(self):
        score = {"league": "NBA", "status": "Q4 2:31", "live": True, "home": {"name": "Celtics", "abbr": "BOS", "score": 99}, "away": {"name": "Lakers", "abbr": "LAL", "score": 102}}
        steps = [(READY, publish("plugins", ROSTER) + notes("ha") + notes("sports")),
                 (READY + 300, notes("sports", rec("g1", "Lakers 102, Celtics 99", "Q4 2:31", channel="scores", icon="base.icons/trophy", ctitle="Scores", score=score))),
                 (READY + 400, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 self.at(READY + 1000, "first"),
                 (READY + 1100, "overlay.key(%d, true, false, 0);" % BACK),
                 self.at(READY + 1700, "second")]
        doc, out = self.run_overlay(steps, READY + 2300)
        first = self.tagged(out, "first")[0]
        self.assertEqual(first["ids"], ["low"], "an alert is before a score: " + out[-2000:])
        second = self.tagged(out, "second")[0]
        self.assertEqual(second["ids"], ["g1"])
        self.assertEqual(second["cls"], ["score"])

    def test_with_the_switch_off_there_is_nothing_of_it_and_the_notification_is_the_toast_it_was(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 self.at(READY + 900, "off"),
                 (READY + 900, "console.info('[luna] ' + JSON.stringify(root.lunaCalls.filter(function(c) { return c.method === '/createToast' }).length));")]
        doc, out = self.run_overlay(steps, READY + 1600, theme={"perf": {"syncResidents": True}})
        off = self.tagged(out, "off")[0]
        self.assertFalse(off["on"])
        self.assertEqual(off["ids"], [])
        self.assertFalse(off["hubOwns"])
        self.assertEqual(off["takes"], [])
        self.assertIn("[luna] 1", out, "the toast LG's notification service draws")

    def test_the_door_makes_a_card_by_hand_for_the_gate_and_the_tv_steps(self):
        score = {"league": "NBA", "status": "Q4 2:31", "live": True, "home": {"name": "Celtics", "score": 99}, "away": {"name": "Lakers", "score": 102}}
        door = {"card": {"class": "score", "id": "g1", "title": "Lakers 102, Celtics 99", "body": "Q4 2:31", "source": "Scores", "score": score, "ttl": 60}}
        with open(os.path.join(own_tmp(self), DOOR), "w") as f:
            json.dump(door, f)
        # the door's own timer ticks once a second from the moment an app is in front; the step reads it itself
        steps = [(READY, "overlay.readDoor();"), self.at(READY + 800, "door")]
        doc, out = self.run_overlay(steps, READY + 1400)
        door = self.tagged(out, "door")[0]
        self.assertEqual(door["ids"], ["g1"], out[-2000:])
        self.assertEqual(door["cls"], ["score"])
        self.assertEqual(door["queued"], 1)

    def test_over_home_the_cards_show_too_and_the_keys_come_through_the_bus(self):
        steps = [(READY, publish("plugins", ROSTER) + notes("ha")),
                 (READY + 300, notes("ha", rec("low", "Low battery", "Door sensor is at 5 %"))),
                 (READY + 1200, "console.info('[home] ' + JSON.stringify({up: overlay.overlaysItem.up, wants: overlay.wants, takes: overlay.takes, took: root.keyAction('back'), after: overlay.overlaysItem.up}));")]
        doc, out = self.run_overlay(steps, READY + 1800, app=False)
        home = self.tagged(out, "home")[0]
        self.assertTrue(home["up"], out[-2000:])
        self.assertFalse(home["wants"], "over Home the layer asks the filter for nothing; Home routes through the bus")
        self.assertEqual(home["takes"], [], "nor the compositor: Home has the focus")
        self.assertTrue(home["took"])
        self.assertFalse(home["after"])


if __name__ == "__main__":
    unittest.main()
