import base64
import copy
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch
import zlib

TOOL = Path(__file__).parents[1] / 'tools' / 'wake.py'
spec = importlib.util.spec_from_file_location('wake_tool', TOOL)
wake = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wake)

# Captured stock webOS action-condition.json, compressed to keep this test self-contained.
NATIVE = (
    'eNrtXMtu2zgU3fsrBKPLTjxod90laTMN0NhBkraLIghoibbVyKSGopwJAv/7UG+JomRREm3K7iZNKT7PPbw8l4+8jQxjDExqY+SN'
    'Pxm/2H8N4y38yT4gsIYsdXyJN5AY5647fp98oq9u8CnJylJMB3hemptl/gZ8ZK7Ow8rTgkFRO6p1jjENf48/bdPK15CusBXkcXwE'
    'Pk0mJl6fvcA59s7MoPLv1xPswqzSsQteHQysYocWNvFo0GlWz7u3r5g+w9dtviMeNDGy0hyzhwfjn+vbQpYFcJw5MJ8LNbN024rK'
    'fAMeNa6R69MAnu04zbMtDSpoyy4CHabP0AMDgRh/GbOrq3zbd3BtI6uc/uXnxX059dyn2Lgg+MUT1RWNPp8SjzafVBxM6QtLMy6Y'
    'zWy0jBuIMzyOciOtJo8Rj3RQJMpzRMiAxHwsy7WV505YIwFrjyuZljVXACHoXLJ/lrBQ0Eg/XnOtXEbJOaJlVMsNfj/MjUillri5'
    '2TEVpYewiz7ESNXwW5K90aiOi7yxpdRyN25kENStdMZtuJvNg6kgucTcIk79ETfo+jHRNqsTuO6ZY28g3ajibmj2YTO3nqMJr6el'
    'xISHjdhWVg11RAtzqyIaI4VjmyCo8wYgsIRk4oTt7GDdDhtlbQc9wj5luT/+3d5ckfySsKPcqnoxnbXQiNC5wRY0PsMF8J0EhVLG'
    'CjXY0WGl2aV0otZUqtSGYg+10z81V4VbRayNGxQyV1fZl00FSR5KKD6taVil8npiYaW+U0xCvTRaa5Y1lmcac2yHJOuJaBVibN80'
    '6yqo2pMm3/JxBaIcpvFvDUNSCsgSUmFF34mz5SX+wndCmSMscMU+eiaBEHWW+8JJ0RdreRJOaxjKQKj5yvOpnCPDpBlNf2DHX0Pj'
    'HlKZJTQqxQr1TFM2ho1twjPgWzaerJkyZb7NgzRqbwdxN1Gm/DoXlRPEBVWmKpSrWntkgZXRyNojm7haaWiLBSsFpCy4aSR0HPBm'
    'gZ00wHxRcbwYQ9cc67CoIbdpEJbRWmeXA+hwA6F3HVRuZ3+iSPG2Resdiml9Pu48qxLBvg69krLHpdJSRGQODKIpdAeBx0bFqgFz'
    'hMkaOHcwGE+/ImuFM3RkNlVFpp4KuRBFhEUOsJ+PQV5R9bUavthMQpFxSoXLpL5xbgTrNUCc4ZKCzAQevLKdgpvO1PFkA5hHYwSZ'
    'uAQuIIHIhN5klS52JbheVixTsamo7V85Q3DmdgljGqE29Lh84VeIwNyBAQaJ7CzkeOSkelBV6I+D7sN/feDwYn4DHD8cO0YFKr3v'
    '1sOvPCh99y/9/XGUT2kS+XF+MbV+YPgp/vKf7VENiYOJprxZLDQnDtfBbszJx4KDcDiVoI9BxoZgVCMB1kWcEaaIhbLN1W/ZV0tv'
    '0wgC7OOBXV/Mc1sWRwJ75F7KWzEaYF8vtweOu4tfIJmheGRhvKEC+trQpqUV+JjmqCxxA35jMhRLCMLU01H7yrV0R8l2W+CVdp2s'
    'dUA7+8jor1ReNrs0ckIxShu27zaSHoTfWz+7cV4p4fmtojLHi9t5sjRnH10xw9+9xYaL9tK2HZdTEu3INVpBeUQzNJuvq4XOy+2i'
    'cvcwDrR0su1a+rTABC4J9pH1xDYcn2zr7HduOG1DqvymooSWqQUvvltwcMWB6SqLFFsC5Pmuiwm9QDjwrE1i/UrGUuJDGfx0WMUO'
    'A2HdvG+MYsWbogNxcQkRG6AgGMidhyhUCO7q1WMHjE4ASnKSKbf85qe/3BJbW7MVS4GqNbP+Kov8Iiq+xnkgUoiP1KQmF47Gc97S'
    'j8u8EWwYg1bdhx08xtyzBD1Qjo/Ep4OBuZsfi01xj31i9hfV3/y4/fx0d9VPBKGwi+zRrOZd3OGa1U8EHURT47nQcZOrrRl3BtNa'
    'zIdmvTz0lGjUy8PNip6XBv22CKr3B2KLKt0mEN89O0aca7di9gG18F3McDUlCYejTLZLX/CueF40dIAVava2EJ+wS45ssg83ceIO'
    'eQ9ACx+WHchdmMVZ3vaey8vcewhbUug1skd6zVE+YY8R2GT2rJ7IJ+4v9gAz93fA/pxZ9o5wsUh/XHa8D2KIK9+N0U14jD6JfrLc'
    'wtHIn4FUnit0OKzk/xpdT8Ts95BoX1iIHzVmFJp7A9uzj0eixZa9+DXuEMGN5KU6bKVDK/GD0D8XHvW+8MgTSKuu3jPzKu5h6wuP'
    'h3Yhau5Vq/MnTd6LB08pR9vR/7qh/d0='
)


def stock():
    return json.loads(zlib.decompress(base64.b64decode(NATIVE)))


def write_db(path, entries):
    data = {}
    for key, value in entries.items():
        namespace, leaf = key.rsplit('.', 1)
        data.setdefault(namespace, {})[leaf] = value
    path.write_text(json.dumps(data))


class PolicyTests(unittest.TestCase):
    def test_native_policy_preserves_priority_and_exceptions(self):
        original = stock()
        before = copy.deepcopy(original)
        policy = wake.make_policy(original)
        self.assertEqual(original, before)
        old = before['actions']
        new = policy['actions']
        self.assertEqual([a['name'] for a in new if a['name'] != 'Luma Cover App'],
                         [a['name'] for a in old])
        for action in old:
            actual = next(a for a in new if a['name'] == action['name'])
            expected = copy.deepcopy(action)
            if action['name'] in ('Input App', 'Cover App'):
                expected['conditions'].append('Luma Normal Resume - OFF')
            self.assertEqual(actual, expected, action['name'])
        cover = next(a for a in old if a['name'] == 'Cover App')
        clone = copy.deepcopy(cover)
        clone['name'] = 'Luma Cover App'
        clone['payload']['fallback']['id'] = '${Luma Resume App}'
        clone['payload']['finalFallback'] = {'id': wake.HOME}
        clone['conditions'].extend(['Luma Normal Resume - ON', 'Luma Resume App'])
        self.assertEqual(new[new.index(next(a for a in new if a['name'] == 'Cover App')) + 1], clone)
        self.assertEqual(policy['conditions'][:len(before['conditions'])], before['conditions'])

    def test_only_normal_power_reasons_are_selected(self):
        conditions = {c['name']: c for c in wake.make_policy(stock())['conditions']}
        on = conditions['Luma Normal Resume - ON']
        off = conditions['Luma Normal Resume - OFF']
        self.assertEqual(on['command']['target'], '${PowerOnReason}')
        self.assertEqual(off['command']['target'], '${PowerOnReason}')
        for reason in ('wakeOnLan', 'remoteKey', 'localKey', 'micomPower', 'lastPowerOn',
                       'onTimer', 'remind', 'hdmiCec', 'cpuAbnormal', '', 'unknown'):
            with self.subTest(reason=reason):
                selected = any(r['operation'] == 'equal' and r['value'] == reason
                               for r in on['where']['or'])
                excluded = all(r['operation'] == 'notequal' and r['value'] != reason
                               for r in off['where']['and'])
                self.assertEqual(selected, reason in ('wakeOnLan', 'remoteKey', 'localKey',
                                                       'micomPower', 'lastPowerOn'))
                self.assertEqual(excluded, not selected)

    def test_selector_validates_saved_application(self):
        with tempfile.TemporaryDirectory() as d:
            saved = Path(d) / 'saved.json'
            self.assertEqual(wake.select(saved), wake.HOME)
            for value, expected in [('com.webos.app.livetv', 'com.webos.app.livetv'),
                                    ('youtube.leanback.v4', 'youtube.leanback.v4'),
                                    ('', wake.HOME), ('bad;cmd', wake.HOME),
                                    ('ápp', wake.HOME), (None, wake.HOME),
                                    ('a' * 129, wake.HOME)]:
                with self.subTest(value=value):
                    saved.write_text(json.dumps({'lastAppId': value}))
                    self.assertEqual(wake.select(saved), expected)
            for bad in ('{', '[]', '{}', 'x' * 4097):
                saved.write_text(bad)
                self.assertEqual(wake.select(saved), wake.HOME)

    def test_native_reader_keeps_resume_identifier(self):
        with tempfile.TemporaryDirectory() as d:
            saved = Path(d) / 'saved.json'
            saved.write_text(json.dumps({'lastAppId': 'com.webos.app.hdmi2'}))
            program = ("import importlib.util,sys; from pathlib import Path; "
                       "s=importlib.util.spec_from_file_location('wake',sys.argv[1]); "
                       "w=importlib.util.module_from_spec(s); s.loader.exec_module(w); "
                       "w.SAVED=Path(sys.argv[2]); "
                       "sys.argv=[sys.argv[1],'select']; w.main()")
            result = subprocess.run([sys.executable, '-B', '-c', program, str(TOOL), str(saved)],
                                    capture_output=True, check=True)
            # Native AbsCondition::runCommand always removes the final byte.
            self.assertEqual(result.stdout[:-1].decode(), 'com.webos.app.hdmi2')

    def test_native_reader_keeps_home_when_selector_fails(self):
        with tempfile.TemporaryDirectory() as d:
            conditions = wake.make_policy(stock(), Path(d) / 'missing.py')['conditions']
            command = next(c['command']['target'] for c in conditions if c['name'] == 'Luma Resume App')
            result = subprocess.run(command, shell=True, capture_output=True, check=True)
            self.assertEqual(result.stdout[:-1].decode(), wake.HOME)


class NativeIntentTests(unittest.TestCase):
    transition = ('DEBUG(1.000000000) : BootSequencer) onPowerStatusChange: '
                  'state(POWER OFF), subState(active standby), subProcessing(prepare power on), '
                  'nextSubState(active), reason(remoteKey)\n')

    def event(self, name, value):
        return "DEBUG(2.000000000) : Event) Trigger '%s' event (%s)\n" % (name, json.dumps(value))

    def test_hotkey_takes_precedence_over_ott_and_resume(self):
        log = (self.transition +
               self.event('OTT GIP', {'id': 'youtube.leanback.v4', 'params': {'a': 1}}) +
               self.event('Hotkey', {'id': 'netflix', 'params': {'reason': 'remoteKey'}}) +
               self.event('Hotkey - ON', {}) +
               self.event('Luma Normal Resume - ON', 'remoteKey') +
               self.event('Luma Resume App', 'com.webos.app.hdmi2'))
        self.assertEqual(wake.resolve_intent(log, 'com.webos.app.hdmi2'),
                         {'appId': 'netflix', 'params': {'reason': 'remoteKey'}, 'source': 'hotkey'})

    def test_latest_power_transition_discards_old_intent(self):
        old = self.transition + self.event('Hotkey', {'id': 'netflix'}) + self.event('Hotkey - ON', {})
        current = (self.transition + self.event('Hotkey - OFF', {}) +
                   self.event('OTT GIP', {'id': 'youtube.leanback.v4', 'params': {}}) +
                   self.event('OTT GIP - ON', {}))
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / 'bootd.log'
            log.write_text(old + current)
            with (patch.object(wake, 'LOG', log),
                  patch.object(wake.time, 'monotonic_ns', return_value=2_000_000_000)):
                self.assertEqual(wake.resolve_intent(wake.native_cycle(), 'com.webos.app.hdmi2'),
                                 {'appId': 'youtube.leanback.v4', 'params': {}, 'source': 'ott'})
    def test_normal_resume_beats_stock_input_but_cec_keeps_input(self):
        base = (self.transition + self.event('Hotkey - OFF', {}) + self.event('OTT GIP - OFF', {}) +
                self.event('Last Input App', 'com.webos.app.hdmi2') +
                'DEBUG(2.000000000) : Client) Request : {"id":"com.webos.app.hdmi2","params":{"boot":true}}\n' +
                'INFO(2.000000000) : Action) Execute AbsAction (Input App) response ({"returnValue":true})\n')
        exclusions = ''.join(self.event(name, {}) for name in
                             ('OnTimer - OFF', 'Remind - OFF', 'EWBS - OFF',
                              'Auto Browser - OFF', 'Last App Booting - OFF'))
        normal = base + exclusions + self.event('Luma Normal Resume - ON', 'remoteKey') + self.event('Luma Resume App', wake.HOME)
        self.assertEqual(wake.resolve_intent(normal, wake.HOME),
                         {'appId': wake.HOME, 'params': {}, 'source': 'resume'})
        cec = base + self.event('Luma Normal Resume - OFF', 'hdmiCec')
        self.assertEqual(wake.resolve_intent(cec, wake.HOME),
                         {'appId': 'com.webos.app.hdmi2', 'params': {'boot': True}, 'source': 'native-input'})


    def test_native_action_precedes_resume_and_preserves_parameters(self):
        log = (self.transition + self.event('Hotkey - OFF', {}) + self.event('OTT GIP - OFF', {}) +
               'DEBUG(2.000000000) : Client) Request : {"id":"com.webos.app.hdmi1","params":{"input":"HDMI_1"}}\n' +
               'INFO(2.000000000) : Action) Execute AbsAction (Input App) response ({"returnValue":true})\n' +
               self.event('Luma Normal Resume - OFF', 'hdmiCec'))
        self.assertEqual(wake.resolve_intent(log, 'com.webos.app.hdmi2'),
                         {'appId': 'com.webos.app.hdmi1', 'params': {'input': 'HDMI_1'},
                          'source': 'native-input'})

    def test_truncated_input_is_preserve_only_with_matching_successful_native_action(self):
        prefix = (self.transition + self.event('Hotkey - OFF', {}) + self.event('OTT GIP - OFF', {}) +
                  self.event('Last Input App', 'com.webos.app.hdmi2') + self.event('Luma Normal Resume - OFF', 'hdmiCec'))
        request = 'DEBUG(2.000000000) : Client) Request : {"id":"com.webos.app.hdmi2","params":{"boot":true,\n'
        success = 'INFO(2.000000000) : Action) Execute AbsAction (Input App) response ({"returnValue":true})\n'
        self.assertEqual(wake.resolve_intent(prefix + request + success, wake.HOME),
                         {'appId': 'com.webos.app.hdmi2', 'params': None, 'source': 'native-input-id-only'})
        for text in (prefix + request, prefix.replace('hdmi2', 'hdmi1') + request + success):
            with self.assertRaises(RuntimeError):
                wake.resolve_intent(text, wake.HOME)

    def test_inputcommon_delivery_normalizes_complete_and_truncated_requests(self):
        event = self.event('Last Input App', 'com.webos.app.hdmi2')
        call = 'DEBUG(2.000000000) : Client) Call luna://com.webos.applicationManager/launch\n'
        request = ('DEBUG(2.000000000) : Client) Request : '
                   '{"id":"com.webos.app.inputcommon","noSplash":true,"params":{"changeRunningAppId":"com.webos.app.hdmi2"')
        end = ('INFO(2.000000000) : Action) Execute AbsAction (Input App) response ({"returnValue":true})\n'
               'DEBUG(2.000000000) : Action) Set first action (Input App) appId (${Last Input App})\n')
        for suffix in ('}}\n', ',"boot":true,...\n'):
            with self.subTest(suffix=suffix):
                result = wake.native_delivered(self.transition + event + call + request + suffix + end)
                self.assertEqual(result, {'appId': 'com.webos.app.hdmi2', 'params': None,
                                          'source': 'native-input-id-only'})

    def test_inputcommon_delivery_rejects_unbound_or_ambiguous_records(self):
        event = self.event('Last Input App', 'com.webos.app.hdmi2')
        call = 'DEBUG(2.000000000) : Client) Call luna://com.webos.applicationManager/launch\n'
        request = ('DEBUG(2.000000000) : Client) Request : '
                   '{"id":"com.webos.app.inputcommon","params":{"changeRunningAppId":"com.webos.app.hdmi2"}}\n')
        success = 'INFO(2.000000000) : Action) Execute AbsAction (Input App) response ({"returnValue":true})\n'
        first = 'DEBUG(2.000000000) : Action) Set first action (Input App) appId (${Last Input App})\n'
        cases = {
            'missing input': call + request + success + first,
            'input after request': call + request + event + success + first,
            'different input': event.replace('hdmi2', 'hdmi1') + call + request + success + first,
            'missing call': event + request + success + first,
            'wrong call': event + call.replace('/launch', '/close') + request + success + first,
            'missing success': event + call + request + first,
            'failed action': event + call + request + success.replace('true', 'false') + first,
            'missing first action': event + call + request + success,
            'first action before success': event + call + request + first + success,
            'other action': event + call + request + success.replace('Input App', 'Input App EWBS') + first,
            'missing destination': event + call + request.replace('"changeRunningAppId":"com.webos.app.hdmi2"', '"boot":true') + success + first,
            'different destination': event + call + request.replace('hdmi2', 'hdmi1') + success + first,
            'duplicate destination': event + call + request.replace('"changeRunningAppId":', '"changeRunningAppId":"com.webos.app.hdmi1","changeRunningAppId":') + success + first,
            'duplicate wrapper': event + call + request.replace('"id":', '"id":"other.app","id":') + success + first,
            'escaped destination': event + call + request.replace('hdmi2', 'hdmi\\u0032') + success + first,
            'nested destination': event + call + request.replace('"params":{"changeRunningAppId":"com.webos.app.hdmi2"}', '"params":{"nested":{"changeRunningAppId":"com.webos.app.hdmi2"}}') + success + first,
            'escaped wrapper id': event + call + request.replace('inputcommon', 'inputcomm\\u006fn') + success + first,
            'incomplete destination field': event + call + request.replace('"params":', '"noSplash":true,"params":').replace('hdmi2"}}', 'hdmi2"junk') + success + first,
        }
        for name, text in cases.items():
            with self.subTest(name=name):
                self.assertIsNone(wake.native_delivered(self.transition + text))

    def test_newest_delivery_supersedes_earlier_delivery_or_invalidates_it(self):
        call = 'DEBUG(2.000000000) : Client) Call luna://com.webos.applicationManager/launch\n'
        success = 'INFO(2.000000000) : Action) Execute AbsAction (Input App) response ({"returnValue":true})\n'
        first = 'DEBUG(2.000000000) : Action) Set first action (Input App) appId (${Last Input App})\n'
        def delivery(app):
            return (self.event('Last Input App', app) + call +
                    'DEBUG(2.000000000) : Client) Request : ' +
                    json.dumps({'id': 'com.webos.app.inputcommon', 'params': {'changeRunningAppId': app}}) +
                    '\n' + success + first)
        prior = delivery('com.webos.app.hdmi1')
        latest = delivery('com.webos.app.hdmi2')
        self.assertEqual(wake.native_delivered(self.transition + prior + latest),
                         {'appId': 'com.webos.app.hdmi2', 'params': None, 'source': 'native-input-id-only'})
        broken = latest.replace('"changeRunningAppId": "com.webos.app.hdmi2"', '"boot": true')
        self.assertIsNone(wake.native_delivered(self.transition + prior + broken))
        for gap in ('', 'DEBUG(3.000000000) : unrelated\n' * 13):
            with self.subTest(gap=gap):
                self.assertIsNone(wake.native_delivered(self.transition + prior + gap + success + first))

    def test_resume_requires_all_exclusions_and_matching_saved_app(self):
        prefix = (self.transition + self.event('Hotkey - OFF', {}) + self.event('OTT GIP - OFF', {}) +
                  self.event('Luma Normal Resume - ON', 'remoteKey') +
                  self.event('Luma Resume App', 'com.webos.app.hdmi2'))
        names = ('OnTimer - OFF', 'Remind - OFF', 'EWBS - OFF', 'Auto Browser - OFF', 'Last App Booting - OFF')
        complete = prefix + ''.join(self.event(name, {}) for name in names)
        self.assertEqual(wake.resolve_intent(complete, 'com.webos.app.hdmi2'),
                         {'appId': 'com.webos.app.hdmi2', 'params': {}, 'source': 'resume'})
        with self.assertRaises(RuntimeError):
            wake.resolve_intent(complete, 'netflix')
        for missing in names:
            with self.subTest(missing=missing), self.assertRaises(RuntimeError):
                wake.resolve_intent(prefix + ''.join(self.event(n, {}) for n in names if n != missing),
                                    'com.webos.app.hdmi2')

    def test_native_cycle_accepts_captured_padded_early_boot_timestamps(self):
        lines = ('PERFORMANCE( 48.233362580) : Boot) BOOTD_START',
                 'DEBUG( 48.233362580) : BootSequencer) onPowerStatusChange: '
                 'state(POWER OFF), subProcessing(prepare power on)')
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / 'bootd.log'
            with (patch.object(wake, 'LOG', log),
                  patch.object(wake.time, 'monotonic_ns', return_value=50_000_000_000)):
                for line in lines:
                    with self.subTest(line=line):
                        log.write_text(line + '\n')
                        self.assertEqual(wake.native_cycle(), line)

    def test_native_cycle_rejects_missing_stale_and_future_markers(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / 'bootd.log'
            with (patch.object(wake, 'LOG', log),
                  patch.object(wake.time, 'monotonic_ns', return_value=122_000_000_000)):
                for text in ('unrelated\n', self.transition,
                             self.transition.replace('1.000000000', '123.000000000')):
                    log.write_text(text)
                    with self.subTest(text=text), self.assertRaises(RuntimeError):
                        wake.native_cycle()

    def test_only_initial_respawn_record_identifies_inherited_foreground(self):
        start = 'PERFORMANCE( 48.233362580) : Boot) BOOTD_START\n'
        respawn = 'DEBUG( 48.240471020) : Respawn) {"respawned":true,"foregroundApp":"com.webos.app.livetv"}\n'
        self.assertEqual(wake.inherited_foreground(start + respawn),
                         {'appId': 'com.webos.app.livetv', 'params': None, 'source': 'native-respawn'})
        self.assertIsNone(wake.inherited_foreground(self.transition + respawn))
        self.assertIsNone(wake.inherited_foreground(start + respawn.replace('true', 'false')))

    def test_missing_or_malformed_intent_does_not_guess_resume(self):
        for log in (self.transition,
                    self.transition + self.event('Hotkey - ON', {}) + self.event('Hotkey', {'id': 'bad;cmd'}),
                    self.transition + "DEBUG(2.000000000) : Event) Trigger 'Hotkey' event ({bad})\n"):
            with self.subTest(log=log), self.assertRaises((ValueError, RuntimeError)):
                wake.resolve_intent(log, 'com.webos.app.hdmi2')


class LandingShellTests(unittest.TestCase):
    def test_restart_input_relaunch_omits_params_and_respects_newer_foreground(self):
        source = (TOOL.parent / 'lumatv').read_text()
        function = 'land_on_app() {' + source.split('land_on_app() {', 1)[1].split('\n}\n', 1)[0] + '\n}\n'
        hdmi = {'appId': 'com.webos.app.hdmi2', 'params': None, 'source': 'native-input-id-only'}
        plex = {'appId': 'com.beb.plxnative', 'params': None, 'source': 'newer-foreground'}
        for foreground, selected, expected in ((wake.HOME, hdmi, {'id': hdmi['appId']}),
                                               (hdmi['appId'], hdmi, None),
                                               (plex['appId'], plex, None)):
            with self.subTest(foreground=foreground), tempfile.TemporaryDirectory() as d:
                root = Path(d)
                (root / 'foreground').write_text(foreground)
                fake_luna = root / 'luna-send'
                fake_luna.write_text('#!/bin/sh\nprintf "%s" "$4" > "$D/launched"\nprintf "%s" com.webos.app.hdmi2 > "$D/foreground"\n')
                fake_luna.chmod(0o755)
                script = '''
d=$1
selected=$2
captured=$3
HOME_LAUNCH_URI=luna://com.webos.applicationManager/launch
reports_before=
boot_landing_app() { printf '%s\n' "$@" > "$d/context"; printf '%s\n' "$selected"; }
foreground_app_id() { cat "$d/foreground"; }
D=$d; PATH="$d:$PATH"; export D PATH
sleep() { :; }
disarm_restart_crashes() { :; }
log_line() { :; }
''' + function + '\nland_on_app "$captured" restart\n'
                subprocess.run(['sh', '-c', script, 'wake-test', str(root), json.dumps(selected), json.dumps(hdmi)],
                               check=True, capture_output=True, text=True)
                launched = root / 'launched'
                self.assertEqual(json.loads(launched.read_text()) if launched.exists() else None, expected)
                self.assertEqual((root / 'context').read_text().splitlines(), ['apply', 'restart', hdmi['appId']])


class ActivationTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        state = root / 'wake'
        native = root / 'stock.json'
        native.write_text(json.dumps(stock()))
        (root / 'bootd.log').write_text('')
        paths = dict(STOCK=native, STATE=state, POLICY=state / 'policy.json',
                     SELECTOR=state / 'selector.py', SNAPSHOT=state / 'original.json',
                     TRIAL=state / 'trial.json', PENDING=state / 'pending.json',
                     REPORT=state / 'report.json', FAILURE=state / 'failure.json',
                     MAIN_DB=root / 'main.json', FACTORY_DB=root / 'factory.json', LOG=root / 'bootd.log')
        p = patch.multiple(wake, **paths)
        p.start()
        self.addCleanup(p.stop)
        self.values = {wake.REASONS_KEY: ['aiKey', 'remoteKey', 'localKey', 'micomPower']}
        write_db(wake.MAIN_DB, self.values)
        write_db(wake.FACTORY_DB, {})
        self.events = []
        self.current_boot = 'boot-one'
        self.app = 'com.webos.app.hdmi2'
        self.witness = {'condition': True, 'action': True}
        self.real_log_cursor = wake.log_cursor
        self.real_log_witness = wake.log_witness
        self.real_set_configs = wake.set_configs
        self.real_configs = wake.configs

        def put(values, volatile=False):
            self.events.append(('config', copy.deepcopy(values), volatile, wake.PENDING.exists()))
            self.values.update(copy.deepcopy(values))

        methods = dict(configs=lambda: copy.deepcopy(self.values), set_configs=put,
                       guard_install=lambda: self.events.append(('guard-install',)),
                       guard_remove=lambda: self.events.append(('guard-remove',)),
                       service=lambda action: self.events.append(('service', action)),
                       foreground=lambda allow_empty=False: self.app, boot_id=lambda: self.current_boot,
                       log_cursor=lambda: {'inode': 7, 'size': 10},
                       log_witness=lambda since=None: self.witness,
                       boot_status=lambda: {'firstAppId': self.app, 'powerStatus': 'active',
                                            'signals': {'boot-done': True}})
        p = patch.multiple(wake, **methods)
        p.start()
        self.addCleanup(p.stop)
    def prepare_boot(self, token='a' * 32):
        write_db(wake.FACTORY_DB, wake.active(wake.snapshot()))
        intent = {'appId': self.app, 'params': {}, 'source': 'resume'}
        with (patch.object(wake, 'saved_id', return_value=self.app),
              patch.object(wake, 'native_cycle', return_value='native cycle'),
              patch.object(wake, 'resolve_intent', return_value=intent)):
            return wake.boot_check('prepare', token)


    def test_stage_trial_persist_boot_witness_commit_and_off(self):
        self.assertEqual(wake.stage()['phase'], 'staged')
        self.assertEqual(self.events, [])
        self.assertNotIn(wake.PATH_KEY, self.values)
        original = json.loads(wake.SNAPSHOT.read_text())
        self.assertEqual(original['prior_reasons'], ['aiKey', 'remoteKey', 'localKey', 'micomPower'])
        self.assertEqual(original['active_reasons'], ['aiKey'])
        self.assertTrue(wake.trial()['passed'])
        active = {wake.PATH_KEY: str(wake.POLICY), wake.REASONS_KEY: ['aiKey']}
        self.assertEqual(self.events, [('guard-install',), ('config', active, True, False),
                                       ('service', 'restart'), ('guard-remove',)])
        self.events.clear()
        self.assertEqual(wake.persist()['phase'], 'pending')
        self.assertEqual(self.events, [('config', active, False, True), ('config', active, True, True)])
        pending = json.loads(wake.PENDING.read_text())
        self.assertTrue(pending['activation_token'])
        with self.assertRaises((OSError, RuntimeError)):
            wake.commit()
        self.current_boot = 'boot-two'
        report = self.prepare_boot()
        self.assertTrue(report['ok'])
        self.assertEqual(report['phase'], 'prepared')
        self.assertEqual(report['activation_token'], pending['activation_token'])
        self.assertEqual(report['invocation_token'], 'a' * 32)
        self.assertEqual(report['native_policy_log'], self.witness)
        self.assertEqual(wake.boot_check('landing', 'a' * 32)['appId'], self.app)
        report = wake.boot_check('finalize', 'a' * 32)
        self.assertTrue(report['ok'])
        self.assertEqual(report['firstAppId'], self.app)
        self.assertEqual(wake.commit()['phase'], 'committed')
        self.assertFalse(wake.PENDING.exists())
        self.events.clear()
        self.assertTrue(wake.off()['ok'])
        restored = {wake.PATH_KEY: str(wake.STOCK), wake.REASONS_KEY: original['prior_reasons']}
        self.assertEqual(self.events, [('guard-install',), ('config', restored, False, False),
                                       ('config', restored, True, False), ('service', 'restart'), ('guard-remove',)])

    def test_rejects_foreign_owner_firmware_and_tampered_files(self):
        self.values[wake.PATH_KEY] = '/foreign/policy'
        with self.assertRaisesRegex(RuntimeError, 'another owner'):
            wake.stage()
        self.assertEqual(self.events, [])
        del self.values[wake.PATH_KEY]
        with patch.object(wake, 'FINGERPRINT', 'unknown'):
            with self.assertRaisesRegex(RuntimeError, 'unsupported'):
                wake.stage()
        wake.stage()
        wake.POLICY.write_text('{}')
        with self.assertRaisesRegex(RuntimeError, 'changed|owned'):
            wake.trial()
        self.assertEqual(self.events, [])

    def test_trial_failure_restores_both_config_layers_before_restart(self):
        wake.stage()
        self.witness = {'condition': True, 'action': False}
        with (patch.object(wake.time, 'monotonic', side_effect=[0, 16, 16]),
              patch.object(wake.time, 'sleep')):
            result = wake.trial()
        self.assertFalse(result['ok'])
        self.assertEqual(result['phase'], 'trial')
        self.assertTrue(wake.FAILURE.exists())
        restored = {wake.PATH_KEY: str(wake.STOCK),
                    wake.REASONS_KEY: ['aiKey', 'remoteKey', 'localKey', 'micomPower']}
        self.assertEqual(self.events[-5:], [('guard-install',), ('config', restored, False, False),
                                           ('config', restored, True, False), ('service', 'restart'),
                                           ('guard-remove',)])
        with self.assertRaisesRegex(RuntimeError, 'attempted'):
            wake.trial()

    def test_factory_overrides_are_removed_for_volatile_trial(self):
        reasons = ['aiKey', 'lastPowerOn', 'localKey', 'remoteKey', 'seniorHelp']
        original = {wake.PATH_KEY: str(wake.STOCK), wake.REASONS_KEY: reasons}
        main = {wake.REASONS_KEY: reasons}
        factory = copy.deepcopy(original)
        volatile = {}
        writes = []
        write_db(wake.MAIN_DB, main)
        write_db(wake.FACTORY_DB, factory)

        def luna(method, payload):
            if method == 'getConfigs':
                return {'returnValue': True, 'configs': dict(main, **volatile, **factory)}
            values = payload['configs']
            is_volatile = payload.get('volatile', False)
            writes.append((copy.deepcopy(values), is_volatile))
            layer = volatile if is_volatile else factory
            for key, value in values.items():
                if value is None:
                    self.assertEqual(json.loads(wake.TRIAL.read_text())['prior_factory'], original)
                    layer.pop(key, None)
                else:
                    layer[key] = copy.deepcopy(value)
            write_db(wake.FACTORY_DB, factory)
            return {'returnValue': True}

        with patch.multiple(wake, luna=luna, configs=self.real_configs,
                            set_configs=self.real_set_configs):
            wake.stage()
            result = wake.trial()
        active = {wake.PATH_KEY: str(wake.POLICY), wake.REASONS_KEY: ['aiKey', 'seniorHelp']}
        self.assertTrue(result['passed'])
        self.assertEqual(writes, [({key: None for key in original}, False), (active, True)])
        self.assertEqual(factory, {})
        self.assertEqual(volatile, active)
        self.assertEqual(wake.database_entries(wake.MAIN_DB), main)
        self.assertEqual(json.loads(wake.TRIAL.read_text())['prior_factory'], original)

    def test_trial_failure_restores_factory_and_volatile_layers(self):
        reasons = ['aiKey', 'lastPowerOn', 'localKey', 'remoteKey', 'seniorHelp']
        original = {wake.PATH_KEY: str(wake.STOCK), wake.REASONS_KEY: reasons}
        main = {wake.REASONS_KEY: reasons}
        factory = copy.deepcopy(original)
        volatile = {}
        writes = []
        write_db(wake.MAIN_DB, main)
        write_db(wake.FACTORY_DB, factory)

        def luna(method, payload):
            if method == 'getConfigs':
                return {'returnValue': True, 'configs': {**main, **volatile, **factory}}
            is_volatile = payload.get('volatile', False)
            values = copy.deepcopy(payload['configs'])
            writes.append((values, is_volatile))
            layer = volatile if is_volatile else factory
            for key, value in values.items():
                if value is None:
                    layer.pop(key, None)
                else:
                    layer[key] = value
            write_db(wake.FACTORY_DB, factory)
            return {'returnValue': True}

        self.witness = {'condition': True, 'action': False}
        with (patch.multiple(wake, luna=luna, configs=self.real_configs,
                             set_configs=self.real_set_configs),
              patch.object(wake.time, 'monotonic', side_effect=[0, 16, 16]),
              patch.object(wake.time, 'sleep')):
            wake.stage()
            result = wake.trial()
        self.assertFalse(result['ok'])
        self.assertEqual(result['rollback_errors'], [])
        self.assertEqual(writes[-2:], [(original, False), (original, True)])
        self.assertEqual(factory, original)
        self.assertEqual(volatile, original)
        self.assertEqual(wake.database_entries(wake.MAIN_DB), main)

    def test_trial_refuses_unsafe_factory_or_main_without_writes(self):
        wake.stage()
        original_main = wake.database_entries(wake.MAIN_DB)
        for main, factory in [(original_main, {wake.PATH_KEY: '/foreign'}),
                              ({wake.REASONS_KEY: ['unexpected']}, {})]:
            with self.subTest(main=main, factory=factory):
                write_db(wake.MAIN_DB, main)
                write_db(wake.FACTORY_DB, factory)
                with patch.object(wake, 'luna') as luna:
                    with self.assertRaises(RuntimeError):
                        wake.trial()
                luna.assert_not_called()
                self.assertEqual(wake.database_entries(wake.MAIN_DB), main)
                self.assertEqual(wake.database_entries(wake.FACTORY_DB), factory)
                self.assertFalse(wake.TRIAL.exists())
                self.assertEqual(self.events, [])

    def test_foreign_factory_before_removal_causes_no_config_write(self):
        wake.stage()
        foreign = {wake.PATH_KEY: '/foreign'}
        def foreground():
            write_db(wake.FACTORY_DB, foreign)
            return self.app
        with (patch.object(wake, 'foreground', side_effect=foreground),
              patch.object(wake, 'luna') as luna):
            result = wake.trial()
        self.assertFalse(result['ok'])
        self.assertFalse(result['rollback_started'])
        luna.assert_not_called()
        self.assertEqual(wake.database_entries(wake.FACTORY_DB), foreign)
        self.assertNotIn(('service', 'restart'), self.events)

    def test_foreign_factory_after_removal_retains_guard_without_overwrite(self):
        self.values[wake.PATH_KEY] = str(wake.STOCK)
        original = copy.deepcopy(self.values)
        write_db(wake.MAIN_DB, original)
        write_db(wake.FACTORY_DB, original)
        wake.stage()
        foreign = {wake.PATH_KEY: '/foreign'}
        calls = []
        def luna(method, payload):
            calls.append((method, copy.deepcopy(payload)))
            write_db(wake.FACTORY_DB, foreign)
            return {'returnValue': True}
        with patch.object(wake, 'luna', side_effect=luna):
            result = wake.trial()
        self.assertFalse(result['ok'])
        self.assertTrue(result['rollback_errors'])
        self.assertEqual(len(calls), 1)
        self.assertEqual(wake.database_entries(wake.FACTORY_DB), foreign)
        self.assertNotIn(('guard-remove',), self.events)
        self.assertNotIn(('service', 'restart'), self.events)

    def test_trial_records_mismatched_readback_before_restoring_config(self):
        reasons = ['aiKey', 'lastPowerOn', 'localKey', 'remoteKey', 'seniorHelp']
        self.values = {wake.PATH_KEY: str(wake.STOCK), wake.REASONS_KEY: reasons}
        write_db(wake.MAIN_DB, self.values)
        wake.stage()
        original = copy.deepcopy(self.values)
        requested = {wake.PATH_KEY: str(wake.POLICY), wake.REASONS_KEY: ['aiKey', 'seniorHelp']}
        observed = {wake.PATH_KEY: str(wake.POLICY), wake.REASONS_KEY: ['seniorHelp', 'aiKey']}
        before_rollback = []
        writes = []

        def luna(method, payload):
            if method == 'getConfigs':
                return {'returnValue': True, 'configs': copy.deepcopy(self.values)}
            self.assertEqual(method, 'setConfigs')
            values = copy.deepcopy(payload['configs'])
            writes.append((values, payload.get('volatile', False)))
            if values == requested:
                self.values = copy.deepcopy(observed)
            else:
                if not before_rollback:
                    before_rollback.append(json.loads(wake.FAILURE.read_text()))
                self.values = values
            return {'returnValue': True}

        with patch.multiple(wake, luna=luna, configs=self.real_configs,
                            set_configs=self.real_set_configs):
            result = wake.trial()

        self.assertFalse(result['ok'])
        self.assertEqual(json.loads(result['error'].partition(': ')[2]),
                         {'requested': requested, 'observed': observed, 'volatile': True})
        self.assertEqual(before_rollback[0]['error'], result['error'])
        self.assertEqual(json.loads(wake.FAILURE.read_text()), result)
        self.assertEqual(result['rollback_errors'], [])
        self.assertEqual(writes, [(requested, True), (original, False), (original, True)])
        self.assertEqual(self.values, original)
        self.assertNotIn(('service', 'restart'), self.events)

    def test_trial_saves_real_log_witness_before_rollback_overwrites_log(self):
        wake.stage()
        log = wake.STATE / 'bootd.log'
        log.write_text('old log\n')
        at_rollback = []
        restarts = []

        def restart(action):
            restarts.append(action)
            if len(restarts) == 1:
                with log.open('a') as stream:
                    stream.write('PERFORMANCE( 10.000000000) : Boot) BOOTD_START\nCondition) Read configuration (' +
                                 str(wake.POLICY) + ')\n')
            else:
                at_rollback.append(json.loads(wake.FAILURE.read_text()))
                log.write_text('rollback replaced the trial log\n')

        with (patch.object(wake, 'LOG', log),
              patch.object(wake, 'log_cursor', self.real_log_cursor),
              patch.object(wake, 'log_witness', self.real_log_witness),
              patch.object(wake, 'service', side_effect=restart),
              patch.object(wake.time, 'monotonic_ns', return_value=10_000_000_000),
              patch.object(wake.time, 'monotonic', side_effect=[0, 16, 16]),
              patch.object(wake.time, 'sleep')):
            result = wake.trial()

        self.assertEqual(restarts, ['restart', 'restart'])
        self.assertEqual(log.read_text(), 'rollback replaced the trial log\n')
        self.assertEqual(at_rollback[0]['observations'], result['observations'])
        self.assertEqual(json.loads(wake.FAILURE.read_text()), result)
        seen = result['observations']
        self.assertEqual(seen['native_policy_log']['value'], {'condition': True, 'action': False})
        self.assertEqual(seen['readiness']['value'], {'firstAppId': self.app, 'powerStatus': 'active',
                                                       'signals': {'boot-done': True}})
        self.assertEqual(seen['foreground_before']['value'], self.app)
        self.assertEqual(seen['foreground_after']['value'], self.app)
        self.assertEqual(seen['cursor_before']['value']['size'], len('old log\n'))
        self.assertGreater(seen['cursor_after']['value']['size'], seen['cursor_before']['value']['size'])

    def test_trial_records_each_check_error_independently(self):
        wake.stage()
        with (patch.object(wake, 'boot_status', side_effect=[RuntimeError('boot unavailable'), {}]),
              patch.object(wake, 'log_witness', side_effect=RuntimeError('log rotated')),
              patch.object(wake, 'foreground', side_effect=[self.app, OSError('foreground unavailable')]),
              patch.object(wake, 'log_cursor', side_effect=[{'inode': 7, 'size': 10}, OSError('log missing')]),
              patch.object(wake.time, 'monotonic', side_effect=[0, 16, 16]),
              patch.object(wake.time, 'sleep')):
            result = wake.trial()
        seen = result['observations']
        self.assertEqual(seen['foreground_before']['value'], self.app)
        self.assertEqual(seen['cursor_before']['value'], {'inode': 7, 'size': 10})
        for name, error_type, message in (
                ('readiness', 'RuntimeError', 'boot unavailable'),
                ('native_policy_log', 'RuntimeError', 'log rotated'),
                ('foreground_after', 'OSError', 'foreground unavailable'),
                ('cursor_after', 'OSError', 'log missing')):
            with self.subTest(check=name):
                self.assertIsNone(seen[name]['value'])
                self.assertEqual(seen[name]['error'], {'type': error_type, 'message': message})

    def test_trial_keeps_latest_success_and_error_without_accepting_old_success(self):
        wake.stage()
        with (patch.object(wake, 'boot_status', side_effect=[{'generation': 1},
                                                             RuntimeError('later failure'), {}]),
              patch.object(wake, 'log_witness', side_effect=[{'condition': True, 'action': False},
                                                              {'condition': True, 'action': True}]),
              patch.object(wake.time, 'monotonic', side_effect=[0, 1, 16, 16]),
              patch.object(wake.time, 'sleep')):
            result = wake.trial()
        self.assertFalse(result['ok'])
        seen = result['observations']
        self.assertEqual(seen['readiness']['value'], {'generation': 1})
        self.assertEqual(seen['readiness']['error'], {'type': 'RuntimeError', 'message': 'later failure'})
        self.assertEqual(seen['native_policy_log']['value'], {'condition': True, 'action': True})

    def test_trial_records_precondition_failure_without_activation(self):
        wake.stage()
        with patch.object(wake, 'log_cursor', side_effect=OSError('cannot stat log')):
            result = wake.trial()
        self.assertFalse(result['ok'])
        self.assertEqual(result['observations']['foreground_before']['value'], self.app)
        self.assertEqual(result['observations']['cursor_before']['error'],
                         {'type': 'OSError', 'message': 'cannot stat log'})
        self.assertNotIn(('service', 'restart'), self.events)

    def test_persist_requires_passed_trial_from_same_boot(self):
        wake.stage()
        wake.trial()
        self.current_boot = 'other-boot'
        self.events.clear()
        with self.assertRaisesRegex(RuntimeError, 'trial required'):
            wake.persist()
        self.assertFalse(wake.PENDING.exists())
        self.assertEqual(self.events, [])

    def test_persist_failure_marks_pending_and_restores_original(self):
        wake.stage()
        wake.trial()
        self.events.clear()
        put = wake.set_configs
        failed = False
        def fail_once(values, volatile=False):
            nonlocal failed
            if not failed:
                failed = True
                self.assertTrue(wake.PENDING.exists())
                raise RuntimeError('configd unavailable')
            put(values, volatile=volatile)
        with patch.object(wake, 'set_configs', side_effect=fail_once):
            result = wake.persist()
        self.assertFalse(result['ok'])
        self.assertEqual(result['phase'], 'persist')
        self.assertEqual(json.loads(wake.PENDING.read_text())['state'], 'failed')
        self.assertEqual(self.values[wake.PATH_KEY], str(wake.STOCK))
        self.assertEqual([e[0] for e in self.events],
                         ['guard-install', 'config', 'config', 'service', 'guard-remove'])

    def test_cold_witness_failure_rolls_back_once(self):
        wake.stage()
        wake.trial()
        wake.persist()
        self.current_boot = 'boot-two'
        self.witness = {'condition': True, 'action': False}
        self.events.clear()
        with (patch.object(wake.time, 'monotonic', side_effect=[0, 41, 41]),
              patch.object(wake.time, 'sleep')):
            result = self.prepare_boot()
        self.assertFalse(result['ok'])
        self.assertEqual(json.loads(wake.PENDING.read_text())['state'], 'failed')
        self.assertEqual(self.values[wake.PATH_KEY], str(wake.STOCK))
        self.events.clear()
        with self.assertRaises(RuntimeError):
            wake.boot_check('prepare', 'a' * 32)
        self.assertEqual(self.events, [])

    def test_missing_policy_on_new_boot_refuses_mutation(self):
        wake.stage()
        wake.trial()
        wake.persist()
        self.current_boot = 'boot-two'
        wake.POLICY.unlink()
        self.events.clear()
        with self.assertRaises((OSError, RuntimeError)):
            wake.boot_check('prepare', 'a' * 32)
        self.assertEqual(json.loads(wake.PENDING.read_text())['state'], 'armed')
        self.assertEqual(self.events, [])

    def test_foreign_config_on_new_boot_refuses_automatic_restore(self):
        wake.stage()
        wake.trial()
        wake.persist()
        self.current_boot = 'boot-two'
        self.values[wake.PATH_KEY] = '/foreign/policy'
        self.events.clear()
        with self.assertRaises(RuntimeError):
            wake.boot_check('prepare', 'a' * 32)
        self.assertEqual(self.values[wake.PATH_KEY], '/foreign/policy')
        self.assertEqual(json.loads(wake.PENDING.read_text())['state'], 'armed')
        self.assertEqual(self.events, [])

    def test_foreign_config_never_overwritten_by_off(self):
        wake.stage()
        self.values[wake.PATH_KEY] = '/foreign/policy'
        with self.assertRaisesRegex(RuntimeError, 'ownership'):
            wake.off()
        self.assertEqual(self.events, [])

    def test_missing_factory_database_refuses_trial_and_off(self):
        wake.stage()
        wake.FACTORY_DB.unlink()
        with self.assertRaises(FileNotFoundError):
            wake.trial()
        with self.assertRaises(FileNotFoundError):
            wake.off()
        self.assertEqual(self.events, [])
        self.assertFalse(wake.TRIAL.exists())

    def test_off_refuses_hidden_foreign_factory_override(self):
        wake.stage()
        foreign = {wake.PATH_KEY: '/foreign/policy'}
        write_db(wake.FACTORY_DB, foreign)
        pending = {'owner': wake.OWNER, 'state': 'armed'}
        wake.PENDING.write_text(json.dumps(pending))
        with self.assertRaises(RuntimeError):
            wake.off()
        self.assertEqual(self.events, [])
        self.assertEqual(wake.database_entries(wake.FACTORY_DB), foreign)
        self.assertEqual(json.loads(wake.PENDING.read_text()), pending)

    def test_activation_and_invocation_tokens_bind_phases_and_commit(self):
        wake.stage()
        wake.trial()
        wake.persist()
        pending = json.loads(wake.PENDING.read_text())
        report = self.prepare_boot()
        self.assertEqual(report['activation_token'], pending['activation_token'])
        for phase in ('landing', 'finalize'):
            with self.subTest(phase=phase), self.assertRaises(RuntimeError):
                wake.boot_check(phase, 'b' * 32)
        altered = dict(pending, activation_token='c' * 32)
        wake.PENDING.write_text(json.dumps(altered))
        with self.assertRaises(RuntimeError):
            wake.boot_check('landing', 'a' * 32)
        wake.PENDING.write_text(json.dumps(pending))
        report = wake.boot_check('finalize', 'a' * 32)
        self.assertTrue(report['ok'])
        wake.PENDING.write_text(json.dumps(altered))
        with self.assertRaises(RuntimeError):
            wake.commit()
        wake.PENDING.write_text(json.dumps(pending))
        wake.commit()
        report = self.prepare_boot('b' * 32)
        self.assertEqual(report['activation_token'], pending['activation_token'])
        self.assertEqual(report['invocation_token'], 'b' * 32)
        with self.assertRaises(RuntimeError):
            wake.boot_check('landing', 'a' * 32)

    def test_newer_foreground_is_not_overridden_or_accepted_as_original(self):
        wake.stage()
        wake.trial()
        wake.persist()
        self.prepare_boot()
        self.app = 'netflix'
        landing = wake.boot_check('landing', 'a' * 32)
        self.assertEqual(landing['appId'], 'netflix')
        self.assertEqual(landing['source'], 'newer-foreground')
        report = wake.boot_check('finalize', 'a' * 32)
        self.assertFalse(report['ok'])
        with self.assertRaises(RuntimeError):
            wake.commit()

    def test_delivered_livetv_is_corrected_but_newer_plex_is_preserved(self):
        self.app = wake.HOME
        wake.stage()
        wake.trial()
        wake.persist()
        write_db(wake.FACTORY_DB, wake.active(wake.snapshot()))
        target = {'appId': wake.HOME, 'params': {}, 'source': 'resume'}
        delivered = {'appId': 'com.webos.app.livetv', 'params': {}, 'source': 'native-action'}
        self.app = delivered['appId']
        with (patch.object(wake, 'saved_id', return_value=wake.HOME),
              patch.object(wake, 'native_cycle', return_value='native cycle'),
              patch.object(wake, 'native_delivered', return_value=delivered),
              patch.object(wake, 'resolve_intent', return_value=target)):
            report = wake.boot_check('prepare', 'a' * 32)
            self.assertEqual(report['target']['appId'], wake.HOME)
            self.assertEqual(wake.boot_check('landing', 'a' * 32)['appId'], wake.HOME)
            self.app = 'com.beb.plxnative'
            report = wake.boot_check('prepare', 'b' * 32)
            self.assertEqual(report['target']['appId'], self.app)
            self.assertEqual(wake.boot_check('landing', 'b' * 32)['appId'], self.app)

    def test_missing_delivery_provenance_is_unknown_not_a_manual_choice(self):
        wake.stage()
        wake.trial()
        wake.persist()
        write_db(wake.FACTORY_DB, wake.active(wake.snapshot()))
        self.app = 'com.webos.app.livetv'
        with (patch.object(wake, 'saved_id', return_value=wake.HOME),
              patch.object(wake, 'native_cycle', return_value='native cycle'),
              patch.object(wake, 'native_delivered', return_value=None),
              patch.object(wake, 'resolve_intent', return_value={'appId': wake.HOME, 'params': {}, 'source': 'resume'})):
            report = wake.boot_check('prepare', 'a' * 32)
        self.assertFalse(report['ok'])
        self.assertEqual(report['phase'], 'intent-unavailable')
        self.assertIn('provenance unavailable', report['error'])

    def test_finalize_requires_native_readiness_but_not_matching_first_app(self):
        wake.stage()
        wake.trial()
        wake.persist()
        prepared = self.prepare_boot()
        for status in ({'powerStatus': 'standby', 'signals': {'boot-done': True}},
                       {'powerStatus': 'active', 'signals': {'boot-done': False}},
                       {'powerStatus': 'active', 'signals': {}}):
            with self.subTest(status=status), patch.object(wake, 'boot_status', return_value=status):
                wake.REPORT.write_text(json.dumps(prepared))
                self.assertFalse(wake.boot_check('finalize', 'a' * 32)['ok'])
        wake.REPORT.write_text(json.dumps(prepared))
        with patch.object(wake, 'boot_status', return_value={
                'powerStatus': 'active', 'signals': {'boot-done': True}, 'firstAppId': 'com.webos.app.livetv'}):
            report = wake.boot_check('finalize', 'a' * 32)
        self.assertTrue(report['ok'])
        self.assertEqual(report['firstAppId'], 'com.webos.app.livetv')

    def test_manual_foreground_before_prepare_beats_old_native_input(self):
        wake.stage()
        wake.trial()
        wake.persist()
        write_db(wake.FACTORY_DB, wake.active(wake.snapshot()))
        self.app = 'com.beb.plxnative'
        old = {'appId': 'com.webos.app.hdmi2', 'params': {'boot': True}, 'source': 'native-action'}
        with (patch.object(wake, 'saved_id', return_value=wake.HOME),
              patch.object(wake, 'native_cycle', return_value='native cycle'),
              patch.object(wake, 'native_delivered', return_value=old),
              patch.object(wake, 'resolve_intent', return_value=old)):
            report = wake.boot_check('prepare', 'a' * 32)
        self.assertEqual(report['target']['appId'], self.app)
        self.assertEqual(report['native_target'], old)
        self.assertFalse(wake.boot_check('finalize', 'a' * 32)['ok'])

    def test_postcommit_native_input_is_accepted_only_while_foreground_matches(self):
        wake.stage()
        wake.trial()
        wake.persist()
        self.prepare_boot()
        self.assertTrue(wake.boot_check('finalize', 'a' * 32)['ok'])
        wake.commit()
        intent = {'appId': self.app, 'params': None, 'source': 'native-input-id-only'}
        with (patch.object(wake, 'saved_id', return_value=wake.HOME),
              patch.object(wake, 'native_cycle', return_value='native cycle'),
              patch.object(wake, 'resolve_intent', return_value=intent)):
            self.assertTrue(wake.boot_check('prepare', 'b' * 32)['ok'])
        self.assertIsNone(wake.boot_check('landing', 'b' * 32)['params'])
        self.assertTrue(wake.boot_check('finalize', 'b' * 32)['destination_accepted'])
        with (patch.object(wake, 'saved_id', return_value=wake.HOME),
              patch.object(wake, 'native_cycle', return_value='native cycle'),
              patch.object(wake, 'resolve_intent', return_value=intent)):
            self.assertTrue(wake.boot_check('prepare', 'c' * 32)['ok'])
        self.app = wake.HOME
        self.assertFalse(wake.boot_check('finalize', 'c' * 32)['destination_accepted'])

    def test_restart_landing_restores_input_after_incidental_home_only(self):
        wake.stage()
        wake.trial()
        wake.persist()
        self.prepare_boot()
        self.assertTrue(wake.boot_check('finalize', 'a' * 32)['ok'])
        wake.commit()
        intent = {'appId': 'com.webos.app.hdmi2', 'params': None, 'source': 'native-input-id-only'}
        with (patch.object(wake, 'saved_id', return_value=wake.HOME),
              patch.object(wake, 'native_cycle', return_value='native cycle'),
              patch.object(wake, 'resolve_intent', return_value=intent)):
            self.assertTrue(wake.boot_check('prepare', 'b' * 32)['ok'])
        self.app = wake.HOME
        landing = wake.boot_check('landing', 'b' * 32, 'restart', intent['appId'])
        self.assertEqual(landing['appId'], intent['appId'])
        self.assertEqual(landing['source'], intent['source'])
        self.assertIsNone(landing['params'])
        self.app = ''
        self.assertEqual(wake.boot_check('landing', 'b' * 32, 'restart', intent['appId'])['appId'], intent['appId'])
        self.app = wake.HOME
        for token, context, captured in (('c' * 32, 'restart', intent['appId']),
                                         ('b' * 32, 'restart', 'com.webos.app.hdmi1'),
                                         ('b' * 32, 'restart', ''),
                                         ('b' * 32, 'invalid', intent['appId'])):
            with self.subTest(token=token, context=context, captured=captured), self.assertRaises(RuntimeError):
                wake.boot_check('landing', token, context, captured)
        self.assertEqual(wake.boot_check('landing', 'b' * 32)['appId'], wake.HOME)
        self.app = 'com.beb.plxnative'
        self.assertEqual(wake.boot_check('landing', 'b' * 32, 'restart', intent['appId'])['appId'], self.app)
        self.app = wake.HOME
        wake.LOG.write_text('DEBUG(9.000000000) : BootSequencer) onPowerStatusChange: '
                            'subProcessing(prepare power on)\n')
        self.assertEqual(wake.boot_check('landing', 'b' * 32, 'restart', intent['appId'])['appId'], wake.HOME)

    def test_rollback_keeps_guard_on_recovery_error(self):
        wake.stage()
        saved = wake.snapshot()
        with (patch.object(wake, 'service', side_effect=RuntimeError('restart failed')),
              patch.object(wake, 'boot_status') as status):
            errors = wake.rollback(saved, True)
        self.assertTrue(any('restart failed' in error for error in errors))
        self.assertIn(('guard-install',), self.events)
        self.assertNotIn(('guard-remove',), self.events)
        status.assert_not_called()

    def test_rollback_waits_for_boot_status_before_removing_guard(self):
        wake.stage()
        saved = wake.snapshot()
        def readiness():
            self.events.append(('boot-status',))
            if sum(event == ('boot-status',) for event in self.events) == 1:
                raise RuntimeError('not ready')
            return {'firstAppId': self.app}
        with (patch.object(wake, 'boot_status', side_effect=readiness),
              patch.object(wake.time, 'monotonic', side_effect=[0, 1]),
              patch.object(wake.time, 'sleep') as sleep):
            errors = wake.rollback(saved, True)
        self.assertEqual(errors, [])
        self.assertEqual([event[0] for event in self.events],
                         ['guard-install', 'config', 'config', 'service', 'boot-status',
                          'boot-status', 'guard-remove'])
        sleep.assert_called_once_with(1)

    def test_rollback_readiness_timeout_retains_guard(self):
        wake.stage()
        saved = wake.snapshot()
        with (patch.object(wake, 'boot_status', side_effect=RuntimeError('not ready')) as status,
              patch.object(wake.time, 'monotonic', side_effect=[0, 1, 5]),
              patch.object(wake.time, 'sleep') as sleep):
            errors = wake.rollback(saved, True)
        self.assertTrue(any('ready' in error for error in errors))
        self.assertEqual(status.call_count, 2)
        sleep.assert_called_once_with(1)
        self.assertIn(('guard-install',), self.events)
        self.assertNotIn(('guard-remove',), self.events)


class LayeredConfigdTests(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        root = Path(tmp.name)
        paths = {name: root / (name.lower() + '.json') for name in
                 ('STOCK', 'POLICY', 'PENDING', 'TRIAL', 'REPORT', 'FAILURE')}
        patcher = patch.multiple(wake, **paths)
        patcher.start()
        self.addCleanup(patcher.stop)
        self.original = {wake.PATH_KEY: str(wake.STOCK),
                         wake.REASONS_KEY: ['aiKey', 'remoteKey']}
        self.active = {wake.PATH_KEY: str(wake.POLICY), wake.REASONS_KEY: ['aiKey']}
        self.persistent = copy.deepcopy(self.active)
        self.volatile = copy.deepcopy(self.active)
        self.events = []

        def luna(method, payload):
            if method == 'setConfigs':
                volatile = payload.get('volatile', False)
                self.events.append(('config', volatile, copy.deepcopy(payload['configs'])))
                layer = self.volatile if volatile else self.persistent
                layer.update(copy.deepcopy(payload['configs']))
                return {'returnValue': True}
            self.assertEqual(method, 'getConfigs')
            self.assertEqual(payload['configNames'], [wake.PATH_KEY, wake.REASONS_KEY])
            effective = {**self.original, **self.volatile, **self.persistent}
            return {'returnValue': True, 'configs': copy.deepcopy(effective)}

        methods = dict(luna=luna,
                       guard_install=lambda: self.events.append(('guard-install',)),
                       guard_remove=lambda: self.events.append(('guard-remove',)),
                       service=lambda action: self.events.append(('service', action)),
                       boot_status=lambda: {})
        patcher = patch.multiple(wake, **methods)
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_rollback_restores_factory_before_volatile(self):
        with self.assertRaises(RuntimeError):
            wake.set_configs(self.original, volatile=True)
        self.assertEqual(wake.configs(), self.active)

        self.volatile = copy.deepcopy(self.active)
        self.events.clear()
        saved = {'prior_path': str(wake.STOCK), 'prior_reasons': self.original[wake.REASONS_KEY]}
        self.assertEqual(wake.rollback(saved, True), [])
        self.assertEqual(self.events, [('guard-install',), ('config', False, self.original),
                                       ('config', True, self.original), ('service', 'restart'),
                                       ('guard-remove',)])
        self.assertEqual(self.volatile, self.original)
        self.assertEqual(self.persistent, self.original)
        self.assertEqual(wake.configs(), self.original)

    def test_status_uses_effective_path_and_pending_marker(self):
        self.volatile = copy.deepcopy(self.original)
        self.assertEqual(wake.status()['phase'], 'enabled')
        self.assertEqual(wake.status()['config'], self.active)
        wake.set_configs(self.original)
        self.volatile = copy.deepcopy(self.active)
        self.assertEqual(wake.status()['phase'], 'off')
        self.assertEqual(wake.status()['config'], self.original)
        marker = {'owner': wake.OWNER, 'state': 'armed'}
        wake.PENDING.write_text(json.dumps(marker))
        result = wake.status()
        self.assertEqual(result['phase'], 'pending')
        self.assertEqual(result['config'], self.original)
        self.assertEqual(result['pending'], marker)


class NativeInterfacesTests(unittest.TestCase):
    def test_timed_out_restart_resolves_same_job_without_restarting(self):
        timeout = subprocess.TimeoutExpired('systemctl', 6)
        with (patch.object(wake, 'systemctl', side_effect=[timeout,
                'Job=42\nActiveState=activating\nResult=success\nExecMainStatus=0\n',
                'Job=\nActiveState=active\nResult=success\nExecMainStatus=0\n']) as ctl,
              patch.object(wake.time, 'sleep')):
            wake.service('restart')
        self.assertEqual(sum(call.args[0] == 'restart' for call in ctl.call_args_list), 1)

    def test_unresolved_restart_never_starts_rollback(self):
        with (patch.object(wake, 'systemctl', side_effect=[subprocess.TimeoutExpired('systemctl', 6),
                'Job=42\nActiveState=activating\nResult=success\nExecMainStatus=0\n']),
              patch.object(wake.time, 'monotonic', side_effect=[0, 0, 31]),
              patch.object(wake.time, 'sleep')):
            with self.assertRaises(wake.JobUnresolved):
                wake.service('restart')

    def test_failed_job_read_remains_unresolved_without_recovery_restart(self):
        with tempfile.TemporaryDirectory() as d:
            with (patch.object(wake, 'FAILURE', Path(d) / 'failure.json'),
                  patch.object(wake, 'rollback') as rollback,
                  patch.object(wake, 'guard_remove') as guard_remove,
                  patch.object(wake, 'systemctl', side_effect=[subprocess.TimeoutExpired('systemctl', 6),
                                                               RuntimeError('show failed')]),
                  patch.object(wake.time, 'monotonic', side_effect=[0, 0, 31]),
                  patch.object(wake.time, 'sleep')):
                with self.assertRaises(wake.JobUnresolved) as raised:
                    wake.service('restart')
                result = wake.record_failure('prepare', raised.exception, {}, True)
            self.assertTrue(result['rollback_errors'])
            rollback.assert_not_called()
            guard_remove.assert_not_called()

    def test_guard_checks_systemd_and_never_removes_foreign_dropin(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / 'bootd.service.d' / 'guard.conf'
            calls = []
            def ctl(*args):
                calls.append(args)
                return 'Restart=no\nStartLimitAction=none\n' if args[0] == 'show' else ''
            with patch.multiple(wake, DROPIN=path, systemctl=ctl):
                wake.guard_install()
                self.assertEqual(path.read_text(), wake.GUARD)
                self.assertEqual(calls[0], ('daemon-reload',))
                wake.guard_remove()
                self.assertFalse(path.exists())
                path.write_text('foreign')
                with self.assertRaisesRegex(RuntimeError, 'foreign'):
                    wake.guard_remove()
                self.assertEqual(path.read_text(), 'foreign')

    def test_log_witness_accepts_fresh_append_truncation_replacement_and_regrow(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / 'bootd.log'
            policy = Path(d) / 'policy.json'
            marker = 'PERFORMANCE(  10.000000001) : Boot) BOOTD_START\n'
            condition = 'Condition) Read configuration (' + str(policy) + ')\n'
            action = 'Action) Read configuration (' + str(policy) + ')\n'
            with (patch.multiple(wake, LOG=log, POLICY=policy),
                  patch.object(wake.time, 'monotonic_ns', return_value=10_000_000_000) as clock):
                for operation in ('append', 'truncate', 'replace', 'regrow'):
                    with self.subTest(operation=operation):
                        log.write_text('old log\n' * 200)
                        clock.return_value = 10_000_000_000
                        cursor = wake.log_cursor()
                        clock.return_value = 20_000_000_000
                        if operation == 'append':
                            with log.open('a') as stream:
                                stream.write(marker + condition + action)
                        elif operation == 'truncate':
                            log.write_text(marker + condition + action)
                            self.assertLess(log.stat().st_size, cursor['size'])
                        elif operation == 'replace':
                            replacement = Path(d) / 'replacement.log'
                            replacement.write_text(marker + condition + action)
                            replacement.replace(log)
                            self.assertNotEqual(log.stat().st_ino, cursor['inode'])
                        else:
                            log.write_text('padding\n' * 300 + marker + condition + action)
                            self.assertEqual(log.stat().st_ino, cursor['inode'])
                            self.assertGreater(log.stat().st_size, cursor['size'])
                        self.assertEqual(wake.log_witness(cursor), {'condition': True, 'action': True})

    def test_log_witness_requires_fresh_well_formed_latest_marker(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / 'bootd.log'
            policy = Path(d) / 'policy.json'
            condition = 'Condition) Read configuration (' + str(policy) + ')\n'
            action = 'Action) Read configuration (' + str(policy) + ')\n'
            with (patch.multiple(wake, LOG=log, POLICY=policy),
                  patch.object(wake.time, 'monotonic_ns', return_value=20_000_000_000)):
                cursor = {'inode': 1, 'size': 0, 'monotonic_ns': 10_000_000_000}
                for marker, message in (
                        ('PERFORMANCE(9.999999999) : Boot) BOOTD_START', 'outside this trial: 9999999999'),
                        ('PERFORMANCE(20.000000001) : Boot) BOOTD_START', 'outside this trial: 20000000001'),
                        ('PERFORMANCE(10.00000000) : Boot) BOOTD_START', 'timestamp missing or malformed'),
                        ('PERFORMANCE(10.0000000000) : Boot) BOOTD_START', 'timestamp missing or malformed'),
                        ('Boot) BOOTD_START', 'timestamp missing or malformed'),
                        ('prefix PERFORMANCE(10.000000000) : Boot) BOOTD_START', 'timestamp missing or malformed')):
                    with self.subTest(marker=marker):
                        log.write_text(marker + '\n' + condition + action)
                        with self.assertRaisesRegex(RuntimeError, message):
                            wake.log_witness(cursor)
                for marker in ('PERFORMANCE(10.000000000) : Boot) BOOTD_START',
                               'PERFORMANCE(20.000000000) : Boot) BOOTD_START'):
                    with self.subTest(marker=marker):
                        log.write_text(marker + '\n' + condition + action)
                        self.assertEqual(wake.log_witness(cursor), {'condition': True, 'action': True})
                log.write_text('PERFORMANCE(10.000000000) : Boot) BOOTD_START\n' + condition + action +
                               'Boot) BOOTD_START\n')
                with self.assertRaisesRegex(RuntimeError, 'timestamp missing or malformed'):
                    wake.log_witness(cursor)
                log.write_text('PERFORMANCE(10.000000000) : Boot) BOOTD_START\n' + condition + action +
                               'PERFORMANCE(11.000000000) : Boot) BOOTD_START\n' + condition)
                self.assertEqual(wake.log_witness(cursor), {'condition': True, 'action': False})
                log.write_text('PERFORMANCE(10.000000000) : Boot) BOOTD_START\n' + action)
                self.assertEqual(wake.log_witness(cursor), {'condition': False, 'action': True})

    def test_log_witness_reads_only_last_mebibyte_with_cursor(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / 'bootd.log'
            policy = Path(d) / 'policy.json'
            marker = 'PERFORMANCE(10.000000000) : Boot) BOOTD_START\n'
            records = ('Condition) Read configuration (' + str(policy) + ')\n' +
                       'Action) Read configuration (' + str(policy) + ')\n')
            with (patch.multiple(wake, LOG=log, POLICY=policy),
                  patch.object(wake.time, 'monotonic_ns', return_value=20_000_000_000)):
                cursor = {'inode': 1, 'size': 0, 'monotonic_ns': 10_000_000_000}
                log.write_text(marker + records + 'x' * 1048576)
                self.assertEqual(wake.log_witness(cursor), {'condition': False, 'action': False})
                with log.open('a') as stream:
                    stream.write('\n' + marker + records)
                self.assertEqual(wake.log_witness(cursor), {'condition': True, 'action': True})


    def test_log_witness_uses_only_latest_bootd_start(self):
        with tempfile.TemporaryDirectory() as d:
            log = Path(d) / 'bootd.log'
            policy = Path(d) / 'policy.json'
            condition = 'Condition) Read configuration (' + str(policy) + ')\n'
            action = 'Action) Read configuration (' + str(policy) + ')\n'
            with patch.multiple(wake, LOG=log, POLICY=policy):
                log.write_text(condition + action)
                self.assertEqual(wake.log_witness(), {'condition': False, 'action': False})
                log.write_text('Boot) BOOTD_START\n' + condition + action + 'Boot) BOOTD_START\n')
                self.assertEqual(wake.log_witness(), {'condition': False, 'action': False})
                with log.open('a') as f:
                    f.write(condition)
                self.assertEqual(wake.log_witness(), {'condition': True, 'action': False})
                with log.open('a') as f:
                    f.write(action)
                self.assertEqual(wake.log_witness(), {'condition': True, 'action': True})
