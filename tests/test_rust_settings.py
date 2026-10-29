"""Run with: cd tests && python3 -m unittest test_rust_settings   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The pages of the Rust shell's Settings that the second half of the port added (AI access, Home Assistant, Cameras and one camera's kinds)
against the stand-in of the compositor's UI channel, LG's settings service stood in and the fake helper (`rust_settings_host.py`): what a key
press sends the helper, and that every page reads clean (the host's layout and contrast lint) in both looks at both text sizes.

AI access is the one where what is NOT sent matters: whatever gives an assistant less goes to the helper at once; whatever gives more
(access on, a pairing code, a wider level, resuming, a higher limit) opens a plain answer on the TV and sends the helper nothing, because
the only approval that proves a person (Face ID on the phone) is not connected to this screen.
"""
import concurrent.futures
import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import render_rust_settings as rr  # noqa: E402
import rust_settings_host as host  # noqa: E402


def scene(name, post=None, keys=None):
    """A copy of the render scene `name`, answering the POSTs of `post` ({path: payload}) and pressing `keys` instead when given."""
    s = rr.BY_NAME[name]
    c = rr.Scene(s.name, s.page, keys if keys is not None else s.keys, dict(s.routes), s.topics, s.theme, s.lg, s.tab, s.layout)
    for path, payload in (post or {}).items():
        c.routes['POST ' + path] = payload
    return c


def posts(helper, prefix):
    return [(p, b) for m, p, b in helper.requests if m == 'POST' and p.startswith(prefix)]


def gets(helper, prefix):
    return [p for m, p, b in helper.requests if m == 'GET' and p.startswith(prefix)]


def owner_answer(schema, values):
    return lambda body: {'ok': True, 'settings': rr.plugin_doc(schema, values, 2)}


@unittest.skipUnless(host.available(), 'needs rustui/target/debug/lumatv-ui and Mesa')
class RustSettings(unittest.TestCase):
    def play(self, s, **kw):
        rc, text, helper, findings, png = rr.play(s, **kw)
        self.assertEqual(rc, 0, text[-3000:])
        self.assertEqual(findings, [], findings)
        return helper

    # ------------------------------------------------------------------------------------------------------------- AI access

    def test_ai_off_goes_to_the_helper_at_once(self):
        h = self.play(scene('ai', {'/v1/ai/listener': {'ok': True}}, ['right', 'left', 'ok']))
        self.assertEqual([b for p, b in posts(h, '/v1/ai/listener')], [{'enabled': False}])

    def test_ai_on_is_explained_and_the_helper_is_sent_nothing(self):
        # Off is in force, the cursor moves to "LAN only" and OK is pressed: the TV answers itself
        h = self.play(scene('ai-unavailable'))
        self.assertEqual(posts(h, '/v1/ai'), [])
        self.assertEqual(gets(h, '/v1/ai/audit') != [], True, 'it still reads the activity')

    def test_a_higher_volume_limit_sends_nothing_and_a_lower_one_is_sent_after_a_quiet_moment(self):
        h = self.play(scene('ai', {'/v1/ai/limits': {'ok': True}}, ['right', 'down', 'right']))
        self.assertEqual(posts(h, '/v1/ai'), [], 'a higher limit gives an assistant more')
        h = self.play(scene('ai', {'/v1/ai/limits': {'ok': True}}, ['right', 'down', 'left']))
        self.assertEqual([b for p, b in posts(h, '/v1/ai/limits')], [{'maxVolume': 58}])

    def test_a_pause_goes_to_the_helper_and_a_resume_does_not(self):
        h = self.play(scene('ai-assistants', {'/v1/ai/assistants/a1/pause': {'ok': True}}, ['right', 'up', 'right', 'down', 'ok']))
        self.assertEqual(len(posts(h, '/v1/ai/assistants/a1/pause')), 1)
        h = self.play(scene('ai-assistants', {}, ['right', 'up', 'right', 'down', 'down', 'ok']))
        self.assertEqual(posts(h, '/v1/ai'), [], 'resuming gives an assistant more')

    # -------------------------------------------------------------------------------------------------------- Home Assistant

    def test_home_assistant_asks_the_plugin_for_each_list_again_when_the_room_changes_and_a_pick_is_written_to_the_plugins_page(self):
        s = scene('homeassistant-room', {'/v1/settings/home-assistant': owner_answer(rr.HA_SCHEMA, dict(rr.HA_VALUES, room='kitchen'))}, ['right', 'up', 'right', 'down', 'ok'])
        h = self.play(s)
        asked = sorted(p.rsplit('/', 1)[1] for p in gets(h, '/v1/plugins/home-assistant/choices/'))
        # once when the page opens and once more when the room changes (the lights and the sensors are the room's)
        self.assertEqual(asked, ['controls', 'controls', 'doorbells', 'doorbells', 'presence', 'presence', 'rooms', 'rooms'])
        written = posts(h, '/v1/settings/home-assistant')
        self.assertEqual([b['changes'] for p, b in written], [[{'key': 'room', 'value': {'type': 'choice', 'value': 'kitchen'}}]])

    def test_home_assistant_unlinked_has_only_what_the_owner_does_in_a_shell(self):
        h = self.play(scene('homeassistant-unlinked'))
        self.assertEqual(posts(h, '/v1/'), [])

    # -------------------------------------------------------------------------------------------------------------- Cameras

    def test_cameras_asks_for_its_lists_and_one_camera_switches_a_kind_in_the_order_picked(self):
        keys = ['right', 'up', 'right', 'right', 'right', 'down', 'down', 'ok', 'down', 'ok']
        # Garage pops up for person, vehicle and motion; the cursor is on Person after the page opens... then moves to Vehicle and switches it off
        values = dict(rr.CAM_VALUES, **{'notify.garage': ['person', 'motion']})
        s = scene('camera-kinds', {'/v1/settings/scrypted-cameras': owner_answer(rr.CAM_SCHEMA, values)}, keys)
        h = self.play(s)
        asked = sorted(p.rsplit('/', 1)[1] for p in gets(h, '/v1/plugins/scrypted-cameras/choices/'))
        self.assertEqual(asked, ['cameras', 'kinds'])
        written = [b['changes'] for p, b in posts(h, '/v1/settings/scrypted-cameras')]
        self.assertEqual(written, [[{'key': 'notify.garage', 'value': {'type': 'ordered', 'value': ['person', 'motion']}}]])

    # ----------------------------------------------------------------------------------------------------------- the pages read clean

    def test_every_new_page_reads_clean_in_both_looks_at_both_text_sizes(self):
        names = ['ai', 'ai-assistants', 'ai-allowed', 'ai-unavailable', 'homeassistant', 'homeassistant-room', 'homeassistant-unlinked',
                 'cameras', 'cameras-notify', 'cameras-video', 'camera-kinds', 'cameras-unlinked']
        jobs = [(n, mode, scale) for n in names for mode, scale in (('dark', 1.0), ('light', 2.0))]

        def one(job):
            n, mode, scale = job
            rc, text, findings, png = rr.render(rr.BY_NAME[n], mode, scale)
            return job, rc, findings, png, text

        with concurrent.futures.ThreadPoolExecutor(3) as pool:
            for job, rc, findings, png, text in pool.map(one, jobs):
                self.assertEqual(rc, 0, '%s\n%s' % (job, text[-1500:]))
                self.assertIsNotNone(png, job)
                self.assertEqual(findings, [], '%s: %s' % (job, json.dumps(findings)))


if __name__ == '__main__':
    unittest.main()
