#!/usr/bin/env python3
"""Renders the Rust shell's channel banner on Live TV (`lumatv-ui --host --kinds banner`: the engine in-process on Mesa, as the compositor runs it)
against the compositor's stand-in channel and a fake of LG's tv/channel services, dark and light, at 100 % and 200 % text, over a bright busy still
(an ad: white ground, black print, a phone number where the banner lands) and a dark one, into build/renders/banner/ as 1280-wide JPEGs: a channel
with its logo, without one, with long names, with nothing known of its programme, and INFO's detail.

    python3 tests/render_rust_banner.py [--out build/renders/banner] [--only channel,channel-info] [--backdrop ad,night] [--png] [--measure] [--list]

A scene is what the tuner says (the channel, its programmes) and whether INFO is pressed; the picture is taken with the banner up, with the
host's layout and contrast lint (`--eval lint`). `--png` keeps the 1920 x 1080 pictures beside the JPEGs, `--measure` reads the contrast of every
word on the picture itself (the background is the median of the word's box, the ink the pixel farthest from it) and fails below 4.5 to 1.
`--timeout` also checks that the banner is gone after its five seconds.
"""
import argparse
import concurrent.futures
import json
import os
import random
import shutil
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
BACKDROPS = ['ad', 'night']
# body text on the picture: 4.5 to 1 (Lucent `contrast.body`)
NEED = 4.5


def font(px):
    from PIL import ImageFont
    for name in ('DejaVuSans-Bold.ttf', 'LiberationSans-Bold.ttf'):
        try:
            return ImageFont.truetype(name, px)
        except OSError:
            continue
    return ImageFont.load_default()


def ad_png(path):
    """The worst picture a banner can land on: an ad. White ground, black print in rows, a poster of colour blocks above, and a phone number as big
    as the banner is tall right where the banner lands, so any picture that shows through its glass shows in the words."""
    from PIL import Image, ImageDraw
    w, h = 1920, 1080
    im = Image.new('RGB', (w, h), (252, 252, 250))
    d = ImageDraw.Draw(im)
    d.rectangle((0, 0, w, 300), fill=(255, 214, 10))
    d.polygon([(0, 0), (980, 0), (620, 300), (0, 300)], fill=(214, 40, 40))
    d.text((70, 70), 'FLOORS 70% OFF', font=font(150), fill=(255, 255, 255))
    for i in range(8):
        d.rectangle((70 + i * 228, 330, 70 + i * 228 + 200, 540), fill=[(30, 90, 200), (20, 20, 20), (230, 120, 0), (0, 150, 90)][i % 4])
    for row in range(14):
        d.text((70, 560 + row * 26), 'TERMS APPLY. OFFER ENDS SUNDAY. SEE STORE FOR DETAILS. NO PURCHASE NECESSARY. ' * 2, font=font(22), fill=(0, 0, 0))
    d.text((w / 2, 840), '1-800-555-0142', font=font(150), fill=(0, 0, 0), anchor='mm')
    d.text((w / 2, 960), 'CALL NOW  CALL NOW  CALL NOW  CALL NOW  CALL NOW', font=font(60), fill=(0, 0, 0), anchor='mm')
    for row in range(5):
        d.text((70, 1000 + row * 16), 'LICENSED AND INSURED. FINANCING AVAILABLE ON APPROVED CREDIT. ' * 3, font=font(14), fill=(0, 0, 0))
    im.convert('RGBA').save(path)


def night_png(path):
    """A dark picture with bright points in it, where a banner's edge and its words are most easily lost: a night street with lit windows."""
    from PIL import Image, ImageDraw
    rnd = random.Random(7)
    w, h = 1920, 1080
    im = Image.new('RGB', (w, h))
    d = ImageDraw.Draw(im)
    for y in range(h):
        k = y / h
        d.line((0, y, w, y), fill=(int(8 + 22 * k), int(10 + 28 * k), int(24 + 40 * k)))
    for i in range(14):
        x0 = i * 140 - 20
        top = rnd.randint(260, 640)
        d.rectangle((x0, top, x0 + 124, h), fill=(rnd.randint(14, 30), rnd.randint(16, 34), rnd.randint(26, 50)))
        for wy in range(top + 14, h - 20, 30):
            for wx in range(x0 + 12, x0 + 112, 24):
                if rnd.random() < 0.55:
                    d.rectangle((wx, wy, wx + 12, wy + 16), fill=(rnd.randint(200, 255), rnd.randint(170, 230), rnd.randint(70, 140)))
    d.ellipse((1500, 90, 1620, 210), fill=(236, 238, 226))
    im.convert('RGBA').save(path)


def backdrops(names, folder):
    """The stills by name written into `folder` (`ad`, `night`), or a PNG that is there already (a capture of the set's picture) under its file name
    without the extension; {label: path}."""
    made = {}
    for name in names:
        if os.path.isfile(name):
            made[os.path.splitext(os.path.basename(name))[0]] = os.path.abspath(name)
            continue
        path = os.path.join(folder, name + '.png')
        {'ad': ad_png, 'night': night_png}[name](path)
        made[name] = path
    return made


def logo_png(path, text='WSB'):
    """A stand-in for a station's logo as LG keeps them: a 356 x 200 PNG, white letters on a colour, with room round them."""
    from PIL import Image, ImageDraw
    im = Image.new('RGBA', (356, 200), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    d.rounded_rectangle((18, 18, 338, 182), radius=22, fill=(196, 52, 52, 255))
    f = font(84)
    w = d.textlength(text, font=f)
    d.text(((356 - w) / 2, 52), text, font=f, fill=(255, 255, 255, 255))
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
    """`now=None` is a channel LG has no programme for (the banner says "No information")."""

    def __init__(self, name, number='2-1', channel='WSB-HD', logo=True, now='Evening News', then='Jeopardy!', about='Local and national news, weather and sports.',
                 rating='TV-PG', info=False):
        self.name, self.number, self.channel, self.logo, self.now, self.then, self.about, self.rating, self.info = name, number, channel, logo, now, then, about, rating, info


LONG_ABOUT = ('A long look at the week in the state: the legislature, the weather that came through on Tuesday, the high school football scores and a '
              'feature on the people who keep the old railway running, with an interview at the depot and a segment on how it began.')
LONG = dict(number='100-12', channel='Georgia Public Broadcasting Atlanta Multicast', now='The Extraordinarily Long Title of an Evening Programme That Goes On',
            then='Another Programme With a Very Long Name Indeed', about=LONG_ABOUT)
SCENES = [
    Scene('channel'),
    Scene('channel-nologo', logo=False, rating=None),
    Scene('channel-long', **LONG),
    Scene('channel-quiet', number='5-3', channel='BUZZR', now=None, rating=None),
    Scene('channel-info', info=True, about=LONG_ABOUT),
    Scene('channel-info-long', info=True, logo=False, **LONG),
]
BY_NAME = {s.name: s for s in SCENES}
VARIANTS = [('dark', 1.0), ('dark', 2.0), ('light', 1.0), ('light', 2.0)]


def answers(scene, logo):
    """LG's tv/channel services as they answer a TV on `scene`'s channel: the programme that started on the half hour and the one after."""
    def now_ms():
        return int(time.time() * 1000)

    def listing(doc):
        if scene.now is None:
            return {'returnValue': True, 'programList': []}
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


def luminance(rgb):
    def lin(v):
        v /= 255.0
        return v / 12.92 if v <= 0.04045 else ((v + 0.055) / 1.055) ** 2.4
    return 0.2126 * lin(rgb[0]) + 0.7152 * lin(rgb[1]) + 0.0722 * lin(rgb[2])


def on_picture(png, words):
    """The contrast of each word `{name, x, y, w, h}` as drawn: the background is the median of its box in the finished picture, the ink the pixel
    farthest from it (Lucent section 3, "Measure text on the picture"). Returns {name: ratio}."""
    from PIL import Image
    im = Image.open(png).convert('RGB')
    out = {}
    for w in words:
        box = im.crop((int(w['x']), int(w['y']), int(w['x'] + w['w']), int(w['y'] + w['h'])))
        px = list(box.get_flattened_data() if hasattr(box, 'get_flattened_data') else box.getdata())
        bg = tuple(sorted(p[c] for p in px)[len(px) // 2] for c in range(3))
        lb = luminance(bg)
        ink = max(px, key=lambda p: abs(luminance(p) - lb))
        hi, lo = max(luminance(ink), lb), min(luminance(ink), lb)
        out[w['name']] = round((hi + 0.05) / (lo + 0.05), 2)
    return out


def wait_up(chan, seconds=30.0):
    """Until the client has made its first report and is polling the channel's events."""
    end = time.time() + seconds
    while time.time() < end and not (chan.reports and chan.polls):
        time.sleep(0.05)


def play(scene, mode='dark', scale=1.0, shot=None, timeout=False, backdrop=None, geometry=False):
    """Runs the scene; returns (rc, output, channel, evals by name, the png's path or None)."""
    work = tempfile.mkdtemp(prefix='bnrender-')
    shot = shot or os.path.join(work, scene.name + '.png')
    logo = os.path.join(work, 'logo.png')
    logo_png(logo)
    at = OPEN_AT + (INFO_AT + SETTLE if scene.info else SETTLE)
    extra = []
    if backdrop:
        extra += ['--backdrop', backdrop]
    if scene.info:
        extra += ['--key', '%d:info' % (OPEN_AT + INFO_AT)]
    evals = ['lint', 'state'] + (['geometry'] if geometry else [])
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


def render(scene, mode='dark', scale=1.0, out=None, backdrop=None, keep=None, measure=False):
    """Runs the scene; returns (rc, output, lint findings, state at the picture, the png's path or None, the contrast of each word on the picture).
    `out` is a path for a JPEG, `keep` for the picture itself."""
    rc, text, _, got, png = play(scene, mode, scale, backdrop=backdrop, geometry=measure)
    if png and out:
        os.makedirs(os.path.dirname(out), exist_ok=True)
        jpeg(png, out)
    if png and keep:
        os.makedirs(os.path.dirname(keep), exist_ok=True)
        shutil.copyfile(png, keep)
    findings = got.get('lint') if isinstance(got.get('lint'), list) else []
    seen = {}
    if png and measure:
        words = (((got.get('geometry') or {}).get('banner') or {}).get('words')) or []
        seen = on_picture(png, words)
    return rc, text, findings, got.get('state'), png, seen


def label(scene, mode, scale, backdrop=None):
    return '%s%s%s%s' % (scene.name, '' if mode == 'dark' else '-' + mode, '' if scale == 1.0 else '-%dx' % int(scale), '-' + backdrop if backdrop else '')


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--only', default='')
    ap.add_argument('--backdrop', default=','.join(BACKDROPS), help='stills behind the banner: ad, night, or the path of a PNG (a capture of the set); "none" for black')
    ap.add_argument('--png', action='store_true', help='keep the 1920 x 1080 pictures too')
    ap.add_argument('--measure', action='store_true', help='read every word\'s contrast on the picture')
    ap.add_argument('--list', action='store_true')
    ap.add_argument('--timeout', action='store_true', help='only check that the banner is gone after its time')
    ap.add_argument('--jobs', type=int, default=2)
    ap.add_argument('--bin', default=os.environ.get('LUMATV_UI_BIN'), help="another lumatv-ui than the debug build (a shared target directory is any worker's)")
    a = ap.parse_args()
    if a.bin:
        host.BIN = a.bin
    only = [n for n in a.only.split(',') if n]
    names = [] if a.backdrop == 'none' else [n for n in a.backdrop.split(',') if n]
    labels = [os.path.splitext(os.path.basename(n))[0] if os.path.isfile(n) else n for n in names]
    jobs = [(s, m, k, b) for s in SCENES if not only or s.name in only for m, k in VARIANTS for b in (labels or [None])]
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
    still = backdrops(names, tempfile.mkdtemp(prefix='bnstills-'))
    bad = 0

    def one(j):
        s, mode, scale, b = j
        base = label(*j)
        return render(s, mode, scale, os.path.join(a.out, base + '.jpg'), still.get(b), os.path.join(a.out, base + '.png') if a.png else None, a.measure)

    with concurrent.futures.ThreadPoolExecutor(a.jobs) as pool:
        for (s, mode, scale, b), (rc, text, findings, state, png, seen) in zip(jobs, pool.map(one, jobs)):
            banner = (state or {}).get('banner') or {}
            up = banner.get('up') is True and (banner.get('info') is True) == s.info
            weak = {k: v for k, v in seen.items() if v < NEED}
            ok = rc == 0 and png is not None and not findings and up and not weak
            bad += not ok
            worst = ('  worst on the picture %.2f (%s)' % min((v, k) for k, v in seen.items())) if seen else ''
            print('%s %s%s' % ('ok  ' if ok else 'FAIL', label(s, mode, scale, b), worst), end='')
            print(''.join('\n     lint: %s %s %s' % (f['kind'], f['subject'], f['detail']) for f in findings), end='')
            print(''.join('\n     picture: %s reads %.2f to 1, %.1f needed' % (k, v, NEED) for k, v in weak.items()))
            if not ok and (rc != 0 or png is None or not up):
                print('     state: %s' % json.dumps(state))
                print(text[-1200:])
    return 1 if bad else 0


if __name__ == '__main__':
    sys.exit(main())
