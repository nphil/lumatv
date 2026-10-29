"""Run with: cd tests && python3 -m unittest test_rust_panels   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The Rust shell's panels (rustui/ui/src/panels: the kinds quick, picker, drawer, menu, confirm, switcher and entry) on the host harness
(`lumatv-ui --host`: the shell's logic against the engine in its own process on Mesa) against the stand-in of the compositor's UI channel
(tests/qmlhost/channel.py), LG's settings service stood in (qmlhost/lgsettings.py) and the fake helper, on the cases the QML tests state
(test_quick_panels.py, test_panels.py, test_switcher.py, test_shell_switcher.py, test_plugins_surfaces.py): the gear's `show` opens the quick
panel and a second one closes it; a locked profile asks its PIN on the pad in place; a tile is picked up for editing by a held OK and a
stepper's value opens its card; a connected plugin's tiles reach the panel through the helper poll (and a plugin that is not connected shows
none); the owners' menu and dialog take the keys and close; the switcher places its cards for the compositor's
pictures, activates the one chosen and is offered by the Home key at rest; Settings' typed text goes to the entry sheet and the answer comes
back to Settings once. Every screen is checked by the host's layout and contrast lint, and `RUST_PANELS_RENDERS=<dir>` writes the renders
(1280-wide JPEGs) the docs keep beside the mocks (docs/mocks/host-rust/panels/).
"""
import concurrent.futures
import json
import os
import sys
import tempfile
import threading
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
sys.path.insert(0, os.path.join(HERE, 'qmlhost'))
import lgsettings  # noqa: E402
import render_rust_settings as rs  # noqa: E402
import rust_cameras_scenes as sc  # noqa: E402
import rust_settings_host as host  # noqa: E402

HOME = {'app': 'com.webos.app.home', 'home': True}
APP = {'app': 'netflix', 'home': False}
SWITCHER_ON = {'remote': {'appSwitcher': True}}
EDIT_ON = {'remote': {'quickPanelEdit': True}}
WIDTH, QUALITY = 1280, 80
RENDERS = os.environ.get('RUST_PANELS_RENDERS')
OPEN = 700


def profiles(active='nitin'):
    people = [('nitin', 'Nitin', '#A79BF5', True), ('isabel', 'Isabel', '#EB6F92', False), ('mia', 'Mia', '#F6C177', False)]
    return {'ready': True, 'active': active, 'owner': 'nitin', 'canAdmin': active == 'nitin', 'pickerOnWake': False, 'startWith': 'last', 'max': 6, 'switchedAt': 1,
            'profiles': [{'id': i, 'name': n, 'initial': n[0], 'color': c, 'avatar': {'kind': 'initial'}, 'avatarFile': None, 'owner': o, 'active': i == active,
                          'locked': i == 'isabel', 'firstRun': False} for i, n, c, o in people]}


def app(id, title, picture='live', state=''):
    return {'id': id, 'title': title, 'picture': picture, 'w': 1920, 'h': 1080, 'state': state}


APPS = [app('plex', 'Plex'), app('youtube.leanback.v4', 'YouTube'), app('com.webos.app.hdmi2', 'HDMI 2', 'video'), app('netflix', 'Netflix', 'none', 'quitting')]
DOCK_MENU = {'title': 'Crunchyroll', 'sub': 'Streaming', 'icon': 'play', 'anchor': {'x': 700, 'y': 600, 'width': 160, 'height': 160},
             'items': [{'id': 'open', 'icon': 'play', 'label': 'Open'}, {'id': 'move', 'icon': 'layout-grid', 'label': 'Move on the dock', 'disabled': True},
                       {'id': 'sleep', 'icon': 'moon', 'label': 'Sleep timer', 'options': ['Off', '15 min', '30 min'], 'index': 0},
                       {'id': 'uninstall', 'icon': 'trash-2', 'label': 'Uninstall', 'detail': 'Asks first', 'danger': True}]}
UNINSTALL = {'icon': 'trash-2', 'title': 'Uninstall Crunchyroll?', 'body': 'The app and its data are removed from this TV. Your account stays where it is.',
             'answers': [{'id': 'keep', 'label': 'Keep it'}, {'id': 'uninstall', 'label': 'Uninstall', 'danger': True}], 'focus': 0}


class Scene:
    """What a person does: the topics the compositor pushes (ms, topic, doc), the keys (ms, name, hold ms), and the evals and snap at the end."""

    def __init__(self, name, kinds, pushes=(), keys=(), theme=None, helper=None, lg=None, show=None, end=None, dim=False):
        # `dim`: the compositor draws the switcher's dim and pictures under the shell; the harness stands in with the dim's colour
        self.dim = dim
        self.name, self.kinds, self.pushes, self.keys, self.theme, self.helper, self.lg = name, kinds, list(pushes), list(keys), theme, helper or {}, lg
        self.show = show
        last = max([k[0] for k in self.keys] + [p[0] for p in self.pushes] + [OPEN]) + 400
        self.end = end or last + 1000


def run(scene, mode='dark', scale=1.0, evals=('geometry', 'state', 'lint', 'report'), snap=None, extra=(), settings_page=None):
    extra = list(extra)
    for ms, topic, doc in scene.pushes:
        extra += ['--push', '%d:%s:%s' % (ms, topic, json.dumps(doc, separators=(',', ':')))]
    for ms, name, hold in scene.keys:
        if hold:
            extra += ['--key', '%d:%s' % (ms, name), '--key', '%d:%s:repeat' % (ms + hold // 2, name), '--key', '%d:%s:up' % (ms + hold, name)]
        else:
            extra += ['--key', '%d:%s' % (ms, name), '--key', '%d:%s:up' % (ms + 80, name)]
    for what in evals:
        extra += ['--eval', '%d:%s' % (scene.end - 300, what)]
    if scene.dim:
        from PIL import Image
        back = os.path.join(tempfile.mkdtemp(prefix='rustpanels-'), 'dim.png')
        Image.new('RGB', (1920, 1080), (0xFA, 0xF4, 0xED) if mode == 'light' else (0x12, 0x0E, 0x1A)).save(back)
        extra += ['--backdrop', back]
    if snap:
        os.makedirs(os.path.dirname(snap), exist_ok=True)
        extra += ['--snap', '%d:%s' % (scene.end, snap)]
    # on the client's own clock like the rest, so a loaded host cannot reorder them
    start = [(OPEN - 400, 'foreground', scene.helper.get('foreground', HOME)), (OPEN - 380, 'screen', {'on': True})]
    if scene.show:
        start.append((OPEN, 'show', scene.show))
    for ms, topic, doc in start:
        extra += ['--push', '%d:%s:%s' % (ms, topic, json.dumps(doc, separators=(',', ':')))]
    helper = host.FakeHelper({'topics': dict(scene.helper.get('topics', {}))})
    lg = scene.lg() if callable(scene.lg) else scene.lg or lgsettings.LgService(answers=rs.ONLINE)
    rc, out, chan, lg, helper = host.run(wait=scene.end + 500, extra=extra, lg=lg, helper=helper, kinds=scene.kinds, text_scale=scale, mode=mode, theme=scene.theme)
    ev = host.evals(out)
    by = {}
    for e in ev:
        if isinstance(e, list):
            by['lint'] = e
        elif isinstance(e, dict) and 'kinds' in e:
            by['state'] = e
        elif isinstance(e, dict) and 'can' in e:
            by['report'] = e
        elif isinstance(e, dict):
            by['geometry'] = e
    return {'rc': rc, 'out': out, 'chan': chan, 'ev': by}


def jpeg(png, path):
    from PIL import Image
    im = Image.open(png).convert('RGB')
    im = im.resize((WIDTH, round(im.height * WIDTH / im.width)), Image.LANCZOS)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    im.save(path, 'JPEG', quality=QUALITY, optimize=True, progressive=True)


def quick(name, keys=(), active='nitin', theme=None):
    return Scene(name, 'quick', keys=keys, theme=theme, helper={'topics': {'profiles': profiles(active)}}, show={'kind': 'quick', 'warm': False, 'n': 1})


# the screens the renders show, each by its name: (scene, the looks it is rendered in)
def scenes():
    return [
        Scene('switcher', 'switcher', pushes=[(OPEN, 'apps', APPS), (OPEN + 100, 'switcher', {'open': True, 'from': '', 'n': 1})], keys=[(OPEN + 700, 'right', 0)], theme=SWITCHER_ON, dim=True),
        Scene('menu', 'menu', pushes=[(OPEN, 'menu.open', DOCK_MENU)], keys=[(OPEN + 600, 'down', 0), (OPEN + 1100, 'down', 0)]),
        Scene('confirm', 'confirm', pushes=[(OPEN, 'confirm.ask', UNINSTALL)], keys=[(OPEN + 600, 'right', 0)]),
        # Settings > Network > Join: the password of "Basement" is typed on the entry sheet
        Scene('entry', 'settings,entry', lg=rs.with_network, show={'kind': 'settings', 'page': 'wifi'},
              keys=[(OPEN + 600, 'right', 0), (OPEN + 1100, 'up', 0), (OPEN + 1600, 'ok', 0), (OPEN + 2100, 'ok', 0), (OPEN + 2600, '2', 0)]),
        quick('quick-pin', [(OPEN + 600, 'up', 0), (OPEN + 1100, 'left', 0), (OPEN + 1600, 'ok', 0)], active='mia'),
        quick('quick-edit', [(OPEN + 600, 'ok', 900)], theme=EDIT_ON),
        quick('quick-card', [(OPEN + 600, 'ok', 0)], theme=EDIT_ON),
    ]


@unittest.skipUnless(host.available(), 'needs rustui/target/debug/lumatv-ui and Mesa')
class RustPanels(unittest.TestCase):
    def ok(self, r):
        self.assertEqual(r['rc'], 0, r['out'][-3000:])
        return r['ev']

    # ---- the quick panel
    def test_the_gears_show_opens_the_quick_panel_back_closes_it_and_a_second_show_closes_it_too(self):
        two = Scene('quick-toggle', 'quick', helper={'topics': {'profiles': profiles()}}, show={'kind': 'quick', 'warm': False, 'n': 1},
                    pushes=[(OPEN + 1400, 'show', {'kind': 'quick', 'warm': False, 'n': 2})])
        ev = self.ok(run(two, evals=('state', 'report')))
        self.assertIn(ev['state']['panels']['quick'], ('leaving', 'parked', 'gone'), 'a second show closed it')
        self.assertFalse(ev['report']['wants'])
        sheet = self.ok(run(quick('quick-open'), evals=('geometry', 'report')))['geometry']['quick']['sheet']
        self.assertEqual((sheet['x'], sheet['w']), (1320.0, 560.0), 'the sheet is 560 wide at x 1320')
        closed = self.ok(run(quick('quick-back', [(OPEN + 600, 'back', 0)]), evals=('state',)))
        self.assertIn(closed['state']['panels']['quick'], ('leaving', 'parked', 'gone'))

    def test_a_locked_profile_asks_its_pin_on_the_pad_in_place_and_back_gives_the_list_back(self):
        pad = self.ok(run(quick('pin', [(OPEN + 600, 'up', 0), (OPEN + 1100, 'left', 0), (OPEN + 1600, 'ok', 0)], active='mia'), evals=('geometry',)))
        self.assertEqual(len(pad['geometry']['quick']['pad']), 12, 'the digits, delete and cancel')
        back = self.ok(run(quick('pin-back', [(OPEN + 600, 'up', 0), (OPEN + 1100, 'left', 0), (OPEN + 1600, 'ok', 0), (OPEN + 2100, 'back', 0)], active='mia'), evals=('geometry',)))
        self.assertEqual(back['geometry']['quick']['pad'], [])
        self.assertTrue(back['geometry']['quick']['rows'], 'the list is back')

    def test_a_held_ok_picks_a_tile_up_for_editing_and_a_stepper_opens_its_card_when_the_owner_switched_editing_on(self):
        edit = self.ok(run(quick('edit', [(OPEN + 600, 'ok', 900)], theme=EDIT_ON), evals=('geometry',)))
        self.assertTrue(edit['geometry']['quick']['edit'], 'the edit list is drawn')
        card = self.ok(run(quick('card', [(OPEN + 600, 'ok', 0)], theme=EDIT_ON), evals=('geometry',)))
        self.assertIsNotNone(card['geometry']['quick']['popup'])
        plain = self.ok(run(quick('plain', [(OPEN + 600, 'ok', 900)]), evals=('geometry',)))
        self.assertEqual(plain['geometry']['quick']['edit'], [], 'with the switch off a hold edits nothing')

    def test_a_connected_plugins_tiles_reach_the_panel_through_the_helper_poll_and_one_that_is_not_connected_shows_none(self):
        import fakehelper
        roster = {'plugins': [{'id': 'ambient', 'state': 'connected'}, {'id': 'sleeper', 'state': 'waiting'}], 'firstParty': []}
        tile = {'items': [{'id': 'ambient', 'label': 'Ambient light', 'icon': 'base.icons/lamp', 'state': {'type': 'on'}, 'action': 'toggle', 'hint': 'Follows the picture'}]}
        other = {'items': [{'id': 'nope', 'label': 'Never shown'}]}

        def setup(chan, st):
            st.filter_topics = True
            st.keepalive = True
            st.topics['plugins'] = roster

            def later():
                give_up = time.time() + 60
                while not chan.reports and time.time() < give_up:
                    time.sleep(0.02)
                time.sleep(0.8)
                st.publish('plugin.sleeper.tiles', other)
                st.publish('plugin.ambient.tiles', tile)
            threading.Thread(target=later, daemon=True).start()

        r = sc.Run(kinds='quick', extra=['--theme', json.dumps({})], wait=4800)
        r.channel(200, 'foreground', sc.HOME).channel(220, 'screen', {'on': True}).channel(3200, 'show', {'kind': 'quick', 'warm': False, 'n': 1})
        r.eval(3900, 'geometry').setup(setup)
        res = sc.run(r)
        self.assertEqual(res['rc'], 0, res['out'][-2000:])
        ids = [row['id'] for row in res['evals'][0]['quick']['rows']]
        self.assertIn('plugin.ambient.ambient', ids, 'the connected plugin\'s tile is on the panel')
        self.assertNotIn('plugin.sleeper.nope', ids)
        # the helper's base list is already past the 16 names a poll may carry: the poll names none and gets every topic (as the QML client's
        # does); which plugins' tiles are shown is the roster's word, and the plugin that is not connected has none
        polls = res['helper'].polls
        self.assertTrue(polls and all(p is None for p in polls), 'every topic is asked for: %s' % polls[-2:])

    # ---- the context menu and the confirm dialog
    def test_a_menu_its_owner_opens_takes_the_keys_skips_the_disabled_row_and_a_pick_closes_it(self):
        s = Scene('menu', 'menu', pushes=[(OPEN, 'menu.open', DOCK_MENU)], keys=[(OPEN + 600, 'down', 0)])
        ev = self.ok(run(s, evals=('geometry', 'report', 'state')))
        g = ev['geometry']['menu']
        self.assertEqual(len(g['rows']), 4)
        self.assertEqual(g['focus'], 2, 'down from Open skips the disabled Move')
        self.assertTrue(ev['report']['wants'], 'the compositor is told the layer has the keys')
        picked = self.ok(run(Scene('menu-pick', 'menu', pushes=[(OPEN, 'menu.open', DOCK_MENU)], keys=[(OPEN + 600, 'ok', 0)]), evals=('report', 'state')))
        self.assertFalse(picked['report']['wants'])
        self.assertIn(picked['state']['panels']['menu'], ('leaving', 'parked', 'gone'))

    def test_a_dialog_has_the_safe_answer_first_and_back_is_that_answer(self):
        ev = self.ok(run(Scene('confirm', 'confirm', pushes=[(OPEN, 'confirm.ask', UNINSTALL)]), evals=('geometry', 'report')))
        self.assertEqual(ev['geometry']['confirm']['focus'], 0)
        self.assertEqual(ev['geometry']['confirm']['answers'][1]['danger'], True)
        self.assertTrue(ev['report']['wants'])
        back = self.ok(run(Scene('confirm-back', 'confirm', pushes=[(OPEN, 'confirm.ask', UNINSTALL)], keys=[(OPEN + 600, 'back', 0)]), evals=('report',)))
        self.assertFalse(back['report']['wants'])

    # ---- the app switcher
    def test_the_switcher_places_the_cards_for_the_compositors_pictures_and_asks_for_the_app_chosen(self):
        s = Scene('switcher', 'switcher', pushes=[(OPEN, 'apps', APPS), (OPEN + 100, 'switcher', {'open': True, 'from': '', 'n': 1})],
                  keys=[(OPEN + 700, 'right', 0), (OPEN + 1200, 'ok', 0)], theme=SWITCHER_ON)
        r = run(s, evals=('geometry', 'report'))
        ev = self.ok(r)
        self.assertIn('switcher', ev['report']['can'], 'the owner switched it on: the compositor may offer it')
        layouts = [d for d in r['chan'].switcher if d.get('op') == 'layout']
        self.assertTrue(layouts, r['out'][-1500:])
        placed = layouts[0]['cards']
        self.assertEqual([c['id'] for c in placed], ['plex', 'youtube.leanback.v4'], 'a picture is placed for the two live apps; the HDMI one is an icon, the closing one has none')
        self.assertTrue(all(c['w'] == 400 and c['h'] == 225 for c in placed))
        self.assertGreater(layouts[0]['scrim']['a'], 0.9)
        self.assertEqual([d for d in r['chan'].switcher if d.get('op') == 'activate'], [{'op': 'activate', 'id': 'youtube.leanback.v4'}])
        self.assertEqual(ev['geometry']['switcher']['cards'][2]['picture'], 'video')

    def test_back_asks_the_compositor_to_close_it_and_it_closes_only_when_the_compositor_says_so(self):
        s = Scene('switcher-back', 'switcher', pushes=[(OPEN, 'apps', APPS), (OPEN + 100, 'switcher', {'open': True, 'from': '', 'n': 1})], keys=[(OPEN + 700, 'back', 0)], theme=SWITCHER_ON)
        r = run(s, evals=('report',))
        ev = self.ok(r)
        self.assertIn({'op': 'close'}, r['chan'].switcher)
        self.assertTrue(ev['report']['wants'], 'the keys stay with it until the compositor closes it')

    def test_the_home_key_at_rest_with_apps_open_opens_it_and_not_with_something_open_on_home(self):
        base = [(OPEN, 'apps', APPS)]
        at_rest = Scene('homekey', 'home,switcher', pushes=base + [(OPEN + 300, 'homekey', {'n': 1, 'switcher': True})], theme=SWITCHER_ON)
        r = run(at_rest, evals=('report',))
        self.ok(r)
        self.assertIn({'op': 'open'}, r['chan'].switcher)
        library = Scene('homekey-busy', 'home,switcher', pushes=base + [(OPEN + 300, 'homekey', {'n': 1, 'switcher': False})], theme=SWITCHER_ON)
        r = run(library, evals=('report',))
        self.ok(r)
        self.assertNotIn({'op': 'open'}, r['chan'].switcher, 'the compositor did not offer it')
        off = Scene('homekey-off', 'home,switcher', pushes=base + [(OPEN + 300, 'homekey', {'n': 1, 'switcher': True})])
        r = run(off, evals=('report',))
        self.ok(r)
        self.assertNotIn({'op': 'open'}, r['chan'].switcher, 'the owner has it switched off')

    # ---- the entry sheet, from Settings
    def entry_scene(self, keys):
        return Scene('entry', 'settings,entry', lg=rs.with_network, show={'kind': 'settings', 'page': 'wifi'}, keys=[(OPEN + 600 + 500 * i, k, 0) for i, k in enumerate(keys)])

    def test_a_password_asked_by_settings_is_typed_as_bullets_and_done_gives_it_back_once_to_the_question_that_follows(self):
        # right: the page; up: Basement; ok: its password; ok types t, 2 types 2; down x3 and right reach Done; ok
        typing = self.ok(run(self.entry_scene(['right', 'up', 'ok', 'ok', '2']), evals=('geometry', 'state')))
        self.assertEqual(typing['geometry']['entry']['shown'], '\u2022' + '2', 'the first character is a bullet, the last is shown')
        self.assertEqual(typing['state']['settings']['sheet'], {'entry': 'Password for Basement'})
        self.assertEqual(typing['state']['panels']['entry'], 'open')
        done = self.ok(run(self.entry_scene(['right', 'up', 'ok', 'ok', '2', 'down', 'down', 'down', 'right', 'ok']), evals=('state',)))
        sheet = done['state']['settings']['sheet']
        self.assertEqual(sheet['confirm'], 'Join Basement?', 'Done handed the text to Settings, which asks before it joins')

    def test_back_cancels_the_entry_and_settings_is_not_left_waiting(self):
        r = self.ok(run(self.entry_scene(['right', 'up', 'ok', 'ok', 'back']), evals=('state',)))
        self.assertIsNone(r['state']['settings']['sheet'])
        self.assertIn(r['state']['panels']['entry'], ('leaving', 'parked', 'gone'))

    def test_without_the_entry_kind_a_text_ask_is_let_go_and_the_sheet_is_not_left_up(self):
        s = Scene('noentry', 'settings', lg=rs.with_network, show={'kind': 'settings', 'page': 'wifi'}, keys=[(OPEN + 600 + 500 * i, k, 0) for i, k in enumerate(['right', 'up', 'ok'])])
        r = run(s, evals=('state',))
        ev = self.ok(r)
        self.assertIsNone(ev['state']['settings']['sheet'])
        self.assertIn('the entry kind is not on', r['out'])

    # ---- every screen reads clean and the renders
    def test_every_panel_screen_reads_clean_at_both_text_sizes_in_both_looks_and_the_renders_are_written_when_asked(self):
        jobs = []
        for s in scenes():
            for mode, scale, suffix in (('dark', 1.0, 'dark-100'), ('light', 2.0, 'light-200')):
                png = os.path.join(tempfile.mkdtemp(prefix='rustpanels-'), s.name + '.png')
                jobs.append((s, mode, scale, suffix, png))
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            results = list(pool.map(lambda j: run(j[0], j[1], j[2], evals=('lint', 'state'), snap=j[4]), jobs))
        for (s, mode, scale, suffix, png), r in zip(jobs, results):
            self.assertEqual(r['rc'], 0, '%s %s: %s' % (s.name, suffix, r['out'][-1500:]))
            lint = r['ev'].get('lint', [])
            self.assertEqual(lint, [], '%s %s: %s' % (s.name, suffix, lint))
            self.assertTrue(os.path.exists(png), '%s %s has a picture' % (s.name, suffix))
            if RENDERS:
                jpeg(png, os.path.join(RENDERS, '%s-%s.jpg' % (s.name, suffix)))


if __name__ == '__main__':
    unittest.main()
