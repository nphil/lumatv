"""Run with: python3 -m unittest discover -s tests -p 'test_*.py'

docs/research/perf/gate.py: the surfaces table, the cycle commands, the trace and log parsing, the
RSS rule and the checks against a window report. Everything here is pure; the set is not touched.
"""
import math
import os
import sys
import unittest
from unittest.mock import Mock, patch

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
        self.assertEqual([s['name'] for s in gate.find(['home-idle', 'picker'], values={'INPUT': '241'})], ['home-idle', 'picker'])
        with self.assertRaisesRegex(ValueError, 'kit-gallery'):
            gate.find(['kit-gallery'])
        for name in ('overlay-netflix', 'overlay-hdmi'):
            with self.assertRaisesRegex(ValueError, 'plugin-broker records.*no card injection'):
                gate.find([name])

    def test_overrides_keep_the_other_thresholds(self):
        s = gate.find(['home-idle'])[0]
        self.assertIsNone(s['thresholds']['first_frame_ms'])
        self.assertEqual(s['thresholds']['p90_render_ms'], 3.0)

    def test_no_cycle_is_longer_than_a_full_capture_can_hold(self):
        # the stock toast's two 8 s settles made 16.3 s: past the ring's 13 s, the first markers of its captures were gone
        for e in gate.SURFACES:
            self.assertLessEqual(gate.cycle_seconds(gate.surface(e)), gate.FULL_CAPTURE_MAX_S, e['name'])
        long = {'name': 'x', 'open': '@x-open !toast', 'close': '@x-close', 'settle_s': 8, 'delivery': True}
        with self.assertRaisesRegex(ValueError, r'16\.3 s.*overflows the trace ring'):
            gate.validate([long])
        gate.validate([dict(long, settle_s=7, close_settle_s=0.5)])

    def test_the_toast_waits_for_its_timer_and_not_after_the_close(self):
        s = gate.find(['toast'])[0]
        self.assertEqual(gate.cycle_tokens(s), ['@toast-open', '!toast', 's7', '@toast-close', 's0.5'])
        self.assertAlmostEqual(gate.cycle_seconds(s), 7.8)
        self.assertEqual(gate.plan_full(s, 3), [1, 1, 1])
        alert = gate.find(['alert'])[0]
        self.assertEqual(gate.cycle_tokens(alert)[-1], 's2.5', 'a surface that gives no close_settle_s waits as long after the close as after the open')

    def test_the_toast_is_raised_by_the_compositors_control_socket_with_a_text_of_its_own(self):
        # kb.py cannot be imported without the set's key device: read what it sends. LG's createToast stalled the set for 20 minutes
        # in round 26: the gate raises LumaTV's toast through `ctl shell show toast`.
        with open(os.path.join(os.path.dirname(gate.__file__), 'kb.py')) as f:
            source = f.read()
        self.assertNotIn('luna://com.webos.notification/createToast', source)
        self.assertIn("'toast': ['shell', 'show', 'toast', 'A_toast_for_the_gate_{N}|Placement_check']", source)
        self.assertIn("word.replace('{N}', n)", source)


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


class CaptureAccess(unittest.TestCase):
    def test_evdev_measurements_never_read_the_ha_token(self):
        import run
        with patch.object(run.Set, 'connect'), patch.object(run.Set, 'sh', return_value=(0, '17', '')):
            with patch('builtins.open', side_effect=AssertionError('unexpected file read')):
                rig = run.Set()
                rig.kbkey('MENU')
        self.assertIsNone(rig.tok)

    def test_gate_json_keeps_the_frame_and_osd_evidence(self):
        for key in ('burst_first_render_ms', 'burst_anim_samples', 'irq_counts', 'gaps'):
            self.assertIn(key, gate.WINDOW_KEYS)


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

    def test_an_apps_drawing_inside_a_burst_is_counted(self):
        import tempfile
        import analyse
        for comm in ('surface-manager', 'ld-linux-aarch6'):       # LG's compositor, and LumaTV's: its loop's threads are named after its loader
            lines = ['  kb.py-100  [001] ....  10.000000: tracing_mark_write: P:x-open',
                     '  <idle>-0  [002] d...  10.001000: sched_switch: prev_comm=swapper prev_pid=0 prev_prio=120 prev_state=R ==> next_comm=netflix next_pid=500 next_prio=120',
                     f'  <idle>-0  [003] d...  10.001500: sched_switch: prev_comm=swapper prev_pid=0 prev_prio=120 prev_state=R ==> next_comm={comm} next_pid=7 next_prio=120',
                     '  netflix-500  [002] ....  10.150000: mali_job_slots_event: gpu=0 event=16842752 tgid=500 pid=500 job_id=1',
                     '  netflix-500  [002] ....  10.200000: mali_job_slots_event: gpu=0 event=33619968 tgid=500 pid=500 job_id=1',
                     f'  {comm}-7  [003] ....  10.210000: mali_job_slots_event: gpu=0 event=16842752 tgid=7 pid=7 job_id=2',
                     f'  {comm}-7  [003] ....  10.212000: mali_job_slots_event: gpu=0 event=33619968 tgid=7 pid=7 job_id=2',
                     '  kb.py-100  [001] ....  10.600000: tracing_mark_write: P:x-close',
                     '  sh-101  [001] ....  11.000000: tracing_mark_write: stop 1']
            lines += ['  <idle>-0  [000] d.h1  %.6f: drm_vblank_event_delivered: file=0, crtc=0, seq=%d' % (10.0167 + 0.0167 * k, k) for k in range(18)]
            with tempfile.NamedTemporaryFile('w', suffix='.trace', delete=False) as f:
                f.write('\n'.join(sorted(lines, key=lambda ln: float(ln.split(': ')[0].split()[-1]))) + '\n')
            try:
                ws = {w['name']: w['burst_app_jobs'] for w in analyse.analyse(f.name, 7)['windows']}
            finally:
                os.unlink(f.name)
            self.assertEqual(ws, {'x-open': 1, 'x-close': 0}, comm)          # the app's job, not the compositor's

    def test_a_ring_that_lost_its_oldest_events_leaves_the_windows_it_began_before_unread(self):
        # 2026-11-04: CPU 0, where every device interrupt lands, overran its ring and began 2 to 5 s into 27 of 40 captures; the OSD interrupt
        # "came back" where CPU 0's events began, and the first frame after a key "took" 474 to 1323 ms
        import tempfile
        import analyse

        def osd(t):
            return [f'  <idle>-0  [000] d.h1 {t:.6f}: irq_handler_entry: irq=104 name=osd_irq',
                    f'  <idle>-0  [000] d.h1 {t + 0.00001:.6f}: irq_handler_exit: irq=104 ret=handled']
        lines = ['  capture.sh-9  [002] ....  10.000000: tracing_mark_write: start x',
                 '  kb.py-100  [001] ....  11.000000: tracing_mark_write: P:x-open',
                 '  kb.py-100  [001] ....  13.000000: tracing_mark_write: P:x-close',
                 '  sh-101  [001] ....  14.000000: tracing_mark_write: stop 1']
        for k in range(100):                    # CPU 0's first event is at 12.0 s
            lines += osd(12.0 + k / 100.0)
        lines += ['  <idle>-0  [000] d.h1  13.010000: drm_vblank_event_delivered: file=0, crtc=0, seq=1']
        with tempfile.NamedTemporaryFile('w', suffix='.trace', delete=False) as f:
            f.write('# tracer: nop\n# entries-in-buffer/entries-written: 600/1000   #P:4\n#\n')
            f.write('\n'.join(sorted(lines, key=lambda ln: float(ln.split(': ')[0].split()[-1]))) + '\n')
        try:
            report = analyse.analyse(f.name, 1)
        finally:
            os.unlink(f.name)
        self.assertEqual((report['events_lost'], report['complete_from_s']), (400, 2.0))
        opened, closed = report['windows']
        self.assertEqual((opened['trace_from_ms'], opened['osd_wake_ms'], opened['first_frame_ms']), (1000.0, None, None))
        self.assertEqual((closed['trace_from_ms'], closed['osd_wake_ms'], closed['first_frame_ms']), (None, None, 10.0))

    def test_a_cpu_that_came_up_in_the_capture_does_not_say_the_ring_lost_anything(self):
        import analyse
        ev = [(10.0, 1, 'sched_switch', 1, 'x', ''), (10.0, 2, 'sched_switch', 1, 'x', ''), (12.0, 0, 'irq_handler_entry', 1, 'x', ''),
              (13.0, 3, 'cpuhp_enter', 1, 'x', '')]
        self.assertEqual(analyse.complete_from(ev, 5), 12.0)
        self.assertIsNone(analyse.complete_from(ev, 0), 'a ring that lost nothing is whole from the start')


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
        bare = "surface-manager LSM {} file:///usr/lib/qt5/qml/SystemUI/Notification/Services.qml:513: TypeError: Type error"
        other = "surface-manager LSM {} file:///usr/lib/qt5/qml/SystemUI/Notification/Services.qml:513: TypeError: x is not a function"
        self.assertEqual(gate.log_failures([toast, bare, other, ours]), ([other, ours], 2))


    VOICE = ('2026-10-28T20:25:52.340589Z [23544.986540980] user.info com.webos.app.voice [] voice voice_app {} (null), '
             'file:///usr/palm/applications/com.webos.app.voice/qml/Views/SearchView.qml:294:13: Unable to assign [undefined] to QQuickItem*')

    def test_another_apps_qml_error_is_not_ours_to_fail_a_surface_for(self):
        # round 15: LG Search's own warning failed keyboard-luma
        self.assertEqual(gate.log_failures([self.VOICE]), ([], 1))
        for program in ('org.nphil.lumatv.shell', 'surface-manager', 'qml-runner', 'lumatv-compositor'):
            ours = self.VOICE.replace('com.webos.app.voice', program)
            self.assertEqual(gate.log_failures([ours]), ([ours], 0), program)
        # a line whose program cannot be read is counted, as before
        bare = 'file:///x/Panel.qml:12:3: Unable to assign [undefined] to QQuickItem*'
        self.assertEqual(gate.log_failures([bare]), ([bare], 0))

    def test_a_crash_or_a_lumatv_line_counts_from_any_process(self):
        crash = '2026-10-28T20:25:52Z [100.0] user.info com.webos.app.voice [] voice {} the app crash report: SIGSEGV'
        luma = '2026-10-28T20:25:52Z [100.0] user.info com.webos.app.voice [] voice {} [LumaKeys] bridge failed'
        both = self.VOICE + ' crash'
        self.assertEqual(gate.log_failures([crash, luma, both]), ([crash, luma, both], 0))

    def test_the_stock_noise_list_still_applies_to_our_own_programs(self):
        toast = ('2026-10-28T20:25:52Z [100.0] user.info surface-manager [] LSM {} file:///usr/lib/qt5/qml/SystemUI/Notification/Services.qml:513: '
                 "TypeError: Cannot read property 'id' of undefined")
        self.assertEqual(gate.log_failures([toast]), ([], 1))

    def test_the_shells_slides_are_read_from_the_log_lines(self):
        lines = ['x [FrameMotion] 17 steps in 150 ms for 110', 'noise', 'y user.info org.nphil.lumatv.shell [FrameMotion] 9 steps in 97 ms for 90']
        self.assertEqual(gate.slides_in(lines), [(17, 150, 110), (9, 97, 90)])
        self.assertEqual(gate.slides_in(['nothing']), [])
        self.assertEqual(gate.slides_summary(gate.slides_in(lines + lines)), '4 slide(s), fewest steps 9: 17 steps in 150 ms for 110; 9 steps in 97 ms for 90')
        r = {'name': 'quick', 'checks': [], 'log_lines': [], 'slides': gate.slides_in(lines)}
        self.assertIn('      slides: 2 slide(s), fewest steps 9:', gate.format_lines([r]))
        self.assertNotIn('slides', gate.format_lines([dict(r, slides=[])]))

    def test_a_surface_keeps_the_slides_and_counts_nobody_elses_qml_errors(self):
        rig = Mock()
        rig.compositor_pid.return_value = 17
        rig.capture.side_effect = RuntimeError('capture failed')
        rig.sh.return_value = (0, '', '')
        log = Mock(mark=Mock(return_value={}), since=Mock(return_value=([self.VOICE, 'a [FrameMotion] 17 steps in 150 ms for 110'], True)))
        res = gate.run_surface(rig, gate.find(['home-dpad'])[0], 't', 0, log)
        self.assertEqual(res['slides'], [])        # the capture failed before the first scan: nothing yet
        rig.capture.side_effect = None
        rig.stop.return_value = ('/dev/null', '/dev/null')
        one = {'trace': 't', 'windows': [window('home-dpad-open'), window('home-dpad-open')]}
        rig.analyse.return_value = one
        rig.lg_compositor.return_value = False
        res = gate.run_surface(rig, gate.find(['home-dpad'])[0], 't', 0, log)
        self.assertEqual(res['slides'], [(17, 150, 110)])
        self.assertEqual((res['log_lines'], res['log_ignored']), ([], 1))

    def test_the_log_since_a_rotation_reads_the_rotated_files_too_oldest_first(self):
        rig = Mock()
        answers = iter([(0, '4096 99', ''), (0, 'x [ 10.0] before\ny [ 55.0] [FrameMotion] 17 steps in 150 ms for 110\nz [ 90.0] after\n', '')])
        rig.sh.side_effect = lambda cmd, timeout=120: next(answers)
        lines, filtered = gate.LogWatch(rig).since({'size': 9000, 'inode': '12', 'uptime': 50.0})
        self.assertEqual(lines, ['y [ 55.0] [FrameMotion] 17 steps in 150 ms for 110', 'z [ 90.0] after'])
        self.assertTrue(filtered)
        command = rig.sh.call_args.args[0]
        self.assertIn('ls -tr /var/log/messages /var/log/messages.*', command)
        self.assertIn('*.gz', command)


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

    def test_a_window_whose_start_the_trace_lost_is_named_and_not_read(self):
        s = gate.find(['quick-settings'])[0]
        r = report('quick-settings', 3)
        r['windows'][0].update(first_frame_ms=None, burst_end_ms=None, osd_wake_ms=None, burst_over20=0, trace_from_ms=2200.0)
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['trace_complete'], 'the other five windows are read, the lost one is not a dead key')
        self.assertEqual(c['trace_complete']['value'], 1)
        self.assertIn('1 of 6 windows began before every CPU', c['trace_complete']['detail'])
        self.assertTrue(c['trace_complete']['detail'].endswith('whole 2200 ms after the key at worst'))
        r['windows'][0]['dropped'] = 7          # what the trace counted of a window it lost the start of is not read ...
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['trace_complete'])
        for w in r['windows']:                  # ... and the compositor's own count is, for every window
            w.update(ring_dropped=0, ring_judged=5)
        r['windows'][0]['ring_dropped'] = 1
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['dropped', 'trace_complete'])

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

    def test_delivery_is_timed_from_the_compositors_marker_when_it_wrote_one(self):
        trace = '\n'.join([
            '  <...>-1 [001] .... 100.000: tracing_mark_write: P:toast-open',
            '  <...>-1 [001] .... 100.050: tracing_mark_write: Q:toast-sent',                     # luna-send returned
            '  <...>-1 [001] .... 100.900: tracing_mark_write: Q:toast-delivered',                # the service's push reached the compositor
            '  <idle>-0 [000] dNH2 100.300: drm_vblank_event_delivered: file=0, crtc=0, seq=1',   # an idle tick between the two
            '  <idle>-0 [000] dNH2 100.925: drm_vblank_event_delivered: file=0, crtc=0, seq=2',
            '  <idle>-0 [000] dNH2 100.942: drm_vblank_event_delivered: file=0, crtc=0, seq=3',
            '  <...>-1 [001] .... 108.000: tracing_mark_write: P:toast-open',                     # a window with the return only
            '  <...>-1 [001] .... 108.050: tracing_mark_write: Q:toast-sent',
            '  <idle>-0 [000] dNH2 108.100: drm_vblank_event_delivered: file=0, crtc=0, seq=4',
            ''])
        rep = {'windows': [{'name': 'toast-open', 'first_frame_ms': 1.0, 'burst_end_ms': None, 'burst_over20': 0},
                           {'name': 'toast-open', 'first_frame_ms': 1.0, 'burst_end_ms': None, 'burst_over20': 0}]}
        gate.apply_delivery(rep, trace, 'toast')
        first, second = rep['windows']
        self.assertEqual((first['sent_ms'], first['delivered_ms']), (50.0, 900.0))
        self.assertAlmostEqual(first['first_frame_ms'], 25.0, places=1)        # 100.925 - 100.900, not the tick at 100.300 nor luna-send's 100.050
        self.assertAlmostEqual(first['burst_end_ms'], 42.0, places=1)
        self.assertEqual((second['sent_ms'], second['delivered_ms']), (50.0, None))
        self.assertAlmostEqual(second['first_frame_ms'], 50.0, places=1)       # under LG's compositor: from luna-send's return

    def test_a_panel_the_compositor_was_asked_for_says_when(self):
        trace = '\n'.join([
            '  <...>-1 [001] .... 100.000: tracing_mark_write: P:picker-open',
            '  <...>-1 [001] .... 100.051: tracing_mark_write: Q:picker-show',
            '  <...>-1 [001] .... 101.500: tracing_mark_write: P:picker-close',
            '  <...>-1 [001] .... 103.000: tracing_mark_write: P:picker-open',
            '  <...>-1 [001] .... 103.030: tracing_mark_write: Q:picker-show',
            '  <...>-1 [001] .... 104.500: tracing_mark_write: P:picker-close',
            '  <...>-1 [001] .... 106.000: tracing_mark_write: stop x',
            ''])
        names = ['picker-open', 'picker-close', 'picker-open', 'picker-close']
        rep = {'windows': [{'name': n} for n in names]}
        gate.annotate_shows(rep, trace, 'picker')
        self.assertEqual([w['show_ms'] for w in rep['windows']], [51.0, None, 30.0, None])
        # under LG's compositor there is no such marker: the windows say so
        rep = {'windows': [{'name': n} for n in names]}
        gate.annotate_shows(rep, trace.replace('Q:picker-show', 'Q:other'), 'picker')
        self.assertEqual([w['show_ms'] for w in rep['windows']], [None] * 4)

    def test_a_toast_without_markers_says_the_trace_lost_them(self):
        s = gate.find(['toast'])[0]
        r = {'windows': [window('toast-open', first=None, end=None) | {'sent_ms': None, 'delivered_ms': None} for _ in range(2)]}
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertEqual(c['windows']['value'], '2/3')
        self.assertIn('no frame after the key in toast-open, toast-open', c['windows']['detail'])
        self.assertIn("neither luna-send's return nor the compositor's delivery marker", c['windows']['detail'])
        # a toast that was marked but never drawn is not blamed on the trace
        r = {'windows': [window('toast-open', first=None, end=None) | {'sent_ms': 40.0, 'delivered_ms': None} for _ in range(3)]}
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertEqual(c['windows']['detail'], 'no frame after the key in toast-open, toast-open, toast-open')

    def test_the_first_frame_check_says_when_the_shell_was_asked(self):
        s = gate.find(['picker'], values={'INPUT': '241'})[0]
        r = report('picker', 3)
        r['windows'][0].update(first_frame_ms=82.0, show_ms=50.5)
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertFalse(c['first_frame_ms']['ok'])
        self.assertEqual(c['first_frame_ms']['detail'], 'the compositor asked the shell 50.5 ms after the key at worst')

    def test_a_delivery_window_without_a_return_fails_the_surface(self):
        s = gate.find(['alert'])[0]
        r = report('alert', 3)
        r['windows'][0]['first_frame_ms'] = None
        self.assertIn('windows', failed(gate.evaluate_windows(s, r, 3)))

    class FakeRig:
        def __init__(self, motion, age=None, keys=('/dev/input/event10', True), hz=None):
            self.motion, self.calls, self.age, self.woken, self.keys, self.hz = motion, [], age, False, keys, hz

        def compositor_age_s(self):
            return self.age

        def wake(self):
            self.woken = True

        def push_tools(self):
            pass

        def keys_read(self):
            return self.keys

        def panel_hz(self):
            return self.hz

        def motion_state(self):
            return dict(self.motion) if isinstance(self.motion, dict) else {path: self.motion for path in gate.MOTION_SWITCHES}

        def set_motion(self, changes):
            self.calls.append(dict(changes))

    def run_gate(self, motion, names):
        rig = self.FakeRig(motion)
        orig = gate.run_surface
        gate.run_surface = lambda rig_, s, tag, soak, log: {'name': s['name'], 'checks': [], 'log_lines': []}
        try:
            gate.run_gate(rig, gate.find(names, values={'INPUT': '241'}), 3, 't', log=object())
        finally:
            gate.run_surface = orig
        return rig

    OFF = {'wallpaperMotion.enabled': False, 'weather.animated': False}
    ON = {'wallpaperMotion.enabled': True, 'weather.animated': True}

    def test_the_motion_is_off_while_a_still_surface_runs_and_back_as_the_owner_had_it(self):
        # idle Home first (the motion as it is), then two still surfaces: off once, back on at the end
        rig = self.run_gate(True, ['home-idle', 'quick-settings', 'picker'])
        self.assertEqual(rig.calls, [self.OFF, self.ON])
        rig = self.run_gate(True, ['quick-settings'])
        self.assertEqual(rig.calls, [self.OFF, self.ON])

    def test_idle_home_is_measured_with_the_motion_the_owner_has(self):
        rig = self.run_gate(True, ['home-idle'])
        self.assertEqual(rig.calls, [])
        rig = self.run_gate(False, ['quick-settings', 'home-idle'])      # the owner has all motion off: nothing to change for them
        self.assertEqual(rig.calls, [])
        rig = self.run_gate(None, ['quick-settings'])                    # the file says nothing: on, the shipped default
        self.assertEqual(rig.calls, [self.OFF, self.ON])

    def test_each_switch_is_judged_on_its_own_a_wallpaper_the_owner_stilled_does_not_keep_the_weather_card_drawing(self):
        # round 23: the owner's file had the wallpaper off and said nothing of the weather card, whose sprites drew every refresh through the whole run
        rig = self.run_gate({'wallpaperMotion.enabled': False, 'weather.animated': None}, ['home-idle', 'quick-settings', 'picker'])
        self.assertEqual(rig.calls, [{'weather.animated': False}, {'weather.animated': True}], 'the weather card off once and back as the shipped default has it')
        self.assertTrue(all('wallpaperMotion.enabled' not in c for c in rig.calls), 'the wallpaper is left as the owner had it')
        rig = self.run_gate({'wallpaperMotion.enabled': True, 'weather.animated': False}, ['quick-settings'])
        self.assertEqual(rig.calls, [{'wallpaperMotion.enabled': False}, {'wallpaperMotion.enabled': True}])

    def test_a_failed_surface_still_gives_the_motion_back(self):
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
        self.assertEqual(rig.calls, [self.OFF, self.ON])

    def test_the_gate_waits_for_a_freshly_started_compositor(self):
        orig = gate.run_surface
        gate.run_surface = lambda rig_, s, tag, soak, log: {'name': s['name'], 'checks': [], 'log_lines': []}
        try:
            for age, slept in ((30.0, [60.0]), (200.0, []), (None, [])):
                rig = self.FakeRig(False, age)
                with patch.object(gate.time, 'sleep') as sleep:
                    gate.run_gate(rig, gate.find(['home-idle']), 3, 't', log=object())
                self.assertEqual([c.args[0] for c in sleep.call_args_list], slept, age)
                self.assertTrue(rig.woken, "Home's saver was not woken before the first surface")
        finally:
            gate.run_surface = orig

    def test_the_gate_stops_when_the_compositor_does_not_read_the_device_its_keys_go_to(self):
        # 2026-11-04: lginput2 made the SSAP key device anew at a wake, the compositor never opened it, the keys went nowhere and the gate timed the saver
        orig = gate.run_surface
        ran = []
        gate.run_surface = lambda rig_, s, tag, soak, log: ran.append(s['name']) or {'name': s['name'], 'checks': [], 'log_lines': []}
        try:
            rig = self.FakeRig(False, keys=('/dev/input/event10', False))
            with self.assertRaisesRegex(RuntimeError, 'event10'):
                gate.run_gate(rig, gate.find(['home-idle']), 3, 't', log=object())
            self.assertEqual((ran, rig.calls), ([], []), 'no surface ran and nothing was changed on the set')
            for keys in (('/dev/input/event10', None), (None, None)):       # no LumaTV compositor, or no device to ask about: nothing to say
                gate.run_gate(self.FakeRig(False, keys=keys), gate.find(['home-idle']), 3, 't', log=object())
            self.assertEqual(ran, ['home-idle'] * 2)
        finally:
            gate.run_surface = orig

    def test_the_gate_stops_when_the_panel_does_not_run_at_the_asked_refresh(self):
        # 2026-11-04: the TV re-timed the panel to 100 Hz for a camera popup, and the gate ran 23 minutes asked for 120
        orig = gate.run_surface
        ran = []
        gate.run_surface = lambda rig_, s, tag, soak, log: ran.append(s['name']) or {'name': s['name'], 'checks': [], 'log_lines': []}
        try:
            with patch.object(gate, 'REFRESH_HZ', 120.0):
                rig = self.FakeRig(False, hz=99.9)
                with self.assertRaisesRegex(RuntimeError, r'99\.9 Hz'):
                    gate.run_gate(rig, gate.find(['home-idle']), 3, 't', log=object())
                self.assertEqual((ran, rig.calls), ([], []), 'no surface ran and nothing was changed on the set')
                for hz in (119.5, None):        # the asked refresh, or a set that lists no vsync interrupt
                    gate.run_gate(self.FakeRig(False, hz=hz), gate.find(['home-idle']), 3, 't', log=object())
            self.assertEqual(ran, ['home-idle'] * 2)
        finally:
            gate.run_surface = orig

    def test_the_panel_rate_is_the_vsync_interrupts_counted_over_the_wait(self):
        rig = object.__new__(gate.Rig)
        rig.sh = Mock(return_value=(0, '1000 1200\n', ''))
        self.assertEqual(rig.panel_hz(2), 100.0)
        rig.sh = Mock(return_value=(0, ' \n', ''))
        self.assertIsNone(rig.panel_hz(2), 'a set with no DE2VSYNC line')

    def test_the_key_device_is_open_when_the_compositors_fds_say_so(self):
        listing = 'lrwx------ 1 root root 64 Nov  4 08:31 21 -> /dev/input/event2\nlrwx------ 1 root root 64 Nov  4 08:31 22 -> /dev/input/event11\n'
        self.assertTrue(gate.device_open_in(listing, '/dev/input/event2'))
        self.assertFalse(gate.device_open_in(listing, '/dev/input/event10'))
        self.assertFalse(gate.device_open_in(listing, '/dev/input/event1'), 'event1 is not event11')
        rig = object.__new__(gate.Rig)
        for answers, expected in ((['/dev/input/event10\n', listing + 'l 23 -> /dev/input/event10\n'], ('/dev/input/event10', True)),
                                  (['/dev/input/event10\n', listing], ('/dev/input/event10', False)),
                                  (['/dev/input/event10\n', 'NO_COMPOSITOR\n'], ('/dev/input/event10', None))):
            rig.sh = Mock(side_effect=[(0, a, '') for a in answers])
            self.assertEqual(rig.keys_read(), expected)
        rig.sh = Mock(return_value=(1, '', 'no SSAP key device'))
        self.assertEqual(rig.keys_read(), (None, None), 'no device to ask about')

    def test_a_capture_where_the_app_drew_is_measured_again(self):
        s = gate.find(['picker-netflix'], values={'INPUT': '241'})[0]
        clean = {'trace': 't', 'windows': [window('picker-netflix-open') | {'burst_app_jobs': 0}, window('picker-netflix-close') | {'burst_app_jobs': 0}]}
        dirty = {'trace': 't', 'windows': [window('picker-netflix-open') | {'burst_app_jobs': 0},
                                           window('picker-netflix-close', end=944.0) | {'burst_app_jobs': 123}]}
        # one dirty capture is measured again; three in a row keep the last, which then fails on its own numbers
        for answers, discarded, traces, ends in (([dirty, clean, clean, clean], 1, 4, [120.0] * 6),
                                                 ([dirty, dirty, dirty, clean, clean], 2, 5, [120.0, 944.0] + [120.0] * 4)):
            rig = Mock()
            rig.compositor_pid.return_value = 17
            rig.sh.return_value = (0, '', '')
            rig.lg_compositor.return_value = True
            rig.stop.return_value = ('/dev/null', '/dev/null')
            rig.analyse.side_effect = answers
            res = gate.run_surface(rig, dict(s, setup=[], teardown=[], soak=False), 't', 0, Mock(since=Mock(return_value=([], True))))
            self.assertEqual((len(res['discarded']), len(res['traces'])), (discarded, traces))
            self.assertEqual(res['discarded'][0]['windows'], [{'name': 'picker-netflix-close', 'burst_end_ms': 944.0, 'burst_app_jobs': 123}])
            self.assertEqual([w['burst_end_ms'] for w in res['windows']], ends)


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

    def test_hdmi_teardown_closes_the_selected_app_then_checks_decoders(self):
        for name in ('quick-hdmi', 'picker-hdmi'):
            for port in ('2', '3'):
                s = gate.find([name], values={'HDMI': port, 'INPUT': '241'})[0]
                self.assertIn('/closeByAppId', s['teardown'][0])
                self.assertIn('com.webos.app.hdmi' + port, s['teardown'][0])
                self.assertTrue(s['inputs_free'])
                rig = Mock()
                rig.compositor_pid.return_value = 17
                rig.sh.return_value = (0, gate.json.dumps({'ok': True, 'inputs': {
                    'ports': [{'port': 0, 'open': False}, {'port': 1, 'open': False}], 'running': []}}), '')
                rig.capture.side_effect = RuntimeError('capture failed')
                result = gate.run_surface(rig, s, 'test', 0, Mock())
                commands = [c.args[0] for c in rig.sh.call_args_list]
                self.assertEqual(commands, s['setup'] + s['teardown'] + [gate.TOOL + ' inputs status'])
                self.assertEqual(failed(result['checks']), ['run'])

    def test_input_cleanup_check_rejects_occupied_missing_and_failed_status(self):
        def status(ports, running=None):
            return gate.json.dumps({'ok': True, 'inputs': {'ports': ports, 'running': [] if running is None else running}})
        closed = [{'port': 0, 'open': False}, {'port': 1, 'open': False}]
        cases = [(0, status(closed), '', True),
                 (0, status([{'port': 1, 'open': True}]), '', False),
                 (0, status(closed, ['com.webos.app.hdmi3']), '', False),
                 (0, status([]), '', False), (0, status([{'port': 1}]), '', False),
                 (0, '{}', '', False), (0, 'not JSON', '', False),
                 (1, status(closed), 'status failed', False)]
        for rc, out, err, expected in cases:
            rig = Mock()
            rig.sh.return_value = (rc, out, err)
            self.assertEqual(gate.inputs_free_check(rig)['ok'], expected, (rc, out))

    def test_failed_teardown_commands_fail_the_surface(self):
        s = gate.find(['quick-hdmi'], values={'HDMI': '3'})[0]
        rig = Mock()
        rig.compositor_pid.return_value = 17
        rig.sh.return_value = (1, '', 'command failed')
        result = gate.run_surface(rig, s, 'test', 0, Mock())
        self.assertEqual(failed(result['checks']), ['inputs_free', 'run'] + ['teardown'] * len(s['teardown']))

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

    def test_kb_names_the_device_it_injects_into_or_says_there_is_none(self):
        import subprocess
        kb = os.path.join(os.path.dirname(HERE), 'docs', 'research', 'perf', 'kb.py')
        r = subprocess.run([sys.executable, kb, '--device'], capture_output=True, text=True, timeout=30)
        if r.returncode == 0:
            self.assertRegex(r.stdout, r'^/dev/input/event\d+\n$')
        else:
            self.assertIn('no SSAP key device', r.stderr)


class LumaTvCompositor(unittest.TestCase):
    """analyse.py reads the compositor's own thread under LumaTV's compositor, whose render burst is a stage of a two-deep frame: half a refresh."""

    def test_the_render_limits_follow_the_refresh(self):
        th = {'p90_render_ms': 3.0, 'p90_window_ms': 4.0}
        with patch.object(gate, 'REFRESH_HZ', 60.0):
            lim = gate.render_limits(th)
        self.assertAlmostEqual(lim['p90_render_ms'], 500.0 / 60.0, places=3)
        self.assertAlmostEqual(lim['p90_window_ms'], 500.0 / 60.0 * 4 / 3, places=3)
        with patch.object(gate, 'REFRESH_HZ', 120.0):
            lim = gate.render_limits(th)
        self.assertAlmostEqual(lim['p90_render_ms'], 500.0 / 120.0, places=3)
        self.assertIsNone(gate.render_limits({'p90_render_ms': None, 'p90_window_ms': None})['p90_render_ms'], 'a surface that turns the check off keeps it off')

    def test_a_render_of_3_4_ms_fails_the_qt_limit_and_passes_the_compositors(self):
        s = gate.find(['quick-settings'])[0]
        r = report('quick-settings', 3, p90=3.4)
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['p90_render_ms'], 'the Qt clients: 3 ms')
        with patch.object(gate, 'REFRESH_HZ', 120.0):
            self.assertEqual(failed(gate.evaluate_windows(s, r, 3, lumatv=True)), [], 'the compositor at 120 Hz: 4.2 ms')
            r = report('quick-settings', 3, p90=4.3)
            self.assertEqual(failed(gate.evaluate_windows(s, r, 3, lumatv=True)), ['p90_render_ms'])


class AnalysePid(unittest.TestCase):
    def test_the_loop_marker_names_the_compositor_and_a_trace_without_one_names_nobody(self):
        sys.path.insert(0, os.path.join(os.path.dirname(HERE), "docs", "research", "perf"))
        import analyse
        ev = [(1.0, 0, 'drm_vblank_event_delivered', 1, 'x', 'seq=1'), (2.0, 1, 'tracing_mark_write', 5, 'capture.sh', 'start gate-x 12.3'),
              (2.1, 1, 'tracing_mark_write', 5, 'capture.sh', 'loop 8969')]
        self.assertEqual(analyse.loop_pid(ev), 8969)
        self.assertIsNone(analyse.loop_pid(ev[:2]))

    def test_lumatvs_wakeups_that_draw_nothing_are_not_renders(self):
        sys.path.insert(0, os.path.join(os.path.dirname(HERE), "docs", "research", "perf"))
        import analyse
        runs = [(1.0, 1.0001), (1.0083, 1.0113), (1.0166, 1.0168)]
        self.assertEqual(analyse.renders(runs, True), [(1.0083, 1.0113)], 'a render is 0.3 ms or more')
        self.assertEqual(analyse.renders(runs, False), runs, "LG's own loop is read as it was")


class PanelRefresh(unittest.TestCase):
    """Round 22 at 120 Hz: a camera popup made the TV re-time the panel to 100 Hz for the rest of the run, and every flip 10.0 ms after the last
    was a dropped frame against the 10 ms of 120 Hz (a thousand on an idle Home)."""

    def report(self, *hz):
        return {'windows': [dict(window('quick-open'), panel_hz=h) for h in hz]}

    def test_a_panel_on_the_asked_refresh_passes_and_one_the_tv_re_timed_fails_with_its_own_rate(self):
        ok = gate.panel_check(self.report(120.1, 119.9, None), 120.0)
        self.assertTrue(ok['ok'], ok)
        bad = gate.panel_check(self.report(120.1, 99.9, 100.1), 120.0)
        self.assertFalse(bad['ok'])
        self.assertEqual(bad['value'], 99.9)
        self.assertIn('2 of 3 windows', bad['detail'])
        self.assertIsNone(gate.panel_check(self.report(None), 60.0), 'no vblanks, no check')
        self.assertTrue(gate.panel_check(self.report(59.9, 60.1), 60.0)['ok'])
        self.assertFalse(gate.panel_check(self.report(50.2), 60.0)['ok'], 'a 100 Hz panel with the OSD at half of it')

    def test_the_checks_of_a_surface_lead_with_it_and_fail_it_only_for_a_report_that_says_so(self):
        s = gate.find(['quick-settings'])[0]
        r = report('quick-settings', 3)
        self.assertNotIn('panel_refresh', [c['name'] for c in gate.evaluate_windows(s, r, 3)])
        for w in r['windows']:
            w['panel_hz'] = 99.9
        with patch.object(gate, 'REFRESH_HZ', 120.0):
            self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['panel_refresh'])


class AnalysePanel(unittest.TestCase):
    def test_a_window_on_another_refresh_is_judged_by_it(self):
        sys.path.insert(0, os.path.join(os.path.dirname(HERE), "docs", "research", "perf"))
        import analyse
        with patch.object(analyse, 'ASKED_HZ', 120.0), patch.object(analyse, 'DROP_MS', 10.0):
            self.assertEqual(analyse.panel_drop_ms([i * 0.01 for i in range(60)], 0.0, 1.0), (12.0, 100.0))
            self.assertEqual(analyse.panel_drop_ms([i / 120.0 for i in range(120)], 0.0, 1.0), (10.0, 120.0))
            self.assertEqual(analyse.panel_drop_ms([i / 120.0 for i in range(5)], 0.0, 1.0), (10.0, None), 'too few vblanks to say')
            idle = [t for k in range(6) for t in (k * 1.5, k * 1.5 + 0.0083)]
            self.assertEqual(analyse.panel_drop_ms(idle, 0.0, 9.0)[1], None, 'idle gaps are not the refresh')
        frames = [i * 0.010 for i in range(1, 11)] + [0.13]
        self.assertEqual(analyse.burst(frames, 0.0, drop_ms=12.0)['over20'], 1, 'the 30 ms interval, not the 10 ms flips')


class RingMotion(unittest.TestCase):
    """The compositor's own count of the refreshes a motion missed (its ring's `anim` flag), where the trace can only guess: round 22 at 60 Hz, home-dpad
    read 17 drops off the trace (every key's response frame after the last key's settle) and the compositor counted none."""
    L = '%.3f L flip %.3f +%.1f ms: frame 3.0 ms (gpu done), late 0 lost 0%s'

    def lines(self, flips):
        return [self.L % (t, t, iv, ' anim' if anim else '') for t, iv, anim in flips]

    def test_only_a_flip_after_a_frame_of_a_motion_is_judged_and_only_a_late_one_is_dropped(self):
        import ringsum
        flips = ringsum.flips(self.lines([(10.000, 16.7, True), (10.017, 16.7, True), (10.050, 33.4, True), (10.067, 16.7, False),
                                          (10.467, 400.0, False), (10.484, 16.7, True), (10.600, 116.0, True)]))
        self.assertEqual(flips[2], (10.05, 33.4, True))
        self.assertEqual(ringsum.motion_drops(flips, 10.0, 11.0, 20.0), (4, 2), 'the 33.4 after a motion and the 116 after the last frame of one; the 400 after a still screen is not judged')
        self.assertEqual(ringsum.motion_drops(flips, 10.04, 10.1, 20.0), (2, 1), 'the window is the flips inside it')
        self.assertEqual(ringsum.motion_drops(flips, 10.0, 11.0, 120.0), (4, 0), 'a threshold of its own')

    def test_a_window_the_ring_does_not_reach_back_to_is_not_judged(self):
        import ringsum
        flips = ringsum.flips(self.lines([(10.0, 16.7, True), (10.017, 16.7, True)]))
        self.assertIsNone(ringsum.motion_drops(flips, 5.0, 11.0, 20.0))
        self.assertIsNone(ringsum.motion_drops([], 0.0, 1.0, 20.0))
        self.assertIsNone(ringsum.burst_drops(flips, 5.0, 20.0))

    def test_a_burst_counts_the_intervals_a_motion_missed_and_not_a_still_screen_before_the_next_key(self):
        import ringsum
        # session 11's home-widgets at 120 Hz: a motion ends (no `anim` after its last frame), the next key's first frame comes 41.7 ms later,
        # 9 ms after the key; then a heavy frame lands a refresh late inside the next motion
        flips = ringsum.flips(self.lines([(10.008, 8.3, True), (10.017, 8.3, True), (10.025, 8.3, False), (10.067, 41.7, True),
                                          (10.075, 8.3, True), (10.092, 16.7, True), (10.100, 8.3, False), (10.200, 100.0, True)]))
        self.assertEqual(ringsum.burst_drops(flips, 10.0, 10.0), 1, 'the 16.7 inside the motion, not the 41.7 after a still frame')
        self.assertEqual(ringsum.burst_drops(flips, 10.06, 10.0), 1, 'from a later mark: its own burst')
        self.assertEqual(ringsum.burst_drops(flips, 10.0, 10.0, gap_ms=40.0), 0, 'the burst ends at an interval over its gap')

    def test_the_gate_judges_a_surface_by_the_rings_count_when_every_window_has_one(self):
        s = gate.find(['quick-settings'])[0]
        r = report('quick-settings', 3)
        for w in r['windows']:
            w.update(dropped=9, dropped_in_motion=9, ring_judged=100, ring_dropped=0)
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertTrue(c['dropped']['ok'], c['dropped'])
        self.assertEqual(c['dropped']['detail'], "the compositor's own count over 600 flips after a frame of a motion")
        r['windows'][2]['ring_dropped'] = 1
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['dropped'])
        del r['windows'][3]['ring_dropped']
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertFalse(c['dropped']['ok'], 'a window without the ring: the trace reading for the surface')
        self.assertEqual(c['dropped']['value'], 54)

    def test_the_gate_reads_a_burst_by_the_rings_motion_when_every_window_has_it(self):
        s = gate.find(['quick-settings'])[0]
        r = report('quick-settings', 3, over20=2)
        for w in r['windows']:
            w['ring_burst_over20'] = 0
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertTrue(c['burst_over20']['ok'], c['burst_over20'])
        self.assertIn('12 with the still screens between keys', c['burst_over20']['detail'])
        r['windows'][0]['ring_burst_over20'] = 1
        self.assertEqual(failed(gate.evaluate_windows(s, r, 3)), ['burst_over20'])
        del r['windows'][1]['ring_burst_over20']
        c = {x['name']: x for x in gate.evaluate_windows(s, r, 3)}
        self.assertEqual(c['burst_over20']['value'], 12, 'a window without the ring: the trace reading')

    def test_attach_gives_each_window_its_count_and_a_missing_ring_changes_nothing(self):
        import json
        import tempfile
        lines = self.lines([(10.0 + i * 0.0167, 16.7, True) for i in range(30)] + [(10.6, 33.4, True)])
        with tempfile.NamedTemporaryFile('w', suffix='.ring.json', delete=False) as f:
            json.dump({'lines': lines}, f)
        try:
            r = {'windows': [{'name': 'a-open', 'start': 10.0, 'dur_s': 1.0, 'drop_ms': 20.0}, {'name': 'a-close', 'start': 3.0, 'dur_s': 1.0, 'drop_ms': 20.0}]}
            gate.attach_ring_motion(r, f.name)
            self.assertEqual((r['windows'][0]['ring_judged'], r['windows'][0]['ring_dropped']), (30, 1))
            self.assertNotIn('ring_dropped', r['windows'][1], 'before the ring starts')
            r2 = {'windows': [{'name': 'a-open', 'start': 10.0, 'dur_s': 1.0}]}
            gate.attach_ring_motion(r2, f.name + '.nope')
            self.assertNotIn('ring_dropped', r2['windows'][0])
        finally:
            os.unlink(f.name)


class RingSummary(unittest.TestCase):
    D = '%.3f D frame %d render %d us owed %d px in %d rects, ui %d draws %d layers (tick)'
    L = '%.3f L flip %.3f +%.1f ms: frame %.1f ms (gpu done), late %d lost 0 anim'

    def lines(self):
        out = ['10.000 info note: open']
        for i in range(10):
            owed = 1930240 if i % 5 == 4 else 43264
            out.append(self.D % (10.0 + i * 0.0083, i, 1000 + 100 * i, owed, 1, 200 if owed > 1000000 else 18, 157 if owed > 1000000 else 11))
            out.append(self.L % (10.0 + i * 0.0083, 10.0 + i * 0.0083, 16.7 if owed > 1000000 else 8.3, 14.8 if owed > 1000000 else 4.1, 1 if owed > 1000000 else 0))
        return out

    def test_the_numbers_of_a_window(self):
        import ringsum
        s = ringsum.summarize(ringsum.window(self.lines(), 'open', 8.0))
        self.assertEqual((s['drawn'], s['flips'], s['late'], s['over']), (10, 10, 2, 2))
        self.assertEqual(s['owed_px'][2], 1930240)
        self.assertEqual(s['ui_layers'][1], 157)
        self.assertAlmostEqual(s['frame_ms'][2], 14.8)
        self.assertIn('2 flips over 10 ms', ringsum.line('w', s))

    def test_no_note_is_an_empty_window_and_no_frame_is_none(self):
        import ringsum
        self.assertEqual(ringsum.window(self.lines(), 'nope', 8.0), [])
        self.assertIsNone(ringsum.summarize([]))
        self.assertIn('no frame', ringsum.line('w', None))


if __name__ == '__main__':
    unittest.main()
