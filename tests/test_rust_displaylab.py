"""Run with: cd tests && python3 -m unittest test_rust_displaylab   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The Rust shell's display lab (`lumatv-ui --host --kinds displaylab`) against the stand-in of the compositor's UI channel and a fake of the
helper's `/v1/displaylab` door (`render_rust_displaylab.py`): what a person's keys send the helper (one look at a time, A being the set's own
state, every way out ending the session), what is kept, and that every step reads clean (the host's layout and contrast lint) in both looks at
both text sizes. The registers themselves are the helper's (`helper/app/tests/displaylab.rs`): the lab sends names of looks, never an address.
"""
import concurrent.futures
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import render_rust_displaylab as rd  # noqa: E402
import rust_settings_host as host  # noqa: E402


def sets(helper):
    """Every look the lab asked for, in order, as (control, option)."""
    return [(b.get('control'), b.get('option')) for m, p, b in helper.requests if m == 'POST' and p == '/v1/displaylab/set']


def posts(helper, path):
    return [b for m, p, b in helper.requests if m == 'POST' and p == '/v1/displaylab/' + path]


@unittest.skipUnless(host.available(), 'needs rustui/target/debug/lumatv-ui and Mesa')
class DisplayLab(unittest.TestCase):
    def play(self, scene, **kw):
        rc, text, helper, got, png = rd.play(scene, **kw)
        self.assertEqual(rc, 0, text[-3000:])
        return helper, got

    def lab(self, got):
        return got['state']['displaylab']

    def test_a_step_starts_on_the_sets_own_state_and_the_switch_and_the_answer_each_send_one_look(self):
        # Start; Right to B; OK on "This one is better": B is the answer, and the next step starts on A again
        helper, got = self.play(rd.Scene('flow', ['ok', 'right', 'ok']))
        self.assertEqual(sets(helper), [('sharpness', 'normal'), ('sharpness', 'strong'), ('sharpness', 'normal')])
        lab = self.lab(got)
        self.assertEqual((lab['step'], lab['shown'], lab['answers']), ('sharpness-medium', 0, {'sharpness-strong': 'b'}))
        self.assertEqual(len(posts(helper, 'begin')), 1)
        self.assertTrue(all(p.get('control') in ('sharpness', 'window') for p in posts(helper, 'set')), 'only names of looks')

    def test_the_wheel_press_is_ok_and_left_and_right_switch_between_the_two_looks(self):
        helper, got = self.play(rd.Scene('switch', ['ok', 'right', 'left', 'right']))
        self.assertEqual(sets(helper), [('sharpness', 'normal'), ('sharpness', 'strong'), ('sharpness', 'normal'), ('sharpness', 'strong')])
        self.assertEqual(self.lab(got)['answers'], {}, 'switching never answers')

    def test_a_whole_run_is_kept_and_sent_to_the_helper_with_what_each_answer_was_and_whether_both_looks_were_seen(self):
        # B on the first step, then past the rest; Yes on the first window setting leaves the second out
        helper, got = self.play(rd.Scene('run', ['ok', 'right', 'ok'] + rd.PAST[1] + rd.PAST[2] + rd.PAST[3] + rd.PAST[4] + ['ok']))
        lab = self.lab(got)
        self.assertEqual((lab['open'], lab['stage']), (True, 'results'))
        sent = posts(helper, 'results')
        self.assertEqual(len(sent), 1)
        run = sent[0]
        self.assertTrue(run['complete'])
        self.assertEqual([s['id'] for s in run['steps']], ['sharpness-strong', 'sharpness-medium', 'sharpness-medium-strong', 'sharpness-soft', 'sharpness-off', 'window-a'])
        first = run['steps'][0]
        self.assertEqual((first['answer'], first['verdict'], first['sawBoth'], first['switches']), ('b', 'Strong looks better', True, 1))
        self.assertEqual([s['answer'] for s in run['steps'][1:]], ['skipped'] * 4 + ['yes'])
        self.assertIn('startedMs', run)

    def test_a_look_the_firmware_puts_back_at_once_is_said_on_the_screen_and_is_recorded_as_not_applied_not_as_an_opinion(self):
        helper, got = self.play(rd.BY_NAME['sharpness-not-applied'])
        lab = self.lab(got)
        self.assertEqual((lab['step'], lab['shown']), ('sharpness-strong', 1))
        self.assertIn("Couldn't apply Strong", lab['note'])
        labels = [a['label'] for a in got['geometry']['displaylab']['answers']]
        self.assertEqual(labels, ['Next'], 'no pick between two identical pictures')
        # OK on Next goes on, and the answer is the lab's finding
        helper, got = self.play(rd.Scene('not-applied-next', ['ok', 'right', 'ok'], rd.firmware_owned))
        lab = self.lab(got)
        self.assertEqual((lab['step'], lab['answers']), ('sharpness-medium', {'sharpness-strong': 'failed'}))

    def test_back_leaves_and_the_helper_is_asked_to_put_everything_back(self):
        helper, got = self.play(rd.Scene('leave', ['ok', 'right', 'back']))
        self.assertFalse(self.lab(got)['open'])
        self.assertGreaterEqual(len(posts(helper, 'end')), 1, 'the session is ended')
        self.assertEqual(sets(helper)[-1], ('sharpness', 'strong'), 'the lab itself puts nothing back: the helper does')

    def test_a_helper_that_cannot_run_it_is_asked_for_no_look_and_the_screen_offers_leaving(self):
        helper, got = self.play(rd.BY_NAME['refused'])
        self.assertEqual(sets(helper), [])
        lab = self.lab(got)
        self.assertEqual((lab['open'], lab['stage'], lab['link']), (True, 'intro', 'refused'))
        # OK is on Leave, and leaving asks the helper for nothing but the end
        helper, got = self.play(rd.Scene('refused-leave', ['ok', 'ok'], rd.refused))
        self.assertEqual(sets(helper), [])
        self.assertFalse(self.lab(got)['open'])

    def test_the_window_says_so_when_only_home_is_behind_it(self):
        helper, got = self.play(rd.Scene('home', rd.to_step(5), app='com.webos.app.home'))
        self.assertIn('Home is behind', self.lab(got)['note'])
        helper, got = self.play(rd.Scene('app', rd.to_step(5)))
        self.assertIsNone(self.lab(got)['note'])

    def test_every_step_reads_clean_in_both_looks_at_both_text_sizes(self):
        names = ['intro', 'sharpness-strong', 'sharpness-medium-b', 'sharpness-not-applied', 'window-a-b', 'results']
        jobs = [(rd.BY_NAME[n], mode, scale) for n in names for mode, scale in (('dark', 1.0), ('light', 2.0))]
        with concurrent.futures.ThreadPoolExecutor(2) as pool:
            for (scene, mode, scale), (rc, text, findings, png) in zip(jobs, pool.map(lambda j: rd.render(*j), jobs)):
                label = '%s %s %s' % (scene.name, mode, scale)
                self.assertEqual(rc, 0, label + text[-2000:])
                self.assertIsNotNone(png, label)
                self.assertEqual(findings, [], label)


if __name__ == '__main__':
    unittest.main()
