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
    """Stands in for the usbfs prober; sample() returns `live` live reports."""
    made = []

    def __init__(self, path, ifnum, ep, size):
        self.args = (path, ifnum, ep, size)
        self.live, self.dead, self.closed, self.samples = 0, False, False, 0
        FakeProbe.made.append(self)

    def sample(self):
        self.samples += 1
        return self.live

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

    def step(self, dt=1.0):
        """One tick `dt` seconds on."""
        self.now += dt
        self.g.tick()
        return self.settle()

    def pad(self):
        return self.g.pads["8-1"]


class AttachedToDetached(Rig):
    LIVE = padguard.CONFIRM_REPORTS

    def run_ticks(self, n, live):
        self.raw = self.LIVE if live else 0
        return [self.step() for _ in range(n)]

    def test_a_live_pad_stays_attached(self):
        self.g.tick()
        ops = self.run_ticks(10, live=True)
        self.assertEqual(ops, [None] * 10)
        self.assertEqual(self.pad().state, "attached")

    def test_a_pad_that_is_off_is_detached_after_two_samples(self):
        self.g.tick()
        ops = self.run_ticks(4, live=False)
        self.assertEqual(ops.count("unbind"), 1)
        self.assertEqual(ops.index("unbind"), 0)       # a kernel-bound pad has no grace: sampled on discovery, again a tick later
        self.assertEqual(self.pad().state, "detached")
        self.assertEqual(self.pad().probe.args, (self.dev + "/bus/usb/008/015", 0, 0x82, 64))
        self.assertEqual(self.raw_calls[0], self.dev + "/hidraw0")

    def test_our_own_bind_gets_a_grace_period(self):
        self.g.tick()
        self.raw = 0
        while self.pad().state != "detached":
            self.step()
        self.pad().probe.live = padguard.CONFIRM_REPORTS
        self.step()
        self.assertEqual(self.pad().state, "attached")
        self.raw_calls.clear()
        self.step(dt=0.5)                               # inside GRACE: not sampled
        self.assertEqual(self.raw_calls, [])
        self.step(dt=0.5)
        self.assertEqual(len(self.raw_calls), 1)

    def test_too_few_live_reports_count_as_off(self):
        self.g.tick()
        self.raw = self.LIVE - 1
        ops = [self.step() for _ in range(6)]
        self.assertIn("unbind", ops)

    def test_a_single_quiet_sample_is_forgiven(self):
        self.g.tick()
        self.run_ticks(3, live=True)
        self.run_ticks(1, live=False)
        self.assertEqual(self.pad().state, "attached")
        self.run_ticks(1, live=True)
        self.run_ticks(1, live=False)
        self.assertEqual(self.pad().state, "attached")

    def test_no_hidraw_node_counts_as_off(self):
        import shutil
        shutil.rmtree(self.sys + "/bus/usb/devices/8-1/8-1:1.0/0003:054C:09CC.000B/hidraw")
        self.g.tick()
        self.raw = 99
        ops = [self.step() for _ in range(6)]
        self.assertIn("unbind", ops)

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
        self.raw = 0
        while self.pad().state != "detached":
            self.step()
        return self.pad().probe

    def test_the_probe_is_sampled_every_tick_while_detached(self):
        probe = self.detach()
        before = probe.samples
        for _ in range(4):
            self.step()
        self.assertEqual(probe.samples, before + 4)
        self.assertEqual(self.pad().state, "detached")

    def test_a_pad_turned_on_reattaches_on_the_next_tick(self):
        probe = self.detach()
        probe.live = padguard.CONFIRM_REPORTS
        self.assertEqual(self.step(), "bind")
        self.assertTrue(probe.closed)                 # usbfs released before usbhid binds
        self.assertEqual(self.pad().state, "attached")
        self.assertIsNone(self.pad().probe)

    def test_a_few_live_reports_are_not_enough(self):
        probe = self.detach()
        probe.live = padguard.CONFIRM_REPORTS - 1
        for _ in range(5):
            self.assertIsNone(self.step())
        self.assertEqual(self.pad().state, "detached")

    def test_a_probe_that_died_is_reopened_after_a_back_off(self):
        probe = self.detach()
        probe.dead = True
        self.step(dt=0.5)
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
            probe.live = padguard.CONFIRM_REPORTS
            self.assertEqual(self.step(), "bind")
            self.assertEqual(self.pad().state, "attached")

    def test_a_failed_bind_is_retried(self):
        probe = self.detach()
        probe.live = padguard.CONFIRM_REPORTS
        os.unlink(self.drv + "/bind")
        os.mkdir(self.drv + "/bind")
        self.now += 1
        self.g.tick()
        self.assertTrue(any("bind failed" in m for m in self.logs))
        os.rmdir(self.drv + "/bind")
        put(self.drv + "/bind")
        self.step(dt=1)
        self.pad().probe.live = padguard.CONFIRM_REPORTS
        self.assertEqual(self.step(dt=0.5), "bind")
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
        self.assertEqual(padguard._DISCARDURB, 0x0000550B)
        self.assertEqual(padguard.Guard.__init__.__defaults__[3].__name__, "UsbfsProbe")


class LiveReports(unittest.TestCase):
    """read_hidraw counts only report ID 0x01; a pad that is off streams ID 0x00."""

    def count(self, reports):
        from unittest import mock
        feed = list(reports)
        with mock.patch.object(padguard.os, "open", return_value=7), \
                mock.patch.object(padguard.os, "close"), \
                mock.patch.object(padguard.select, "select", side_effect=lambda r, w, x, t: (r, [], []) if feed else ([], [], [])), \
                mock.patch.object(padguard.os, "read", side_effect=lambda fd, n: feed.pop(0)):
            return padguard.read_hidraw("/dev/hidraw0", 0.3)

    def test_zero_id_reports_from_a_receiver_with_the_pad_off_are_not_live(self):
        off = bytes(10) + b"\xb6\x45" + bytes(52)
        self.assertEqual(self.count([off] * 100), 0)

    def test_id_one_reports_are_live_and_counting_stops_at_the_threshold(self):
        on = b"\x01\x83\x80\x7e" + bytes(60)
        self.assertEqual(self.count([on] * 100), padguard.CONFIRM_REPORTS)

    def test_a_mix_counts_only_the_live_ones(self):
        on, off = b"\x01" + bytes(63), bytes(64)
        self.assertEqual(self.count([on, off, on, off, on]), 3)


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
