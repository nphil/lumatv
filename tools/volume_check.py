#!/usr/bin/env python3
"""The volume bar against the real receiver, on the set: held keys and taps, with everything on one clock.

    python3 volume_check.py [OUT]           on the set (copied to /tmp; Python 3.10); the receiver is the helper's (`base.receiver`)
    python3 volume_check.py --analyse OUT   anywhere, on a run's files

Five runs of Volume- held 1 s and Volume+ held 0.9 s (the remote's repeat: 300 ms, then every 100 ms), each followed by a
change made at the receiver itself (its start level over its web port), then five taps each way 1.5 s apart. It records the keys
(`keys.jsonl`), what the bar shows (`ui.jsonl`: the compositor's `ctl shell` report, read every 10 ms, 4 ms for the taps) and the
helper's `receiver` topic events (`rx.jsonl`), and writes `summary.json`: per hold whether the number moved back after the release,
the number left and the receiver's level then; per change whether the bar took it; per key the time to the bar's new number.

Receiver safety: it starts only with no media pipeline playing, reads the receiver's level and mute first, never lets the level
(read or targeted) past the start + 3 dB or -30 dB (50 on the panel), whichever is lower, and puts the level and mute back at the end,
on an error and on SIGTERM or SIGHUP. Every run nets back to the start; Volume+ is held shorter than Volume-.
"""
import json
import os
import signal
import socket
import struct
import subprocess
import sys
import threading
import time
import urllib.request

KEYS = {'up': 115, 'down': 114}
CTL = '/tmp/xdg/lumatv-control'
HELPER = 'http://127.0.0.1:18731'
PLAN = ['poll:10', 's1'] + ['hold:down:1000', 's2.5', 'hold:up:900', 's2.5', 'ext', 's2.5'] * 5 + \
    ['poll:4', 'taps:down:5', 'taps:up:5', 's2.5']


def receiver_address():
    view = json.load(urllib.request.urlopen(HELPER + '/v1/settings/base.receiver', timeout=5))
    return '%s:%s' % (view['effective']['host']['value'], view['effective']['port']['value'])


def mv(level):
    halves = int(level * 2 + 0.5)
    return 'MV%02d' % (halves // 2) if halves % 2 == 0 else 'MV%02d5' % (halves // 2)


class Run:
    def __init__(self, denon, out):
        self.denon, self.out = denon, out
        os.makedirs(out, exist_ok=True)
        self.stop, self.abort = threading.Event(), threading.Event()
        self.poll, self.held = 0.01, None
        self.files = {name: open(os.path.join(out, name), 'w') for name in ('keys.jsonl', 'ui.jsonl', 'rx.jsonl')}
        self.lock = threading.Lock()

    def put(self, name, obj):
        with self.lock:
            self.files[name].write(json.dumps(obj) + '\n')
            self.files[name].flush()

    def status(self):
        text = urllib.request.urlopen(f'http://{self.denon}/goform/formMainZone_MainZoneXmlStatusLite.xml', timeout=3).read().decode()
        level = float(text.split('<MasterVolume><value>')[1].split('<')[0]) + 80.0
        return level, '<Mute><value>on' in text

    def command(self, command):
        urllib.request.urlopen(f'http://{self.denon}/goform/formiPhoneAppDirect.xml?{command}', timeout=3).read()

    def guard(self, what, value):
        if value is not None and value > self.limit and not self.abort.is_set():
            self.abort.set()
            self.put('keys.jsonl', {'t': time.time(), 'abort': what, 'value': value})

    def ui_poll(self):
        last, prev = None, time.time()
        while not self.stop.is_set():
            t0 = time.time()
            try:
                s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                s.connect(CTL)
                s.sendall(b'shell\n')
                buf = b''
                while not buf.endswith(b'\n'):
                    chunk = s.recv(65536)
                    if not chunk:
                        break
                    buf += chunk
                s.close()
                ui = json.loads(buf).get('ui') or {}
                cur = {k: ui.get(k) for k in ('osd', 'level', 'target', 'source', 'muted')}
                self.guard('target', cur['target'])
                if cur != last:
                    self.put('ui.jsonl', {'t': time.time(), 'prev': prev, **cur})
                    last = cur
                prev = t0
            except (OSError, ValueError) as e:
                self.put('ui.jsonl', {'t': time.time(), 'error': str(e)})
                time.sleep(0.2)
            time.sleep(max(0.0, self.poll - (time.time() - t0)))

    def rx_poll(self):
        seq = json.load(urllib.request.urlopen(HELPER + '/v1/state', timeout=5)).get('seq')
        while not self.stop.is_set():
            try:
                r = json.load(urllib.request.urlopen(f'{HELPER}/v1/events?after={seq}&timeout=1&topics=receiver', timeout=5))
            except (OSError, ValueError) as e:
                self.put('rx.jsonl', {'t': time.time(), 'error': str(e)})
                time.sleep(0.2)
                continue
            seq = r.get('seq', seq)
            for ev in r.get('events', []):
                d = ev.get('data') or {}
                self.guard('receiver', d.get('volumeAbs'))
                self.put('rx.jsonl', {'t': time.time(), 'abs': d.get('volumeAbs'), 'mute': d.get('mute'), 'at': d.get('updatedAt')})

    def key(self, code, value):
        # 32-bit userland on a 64-bit kernel: the compat input_event has two 4-byte time fields
        os.write(self.fd, struct.pack('llHHi', 0, 0, 1, code, value) + struct.pack('llHHi', 0, 0, 0, 0, 0))
        self.held = code if value else None
        self.put('keys.jsonl', {'t': time.time(), 'code': code, 'value': value})

    def nap(self, seconds):
        end = time.time() + seconds
        while time.time() < end and not self.abort.is_set():
            time.sleep(0.005)

    def hold(self, code, ms):
        start = time.time()
        self.key(code, 1)
        repeat, end = start + 0.3, start + ms / 1000.0
        while not self.abort.is_set() and time.time() < end:
            if time.time() >= repeat:
                self.key(code, 2)
                repeat += 0.1
            time.sleep(0.002)
        self.key(code, 0)

    def restore(self):
        # the helper may still be correcting toward the bar's last level: put the start back until it stays
        for attempt in range(4):
            self.command(mv(self.start))
            time.sleep(0.1)
            self.command('MUON' if self.mute else 'MUOFF')
            time.sleep(1.8)
            level, mute = self.status()
            self.put('keys.jsonl', {'t': time.time(), 'restored': mv(self.start), 'read': level, 'mute': mute, 'attempt': attempt})
            if abs(level - self.start) < 0.25 and mute == self.mute:
                return True
        return False

    def run(self, plan):
        pipelines = subprocess.run(['luna-send', '-n', '1', 'luna://com.webos.media/getActivePipelines', '{}'],
                                   capture_output=True, text=True, timeout=10).stdout
        if '"type":"media"' in pipelines.replace(' ', ''):
            raise SystemExit('media is playing: not run')
        self.start, self.mute = self.status()
        self.limit = min(self.start + 3.0, 50.0)
        self.put('keys.jsonl', {'t': time.time(), 'start': self.start, 'mute': self.mute, 'limit': self.limit})
        self.fd = os.open(device(), os.O_WRONLY)
        for target in (self.ui_poll, self.rx_poll):
            threading.Thread(target=target, daemon=True).start()
        try:
            time.sleep(1.0)
            for tok in plan:
                if self.abort.is_set():
                    break
                self.put('keys.jsonl', {'t': time.time(), 'mark': tok})
                word, *args = tok.split(':')
                if word == 'hold':
                    self.hold(KEYS[args[0]], int(args[1]))
                elif word == 'taps':
                    for _ in range(int(args[1])):
                        self.key(KEYS[args[0]], 1)
                        time.sleep(0.05)
                        self.key(KEYS[args[0]], 0)
                        self.nap(1.5)
                elif word == 'poll':
                    self.poll = int(args[0]) / 1000.0
                elif word == 'ext':
                    self.command(mv(self.start))
                    self.put('keys.jsonl', {'t': time.time(), 'ext': self.start})
                elif word.startswith('s'):
                    self.nap(float(word[1:]))
        finally:
            if self.held is not None:
                self.key(self.held, 0)
            try:
                restored = self.restore()
            finally:
                self.stop.set()
                time.sleep(1.2)
        return restored


def device():
    for block in open('/proc/bus/input/devices').read().split('\n\n'):
        if 'Name="Smart Remote RCU Input"' in block:
            return '/dev/input/' + next(w for w in block.split() if w.startswith('event'))
    raise SystemExit('no SSAP key device')


def analyse(out):
    def lines(name):
        return [json.loads(line) for line in open(os.path.join(out, name)) if line.strip()]
    keys, ui, rx = lines('keys.jsonl'), lines('ui.jsonl'), lines('rx.jsonl')
    start = next(k['start'] for k in keys if 'start' in k)
    kev = [k for k in keys if 'code' in k]
    exts = [k for k in keys if 'ext' in k]
    shown = [u for u in ui if u.get('level') not in (None, '')]
    reports = [r for r in rx if r.get('abs') is not None]
    ends = sorted([k['t'] for k in kev if k['value'] == 1] + [e['t'] for e in exts] + [max(k['t'] for k in keys)])

    def level_at(t):
        before = [u for u in shown if u['t'] <= t]
        return float(before[-1]['level']) if before else None
    holds, taps, changes = [], [], []
    for i, press in enumerate(kev):
        if press['value'] != 1:
            continue
        sign = 1 if press['code'] == KEYS['up'] else -1
        release = next(k for k in kev[i + 1:] if k['value'] == 0)
        repeats = sum(1 for k in kev[i + 1:] if k['value'] == 2 and k['t'] < release['t'])
        end = min(t for t in ends if t > press['t'])
        was = level_at(press['t'])
        first = next((u for u in shown if u['t'] >= press['t'] and float(u['level']) != was), None)
        after = [level_at(release['t'])] + [float(u['level']) for u in shown if release['t'] < u['t'] < end]
        last = [u for u in shown if u['t'] < end][-1]
        heard = [r for r in reports if r['t'] < end]
        one = {
            'dir': 'up' if sign > 0 else 'down', 'repeats': repeats, 'after_release': after,
            'back': any((b - a) * sign < 0 for a, b in zip(after, after[1:])),
            'shown': float(last['level']), 'target': last.get('target'), 'receiver': heard[-1]['abs'] if heard else None,
            'key_to_bar_ms': first and [round((first.get('prev', first['t']) - press['t']) * 1000, 1), round((first['t'] - press['t']) * 1000, 1)],
        }
        one['pass'] = not one['back'] and one['shown'] == one['receiver'] and one['target'] is None
        (holds if repeats else taps).append(one)
    for e in exts:
        end = min(t for t in ends if t > e['t'])
        was = level_at(e['t'])
        if was == start:
            changes.append({'from': was, 'unchanged': True})
            continue
        took = next((u for u in shown if e['t'] < u['t'] < end and float(u['level']) == start and u.get('target') is None), None)
        changes.append({'from': was, 'to': start, 'shown_after_ms': took and round((took['t'] - e['t']) * 1000)})
    latency = sorted(x['key_to_bar_ms'][1] for x in holds + taps if x['key_to_bar_ms'])
    summary = {
        'start': start,
        'holds_pass': sum(h['pass'] for h in holds), 'holds': len(holds),
        'changes_shown': sum(c.get('shown_after_ms') is not None for c in changes), 'changes': sum('unchanged' not in c for c in changes),
        'key_to_bar_ms': latency and {'median': latency[len(latency) // 2], 'max': latency[-1], 'n': len(latency)},
        'restored': [k for k in keys if 'restored' in k][-1:],
        'aborted': [k for k in keys if 'abort' in k],
        'detail': {'holds': holds, 'taps': taps, 'changes': changes},
    }
    json.dump(summary, open(os.path.join(out, 'summary.json'), 'w'), indent=1)
    return summary


def main():
    if sys.argv[1:2] == ['--analyse']:
        summary = analyse(sys.argv[2])
    else:
        out = sys.argv[1] if len(sys.argv) > 1 else '/tmp/volume-check'
        run = Run(receiver_address(), out)

        def leave(*_):
            raise SystemExit('signal')
        signal.signal(signal.SIGTERM, leave)
        signal.signal(signal.SIGHUP, leave)
        run.run(PLAN)
        summary = analyse(out)
    print(json.dumps({k: v for k, v in summary.items() if k != 'detail'}))


if __name__ == '__main__':
    main()
