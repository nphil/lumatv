#!/usr/bin/env python3
"""A virtual remote for the gate's Home checks, run ON THE SET as /tmp/lumatv/uinput-key.py (Linux uinput, python3).

    uinput-key.py hold SECONDS [--settle S] [--key CODE] [--device PATH]

Creates a keyboard device, waits S seconds (default 1) so that anything that scans the event nodes has opened it, presses the
key (default KEY_RF_HOME, 773, LG's code for Home: PlainHome reads it as the Magic Remote's Home and perf/kb.py injects it; the routing
takes 172 and 102 as Home too, --key 172), holds it SECONDS, releases it and destroys the device. Exit 0
when every step went through. The press is a real evdev press on a node of its own: the compositor opens the nodes once, at
its start, so a device made later is seen only by a reader that rescans (the supervisor's Home reader does, every 6 s).

Only what a gate check asks is done: one device, one key, no network, nothing written outside /dev/uinput."""
import fcntl
import os
import struct
import sys
import time

UI_SET_EVBIT = 0x40045564
UI_SET_KEYBIT = 0x40045565
UI_DEV_CREATE = 0x5501
UI_DEV_DESTROY = 0x5502
EV_SYN = 0
EV_KEY = 1
KEY_RF_HOME = 773
BUS_USB = 3
NAME = b"lumatv-gate-remote"


def user_dev():
    """struct uinput_user_dev: name[80], input_id {bustype, vendor, product, version}, ff_effects_max, and four
    arrays of 64 s32 for the absolute axes (none here): 1116 bytes."""
    return struct.pack("80sHHHHi", NAME, BUS_USB, 0x4C54, 0x0001, 1, 0) + b"\0" * (4 * 64 * 4)


def event(etype, code, value, now=None):
    """struct input_event in the layout of the process that writes it (the kernel converts a 32-bit process's)."""
    now = time.time() if now is None else now
    return struct.pack("llHHi", int(now), int((now % 1) * 1000000), etype, code, value)


def press(fd, key, value):
    os.write(fd, event(EV_KEY, key, value))
    os.write(fd, event(EV_SYN, 0, 0))


def hold(seconds, settle, key, device, sleep=time.sleep):
    fd = os.open(device, os.O_WRONLY | os.O_NONBLOCK)
    try:
        fcntl.ioctl(fd, UI_SET_EVBIT, EV_KEY)
        fcntl.ioctl(fd, UI_SET_KEYBIT, key)
        os.write(fd, user_dev())
        fcntl.ioctl(fd, UI_DEV_CREATE)
        try:
            print("device made; pressing key %d in %.1f s for %.1f s" % (key, settle, seconds), flush=True)
            sleep(settle)
            press(fd, key, 1)
            sleep(seconds)
            press(fd, key, 0)
            sleep(0.2)
        finally:
            fcntl.ioctl(fd, UI_DEV_DESTROY)
    finally:
        os.close(fd)
    print("released and destroyed", flush=True)


def main(argv):
    if len(argv) < 3 or argv[1] != "hold":
        print(__doc__)
        return 2
    try:
        seconds = float(argv[2])
        settle, key, device = 1.0, KEY_RF_HOME, "/dev/uinput"
        rest = argv[3:]
        while rest:
            flag, value = rest[0], rest[1]
            rest = rest[2:]
            if flag == "--settle":
                settle = float(value)
            elif flag == "--key":
                key = int(value)
            elif flag == "--device":
                device = value
            else:
                raise ValueError(flag)
    except (ValueError, IndexError):
        print(__doc__)
        return 2
    try:
        hold(seconds, settle, key, device)
    except OSError as e:
        print("uinput: %s" % e, file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
