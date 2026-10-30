#!/usr/bin/env python3
"""Renders the Rust shell's weather card, full weather view and radar on the host harness (`lumatv-ui --host`: the engine in-process on Mesa, as the
compositor runs it) into docs/mocks/host-rust/home/: the card for every condition (a contact sheet, dark and light, 100 % and 200 % text), the view's
pages, the radar page, each from the same fixed clock and a fixed reading.

    python3 tests/render_rust_weather.py [--out DIR] [--only card,view,radar] [--bin PATH] [--jobs N]

`CASES` are the conditions, `reading` and `more` the helper's two documents for one of them (the card's `weather` and the view's `weather.more`).
"""
import argparse
import concurrent.futures
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
import test_rust_home as H  # noqa: E402
import rust_settings_host as SH  # noqa: E402

OUT = os.path.join(ROOT, 'docs', 'mocks', 'host-rust', 'home')
CLOCK = '2026-10-30T15:00'
CARD = (1423, 134, 433, 480)
DAYS = ['Fri', 'Sat', 'Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun']
KIND = {0: 'sun', 1: 'partly', 2: 'partly', 3: 'cloud', 45: 'fog', 53: 'drizzle', 61: 'rain', 63: 'rain', 65: 'rain', 73: 'snow', 75: 'snow', 95: 'storm', 96: 'storm'}

# name: (WMO code, day?, wind mph, label)
CASES = {
    'clear-day': (0, True, 6, 'Clear'),
    'clear-night': (0, False, 6, 'Clear'),
    'partly-day': (2, True, 8, 'Partly cloudy'),
    'partly-night': (2, False, 8, 'Partly cloudy'),
    'cloudy': (3, True, 9, 'Overcast'),
    'fog': (45, True, 3, 'Fog'),
    'drizzle': (53, True, 7, 'Drizzle'),
    'rain': (63, True, 8, 'Rain'),
    'rain-heavy': (65, True, 12, 'Heavy rain'),
    'storm': (95, True, 14, 'Thunderstorm'),
    'snow': (73, True, 6, 'Snow'),
    'windy': (1, True, 24, 'Mostly clear'),
}


def kind(code, day):
    k = KIND.get(code, 'cloud')
    if not day and k in ('sun', 'partly'):
        return 'moon' if k == 'sun' else 'partly-night'
    return k


def reading(case, now_ms):
    code, day, wind, label = CASES[case]
    k = kind(code, day)
    daily = [(71, 51, 'sun'), (73, 51, 'cloud'), (62, 48, 'rain'), (66, 56, 'partly'), (59, 44, 'drizzle'), (68, 42, 'sun'), (74, 55, 'storm')]
    days = [{"day": 'Today' if i == 0 else DAYS[i], "kind": kk, "hi": '%d\u00b0' % hi, "lo": '%d\u00b0' % lo} for i, (hi, lo, kk) in enumerate(daily)]
    doc = {"schema": 1, "fetchedAt": now_ms, "staleAt": now_ms + 900000, "expiresAt": now_ms + 86400000, "zip": "30301", "units": "fahrenheit", "place": "Atlanta", "lat": 33.75, "lon": -84.39,
           "current": {"time": CLOCK, "temp": 61.0, "feelsLike": 57.0, "code": code, "isDay": day, "humidity": 53.0, "windMph": float(wind)},
           "daily": [{"date": '2026-10-%02d' % (30 + i) if i < 2 else '2026-11-%02d' % (i - 1), "code": 3, "hi": float(hi), "lo": float(lo)} for i, (hi, lo, _) in enumerate(daily)],
           "display": {"place": "Atlanta", "temp": "61\u00b0", "cond": label, "stats": "Feels like 57\u00b0 \u00b7 53%% humidity \u00b7 %d mph" % wind, "nowKind": k, "days": days}}
    return {"doc": doc, "state": "fresh"}


def more(case, now_ms):
    code, day, wind, _ = CASES[case]
    wet = code >= 51
    hours = []
    temps = [61, 60, 59, 57, 56, 55, 54, 54, 53, 53, 53, 54, 56, 58, 60, 62, 64, 65, 66, 66, 65, 63, 61, 59, 57, 55, 54, 53, 52, 52, 53, 55, 57, 59, 60, 61]
    for i in range(36):
        h = (15 + i) % 24
        d = '2026-10-%02d' % (30 + (15 + i) // 24) if (15 + i) // 24 < 2 else '2026-11-01'
        rain = 0
        c = code if i < 2 else (3 if h in (18, 19, 20, 21, 22) else 0 if 7 <= h <= 17 else 2)
        if wet:
            rain = [70, 80, 85, 90, 80, 60, 40, 30, 20, 10, 10][i] if i < 11 else 10
            c = code if i < 6 else 3
        elif case in ('cloudy', 'partly-day', 'partly-night'):
            rain = [10, 10, 20, 40, 70, 80, 60, 30, 10, 0][i] if i < 10 else 0
            c = 3 if i < 4 else (61 if 4 <= i < 7 else 3)
        hours.append({"time": '%sT%02d:00' % (d, h), "temp": float(temps[i]), "rainPct": float(rain), "code": c, "isDay": 7 <= h < 18, "windMph": float(wind)})
    daily = []
    for i in range(10):
        dd = '2026-10-%02d' % (30 + i) if i < 2 else '2026-11-%02d' % (i - 1)
        hi, lo, kk = [(71, 51, 3), (73, 51, 3), (62, 48, 63), (66, 56, 2), (59, 44, 53), (68, 42, 0), (74, 55, 95), (70, 53, 1), (65, 49, 61), (63, 47, 3)][i]
        daily.append({"date": dd, "sunrise": dd + 'T07:41', "sunset": dd + 'T18:25', "uvMax": 4.0, "rainPct": float([20, 70, 40, 10, 40, 60, 30, 20, 50, 20][i]), "rainIn": 0.0, "code": kk, "hi": float(hi), "lo": float(lo)})
    alerts = []
    if case == 'storm':
        alerts = [{"event": "Flood Watch", "severity": "Moderate", "headline": "Flood Watch in effect until Friday morning for north Georgia", "ends": "2026-10-31T08:00:00-04:00"},
                  {"event": "Wind Advisory", "severity": "Minor", "headline": "Wind Advisory from 2 PM to 8 PM", "ends": "2026-10-30T20:00:00-04:00"}]
    return {"doc": {"schema": 1, "fetchedAt": now_ms, "staleAt": now_ms + 900000, "expiresAt": now_ms + 86400000, "zip": "30301", "units": "fahrenheit", "place": "Atlanta", "lat": 33.75, "lon": -84.39,
                    "now": {"uvIndex": 4.0, "gustMph": float(wind) * 2, "windDeg": 225.0, "pressureHpa": 1013.0, "cloudPct": 40.0, "visibilityM": 24000.0, "rainIn": 0.0},
                    "hourly": hours, "daily": daily, "alerts": alerts}, "state": "fresh"}


class Runner:
    def __init__(self, binary):
        H.BIN = binary
        self.t = H.RustHome('runTest')

    def host(self, wait, steps, case, mode='dark', scale=None, layout=None, feeds=None, extra=()):
        now_ms = int(time.time() * 1000)
        fd = {"weather": reading(case, now_ms), "weather.more": more(case, now_ms)}
        fd.update(feeds or {})
        extra = ['--clock', CLOCK] + list(extra)
        return self.t.host(wait, steps, layout=layout, feeds=fd, mode=mode, scale=scale, extra=extra)


def crop(src, box, dst):
    from PIL import Image
    im = Image.open(src).convert('RGB')
    x, y, w, h = box
    im.crop((x, y, x + w, y + h)).save(dst)


def render_card(run, case, mode, scale, tmp):
    """The card's picture twice (a moment apart, so that the motion shows) and where the card is (at a large text size the cards are lower)."""
    names = [os.path.join(tmp, '%s-%s-%s-%d.png' % (case, mode, scale or 1, i)) for i in range(2)]
    box = CARD
    for attempt in range(3):
        steps = [['--snap', '%d:%s' % (ms, p)] for ms, p in zip((6500, 8200), names)] + [['--eval', '8300:home'], ['--eval', '8350:atlas']]
        out, evals, _ = run.host(8800 + 1500 * attempt, steps, case, mode=mode, scale=scale)
        cards = {c[0]: c for c in (evals[0].get('cards') or [])} if evals else {}
        if 'weather' in cards:
            box = tuple(int(v) for v in cards['weather'][1:5])
        if all(os.path.exists(p) for p in names) and has_text(names[1], box, mode):
            break
    return names, box


def jpeg(src, dst):
    """A page of the view as a JPEG: a few hundred KiB each, not two MiB."""
    from PIL import Image
    Image.open(src).convert('RGB').save(dst, quality=88, optimize=True)


def has_text(src, box, mode):
    """Whether the card's picture has its words (a render that was taken before they were drawn has glass only)."""
    from PIL import Image
    im = Image.open(src).convert('L').crop((box[0], box[1], box[0] + 200, box[1] + 230))
    px = list(im.getdata())
    ink = sum(1 for v in px if (v > 235 if mode == 'dark' else v < 45))
    return ink > 300


def keys_to_card():
    """From the status bar down to the cards and along them to the weather card, and OK on it."""
    return H.RustHome.k(1200, 'down', 'right', 'right') + H.RustHome.k(2200, 'ok')


def render_view(run, case, mode, scale, tmp, pages=('now', 'details')):
    """The view's pages, one picture each."""
    out = {}
    names = {p: os.path.join(tmp, 'view-%s-%s-%s-%s.png' % (p, case, mode, scale or 1)) for p in pages}
    steps = [keys_to_card()]
    t = 6500
    for p in ('now', 'radar', 'details'):
        if p in pages:
            steps.append(['--snap', '%d:%s' % (t, names[p])])
        steps.append(H.RustHome.k(t + 300, 'down'))
        t += 3500
    run.host(t + 1500, steps, case, mode=mode, scale=scale)
    return names


class Helper:
    """The real helper (a debug build, on the real network) on a port of its own, for the radar: its weather service draws the map and the frames."""

    def __init__(self, binary, port=18791):
        self.port = port
        self.dir = tempfile.mkdtemp(prefix='wxhelper-')
        with open(os.path.join(self.dir, 'helper.json'), 'w') as f:
            json.dump({'listen': {'host': '127.0.0.1', 'port': port}}, f)
        # a helper on a workstation must not read the owner's receiver on the LAN
        os.makedirs(os.path.join(self.dir, 'settings'))
        with open(os.path.join(self.dir, 'settings', 'base.receiver.json'), 'w') as f:
            json.dump({'owner': 'base.receiver', 'schema_revision': 1, 'version': 1, 'values': {'enabled': False}}, f)
        self.p = subprocess.Popen([binary, 'run', '--data-dir', self.dir, '--stderr'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # up, and with a reading of the weather: a helper that has none when the shell asks publishes an empty one over the shell's, and the radar is
        # opened only for a place (a person opens the view long after the helper has its reading)
        import urllib.request
        for _ in range(90):
            try:
                topics = json.loads(urllib.request.urlopen('http://127.0.0.1:%d/v1/state' % port, timeout=2).read()).get('topics', {})
                if (topics.get('weather') or {}).get('doc'):
                    break
            except Exception:
                pass
            time.sleep(1)

    def start(self):
        return self

    def stop(self):
        pass

    def close(self):
        self.p.terminate()
        try:
            self.p.wait(10)
        except subprocess.TimeoutExpired:
            self.p.kill()
        shutil.rmtree(self.dir, ignore_errors=True)


def render_radar(binary, helper, case, mode, scale, tmp, level='1', at=26000):
    """The radar page: OK on the card, Down to the radar, the frames and the map coming from the helper; one picture when they are there."""
    SH.BIN = binary
    name = os.path.join(tmp, 'radar-%s-%s-%s-%s.png' % (case, mode, scale or 1, level))
    now_ms = int(time.time() * 1000)
    feeds = {"weather": reading(case, now_ms), "weather.more": more(case, now_ms)}
    # the shell's clock is the real one here, as the helper's is
    extra = ['--push', '50:foreground:' + json.dumps(H.HOME)]
    for topic, doc in feeds.items():
        extra += ['--helper', '100:%s:%s' % (topic, json.dumps(doc))]
    for step in (keys_to_card(), H.RustHome.k(5000, 'down')):
        extra += step
    if level != '1':
        extra += H.RustHome.k(at - 4000, level)
    extra += ['--snap', '%d:%s' % (at, name)]
    rc, out, _, _, _ = SH.run(wait=at + 1500, extra=extra, helper=helper, kinds='home', text_scale=scale or 1.0, mode=mode, timeout=300)
    return name


def sheet(paths, cols, cell, label=None):
    from PIL import Image, ImageDraw
    rows = (len(paths) + cols - 1) // cols
    gap = 12
    img = Image.new('RGB', (cols * cell[0] + (cols + 1) * gap, rows * cell[1] + (rows + 1) * gap), (14, 12, 24))
    for i, p in enumerate(paths):
        x, y = gap + (i % cols) * (cell[0] + gap), gap + (i // cols) * (cell[1] + gap)
        img.paste(Image.open(p).convert('RGB'), (x, y))
    return img


def before_after(out, tmp, base='fd0c9687'):
    """One sheet of the old renders (from `base` in git) above the new ones: four cards, the view's first page, its details (the old week page for the old
    view), the radar. Written to `out/weather-redesign-before-after.png`."""
    from PIL import Image, ImageDraw
    rel = 'docs/mocks/host-rust/home/'

    def old(name):
        data = subprocess.run(['git', 'show', '%s:%s%s' % (base, rel, name)], cwd=ROOT, capture_output=True).stdout
        dst = os.path.join(tmp, 'old-' + name)
        with open(dst, 'wb') as f:
            f.write(data)
        return Image.open(dst).convert('RGB')

    def new(name):
        return Image.open(os.path.join(out, name)).convert('RGB')

    W, gap = 2480, 20
    label_h = 56
    rows = []

    def cards(sheet, order, cell, cols=4):
        """The picked cells of a contact sheet of cards in a row."""
        cw, ch = cell
        tiles = []
        for idx in order:
            x, y = (idx % cols) * cw, (idx // cols) * ch
            tiles.append(sheet.crop((x, y, x + cw, y + ch)))
        return tiles

    # the old sheet: 1840 x 1500, four across; the new one: 1792 x 1488 with 12 px gaps (433 x 480 cards)
    old_cards = cards(old('weather-card-dark-text100.png'), [0, 7, 9, 10], (460, 500))
    nsheet = new('weather-card-dark-text100.png')
    new_cards = []
    for idx in [0, 7, 9, 10]:
        x, y = 12 + (idx % 4) * (433 + 12), 12 + (idx // 4) * (480 + 12)
        new_cards.append(nsheet.crop((x, y, x + 433, y + 480)))

    def strip(tiles, label, height):
        img = Image.new('RGB', (W, height + label_h), (14, 12, 24))
        d = ImageDraw.Draw(img)
        d.text((24, 14), label, fill=(230, 226, 245))
        x = gap
        scale = (W - gap * (len(tiles) + 1)) / sum(t.width for t in tiles)
        for t in tiles:
            tw, th = int(t.width * scale), int(t.height * scale)
            img.paste(t.resize((tw, th), Image.LANCZOS), (x, label_h))
            x += tw + gap
        return img

    rows.append(strip(old_cards, 'BEFORE  the card (clear, rain, storm, snow)', 640))
    rows.append(strip(new_cards, 'AFTER   the card', 640))
    pair = lambda a, b, la, lb: strip([a, b], la + '   |   ' + lb, 650)
    rows.append(pair(old('weather-view-now.png'), new('weather-view-now-rain-dark-text100.jpg'), 'BEFORE  the view, first page', 'AFTER   the view, Now (ten days, hours, sky)'))
    rows.append(pair(old('weather-view-details.png'), new('weather-view-details-rain-dark-text100.jpg'), 'BEFORE  Details', 'AFTER   Details'))
    rows.append(pair(old('weather-view-radar-forecast.png'), new('weather-radar-rain-local-dark-text100.jpg'), 'BEFORE  the radar', 'AFTER   the radar'))
    height = sum(r.height for r in rows)
    sheet_img = Image.new('RGB', (W, height), (14, 12, 24))
    y = 0
    for r in rows:
        sheet_img.paste(r, (0, y))
        y += r.height
    dst = os.path.join(out, 'weather-redesign-before-after.png')
    sheet_img.save(dst, optimize=True)
    print('wrote', dst)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--out', default=OUT)
    ap.add_argument('--only', default='card,view,radar')
    ap.add_argument('--bin', default=os.environ.get('LUMATV_BIN', H.BIN))
    ap.add_argument('--jobs', type=int, default=6)
    ap.add_argument('--cases', default=','.join(CASES))
    ap.add_argument('--modes', default='dark,light')
    ap.add_argument('--scales', default='1,2')
    ap.add_argument('--view-cases', default='rain,clear-day,storm')
    ap.add_argument('--radar-cases', default='rain')
    ap.add_argument('--radar-at', type=int, default=26000)
    ap.add_argument('--helper-bin', default='/data/home/lumaglass/rustui/target-helper-pt/debug/lumatv-helper')
    ap.add_argument('--helper-port', type=int, default=18791)
    ap.add_argument('--base', default='fd0c9687')
    args = ap.parse_args()
    run = Runner(args.bin)
    only = set(args.only.split(','))
    os.makedirs(args.out, exist_ok=True)
    tmp = os.path.join('/tmp', 'wxrender-%d' % os.getpid())
    os.makedirs(tmp, exist_ok=True)
    cases = args.cases.split(',')
    if 'card' in only:
        for mode in args.modes.split(','):
            for scale in [float(s) for s in args.scales.split(',')]:
                sc = None if scale == 1 else scale
                with concurrent.futures.ThreadPoolExecutor(args.jobs) as ex:
                    results = list(ex.map(lambda c: render_card(run, c, mode, sc, tmp), cases))
                crops = []
                for c, (paths, box) in zip(cases, results):
                    dst = os.path.join(tmp, 'crop-%s-%s-%s.png' % (c, mode, scale))
                    crop(paths[1], box, dst)
                    crops.append(dst)
                name = 'weather-card-%s-text%d.png' % (mode, int(scale * 100))
                sheet(crops, 4, (CARD[2], CARD[3])).save(os.path.join(args.out, name))
                print('wrote', name)
    if 'view' in only:
        view_cases = [c for c in args.view_cases.split(',') if c]
        for mode in args.modes.split(','):
            for scale in [float(s) for s in args.scales.split(',')]:
                sc = None if scale == 1 else scale
                with concurrent.futures.ThreadPoolExecutor(min(args.jobs, 3)) as ex:
                    results = list(ex.map(lambda c: render_view(run, c, mode, sc, tmp), view_cases))
                for c, names in zip(view_cases, results):
                    for page, src in names.items():
                        if os.path.exists(src):
                            jpeg(src, os.path.join(args.out, 'weather-view-%s-%s-%s-text%d.jpg' % (page, c, mode, int(scale * 100))))
                            print('wrote', page, c, mode, scale)
    if 'radar' in only:
        helper = Helper(args.helper_bin, args.helper_port)
        try:
            for mode in args.modes.split(','):
                for scale in [float(s) for s in args.scales.split(',')]:
                    sc = None if scale == 1 else scale
                    for case in args.radar_cases.split(','):
                        for level in ('1', '3') if (mode == 'dark' and scale == 1) else ('1',):
                            src = render_radar(args.bin, helper, case, mode, sc, tmp, level=level, at=args.radar_at)
                            if os.path.exists(src):
                                lvl = {'1': 'local', '2': 'region', '3': 'globe'}[level]
                                jpeg(src, os.path.join(args.out, 'weather-radar-%s-%s-%s-text%d.jpg' % (case, lvl, mode, int(scale * 100))))
                                print('wrote radar', case, lvl, mode, scale)
        finally:
            helper.close()
    if 'sheet' in only:
        before_after(args.out, tmp, args.base)
    shutil.rmtree(tmp, ignore_errors=True)


if __name__ == '__main__':
    main()
