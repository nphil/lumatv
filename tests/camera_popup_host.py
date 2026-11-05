#!/usr/bin/env python3
"""The camera list that grows out of Home's camera button, on the host harness (`lumatv-ui --host`: the shell's logic against the engine in its own
process on Mesa), against a make-believe helper that answers the stills as the TV's helper does (a 24-bit BMP at the size asked, at most eight
cameras' streams at once, the first picture of a stream slower than the rest).

    python3 tests/camera_popup_host.py renders [--out DIR] [--modes dark,light] [--scales 1,2] [--cameras 8] [--bin PATH]
    python3 tests/camera_popup_host.py first-picture [--cameras 8] [--first-ms 150] [--later-ms 20] [--bin PATH]

`renders` writes `camlist-<mode>-<text size>-<closed|growing|open|scrolled>.png` (the focus on the camera button; the list a moment after OK, on its
way out of the button; at rest; walked down six rows) and prints the layout and contrast lint of the open and the scrolled list.
`first-picture` opens the list twice, straight after the focus reached the button and after it rested there, and prints each row's first fresh picture
(milliseconds from the opening), the rate each kind of row was asked at, and what was asked after the list closed.
"""
import argparse
import http.server
import io
import json
import os
import sys
import tempfile
import threading
import time
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, '..'))
sys.path.insert(0, HERE)
import rust_cameras_scenes as cams  # noqa: E402  (the cameras topic's make-believe cameras and the mocks' pictures)
import rust_home_host as H  # noqa: E402

OUT = '/data/home/tmp/camera-popup'
CLOCK = '2026-11-04T20:05'
#: what the host's Home is made to hold still for: no wallpaper motion, no weather sprites
STILL = {"wallpaperMotion": {"enabled": False}, "weather": {"animated": False}}
KINDS = 'osd,toast,home,settings,quick,picker,drawer,menu,confirm,cameras'
PORT = 18951
#: the helper's bridge serves at most eight cameras at once and lets a stream go three seconds after the last ask (`scrypted/stills.rs`)
MAX_STREAMS = 8
IDLE_S = 3.0


def bmp(i, w, h):
    """Camera `i`'s still at `w` by `h`: a part of one of the mocks' pictures, a different one for each camera, as a 24-bit BMP (the decoder's answer)."""
    from PIL import Image
    im = Image.open(os.path.join(cams.IMG, cams.PICS[i % len(cams.PICS)] + '.jpg')).convert('RGB')
    sw, sh = im.size
    cw = int(sw * (0.55 + 0.4 * ((i * 7) % 10) / 10))
    ch = int(cw * 9 / 16)
    x, y = int((sw - cw) * ((i * 37) % 100) / 100), int(max(0, sh - ch) * ((i * 53) % 100) / 100)
    out = io.BytesIO()
    im.crop((x, y, x + cw, min(sh, y + ch))).resize((w, h), Image.LANCZOS).save(out, 'BMP')
    return out.getvalue()


class Stills(http.server.ThreadingHTTPServer):
    """The helper's `GET /v1/cameras/{id}/frame?w=&h=&fps=`, the requests it was asked, and the bridge's eight streams."""

    daemon_threads = True

    def __init__(self, port, first_ms=0, later_ms=0):
        super().__init__(('127.0.0.1', port), Handler)
        self.first_ms, self.later_ms = first_ms, later_ms
        self.log = []       # (time, camera, fps, status)
        self.active = {}    # camera -> when it was last asked: the streams that are open
        self.lock = threading.Lock()
        self.cache = {}

    def picture(self, camera, w, h):
        key = (camera, w, h)
        if key not in self.cache:
            self.cache[key] = bmp(int(camera) - 300, w, h)
        return self.cache[key]


class Handler(http.server.BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def reply(self, doc, status=200):
        body = json.dumps(doc).encode()
        self.send_response(status)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Connection', 'close')
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self):
        u = urllib.parse.urlparse(self.path)
        q = urllib.parse.parse_qs(u.query)
        srv = self.server
        if u.path.startswith('/v1/cameras/') and u.path.endswith('/frame'):
            camera = u.path[len('/v1/cameras/'):-len('/frame')]
            w, h, fps = (int(q.get(k, ['0'])[0]) for k in ('w', 'h', 'fps'))
            now = time.monotonic()
            with srv.lock:
                srv.active = {c: t for c, t in srv.active.items() if now - t < IDLE_S}
                first = camera not in srv.active
                busy = first and len(srv.active) >= MAX_STREAMS
                if not busy:
                    srv.active[camera] = now
                srv.log.append((now, camera, fps, 503 if busy else 200))
            if busy:
                return self.reply({"ok": False, "api": 1, "error": {"code": "busy", "message": "eight cameras are being served"}}, 503)
            time.sleep((srv.first_ms if first else srv.later_ms) / 1000)
            body = srv.picture(camera, w or 160, h or 90)
            self.send_response(200)
            self.send_header('Content-Type', 'image/bmp')
            self.send_header('Content-Length', str(len(body)))
            self.send_header('Connection', 'close')
            self.end_headers()
            self.wfile.write(body)
            return
        if u.path.startswith('/v1/state'):
            return self.reply({"ok": True, "boot": 1, "seq": 0, "topics": {}})
        if u.path.startswith('/v1/events'):
            time.sleep(1.0)
            return self.reply({"ok": True, "boot": 1, "seq": 0, "events": []})
        self.reply({"ok": False}, 404)


def topic(n, offline=()):
    return cams.cameras_topic(n, per={i: {'online': False} for i in offline})


def host(binary, mode, scale, wait, steps, stills, n, offline=(), extra=()):
    """One run of the harness with Home in front, `n` cameras and the helper of `stills`; returns the output and the evals."""
    H.BIN = binary
    out, evals, _ = H.RustHome().host(wait, steps, feeds={"cameras": topic(n, offline)}, mode=mode, scale=scale, theme=dict(STILL, material=mode), kinds=KINDS,
                                      extra=['--clock', CLOCK, '--port', str(stills.server_address[1]), '--helper-port', str(stills.server_address[1]), *extra])
    return out, evals


def serving(first_ms=0, later_ms=0):
    stills = Stills(PORT, first_ms, later_ms)
    threading.Thread(target=stills.serve_forever, daemon=True).start()
    return stills


def mean_of(path):
    from PIL import Image, ImageStat
    return sum(ImageStat.Stat(Image.open(path).convert('L')).mean) / 1


def renders(binary, out_dir, modes, scales, n):
    os.makedirs(out_dir, exist_ok=True)
    stills = serving(first_ms=60, later_ms=10)
    problems = []
    try:
        for mode in modes:
            for scale in scales:
                tag = '%s-%d' % (mode, round(scale * 100) // 100)
                shots = {s: os.path.join(out_dir, 'camlist-%s-%s.png' % (tag, s)) for s in ('closed', 'growing', 'open', 'scrolled')}
                for attempt in range(3):
                    steps = [H.RustHome.k(7000, 'right'), ['--snap', '8200:' + shots['closed']],
                             ['--key', '8600:ok', '--key', '8640:ok:up'], ['--snap', '8780:' + shots['growing']], ['--snap', '10200:' + shots['open']],
                             ['--eval', '10300:lint'], ['--eval', '10300:geometry'],
                             H.RustHome.k(10500, 'down', 'down', 'down', 'down', 'down', 'down'), ['--snap', '13200:' + shots['scrolled']],
                             ['--eval', '13300:lint'], ['--eval', '13300:geometry']]
                    _, evals = host(binary, mode, scale, 14000, steps, stills, n, offline=(2,))
                    # a picture taken before Home had drawn (a host under load) is black: the run is made again
                    if all(os.path.exists(p) and mean_of(p) > 5 for p in shots.values()):
                        break
                lints = [e for e in evals if isinstance(e, list)]
                geos = [e for e in evals if isinstance(e, dict) and 'cameraList' in e]
                for label, findings in zip(('open', 'scrolled'), lints):
                    # Home's words under the sheet are not hidden for the lint: only the list's own words are read here
                    findings = [f for f in findings if f['subject'].startswith('camera')]
                    if findings:
                        problems.append('%s %s: %s' % (tag, label, findings))
                for label, geo in zip(('open', 'scrolled'), geos):
                    c = geo['cameraList']
                    print('%s %-8s sheet %s window %s offset %s made %s of %s rows, layers %s' % (tag, label, [round(c['sheet'][k]) for k in 'xywh'], [round(c['window'][k]) for k in 'xywh'],
                                                                                        c['offset'], len(c['rows']), c['entries'], c['layers']))
                print('  wrote %s' % ', '.join(os.path.basename(p) for p in shots.values()))
    finally:
        stills.shutdown()
    for p in problems:
        print('LINT', p)
    return 1 if problems else 0


def first_picture(binary, n, first_ms, later_ms):
    stills = serving(first_ms, later_ms)
    try:
        for label, rest in (('opened straight after the focus reached the button', 0), ('opened after the focus rested there for 1.2 s', 1200)):
            stills.log.clear()
            stills.active.clear()
            focus, press = 7000, 7000 + 40 + rest + 100
            close = press + 4000
            steps = [H.RustHome.k(focus, 'right'), ['--key', '%d:ok' % press, '--key', '%d:ok:up' % (press + 40)],
                     ['--eval', '%d:state' % (press + 3000)], ['--key', '%d:back' % close, '--key', '%d:back:up' % (close + 40)], ['--eval', '%d:state' % (close + 3500)]]
            _, evals = host(binary, 'dark', 1, close + 4000, steps, stills, n)
            states = [e for e in evals if isinstance(e, dict) and 'panels' in e]
            if len(states) < 2:
                print('%s: no state printed' % label)
                continue
            listing = states[0]['panels']['cameraList']
            firsts = listing['firstMs']
            log = list(stills.log)
            opened = min((t for t, _, fps, _ in log if fps == 8), default=None)
            print('%s:' % label)
            print('  first fresh picture of each row, ms from the opening: %s' % ', '.join('row %d %d' % (int(c) - 300, ms) for c, ms in sorted(firsts.items(), key=lambda kv: int(kv[0]))))
            if firsts:
                print('  slowest %d ms, mean %.0f ms' % (max(firsts.values()), sum(firsts.values()) / len(firsts)))
            if opened is not None:
                span = (opened + 0.5, opened + 2.5)
                for role, fps in (('the focused row', 8), ('rows in view', 4)):
                    rows = {c for t, c, f, _ in log if f == fps and span[0] <= t <= span[1]}
                    asks = sum(1 for t, c, f, _ in log if f == fps and span[0] <= t <= span[1])
                    print('  %s: %d row(s), %.1f asks a second a row' % (role, len(rows), asks / 2.0 / max(1, len(rows))))
                near = [c for t, c, f, _ in log if f == 0 and span[0] <= t <= span[1]]
                print('  rows beside the view: %.1f asks a second in all' % (len(near) / 2.0))
                after = [t for t, _, _, _ in log if t > opened + 4.0 + 0.4]
                print('  asks after Back: %d (busy refusals in all: %d, streams at most %d)' % (len(after), sum(1 for e in log if e[3] == 503), MAX_STREAMS))
            print('  open in the helper at the end: %d, held by the list: %s' % (len(stills.active), listing['asking']))
    finally:
        stills.shutdown()
    return 0


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    sub = ap.add_subparsers(dest='what', required=True)
    r = sub.add_parser('renders')
    r.add_argument('--out', default=OUT)
    r.add_argument('--modes', default='dark,light')
    r.add_argument('--scales', default='1,2')
    r.add_argument('--cameras', type=int, default=8)
    r.add_argument('--bin', default=H.BIN)
    f = sub.add_parser('first-picture')
    f.add_argument('--cameras', type=int, default=8)
    f.add_argument('--first-ms', type=int, default=150)
    f.add_argument('--later-ms', type=int, default=20)
    f.add_argument('--bin', default=H.BIN)
    a = ap.parse_args(argv)
    if a.what == 'renders':
        return renders(a.bin, a.out, a.modes.split(','), [float(s) for s in a.scales.split(',')], a.cameras)
    return first_picture(a.bin, a.cameras, a.first_ms, a.later_ms)


if __name__ == '__main__':
    sys.exit(main())
