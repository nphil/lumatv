#!/usr/bin/env python3
"""What each frame of the frame gate's surfaces asks of the set's GPU, measured on the host harness (`lumatv-ui --host`): Home with its
feeds, the gate's walks (`docs/research/perf/gate.py`), the engine drawing its pages as the compositor does (two stacks, Home from its
floor, a page copying from the last what it owes from the frames before), one refresh a frame (`--virtual`, so a slow host does not fold
refreshes into one frame), wallpaper motion and the weather card's sprites off as the gate has them.

A frame's GPU time on the set is about 1.5 ms and 0.085 ms for each layer its passes draw (round 23, 604 frames of an idle GPU, R2 0.47):
the 8.3 ms of a 120 Hz refresh is about 80 layers. For each window of a walk this prints the frames, the most layers a frame drew, that
frame's estimate, and the work of a frame (that estimate and the time this process took to apply what the frame showed) at its worst and
at the 99th percentile.

    python3 tests/host_budget.py [--refresh 120] [--surfaces quick,picker,settings-scroll,home-widgets,toast,onhome,guide,banner,sky] [--frames] [--reference]
                                 [--bin PATH]

`--frames` lists every frame; `--reference` draws each frame whole as well and counts the frames that differ (the floor and the copies
must change nothing); `--bin` (or `LUMATV_UI_BIN`) names another `lumatv-ui`. `onhome` is the Smart home card: entered, a switch and a lamp
toggled, a long scroll, a sensor's history, a lamp's colours changed, and a rest in which no frame may be drawn. `guide` is the channel
guide over a make-believe set of channels: walked by time and by channel with a held arrow, paged, a programme's sheet opened, the filters
chosen, a rest, and left. `banner` is Live TV's channel banner: Live TV comes to the front and the tuner names its channel, then another after the
banner's five seconds, then INFO (the tallest it gets). `sky` is Home at rest for ten seconds with the weather card's sky moving, which the gate has off:
the frames a second the sky asks for (each is a flip of the whole screen on the set) and what each one draws.

The Cameras app is measured by itself (no Home under it) in four walks: `cameras-grid` (eight tiles whose stills come five a second a tile, three of
them not 16:9, then the focus walked) and `cameras-full` (a portrait doorbell's video in its window with the bars beside it, the chrome fading and
coming back), each with the fill of the bars as shipped and as `cameras-grid-nofill` and `cameras-full-nofill` (`theme.cameras.fillBars` off). The
difference between a walk and its `-nofill` is what the fill costs."""
import argparse
import colorsys
import http.server
import json
import math
import os
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import threading
import time
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import rust_home_host as home  # noqa: E402  (Home's make-believe feeds)

BIN = os.environ.get('LUMATV_UI_BIN') or os.path.join(ROOT, 'rustui', 'target', 'debug', 'lumatv-ui')
FRAME = re.compile(r"^\[frame\] (\d+) at (\d+) ms: damage (\d+) px, drew (\d+) px in (\d+) rects \([\d.]+ %\), layers (\d+) culled \d+ cpu (\d+) us gpu \d+ us"
                   r".*? floor (\d+) px apply (\d+) us", re.M)
SURFACES = ['quick', 'picker', 'settings-scroll', 'home-widgets', 'toast', 'onhome', 'guide', 'banner', 'camera-popup', 'weather-zoom', 'sky']
#: when a walk starts: Home has drawn its feeds and rests
START_MS = 6000
#: the layers a 120 Hz refresh holds (1.5 ms + 0.085 ms a layer <= 8.3 ms)
LAYERS_120 = 80
#: the Cameras walks, the fill on and off
CAMERAS = ['cameras-grid', 'cameras-grid-nofill', 'cameras-full', 'cameras-full-nofill']


def gpu_ms(layers):
    return 1.5 + 0.085 * layers


def work_ms(frame):
    """What a frame asks: its GPU estimate and the time this process took to apply what it showed."""
    return gpu_ms(frame[3]) + frame[6] / 1000.0


def p99(values):
    values = sorted(values)
    return values[min(len(values) - 1, math.ceil(0.99 * len(values)) - 1)] if values else 0.0


def write_logos(folder, n=12):
    """Twelve invented channel logos for the guide's fixture (356 by 200 PNGs: a rounded plate in a colour of its own with a lighter band)."""
    os.makedirs(folder, exist_ok=True)
    w, h, r = 356, 200, 40
    for i in range(n):
        red, green, blue = (int(255 * c) for c in colorsys.hsv_to_rgb(i / n, 0.55, 0.95))
        raw = bytearray()
        for y in range(h):
            raw.append(0)
            for x in range(w):
                cx, cy = min(max(x, r), w - 1 - r), min(max(y, r), h - 1 - r)
                inside = (x - cx) ** 2 + (y - cy) ** 2 <= r * r
                band = 80 < y < 120 and 50 < x < w - 50
                raw += bytes((255, 255, 255, 235) if inside and band else (red, green, blue, 255) if inside else (0, 0, 0, 0))
        chunk = lambda kind, data: struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))  # noqa: E731
        png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes(raw), 6)) + chunk(b'IEND', b'')
        with open(os.path.join(folder, 'logo%02d.png' % i), 'wb') as f:
            f.write(png)
    return folder


#: the Smart home card tall (6 of 12 columns, 9 of 16 rows), as the walk finds it beside the weather and the dock
ONHOME_LAYOUT = {"grid": {"rows": 16}, "widgets": [
    {"id": "smart", "type": "onhome", "col": 0, "row": 0, "span": 6, "rows": 9, "title": "Smart home"},
    {"id": "weather", "type": "weather", "col": 6, "row": 0, "span": 6, "rows": 9},
    {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]}
HELPER_PORT = 18947


def onhome_doc():
    """The helper's `onhome` topic: a lamp with colours and effects, lamps, switches, sensors, a thermostat and a scene, and enough
    more to scroll a long way."""
    def thing(id, name, group, icon, words, tone, can=(), verbs=(), ctl=None):
        return {"id": id, "name": name, "area": "Home", "group": group, "words": words, "tone": tone, "icon": icon, "available": True,
                "can": list(can), "verbs": list(verbs), "ctl": ctl or {}}

    def lamp(id, name, on, rgb=None, level=70, effects=()):
        ctl = {"on": on, "dimmable": True, "brightness": level if on else None, "colour": rgb is not None, "rgb": rgb if on else None, "kelvin": None,
               "effects": list(effects), "effect": effects[0] if effects else None}
        return thing('light.' + id, name, 'lights', 'lightbulb', ('On \u00b7 %d%%' % level) if on else 'Off', 'on' if on else 'off', ['toggle'], ['turn_on', 'turn_off', 'toggle'], ctl)

    def switch(id, name, on):
        return thing('switch.' + id, name, 'switches', 'plug', 'On' if on else 'Off', 'on' if on else 'off', ['toggle'], ['toggle'], {"on": on})

    def number(id, name, value, unit, cls, icon):
        return thing('sensor.' + id, name, 'sensors', icon, '%g%s' % (value, unit), 'info', ctl={"kind": "number", "value": value, "unit": unit, "class": cls})

    tiles = [lamp('kitchen', 'Kitchen', True, [255, 176, 64], 80, ['None', 'Rainbow', 'Candle', 'Pulse']), lamp('strip', 'TV strip', True, [64, 140, 255], 55), lamp('desk', 'Desk lamp', False),
             switch('fan', 'Fan', True), switch('heater', 'Heater', False), number('hall', 'Hall', 21.4, '\u00b0C', 'temperature', 'thermometer'),
             number('hum', 'Humidity', 48, '%', 'humidity', 'droplets'),
             thing('binary_sensor.door', 'Front door', 'sensors', 'door-closed', 'Closed', 'off', ctl={"kind": "binary", "on": False, "class": "door", "yes": "Open", "no": "Closed"}),
             thing('climate.t', 'Thermostat', 'climate', 'thermometer', '68\u00b0 \u00b7 Heating to 70\u00b0', 'warn', ['sheet'], ['set_temperature', 'set_hvac_mode'],
                   {"mode": "heat", "modes": ["off", "heat", "cool"], "current": 68.0, "target": 70.0, "min": 50.0, "max": 90.0, "step": 1.0, "unit": "\u00b0F", "action": "heating"}),
             thing('scene.movie', 'Movie night', 'scenes', 'sparkles', 'Scene', 'muted', ['run'], ['turn_on'])]
    tiles += [switch('s%d' % n, 'Socket %d' % n, n % 2 == 0) for n in range(1, 9)]
    tiles += [number('t%d' % n, 'Room %d' % n, 19 + n / 2, '\u00b0C', 'temperature', 'thermometer') for n in range(1, 9)]
    return {"state": "connected", "words": "%d things" % len(tiles), "tiles": tiles}


class FakeHelper(http.server.BaseHTTPRequestHandler):
    """The two answers the card asks the helper for in the walk: a sensor's history (a day or the range asked, made up here) and
    {"ok": true} for what a lamp or a switch is told."""

    def log_message(self, *args):
        pass

    def reply(self, doc):
        body = json.dumps(doc).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        # the feeds the harness starts with --port: a state with no topics, and a long poll that has nothing to say
        if self.path.startswith('/v1/state'):
            return self.reply({"ok": True, "boot": 1, "seq": 0, "topics": {}})
        if self.path.startswith('/v1/events'):
            time.sleep(1.0)
            return self.reply({"ok": True, "boot": 1, "seq": 0, "events": []})
        query = dict(part.split('=', 1) for part in self.path.partition('?')[2].split('&') if '=' in part)
        rng = query.get('range', '24h')
        span = {'1h': 3600, '6h': 6 * 3600, '24h': 86400, '7d': 7 * 86400, '30d': 30 * 86400}.get(rng, 86400) * 1000
        now = int(time.time() * 1000)
        base = {"id": query.get('id', ''), "range": rng, "from": now - span, "to": now, "source": "history"}
        points = [[now - span + i * span // 120, round(20.5 + 2.2 * math.sin(i / 20.0) + 0.25 * math.sin(i * 1.7), 2)] for i in range(121)]
        values = [p[1] for p in points]
        low, high = min(points, key=lambda p: p[1]), max(points, key=lambda p: p[1])
        self.reply({"history": {**base, "kind": "number", "unit": "\u00b0C", "class": "temperature", "points": points,
                                "stats": {"min": low[1], "max": high[1], "avg": round(sum(values) / len(values), 2), "min_at": low[0], "max_at": high[0], "now": 21.4, "now_at": now}}})

    def do_POST(self):
        self.rfile.read(int(self.headers.get('Content-Length') or 0))
        self.reply({"ok": True})



def png(w, h, pixel_row):
    """A PNG of `w` by `h` whose row `y` is `pixel_row(y)` (RGBA bytes, `w` pixels)."""
    raw = b''.join(b'\x00' + pixel_row(y) for y in range(h))
    chunk = lambda kind, data: struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))  # noqa: E731
    return b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 6, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(raw, 1)) + chunk(b'IEND', b'')


class FakeWx(FakeHelper):
    """The helper's weather service for the weather view's map (`/v1/wx/...`): a map, a loop of frames and a globe, made up here, at the sizes the
    helper draws them (1920 by 1080 map, 2048 px frames, a 760 px globe), so that the map's layers weigh what they do on the set."""

    LAT, LON = 33.75, -84.39
    maps = {}
    frames = {}
    zoom = 6

    @classmethod
    def prepare(cls):
        cls.maps['map'] = png(1920, 1080, lambda y: bytes((200 - y % 90, 214 - y % 60, 226 - y % 40, 255)) * 1920)
        cls.maps['globe'] = png(760, 760, lambda y: bytes((30, 70 + y % 80, 140, 255)) * 760)
        cls.maps['frame'] = png(2048, 2048, lambda y: bytes((60, 170, 190, 150 if 600 < y < 1400 else 0)) * 2048)

    def pic(self, kind):
        body = self.maps[kind]
        self.send_response(200)
        self.send_header('Content-Type', 'image/png')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(body)

    def state(self):
        # the frames' times are fixed for a quarter of an hour, as the helper's are: a clock that moved with every ask would make every frame another one
        now = int(time.time()) // 900 * 900
        frames = [[now - 7200 + i * 900, 0 if i < 8 else 1, True, 0] for i in range(17)]
        side = 2048 / (256 * 2 ** (self.zoom + 3))
        wx, wy = (self.LON + 180) / 360, 0.5 - math.log(math.tan(math.pi / 4 + math.radians(self.LAT) / 2)) / (2 * math.pi)
        return {"status": "ready", "have": len(frames), "want": len(frames), "frames": frames, "framePx": 2048, "attribution": "fake", "now": now, "newest": now - 300,
                "extent": {"x0": wx - side / 2, "y0": wy - side / 2, "x1": wx + side / 2, "y1": wy + side / 2}}

    def do_GET(self):
        path = self.path.partition('?')[0]
        if path == '/v1/wx/state':
            return self.reply(self.state())
        if path == '/v1/wx/map':
            return self.pic('map')
        if path == '/v1/wx/frame':
            return self.pic('frame')
        if path == '/v1/wx/globe':
            return self.pic('globe')
        return super().do_GET()

    def do_POST(self):
        body = self.rfile.read(int(self.headers.get('Content-Length') or 0))
        if self.path == '/v1/wx/open':
            type(self).zoom = int((json.loads(body or b'{}')).get('zoom') or 6)
            return self.reply(self.state())
        self.reply({"ok": True})


class Walk:
    """A walk's keys and pushes on the harness's time, and the windows its frames are read in."""

    def __init__(self, at):
        self.at = at
        self.args = []
        self.marks = []

    def mark(self, label):
        self.marks.append((self.at, label))

    def key(self, name, how='', at=None):
        self.args += ['--key', '%d:%s%s' % (self.at if at is None else at, name, ':' + how if how else '')]

    def press(self, name):
        """A key and its release, as OK needs them."""
        self.key(name)
        self.key(name, 'up', at=self.at + 40)

    def taps(self, name, n, gap):
        for _ in range(n):
            self.key(name)
            self.at += gap

    def hold(self, name, ms):
        """A held key as the remote sends it: the first repeat after 300 ms, then one every 100 ms."""
        self.key(name)
        for t in range(self.at + 300, self.at + ms, 100):
            self.key(name, 'repeat', at=t)
        self.key(name, 'up', at=self.at + ms)
        self.at += ms

    def push(self, topic, doc):
        self.args += ['--push', '%d:%s:%s' % (self.at, topic, json.dumps(doc))]

    def wait(self, ms):
        self.at += ms


LIVE_TV = {"app": "com.webos.app.livetv", "home": False, "covered": True}
#: the tags of the channel banner's three asks (`system/channel_feed.rs`): the tuner's channel, that channel's record, what is on it
BANNER_TAGS = (0x4A00_0010, 0x4A00_0011, 0x4A00_0012)


def banner_answers(w, number, name):
    """What the tuner and LG's channel and programme services answer when channel `number` (`2-1`) is named, on the clock of now."""
    import render_rust_banner as rb
    channel = '1_32_%s_0_0_0' % number.replace('-', '_')
    now = int(time.time() * 1000)
    start = now - now % (30 * 60_000)
    record = {'returnValue': True, 'channelList': [{'channelId': channel, 'channelNumber': number, 'channelName': name, 'channelTypeId': 1}]}
    listing = {'returnValue': True, 'programList': [rb.programme(channel, 'Evening News', start, start + 60 * 60_000, 'Local and national news.', 'TV-PG'),
                                                    rb.programme(channel, 'Jeopardy!', start + 60 * 60_000, start + 90 * 60_000, 'What comes next.', 'TV-PG')]}
    for dt, tag, doc in ((0, BANNER_TAGS[0], {'returnValue': True, 'channelId': channel}), (40, BANNER_TAGS[1], record), (60, BANNER_TAGS[2], listing)):
        w.args += ['--luna', '%d:%d:%s' % (w.at + dt, tag, json.dumps(doc))]


def walk_of(surface):
    """The gate's walk of `surface` (its `open` sequence) from START_MS: a panel opened and closed twice, Settings walked as
    `settings-scroll` walks it, Home's rows as `home-widgets` does."""
    w = Walk(START_MS)
    if surface in ('quick', 'picker'):
        for n in (1, 2):
            w.mark('open %d' % n)
            w.push('show', {'kind': surface, 'n': n})
            w.wait(1500)
            w.mark('close %d' % n)
            w.key('back')
            w.wait(1500)
    elif surface == 'toast':
        for n in (1, 2):
            w.mark('toast %d' % n)
            w.push('show', {'kind': 'toast', 'n': n, 'page': 'Placement check|A toast for the gate'})
            w.wait(7000)
    elif surface == 'weather-zoom':
        # from the status bar down to the weather card and OK on it, Right to the map, the frames loading, then the wheel: in a flick, out a flick to the
        # globe and back, one notch at a time, and a rest
        w.mark('open')
        for name in ('down', 'right', 'right'):
            w.press(name)
            w.wait(200)
        w.press('ok')
        w.wait(1200)
        w.mark('map loading')
        w.press('right')
        w.wait(9000)
        w.mark('zoom in')
        for _ in range(12):
            w.key('wheelup')
            w.wait(45)
        w.wait(1200)
        w.mark('zoom out to the globe')
        for _ in range(40):
            w.key('wheeldown')
            w.wait(45)
        w.wait(1500)
        w.mark('zoom back')
        for _ in range(30):
            w.key('wheelup')
            w.wait(45)
        w.wait(1500)
        w.mark('notches')
        for _ in range(4):
            w.key('wheelup')
            w.wait(500)
        w.wait(1000)
        w.mark('rest')
        w.wait(2000)
        w.mark('end')
    elif surface == 'banner':
        w.mark('launch')
        w.push('foreground', LIVE_TV)
        w.wait(300)
        banner_answers(w, '2-1', 'WSB-HD')
        w.wait(6500)
        w.mark('change')
        banner_answers(w, '5-1', 'WXIA-HD')
        w.wait(6500)
        w.mark('info')
        w.key('info')
        w.wait(9000)
        w.mark('end')
    elif surface == 'settings-scroll':
        # the quick panel's foot (QUICK_PANEL_FOOT), OK on its "All settings", then SETTINGS_PAGE, the rail down, SETTINGS_PAGE, the rail up, Back
        w.at = START_MS - 2200
        w.push('show', {'kind': 'quick', 'n': 1})
        w.wait(1300)
        w.taps('down', 20, 30)
        w.at = START_MS
        w.mark('open')
        w.key('ok')
        w.key('ok', 'up', at=w.at + 50)
        w.wait(600)
        for page in ('picture', 'system'):
            w.mark(page + ' list')
            w.key('right')
            w.wait(100)
            w.hold('down', 800)
            w.wait(150)
            w.hold('up', 800)
            w.wait(100)
            w.key('back')
            w.wait(150)
            if page == 'picture':
                w.mark('rail down x8')
                w.taps('down', 8, 60)
        w.mark('rail up x9')
        w.taps('up', 9, 60)
        w.mark('close')
        w.key('back')
        w.wait(1500)
    elif surface == 'home-widgets':
        # HOME_TOP first, then the walk: the status bar, the cards, the dock and back up
        w.at = START_MS - 1500
        w.taps('up', 8, 100)
        w.taps('left', 6, 60)
        w.at = START_MS
        w.mark('status bar')
        w.taps('right', 3, 120)
        w.taps('left', 3, 120)
        w.mark('into the cards')
        w.taps('down', 2, 150)
        w.mark('across the cards')
        w.taps('right', 2, 150)
        w.mark('to the dock')
        w.taps('down', 1, 150)
        w.mark('along the dock')
        w.hold('left', 900)
        w.wait(200)
        w.hold('right', 900)
        w.wait(200)
        w.mark('down and up the dock')
        w.taps('down', 2, 150)
        w.taps('up', 2, 150)
        w.mark('up to the status bar')
        w.taps('up', 6, 120)
        w.wait(600)
    elif surface == 'onhome':
        # HOME_TOP, then into the Smart home card (the first card): a camera is in front; then Kitchen, TV strip, Desk lamp and Fan make a row of four
        w.at = START_MS - 1500
        w.taps('up', 8, 100)
        w.at = START_MS
        w.mark('enter the card')
        w.taps('down', 1, 500)
        w.press('ok')
        w.wait(1200)
        w.mark('along a row')
        w.taps('down', 1, 400)
        w.taps('right', 3, 300)
        w.wait(600)
        w.mark('toggle a switch')
        w.press('ok')
        w.wait(1500)
        w.press('ok')
        w.wait(1500)
        w.mark('toggle a lamp')
        w.taps('left', 3, 300)
        w.press('ok')
        w.wait(1500)
        w.press('ok')
        w.wait(1500)
        w.mark('long scroll')
        w.hold('down', 2500)
        w.wait(400)
        w.hold('up', 2500)
        w.wait(1500)
        w.mark('sensor history')
        w.taps('down', 2, 400)
        w.taps('right', 1, 400)
        w.press('ok')
        w.wait(2500)
        w.taps('right', 1, 1500)
        w.key('back')
        w.wait(1500)
        w.mark('colour sheet')
        w.taps('up', 1, 400)
        w.taps('left', 1, 400)
        w.hold('ok', 1100)
        w.wait(2200)
        w.hold('right', 1000)
        w.wait(800)
        w.key('back')
        w.wait(2000)
        w.mark('rest')
        w.wait(3000)
    elif surface == 'guide':
        # LG's guide opened (`ctl shell show guide`), its programmes arrive, the grid walked by time and by channel (a held arrow), paged with CH+ and
        # CH-, a programme's sheet opened and left, the filters and Now chosen, and the guide left with Back
        w.mark('open')
        w.push('show', {'kind': 'guide'})
        w.wait(1800)
        w.mark('by time')
        w.hold('right', 2600)
        w.wait(300)
        w.hold('left', 2600)
        w.wait(300)
        w.mark('by channel')
        w.hold('down', 2600)
        w.wait(300)
        w.hold('up', 2600)
        w.wait(300)
        w.mark('paging')
        w.taps('chdown', 6, 280)
        w.taps('chup', 6, 280)
        w.wait(300)
        w.mark('sheet')
        w.key('info')
        w.wait(700)
        w.key('back')
        w.wait(400)
        w.mark('filters')
        w.taps('up', 40, 40)
        w.taps('right', 2, 260)
        w.key('ok')
        w.wait(1200)
        w.taps('right', 1, 260)
        w.key('ok')
        w.wait(900)
        w.mark('rest')
        w.wait(2500)
        w.mark('close')
        w.key('back')
        w.wait(1500)
    elif surface == 'camera-popup':
        # HOME_TOP, the focus Right onto the camera button (Search is the bar's first), OK: the list grows out of the button and its stills
        # come (the make-believe helper of camera_popup_host.py); walked down past its fifth row and up with a held arrow; Back: it goes
        # back into the button; opened and closed again
        w.at = START_MS - 1500
        w.taps('up', 8, 100)
        w.at = START_MS
        w.mark('focus the button')
        w.taps('right', 1, 800)
        w.mark('grow')
        w.press('ok')
        w.wait(1500)
        w.mark('walk down')
        w.taps('down', 7, 250)
        w.wait(600)
        w.mark('held up')
        w.hold('up', 1500)
        w.wait(600)
        w.mark('close')
        w.key('back')
        w.wait(1200)
        w.mark('again')
        w.press('ok')
        w.wait(1200)
        w.key('back')
        w.wait(1200)
    elif surface == 'sky':
        w.mark('sky moving')
        w.wait(10000)
    else:
        raise ValueError('no walk for %s' % surface)
    w.mark('end')
    return w


def cameras_walk(r, cams, kind):
    """The steps of a Cameras walk on `r` (a `rust_cameras_scenes.Run`); returns the windows' starts (ms, label) and the end."""
    if kind == 'grid':
        cams.base(r, cams.cameras_topic(8))
        r.setup(lambda c, s: s.pictures.update({'300': cams.shaped_jpeg('game', 3 / 4), '303': cams.shaped_jpeg('home', 4 / 3), '305': cams.shaped_jpeg('wall', 3 / 4)}))
        cams.open_grid(r, 900)
        for i in range(6):
            r.press(8000 + 350 * i, 'right')
        return [(900, 'open'), (3000, 'stills at rest'), (8000, 'focus walk'), (10300, 'end')]
    r.backdrop = cams.video_backdrop('game', 3 / 4)
    cams.full(r, state='playing', per={0: cams.DOORBELL}, width=1920, height=2560, codec='h264', fps=15)
    r.setup(lambda c, s: s.pictures.update({'300': cams.shaped_jpeg('game', 3 / 4)}))
    r.channel(2400, 'vwin', cams.VWIN_CARRIED)
    r.press(9000, 'down')
    return [(1800, 'open'), (2500, 'video plays'), (4500, 'chrome gone'), (9000, 'a key brings it back'), (10500, 'end')]


def run_cameras(surface, refresh, binary, reference):
    """One Cameras walk (`cameras-grid`, `cameras-full`, with `-nofill`): the same return as `run`."""
    import rust_cameras_scenes as cams
    cams.BIN = binary
    _, kind, *rest = surface.split('-')
    log = tempfile.mktemp(prefix='hostbudget-cameras-', suffix='.log')
    extra = ['--virtual', '--refresh', str(refresh), '--log', log]
    if 'nofill' in rest:
        extra += ['--theme', json.dumps({'cameras': {'fillBars': False}})]
    if reference:
        extra += ['--reference']
    r = cams.Run(extra=extra)
    marks = cameras_walk(r, cams, kind)
    r.wait = marks[-1][0] + 500
    res = cams.run(r)
    if res['rc'] != 0:
        raise RuntimeError('%s: lumatv-ui failed: %s' % (surface, res['out'][-2000:]))
    text = open(log).read() if os.path.exists(log) else ''
    os.path.exists(log) and os.remove(log)
    fr = [(int(m[2]), int(m[4]), int(m[5]), int(m[6]), int(m[8]), int(m[7]), int(m[9])) for m in FRAME.finditer(text)]
    windows = [(label, [f for f in fr if at <= f[0] < nxt]) for (at, label), (nxt, _) in zip(marks, marks[1:])]
    return windows, len(re.findall(r"^\[mismatch\]", text, re.M)), text


def run(surface, refresh=120, binary=BIN, reference=False):
    if surface in CAMERAS:
        return run_cameras(surface, refresh, binary, reference)
    return run_walk(surface, refresh, binary, reference)


def run_walk(surface, refresh=120, binary=BIN, reference=False):
    """Runs the walk of `surface`; returns ([(label, [frame, ...]), ...], mismatched frames, the run's output). A frame is
    (ms, drawn px, rects, layers, floor px, CPU us, of it applying commits us)."""
    data = tempfile.mkdtemp(prefix='hostbudget-')
    os.makedirs(os.path.join(data, 'theme'))
    shutil.copy(os.path.join(ROOT, 'compositor', 'assets', 'ui', 'wallpaper', 'wall_1080.png'), os.path.join(data, 'theme', 'wall_1080.png'))
    with open(os.path.join(ROOT, 'payload', 'theme', 'theme.json')) as f:
        theme = json.load(f)
    # the gate's `still`: the wallpaper's tick and the weather card's sprites off while a surface is measured (the `sky` walk has the sprites on)
    theme.setdefault('wallpaperMotion', {})['enabled'] = False
    theme.setdefault('weather', {})['animated'] = surface == 'sky'
    if surface == 'banner':
        theme.setdefault('banners', {})['own'] = True
    with open(os.path.join(data, 'theme', 'theme.json'), 'w') as f:
        json.dump(theme, f)
    w = walk_of(surface)
    log = os.path.join(data, 'frames.log')
    feeds = {"weather": home.weather(), "news": home.news(), "profiles": home.profiles(), "cameras": home.CAMERAS,
             "plugins": {"plugins": [{"id": "home-assistant", "name": "Home Assistant", "state": "connected"}], "firstParty": []},
             "plugin.home-assistant.cards": home.HOME_CARDS}
    extra, server = [], None
    if surface == 'onhome':
        feeds['onhome'] = onhome_doc()
        with open(os.path.join(data, 'theme', 'layout.json'), 'w') as f:
            json.dump(ONHOME_LAYOUT, f)
        server = http.server.ThreadingHTTPServer(('127.0.0.1', HELPER_PORT), FakeHelper)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        extra = ['--port', str(HELPER_PORT), '--helper-port', str(HELPER_PORT)]
    elif surface == 'weather-zoom':
        weather = home.weather()
        weather['doc'].update({"lat": FakeWx.LAT, "lon": FakeWx.LON, "place": "Atlanta"})
        feeds['weather'] = weather
        FakeWx.prepare()
        server = http.server.ThreadingHTTPServer(('127.0.0.1', HELPER_PORT), FakeWx)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        extra = ['--port', str(HELPER_PORT), '--helper-port', str(HELPER_PORT)]
    elif surface == 'camera-popup':
        import camera_popup_host as cp
        feeds['cameras'] = cp.topic(8, offline=(2,))
        server = cp.serving(first_ms=60, later_ms=10)
        extra = ['--port', str(server.server_address[1]), '--helper-port', str(server.server_address[1])]
    cmd = [binary, '--host', '--virtual', '--refresh', str(refresh), '--kinds', 'osd,toast,home,settings,quick,picker,drawer,menu,confirm' + (',guide' if surface == 'guide' else '') + (',banner' if surface == 'banner' else '') + (',cameras' if surface == 'camera-popup' else ''),
           '--data-dir', data, '--shm', os.path.join(data, 'shm'), '--wait', str(w.at + 500), '--log', log,
           '--module-dir', os.path.join(ROOT, 'compositor', 'assets', 'ui'),
           '--push', '40:refresh:' + json.dumps({"hz": refresh}), '--push', '50:foreground:' + json.dumps(home.HOME),
           '--luna', '80:%d:%s' % (home.LP_TAG, json.dumps({"launchPoints": home.launch_points()}))]
    for topic, doc in feeds.items():
        cmd += ['--helper', '100:%s:%s' % (topic, json.dumps(doc))]
    if surface == 'guide':
        cmd += ['--guide-fixture', write_logos(os.path.join(data, 'logos'))]
    cmd += w.args + extra + (['--reference'] if reference else [])
    try:
        r = subprocess.run(cmd, env=dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1'), capture_output=True, text=True, timeout=1800)
    finally:
        if server:
            server.shutdown()
            server.server_close()
    text = open(log).read() if os.path.exists(log) else ''
    shutil.rmtree(data, ignore_errors=True)
    if r.returncode != 0:
        raise RuntimeError('%s: lumatv-ui failed: %s' % (surface, (r.stdout + r.stderr)[-2000:]))
    fr = [(int(m[2]), int(m[4]), int(m[5]), int(m[6]), int(m[8]), int(m[7]), int(m[9])) for m in FRAME.finditer(text)]
    windows = []
    for (at, label), (nxt, _) in zip(w.marks, w.marks[1:]):
        windows.append((label, [f for f in fr if at <= f[0] < nxt]))
    return windows, len(re.findall(r"^\[mismatch\]", text, re.M)), text


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--refresh', type=int, default=120)
    ap.add_argument('--surfaces', default=','.join(SURFACES))
    ap.add_argument('--frames', action='store_true')
    ap.add_argument('--reference', action='store_true')
    ap.add_argument('--bin', default=BIN)
    a = ap.parse_args(argv)
    budget = LAYERS_120 * 120 // a.refresh
    over = 0
    for surface in a.surfaces.split(','):
        windows, mismatched, _ = run(surface, a.refresh, a.bin, a.reference)
        marks = [] if surface in CAMERAS else walk_of(surface).marks
        spans = {label: (nxt - at) / 1000.0 for (at, label), (nxt, _) in zip(marks, marks[1:])}
        worst = max((f[3] for _, fs in windows for f in fs), default=0)
        print('%s at %d Hz: most layers in a frame %d (%.1f ms)%s' % (surface, a.refresh, worst, gpu_ms(worst),
                                                                    ', %d frames differ from the frame drawn whole' % mismatched if a.reference else ''))
        for label, fs in windows:
            most = max((f[3] for f in fs), default=0)
            heavy = sum(1 for f in fs if f[3] > budget)
            over += heavy
            work = [work_ms(f) for f in fs]
            drawn = sum(1 for f in fs if f[1] > 0)
            rate = ' (%.1f a second)' % (len(fs) / spans[label]) if spans.get(label) else ''
            print('  %-22s %4d frames%s, %4d drew pixels, most layers %3d (%.1f ms), work max %.1f ms p99 %.1f ms, %d over %d' % (label, len(fs), rate, drawn, most, gpu_ms(most), max(work, default=0.0), p99(work), heavy, budget))
            if a.frames:
                for f in fs:
                    print('      %6d ms  drew %8d px in %2d rects  layers %3d  floor %8d px  cpu %6d us (commits %6d us)' % f)
        frames = [f for _, fs in windows for f in fs]
        work = [work_ms(f) for f in frames]
        print('  all %d frames: most layers %d (%.1f ms), work max %.1f ms p99 %.1f ms' % (len(frames), max((f[3] for f in frames), default=0),
                                                                                      gpu_ms(max((f[3] for f in frames), default=0)), max(work, default=0.0), p99(work)))
    return 0


if __name__ == '__main__':
    sys.exit(main())
