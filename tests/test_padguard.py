"""Run with: python3 -m unittest tests.test_padguard

payload/padguard/padguard.py against a fake sysfs/devfs: the attached/detached state machine, the
hidraw double-check, the usbfs structures' ABI, and the boot hook's shape. The set is not touched.
"""
import ctypes
import os
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
PAD = os.path.join(os.path.dirname(HERE), "payload", "padguard")
sys.path.insert(0, PAD)
import padguard  # noqa: E402


def put(path, text=""):
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "w") as f:
        f.write(text)


class FakeProbe:
    """Stands in for the usbfs prober; `queue` is how many reports each service() reaps."""
    made = []

    def __init__(self, path, ifnum, ep, size):
        self.args = (path, ifnum, ep, size)
        self.queue, self.dead, self.closed = 0, False, False
        FakeProbe.made.append(self)

    def fileno(self):
        return 99

    def service(self):
        n, self.queue = self.queue, 0
        return n

    def close(self):
        self.closed = True


class Rig(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.sys = self.tmp.name + "/sys"
        self.dev = self.tmp.name + "/dev"
        d = self.sys + "/bus/usb/devices/8-1"
        i = d + "/8-1:1.0"
        put(d + "/idVendor", "054c\n")
        put(d + "/idProduct", "09cc\n")
        put(d + "/busnum", "8\n")
        put(d + "/devnum", "15\n")
        put(d + "/urbnum", "1000\n")
        put(i + "/bInterfaceClass", "03\n")
        put(i + "/bInterfaceNumber", "00\n")
        put(i + "/ep_02/type", "Interrupt\n")
        put(i + "/ep_02/direction", "out\n")
        put(i + "/ep_02/bEndpointAddress", "02\n")
        put(i + "/ep_02/wMaxPacketSize", "0040\n")
        put(i + "/ep_82/type", "Interrupt\n")
        put(i + "/ep_82/direction", "in\n")
        put(i + "/ep_82/bEndpointAddress", "82\n")
        put(i + "/ep_82/wMaxPacketSize", "0040\n")
        put(i + "/0003:054C:09CC.000B/hidraw/hidraw0/uevent")
        self.drv = self.sys + "/bus/usb/drivers/usbhid"
        put(self.drv + "/bind")
        put(self.drv + "/unbind")
        self.link = i + "/driver"
        self.bind_iface()
        self.now = 100.0
        self.logs = []
        self.raw = 0
        self.raw_calls = []
        FakeProbe.made = []
        self.g = padguard.Guard(self.sys, self.dev, clock=lambda: self.now, probe_factory=FakeProbe,
                                hidraw=self.hidraw, log=self.logs.append)

    def hidraw(self, node, secs):
        self.raw_calls.append(node)
        return self.raw

    def bind_iface(self):
        if not os.path.islink(self.link):
            os.symlink("../../../../bus/usb/drivers/usbhid", self.link)

    def settle(self):
        """What the kernel does with a write to usbhid's bind/unbind."""
        for op in ("unbind", "bind"):
            p = self.drv + "/" + op
            text = open(p).read()
            if text:
                self.assertEqual(text, "8-1:1.0")
                open(p, "w").close()
                if op == "unbind" and os.path.islink(self.link):
                    os.unlink(self.link)
                if op == "bind":
                    self.bind_iface()
                return op
        return None

    def urbs(self, n):
        put(self.sys + "/bus/usb/devices/8-1/urbnum", "%d\n" % n)

    def step(self, rate=None, dt=1.0):
        """One tick `dt` seconds on, with the pad submitting `rate` URBs/s since the last one."""
        self.now += dt
        if rate is not None:
            cur = int(open(self.sys + "/bus/usb/devices/8-1/urbnum").read())
            self.urbs(cur + int(rate * dt))
        self.g.tick()
        return self.settle()

    def pad(self):
        return self.g.pads["8-1"]


class AttachedToDetached(Rig):
    def test_a_live_pad_stays_attached(self):
        self.g.tick()
        for _ in range(10):
            self.assertIsNone(self.step(rate=1000))
        self.assertEqual(self.pad().state, "attached")
        self.assertEqual(self.raw_calls, [])        # no hidraw reads while the counter is healthy

    def test_a_silent_pad_is_detached_within_grace_plus_two_samples(self):
        self.g.tick()
        ops = [self.step(rate=0) for _ in range(6)]
        self.assertEqual(ops.count("unbind"), 1)
        self.assertEqual(self.pad().state, "detached")
        self.assertIsNotNone(self.pad().probe)       # claimed through usbfs
        self.assertEqual(self.pad().probe.args, (self.dev + "/bus/usb/008/015", 0, 0x82, 64))
        # grace is 3 s: ticks at +1,+2 ignored, +3 first sample, +4 second -> unbound on the 4th
        self.assertEqual(ops.index("unbind"), 3)

    def test_grace_after_start_ignores_the_probe_noise_of_a_fresh_bind(self):
        self.g.tick()
        self.assertIsNone(self.step(rate=0))
        self.assertIsNone(self.step(rate=0))
        self.assertEqual(self.pad().state, "attached")

    def test_low_counter_but_hidraw_reports_means_nobody_was_polling(self):
        self.g.tick()
        self.raw = padguard.CONFIRM_REPORTS
        for _ in range(8):
            self.assertIsNone(self.step(rate=0))
        self.assertEqual(self.pad().state, "attached")
        self.assertTrue(self.raw_calls)
        self.assertEqual(self.raw_calls[0], self.dev + "/hidraw0")

    def test_a_few_stray_urbs_do_not_count_as_a_live_pad(self):
        self.g.tick()
        ops = [self.step(rate=3) for _ in range(6)]
        self.assertIn("unbind", ops)

    def test_a_single_quiet_second_is_forgiven(self):
        self.g.tick()
        for _ in range(3):
            self.step(rate=1000)
        self.step(rate=0)
        self.assertEqual(self.pad().state, "attached")
        self.step(rate=1000)
        self.step(rate=0)
        self.assertEqual(self.pad().state, "attached")

    def test_failed_unbind_keeps_it_attached_and_retries(self):
        os.unlink(self.drv + "/unbind")
        os.mkdir(self.drv + "/unbind")         # opening a directory for write fails
        self.g.tick()
        for _ in range(6):
            self.now += 1
            self.g.tick()
        self.assertEqual(self.pad().state, "attached")
        self.assertTrue(any("unbind failed" in m for m in self.logs))


class DetachedToAttached(Rig):
    def detach(self):
        self.g.tick()
        while self.pad().state != "detached":
            self.step(rate=0)
        return self.pad().probe

    def test_a_burst_of_reports_reattaches(self):
        probe = self.detach()
        probe.queue = padguard.CONFIRM_REPORTS
        self.now += 0.2
        self.g.wake({probe})
        self.assertEqual(self.settle(), "bind")
        self.assertTrue(probe.closed)                 # usbfs released before usbhid binds
        self.assertEqual(self.pad().state, "attached")
        self.assertIsNone(self.pad().probe)

    def test_reports_trickling_in_across_windows_do_not(self):
        probe = self.detach()
        for _ in range(5):
            probe.queue = padguard.CONFIRM_REPORTS - 1
            self.now += padguard.CONFIRM_WINDOW + 0.1
            self.g.wake({probe})
        self.assertIsNone(self.settle())
        self.assertEqual(self.pad().state, "detached")

    def test_reports_accumulate_inside_one_window(self):
        probe = self.detach()
        for _ in range(4):
            probe.queue = 3
            self.now += 0.05
            self.g.wake({probe})
        self.assertEqual(self.settle(), "bind")

    def test_a_probe_that_died_is_reopened_on_the_next_tick(self):
        probe = self.detach()
        probe.dead = True
        self.g.wake({probe})
        self.assertTrue(probe.closed)
        self.assertIsNone(self.pad().probe)
        self.step(dt=0.5)
        self.assertIsNone(self.pad().probe)           # one second back-off
        self.step(dt=1)
        self.assertIsNotNone(self.pad().probe)
        self.assertIsNot(self.pad().probe, probe)

    def test_the_full_cycle_repeats(self):
        for _ in range(3):
            probe = self.detach()
            probe.queue = padguard.CONFIRM_REPORTS
            self.g.wake({probe})
            self.assertEqual(self.settle(), "bind")
            self.assertEqual(self.pad().state, "attached")

    def test_a_pad_found_unbound_at_start_goes_straight_to_probing(self):
        os.unlink(self.link)
        self.g.tick()
        self.assertEqual(self.pad().state, "detached")
        self.assertIsNotNone(self.pad().probe)

    def test_claim_failure_backs_off_and_retries(self):
        def boom(*a):
            raise OSError(16, "busy")
        os.unlink(self.link)
        self.g.probe_factory = boom
        self.g.tick()
        self.assertIsNone(self.pad().probe)
        self.g.probe_factory = FakeProbe
        self.step(dt=1)
        self.assertIsNone(self.pad().probe)
        self.step(dt=5)
        self.assertIsNotNone(self.pad().probe)


class Discovery(Rig):
    def test_other_devices_are_ignored(self):
        put(self.sys + "/bus/usb/devices/8-2/idVendor", "046d\n")
        put(self.sys + "/bus/usb/devices/8-2/idProduct", "c52b\n")
        self.g.tick()
        self.assertEqual(list(self.g.pads), ["8-1"])

    def test_an_interface_held_by_another_driver_is_left_alone(self):
        os.unlink(self.link)
        os.symlink("../../../../bus/usb/drivers/xpad", self.link)
        self.g.tick()
        self.assertEqual(self.g.pads, {})

    def test_unplug_drops_the_pad_and_its_probe(self):
        os.unlink(self.link)
        self.g.tick()
        probe = self.pad().probe
        import shutil
        shutil.rmtree(self.sys + "/bus/usb/devices/8-1")
        self.step()
        self.assertEqual(self.g.pads, {})
        self.assertTrue(probe.closed)

    def test_a_second_pad_is_tracked_independently(self):
        import shutil
        shutil.copytree(self.sys + "/bus/usb/devices/8-1", self.sys + "/bus/usb/devices/8-2", symlinks=True)
        os.rename(self.sys + "/bus/usb/devices/8-2/8-1:1.0", self.sys + "/bus/usb/devices/8-2/8-2:1.0")
        os.unlink(self.sys + "/bus/usb/devices/8-2/8-2:1.0/driver")
        put(self.sys + "/bus/usb/devices/8-2/devnum", "16\n")
        self.g.tick()
        self.assertEqual(self.g.pads["8-1"].state, "attached")
        self.assertEqual(self.g.pads["8-2"].state, "detached")
        self.assertEqual(self.g.pads["8-2"].probe.args[0], self.dev + "/bus/usb/008/016")


class Abi(unittest.TestCase):
    def test_usbfs_structs_match_the_kernel_on_64_bit(self):
        if ctypes.sizeof(ctypes.c_void_p) != 8:
            self.skipTest("64-bit layout")
        self.assertEqual(ctypes.sizeof(padguard._Urb), 56)
        self.assertEqual(padguard._SUBMITURB, 0x8038550A)
        self.assertEqual(padguard._REAPURBNDELAY, 0x4008550D)
        self.assertEqual(padguard._CLAIMINTERFACE, 0x8004550F)
        self.assertEqual(padguard.Guard.__init__.__defaults__[3].__name__, "UsbfsProbe")


class Hook(unittest.TestCase):
    def test_the_boot_hook_is_a_detached_one_liner_with_no_dot_in_its_name(self):
        path = os.path.join(PAD, "50-padguard")
        text = open(path).read()
        self.assertTrue(text.startswith("#!/bin/sh"))
        self.assertTrue(os.access(path, os.X_OK))
        self.assertIn("setsid", text)
        self.assertIn("/var/lib/webosbrew/padguard.py", text)
        self.assertTrue(text.rstrip().endswith("exit 0"))


if __name__ == "__main__":
    unittest.main()
