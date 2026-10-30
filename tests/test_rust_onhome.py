"""Run with: cd tests && python3 -m unittest test_rust_onhome   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The smart-home card of Home in the Rust shell (rustui/ui/src/home/onhome.rs) on the host harness, fed the helper's `onhome` topic as the helper
makes it (helper/app/src/onhome/tile.rs): the card lists Home Assistant's shared things as tiles two to a row, OK on the card enters it, OK
on a tile does its one action, a held OK opens the tile's controls over a stepped-back Home, and Back closes them. With no tile the card says
why. The renders (JPEGs) go beside the other Rust Home mocks (docs/mocks/host-rust/onhome/), dark and light, 100 % and 200 %.
"""
import json
import os
import tempfile
import unittest

import test_rust_home as base

BIN, MESA, ROOT = base.BIN, base.MESA, base.ROOT

OUT = os.path.join(ROOT, 'docs', 'mocks', 'host-rust', 'onhome')
SHOTS = os.path.join(tempfile.gettempdir(), 'rust-onhome-shots')
# When the pictures are taken: late, because on a loaded host Home and its glass take seconds to settle and a picture is of the first frame after its time
SNAP = 8800

LAYOUT = {"grid": {"rows": 16}, "widgets": [
    {"id": "smart", "type": "onhome", "col": 0, "row": 0, "span": 6, "rows": 9, "title": "Smart home"},
    {"id": "clock", "type": "clock", "col": 6, "row": 0, "span": 6, "rows": 4}, {"id": "weather", "type": "weather", "col": 6, "row": 4, "span": 6, "rows": 5},
    {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]}

TILES = [
    {"id": "light.kitchen", "name": "Kitchen main light", "area": "Kitchen", "group": "lights", "words": "On \u00b7 50%", "tone": "on", "icon": "lightbulb", "available": True, "can": ["toggle"],
     "verbs": ["turn_on", "turn_off", "toggle"], "ctl": {"on": True, "dimmable": True, "brightness": 50, "kelvin": {"now": 3000, "min": 2000, "max": 6500}}},
    {"id": "light.desk", "name": "Desk lamp", "area": "Office", "group": "lights", "words": "Off", "tone": "off", "icon": "lightbulb", "available": True, "can": ["toggle"], "verbs": ["toggle"],
     "ctl": {"on": False, "dimmable": False, "brightness": None, "kelvin": None}},
    {"id": "scene.movie", "name": "Movie night", "group": "scenes", "words": "Scene", "tone": "muted", "icon": "sparkles", "available": True, "can": ["run"], "verbs": ["turn_on"], "ctl": {}},
    {"id": "climate.living", "name": "Living room thermostat", "area": "Living room", "group": "climate", "words": "68\u00b0 \u00b7 Heating to 70\u00b0", "tone": "warn", "icon": "thermometer", "available": True,
     "can": ["sheet"], "verbs": ["set_temperature", "set_hvac_mode"],
     "ctl": {"mode": "heat", "modes": ["off", "heat", "cool"], "current": 68.0, "target": 70.0, "min": 50.0, "max": 90.0, "step": 1.0, "unit": "\u00b0F", "action": "heating"}},
    {"id": "cover.blinds", "name": "Master bedroom blinds", "group": "covers", "words": "Open \u00b7 60%", "tone": "on", "icon": "blinds", "available": True, "can": ["sheet"],
     "verbs": ["open_cover", "close_cover", "stop_cover", "set_cover_position"], "ctl": {"state": "open", "position": 60}},
    {"id": "sensor.hall", "name": "Hall temperature", "area": "Hall", "group": "sensors", "words": "21.4\u00b0C", "tone": "info", "icon": "thermometer", "available": True, "can": [], "verbs": [], "ctl": {}},
    {"id": "binary_sensor.door", "name": "Front door", "group": "sensors", "words": "Closed", "tone": "off", "icon": "door-closed", "available": True, "can": [], "verbs": [], "ctl": {}},
    {"id": "sensor.pm", "name": "Air quality", "group": "sensors", "words": "12 \u00b5g/m\u00b3", "tone": "info", "icon": "wind", "available": True, "can": [], "verbs": [], "ctl": {}},
    {"id": "lock.front", "name": "Front door lock", "group": "locks", "words": "Locked", "tone": "ok", "icon": "lock", "available": True, "can": [], "verbs": [], "ctl": {}},
    {"id": "light.garage", "name": "Garage lights", "group": "lights", "words": "Unavailable", "tone": "muted", "icon": "lightbulb", "available": False, "can": [], "verbs": [], "ctl": {}},
]


def topic(state="connected", tiles=TILES, **more):
    return {"state": state, "words": "%d things" % len(tiles), "detail": "", "shared": len(tiles), "controllable": 5, "paused": False, "picked": None, "tiles": tiles, **more}


@unittest.skipUnless(os.path.exists(BIN) and os.path.exists(MESA), "needs rustui/target/debug/lumatv-ui and Mesa")
class RustOnHome(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.h = base.RustHome('test_renders')

    def host(self, wait, steps, doc=None, **kw):
        """The harness with the smart-home layout. A picture is taken at the first frame drawn after its time, and on a loaded host the
        last frame can come before it: a run whose picture was not written is run again (up to three times)."""
        pictures = [step[i + 1].split(':', 1)[1] for step in steps for i in range(len(step) - 1) if step[i] == '--snap']
        for attempt in range(3):
            # the feeds again at 1.5 s: a text size or a material change rebuilds Home's model on a slow host after the first push
            again = ['--helper', '1500:onhome:' + json.dumps(doc or topic()), '--helper', '1500:weather:' + json.dumps(base.weather())]
            got = self.h.host(wait + 1500 * attempt, steps, layout=LAYOUT, feeds={"onhome": doc or topic()}, extra=again, **kw)
            if all(os.path.exists(p) and self.drawn(p) for p in pictures):
                break
            for p in pictures:
                if os.path.exists(p):
                    os.remove(p)
        # the mocks are JPEGs, as the cameras' are: a 1920 by 1080 PNG of a blurred wallpaper is 1.6 MB
        from PIL import Image
        os.makedirs(OUT, exist_ok=True)
        for p in pictures:
            if os.path.exists(p):
                Image.open(p).convert('RGB').save(os.path.join(OUT, os.path.basename(p)[:-4] + '.jpg'), quality=88)
        return got

    @staticmethod
    def drawn(path):
        """Whether the picture is of a drawn Home: a frame that came before the first draw is black."""
        from PIL import Image, ImageStat
        return ImageStat.Stat(Image.open(path).convert('L')).mean[0] > 12

    k = staticmethod(base.RustHome.k)
    hold = staticmethod(base.RustHome.hold)
    ev = staticmethod(base.RustHome.ev)

    @staticmethod
    def snap(ms, name):
        """A picture at `ms`, after an eval a moment before it (the harness draws a frame for the eval, so a picture is of a drawn Home)."""
        os.makedirs(SHOTS, exist_ok=True)
        return ['--eval', '%d:state' % (ms - 150), '--snap', '%d:%s' % (ms, os.path.join(SHOTS, name + '.png'))]

    def test_the_card_is_where_the_layout_put_it_and_ok_enters_it_and_the_keys_move_over_the_tiles(self):
        _, evals, _ = self.host(3000, [self.k(600, "down"), self.ev(1300), self.k(1400, "ok"), self.ev(1900), self.k(2000, "right", "down"), self.ev(2700)])
        self.assertEqual(evals[0]["widgetFocusId"], "smart")
        self.assertEqual(evals[0]["onhome"]["tiles"], len(TILES))
        self.assertEqual(evals[1]["focusLayer"], "widget", "OK entered the card")
        self.assertEqual(evals[2]["onhome"]["index"], 3, "right then down is the second row's right tile: the thermostat")

    def test_a_held_ok_opens_the_controls_of_the_tile_and_back_closes_them(self):
        _, evals, _ = self.host(4600, [self.k(600, "down"), self.k(1400, "ok"), self.hold(2000, "ok", 900), self.ev(3200), self.k(3300, "down", "right"), self.ev(3900),
                                       self.k(4000, "back"), self.ev(4400)])
        self.assertEqual(evals[0]["layer"], "onhome")
        self.assertEqual(evals[0]["onhome"]["sheet"], "light.kitchen")
        self.assertEqual(evals[1]["layer"], "onhome", "a value changed in the sheet leaves it open")
        self.assertEqual(evals[2]["layer"], "")
        self.assertEqual(evals[2]["focusLayer"], "widget", "Back from the sheet returns to the tiles")

    def test_the_card_and_the_sheet_read_clean_to_the_lint_at_both_sizes_and_looks_and_are_rendered(self):
        for mode in ("dark", "light"):
            suffix = "" if mode == "dark" else "-light"
            for scale, size in ((None, ""), (2, "-text200")):
                _, evals, _ = self.host(SNAP + 600, [self.k(600, "down"), ['--eval', '%d:lint' % (SNAP - 1500)], self.snap(SNAP, "card" + size + suffix)], mode=mode, scale=scale)
                lints = [e for e in evals if isinstance(e, list)]
                self.assertEqual(len(lints), 1)
                self.assertEqual([f for f in lints[0] if "smart" in str(f)], [], "the card at %s %s" % (mode, size))
                self.host(SNAP + 600, [self.k(600, "down"), self.k(1800, "ok"), self.k(2100, "right", "down"), self.snap(SNAP, "card-focus" + size + suffix)], mode=mode, scale=scale)
                _, evals, _ = self.host(SNAP + 600, [self.k(600, "down"), self.k(1800, "ok"), self.k(2100, "right", "down"), self.hold(2600, "ok", 900), ['--eval', '%d:lint' % (SNAP - 1500)],
                                                     self.snap(SNAP, "sheet-climate" + size + suffix)], mode=mode, scale=scale)
                lints = [e for e in evals if isinstance(e, list)]
                self.assertEqual(len(lints), 1)
                self.assertEqual([f for f in lints[0] if "onhome" in str(f)], [], "the sheet at %s %s" % (mode, size))
                self.host(SNAP + 600, [self.k(600, "down"), self.k(1800, "ok"), self.hold(2600, "ok", 900), self.snap(SNAP, "sheet-light" + size + suffix)], mode=mode, scale=scale)
            for state, name in (("unpaired", "card-unpaired"), ("waiting", "card-waiting"), ("empty", "card-empty")):
                self.host(SNAP + 600, [self.k(600, "down"), self.snap(SNAP, name + suffix)], doc=topic(state, [], shared=0, detail="x"), mode=mode)
            away = [dict(t, words="Not reachable", tone="muted", available=False, can=[], verbs=[]) for t in TILES[:6]]
            self.host(SNAP + 600, [self.k(600, "down"), self.snap(SNAP, "card-not-reachable" + suffix)], doc=topic("waiting", away), mode=mode)
        for name in ("card", "card-light", "card-text200", "card-focus", "sheet-climate", "sheet-light", "card-unpaired", "card-not-reachable"):
            self.assertTrue(os.path.exists(os.path.join(OUT, name + ".jpg")), name)


if __name__ == "__main__":
    unittest.main()
