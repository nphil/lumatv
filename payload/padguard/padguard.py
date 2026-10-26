#!/usr/bin/python3
"""padguard: keep a silent USB game controller invisible to apps, a live one usable.

Netflix opens every gamepad evdev node and reads the initial axis values. A
DualShock-4 clone (054c:09cc) that is off, or wired only for charging, sends no
reports, so its axes sit at 0 and read as a stick held full up-left. The
pad is therefore attached (usbhid + hid-sony bound, evdev nodes present) only while
it is sending reports, and detached while it is silent.

Attached -> detached: the interface's sysfs urbnum (URBs submitted, which for an
interrupt endpoint is one per report) is sampled once a second. A low rate is
double-checked by reading the pad's hidraw node briefly, because usbhid only
polls while something has a node open. Silent for SILENT_TICKS samples: usbhid
is unbound from the interface, so every input node vanishes.

Detached -> attached: with usbhid unbound nothing polls the endpoint, so this
process claims the interface through usbfs and keeps one interrupt-IN URB in
flight. A burst of CONFIRM_REPORTS reports means the pad is on: the interface
is released and usbhid is bound again (hid-sony probes, nodes reappear).
The URB wait is a blocking poll(), so the detached state costs no CPU.

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
TICK = 1.0            # seconds between attached-state samples
GRACE = 3.0           # after an attach, hid-sony probing makes the counter meaningless
MIN_RATE = 20         # URBs/s below which an attached pad looks silent (a live pad: ~1000)
SILENT_TICKS = 2      # consecutive silent samples before detaching
HIDRAW_PROBE = 0.5    # seconds the confirmation read waits for a report
CONFIRM_REPORTS = 10  # reports in CONFIRM_WINDOW that prove a detached pad is on
CONFIRM_WINDOW = 1.0
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
    """Claims one interface and counts reports on its interrupt-IN endpoint."""

    def __init__(self, path, ifnum, ep, size):
        self.count = 0
        self.dead = False
        self.fd = os.open(path, os.O_RDWR | os.O_NONBLOCK)
        try:
            _ioctl(self.fd, _CLAIMINTERFACE, ctypes.byref(ctypes.c_uint(ifnum)))
            self.buf = ctypes.create_string_buffer(size)
            self.urb = _Urb(type=_URB_TYPE_INTERRUPT, endpoint=ep,
                            buffer=ctypes.addressof(self.buf), buffer_length=size)
            self._submit()
        except Exception:
            os.close(self.fd)
            raise

    def _submit(self):
        _ioctl(self.fd, _SUBMITURB, ctypes.byref(self.urb))

    def fileno(self):
        return self.fd

    def service(self):
        """Reap completed URBs; return how many carried a report."""
        n = 0
        while not self.dead:
            out = ctypes.c_void_p()
            try:
                _ioctl(self.fd, _REAPURBNDELAY, ctypes.byref(out))
            except OSError as e:
                if e.errno != errno.EAGAIN:
                    self.dead = True
                return n
            if self.urb.status != 0:
                self.dead = True
                return n
            if self.urb.actual_length > 0:
                n += 1
            try:
                self._submit()
            except OSError:
                self.dead = True
        return n

    def close(self):
        # Closing the fd discards the URB and releases the interface.
        if self.fd >= 0:
            os.close(self.fd)
            self.fd = -1


def read_hidraw(node, secs):
    """Count reports on a hidraw node for up to `secs` (a reader never steals evdev events)."""
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
                    if os.read(fd, 128):
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
        self.last_urbnum = None
        self.last_time = 0.0
        self.silent = 0
        self.burst = 0           # detached: reports seen in the current window
        self.burst_start = 0.0
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

    def _urbnum(self, pad):
        v = _read(os.path.join(self.sys, "bus/usb/devices", pad.name, "urbnum"))
        return int(v) if v and v.isdigit() else None

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
        pad.last_urbnum, pad.last_time, pad.silent = self._urbnum(pad), now, 0

    def _open_probe(self, pad, now):
        try:
            pad.probe = self.probe_factory(pad.dev_node, pad.ifnum, pad.ep, pad.size)
        except OSError as e:
            pad.probe, pad.retry_at = None, now + 5.0
            self.log("%s: cannot claim the interface (%s)" % (pad.name, e))
            return False
        pad.state, pad.burst, pad.burst_start = "detached", 0, 0.0
        return True

    def _detach(self, pad, now):
        if not _write(self._usbhid("unbind"), pad.iface):
            self.log("%s: usbhid unbind failed" % pad.name)
            pad.silent = 0
            return
        self.log("%s: silent, detached" % pad.name)
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
                self._attached(new, now)
            elif drv is None:
                new.state = "detached"
            else:
                continue          # claimed by something else: not ours
            self.pads[name] = new
            self.log("%s: found (%s)" % (name, new.state))

    def _sample(self, pad, now):
        n = self._urbnum(pad)
        if now < pad.until:
            pad.last_urbnum, pad.last_time = n, now
            return
        dt = max(now - pad.last_time, 1e-3)
        rate = (n - pad.last_urbnum) / dt if n is not None and pad.last_urbnum is not None else None
        pad.last_urbnum, pad.last_time = n, now
        if rate is not None and rate >= MIN_RATE:
            pad.silent = 0
            return
        # usbhid only polls while a node is open: look at the pad directly.
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
        self.next_tick = now + TICK

    def wake(self, ready):
        """Handle probes that polled readable; attach pads whose reports confirm the pad is on."""
        now = self.clock()
        for pad in list(self.pads.values()):
            if pad.state != "detached" or pad.probe is None or pad.probe not in ready:
                continue
            n = pad.probe.service()
            if pad.probe.dead:
                pad.probe.close()
                pad.probe, pad.retry_at = None, now + 1.0
                continue
            if n == 0:
                continue
            if now - pad.burst_start > CONFIRM_WINDOW:
                pad.burst, pad.burst_start = 0, now
            pad.burst += n
            if pad.burst >= CONFIRM_REPORTS:
                self._attach(pad, now)

    def probes(self):
        return [p.probe for p in self.pads.values() if p.state == "detached" and p.probe]

    def run(self):
        self.tick()
        while True:
            poller = select.poll()
            by_fd = {}
            for pr in self.probes():
                poller.register(pr.fileno(), select.POLLOUT | select.POLLERR | select.POLLHUP)
                by_fd[pr.fileno()] = pr
            # An empty poller just sleeps until the next tick.
            events = poller.poll(max(0.0, self.next_tick - self.clock()) * 1000)
            if events:
                self.wake({by_fd[fd] for fd, _ in events if fd in by_fd})
            if self.clock() >= self.next_tick:
                self.tick()


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
