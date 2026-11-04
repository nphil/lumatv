#!/usr/bin/env python3
"""Renders the Rust shell's channel banner on Live TV (`lumatv-ui --host --kinds banner`: the engine in-process on Mesa, as the compositor runs it)
against the compositor's stand-in channel and a fake of LG's tv/channel services, dark and light, at 100 % and 200 % text, into
build/renders/banner/ as 1280-wide JPEGs: a channel with its logo, without one, with long names, and INFO's detail.

    python3 tests/render_rust_banner.py [--out build/renders/banner] [--only channel,channel-info] [--list]

A scene is what the tuner says (the channel, its programmes) and whether INFO is pressed; the picture is taken with the banner up, with the
host's layout and contrast lint (`--eval lint`). `--timeout` also checks that the banner is gone after its five seconds.
"""
import argparse
import concurrent.futures
import json
import os
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
import rust_settings_host as host  # noqa: E402
import lgsettings  # noqa: E402

OUT = os.path.join(ROOT, 'build', 'renders', 'banner')
WIDTH = 1280
QUALITY = 80
# the client must be up (connected and polling) before Live TV is put in front, or the tuner is asked for before anyone listens
OPEN_AT = 2500
INFO_AT = 1500
SETTLE = 2400
# LG's banner is up five seconds (`HOLD_CHANNEL_MS`); the check for it being gone waits this long after the first answer
GONE_AFTER = 7500
LIVE_TV = 'com.webos.app.livetv'
CHANNEL_ID = '1_32_2_1_0_0_0'
TZ_OFFSET = -5 * 3600


def logo_png(path, text='WSB'):
    """A stand-in for a station's logo as LG keeps them: a 356 x 200 PNG, white letters on a colour, with room round them."""
    from PIL import Image, ImageDraw, ImageFont
    im = Image.new('RGBA', (356, 200), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((18, 18, 338, 182), radius=22, fill=(196, 52, 52, 255))
    try:
        font = ImageFont.truetype('DejaVuSans-Bold.ttf', 84)
    except OSError:
        font = ImageFont.load_default()
    w = d.textlength(text, font=font)
    d.text(((356 - w) / 2, 52), text, font=font, fill=(255, 255, 255, 255))
    im.save(path)


def stamp(ms, offset_s=0):
    """LG's `YYYY,MM,DD,hh,mm,ss` for an epoch in ms, on the clock `offset_s` east of UTC."""
    t = time.gmtime(ms / 1000.0 + offset_s)
    return '%04d,%02d,%02d,%02d,%02d,%02d' % (t.tm_year, t.tm_mon, t.tm_mday, t.tm_hour, t.tm_min, t.tm_sec)


def programme(channel, title, start_ms, end_ms, text, rating=None, captions=True, described=False):
    doc = {'programId': '%s_%d' % (channel, start_ms // 60000), 'channelId': channel, 'programName': title, 'programMainTitle': title, 'description': text,
           'startTime': stamp(start_ms), 'endTime': stamp(end_ms), 'localStartTime': stamp(start_ms, TZ_OFFSET), 'localEndTime': stamp(end_ms, TZ_OFFSET),
           'isSubtitle': captions, 'isAD': described, 'rating': []}
    if rating:
        doc['rating'] = [{'dimension': [{'dim': 0, 'dimRatingString': 'TV ' + rating}]}]
    return doc


class Scene:
    def __init__(self, name, number='2-1', channel='WSB-HD', logo=True, now='Evening News', then='Jeopardy!', about='Local and national news, weather and sports.',
                 rating='TV-PG', info=False):
        self.name, self.number, self.channel, self.logo, self.now, self.then, self.about, self.rating, self.info = name, number, channel, logo, now, then, about, rating, info


LONG_ABOUT = ('A long look at the week in the state: the legislature, the weather that came through on Tuesday, the high school football scores and a '
              'feature on the people who keep the old railway running, with an interview at the depot and a segment on how it began.')
SCENES = [
    Scene('channel'),
    Scene('channel-nologo', logo=False, rating=None),
    Scene('channel-long', number='100-12', channel='Georgia Public Broadcasting Atlanta Multicast', now='The Extraordinarily Long Title of an Evening Programme That Goes On',
          then='Another Programme With a Very Long Name Indeed', about=LONG_ABOUT),
    Scene('channel-info', info=True, about=LONG_ABOUT),
]
BY_NAME = {s.name: s for s in SCENES}
VARIANTS = [('dark', 1.0), ('dark', 2.0), ('light', 1.0), ('light', 2.0)]


def answers(scene, logo):
    """LG's tv/channel services as they answer a TV on `scene`'s channel: the programme that started on the half hour and the one after."""
    def now_ms():
        return int(time.time() * 1000)

    def listing(doc):
        t = now_ms()
        start = t - t % (30 * 60_000)
        ch = (doc.get('payload') or {}).get('channelId') or [CHANNEL_ID]
        ch = ch[0] if isinstance(ch, list) else ch
        return {'returnValue': True, 'programList': [
            programme(ch, scene.now, start, start + 60 * 60_000, scene.about, scene.rating),
            programme(ch, scene.then, start + 60 * 60_000, start + 90 * 60_000, 'What comes next.', scene.rating)]}

    record = {'returnValue': True, 'channelList': [{'channelId': CHANNEL_ID, 'channelNumber': scene.number, 'channelName': scene.channel,
                                                     'imgUrl': logo if scene.logo else '', 'channelTypeId': 1}]}
    return {'/getLastChannelId': {'returnValue': True, 'channelId': CHANNEL_ID}, '/getChannelInfo': record, '/getCurrentNextEventList': listing}


def jpeg(png, path):
    from PIL import Image
    im = Image.open(png).convert('RGB')
    im = im.resize((WIDTH, round(im.height * WIDTH / im.width)), Image.LANCZOS)
    im.save(path, 'JPEG', quality=QUALITY, optimize=True, progressive=True)


def wait_up(chan, seconds=30.0):
    """Until the client has made its first report and is polling the channel's events."""
    end = time.time() + seconds
    while time.time() < end and not (chan.reports and chan.polls):
        time.sleep(0.05)


def play(scene, mode='dark', scale=1.0, shot=None, timeout=False):
    """Runs the scene; returns (rc, output, channel, evals by name, the png's path or None)."""
    work = tempfile.mkdtemp(prefix='bnrender-')
    shot = shot or os.path.join(work, scene.name + '.png')
    logo = os.path.join(work, 'logo.png')
    logo_png(logo)
    at = OPEN_AT + (INFO_AT + SETTLE if scene.info else SETTLE)
    extra = []
    if scene.info:
        extra += ['--key', '%d:info' % (OPEN_AT + INFO_AT)]
    evals = ['lint', 'state']
    for e in evals:
        extra += ['--eval', '%d:%s' % (at - 200, e)]
    extra += ['--snap', '%d:%s' % (at, shot)]
    wait = at + 600
    if timeout:
        extra += ['--eval', '%d:state' % (OPEN_AT + GONE_AFTER)]
        wait = OPEN_AT + GONE_AFTER + 600
    steps = [(OPEN_AT - 100, lambda c: wait_up(c)),
             (OPEN_AT, lambda c: c.push('foreground', {'app': LIVE_TV, 'home': False, 'covered': True}))]
    rc, text, chan, _, _ = host.run(steps=steps, wait=wait, extra=extra, lg=lgsettings.LgService(answers=answers(scene, logo)), kinds='banner', text_scale=scale, mode=mode,
                                    theme={'banners': {'own': True}})
    got = host.evals(text)
    return rc, text, chan, dict(zip(evals + (['gone'] if timeout else []), got)), shot if os.path.exists(shot) else None


def render(scene, mode='dark', scale=1.0, out=None):
    """Runs the scene; returns (rc, output, lint findings, state at the picture, the png's path or None). `out` is a path for a JPEG."""
    rc, text, _, got, png = play(scene, mode, scale)
    if png and out:
        os.makedirs(os.path.dirname(out), exist_ok=True)
        jpeg(png, out)
    findings = got.get('lint') if isinstance(got.get('lint'), list) else []
    return rc, text, findings, got.get('state'), png


def label(scene, mode, scale):
    return '%s%s%s' % (scene.name, '' if mode == 'dark' else '-' + mode, '' if scale == 1.0 else '-%dx' % int(scale))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--only', default='')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('--timeout', action='store_true', help='only check that the banner is gone after its time')
    ap.add_argument('--jobs', type=int, default=2)
    ap.add_argument('--bin', default=os.environ.get('LUMATV_UI_BIN'), help="another lumatv-ui than the debug build (a shared target directory is any worker's)")
    a = ap.parse_args()
    if a.bin:
        host.BIN = a.bin
    only = [n for n in a.only.split(',') if n]
    jobs = [(s, m, k) for s in SCENES if not only or s.name in only for m, k in VARIANTS]
    if a.list:
        print('\n'.join(label(*j) for j in jobs))
        return 0
    if not host.available():
        print('needs rustui/target/debug/lumatv-ui and Mesa', file=sys.stderr)
        return 1
    if a.timeout:
        rc, text, _, got, _ = play(BY_NAME['channel'], timeout=True)
        shown, gone = (got.get('state') or {}).get('banner') or {}, (got.get('gone') or {}).get('banner') or {}
        ok = rc == 0 and shown.get('up') is True and gone.get('up') is False
        print('%s banner up at the picture: %s, after %d ms: %s' % ('ok  ' if ok else 'FAIL', shown.get('up'), GONE_AFTER, gone.get('up')))
        if not ok:
            print(text[-1500:])
        return 0 if ok else 1
    bad = 0
    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for (s, mode, scale), (rc, text, findings, state, png) in zip(jobs, pool.map(lambda j: render(j[0], j[1], j[2], os.path.join(a.out, label(*j) + '.jpg')), jobs)):
            banner = (state or {}).get('banner') or {}
            up = banner.get('up') is True and (banner.get('info') is True) == s.info
            ok = rc == 0 and png is not None and not findings and up
            bad += not ok
            print('%s %s%s' % ('ok  ' if ok else 'FAIL', label(s, mode, scale), ''.join('\n     lint: %s %s %s' % (f['kind'], f['subject'], f['detail']) for f in findings)))
            if not ok and (rc != 0 or png is None or not up):
                print('     state: %s' % json.dumps(state))
                print(text[-1200:])
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
