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

    def test_the_stock_notification_surfaces_run_only_when_named(self):
        defaults = [e['name'] for e in gate.SURFACES if gate.surface(e)['default']]
        self.assertNotIn('toast', defaults)
        self.assertNotIn('alert', defaults)
        self.assertEqual([s['name'] for s in gate.find(['toast', 'alert'])], ['toast', 'alert'])

    def test_a_luna_token_is_not_a_key_and_takes_no_time(self):
        self.assertEqual(gate.token_seconds('!toast'), 0.0)
        s = gate.find(['toast'])[0]
        self.assertIn('!toast', gate.cycle_tokens(s))

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

    def test_surfaces_over_the_film_get_one_cycle_per_full_capture(self):
        # round 11: three cycles over the film overflowed CPU 0's ring and lost the first cycle's markers
        for name in ('quick-netflix', 'picker-netflix', 'lgqs-netflix'):
            s = gate.find([name], values={'INPUT': '241'})[0]
            self.assertEqual(gate.plan_full(s, 3), [1, 1, 1], name)
        quick = gate.find(['quick'])[0]
        self.assertEqual(gate.plan_full(quick, 3), [3])

    def test_the_lg_baseline_over_the_film_hands_the_gear_back_and_returns_it(self):
        s = gate.find(['lgqs-netflix'])[0]
        self.assertIn('"quickPanel":false', s['setup'][0])
        self.assertIn('"quickPanel":true', s['teardown'][-1])
        self.assertEqual(s['setup'][2:], gate.NETFLIX_SETUP)


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

    def test_a_key_that_finds_the_osd_off_says_when_it_came_back(self):
        import tempfile
        import analyse

        def osd(t):
            return [f'  <idle>-0  [000] d.h1 {t:.6f}: irq_handler_entry: irq=104 name=osd_irq',
                    f'  <idle>-0  [000] d.h1 {t + 0.00001:.6f}: irq_handler_exit: irq=104 ret=handled']
        lines = ['  kb.py-100  [001] ....  10.000000: tracing_mark_write: P:x-open']
        for k in range(60):                     # off at the open, back 1.2 s later and running at 120 Hz through the close
            lines += osd(11.2 + k / 120.0)
        lines += ['  kb.py-100  [001] ....  11.600000: tracing_mark_write: P:x-close',
                  '  sh-101  [001] ....  12.000000: tracing_mark_write: stop 1']
        with tempfile.NamedTemporaryFile('w', suffix='.trace', delete=False) as f:
            f.write('\n'.join(sorted(lines, key=lambda ln: float(ln.split(': ')[0].split()[-1]))) + '\n')
        try:
            ws = {w['name']: w['osd_wake_ms'] for w in analyse.analyse(f.name, 1)['windows']}
        finally:
            os.unlink(f.name)
        self.assertEqual(ws, {'x-open': 1200.0, 'x-close': None})


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

    def test_netflix_reporting_the_instance_the_setup_closed_is_noise(self):
        kill = ('netflix.appmgr INFO {} [CrashReporter] Previous CrashReport : {"shutdown_reason":"normal","reason":"REASON_USER_KILL",'
                '"appStatus":"Terminate","log":"Terminated by User","crashReportClient":"Netflix App"}')
        crash = 'netflix.appmgr INFO {} [CrashReporter] Previous CrashReport : {"reason":"SIGSEGV","crashReportClient":"Netflix App"}'
        self.assertEqual(gate.log_failures([kill, crash]), ([crash], 1))

    def test_lgs_toast_model_on_the_dolby_toast_is_noise_and_other_type_errors_are_not(self):
        toast = ("surface-manager LSM {} file:///usr/lib/qt5/qml/SystemUI/Notification/Services.qml:513: TypeError: Cannot read property "
                 "'id' of undefined")
        ours = "surface-manager LSM {} file:///x/panels/QuickPanel.qml:513: TypeError: Cannot read property 'id' of undefined"
        self.assertEqual(gate.log_failures([toast, ours]), ([ours], 1))


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

    def test_a_late_first_frame_names_a_key_that_found_the_osd_off(self):
        s = gate.find(['quick-netflix'])[0]
        r = report('quick-netflix', 3)
        r['windows'][0].update(first_frame_ms=1479.4, osd_wake_ms=1171.0)
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertFalse(c['first_frame_ms']['ok'])
        self.assertEqual(c['first_frame_ms']['detail'], 'the OSD was off at 1 key(s), on after 1171 ms at worst')
        self.assertEqual(c['burst_end_ms']['detail'], '')

    def test_a_frame_over_20_ms_fails(self):
        self.assertEqual(failed(self.evaluate(over20=1)), ['burst_over20'])
        self.assertEqual(failed(self.evaluate(dropped=1)), ['dropped'])

    def test_a_key_that_was_not_taken_fails(self):
        self.assertIn('windows', failed(self.evaluate(first=None, end=None)))

    def test_under_lgs_compositor_drops_are_held_to_its_measured_floor(self):
        s = gate.find(['quick-settings'])[0]
        r = report('quick-settings', 3)
        for w in r['windows']:
            w['anim_frames'] = 100
            w['dropped_in_motion'] = 10        # 10 % of the frames
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3, lg_compositor=True)), [])
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3, lg_compositor=False)), ['dropped'])
        for w in r['windows']:
            w['dropped_in_motion'] = 20        # 20 % is over the 17 % floor
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3, lg_compositor=True)), ['dropped_share'])

    def test_an_idle_surface_without_a_key_is_not_a_dead_key(self):
        s = gate.find(['home-idle'])[0]
        r = {'windows': [window('home-idle-open', first=None, end=None, anim={'p90': 2.4}) for _ in range(3)]}
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), [])

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

    def test_no_command_on_the_set_calls_the_tool_by_a_bare_name(self):
        # the tool is not on the set's PATH: the gallery's setup failed with "lumatv: not found" on the first run
        for s in gate.SURFACES:
            for key in ('setup', 'teardown', 'reopen'):
                for cmd in s.get(key, []):
                    self.assertFalse(cmd.startswith('lumatv '), cmd)
                    if 'theme set' in cmd:
                        self.assertTrue(cmd.startswith(gate.TOOL), cmd)


class Edges(unittest.TestCase):
    """What a key that acts on its release, a luna call and the wallpaper's tick change in a run."""

    def test_quick_settings_is_timed_from_the_release_and_the_key_goes_down_first(self):
        s = gate.find(['quick-settings'])[0]
        self.assertEqual(s['act'], 'release')
        toks = gate.cycle_tokens(s)
        self.assertEqual(toks[:4], ['MENU+', 's0.05', '@quick-settings-open', 'MENU-'])
        # the marker is between the press and the release, so the press is not in the measured window
        self.assertLess(toks.index('MENU+'), toks.index('@quick-settings-open'))
        self.assertLess(toks.index('@quick-settings-open'), toks.index('MENU-'))

    def test_a_press_surface_keeps_its_tokens(self):
        s = gate.find(['home-dpad'])[0]
        self.assertEqual(gate.open_tokens(s), s['open'].split())

    def test_act_release_needs_one_plain_key(self):
        bad = {'name': 'x', 'act': 'release', 'open': '@x-open RIGHTx3', 'close': '@x-close'}
        with self.assertRaises(ValueError):
            gate.validate([bad])
        bad = {'name': 'x', 'act': 'sideways', 'open': '@x-open OK', 'close': '@x-close'}
        with self.assertRaises(ValueError):
            gate.validate([bad])

    def test_delivery_needs_a_luna_call(self):
        with self.assertRaises(ValueError):
            gate.validate([{'name': 'x', 'delivery': True, 'open': '@x-open OK', 'close': '@x-close'}])
        self.assertTrue(gate.surface([e for e in gate.SURFACES if e['name'] == 'alert'][0])['delivery'])

    TRACE = '\n'.join([
        '  <...>-1 [001] .... 100.000: tracing_mark_write: P:alert-open',
        '  <...>-1 [001] .... 100.060: tracing_mark_write: Q:alert-sent',
        '  <idle>-0 [000] dNH2 100.030: drm_vblank_event_delivered: file=0, crtc=0, seq=1',     # before the call returned
        '  <idle>-0 [000] dNH2 100.100: drm_vblank_event_delivered: file=0, crtc=0, seq=2',
        '  <idle>-0 [000] dNH2 100.117: drm_vblank_event_delivered: file=0, crtc=0, seq=6',
        '  <idle>-0 [000] dNH2 100.134: drm_vblank_event_delivered: file=0, crtc=0, seq=3',
        '  <idle>-0 [000] dNH2 100.151: drm_vblank_event_delivered: file=0, crtc=0, seq=4',
        '  <...>-1 [001] .... 105.000: tracing_mark_write: P:alert-open',                        # a call that never returned
        '  <idle>-0 [000] dNH2 105.050: drm_vblank_event_delivered: file=0, crtc=0, seq=5',
        ''])

    def test_delivery_times_the_first_frame_from_the_return_of_luna_send(self):
        rep = {'windows': [{'name': 'alert-open', 'first_frame_ms': 30.0, 'burst_end_ms': None, 'burst_over20': 0},
                           {'name': 'alert-open', 'first_frame_ms': 50.0, 'burst_end_ms': 50.0, 'burst_over20': 0}]}
        gate.apply_delivery(rep, self.TRACE, 'alert')
        first, second = rep['windows']
        self.assertEqual(first['sent_ms'], 60.0)
        self.assertAlmostEqual(first['first_frame_ms'], 40.0, places=1)       # 100.100 - 100.060
        self.assertAlmostEqual(first['burst_end_ms'], 91.0, places=1)         # the burst runs to 100.151
        self.assertIsNone(second['sent_ms'])
        self.assertIsNone(second['first_frame_ms'])                           # no return: dead, and the gate says so

    def test_a_delivery_window_without_a_return_fails_the_surface(self):
        s = gate.find(['alert'])[0]
        r = report('alert', 3)
        r['windows'][0]['first_frame_ms'] = None
        self.assertIn('windows', failed(gate.evaluate_windows(s, r, 3)))

    class FakeRig:
        def __init__(self, motion):
            self.motion, self.calls = motion, []

        def push_tools(self):
            pass

        def motion_enabled(self):
            return self.motion

        def set_motion(self, enabled):
            self.calls.append(enabled)

    def run_gate(self, motion, names):
        rig = self.FakeRig(motion)
        orig = gate.run_surface
        gate.run_surface = lambda rig_, s, tag, soak, log: {'name': s['name'], 'checks': [], 'log_lines': []}
        try:
            gate.run_gate(rig, gate.find(names), 3, 't', log=object())
        finally:
            gate.run_surface = orig
        return rig

    def test_the_tick_is_off_while_a_still_surface_runs_and_back_as_the_owner_had_it(self):
        # idle Home first (the tick as it is), then two still surfaces: off once, back on at the end
        rig = self.run_gate(True, ['home-idle', 'quick-settings', 'kit-gallery'])
        self.assertEqual(rig.calls, [False, True])
        rig = self.run_gate(True, ['quick-settings'])
        self.assertEqual(rig.calls, [False, True])

    def test_idle_home_is_measured_with_the_tick_the_owner_has(self):
        rig = self.run_gate(True, ['home-idle'])
        self.assertEqual(rig.calls, [])
        rig = self.run_gate(False, ['quick-settings', 'home-idle'])      # the owner has motion off: nothing to change for them
        self.assertEqual(rig.calls, [])
        rig = self.run_gate(None, ['quick-settings'])                    # the file says nothing: on, the shipped default
        self.assertEqual(rig.calls, [False, True])

    def test_a_failed_surface_still_gives_the_tick_back(self):
        rig = self.FakeRig(True)
        orig = gate.run_surface
        def boom(*a):
            raise RuntimeError('x')
        gate.run_surface = boom
        try:
            with self.assertRaises(RuntimeError):
                gate.run_gate(rig, gate.find(['quick-settings']), 3, 't', log=object())
        finally:
            gate.run_surface = orig
        self.assertEqual(rig.calls, [False, True])


class QuickAndPicker(unittest.TestCase):
    """LumaTV's quick panel and input picker, over Home and over playback: what the gate presses and measures, and what it needs to be told."""

    NAMES = ['quick', 'picker', 'quick-netflix', 'picker-netflix', 'quick-hdmi', 'picker-hdmi']
    VALUES = {'INPUT': '227', 'HDMI': '3'}

    def test_they_run_when_named_and_not_in_the_default_gate(self):
        defaults = [e['name'] for e in gate.SURFACES if gate.surface(e)['default']]
        for n in self.NAMES:
            self.assertNotIn(n, defaults)
        self.assertEqual([s['name'] for s in gate.find(self.NAMES, values=self.VALUES)], self.NAMES)
        self.assertTrue(gate.validate(gate.SURFACES))

    def test_the_bar_is_the_plans_and_nothing_is_switched_off(self):
        for s in gate.find(self.NAMES, values=self.VALUES):
            self.assertEqual(s['thresholds'], gate.THRESHOLDS, s['name'])
            self.assertEqual((s['thresholds']['first_frame_ms'], s['thresholds']['burst_end_ms'], s['thresholds']['burst_over20'],
                              s['thresholds']['p90_render_ms'], s['thresholds']['dropped']), (50.0, 180.0, 0, 3.0, 0), s['name'])
            self.assertTrue(s['transition'] and s['soak'] and s['still'], s['name'])      # the close is measured too, the RSS is watched, the tick is off

    def test_the_gear_is_timed_from_its_release_and_the_input_key_from_its_press(self):
        # LG swallows the gear's press and replays a short press when the key comes up; the Input key opens on its press, LG's picker and ours
        quick = gate.find(['quick'], values=self.VALUES)[0]
        self.assertEqual(gate.cycle_tokens(quick)[:4], ['MENU+', 's0.05', '@quick-open', 'MENU-'])
        self.assertEqual(gate.token_seconds('227+'), 0.0)
        for name in ('picker', 'picker-netflix', 'picker-hdmi'):
            picker = gate.find([name], values=self.VALUES)[0]
            self.assertEqual(gate.cycle_tokens(picker)[:2], ['@%s-open' % name, '227'], name)

    def test_the_picker_needs_the_input_keys_code_and_says_how_to_find_it(self):
        with self.assertRaisesRegex(ValueError, r'picker needs .*kb\.py --learn.*--input-key'):
            gate.find(['picker'])
        with self.assertRaises(ValueError):
            gate.find(['picker-netflix'], values={'INPUT': ''})
        # the gear needs nothing
        self.assertEqual(gate.find(['quick', 'quick-netflix'], values={})[0]['open'], '@quick-open MENU')

    def test_the_hdmi_surfaces_name_the_input_they_put_in_front(self):
        s = gate.find(['quick-hdmi'], values={'HDMI': '2'})[0]
        self.assertIn('"id":"com.webos.app.hdmi2"', s['setup'][0])
        self.assertIn('/launch', s['setup'][0])
        with self.assertRaisesRegex(ValueError, 'hdmi'):
            gate.find(['picker-hdmi'], values={'INPUT': '227'})

    def test_the_film_is_launched_by_its_deep_link_and_played_with_ok_and_the_set_is_left_on_home(self):
        s = gate.find(['quick-netflix'], values={})[0]
        setup = '\n'.join(s['setup'])
        self.assertIn('closeByAppId', s['setup'][0])
        self.assertIn('m=https://www.netflix.com/watch/80141336&source_type=4', setup)
        self.assertIn('python3 /tmp/kb.py OK', setup)
        self.assertEqual(gate.KB, '/tmp/kb.py')
        teardown = '\n'.join(s['teardown'])
        self.assertIn('closeByAppId', teardown)
        self.assertIn('com.webos.app.home', teardown)
        for e in (s, gate.find(['quick-hdmi'], values={'HDMI': '3'})[0]):
            for cmd in e['setup'] + e['teardown']:
                self.assertFalse(cmd.startswith('lumatv '), cmd)

    def test_the_gate_stops_with_a_message_before_touching_the_set(self):
        # an unfilled placeholder is an argument error, not a run that presses a key that is not there
        import contextlib
        import io
        err = io.StringIO()
        with contextlib.redirect_stderr(err), self.assertRaises(SystemExit):
            gate.main(['--surfaces', 'picker'])
        self.assertIn('--input-key', err.getvalue())

    def test_kb_can_learn_a_key_without_a_device_to_inject_into(self):
        import subprocess
        kb = os.path.join(os.path.dirname(HERE), 'docs', 'research', 'perf', 'kb.py')
        r = subprocess.run([sys.executable, kb, '--learn', '0'], capture_output=True, text=True, timeout=30)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout, '')


if __name__ == '__main__':
    unittest.main()
