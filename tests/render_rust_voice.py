#!/usr/bin/env python3
"""Renders the voice assistant's screen effect (`lumatv-ui --host --kinds toast`: the edge glow, the words heard and the answer's card) and its
Settings page, the way the compositor runs the engine (in-process on Mesa), with the harness keeping its own time (`--virtual`) so the same
command gives the same frames. The pictures are PNGs in rustui/shots-out/voice-assistant/, which stays out of git.

    cargo build -p lumatv-ui
    python3 tests/render_rust_voice.py [--out rustui/shots-out/voice-assistant] [--only glow-thinking,card-ok] [--list] [--jobs 2]

`CARGO_TARGET_DIR` names the build's target when it is not rustui/target. The glow's two listening frames are a second apart. The glow is
also shown over the banner renders' two stills (an ad: bright; a night street: dark) under the dark and the light theme.
"""
import argparse
import concurrent.futures
import json
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
import rust_settings_host as host  # noqa: E402
import render_rust_banner as banner  # noqa: E402
import render_rust_settings as settings  # noqa: E402
import lgsettings  # noqa: E402

OUT = os.path.join(ROOT, 'rustui', 'shots-out', 'voice-assistant')
TOPIC = 'voice.assistant'
ID = 'voice-assistant'
# the first state arrives once the shell is up; the harness keeps its own time, so these are the same on a loaded host
FIRST = 1500
# the page is opened once the client is up and the picture taken after its reads were answered and drawn
OPEN = 2500
SETTLE = 1500
HEARD = 'turn on the living room lights'
LONG = 'turn on the living room lights and set the kitchen to warm white at half brightness'


def turn(phase, heard=None, reply=None, ok=None, spoken=False, error=None, mic='turn'):
    """The helper's `voice.assistant` topic (schema 1) for one turn."""
    return {'schema': 1, 'enabled': True, 'mic': mic, 'turn': {'id': 1, 'phase': phase, 'heard': heard, 'reply': reply, 'ok': ok, 'spoken': spoken, 'error': error}}


class Take:
    """What the helper says and when (ms), and the pictures taken: (ms, file name). `picture` is a backdrop's name, an app behind the effect."""

    def __init__(self, name, states, snaps, mode='dark', picture=None):
        self.name, self.states, self.snaps, self.mode, self.picture = name, states, snaps, mode, picture


TAKES = [
    Take('glow-listening', [(FIRST, turn('listening'))], [(FIRST + 1100, 'glow-listening-1'), (FIRST + 2100, 'glow-listening-2')]),
    Take('glow-thinking', [(FIRST, turn('listening', HEARD)), (FIRST + 700, turn('thinking', HEARD))], [(FIRST + 2000, 'glow-thinking')]),
    Take('transcript-mid-reveal', [(FIRST, turn('listening', LONG))], [(FIRST + 420, 'transcript-mid-reveal')]),
    Take('card-ok', [(FIRST, turn('listening', HEARD)), (FIRST + 1000, turn('done', HEARD, 'Done. The living room lights are on.', True, True, mic='listening'))],
         [(FIRST + 1900, 'card-ok')]),
    Take('card-failed', [(FIRST, turn('listening', HEARD)), (FIRST + 1000, turn('failed', HEARD, None, False, False, 'Home Assistant did not answer', 'listening'))],
         [(FIRST + 1900, 'card-failed')]),
]
for _mode in ('dark', 'light'):
    for _picture, _kind in (('ad', 'bright'), ('night', 'dark')):
        _name = 'glow-over-%s-picture-%s-theme' % (_kind, _mode)
        TAKES.append(Take(_name, [(FIRST, turn('listening', HEARD))], [(FIRST + 1700, _name)], mode=_mode, picture=_picture))

CONSENT = "Listens for the wake word through the TV's microphone and sends what is said to Home Assistant."
SCHEMA = {'sections': [{'id': 'assistant', 'title': 'Voice assistant', 'fields': [
    {'key': 'status', 'label': 'Status', 'kind': {'type': 'read-only', 'value': {'capability': 'plugin.%s.status' % ID}}},
    {'key': 'microphone', 'label': 'Microphone', 'kind': {'type': 'read-only', 'value': {'capability': 'plugin.%s.microphone' % ID}}},
    {'key': 'homeassistant', 'label': 'Home Assistant', 'kind': {'type': 'read-only', 'value': {'capability': 'plugin.%s.homeassistant' % ID}}},
    {'key': 'last', 'label': 'Last heard', 'kind': {'type': 'read-only', 'value': {'capability': 'plugin.%s.last' % ID}}},
    {'key': 'wake-word', 'label': 'Wake word', 'kind': {'type': 'choice', 'options': {'inline': [{'id': 'okay_nabu', 'label': 'Okay Nabu'}, {'id': 'hey_jarvis', 'label': 'Hey Jarvis'}]}}},
    {'key': 'sensitivity', 'label': 'Sensitivity', 'kind': {'type': 'choice', 'options': {'inline': [
        {'id': 'low', 'label': 'Low'}, {'id': 'normal', 'label': 'Normal'}, {'id': 'high', 'label': 'High'}]}}},
    {'key': 'speak', 'label': 'Speak answers', 'kind': {'type': 'bool'}},
    {'key': 'volume', 'label': 'Answer volume', 'kind': {'type': 'integer', 'min': 1, 'max': 100, 'step': 5, 'unit': '%'}},
    {'key': 'quiet-hours', 'label': 'Quiet at night', 'help': 'Answers are shown, not spoken, from 22:00 to 07:00.', 'kind': {'type': 'bool'}},
    {'key': 'pair', 'label': 'Pair with Home Assistant', 'kind': {'type': 'action', 'action': 'pair', 'verb': 'Pair', 'confirmation': 'none'}},
    {'key': 'forget', 'label': 'Forget Home Assistant', 'kind': {'type': 'action', 'action': 'forget', 'verb': 'Forget', 'confirmation': 'destructive'}}]}]}
VALUES = {'wake-word': 'okay_nabu', 'sensitivity': 'normal', 'speak': True, 'volume': 60, 'quiet-hours': True}


def plugins(state):
    return {'plugins': [], 'firstParty': [{'id': ID, 'name': 'Voice assistant', 'state': state, 'firstParty': True, 'consent': CONSENT, 'approve': 'c0de'}]}


def page(name, state, facts=None):
    routes = {'GET /v1/settings/%s' % ID: settings.plugin_doc(SCHEMA, VALUES)} if facts else {}
    topics = {'plugins': plugins(state)}
    if facts:
        topics['plugin.%s.facts' % ID] = {'facts': facts}
    return settings.Scene(name, ID, routes=routes, topics=topics)


PAGES = [
    page('settings-off', 'off'),
    page('settings-on-not-paired', 'running', {'status': 'Not paired', 'microphone': 'Ready', 'homeassistant': 'Not paired', 'last': 'Never'}),
    page('settings-on-paired', 'running', {'status': 'Listening for Okay Nabu', 'microphone': 'Ready', 'homeassistant': 'Paired with Home Assistant', 'last': 'Today at 7:42 PM'}),
]


def target():
    return os.path.join(os.environ.get('CARGO_TARGET_DIR') or os.path.join(ROOT, 'rustui', 'target'), 'debug', 'lumatv-ui')


def effect(take, out, pictures):
    """Runs the take; returns (rc, output, evals, the files made)."""
    extra = ['--virtual', '--refresh', '60']
    if take.picture:
        extra += ['--backdrop', pictures[take.picture]]
    for ms, doc in take.states:
        extra += ['--helper', '%d:%s:%s' % (ms, TOPIC, json.dumps(doc))]
    last = take.snaps[-1][0]
    extra += ['--eval', '%d:lint' % (last - 50), '--eval', '%d:state' % (last - 50)]
    files = []
    for ms, name in take.snaps:
        files.append(os.path.join(out, name + '.png'))
        extra += ['--snap', '%d:%s' % (ms, files[-1])]
    rc, text, _, _, _ = host.run(wait=last + 400, extra=extra, kinds='toast', mode=take.mode)
    return rc, text, host.evals(text), files


def page_shot(scene, shot):
    """Opens the scene's page of Settings on a client that is already polling (a `show` that comes before the client's first read is taken for an old
    press and dropped) and takes the picture; returns (rc, output, evals, the files made)."""
    at = OPEN + SETTLE
    extra = ['--eval', '%d:lint' % (at - 200), '--eval', '%d:state' % (at - 200), '--snap', '%d:%s' % (at, shot)]
    steps = [(OPEN - 200, banner.wait_up), (OPEN, lambda c: c.push('foreground', {'app': 'com.webos.app.home', 'home': True})),
             (OPEN + 100, lambda c: c.push('show', {'kind': 'settings', 'page': ID}))]
    rc, text, _, _, _ = host.run(steps=steps, wait=at + 800, extra=extra, lg=lgsettings.LgService(answers=settings.ONLINE), helper=host.FakeHelper(scene.helper_routes()))
    return rc, text, host.evals(text), [shot]


def say(ok, files, lint, state):
    lines = ['%s %s' % ('ok  ' if ok else 'FAIL', os.path.relpath(f, ROOT)) for f in files]
    lines += ['     lint: %s %s %s' % (f['kind'], f['subject'], f['detail']) for f in lint]
    if state:
        lines.append('     room left: %s' % json.dumps(state.get('room')))
    return '\n'.join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--only', default='')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('--jobs', type=int, default=2)
    a = ap.parse_args()
    if a.list:
        print('\n'.join(t.name for t in TAKES + PAGES))
        return 0
    host.BIN = target()
    if not host.available():
        print('needs %s (cargo build -p lumatv-ui) and Mesa' % host.BIN, file=sys.stderr)
        return 1
    only = [n for n in a.only.split(',') if n]
    os.makedirs(a.out, exist_ok=True)
    bad = 0
    with tempfile.TemporaryDirectory(prefix='voicerender-') as work:
        # the harness's data directories and the backdrops land here and go with it
        tempfile.tempdir = work
        pictures = banner.backdrops(['ad', 'night'], work)

        def run(job):
            if isinstance(job, Take):
                rc, text, evals, files = effect(job, a.out, pictures)
            else:
                rc, text, evals, files = page_shot(job, os.path.join(a.out, job.name + '.png'))
            lint = evals[0] if evals and isinstance(evals[0], list) else []
            state = evals[1] if len(evals) > 1 and isinstance(evals[1], dict) else {}
            return rc, text, files, lint, state

        jobs = [j for j in TAKES + PAGES if not only or j.name in only]
        with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
            for rc, text, files, lint, state in pool.map(run, jobs):
                ok = rc == 0 and all(os.path.exists(f) for f in files)
                bad += not ok
                print(say(ok, files, lint, state))
                if not ok:
                    print(text[-1500:])
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
