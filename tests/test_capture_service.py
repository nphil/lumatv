"""Run with: python3 -m unittest tests.test_capture_service   (needs `ambient/capture/build.sh` for the host build)

lumatv-capture (ambient/capture/service.c), the 32-bit colour reader the package ships, on the host against the fake
libvtcapture and a stand-in for the helper's capture socket (the helper listens only while a picture is wanted; the service
connects, is told the preset, and sends one frame per poll, ambient/src/wire.rs). What matters: nothing runs while nobody
wants a picture, the capture is given back the moment the helper closes the connection, LG's policy callback or a failing
library ends it cleanly, a refused init is not retried in a tight loop, and the frames carry what the library said.
"""
import os
import signal
import socket
import struct
import subprocess
import tempfile
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
BUILD = os.path.join(os.path.dirname(HERE), "ambient", "capture", "build", "host")
BIN = os.path.join(BUILD, "lumatv-capture")

MAGIC = b"AMB1"
ZONES = {0: 16, 1: 16, 2: 16, 3: 14}


def q16(v):
    return int(max(0.0, min(1.0, v)) * 65535 + 0.5)


@unittest.skipUnless(os.path.exists(BIN), "needs ambient/capture/build.sh (the host build)")
class Service(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="capture-test-")
        self.sock = os.path.join(self.tmp, "ambient.sock")
        self.fin = os.path.join(self.tmp, "fin")
        self.procs, self.listeners, self.conns = [], [], []
        self.addCleanup(self.cleanup)

    def cleanup(self):
        for c in self.conns:
            c.close()
        for p in self.procs:
            if p.poll() is None:
                p.kill()
            p.wait()
            if p.stderr:
                p.stderr.close()
        for s in self.listeners:
            s.close()
        for name in os.listdir(self.tmp):
            os.unlink(os.path.join(self.tmp, name))
        os.rmdir(self.tmp)

    def start(self, *args, **env):
        e = dict(os.environ, LD_LIBRARY_PATH=BUILD, FAKE_VT_FIN_LOG=self.fin, **env)
        p = subprocess.Popen([BIN, "-s", self.sock, "-i", "50", "-b", "400", *args], env=e, stderr=subprocess.PIPE, text=True)
        self.procs.append(p)
        return p

    def listen(self):
        s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        s.bind(self.sock)
        s.listen(4)
        s.settimeout(5)
        self.listeners.append(s)
        return s

    def accept(self, listener, seconds=5):
        listener.settimeout(seconds)
        conn, _ = listener.accept()
        conn.settimeout(3)
        self.conns.append(conn)
        return conn

    def read_frame(self, conn):
        head = self.read_exact(conn, 12)
        preset, areas = head[4], head[5]
        body = self.read_exact(conn, areas * 12)
        return head + body

    def read_exact(self, conn, n):
        data = b""
        while len(data) < n:
            chunk = conn.recv(n - len(data))
            if not chunk:
                raise EOFError("closed after %d of %d bytes" % (len(data), n))
            data += chunk
        return data

    def finalized(self):
        try:
            with open(self.fin) as f:
                return len(f.read().split())
        except FileNotFoundError:
            return 0

    def wait_for(self, what, seconds=5):
        end = time.time() + seconds
        while time.time() < end:
            if what():
                return True
            time.sleep(0.02)
        return False

    # ---- the frames

    def test_nothing_runs_until_the_socket_exists_and_then_the_frames_carry_what_the_library_said(self):
        p = self.start()
        time.sleep(0.4)
        self.assertIsNone(p.poll(), "it waits, it does not exit")
        self.assertEqual(self.finalized(), 0)
        listener = self.listen()
        conn = self.accept(listener)
        conn.sendall(b"\x00")
        frames = [self.read_frame(conn) for _ in range(6)]
        for f in frames:
            self.assertEqual(f[:4], MAGIC)
            self.assertEqual((f[4], f[5], f[6], f[7]), (0, 16, 0, 0))
            self.assertEqual(len(f), 12 + 16 * 12)
        times = [struct.unpack_from("<I", f, 8)[0] for f in frames]
        self.assertEqual(times, sorted(times))
        self.assertGreater(times[-1], times[0], "the capture time moves")
        f = frames[-1]
        for i in (0, 3, 15):
            r, g, b, black, count = struct.unpack_from("<HHHHI", f, 12 + 12 * i)
            self.assertEqual(g, q16((i * 8 % 256) / 255), "area %d green: the library put 1000 pixels at code %d" % (i, i * 8 % 256))
            self.assertEqual(b, q16((255 - i * 4 % 256) / 255), "area %d blue" % i)
            self.assertEqual(count, 1000)
            self.assertEqual(black, 0, "blue is never near black, so no area is")

    def test_the_helper_changing_the_preset_changes_the_frames_and_nothing_restarts(self):
        self.start()
        conn = self.accept(self.listen())
        conn.sendall(b"\x00")
        self.assertEqual(self.read_frame(conn)[4], 0)
        conn.sendall(b"\x03")
        for _ in range(40):
            f = self.read_frame(conn)
            if f[4] == 3:
                break
        self.assertEqual((f[4], f[5], len(f)), (3, 14, 12 + 14 * 12), "the games preset has 14 areas")
        self.assertEqual(self.finalized(), 0, "no new capture was started for it")

    def test_an_eagain_from_the_library_is_not_a_failure(self):
        p = self.start(FAKE_VT_EAGAIN_EVERY="2")
        conn = self.accept(self.listen())
        conn.sendall(b"\x00")
        for _ in range(12):
            self.read_frame(conn)
        self.assertIsNone(p.poll())
        self.assertEqual(self.finalized(), 0)

    # ---- giving the capture back

    def test_the_capture_is_given_back_when_the_helper_closes_and_taken_again_when_it_asks_again(self):
        p = self.start()
        listener = self.listen()
        conn = self.accept(listener)
        conn.sendall(b"\x00")
        self.read_frame(conn)
        conn.close()
        self.assertTrue(self.wait_for(lambda: self.finalized() == 1, 2), "finalized at once, exactly once")
        conn2 = self.accept(listener)
        conn2.sendall(b"\x01")
        self.assertEqual(self.read_frame(conn2)[4], 1)
        p.send_signal(signal.SIGTERM)
        self.assertEqual(p.wait(timeout=5), 0)
        self.assertEqual(self.finalized(), 2, "once more when it was stopped, and not twice")

    def test_lgs_policy_callback_ends_the_capture_and_the_helper_sees_it_close(self):
        self.start(FAKE_VT_POLICY_MS="400")
        conn = self.accept(self.listen())
        conn.sendall(b"\x00")
        with self.assertRaises(EOFError):
            while True:
                self.read_frame(conn)
        self.assertEqual(self.finalized(), 1)

    def test_a_refused_init_closes_the_connection_and_is_not_retried_in_a_tight_loop(self):
        p = self.start(FAKE_VT_DENY="1")
        listener = self.listen()
        listener.settimeout(0.05)
        accepted = 0
        end = time.time() + 2.5
        while time.time() < end:
            try:
                conn, _ = listener.accept()
            except socket.timeout:
                continue
            accepted += 1
            self.conns.append(conn)
            conn.settimeout(2)
            conn.sendall(b"\x00")
            self.assertEqual(conn.recv(1), b"", "closed without a frame")
        self.assertGreaterEqual(accepted, 2, "it tries again")
        self.assertLessEqual(accepted, 8, "after the back-off, not at once: %d tries in 2.5 s" % accepted)
        self.assertEqual(self.finalized(), 0, "nothing was taken, nothing to give back")
        self.assertIsNone(p.poll())

    # ---- ending

    def test_a_stop_while_it_waits_exits_at_once(self):
        p = self.start()
        time.sleep(0.2)
        p.send_signal(signal.SIGTERM)
        self.assertEqual(p.wait(timeout=3), 0)
        self.assertEqual(self.finalized(), 0)

    def test_the_dry_run_resolves_the_four_functions_and_takes_nothing(self):
        r = subprocess.run([BIN, "-n"], env=dict(os.environ, LD_LIBRARY_PATH=BUILD, FAKE_VT_FIN_LOG=self.fin), capture_output=True, text=True, timeout=10)
        self.assertEqual(r.returncode, 0)
        self.assertRegex(r.stdout, r"^library: init=0x\w+ preset=0x\w+ get=0x\w+ finalize=0x\w+")
        self.assertEqual(self.finalized(), 0)

    def test_it_does_not_run_without_a_socket_to_look_for(self):
        r = subprocess.run([BIN], env=dict(os.environ, LD_LIBRARY_PATH=BUILD, LUMATV_CAPTURE_SOCKET=""), capture_output=True, text=True, timeout=10)
        self.assertEqual(r.returncode, 1)


if __name__ == "__main__":
    unittest.main()
