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
  !NAME     a luna call from LUNA below, started at this instant without waiting for its answer
            (a surface that no key opens: a toast, an alert)
KEY is UP DOWN LEFT RIGHT OK BACK HOME MENU or an evdev code.
"""
import os
import struct
import sys
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


fd = os.open(device(), os.O_WRONLY)


def event(kind, code, value):
    # 32-bit userland on a 64-bit kernel: the compat input_event has two 4-byte time fields
    os.write(fd, struct.pack('llHHi', 0, 0, kind, code, value))


def key(name, value):
    event(1, CODES.get(name) or int(name), value)
    event(0, 0, 0)


def mark(name):
    with open(MARKER, 'w') as f:
        f.write('P:' + name + '\n')


LUNA = {
    # The fields LG's view reads of every notification (`userData.id` of a toast, `readable` and `showTimeout` of an
    # alert): without them its own code logs a TypeError or a binding warning per notification, which the gate counts.
    'toast': ('luna://com.webos.notification/createToast',
              '{"sourceId":"com.webos.app.home","message":"A toast for the gate","readable":"A toast for the gate","userData":{"id":"gate"}}'),
    'alert': ('luna://com.webos.notification/createAlert',
              '{"sourceId":"com.webos.app.home","title":"An alert for the gate","message":"Closed by the gate with OK.","readable":"An alert for the gate",'
              '"showTimeout":false,"buttons":[{"label":"OK"}]}'),
}


def luna(name):
    import subprocess
    uri, payload = LUNA[name]
    subprocess.Popen(['luna-send', '-n', '1', uri, payload], stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
                     stderr=subprocess.DEVNULL, close_fds=True)


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


for token in sys.argv[1:]:
    run(token)
