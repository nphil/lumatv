"""Run with: cd tests && python3 -m unittest test_rust_home   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The Rust shell's Home (rustui/ui/src/home, the `home` kind) on the host harness (`lumatv-ui --host`: the shell's logic against the
engine in its own process on Mesa), driven by the harness's own script (topics, luna answers, the helper's feeds, keys, the pointer),
on the cases the QML's tests state (test_widgets.py, test_profiles_host.py, test_shell_surfaces.py): the shipped layout is drawn where
it always was; the dock is the person's own; the focus walks the status bar, the cards and the dock; the layout editor picks a card up,
moves it by the cell, turns a move down, saves the owner's layout.json and asks on Back; a held OK on a card opens the editor; the profile
chip shows the person in front of the TV, the picker switches and a locked profile asks a PIN; a switch on the topic closes what was
open; hover is a cue and a click selects. The renders go beside the mocks (docs/mocks/host-rust/home/), dark and light, 100 % and 200 %.
"""
import json
import os
import re
import shutil
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
BIN = os.path.join(ROOT, 'rustui', 'target', 'debug', 'lumatv-ui')
MESA = '/usr/lib/x86_64-linux-gnu/libEGL.so.1'
OUT = os.path.join(ROOT, 'docs', 'mocks', 'host-rust', 'home')
NOW = int(time.time() * 1000)
HOME = {"app": "com.webos.app.home", "home": True, "covered": False}
LP_TAG = 300
TOAST_TAG = 0x5100_0100  # panels/notelog.rs FEED_TAG: LG's toast feed


def launch_points():
    """The offline host's make-believe launch points (tests/qmlhost/stubs/WebOSServices/FakeApps.js), with the mocks' icons."""
    with open(os.path.join(HERE, 'qmlhost', 'stubs', 'WebOSServices', 'FakeApps.js')) as fh:
        src = fh.read()
    icons = os.path.join(ROOT, 'docs', 'mocks', 'img', 'apps')
    out = []
    for id, title, icon, opts in re.findall(r'\["([^"]+)", "([^"]+)", "([^"]+)", \{([^}]*)\}\]', src):
        o = dict((k, v.strip().strip('"')) for k, v in re.findall(r'(\w+): ([^,]+)', opts))
        path = os.path.join(ROOT, 'assets', 'icon160.png') if icon.startswith('@') else os.path.join(icons, icon + '.png')
        out.append({"id": id, "launchPointId": id + "_default", "title": title, "icon": path, "largeIcon": path, "iconColor": o.get('color', ''),
                    "hidden": o.get('hidden') == 'true', "params": {}, "unmovable": o.get('unmovable') == 'true'})
    return out


def weather():
    days = [("Today", "sun", 71, 51), ("Mon", "cloud", 73, 51), ("Tue", "rain", 62, 48), ("Wed", "partly", 66, 56), ("Thu", "drizzle", 59, 44), ("Fri", "sun", 68, 42), ("Sat", "storm", 74, 55)]
    return {"doc": {"schema": 1, "fetchedAt": NOW, "display": {"place": "Atlanta", "temp": "61\u00b0", "cond": "Clear", "stats": "Feels like 57\u00b0 \u00b7 53% humidity \u00b7 6 mph", "nowKind": "moon",
                                                          "days": [{"day": d, "kind": k, "hi": "%d\u00b0" % hi, "lo": "%d\u00b0" % lo} for d, k, hi, lo in days]}}, "state": "fresh"}


def news():
    return {"doc": {"schema": 1, "fetchedAt": NOW, "items": [
        {"tag": "WORLD", "src": "BBC News", "title": "Denmark says Russian warship fired flares at military helicopter",
         "summary": "The Danish prime minister has accused Russia of \"reckless\" behaviour, while the Kremlin is yet to comment. The helicopter was on a routine patrol over the Baltic when the flares were fired, the Danish defence ministry said.",
         "link": "https://example.invalid/a", "pubMs": NOW - 1800000},
        {"tag": "U.S.", "src": "BBC News", "title": "A second story", "summary": "", "pubMs": NOW - 7200000}]}, "state": "fresh"}


def card(id, title, facts, icon, role, actions=()):
    return {"id": id, "revision": 1, "card": {"id": id, "size": "half", "content": {"type": "facts", "title": title, "icon": icon, "role": role, "facts": [{"label": a, "value": b} for a, b in facts]}, "actions": list(actions)}}


HOME_CARDS = {"items": [card("living", "Living room", [("Lights", "3 on")], "base.icons/lightbulb", "warning", [{"action": "off", "label": "Turn off"}]),
                        card("climate", "72\u00b0 cooling", [("Set to", "70\u00b0")], "base.icons/sun", "info"), card("door", "Front door", [("State", "Locked")], "base.icons/lock", "positive")]}
CAMERAS = {"state": "ready", "cameras": [{"id": "front", "name": "Front door", "room": "Porch", "online": True}, {"id": "back", "name": "Back yard", "room": "Garden", "online": False, "since": NOW - 200000}]}


def profiles(active="nitin", can_admin=True, locked=("isabel",), at=1):
    people = [("nitin", "Nitin", "#A79BF5", True), ("isabel", "Isabel", "#EB6F92", False), ("mia", "Mia", "#F6C177", False)]
    return {"ready": True, "active": active, "owner": "nitin", "canAdmin": can_admin, "pickerOnWake": False, "startWith": "last", "max": 6, "switchedAt": at,
            "profiles": [{"id": i, "name": n, "initial": n[0], "color": c, "avatar": {"kind": "initial"}, "avatarFile": None, "owner": o, "active": i == active, "locked": i in locked, "firstRun": False} for i, n, c, o in people]}


FREE = {"grid": {"rows": 16}, "widgets": [
    {"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 4, "rows": 4}, {"id": "home", "type": "homeassistant", "col": 0, "row": 4, "span": 4, "rows": 5, "title": "At home"},
    {"id": "news", "type": "news", "col": 4, "row": 0, "span": 5, "rows": 9, "hidden": True}, {"id": "weather", "type": "weather", "col": 9, "row": 0, "span": 3, "rows": 9},
    {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]}
HALVES = {"grid": {"rows": 16}, "widgets": [
    {"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 6, "rows": 4}, {"id": "home", "type": "homeassistant", "col": 0, "row": 4, "span": 6, "rows": 5, "title": "At home"},
    {"id": "weather", "type": "weather", "col": 6, "row": 0, "span": 6, "rows": 9},
    {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]}
CAMERAS_LAYOUT = {"grid": {"rows": 16}, "widgets": [
    {"id": "clock", "type": "clock", "col": 0, "row": 0, "span": 4, "rows": 4}, {"id": "cameras", "type": "cameras", "col": 0, "row": 4, "span": 4, "rows": 5},
    {"id": "news", "type": "news", "col": 4, "row": 0, "span": 5, "rows": 9}, {"id": "weather", "type": "weather", "col": 9, "row": 0, "span": 3, "rows": 9},
    {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]}


@unittest.skipUnless(os.path.exists(BIN) and os.path.exists(MESA), "needs rustui/target/debug/lumatv-ui and Mesa")
class RustHome(unittest.TestCase):
    def host(self, wait, steps=(), layout=None, feeds=None, mode=None, scale=None, data=None, theme=None, kinds='home', extra=()):
        """Runs the harness with Home in front, the launch points answered and the feeds published at 100 ms; `steps` are extra
        arguments (`k`, `snap`, `ev`). Returns (the output, the evals by label, the data directory)."""
        data = data or tempfile.mkdtemp(prefix='rusthome-')
        os.makedirs(os.path.join(data, 'theme'), exist_ok=True)
        shutil.copy(os.path.join(ROOT, 'payload', 'wallpaper', 'wall_1080.png'), os.path.join(data, 'theme', 'wall_1080.png'))
        if layout is not None:
            with open(os.path.join(data, 'theme', 'layout.json'), 'w') as f:
                json.dump(layout, f)
        if theme is not None:
            with open(os.path.join(data, 'theme', 'theme.json'), 'w') as f:
                json.dump(theme, f)
        feeds = {"weather": weather(), "news": news(), "plugin.homeassistant.cards": HOME_CARDS, "profiles": profiles(), "cameras": CAMERAS, **(feeds or {})}
        cmd = [BIN, '--host', '--kinds', kinds, '--data-dir', data, '--shm', os.path.join(data, 'shm'), '--wait', str(wait),
               '--push', '50:foreground:' + json.dumps(HOME), '--luna', '80:%d:%s' % (LP_TAG, json.dumps({"launchPoints": launch_points()}))]
        for topic, doc in feeds.items():
            cmd += ['--helper', '100:%s:%s' % (topic, json.dumps(doc))]
        if mode:
            # the material forced, so the wallpaper's luma does not choose (as scenes.py renders the QML)
            cmd += ['--mode', mode, '--theme', json.dumps({"material": mode})]
        if scale:
            cmd += ['--text-scale', str(scale)]
        cmd += list(extra)
        for s in steps:
            cmd += s
        env = dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1')
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get('HOST_TIMEOUT', '240')))
        out = r.stdout + r.stderr
        self.assertEqual(r.returncode, 0, out[-2000:])
        evals = [json.loads(m.group(1)) for m in re.finditer(r"^\[eval\] (.*)$", out, re.M)]
        return out, evals, data

    @staticmethod
    def k(ms, *names):
        """Presses, 200 ms apart from `ms`, each released 40 ms later."""
        args = []
        for i, n in enumerate(names):
            at = ms + 200 * i
            args += ['--key', '%d:%s' % (at, n), '--key', '%d:%s:up' % (at + 40, n)]
        return args

    @staticmethod
    def hold(ms, name, for_ms):
        return ['--key', '%d:%s' % (ms, name), '--key', '%d:%s:repeat' % (ms + for_ms // 2, name), '--key', '%d:%s:up' % (ms + for_ms, name)]

    @staticmethod
    def ev(ms):
        return ['--eval', '%d:home' % ms]

    @staticmethod
    def snap(ms, name):
        os.makedirs(OUT, exist_ok=True)
        return ['--snap', '%d:%s' % (ms, os.path.join(OUT, name + '.png'))]

    # ---- where the cards are
    def test_the_shipped_layout_is_drawn_where_it_always_was_and_the_dock_is_the_persons_own(self):
        theme = {"apps": {"dockHide": ["com.lighting.huesync"], "dockOrder": ["youtube.leanback.v4", "netflix"]}}
        out, evals, _ = self.host(1600, [self.ev(1400)], theme=theme)
        h = evals[-1]
        self.assertEqual(h["cards"], [["clock", 64, 134, 584, 222], ["home", 64, 376, 584, 238], ["news", 668, 134, 735, 480], ["weather", 1423, 134, 433, 480]])
        self.assertEqual(h["dock"][:3], ["com.webos.app.discovery", "youtube.leanback.v4", "netflix"], "LG's Apps tile first, then the person's order")
        self.assertNotIn("com.lighting.huesync", h["dock"])
        self.assertEqual(h["profile"], "Nitin")
        self.assertEqual(h["focusLayer"], "statusbar")

    def test_the_focus_walks_the_status_bar_the_cards_and_the_dock_and_ok_opens_a_tile(self):
        out, evals, _ = self.host(3200, [self.k(600, "down"), self.ev(900), self.k(1000, "right", "down"), self.ev(1500), self.k(1600, "left", "left", "ok"), self.ev(2400),
                                          self.k(2500, "back"), self.ev(3000)])
        self.assertEqual(evals[0]["widgetFocusId"], "clock")
        self.assertEqual(evals[1]["focusLayer"], "dock")
        self.assertTrue(evals[1]["dockIndex"] > 0, "the tile under the news card, not the first")
        self.assertIn("[log] launch", out)
        self.assertIsNotNone(evals[2]["launching"], "Home launched the app")
        self.assertEqual(evals[2]["layer"], "", "and closed what it had")
        self.assertEqual(evals[3]["focusLayer"], "widgets", "back from the dock goes to the cards")

    # ---- the bell and the notification drawer
    def test_the_bell_opens_the_drawer_kind_and_back_closes_it(self):
        out, evals, _ = self.host(4200, [self.k(600, "right", "ok"), ['--eval', '2200:state'], self.k(2300, "back"), ['--eval', '3800:state']], kinds='home,drawer')
        self.assertIn(evals[0]["panels"]["drawer"], ("opening", "open"), "OK on the bell opens the drawer")
        self.assertIn(evals[1]["panels"]["drawer"], ("leaving", "parked"), "Back closes it")
        self.assertNotIn("home asks for notifications", out)
        self.assertEqual(evals[1]["home"]["focusLayer"], "statusbar", "Home is where it was left")

    @staticmethod
    def toast(ms, text):
        return ['--luna', '%d:%d:%s' % (ms, TOAST_TAG, json.dumps({"message": text}))]

    def test_the_bell_counts_the_toasts_since_the_drawer_was_last_open(self):
        out, evals, _ = self.host(4200, [self.toast(400, "Update ready\nA new version"), self.toast(500, "Backup done"), ['--eval', '1200:state'],
                                          self.k(1300, "right", "ok"), ['--eval', '2600:state']], kinds='home,drawer')
        self.assertEqual(evals[0]["home"]["unread"], 2, "two toasts are two unread on the bell")
        self.assertEqual(evals[1]["home"]["unread"], 0, "opening the drawer marks them seen")

    def test_the_bell_is_logged_only_when_the_drawer_kind_is_not_the_shells(self):
        out, _, _ = self.host(1600, [self.k(600, "right", "ok")])
        self.assertIn("home asks for notifications", out)

    def test_the_library_rail_follows_the_last_section_at_both_text_sizes(self):
        for mode in ("dark", "light"):
            for scale in (1, 2):
                suffix = "-light" if mode == "light" else ""
                steps = [self.k(500, "1"), ['--pointer', '900:120:200:1'], ['--pointer', '940:120:200:0'],
                         self.k(1100, "down", "down", "down", "down", "down"), self.ev(2400),
                         ['--eval', '2500:lint'], self.snap(2600, "42-library-rail-text%d%s" % (scale * 100, suffix)),
                         self.k(2700, "down"), self.ev(3100)]
                _, values, _ = self.host(3400, steps, mode=mode, scale=scale, theme={"apps": {"hidden": ["netflix"]}})
                self.assertEqual(values[0]["apps"]["section"], "dock")
                self.assertGreaterEqual(values[0]["apps"]["railFirst"], int(scale == 2))
                self.assertEqual(values[1], [], "the visible library words pass layout and contrast lint")
                self.assertEqual(values[2]["apps"]["section"], "hidden")

    def test_the_cameras_card_fetches_decodes_and_draws_the_helpers_stills(self):
        import rust_cameras_scenes as sc
        import channel
        import fakehelper
        for mode in ("dark", "light"):
            for scale in (1, 2):
                port, helper_port = sc.free_port(), sc.free_port()
                chan = channel.Channel('offline-token')
                server = channel.serve(chan, port, say=lambda _: None)
                helper, state = fakehelper.serve(helper_port)
                state.pictures['front'] = sc.small('still')
                state.publish('cameras', CAMERAS)
                data = tempfile.mkdtemp(prefix='rusthome-stills-')
                token = os.path.join(data, 'test-token')
                with open(token, 'w') as f:
                    f.write('offline-token')
                suffix = '-light' if mode == 'light' else ''
                try:
                    steps = [self.ev(3000), ['--eval', '3100:lint'], self.snap(3200, '43-home-camera-stills-text%d%s' % (scale * 100, suffix)),
                             ['--push', '3400:screen:{"on":false}']]
                    out, values, _ = self.host(4000, steps, layout=CAMERAS_LAYOUT, data=data, mode=mode, scale=scale,
                                             extra=['--port', str(port), '--helper-port', str(helper_port), '--token-file', token])
                    self.assertGreater(values[0]['cameraFrames'], 0, 'binary answers are decoded into Home pictures: %s %s' % (state.frames, out))
                    self.assertEqual(values[1], [], repr(values[1]))
                    self.assertTrue(state.frames, 'the helper supplies the card stills')
                    self.assertEqual({frame[0] for frame in state.frames}, {'front'}, 'offline cameras ask for no still')
                    self.assertTrue(all(w < 300 and h < 200 for _, w, h, _ in state.frames), 'only thumbnail-sized stills')
                finally:
                    server.shutdown()
                    helper.shutdown()
                    server.server_close()
                    helper.server_close()

    # ---- the layout editor
    def test_the_editor_picks_up_a_card_moves_it_by_the_cell_turns_a_move_down_and_saves_the_owners_layout(self):
        steps = [self.k(600, "down"), self.hold(900, "ok", 900), self.ev(2000),
                 # the clock: OK picks it up, Right moves it a column (the news card is hidden), Down is under the home card: turned down
                 self.k(2100, "ok", "right", "down"), self.ev(2900),
                 # OK turns to the size, Right is a column wider, OK puts it down
                 self.k(3000, "ok", "right", "ok"), self.ev(3800),
                 # up to the bar (Add), right to Remove, Undo, Cancel, Save; OK saves
                 self.k(3900, "up", "right", "right", "right", "right", "ok"), self.ev(5400)]
        out, evals, data = self.host(5600, steps, layout=FREE)
        self.assertTrue(evals[0]["editing"])
        self.assertEqual(evals[0]["editor"]["sel"], "clock")
        self.assertEqual(evals[0]["editor"]["mode"], "select", "the release of the hold picked the card up")
        moved = {r[0]: r for r in evals[1]["layout"]}
        self.assertEqual(moved["clock"][1:5], [1, 0, 4, 4], "Right did not move the clock a column, or Down was not turned down")
        self.assertIn("At home is in the way", evals[1]["editor"]["toast"])
        sized = {r[0]: r for r in evals[2]["layout"]}
        self.assertEqual(sized["clock"][1:5], [1, 0, 5, 4])
        self.assertEqual(evals[2]["editor"]["mode"], "select")
        self.assertFalse(evals[3]["editing"])
        self.assertEqual(evals[3]["focusLayer"], "widgets")
        with open(os.path.join(data, "theme", "layout.json")) as fh:
            on_disk = json.load(fh)
        self.assertEqual(sorted(on_disk), ["grid", "widgets"], "the file holds what is shipped too")
        self.assertEqual(on_disk["grid"], {"rows": 16})
        saved = {c["id"]: c for c in on_disk["widgets"]}
        self.assertEqual([saved["clock"][k] for k in ("col", "row", "span", "rows")], [1, 0, 5, 4])
        self.assertTrue(saved["news"]["hidden"])

    def test_back_asks_when_something_changed_and_discard_puts_home_as_it_was(self):
        steps = [self.k(600, "down", "right"), self.hold(1100, "ok", 900), self.ev(2100),
                 # the weather card: pick up, left (the news card is hidden), put down by two OKs; Back with it put down asks
                 self.k(2200, "ok", "left", "ok", "ok", "back"), self.ev(3300),
                 # Keep editing is the first answer and what Back is: still editing; Back again, then to Discard, OK
                 self.k(3400, "back"), self.ev(3700), self.k(3800, "back", "right", "right", "ok"), self.ev(4800)]
        out, evals, data = self.host(5000, steps, layout=FREE)
        self.assertEqual(evals[0]["editor"]["sel"], "weather")
        self.assertEqual(evals[1]["editor"]["zone"], "ask", "Back after a change did not ask")
        self.assertEqual({r[0]: r for r in evals[1]["layout"]}["weather"][1], 8, "the weather card was not moved a column left")
        self.assertTrue(evals[2]["editing"], "Keep editing (and Back, which is its answer) left the editor")
        self.assertFalse(evals[3]["editing"])
        self.assertEqual({r[0]: r for r in evals[3]["layout"]}["weather"][1:5], [9, 0, 3, 9], "Discard left the moved card where it was put")
        self.assertFalse(os.path.exists(os.path.join(data, "theme", "layout.json.tmp")))

    # ---- profiles
    def test_the_picker_switches_and_a_locked_profile_asks_a_pin_from_the_owners_guest(self):
        steps = [self.k(600, "ok"), self.ev(900), self.k(1000, "right", "ok"), self.ev(1500), self.k(1600, "1", "2", "3", "4"), self.ev(2600)]
        out, evals, _ = self.host(2800, steps, feeds={"profiles": profiles(can_admin=False)})
        self.assertEqual(evals[0]["layer"], "picker")
        self.assertIsNotNone(evals[1]["picker"]["pin"], "Isabel has a PIN: the picker asks it")
        self.assertEqual(evals[1]["picker"]["pin"]["id"], "isabel")
        self.assertIn('[helper] POST /v1/profiles/switch', out)
        self.assertIn('"pin":"1234"', out.replace(' ', ''))
        self.assertEqual(evals[2]["picker"]["veil"], "in", "the veil comes up while the switch is asked")

    def test_a_switch_on_the_topic_closes_what_was_open_and_reads_the_files_again(self):
        steps = [self.k(600, "1"), self.ev(900), ['--helper', '1000:profiles:' + json.dumps(profiles(active="isabel", at=2))], self.ev(1400)]
        out, evals, _ = self.host(1600, steps)
        self.assertEqual(evals[0]["layer"], "apps")
        self.assertEqual(evals[1]["layer"], "", "the change of person closed the library")
        self.assertEqual(evals[1]["profile"], "Isabel")

    # ---- the pointer
    def test_a_hover_is_a_cue_and_a_click_selects(self):
        # the second dock tile: column 1 of the grid (120 px tiles at a 136 pitch from the card's 88 px padding; two rows of 22 tiles
        # are centred in the 392 px card, so the first row starts 68 px down)
        x, y = 64 + 88 + 136 + 60, 634 + 68 + 60
        steps = [['--pointer', '600:%d:%d' % (x, y)], self.ev(900), ['--pointer', '1000:%d:%d:1' % (x, y)], ['--pointer', '1100:%d:%d:0' % (x, y)], self.ev(1400)]
        out, evals, _ = self.host(1600, steps)
        self.assertEqual(evals[0]["focusLayer"], "statusbar", "hover moved no focus")
        self.assertIn("[log] launch", out, "a click opens what is under it")

    # ---- the renders beside the mocks
    def test_renders(self):
        for mode in ("dark", "light"):
            suffix = "" if mode == "dark" else "-light"
            self.host(2200, [self.k(600, "down", "down", "down"), self.snap(2000, "36-home" + suffix)], mode=mode)
            self.host(2200, [self.k(600, "down", "down", "down"), self.snap(2000, "36-home-text200" + suffix)], mode=mode, scale=2)
            self.host(2200, [self.k(600, "down"), self.snap(2000, "37-home-halves" + suffix)], layout=HALVES, mode=mode)
            self.host(2200, [self.k(600, "down", "down", "ok", "down"), self.snap(2000, "37-home-cameras" + suffix)], layout=CAMERAS_LAYOUT, mode=mode)
            self.host(3000, [self.k(600, "down"), self.hold(900, "ok", 900), self.snap(2800, "38-home-edit" + suffix)], layout=FREE, mode=mode)
            self.host(4000, [self.k(600, "down"), self.hold(900, "ok", 900), self.k(2000, "ok", "right", "down"), self.snap(3800, "38-home-edit-move" + suffix)], layout=FREE, mode=mode)
            self.host(4000, [self.k(600, "down"), self.hold(900, "ok", 900), self.k(2000, "up", "left", "left", "ok"), self.snap(3800, "38-home-edit-add" + suffix)], layout=FREE, mode=mode)
            for scale, size in ((None, ""), (2, "-text200")):
                toasts = [self.toast(300, "Update ready\nA new version is ready to install"), self.toast(400, "Backup done"), self.toast(500, "Remote battery low")]
                self.host(2400, toasts + [self.snap(2200, "41-home-bell-badge" + size + suffix)], mode=mode, scale=scale, kinds='home,drawer')
            self.host(3600, [self.toast(300, "Update ready\nA new version is ready to install"), self.toast(400, "Backup done"), self.toast(500, "Remote battery low"), self.k(1300, "right", "ok"),
                             self.snap(3400, "41-home-drawer" + suffix)], mode=mode, kinds='home,drawer')
            self.host(2200, [self.k(600, "1"), self.snap(2000, "12-all-apps" + suffix)], mode=mode)
            self.host(3200, [self.k(600, "1", "up", "ok", "right", "ok", "right", "ok"), self.snap(3000, "09-search-keyboard" + suffix)], mode=mode)
            self.host(2200, [self.k(600, "ok"), self.snap(2000, "40-profiles-picker" + suffix)], mode=mode)
            self.host(2600, [self.k(600, "ok", "right", "ok", "1", "2"), self.snap(2400, "40-profiles-pin" + suffix)], mode=mode, feeds={"profiles": profiles(can_admin=False)})
            self.host(2800, [self.k(600, "down", "down", "down"), self.hold(1300, "ok", 900), self.snap(2600, "16-context-menu-dock" + suffix)], mode=mode)
        for name in ("36-home", "36-home-light", "36-home-text200", "38-home-edit", "12-all-apps", "09-search-keyboard", "40-profiles-picker", "40-profiles-pin"):
            self.assertTrue(os.path.exists(os.path.join(OUT, name + ".png")), name)


if __name__ == "__main__":
    unittest.main()
