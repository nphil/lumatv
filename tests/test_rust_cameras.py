"""Run with: cd tests && python3 -m unittest test_rust_cameras   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The Cameras app of the Rust shell (rustui/ui/src/cameras, kind `cameras`: the grid, the full view, the strip, the sheet, the options page)
against the stand-in of the compositor's UI channel (tests/qmlhost/channel.py) and the fake helper (tests/qmlhost/fakehelper.py), on the cases
test_camera_popup.py states for the QML viewer (the spec's numbers where the spec changed them): the grid asks a picture of every camera at the
tile size and its keys move the focus; over Home the full view asks for the live stream, is a hole while it plays, switches and stops it on Back;
over an app it is pictures only unless the theme offers live; a link that is not ready asks for no stream; the feeder's own window is not an app
in front; the strip commits a playback, the sheet plays a clip, the options page writes the pick through the settings route; the pointer's hover
is a cue and its click selects; nothing asks for a picture once the app is down; every screen reads clean at both text sizes in both looks.
"""
import concurrent.futures
import json
import os
import unittest

import rust_cameras_scenes as sc

NAMES = {'274': 'Back Door', '273': 'Front Door', '275': 'Garden'}
THEME_LIVE_OVER_APPS = ['--theme', json.dumps({'cameras': {'liveOverApps': True}})]


def three(state='ready'):
    """The `cameras` topic as test_camera_popup.py has it: Back Door, Front Door (the doorbell), Garden."""
    return {'state': state, 'reason': '', 'now_skew_ms': 0, 'rev': 1,
            'cameras': [sc.camera(274, 'Back Door'), sc.camera(273, 'Front Door', room='Entry', type='doorbell'), sc.camera(275, 'Garden')]}


def start(r, topic=None, foreground=None):
    """Home (or the app) in front, the screen on, the helper's cameras, and the grid open at 900 ms."""
    r.channel(200, 'foreground', foreground or sc.HOME).channel(220, 'screen', {'on': True})
    r.helper(250, 'cameras', topic or three())
    return sc.open_grid(r)


def live_posts(res):
    return [p for p in res['helper'].posts if p[0].startswith('/v1/cameras/live')]


def playback_posts(res):
    return [p[1] for p in res['helper'].posts if p[0] == '/v1/cameras/playback']


def states(res):
    return [e['cameras'] for e in res['evals'] if isinstance(e, dict) and isinstance(e.get('cameras'), dict) and 'screen' in e['cameras']]


def geometry(res):
    return [e['cameras'] for e in res['evals'] if isinstance(e, dict) and isinstance(e.get('cameras'), dict) and 'base' in e['cameras']]


@unittest.skipUnless(sc.have_binary(), 'needs rustui/target/debug/lumatv-ui and Mesa')
class RustCameras(unittest.TestCase):
    def play(self, r, end, **kw):
        r.wait = end + 600
        res = sc.run(r, **kw)
        self.assertEqual(res['rc'], 0, res['out'][-3000:])
        return res

    def test_the_grid_asks_a_picture_of_every_camera_at_the_tile_size_and_its_keys_move_the_focus(self):
        r = sc.Run()
        start(r)
        r.eval(1600, 'state').eval(1600, 'geometry')
        r.press(1800, 'right').eval(2000, 'state')
        r.press(2100, 'down').eval(2300, 'state')
        r.press(2400, 'left').eval(2600, 'state')
        res = self.play(r, 2700)
        s = states(res)
        self.assertEqual((s[0]['screen']['mode'], s[0]['screen']['focus'], s[0]['cameras']), ('grid', 'Tile(0)', 3))
        self.assertEqual(s[1]['screen']['focus'], 'Tile(1)')
        self.assertEqual(s[2]['screen']['focus'], 'Tile(2)', 'down from the last tile of a longer row lands on the lone tile below')
        self.assertEqual(s[3]['screen']['focus'], 'Tile(2)', 'left from the first of a row stays')
        tile = geometry(res)[0]['tiles'][0]
        asked = {c: (w, h) for c, w, h, _ in res['helper'].frames}
        self.assertEqual(set(asked), set(NAMES), 'a picture of each camera')
        self.assertEqual(set(asked.values()), {(int(tile['w']), int(tile['h']) - 44)}, 'at the picture size of a tile, its name apart')
        self.assertEqual(geometry(res)[0]['tileCameras'], ['274', '273', '275'])
        self.assertIn('[helper] GET /v1/settings/scrypted-cameras', res['out'], 'the settings are read when the app opens')

    def test_over_home_the_full_view_asks_for_the_live_stream_opens_a_hole_while_it_plays_switches_and_stops_it_on_back(self):
        r = sc.Run()
        r.backdrop = 'game'
        start(r)
        r.press(1500, 'right')                                          # Front Door
        r.press(1700, 'ok').eval(1900, 'state')
        r.helper(2000, 'cameras.live', sc.live_topic('273'))
        r.eval(2300, 'state')
        r.press(2400, 'right').eval(2600, 'state')
        r.press(2700, 'back').eval(2900, 'state')
        res = self.play(r, 3000)
        s = states(res)
        self.assertEqual((s[0]['screen']['mode'], s[0]['asked'], s[0]['hole']), ('full', '273', False), 'asked for the stream, pictures until it plays')
        self.assertTrue(s[1]['hole'], 'the layer is a hole over the video plane while the stream plays')
        self.assertEqual((s[2]['asked'], s[2]['hole'], s[2]['screen']['camera']), ('275', False, '275'), 'the next camera is asked for; the hole waits for its stream')
        self.assertEqual((s[3]['screen']['mode'], s[3]['asked'], s[3]['layers']), ('', None, 0), 'Back closes the viewer and takes every layer off')
        self.assertEqual(live_posts(res), [['/v1/cameras/live', {'camera': '273'}], ['/v1/cameras/live', {'camera': '275'}], ['/v1/cameras/live/stop', {}]])

    def test_over_an_app_the_full_view_is_pictures_only_and_asks_for_no_live_stream(self):
        r = sc.Run()
        start(r, foreground=sc.APP)
        r.press(1500, 'right').press(1700, 'ok').eval(2300, 'state')
        res = self.play(r, 2400)
        s = states(res)[0]
        self.assertEqual((s['screen']['mode'], s['appFront'], s['asked'], s['hole']), ('full', True, None, False))
        self.assertEqual(live_posts(res), [], 'an app is playing: the feeder window over it is unmeasured')
        self.assertTrue([f for f in res['helper'].frames if f[0] == '273' and (f[1], f[2]) == (1280, 720)], 'the full view draws the camera pictures at 1280 by 720')

    def test_over_an_app_live_is_asked_for_only_when_the_theme_offers_it(self):
        r = sc.Run(extra=THEME_LIVE_OVER_APPS)
        start(r, foreground=sc.APP)
        r.press(1500, 'right').press(1700, 'ok').eval(2300, 'state')
        res = self.play(r, 2400)
        self.assertEqual((states(res)[0]['screen']['mode'], states(res)[0]['asked']), ('full', '273'))
        self.assertEqual(live_posts(res), [['/v1/cameras/live', {'camera': '273'}]])

    def test_a_link_that_is_not_ready_asks_for_no_live_stream(self):
        r = sc.Run()
        start(r, three('offline'))
        r.press(1500, 'ok').eval(2000, 'state')
        res = self.play(r, 2100)
        s = states(res)[0]
        self.assertEqual((s['link'], s['asked']), ('offline', None))
        self.assertEqual(live_posts(res), [])

    def test_the_feeders_own_window_in_front_is_not_an_app_over_the_picture(self):
        r = sc.Run()
        start(r)
        r.press(1500, 'ok')
        r.channel(1900, 'foreground', {'app': 'com.lumatv.app.cameras', 'home': False})
        r.eval(2300, 'state')
        res = self.play(r, 2400)
        s = states(res)[0]
        self.assertEqual((s['appFront'], s['asked']), (False, '274'), 'the stream plays over Home still')
        self.assertEqual(live_posts(res), [['/v1/cameras/live', {'camera': '274'}]], 'nothing stopped it')

    def test_the_strip_scrubs_commits_after_a_moment_and_asks_for_the_recording_at_the_cursor(self):
        r = sc.Run()
        r.backdrop = 'game'
        start(r)
        r.press(1500, 'right').press(1700, 'ok')
        r.helper(2000, 'cameras.live', sc.live_topic('273'))
        sc.timeline(r, camera='273')
        r.press(2400, 'up').eval(2700, 'state')
        r.press(2900, 'left').eval(3000, 'state')
        r.eval(3600, 'state')
        res = self.play(r, 3700)
        s = states(res)
        self.assertTrue(s[0]['strip'], 'Up opens the strip')
        asked = [a[0] for a in res['helper'].camera_asks]
        self.assertIn('/v1/cameras/273/recordings', asked)
        self.assertIn('/v1/cameras/273/events', asked)
        posts = playback_posts(res)
        self.assertEqual(len(posts), 1, posts)
        self.assertEqual(posts[0]['camera'], '273')
        behind = sc.NOW() - posts[0]['at']
        self.assertTrue(0 < behind < 10 * 60000, 'one step of an hour strip is a minute: %s' % behind)
        self.assertNotIn('rate', posts[0])
        self.assertEqual(s[-1]['screen']['camera'], '273')

    def test_the_sheet_lists_the_events_asks_their_pictures_and_plays_the_clip_that_is_picked(self):
        r = sc.Run()
        r.backdrop = 'game'
        start(r)
        r.press(1500, 'right').press(1700, 'ok')
        r.helper(2000, 'cameras.live', sc.live_topic('273'))
        sc.sheet_data(r, camera='273')
        r.press(2400, 'down').eval(2800, 'state').eval(2800, 'geometry')
        r.press(3000, 'up').press(3200, 'right')                            # the Clips tab
        r.press(3400, 'down').press(3600, 'right').press(3800, 'ok')        # the second clip
        r.eval(4200, 'state')
        res = self.play(r, 4300)
        self.assertTrue(states(res)[0]['sheet'])
        geo = geometry(res)[0]
        self.assertGreaterEqual(len(geo['cards']), 4)
        asks = res['helper'].camera_asks
        self.assertTrue([a for a in asks if a[0] == '/v1/cameras/273/events'], asks)
        self.assertTrue([a for a in asks if a[0] == '/v1/cameras/273/clips'], asks)
        thumbs = [a for a in asks if a[0] == '/v1/cameras/273/thumbnail']
        self.assertTrue(len(thumbs) >= 4 and all((a[1]['w'], a[1]['h']) == ('352', '198') for a in thumbs), 'a picture of each shown card at its size')
        self.assertEqual(playback_posts(res), [{'camera': '273', 'clip': 'c2.b'}])

    def test_the_options_page_opens_on_a_held_ok_and_a_switch_writes_the_pick_through_the_settings_route(self):
        r = sc.Run()
        start(r)
        r.press(1500, 'right')                                              # Front Door
        r.hold(1700, 'ok').eval(2600, 'state')
        r.release(2700, 'ok')
        r.press(3000, 'ok')                                                 # the first row: Person, on by default
        r.eval(3600, 'state')
        res = self.play(r, 3700)
        s = states(res)
        self.assertTrue(s[0]['options'], 'a held OK on a tile opens the page of its camera')
        writes = [p for p in res['helper'].posts if p[0] == '/v1/settings/scrypted-cameras']
        self.assertEqual(writes, [['/v1/settings/scrypted-cameras', {'changes': [{'key': 'notify.273', 'value': {'type': 'ordered', 'value': ['doorbell']}}], 'ifVersion': 1}]])
        self.assertEqual(live_posts(res), [], 'a held OK is not the full view')

    def test_the_pointers_hover_is_a_cue_and_its_click_selects(self):
        r = sc.Run()
        start(r)
        r.pointer(1500, 1200, 300, 0).eval(1700, 'state')                   # over the second tile
        r.pointer(1800, 1200, 300, 1).pointer(1900, 1200, 300, 0).eval(2400, 'state')
        res = self.play(r, 2500)
        s = states(res)
        self.assertEqual((s[0]['screen']['mode'], s[0]['screen']['focus']), ('grid', 'Tile(0)'), 'hover moves no focus and asks nothing')
        self.assertEqual((s[1]['screen']['mode'], s[1]['screen']['camera']), ('full', '273'), 'the click opens the camera under it')
        self.assertEqual(live_posts(res), [['/v1/cameras/live', {'camera': '273'}]])

    def test_the_grids_settings_button_closes_the_app_and_opens_settings_at_the_cameras_page(self):
        r = sc.Run(kinds='cameras,settings')
        start(r)
        r.press(1500, 'up').press(1700, 'right').eval(1900, 'state')       # the header's Settings button
        r.press(2000, 'ok').eval(2500, 'state')
        for ms in (2600, 2800, 3000, 3200):                                 # out of the page, up to the rail, out
            r.press(ms, 'back')
        r.eval(3800, 'state')
        res = self.play(r, 3900)
        docs = [e for e in res['evals'] if isinstance(e, dict) and isinstance(e.get('cameras'), dict) and 'screen' in e['cameras']]
        self.assertEqual(docs[0]['cameras']['screen']['focus'], 'Settings')
        self.assertFalse(docs[0]['settings']['open'])
        self.assertEqual(docs[1]['cameras']['screen']['mode'], '', 'the app makes way')
        self.assertEqual((docs[1]['settings']['open'], docs[1]['settings']['page']), (True, 'cameras'), 'Settings, the Cameras page')
        self.assertFalse(docs[2]['settings']['open'], 'Back leaves Settings')
        self.assertEqual(docs[2]['cameras']['screen']['mode'], '')

    def test_a_refused_video_window_leaves_the_plane_path_and_a_glyph_beside_the_name(self):
        r = sc.Run()
        r.backdrop = 'game'
        start(r)
        r.press(1500, 'ok')
        r.helper(2000, 'cameras.live', sc.live_topic('274'))
        r.eval(2300, 'state').eval(2300, 'geometry')
        r.channel(2400, 'vwin', sc.VWIN_REFUSED)
        r.eval(2900, 'state').eval(2900, 'geometry')
        res = self.play(r, 3000)
        s, g = states(res), geometry(res)
        self.assertEqual((s[0]['window'], s[0]['hole']), ('Unclaimed', True), 'nothing refused yet: the stream plays')
        self.assertNotIn('cue', g[0]['pills'])
        self.assertEqual((s[1]['window'], s[1]['hole']), ('Refused', True), 'refused: the plane path goes on, nothing is faked')
        cue, name = g[1]['pills']['cue'], g[1]['pills']['name']
        self.assertGreaterEqual(cue['x'], name['x'] + name['w'], 'the glyph sits beside the name')
        self.assertEqual(live_posts(res), [['/v1/cameras/live', {'camera': '274'}]], 'the stream was not stopped')

    def test_a_refusal_is_a_toast_and_the_picture_goes_on(self):
        r = sc.Run()
        r.setup(lambda c, st: st.refusals.update({'/v1/cameras/live': (409, 'busy', 'Scrypted is busy')}))
        start(r)
        r.press(1500, 'ok').eval(2000, 'state')
        res = self.play(r, 2100)
        s = states(res)[0]
        self.assertEqual(s['toast'], 'Scrypted is busy')
        self.assertEqual(s['screen']['mode'], 'full')

    def test_nothing_asks_for_a_picture_once_the_app_is_down_and_its_layers_are_gone(self):
        r = sc.Run()
        start(r)
        r.press(2000, 'back').eval(2400, 'state')
        res = self.play(r, 3600)
        s = states(res)[0]
        self.assertEqual((s['screen']['mode'], s['layers'], s['inFlight']), ('', 0, 0))
        last = max(f[3] for f in res['helper'].frames)
        self.assertLess(last, res['helper'].frames[0][3] + 1800, 'no ask after the close')

    def test_every_screen_reads_clean_at_both_text_sizes_in_both_looks(self):
        work = [(name, scale, mode) for name in ('grid', 'grid-more', 'grid-unlinked', 'full-start', 'full-failed', 'full-refused', 'strip', 'strip-scrub', 'events', 'clips', 'events-all', 'options')
                for scale in (1.0, 2.0) for mode in ('dark', 'light')]

        def one(args):
            name, scale, mode = args
            res = sc.run_scene(name, scale, mode)
            lint = next((e for e in res['evals'] if isinstance(e, list)), None)
            return name, scale, mode, res['rc'], lint
        with concurrent.futures.ThreadPoolExecutor(int(os.environ.get('RUST_CAMERAS_JOBS', '4'))) as pool:
            for name, scale, mode, rc, lint in pool.map(one, work):
                self.assertEqual(rc, 0, (name, scale, mode))
                self.assertEqual(lint, [], (name, scale, mode))


if __name__ == '__main__':
    unittest.main()
