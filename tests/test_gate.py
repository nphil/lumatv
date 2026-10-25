"""Run with: python3 -m unittest discover -s tests -p 'test_*.py'

docs/research/perf/gate.py: the surfaces table, the cycle commands, the trace and log parsing, the
RSS rule and the checks against a window report. Everything here is pure; the set is not touched.
"""
import math
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(os.path.dirname(HERE), "docs", "research", "perf"))
import gate  # noqa: E402


def window(name, first=20.0, end=120.0, over20=0, dropped=0, p90=2.5, anim=None):
    return {'name': name, 'first_frame_ms': first, 'burst_end_ms': end, 'burst_over20': over20, 'dropped': dropped,
            'burst_anim_p90_ms': p90, 'anim_render_ms': anim}


def report(surface, cycles, **kw):
    ws = []
    for _ in range(cycles):
        ws += [window(f'{surface}-open', **kw), window(f'{surface}-close', **kw)]
    return {'windows': ws}


def failed(checks):
    return sorted(c['name'] for c in checks if not c['ok'])


class Table(unittest.TestCase):
    def test_shipped_table_is_valid(self):
        self.assertTrue(gate.validate(gate.SURFACES))

    def test_marker_tokens_are_required(self):
        with self.assertRaises(ValueError):
            gate.validate([{'name': 'x', 'open': 'MENU', 'close': '@x-close BACK'}])
        with self.assertRaises(ValueError):
            gate.validate([{'name': 'x', 'open': '@x-open MENU', 'close': '@y-close BACK'}])

    def test_duplicates_and_unknown_thresholds_are_refused(self):
        e = {'name': 'x', 'open': '@x-open MENU', 'close': '@x-close BACK'}
        with self.assertRaises(ValueError):
            gate.validate([e, dict(e)])
        with self.assertRaises(ValueError):
            gate.validate([dict(e, thresholds={'nonsense': 1})])

    def test_find_names_the_unknown(self):
        with self.assertRaisesRegex(ValueError, 'nope'):
            gate.find(['nope'])
        self.assertEqual([s['name'] for s in gate.find(['home-idle', 'kit-gallery'])], ['home-idle', 'kit-gallery'])

    def test_overrides_keep_the_other_thresholds(self):
        s = gate.find(['home-idle'])[0]
        self.assertIsNone(s['thresholds']['first_frame_ms'])
        self.assertEqual(s['thresholds']['p90_render_ms'], 3.0)


class Commands(unittest.TestCase):
    def test_cycle_tokens_and_seconds(self):
        s = gate.surface({'name': 'x', 'open': '@x-open MENU', 'close': '@x-close BACK', 'settle_s': 2})
        self.assertEqual(gate.cycle_tokens(s), ['@x-open', 'MENU', 's2', '@x-close', 'BACK', 's2'])
        self.assertAlmostEqual(gate.cycle_seconds(s), 0.05 + 2 + 0.05 + 2 + 0.3)

    def test_held_key_and_repeat_token_durations(self):
        self.assertEqual(gate.token_seconds('RIGHT:1500'), 1.5)
        self.assertAlmostEqual(gate.token_seconds('RIGHTx8@0.35'), 8 * 0.40)
        self.assertEqual(gate.token_seconds('s0.4'), 0.4)
        self.assertEqual(gate.token_seconds('@mark'), 0.0)

    def test_one_process_without_reopen(self):
        s = gate.surface({'name': 'x', 'open': '@x-open MENU', 'close': '@x-close BACK', 'settle_s': 1})
        cmd = gate.cycle_command(s, 2)
        self.assertEqual(cmd.count('python3'), 1)
        self.assertEqual(cmd.count('@x-open'), 2)

    def test_reopen_runs_between_cycles(self):
        s = gate.surface({'name': 'x', 'open': '@x-open DOWN', 'close': '@x-close BACK', 'reopen': ['prep'], 'reopen_wait_s': 3})
        cmd = gate.cycle_command(s, 3)
        self.assertEqual(cmd.count('prep'), 3)
        self.assertEqual(cmd.count('sleep 3'), 3)

    def test_split_cycles_is_even_and_complete(self):
        for n in (1, 5, 60, 61, 500):
            parts = gate.split_cycles(n, 60)
            self.assertEqual(sum(parts), n)
            self.assertLessEqual(max(parts), 60)
            self.assertLessEqual(max(parts) - min(parts), 1)
        self.assertEqual(gate.split_cycles(0, 60), [])

    def test_full_capture_plan_stays_inside_the_ring_budget(self):
        s = gate.surface({'name': 'x', 'open': '@x-open MENU', 'close': '@x-close BACK', 'settle_s': 2.5})
        plan = gate.plan_full(s, 3)
        self.assertEqual(sum(plan), 3)
        for n in plan:
            self.assertTrue(n == 1 or n * gate.cycle_seconds(s) <= gate.FULL_TRACE_BUDGET_S)


class Parsing(unittest.TestCase):
    TRACE = '\n'.join([
        '    kb.py-100   [001] ....  10.000000: tracing_mark_write: P:qs-open',
        '    kb.py-100   [001] ....  12.500000: tracing_mark_write: P:qs-close',
        '    kb.py-100   [001] ....  15.000000: tracing_mark_write: P:qs-open',
        '    kb.py-100   [001] ....  17.500000: tracing_mark_write: P:qs-close',
        '    sh-101      [002] ....  20.000000: tracing_mark_write: stop 1',
        '  surface-1     [000] d...  11.000000: drm_vblank_event_delivered: x',
    ])

    def test_markers_skip_other_events(self):
        m = gate.parse_markers(self.TRACE)
        self.assertEqual(len(m), 5)
        self.assertEqual(m[0], (10.0, 'P:qs-open'))

    def test_cycle_ends_use_next_open_then_the_stop(self):
        self.assertEqual(gate.cycle_ends(gate.parse_markers(self.TRACE), 'qs'), [15.0, 20.0])

    def test_cycle_ends_without_stop_use_the_last_marker(self):
        text = '\n'.join(self.TRACE.splitlines()[:4])
        self.assertEqual(gate.cycle_ends(gate.parse_markers(text), 'qs'), [15.0, 17.5])

    def test_rss_series_skips_lines_without_rss(self):
        text = '1.0|x rss=1000\n2.0|compositor gone\n3.0|x rss=1100\nnot a number|rss=5\n'
        self.assertEqual(gate.parse_rss(text), [(1.0, 1000), (3.0, 1100)])


class Rss(unittest.TestCase):
    def series(self, pages):
        return [(float(i), p) for i, p in enumerate(pages)]

    def test_flat_passes(self):
        ends = [float(i) for i in range(1, 11)]
        g = gate.rss_growth(self.series([5000] * 12), ends)
        self.assertTrue(g['ok'])
        self.assertEqual(g['growth_mb'], 0.0)

    def test_growth_beyond_noise_fails(self):
        ends = [float(i) for i in range(1, 11)]
        pages = [5000] * 4 + [5000 + 2560] * 8          # +10 MB after the warm-up cycles
        g = gate.rss_growth(self.series(pages), ends)
        self.assertFalse(g['ok'])
        self.assertGreater(g['growth_mb'], g['allowed_mb'])

    def test_shrink_passes(self):
        ends = [float(i) for i in range(1, 11)]
        g = gate.rss_growth(self.series([6000] * 4 + [5000] * 8), ends)
        self.assertTrue(g['ok'])

    def test_missing_samples_fail(self):
        g = gate.rss_growth([], [1.0, 2.0, 3.0, 4.0])
        self.assertFalse(g['ok'])
        self.assertTrue(math.isnan(g['growth_mb']))
        self.assertFalse(gate.rss_check(g)['ok'])
        self.assertFalse(gate.rss_growth(self.series([5000] * 3), [1.0, 2.0])['ok'])


class Log(unittest.TestCase):
    def test_timestamps_cut_a_rotated_log(self):
        text = '[ 10.0] old line\n[ 50.0] new TypeError\ncontinuation\n'
        lines, filtered = gate.lines_since(text, 20.0)
        self.assertTrue(filtered)
        self.assertEqual(lines, ['[ 50.0] new TypeError', 'continuation'])

    def test_no_timestamps_returns_everything(self):
        lines, filtered = gate.lines_since('a\nb\n', 20.0)
        self.assertFalse(filtered)
        self.assertEqual(lines, ['a', 'b'])

    def test_failures_and_known_noise(self):
        lines = ['fine', 'Binding loop detected for property "x"', 'ShaderEffectSource: recursive',
                 'Controllers/ScreenOffInterfaces/Interfaces.qml:52: TypeError: x']
        bad, ignored = gate.log_failures(lines)
        self.assertEqual(len(bad), 2)
        self.assertEqual(ignored, 1)
        self.assertFalse(gate.log_check(bad, ignored)['ok'])
        self.assertTrue(gate.log_check([], 0)['ok'])


class Windows(unittest.TestCase):
    def evaluate(self, surface='quick-settings', cycles=3, **kw):
        s = gate.find([surface])[0]
        return gate.evaluate_windows(s, report(surface, cycles, **kw), cycles)

    def test_a_good_surface_passes(self):
        self.assertEqual(failed(self.evaluate()), [])

    def test_slow_render_fails_at_3_ms(self):
        self.assertEqual(failed(self.evaluate(p90=3.0)), ['p90_render_ms'])
        self.assertEqual(failed(self.evaluate(p90=2.99)), [])

    def test_p90_is_pooled_over_the_cycles_and_one_bad_window_still_fails(self):
        s = gate.find(['quick-settings'])[0]
        good = [2.0] * 9 + [2.9]
        ws = [window('quick-settings-open', p90=2.9) | {'burst_anim_samples': good} for _ in range(3)]
        ws += [window('quick-settings-close', p90=2.9) | {'burst_anim_samples': good} for _ in range(3)]
        self.assertEqual(failed(gate.evaluate_windows(s, {'windows': ws}, 3)), [])
        # a pooled p90 over the limit fails even though no window is far over
        bad = [2.0] * 8 + [3.4] * 2
        ws2 = [dict(w, burst_anim_samples=bad, burst_anim_p90_ms=3.4) for w in ws]
        self.assertEqual(failed(gate.evaluate_windows(s, {'windows': ws2}, 3)), ['p90_render_ms'])
        # one window at 4 ms fails the window limit while the pool stays under 3
        ws3 = [dict(w) for w in ws]
        ws3[0] = dict(ws3[0], burst_anim_p90_ms=4.2)
        self.assertEqual(failed(gate.evaluate_windows(s, {'windows': ws3}, 3)), ['p90_window_ms'])

    def test_a_first_frame_after_an_idle_gap_is_not_a_drop(self):
        s = gate.find(['quick-settings'])[0]
        r = report('quick-settings', 3)
        for w in r['windows']:
            w['dropped'] = 3
            w['dropped_in_motion'] = 0
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), [])
        r['windows'][0]['dropped_in_motion'] = 1
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['dropped'])

    def test_late_first_frame_and_late_burst_fail(self):
        self.assertEqual(failed(self.evaluate(first=51.0)), ['first_frame_ms'])
        self.assertEqual(failed(self.evaluate(end=181.0)), ['burst_end_ms'])
        self.assertEqual(failed(self.evaluate(first=50.0, end=180.0)), [])

    def test_a_frame_over_20_ms_fails(self):
        self.assertEqual(failed(self.evaluate(over20=1)), ['burst_over20'])
        self.assertEqual(failed(self.evaluate(dropped=1)), ['dropped'])

    def test_a_key_that_was_not_taken_fails(self):
        self.assertIn('windows', failed(self.evaluate(first=None, end=None)))

    def test_a_lost_marker_fails(self):
        s = gate.find(['quick-settings'])[0]
        r = report('quick-settings', 3)
        r['windows'].pop()
        self.assertIn('windows', failed(gate.evaluate_windows(s, r, 3)))

    def test_a_workload_surface_checks_open_windows_only(self):
        s = gate.find(['home-dpad'])[0]
        r = {'windows': [window('home-dpad-open', first=20.0, end=9000.0, anim={'p90': 2.4})] * 3}
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), [])
        r = {'windows': [window('home-dpad-open', anim={'p90': 3.4}, p90=None)] * 3}
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['p90_render_ms'])

    def test_verdict_needs_every_check(self):
        ok = {'name': 'a', 'checks': [gate.check('x', True, 1, '')]}
        bad = {'name': 'b', 'checks': [gate.check('y', False, 1, '')]}
        self.assertEqual(gate.verdict([ok]), 'PASS')
        self.assertEqual(gate.verdict([ok, bad]), 'FAIL')
        self.assertEqual(gate.verdict([]), 'FAIL')


if __name__ == '__main__':
    unittest.main()
