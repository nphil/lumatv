"""Run with: python3 -m unittest discover -s tests -p test_widgets.py   (needs Xvfb and PyQt5; skipped without them)

Home's widgets through the offline host: where the cards are drawn (the shipped layout is where it always was), what hiding a card does,
Settings' Home widgets page (a card Off, Small, Half or Wide, a size that does not fit refused and said, Reset to default), the layout
editor (opened from the hold of OK on a card, a card picked up, moved and sized by the cell, a move that would overlap turned down,
Save writing the owner's layout.json, Back asking, Discard putting Home as it was), and what each of those writes: the owner's file
holds only what differs from the shipped layout. The pure model has its own tests (tests/layoutgrid.test.js, run with bun). Timing
and Qt 5.12 are the set's to say.

Every time below is in ms after READY, when Home has presented and built its residents.
"""
import ast
import json
import os
import re
import tempfile
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

READY = 7000
FIND = ("function find(p, k) { if (p[k] !== undefined) return p; var c = p.children; "
        "for (var i = 0; i < c.length; i++) { var f = find(c[i], k); if (f) return f } return null } ")


def ed(expr):
    """An expression over the layout editor's item `ed` (the one with a toast), as a function call the host can evaluate."""
    return "(function(){ " + FIND + "var ed = find(root, 'toastText'); return " + expr + " })()"


def pg(expr):
    """The same over Settings' Home widgets page `pg` (the one with a `shape`)."""
    return "(function(){ " + FIND + "var pg = find(root, 'shape'); return " + expr + " })()"


# the files this slice owns: what other surfaces warn about while they are being written is theirs
MINE = re.compile(r"LayoutEdit|LayoutGrid|PageWidgets|ClockWidget|WeatherWidget|WeatherDayRow|RangeBar|NewsWidget|HomeAssistantWidget|HomeLayout|Widget\.qml|"
                  r"SyntaxError|ReferenceError: (Grid|HomeLayout)|binding loop.*(Layout|Widget)")
RECTS = ("JSON.stringify(root.nonDockWidgets.map(function(e) { var r = root.cellRect(e); return [e.id, r.x, r.y, r.width, r.height] })"
         ".concat([['dock'].concat((function(r) { return [r.x, r.y, r.width, r.height] })(root.cellRect(root.dockEntry)))]))")
LAYOUT = "JSON.stringify((root.layout.widgets || []).map(function(e) { return [e.id, e.col, e.row, e.span, e.rows, e.hidden === true ? 1 : 0] }))"
STATE = "[root.editing, root.layerOpen].join('/')"
ENTER = "Qt.Key_Return"


def card(id, col, row, span, rows, type="clock", **extra):
    return {"id": id, "type": type, "col": col, "row": row, "span": span, "rows": rows, **extra}


def sixteen(*cards):
    return json.dumps({"grid": {"rows": 16}, "widgets": list(cards)})


# the shipped arrangement in sixteen rows with the news card hidden (a free hole in the middle), and the dock under
FREE = sixteen(card("clock", 0, 0, 4, 4), card("home", 0, 4, 4, 5, "homeassistant", title="At home"), card("news", 4, 0, 5, 9, "news", hidden=True),
               card("weather", 9, 0, 3, 9, "weather"), card("dock", 0, 9, 12, 7, "dock", tile=120, gap=16, padY=40, fade=110))


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Widgets(HostCase):
    def run_host(self, steps, wait, data=None, layout=None):
        """`steps` are lists of host arguments (built by `at`, `keys_at` and `js_at`). Returns {label: the eval's text}."""
        args = [a for s in steps for a in s]
        if layout:
            args += ["--layout", layout]
        rc, out = self.host(*args, "--wait", str(READY + wait), data=data)
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln and MINE.search(ln)]
        self.assertEqual(stray, [], "\n".join(stray) + "\n" + out[-1500:])
        got = {}
        for line in out.splitlines():
            m = re.match(r"\[eval\] \((.*), (True|False)\)$", line)
            if not m:
                continue
            v = ast.literal_eval(m.group(1))
            lab = re.match(r"^([A-Za-z0-9_]+)=(.*)$", v, re.S) if isinstance(v, str) else None
            if lab:
                got[lab.group(1)] = lab.group(2)
        return got

    def at(self, ms, label, js):
        """An `--eval` that comes back as `label=<value>` (text; JSON for anything bigger)."""
        return ["--eval", "%d:(function(){ return %s + (%s) })()" % (READY + ms, json.dumps(label + "="), js)]

    def js_at(self, ms, js):
        return ["--eval", "%d:%s" % (READY + ms, js)]

    def keys_at(self, ms, *names):
        return self.keys(READY + ms, *names)

    # ---- where the cards are
    def test_the_shipped_layout_is_drawn_where_it_always_was(self):
        got = self.run_host([self.at(-2000, "rects", RECTS)], 200)
        self.assertEqual(json.loads(got["rects"]), [["clock", 64, 134, 584, 222], ["home", 64, 376, 584, 238], ["news", 668, 134, 735, 480],
                                                    ["weather", 1423, 134, 433, 480], ["dock", 64, 634, 1792, 392]])

    def test_a_hidden_card_is_not_drawn_and_takes_no_focus_and_up_from_a_card_with_nothing_over_it_reaches_the_bar(self):
        got = self.run_host([self.at(-2000, "ids", "JSON.stringify(root.nonDockWidgets.map(function(e) { return e.id }))"),
                             self.at(-2000, "newsIndex", "root.widgetIndexById('news')"),
                             # the card at the top left has no card above it; the home card under it has; nothing is under the home card
                             self.at(-2000, "tops", "JSON.stringify(root.nonDockWidgets.map(function(e) { return root.isTopRow(e) }))"),
                             self.at(-2000, "bottoms", "JSON.stringify(root.nonDockWidgets.map(function(e) { return root.isBottomRow(e) }))")], 200, layout=FREE)
        self.assertEqual(json.loads(got["ids"]), ["clock", "home", "weather"])
        self.assertEqual(got["newsIndex"], "-1")
        self.assertEqual(json.loads(got["tops"]), [True, False, True])
        self.assertEqual(json.loads(got["bottoms"]), [False, True, True])

    # ---- Settings' Home widgets page
    def page(self):
        """Opens Settings, takes the rail to Home & look and opens Home widgets from the Home tab's first row."""
        return [["--eval", "1500:root.openSettings()"], self.keys_at(600, *(["down"] * 5)), self.keys_at(1500, "right", "up", "right", "down"),
                self.keys_at(2300, "ok")]

    def test_a_card_that_does_not_fit_is_refused_and_named_then_hidden_and_made_wide_and_reset_writes_an_empty_file(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            steps = self.page() + [
                self.at(3000, "open", "root.settingsOpen"),
                # the weather card is the fourth row, three columns wide on the right edge; Wide there is the news card's place too
                self.keys_at(3300, "down", "down", "down", "right", "right", "right", "ok"),
                self.at(4200, "refused", "root.lastLayoutText"),
                self.at(4200, "notice", pg("JSON.stringify(pg ? pg.notice : null)")),
                # the news card off (the third row), then the weather card Wide
                self.keys_at(4500, "up", "ok"),
                self.at(5300, "hidden", LAYOUT),
                self.keys_at(5500, "down", "right", "right", "right", "ok"),
                self.at(6300, "wide", LAYOUT),
                self.at(6300, "file", "root.lastLayoutText"),
                # Edit on Home is under the four cards, then Reset to default: Right to Reset, OK
                self.keys_at(6500, "down", "down", "right", "ok"),
                self.at(7400, "reset", "root.lastLayoutText"),
                self.at(7400, "after", LAYOUT)]
            got = self.run_host(steps, 7900, data=data)
            self.assertEqual(got["open"], "true")
            self.assertEqual(json.loads(got["refused"]), {}, "a refused change wrote the file")
            self.assertIn("News is in the way", got["notice"])
            hidden = {r[0]: r for r in json.loads(got["hidden"])}
            self.assertEqual(hidden["news"][5], 1, "the news card was not hidden")
            wide = {r[0]: r for r in json.loads(got["wide"])}
            self.assertEqual(wide["weather"][1:5], [4, 0, 8, 2], "the weather card did not grow to the left to eight columns")
            f = json.loads(got["file"])
            self.assertEqual(sorted(f), ["widgets"], "the owner's file holds more than what changed: %s" % sorted(f))
            self.assertEqual(json.loads(got["reset"]), {})
            self.assertEqual([r[5] for r in json.loads(got["after"])], [0, 0, 0, 0, 0])
            with open(os.path.join(data, "theme", "layout.json")) as fh:
                self.assertEqual(json.load(fh), {})
            self.assertEqual(sorted(n for n in os.listdir(os.path.join(data, "theme")) if n.startswith("layout")), ["layout.json"], "a copy of the replaced file was kept")

    # ---- the layout editor
    def test_the_editor_picks_up_a_card_moves_it_by_the_cell_turns_a_move_down_and_saves_the_owners_layout(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            steps = [self.at(-1500, "before", RECTS),
                     self.js_at(-1000, "root.editHome('clock')"),
                     self.at(-600, "editing", "root.editing"),
                     self.at(-600, "focus", "root.focusLayer"),
                     # the clock: OK picks it up, Right moves it a column (the news card is hidden), Down is under the home card: turned down
                     self.keys_at(-200, "ok", "right", "down"),
                     self.at(500, "moved", LAYOUT),
                     self.at(500, "toast", ed("ed.toastText")),
                     # OK turns to the size, Right is a column wider, OK puts it down
                     self.keys_at(800, "ok", "right", "ok"),
                     self.at(1500, "sized", LAYOUT),
                     self.at(1500, "mode", ed("ed.mode")),
                     # up to the bar (Add), right to Remove, Undo, Cancel, Save; OK saves
                     self.keys_at(1800, "up", "right", "right", "right", "right", "ok"),
                     self.at(2700, "after", LAYOUT),
                     self.at(2700, "editingAfter", "root.editing"),
                     self.at(2700, "focusAfter", "root.focusLayer"),
                     self.at(2700, "file", "root.lastLayoutText")]
            got = self.run_host(steps, 3300, data=data, layout=FREE)
            self.assertEqual(got["editing"], "true")
            self.assertEqual(got["focus"], "edit", "Home's own focus stayed on a card while the editor is up")
            moved = {r[0]: r for r in json.loads(got["moved"])}
            self.assertEqual(moved["clock"][1:5], [1, 0, 4, 4], "Right did not move the clock a column, or Down was not turned down")
            self.assertIn("At home is in the way", got["toast"])
            sized = {r[0]: r for r in json.loads(got["sized"])}
            self.assertEqual(sized["clock"][1:5], [1, 0, 5, 4])
            self.assertEqual(got["mode"], "select")
            self.assertEqual(got["editingAfter"], "false")
            self.assertEqual(got["focusAfter"], "widgets")
            saved = {r[0]: r for r in json.loads(got["after"])}
            self.assertEqual(saved["clock"][1:5], [1, 0, 5, 4])
            with open(os.path.join(data, "theme", "layout.json")) as fh:
                on_disk = json.load(fh)
            self.assertEqual(on_disk, json.loads(got["file"]))
            self.assertEqual(sorted(on_disk), ["grid", "widgets"], "the file holds what is shipped too")
            self.assertEqual(on_disk["grid"], {"rows": 16})
            self.assertEqual([c["id"] for c in on_disk["widgets"]], ["clock", "home", "news", "weather", "dock"])
            self.assertTrue(next(c for c in on_disk["widgets"] if c["id"] == "news")["hidden"])

    def test_back_asks_when_something_changed_and_discard_puts_home_as_it_was_and_the_bar_removes_undoes_and_adds(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            steps = [self.js_at(-1000, "root.editHome('weather')"),
                     # the weather card: pick up, left (the news card is hidden), put down by two OKs; Back with it put down asks
                     self.keys_at(-200, "ok", "left", "ok", "ok", "back"),
                     self.at(900, "asked", "root.layerOpen"),
                     self.at(900, "movedLive", LAYOUT),
                     # Keep editing is the first answer and what Back is: still editing
                     self.keys_at(1100, "back"),
                     self.at(1900, "keep", STATE),
                     # Back again, then to Discard (Keep editing, Save, Discard), OK
                     self.keys_at(2100, "back"),
                     self.keys_at(2900, "right", "right", "ok"),
                     self.at(3700, "discarded", STATE),
                     self.at(3700, "restored", LAYOUT),
                     self.at(3700, "file", "root.lastLayoutText"),
                     # a second session: Remove takes the selected card away, Undo brings it back, Add lists the types
                     self.js_at(4100, "root.editHome('clock')"),
                     self.keys_at(4700, "up", "right"),
                     self.keys_at(5200, "ok"),
                     self.at(5900, "removed", LAYOUT),
                     self.keys_at(6100, "right", "ok"),
                     self.at(6900, "undone", LAYOUT),
                     self.keys_at(7100, "left", "left", "ok"),
                     self.at(7900, "list", ed("ed.zone")),
                     # Back closes the list; Right, Right (Undo has nothing to undo: it is passed) is Cancel, OK leaves: nothing changed, nothing asked
                     self.keys_at(8100, "back", "right", "right", "ok"),
                     self.at(9100, "left", STATE)]
            got = self.run_host(steps, 9700, data=data, layout=FREE)
            self.assertEqual(got["asked"], "confirm", "Back after a change did not ask")
            weather = {r[0]: r for r in json.loads(got["movedLive"])}["weather"]
            self.assertEqual(weather[1], 8, "the weather card was not moved a column left")
            self.assertEqual(got["keep"], "true/", "Keep editing (and Back, which is its answer) left the editor")
            self.assertEqual(got["discarded"], "false/")
            restored = {r[0]: r for r in json.loads(got["restored"])}
            self.assertEqual(restored["weather"][1:5], [9, 0, 3, 9], "Discard left the moved card where it was put")
            self.assertEqual(got["file"], FREE, "Discard wrote the file")
            self.assertNotIn("clock", [r[0] for r in json.loads(got["removed"])], "Remove did not take the clock card away")
            self.assertIn("clock", [r[0] for r in json.loads(got["undone"])], "Undo did not bring it back")
            self.assertEqual(got["list"], "add", "the Add button did not open the list of types")
            self.assertEqual(got["left"], "false/", "Cancel with nothing changed asked or stayed")

    def test_ok_on_save_in_the_save_changes_prompt_writes_the_layout_and_leaves_the_editor(self):
        with tempfile.TemporaryDirectory() as d:
            data = os.path.join(d, "data")
            steps = [self.js_at(-1000, "root.editHome('weather')"),
                     # the weather card: pick up, left, put down; Back asks; Right is Save, OK answers it
                     self.keys_at(-200, "ok", "left", "ok", "ok", "back"),
                     self.at(900, "asked", "root.layerOpen"),
                     self.keys_at(1100, "right", "ok"),
                     self.at(1900, "after", STATE),
                     self.at(1900, "layout", LAYOUT),
                     self.at(1900, "file", "root.lastLayoutText")]
            got = self.run_host(steps, 2500, data=data, layout=FREE)
            self.assertEqual(got["asked"], "confirm")
            self.assertEqual(got["after"], "false/", "OK on Save left the editor up or the prompt open")
            weather = {r[0]: r for r in json.loads(got["layout"])}["weather"]
            self.assertEqual(weather[1], 8, "OK on Save did not keep the move")
            with open(os.path.join(data, "theme", "layout.json")) as fh:
                saved = {c["id"]: c for c in json.load(fh)["widgets"]}
            self.assertEqual(saved["weather"]["col"], 8, "OK on Save wrote nothing")

    def test_holding_ok_on_a_card_opens_the_editor_and_its_release_is_not_an_ok(self):
        steps = [self.js_at(-1500, "root.focusLayer = 'widgets'; root.widgetFocusId = 'weather'"),
                 self.js_at(-1000, "root.key(%s, true, false, 0)" % ENTER),
                 self.at(-500, "early", "root.editing"),
                 self.at(400, "held", "[root.editing, root.focusLayer].join('/')"),
                 self.js_at(500, "root.key(%s, true, true, 0)" % ENTER),                  # the auto-repeat of a key held down
                 self.js_at(700, "root.key(%s, false, false, 0)" % ENTER),
                 self.at(1200, "released", ed("[root.editing, ed.mode, ed.sel].join('/')"))]
        got = self.run_host(steps, 1700)
        self.assertEqual(got["early"], "false", "the press opened the editor before the hold")
        self.assertEqual(got["held"], "true/edit")
        self.assertEqual(got["released"], "true/select/weather", "the release of the hold picked the card up")


if __name__ == "__main__":
    unittest.main()
