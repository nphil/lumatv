#!/usr/bin/env python3
"""On the set: inject remote keys with exact timing, and write trace markers at the instant.

    python3 /tmp/kb.py TOKEN...

Events go to the SSAP key device ("Smart Remote RCU Input"), which the compositor treats as
any remote. Home Assistant's webostv.button goes through the same device but adds 100-300 ms of
jitter per key, so timing-sensitive workloads inject here instead.

Tokens:
  KEY       press, 50 ms, release
  KEY+      press only                KEY-      release only
  KEY:MS    press, hold MS ms (auto-repeat every 100 ms after the first 300 ms, as the remote
            does), release
  KEYxN@G   N taps, G seconds apart (default 0.35)
  sS        sleep S seconds
  @NAME     trace marker "P:NAME" (a phase boundary for analyse.py) written at this instant
  !NAME     start a control command or luna call for a surface no key opens; once it returns,
            write the marker "Q:NAME-sent" (kb.py waits up to 10 s at the end). The time from the
            "P:" marker includes the command and the service; analyse.py ignores "Q:" markers.
KEY is UP DOWN LEFT RIGHT OK BACK HOME MENU or an evdev code.

    python3 /tmp/kb.py --learn SECONDS
            prints the evdev code and the device of every key pressed on the set's input devices in that time, so the code of a
            key that has none above (the remote's Input key) is read off while someone presses it; it is what gate.py's
            --input-key takes. Nothing is injected.

    python3 /tmp/kb.py --device
            prints the device the keys are injected into (the gate asks the compositor to have it open). Nothing is injected.
"""
import os
import struct
import sys
import threading
import time

CODES = {'UP': 103, 'DOWN': 108, 'LEFT': 105, 'RIGHT': 106, 'OK': 28, 'BACK': 412, 'HOME': 773, 'MENU': 139}
MARKER = '/sys/kernel/debug/tracing/trace_marker'


def device():
    for block in open('/proc/bus/input/devices').read().split('\n\n'):
        if 'Name="Smart Remote RCU Input"' in block:
            for word in block.split():
                if word.startswith('event'):
                    return '/dev/input/' + word
    raise SystemExit('no SSAP key device')


def learn(seconds):
    import glob
    import select
    names = {}
    for block in open('/proc/bus/input/devices').read().split('\n\n'):
        name = next((l.split('"')[1] for l in block.splitlines() if l.startswith('N: Name=')), '')
        for word in block.split():
            if word.startswith('event'):
                names['/dev/input/' + word] = name
    fds = {}
    for path in glob.glob('/dev/input/event*'):
        try:
            fds[os.open(path, os.O_RDONLY | os.O_NONBLOCK)] = names.get(path, path)
        except OSError:
            pass
    size = struct.calcsize('llHHi')           # 32-bit userland on a 64-bit kernel: see event()
    end = time.time() + seconds
    while time.time() < end:
        ready, _, _ = select.select(list(fds), [], [], 0.2)
        for fd in ready:
            data = os.read(fd, size * 64)
            for i in range(0, len(data) - size + 1, size):
                _, _, kind, code, value = struct.unpack('llHHi', data[i:i + size])
                if kind == 1 and value == 1:
                    print(f'{code} {fds[fd]}', flush=True)


if sys.argv[1:2] == ['--learn']:
    learn(float(sys.argv[2]) if len(sys.argv) > 2 else 15.0)
    sys.exit(0)

if sys.argv[1:2] == ['--device']:
    print(device())
    sys.exit(0)


fd = os.open(device(), os.O_WRONLY)


def event(kind, code, value):
    # 32-bit userland on a 64-bit kernel: the compat input_event has two 4-byte time fields
    os.write(fd, struct.pack('llHHi', 0, 0, kind, code, value))


def key(name, value):
    event(1, CODES.get(name) or int(name), value)
    event(0, 0, 0)


def mark_text(text):
    with open(MARKER, 'w') as f:
        f.write(text + '\n')


def mark(name):
    mark_text('P:' + name)


LUNA = {
    # The fields LG's view reads of every notification (`readable` and `showTimeout` of an alert): without them its own code logs a
    # binding warning per notification, which the gate counts.
    'alert': ('luna://com.webos.notification/createAlert',
              '{"sourceId":"com.webos.app.home","title":"An alert for the gate","message":"Closed by the gate with OK.","readable":"An alert for the gate",'
              '"showTimeout":false,"buttons":[{"label":"OK"}]}'),
}

# What the compositor's control socket raises (`lumatv-compositor ctl shell show KIND TEXT`, the gate's tools deployed under LUMATV_D):
# LumaTV's toast by the shell's own feed path. LG's notification service is not asked (a createToast stalled the set for 20 minutes in
# round 26). In TEXT an underscore is a space and a bar a line break. {N} is made different for every call: a toast that repeats the
# text of one still up replaces it in place, and nothing then changes on the screen for the gate to time.
CTL = {
    'toast': ['shell', 'show', 'toast', 'A_toast_for_the_gate_{N}|Placement_check'],
}



def luna(name):
    """A `!name` token: a control command or luna call; mark when it has returned."""
    import subprocess
    n = str(int(time.time() * 1000) % 1000000)
    if name in CTL:
        d = os.environ.get('LUMATV_D', '/tmp/lumatv')
        argv = [d + '/lib/ld-linux-aarch64.so.1', '--library-path', d + '/lib', d + '/bin/lumatv-compositor', 'ctl', '--runtime-dir', '/tmp/xdg']
        argv += [word.replace('{N}', n) for word in CTL[name]]
    else:
        uri, payload = LUNA[name]
        argv = ['luna-send', '-n', '1', uri, payload.replace('{N}', n)]
    child = subprocess.Popen(argv, stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, close_fds=True)

    def sent():
        child.wait()
        mark_text('Q:%s-sent' % name)
    waiter = threading.Thread(target=sent)
    waiter.start()
    WAITERS.append(waiter)


def run(token):
    if token[0] == '@':
        mark(token[1:])
    elif token[0] == '!':
        luna(token[1:])
    elif token[0] == 's' and token[1:].replace('.', '').isdigit():
        time.sleep(float(token[1:]))
    elif token.endswith('+'):
        key(token[:-1], 1)
    elif token.endswith('-'):
        key(token[:-1], 0)
    elif 'x' in token:
        name, rest = token.split('x', 1)
        count, _, gap = rest.partition('@')
        for _ in range(int(count)):
            key(name, 1)
            time.sleep(0.05)
            key(name, 0)
            time.sleep(float(gap or 0.35))
    elif ':' in token:
        name, ms = token.split(':')
        hold = int(ms) / 1000.0
        start = time.time()
        key(name, 1)
        while time.time() - start < hold:
            time.sleep(0.1)
            elapsed = time.time() - start
            if 0.3 < elapsed < hold:
                key(name, 2)
        key(name, 0)
    else:
        key(token, 1)
        time.sleep(0.05)
        key(token, 0)


WAITERS = []
for token in sys.argv[1:]:
    run(token)
for waiter in WAITERS:
    waiter.join(10)
