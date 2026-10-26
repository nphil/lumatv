#!/usr/bin/env python3
"""On the set: press and release one evdev key on the SSAP key device, printing the monotonic time of the press (the clock of
/var/log/messages' bracketed stamp).  python3 /tmp/r8key.py CODE [HOLD_MS]"""
import os, struct, sys, time

CODES = {'UP': 103, 'DOWN': 108, 'LEFT': 105, 'RIGHT': 106, 'OK': 28, 'BACK': 412, 'HOME': 773, 'MENU': 139,
         'VOLUP': 115, 'VOLDOWN': 114, 'MUTE': 113}


def device():
    for block in open('/proc/bus/input/devices').read().split('\n\n'):
        if 'Name="Smart Remote RCU Input"' in block:
            for word in block.split():
                if word.startswith('event'):
                    return '/dev/input/' + word
    raise SystemExit('no SSAP key device')


fd = os.open(device(), os.O_WRONLY)
code = CODES.get(sys.argv[1]) or int(sys.argv[1])
hold = int(sys.argv[2]) / 1000.0 if len(sys.argv) > 2 else 0.05


def ev(kind, c, v):
    os.write(fd, struct.pack('llHHi', 0, 0, kind, c, v))


t = time.monotonic()
ev(1, code, 1); ev(0, 0, 0)
if hold > 0.3:
    # the remote's repeat cadence: first repeat after 300 ms, then every 100 ms
    end = t + hold
    time.sleep(0.3)
    while time.monotonic() < end:
        ev(1, code, 2); ev(0, 0, 0)
        time.sleep(0.1)
else:
    time.sleep(hold)
ev(1, code, 0); ev(0, 0, 0)
print('press %.6f release %.6f' % (t, time.monotonic()))
