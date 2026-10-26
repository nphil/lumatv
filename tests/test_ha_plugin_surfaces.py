"""Run with: cd tests && python3 -m unittest test_ha_plugin_surfaces   (needs Xvfb and PyQt5; skipped without them)

Home's At-home lane with the Home Assistant plugin, through the offline QML host against the fake helper, with what the real plugin
and the real helper publish (tests/qmlhost/fixtures/ha-plugin.json). The plugin's cards are chips, the ring first, and OK on a chip
runs its first action; with no card the lane says where Home Assistant stands (not switched on, not linked, token refused, offline,
connecting, linked with nothing to show, cannot run, starting).
"""
import unittest

from ha_case import FOCUS, HUB, HaCase, feed  # noqa: E402  (first: it puts tests/qmlhost on the path)
import hafixture  # noqa: E402
from test_plugins_surfaces import GET, js, probe, probes, publish  # noqa: E402
from test_settings import READY  # noqa: E402
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Lane(HaCase):
    def test_the_real_plugins_cards_are_chips_the_ring_first_and_ok_runs_the_first_action(self):
        lane = (HUB + "var h = hub(); var w = byProp(root, 'pluginMode'); return {mode: w.pluginMode, inner: w.inner, index: w.index,"
                " cards: h.cards.map(function(c) { return c.id + ':' + c.size + ':' + c.priority }),"
                " words: h.cards.map(function(c) { var g = h.glance(c); return g.title + '|' + g.sub }),"
                " places: w.places.map(function(p) { return p.row + ':' + p.col + ':' + p.span })};")
        rc, out = self.run_host(
            {}, *feed(),
            *js(READY - 600, FOCUS),
            *probe(READY, "start", lane, find="root"),
            *self.keys(READY + 200, "ok"), *probe(READY + 500, "entered", lane, find="root"),
            *self.keys(READY + 700, "ok"),                       # the ring: its first action clears it
            *self.keys(READY + 1000, "down", "ok"),              # the room's card, below it: "All off"
            *probe(READY + 1400, "moved", lane, find="root"),
            *self.keys(READY + 1500, "back"), *probe(READY + 1800, "left", lane, find="root"),
            *js(READY + 1900, GET % ("posts", "__posts")),
            "--wait", str(READY + 2500))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        self.assertEqual(p["start"]["cards"], ["ring-1:full:running", "room:half:new"], "a ring that runs comes before the room's card")
        self.assertEqual(p["start"]["words"], ["Doorbell|Doorbell", "Living room|1 of 2 on"])
        self.assertEqual(p["start"]["places"], ["0:0:2", "1:0:1"], "the ring takes a whole row, the room's card the first place under it")
        self.assertTrue(p["start"]["mode"] and not p["start"]["inner"])
        self.assertTrue(p["entered"]["inner"])
        self.assertEqual(p["moved"]["index"], 1)
        self.assertFalse(p["left"]["inner"])
        self.assertEqual(self.plugin_posts(out), [("/v1/plugins/home-assistant/actions/clear_doorbell", {"target": "ring-1"}),
                                                  ("/v1/plugins/home-assistant/actions/all_off", {})])
        self.assertEqual(self.own_warnings(out), [])

    def test_with_no_card_the_lane_says_where_home_assistant_stands(self):
        roster = lambda state, **extra: ("plugins", {"plugins": [dict(hafixture.FIXTURE["topics"]["plugins"]["plugins"][0], state=state, **extra)] if state else [],
                                                      "firstParty": [], "problem": None})
        link = lambda state: ("ha.link", {"state": state, "detail": "", "address": "http://192.168.1.146:8123", "token": state != "unconfigured", "holders": 1})
        empty = ("plugin.home-assistant.cards", {"items": []})
        steps = [
            ("off", [roster(None)]),
            ("unlinked", [roster("connected"), link("unconfigured"), empty]),
            ("connecting", [link("connecting")]),
            ("refused", [link("auth_failed")]),
            ("offline", [link("offline")]),
            ("connected", [link("connected")]),
            ("inert", [roster("inert", reason="its manifest is not the one that was approved")]),
            ("starting", [roster("waiting")]),
            ("not running", [roster("waiting", reason="not started: its program is not the one that was installed")]),
        ]
        args, probes_at = [], []
        for n, (name, docs) in enumerate(steps):
            at = READY + 400 * n
            for topic, doc in docs:
                args += publish(at, topic, doc)
            probes_at += probe(at + 250, "s%d" % n, HUB + "var h = hub(); var w = byProp(root, 'noneTitle'); return {st: h.st, title: w.noneTitle, sub: w.noneSub, role: h.none.role, mode: w.pluginMode};", find="root")
        rc, out = self.run_host({}, *args, *probes_at, "--wait", str(READY + 400 * len(steps) + 800))
        self.assertEqual(self.own_warnings(out), [], out[-3000:])
        p = probes(out)
        got = [(name, p["s%d" % n]["st"], p["s%d" % n]["title"], p["s%d" % n]["sub"]) for n, (name, _) in enumerate(steps)]
        where = "Settings \u203a Home Assistant"
        self.assertEqual(got, [
            ("off", "off", "Home Assistant isn't linked yet", "Link it in " + where),
            ("unlinked", "unlinked", "Home Assistant isn't linked yet", "Link it in " + where),
            ("connecting", "connecting", "Connecting to Home Assistant", ""),
            ("refused", "refused", "Home Assistant refused the token", "Link it again in " + where),
            ("offline", "offline", "Can't reach Home Assistant", "It is tried again. See " + where),
            ("connected", "connected", "Nothing to show yet", "Pick a room in " + where),
            ("inert", "inert", "Home Assistant can't run on this TV", "See " + where),
            ("starting", "waiting", "Starting Home Assistant", ""),
            ("not running", "waiting", "Home Assistant isn't running", "See " + where),
        ])
        self.assertEqual([p["s%d" % n]["role"] for n in (3, 4)], ["danger", "warning"], "a refusal is bad, an outage is slow: the glyph says which")
        self.assertFalse(any(p["s%d" % n]["mode"] for n in range(len(steps))), "no card, no chips")


if __name__ == "__main__":
    unittest.main()
