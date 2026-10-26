"""Run with: python3 -m unittest tests.test_layout_tool

tools/layout.py, the owner's Home layout as the TV's tool (`lumatv layout`) and the workstation's push tool read and write it: the rules
a layout must meet (the layout editor's: every card has a unique id and a type, whole numbers, sits inside the grid, covers no other),
the writes (whole, atomic, `{}` for the way back) and the verb in tools/lumatv that calls it.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "layout.py")
sys.path.insert(0, os.path.join(ROOT, "tools"))
import layout  # noqa: E402

with open(os.path.join(ROOT, "payload", "theme", "layout.json")) as f:
    SHIPPED = json.load(f)


def card(id, col, row, span, rows, type="clock", **extra):
    return {"id": id, "type": type, "col": col, "row": row, "span": span, "rows": rows, **extra}


class Check(unittest.TestCase):
    def test_the_shipped_layout_is_a_layout(self):
        self.assertEqual(layout.check(SHIPPED, SHIPPED), [])
        self.assertEqual(layout.check({}, SHIPPED), [])

    def test_the_owners_two_halves_pass_and_a_card_over_another_does_not(self):
        halves = {"widgets": [card("clock", 0, 0, 6, 1), card("home", 0, 1, 6, 1, "homeassistant"), card("weather", 6, 0, 6, 2, "weather"),
                              card("dock", 0, 2, 12, 1, "dock")]}
        self.assertEqual(layout.check(halves, SHIPPED), [])
        halves["widgets"][2]["col"] = 5
        self.assertEqual(layout.check(halves, SHIPPED), ["weather covers clock", "weather covers home"])

    def test_a_hidden_card_takes_no_room(self):
        both = {"widgets": [card("a", 0, 0, 6, 1), card("b", 0, 0, 6, 1, hidden=True)]}
        self.assertEqual(layout.check(both, SHIPPED), [])

    def test_the_grid_the_file_does_not_name_is_the_shipped_one(self):
        # sixteen-row numbers on the shipped three pixel rows run off the grid; with the rows the file names they fit
        sixteen = {"widgets": [card("a", 0, 0, 4, 4), card("dock", 0, 9, 12, 7, "dock")]}
        self.assertTrue(any("outside the grid" in e for e in layout.check(sixteen, SHIPPED)))
        sixteen["grid"] = {"rows": 16}
        self.assertEqual(layout.check(sixteen, SHIPPED), [])

    def test_what_is_not_a_card_is_named(self):
        errors = layout.check({"widgets": [card("a", 0, 0, 4, 1), card("a", 4, 0, 4, 1), {"type": "clock", "col": 0, "row": 0, "span": 1, "rows": 1},
                                           card("b", -1, 0, 4, 1), card("c", 0, 0, 0, 1), card("d", 0, 0, 1.5, 1), "x", card("e", 10, 0, 4, 1)]}, SHIPPED)
        joined = "; ".join(errors)
        for needle in ("id a is used twice", "has no id", "b: col", "c: span", "d: span", "is not an object", "e is outside the grid"):
            self.assertIn(needle, joined)
        self.assertEqual(layout.check([], SHIPPED), ["the layout is not an object"])
        self.assertEqual(layout.check({"widgets": {}}, SHIPPED), ["widgets is not a list"])
        self.assertTrue(layout.check({"grid": {"cols": 0}}, SHIPPED))
        self.assertTrue(layout.check({"grid": {"rows": [0, 5]}}, SHIPPED))
        self.assertTrue(layout.check({"grid": {"rows": 99}}, SHIPPED))


class Files(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.path = os.path.join(self.dir.name, "layout.json")
        self.shipped = os.path.join(ROOT, "payload", "theme", "layout.json")

    def tearDown(self):
        self.dir.cleanup()

    def run_tool(self, *args):
        r = subprocess.run([sys.executable, TOOL, self.path, *args], capture_output=True, text=True)
        return r.returncode, (json.loads(r.stdout) if r.stdout.strip() else None)

    def test_put_writes_the_layout_whole_and_leaves_no_other_file(self):
        new = {"grid": {"rows": 16}, "widgets": [card("a", 0, 0, 6, 4)]}
        rc, out = self.run_tool("put", json.dumps(new), "--shipped", self.shipped)
        self.assertEqual((rc, out["ok"]), (0, True))
        with open(self.path) as f:
            self.assertEqual(json.load(f), new)
        self.assertEqual(os.listdir(self.dir.name), ["layout.json"], "a temporary or a backup file was left")

    def test_put_of_a_bad_layout_changes_nothing_and_says_why(self):
        self.run_tool("put", json.dumps({"widgets": [card("a", 0, 0, 4, 1)]}), "--shipped", self.shipped)
        before = open(self.path).read()
        rc, out = self.run_tool("put", json.dumps({"widgets": [card("a", 0, 0, 4, 1), card("b", 0, 0, 4, 1)]}), "--shipped", self.shipped)
        self.assertEqual((rc, out["ok"]), (1, False))
        self.assertIn("b covers a", out["error"])
        self.assertEqual(open(self.path).read(), before)
        rc, out = self.run_tool("put", "not json")
        self.assertEqual((rc, out["ok"]), (1, False))

    def test_set_merges_objects_and_replaces_lists(self):
        self.run_tool("put", json.dumps({"grid": {"rows": 16}, "safe": {"x": 48, "y": 44}, "widgets": [card("a", 0, 0, 4, 4)]}), "--shipped", self.shipped)
        rc, out = self.run_tool("set", json.dumps({"safe": {"x": 64}, "widgets": [card("b", 0, 0, 4, 4)]}), "--shipped", self.shipped)
        self.assertEqual(rc, 0)
        self.assertEqual(out["layout"], {"grid": {"rows": 16}, "safe": {"x": 64, "y": 44}, "widgets": [card("b", 0, 0, 4, 4)]})

    def test_check_writes_nothing_and_reset_is_the_way_back(self):
        rc, out = self.run_tool("check", json.dumps({"widgets": [card("a", 0, 0, 4, 1)]}), "--shipped", self.shipped)
        self.assertEqual((rc, out["ok"]), (0, True))
        self.assertFalse(os.path.exists(self.path))
        self.run_tool("put", json.dumps({"widgets": [card("a", 0, 0, 4, 1)]}), "--shipped", self.shipped)
        rc, out = self.run_tool("reset")
        self.assertEqual((rc, out["layout"]), (0, {}))
        self.assertEqual(json.load(open(self.path)), {})

    def test_get_reads_the_file_and_a_missing_one_is_empty(self):
        self.assertEqual(self.run_tool("get")[1]["layout"], {})
        self.run_tool("put", json.dumps({"widgets": [card("a", 0, 0, 4, 1)]}), "--shipped", self.shipped)
        self.assertEqual(self.run_tool("get")[1]["layout"]["widgets"][0]["id"], "a")


class Verb(unittest.TestCase):
    def test_the_tool_has_a_layout_verb_that_calls_this_script_and_the_package_carries_it(self):
        with open(os.path.join(ROOT, "tools", "lumatv")) as f:
            text = f.read()
        self.assertIn('LAYOUT_TOOL="$APPDIR/tools/layout.py"', text)
        self.assertIn("  layout) verb_layout ", text)
        self.assertIn('--shipped "$THEME_SRC/layout.json"', text)
        with open(os.path.join(ROOT, "tools", "build-ipk.sh")) as f:
            build = f.read()
        self.assertIn("tools/layout.py\n", build)
        self.assertIn('cp tools/layout.py "$DATA_ROOT/tools/"', build)


if __name__ == "__main__":
    unittest.main()
