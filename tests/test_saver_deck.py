"""Run with: cd tests && python3 -m unittest test_saver_deck   (needs Xvfb and PyQt5; skipped without them; the library's own cases: bun test tests/artdeck.test.js)

The art saver's shuffle bag kept across restarts (payload/compositor/lumatv/ArtDeck.js, panels/Screensaver.qml): a small file in the data
directory, a deck for each collection of prints. The QML host stands in for a restart: the file is written before the saver is built, the way the
last run left it, and read back when the run is over.
"""
import json
import os
import tempfile
import time
import unittest

from test_plugins_surfaces import AWAKE, BYNAME, Surfaces, build, js, probe, probes, publish, REPO
from test_settings import READY
from xdisplay import DISPLAY, HAVE_QT

with open(os.path.join(REPO, "payload", "compositor", "lumatv", "art", "index.json")) as f:
    IDS = [p["id"] for p in json.load(f)["pictures"]]
SNAP = "var d = JSON.parse(s.snapshot()); return {up: d.up, shown: d.art.shown, bag: d.art.bag, last: d.art.last, key: d.art.key, current: d.art.current, timers: d.timers};"
FILE = os.path.join("theme", "saver-deck.json")


def deck(bag, last="", known=None, at=None):
    return {"at": at or int(time.time() * 1000), "known": IDS if known is None else known, "bag": bag, "last": last}


def doc_of(d, key="art/"):
    """The file's document with `d` as the deck of the collection `key`."""
    return {"schema": 1, "decks": {key: d}}


def seed(at, text):
    """JS: the file as the last run left it (a TextFile, as the saver's own)."""
    return js(at, "var f = Qt.createQmlObject('import QtQuick 2.4\\nimport TextFile 1.0\\nTextFile {}', root); f.name = root.themeDirUrl + 'saver-deck.json'; f.write(%s);" % json.dumps(text))


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class SaverDeck(Surfaces):
    MINE = Surfaces.MINE + ("ArtDeck.js",)
    THEME = {"screensaver": {"style": "art", "crossfadeMs": 300, "art": {"keepOrder": True}}}
    PROPS = {"z": 95, "objectName": "under-test", "periodMs": 2200, "leadMs": 800}

    def run_saver(self, doc=None, theme=None, props=None, probes_at=(), extra=(), wait=3000, allow=()):
        """The saver comes up (with `doc` in the file when given); returns the probes and the file as the run left it.
        It is brought up by `forced`, not by the power state: Home's own saver (in Layers) answers the power state, and two savers would
        share the one file."""
        at = READY - 900
        d = tempfile.mkdtemp(prefix="deck-")
        self.addCleanup(__import__("shutil").rmtree, d, True)
        args = [*publish(300, "screen", AWAKE)]
        if doc is not None:
            args += seed(at - 700, doc if isinstance(doc, str) else json.dumps(doc))
        args += [*build(at, "Screensaver.qml", dict(self.PROPS, **(props or {}))),
                 *js(at + 100, BYNAME + "byName(root, 'under-test').forced = true;")]
        for i, ms in enumerate(probes_at):
            args += probe(at + ms, "p%d" % i, SNAP)
        rc, out = self.run_host(theme or self.THEME, *args, *extra, "--data", os.path.join(d, "data"), "--wait", str(at + wait))
        self.assertEqual(self.own_warnings(out, allow=allow), [], out[-3000:])
        path = os.path.join(d, "data", FILE)
        text = open(path).read() if os.path.exists(path) else None
        return [probes(out)["p%d" % i] for i in range(len(probes_at))], text

    # ---- what is saved

    def test_each_print_dealt_is_saved_with_what_is_left_of_the_round(self):
        (p,), text = self.run_saver(props={"periodMs": 20000}, probes_at=(1200,))
        self.assertEqual(p["key"], "art/", "the collection is the folder of the index")
        self.assertEqual(len(p["bag"]), len(IDS) - 1)
        self.assertNotIn(p["shown"], p["bag"])
        doc = json.loads(text)
        self.assertEqual(doc["schema"], 1)
        d = doc["decks"]["art/"]
        self.assertEqual((d["last"], d["bag"]), (p["shown"], p["bag"]), "the file says what the saver says")
        self.assertEqual(sorted(d["known"]), sorted(IDS))
        self.assertLess(abs(d["at"] - time.time() * 1000), 120000)
        self.assertLess(len(text), 1000, "a small file")

    def test_in_order_nothing_is_written(self):
        (p,), text = self.run_saver(theme={"screensaver": {"style": "art", "crossfadeMs": 300, "art": {"order": "sequence", "keepOrder": True}}}, probes_at=(1200,))
        self.assertEqual(p["shown"], IDS[0])
        self.assertIsNone(text, "only the shuffle has a bag")

    def test_by_default_the_order_is_kept(self):
        (p,), text = self.run_saver(theme={"screensaver": {"style": "art", "crossfadeMs": 300}}, props={"periodMs": 20000}, probes_at=(1200,))
        self.assertEqual(json.loads(text)["decks"]["art/"]["last"], p["shown"], "kept unless the owner turns `art.keepOrder` off")

    def test_turned_off_nothing_is_written(self):
        (p,), text = self.run_saver(theme={"screensaver": {"style": "art", "crossfadeMs": 300, "art": {"keepOrder": False}}}, probes_at=(1200,))
        self.assertEqual(len(p["bag"]), len(IDS) - 1, "the shuffle itself is as it was")
        self.assertIsNone(text)

    # ---- what a restart does with it

    def test_a_restart_goes_on_with_the_round_where_it_was(self):
        order = ["view-of-delft", "starry-night", "great-wave"]
        ps, text = self.run_saver(doc_of(deck(order, last="sierra-nevada")), probes_at=[900 + 450 * i for i in range(19)], wait=9800)
        shown = [p["shown"] for p in ps]
        seen = [s for i, s in enumerate(shown) if s and (i == 0 or s != shown[i - 1])]
        self.assertEqual(seen[:3], order, "the round that was left, in its order: %s" % shown)
        self.assertEqual(ps[0]["bag"], order[1:], "restored, and the first print taken from it")
        self.assertEqual(ps[0]["last"], "view-of-delft")
        self.assertGreaterEqual(len(seen), 4, shown)
        self.assertNotEqual(seen[3], "great-wave", "the next round does not begin with the print the last one ended on")
        self.assertIn(json.loads(text)["decks"]["art/"]["last"], IDS)

    def test_a_print_that_is_no_longer_in_the_index_or_has_no_file_is_dropped(self):
        doc = doc_of(deck(["view-of-delft", "no-longer-there", "starry-night", "great-wave"], last="ghost"))
        (p,), text = self.run_saver(doc, extra=("--remove", "art/view-of-delft.jpg"), probes_at=(1200,), allow=("view-of-delft.jpg",))
        self.assertEqual(p["shown"], "starry-night", "the print whose file is gone is passed over: it is not even tried")
        self.assertEqual(p["bag"], ["great-wave"])
        self.assertEqual(p["last"], "starry-night")

    def test_a_print_that_has_come_since_is_dealt_into_the_round(self):
        known = [i for i in IDS if i != "sierra-nevada"]
        (p,), text = self.run_saver(doc_of(deck(["starry-night"], last="great-wave", known=known)), probes_at=(1200,))
        left = set(p["bag"]) | {p["shown"]}
        self.assertEqual(left, {"starry-night", "sierra-nevada"}, "the new print is in this round; the ones already shown are not")

    def test_a_file_that_is_not_ours_starts_the_round_over_and_is_replaced(self):
        for junk in ("this is not json", '{"schema":9,"decks":{}}'):
            (p,), text = self.run_saver(junk, props={"periodMs": 20000}, probes_at=(1200,))
            self.assertEqual(len(p["bag"]), len(IDS) - 1, junk)
            self.assertEqual(json.loads(text)["decks"]["art/"]["last"], p["shown"], junk)

    def test_another_collections_deck_is_left_alone(self):
        other = deck(["x", "y"], last="x", known=["x", "y", "z"], at=12345)
        (p,), text = self.run_saver({"schema": 1, "decks": {"file:///media/usb/prints/": other}}, probes_at=(1200,))
        decks = json.loads(text)["decks"]
        self.assertEqual(decks["file:///media/usb/prints/"], other)
        self.assertIn("art/", decks)

    def test_a_collection_elsewhere_has_its_own_deck(self):
        with tempfile.TemporaryDirectory() as tmp:
            index = os.path.join(tmp, "index.json")
            pics = [{"id": n, "file": "../icons/lucide/%s.png" % n} for n in ("bell", "camera", "clock")]
            with open(index, "w") as f:
                json.dump({"pictures": [dict(q, licence="Public domain", artist="A", title="T", year="1900", width=64, height=64) for q in pics]}, f)
            mine = deck(["clock"], last="bell", known=["bell", "camera", "clock"])
            (p,), text = self.run_saver({"schema": 1, "decks": {"art/": deck(["great-wave"], "x"), "file://%s/" % tmp: mine}},
                                        props={"indexUrl": "file://" + index, "periodMs": 20000}, probes_at=(1200,))
            self.assertEqual((p["shown"], p["key"]), ("clock", "file://%s/" % tmp))
            decks = json.loads(text)["decks"]
            self.assertEqual(decks["art/"]["bag"], ["great-wave"], "the shipped prints' deck is as it was")
            self.assertEqual(decks["file://%s/" % tmp]["last"], "clock")

    def test_nothing_is_read_or_written_while_the_saver_is_down(self):
        (p,), text = self.run_saver(doc_of(deck(["great-wave"], last="x")), props={"visible": False}, probes_at=(1200,))
        self.assertEqual((p["up"], p["shown"], p["bag"], p["timers"]), (False, "", [], False))
        self.assertEqual(json.loads(text)["decks"]["art/"]["bag"], ["great-wave"])


if __name__ == "__main__":
    unittest.main()
