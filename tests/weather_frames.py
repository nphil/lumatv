#!/usr/bin/env python3
"""What the weather card's sky asks of each frame, measured on the host harness (`lumatv-ui --host --virtual`, one refresh a frame): the layers a
frame drew, the pixels, the engine's CPU time, and the frame rate the screen runs at while the sky moves and while it rests. A frame's GPU time on
the set is about 1.5 ms + 0.085 ms a layer (`docs/research/frames-120-result.md`), so the estimate printed is that against the refresh's deadline.

    python3 tests/weather_frames.py --bin PATH [--cases clear-day,rain,snow,storm] [--refresh 120] [--seconds 10] [--clock 2026-10-30T15:00]
                                    [--window FROM_MS:TO_MS] [--rest] [--layout one|shipped]

Each case runs once with the wallpaper's motion off (the gate's `still`, so that the card is what is measured) and the card's sky on; `--rest` also runs
it with the sky at rest (theme `weather.animated` false), which is what Home costs when nothing moves. `--window` measures only the frames of that
stretch of the run's clock (a storm moment, say). The harness's CPU numbers are the host's, on Mesa: they rank runs, they are not the set's.
"""
import argparse
import json
import os
import re
import shutil
import statistics
import struct
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
import render_rust_weather as R  # noqa: E402
import rust_home_host as H  # noqa: E402

FRAME = re.compile(r"^\[frame\] (\d+) at (\d+) ms: damage (\d+) px, drew (\d+) px in (\d+) rects \([\d.]+ %\), layers (\d+) culled (\d+) cpu (\d+) us gpu (\d+) us", re.M)
#: the weather card alone with the dock, as the default size (3 columns, 9 of 16 rows)
ONE = {"grid": {"rows": 16}, "widgets": [{"id": "weather", "type": "weather", "col": 9, "row": 0, "span": 3, "rows": 9},
                                          {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]}
#: the card sky is made once the atlas has settled; the run is at rest by then
START_MS = 4500


# ---- when the weather's moments are: `weather_art::Schedule`, one in every two minutes, 1 to 3 minutes apart, from a hash of the slot, so that a run can be
# put where one is. Kind: (seed, length ms).
EVERY, SPREAD = 120_000, 60_000
MOMENTS = {'storm': (0x57, 4_800), 'snow': (0x5A, 8_000)}


def f32(x):
    return struct.unpack('f', struct.pack('f', x))[0]


def h16(n):
    """`weather_sprites::hash`."""
    x = ((n * 0x9E3779B1) & 0xFFFFFFFF) ^ 0x85EBCA6B
    x ^= x >> 15
    x = (x * 0x2C1B3C6D) & 0xFFFFFFFF
    x ^= x >> 12
    return f32((x & 0xFFFF) / 65535.0)


def moment_start(kind, slot):
    """When (epoch ms) the moment of slot `slot` starts (`Schedule::start_of`)."""
    seed, _ = MOMENTS[kind]
    return slot * EVERY + int(f32(h16((seed + slot * 7919) & 0xFFFFFFFF) * f32(SPREAD)))


def epoch_ms(clock):
    return int(time.mktime(time.strptime(clock, '%Y-%m-%dT%H:%M:%S')) * 1000)


def moment_clock(kind, lead_ms=6000, after='2026-10-30T15:00:00', nth=0):
    """(a `--clock` text, when in the run's time the moment starts): the first moment of `kind` after `after` that starts `lead_ms` or more into the run."""
    e0 = epoch_ms(after) + lead_ms
    slot = -(-e0 // EVERY)
    found = [moment_start(kind, s) for s in range(slot - 1, slot + 3 + nth)]
    start = [t for t in found if t >= e0][nth]
    clock = (start - lead_ms) // 1000 * 1000
    return time.strftime('%Y-%m-%dT%H:%M:%S', time.localtime(clock / 1000)), start - clock


def gpu_ms(layers):
    return 1.5 + 0.085 * layers


def pct(values, p):
    s = sorted(values)
    return s[min(len(s) - 1, int(round(p / 100.0 * (len(s) - 1))))] if s else 0


def run_case(binary, case, refresh, seconds, clock, animated, layout, mode='dark', scale=None, window=None):
    """The frames of one run: [(ms, drew px, rects, layers, culled, cpu us)] inside the window (default: from START_MS for `seconds`)."""
    data = tempfile.mkdtemp(prefix='wxframes-')
    os.makedirs(os.path.join(data, 'theme'))
    shutil.copy(os.path.join(ROOT, 'payload', 'wallpaper', 'wall_1080.png'), os.path.join(data, 'theme', 'wall_1080.png'))
    with open(os.path.join(ROOT, 'payload', 'theme', 'theme.json')) as f:
        theme = json.load(f)
    theme.setdefault('wallpaperMotion', {})['enabled'] = False
    theme.setdefault('weather', {})['animated'] = animated
    theme['material'] = mode
    with open(os.path.join(data, 'theme', 'theme.json'), 'w') as f:
        json.dump(theme, f)
    if layout is not None:
        with open(os.path.join(data, 'theme', 'layout.json'), 'w') as f:
            json.dump(layout, f)
    start, end = window if window else (START_MS, START_MS + int(seconds * 1000))
    log = os.path.join(data, 'frames.log')
    now_ms = 1_800_000_000_000
    feeds = {"weather": R.reading(case, now_ms), "weather.more": R.more(case, now_ms), "news": H.news(), "profiles": H.profiles(), "cameras": H.CAMERAS,
             "plugins": {"plugins": [{"id": "home-assistant", "name": "Home Assistant", "state": "connected"}], "firstParty": []}, "plugin.home-assistant.cards": H.HOME_CARDS}
    cmd = [binary, '--host', '--virtual', '--refresh', str(refresh), '--kinds', 'home', '--data-dir', data, '--shm', os.path.join(data, 'shm'), '--wait', str(end + 300), '--log', log,
           '--module-dir', os.path.join(ROOT, 'compositor', 'assets', 'ui'), '--clock', clock, '--mode', mode, '--theme', json.dumps({"material": mode}),
           '--push', '40:refresh:' + json.dumps({"hz": refresh}), '--push', '50:foreground:' + json.dumps(H.HOME),
           '--luna', '80:%d:%s' % (H.LP_TAG, json.dumps({"launchPoints": H.launch_points()}))]
    if scale:
        cmd += ['--text-scale', str(scale)]
    for topic, doc in feeds.items():
        cmd += ['--helper', '100:%s:%s' % (topic, json.dumps(doc))]
    r = subprocess.run(cmd, env=dict(os.environ, LIBGL_ALWAYS_SOFTWARE='1'), capture_output=True, text=True, timeout=1800)
    text = open(log).read() if os.path.exists(log) else ''
    shutil.rmtree(data, ignore_errors=True)
    if r.returncode != 0:
        raise RuntimeError('lumatv-ui failed: %s' % (r.stdout + r.stderr)[-1500:])
    frames = [(int(m[2]), int(m[4]), int(m[5]), int(m[6]), int(m[7]), int(m[8])) for m in FRAME.finditer(text)]
    return [f for f in frames if start <= f[0] < end], (end - start) / 1000.0


def report(label, frames, secs, refresh):
    if not frames:
        print('%-28s no frames' % label)
        return
    layers = [f[3] for f in frames]
    px = [f[1] for f in frames]
    rects = [f[2] for f in frames]
    cpu = [f[5] for f in frames]
    deadline = 1000.0 / refresh
    print('%-26s %5.1f fps  layers p50 %3d p99 %3d max %3d (est. %.1f/%.1f/%.1f ms of %.1f)  rects p50 %2d max %2d  drew px p50 %7d p99 %7d max %7d  cpu us p50 %5d p99 %5d max %5d'
          % (label, len(frames) / secs, pct(layers, 50), pct(layers, 99), max(layers), gpu_ms(pct(layers, 50)), gpu_ms(pct(layers, 99)), gpu_ms(max(layers)), deadline,
             pct(rects, 50), max(rects), pct(px, 50), pct(px, 99), max(px), pct(cpu, 50), pct(cpu, 99), max(cpu)))


def main():
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0])
    ap.add_argument('--bin', required=True)
    ap.add_argument('--cases', default='clear-day,rain,snow,storm')
    ap.add_argument('--refresh', type=int, default=120)
    ap.add_argument('--seconds', type=float, default=10)
    ap.add_argument('--clock', default=R.CLOCK)
    ap.add_argument('--rest', action='store_true')
    ap.add_argument('--layout', default='one')
    ap.add_argument('--mode', default='dark')
    ap.add_argument('--scale', type=float, default=None)
    ap.add_argument('--window', default=None, help='FROM_MS:TO_MS of the run (virtual time)')
    a = ap.parse_args()
    layout = ONE if a.layout == 'one' else None
    window = tuple(int(v) for v in a.window.split(':')) if a.window else None
    for case in a.cases.split(','):
        frames, secs = run_case(a.bin, case, a.refresh, a.seconds, a.clock, True, layout, a.mode, a.scale, window)
        report('%s sky moving' % case, frames, secs, a.refresh)
        if a.rest:
            frames, secs = run_case(a.bin, case, a.refresh, a.seconds, a.clock, False, layout, a.mode, a.scale, window)
            report('%s sky at rest' % case, frames, secs, a.refresh)


if __name__ == '__main__':
    main()
