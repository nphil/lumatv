#!/usr/bin/python3
"""padguard: keep a silent USB game controller invisible to apps, a live one usable.

Netflix opens every gamepad evdev node and reads the initial axis values. A
DualShock-4 clone (054c:09cc) that is off, or wired only for charging, sends no
reports, so its axes sit at 0 and read as a stick held full up-left. The
pad is therefore attached (usbhid + hid-sony bound, evdev nodes present) only while
it is sending reports, and detached while it is silent.

A pad that is off is not silent on the wire: its 2.4 GHz receiver stays enumerated and
streams ~500 all-zero reports per second with report ID 0x00 (bytes 10-11 a running counter).
hid-sony ignores them, so the nodes exist but never receive an event and the axes read 0. A live
pad sends report ID 0x01 (~1000/s). "Pad on" therefore means reports whose first byte is 0x01.

Attached -> detached: once a second the pad's hidraw node is read for up to HIDRAW_PROBE seconds
(a separate reader never steals evdev events). Without CONFIRM_REPORTS live reports for
SILENT_TICKS samples in a row, usbhid is unbound from the interface and every input node vanishes.

Detached -> attached: with usbhid unbound nothing polls the endpoint, so this process claims the
interface through usbfs and, twice a second, polls the interrupt-IN endpoint for a few ms (a pad
that is off yields a handful of ID-0 reports and no verdict). CONFIRM_REPORTS live reports mean the
pad is on: the interface is released and usbhid is bound again (hid-sony probes, nodes reappear).
Polling only during those windows keeps the detached state near zero CPU; a URB left in flight
would wake this process for every one of the receiver's ~500 reports/s.

Remove /var/lib/webosbrew/init.d/50-padguard to uninstall (reboot, or kill this
process and rebind usbhid to the interface).
"""
import ctypes
import errno
import fcntl
import os
import select
import sys
import time

IDS = (("054c", "09cc"),)
TICK = 0.5            # seconds between samples, attached or detached
GRACE = 1.0           # after our own bind, hid-sony is still probing: no verdict yet
SILENT_TICKS = 2      # consecutive samples without live reports before detaching
HIDRAW_PROBE = 0.2    # seconds one attached-state sample waits for live reports
LIVE_ID = 0x01        # DS4 USB input report ID; a pad that is off sends ID 0x00 instead
CONFIRM_REPORTS = 10  # live reports that prove a pad is on
PEEK_WINDOW = 0.03    # detached sample: how long to look for a first live report (~15 reports from a pad that is off)
CONFIRM_WINDOW = 1.0  # ... and how long to wait for CONFIRM_REPORTS once one has appeared
LOG_MAX = 65536


# ---------------------------------------------------------------------------
# usbfs interrupt-IN prober (the only part that needs a real USB device)

_IOC_READ, _IOC_WRITE = 2, 1


def _ioc(d, nr, size):
    return (d << 30) | (size << 16) | (ord("U") << 8) | nr


class _Urb(ctypes.Structure):
    _fields_ = [("type", ctypes.c_ubyte), ("endpoint", ctypes.c_ubyte),
                ("status", ctypes.c_int), ("flags", ctypes.c_uint),
                ("buffer", ctypes.c_void_p), ("buffer_length", ctypes.c_int),
                ("actual_length", ctypes.c_int), ("start_frame", ctypes.c_int),
                ("number_of_packets", ctypes.c_int), ("error_count", ctypes.c_int),
                ("signr", ctypes.c_uint), ("usercontext", ctypes.c_void_p)]


_SUBMITURB = _ioc(_IOC_READ, 10, ctypes.sizeof(_Urb))
_REAPURBNDELAY = _ioc(_IOC_WRITE, 13, ctypes.sizeof(ctypes.c_void_p))
_CLAIMINTERFACE = _ioc(_IOC_READ, 15, 4)
_DISCARDURB = _ioc(0, 11, 0)
_URB_TYPE_INTERRUPT = 1
_libc = None


def _ioctl(fd, req, arg):
    global _libc
    if _libc is None:
        _libc = ctypes.CDLL(None, use_errno=True)
    if _libc.ioctl(fd, ctypes.c_ulong(req), arg) < 0:
        e = ctypes.get_errno()
        raise OSError(e, os.strerror(e))


class UsbfsProbe:
    """Claims one interface and samples its interrupt-IN endpoint on demand.

    The endpoint is only polled during sample(): a pad that is off still streams ~500 reports/s, and
    keeping a URB in flight all the time would wake this process for every one of them.
    """

    def __init__(self, path, ifnum, ep, size):
        self.dead = False
        self.pending = False
        self.fd = os.open(path, os.O_RDWR | os.O_NONBLOCK)
        try:
            _ioctl(self.fd, _CLAIMINTERFACE, ctypes.byref(ctypes.c_uint(ifnum)))
        except Exception:
            os.close(self.fd)
            raise
        self.buf = ctypes.create_string_buffer(size)
        self.urb = _Urb(type=_URB_TYPE_INTERRUPT, endpoint=ep,
                        buffer=ctypes.addressof(self.buf), buffer_length=size)
        self.poller = select.poll()
        self.poller.register(self.fd, select.POLLOUT | select.POLLERR | select.POLLHUP)

    def _reap(self):
        """The completed URB's status, or None if none is ready."""
        try:
            _ioctl(self.fd, _REAPURBNDELAY, ctypes.byref(ctypes.c_void_p()))
        except OSError as e:
            if e.errno != errno.EAGAIN:
                self.dead = True
            return None
        self.pending = False
        return self.urb.status

    def sample(self, peek=PEEK_WINDOW, window=CONFIRM_WINDOW):
        """Poll the endpoint; return the number of live reports seen.

        Looks for `peek` seconds, or `window` once a live report has appeared, and stops at
        CONFIRM_REPORTS live ones. A pad that is off sends only report ID 0, so the common
        outcome is a few reaped URBs and 0.
        """
        live = 0
        start = time.monotonic()
        end = start + peek
        try:
            while not self.dead and live < CONFIRM_REPORTS:
                left = end - time.monotonic()
                if left <= 0:
                    break
                if not self.pending:
                    _ioctl(self.fd, _SUBMITURB, ctypes.byref(self.urb))
                    self.pending = True
                if not self.poller.poll(left * 1000):
                    break
                status = self._reap()
                if status is None:
                    continue
                if status != 0:
                    self.dead = True
                elif self.urb.actual_length > 0 and self.buf[0] == bytes([LIVE_ID]):
                    live += 1
                    end = start + window
        except OSError:
            self.dead = True
        self._park()
        return live

    def _park(self):
        """Stop polling: cancel the in-flight URB and wait for its completion."""
        if self.pending and not self.dead:
            try:
                _ioctl(self.fd, _DISCARDURB, ctypes.byref(self.urb))
                self.poller.poll(100)
                self._reap()
            except OSError:
                self.dead = True
        self.pending = False

    def close(self):
        # Closing the fd discards any URB and releases the interface.
        if self.fd >= 0:
            os.close(self.fd)
            self.fd = -1


def read_hidraw(node, secs):
    """Count live reports on a hidraw node for up to `secs`, stopping at CONFIRM_REPORTS."""
    try:
        fd = os.open(node, os.O_RDONLY | os.O_NONBLOCK)
    except OSError:
        return 0
    n = 0
    end = time.monotonic() + secs
    try:
        while n < CONFIRM_REPORTS:
            left = end - time.monotonic()
            if left <= 0:
                break
            if select.select([fd], [], [], left)[0]:
                try:
                    if os.read(fd, 128)[:1] == bytes([LIVE_ID]):
                        n += 1
                except OSError:
                    break
    finally:
        os.close(fd)
    return n


# ---------------------------------------------------------------------------
# state machine

def _read(path):
    try:
        with open(path) as f:
            return f.read().strip()
    except OSError:
        return None


def _write(path, text):
    try:
        with open(path, "w") as f:
            f.write(text)
        return True
    except OSError:
        return False


class Pad:
    def __init__(self, name, iface, ifnum, ep, size, dev_node):
        self.name, self.iface, self.ifnum, self.ep, self.size = name, iface, ifnum, ep, size
        self.dev_node = dev_node
        self.idir = None         # sysfs directory of the interface
        self.state = None        # "attached" | "detached"
        self.probe = None
        self.until = 0.0         # attached: end of grace
        self.silent = 0
        self.retry_at = 0.0


class Guard:
    def __init__(self, sysfs="/sys", devfs="/dev", clock=time.monotonic,
                 probe_factory=UsbfsProbe, hidraw=read_hidraw, log=None):
        self.sys, self.dev, self.clock = sysfs, devfs, clock
        self.probe_factory, self.hidraw = probe_factory, hidraw
        self.log = log or (lambda msg: None)
        self.pads = {}
        self.next_tick = 0.0

    # -- discovery ---------------------------------------------------------
    def find(self):
        found = {}
        base = os.path.join(self.sys, "bus/usb/devices")
        try:
            names = sorted(os.listdir(base))
        except OSError:
            return found
        for name in names:
            if ":" in name:
                continue
            d = os.path.join(base, name)
            if (_read(d + "/idVendor"), _read(d + "/idProduct")) not in IDS:
                continue
            for sub in sorted(os.listdir(d)):
                idir = os.path.join(d, sub)
                if not sub.startswith(name + ":") or _read(idir + "/bInterfaceClass") != "03":
                    continue
                ep = self._interrupt_in(idir)
                if ep is None:
                    continue
                bus, num = _read(d + "/busnum"), _read(d + "/devnum")
                if not bus or not num:
                    continue
                node = os.path.join(self.dev, "bus/usb/%03d/%03d" % (int(bus), int(num)))
                found[name] = Pad(name, sub, int(_read(idir + "/bInterfaceNumber") or "0", 16),
                                  ep[0], ep[1], node)
                found[name].idir = idir
                break
        return found

    @staticmethod
    def _interrupt_in(idir):
        for e in sorted(os.listdir(idir)):
            if not e.startswith("ep_"):
                continue
            if _read("%s/%s/type" % (idir, e)) == "Interrupt" and _read("%s/%s/direction" % (idir, e)) == "in":
                return int(_read("%s/%s/bEndpointAddress" % (idir, e)), 16), \
                    min(int(_read("%s/%s/wMaxPacketSize" % (idir, e)), 16), 1024)
        return None

    def _usbhid(self, op):
        return os.path.join(self.sys, "bus/usb/drivers/usbhid", op)

    def _driver(self, pad):
        link = pad.idir + "/driver"
        return os.path.basename(os.readlink(link)) if os.path.islink(link) else None

    def _hidraw_node(self, pad):
        try:
            for h in os.listdir(pad.idir):
                r = os.path.join(pad.idir, h, "hidraw")
                if os.path.isdir(r):
                    for n in sorted(os.listdir(r)):
                        return os.path.join(self.dev, n)
        except OSError:
            pass
        return None

    # -- transitions -------------------------------------------------------
    def _attached(self, pad, now):
        pad.state, pad.until = "attached", now + GRACE
        pad.silent = 0

    def _open_probe(self, pad, now):
        try:
            pad.probe = self.probe_factory(pad.dev_node, pad.ifnum, pad.ep, pad.size)
        except OSError as e:
            pad.probe, pad.retry_at = None, now + 5.0
            self.log("%s: cannot claim the interface (%s)" % (pad.name, e))
            return False
        pad.state = "detached"
        return True

    def _detach(self, pad, now):
        if not _write(self._usbhid("unbind"), pad.iface):
            self.log("%s: usbhid unbind failed" % pad.name)
            pad.silent = 0
            return
        self.log("%s: no live reports, detached" % pad.name)
        pad.state, pad.retry_at = "detached", 0.0
        self._open_probe(pad, now)

    def _attach(self, pad, now):
        if pad.probe:
            pad.probe.close()
            pad.probe = None
        if not _write(self._usbhid("bind"), pad.iface):
            self.log("%s: usbhid bind failed" % pad.name)
            pad.state, pad.retry_at = "detached", now + 1.0
            return
        self.log("%s: reports flowing, attached" % pad.name)
        self._attached(pad, now)

    # -- loop pieces -------------------------------------------------------
    def _adopt(self, now):
        seen = self.find()
        for name in [n for n in self.pads if n not in seen]:
            p = self.pads.pop(name)
            if p.probe:
                p.probe.close()
            self.log("%s: gone" % name)
        for name, new in seen.items():
            if name in self.pads:
                continue
            drv = self._driver(new)
            if drv == "usbhid":
                # Bound by the kernel (boot, or the receiver re-enumerated): judge it at once, its
                # nodes are already visible.
                self._attached(new, now)
                new.until = now
            elif drv is None:
                new.state = "detached"
            else:
                continue          # claimed by something else: not ours
            self.pads[name] = new
            self.log("%s: found (%s)" % (name, new.state))

    def _sample(self, pad, now):
        if now < pad.until:
            return
        node = self._hidraw_node(pad)
        if node and self.hidraw(node, HIDRAW_PROBE) >= CONFIRM_REPORTS:
            pad.silent = 0
            return
        pad.silent += 1
        if pad.silent >= SILENT_TICKS:
            self._detach(pad, now)

    def tick(self):
        now = self.clock()
        self._adopt(now)
        for pad in list(self.pads.values()):
            if pad.state == "attached":
                if self._driver(pad) != "usbhid":
                    pad.state = "detached"      # someone else unbound it; probe next
                    pad.retry_at = now
                else:
                    self._sample(pad, now)
            elif pad.state == "detached":
                if pad.probe and pad.probe.dead:
                    pad.probe.close()
                    pad.probe, pad.retry_at = None, now + 1.0
                if pad.probe is None and now >= pad.retry_at:
                    if self._driver(pad) is None:
                        self._open_probe(pad, now)
                    elif self._driver(pad) == "usbhid":
                        self._attached(pad, now)
                if pad.probe is not None:
                    self._look(pad, now)
        self.next_tick = now + TICK

    def _look(self, pad, now):
        """Detached: sample the endpoint; enough live reports mean the pad is on."""
        n = pad.probe.sample()
        if pad.probe.dead:
            pad.probe.close()
            pad.probe, pad.retry_at = None, now + 1.0
        elif n >= CONFIRM_REPORTS:
            self._attach(pad, now)

    def run(self):
        while True:
            self.tick()
            time.sleep(max(0.0, self.next_tick - self.clock()))


def main():
    lock = open("/tmp/padguard.lock", "w")
    try:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except OSError:
        return 0
    logpath = "/tmp/padguard.log"

    def log(msg):
        try:
            if os.path.getsize(logpath) > LOG_MAX:
                os.truncate(logpath, 0)
        except OSError:
            pass
        with open(logpath, "a") as f:
            f.write("%s %s\n" % (time.strftime("%H:%M:%S"), msg))

    guard = Guard(log=log)
    log("start")
    while True:
        try:
            guard.run()
        except Exception as e:  # keep guarding whatever happens
            log("error: %r" % (e,))
            for p in guard.pads.values():
                if p.probe:
                    p.probe.close()
                    p.probe = None
            guard.pads.clear()
            time.sleep(2)


if __name__ == "__main__":
    sys.exit(main())
