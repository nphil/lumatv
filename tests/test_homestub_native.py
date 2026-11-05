"""Run with: python3 -m unittest tests.test_homestub_native   (builds homestub/ with zig when its binaries are missing or older than the source)

lumatv-homestub (homestub/homestub.c), the native Home stub, against a stand-in compositor that speaks the server end of the wire for exactly
what the stub uses: the registry, wl_compositor, wl_shm and wl_webos_shell. Shown: the card it maps (LG's Home id on a card of one transparent
pixel, in front only when SAM did not launch it as a preload), what it costs while it idles, how it ends (the compositor's close, SAM's
signal, the compositor going away), that it waits for a compositor that is restarting, and that the newest of two keeps the screen.
The binary of the set is a static aarch64 ELF of a few KB and runs here under qemu-user when that is installed.
"""
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import threading
import time
import unittest

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "homestub" / "build"
HOST = BUILD / "host" / "lumatv-homestub"
ARM64 = BUILD / "arm64" / "lumatv-homestub"
QEMU = shutil.which("qemu-aarch64-static")
NATIVE_APPINFO = ROOT / "payload" / "homestub-native" / "appinfo.json"
WEB_APPINFO = ROOT / "payload" / "homestub" / "appinfo.json"
# What the stub's size is held to: it is 13 KB, and a libc that drags stdio in is 100.
MAX_BYTES = 32 * 1024


def build_if_needed():
    sources = [ROOT / "homestub" / "homestub.c", ROOT / "homestub" / "build.sh"]
    newest = max(p.stat().st_mtime for p in sources)
    if all(b.exists() and b.stat().st_mtime >= newest for b in (HOST, ARM64)):
        return
    subprocess.run(["sh", str(ROOT / "homestub" / "build.sh")], check=True, capture_output=True, timeout=600)


def words(*values):
    return struct.pack("=%dI" % len(values), *values)


def read_string(body, at):
    n = struct.unpack_from("=I", body, at)[0]
    return body[at + 4:at + 4 + n - 1].decode(), at + 4 + (n + 3) // 4 * 4


class StandInCompositor:
    """The server end of what the stub says, one thread to accept and one to a connection. Everything it is told is in `log`, in order;
    `mapped` is set at the first commit of a connection. Opcodes are the XML's, which are positional (compositor/protocol/webos/webos-shell.xml)."""

    GLOBALS = [(1, "wl_compositor", 4), (2, "wl_shm", 1), (3, "wl_seat", 4), (4, "wl_webos_shell", 1)]

    def __init__(self, directory, name="wl-test", globals_=None):
        self.path = os.path.join(directory, name)
        self.globals = self.GLOBALS if globals_ is None else globals_
        self.log = []
        self.mapped = threading.Event()
        self.connections = 0
        self.shell_surface = {}  # the connection -> the id of its shell surface
        self._lock = threading.Lock()
        self._conns = []
        self._listener = socket.socket(socket.AF_UNIX)
        self._listener.bind(self.path)
        self._listener.listen(4)
        threading.Thread(target=self._accept, daemon=True).start()

    def close(self):
        for c in [self._listener] + self._conns:
            try:
                c.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            c.close()

    def drop_clients(self):
        for c in self._conns:
            try:
                c.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def says(self, *entry):
        return tuple(entry) in self.log

    def wait_for(self, entry, seconds=3):
        end = time.time() + seconds
        while time.time() < end:
            if tuple(entry) in self.log:
                return True
            time.sleep(0.005)
        return tuple(entry) in self.log

    def send_close(self):
        for conn, ident in list(self.shell_surface.items()):
            self._event(conn, ident, 2)  # wl_webos_shell_surface event 2: close

    def _accept(self):
        while True:
            try:
                conn, _ = self._listener.accept()
            except OSError:
                return
            with self._lock:
                self._conns.append(conn)
                self.connections += 1
            threading.Thread(target=self._serve, args=(conn,), daemon=True).start()

    def _event(self, conn, obj, opcode, args=b""):
        conn.sendall(struct.pack("=II", obj, (8 + len(args)) << 16 | opcode) + args)

    def _serve(self, conn):
        objects = {1: "wl_display"}
        buf, fds = b"", []
        while True:
            try:
                data, got, _, _ = socket.recv_fds(conn, 4096, 4)
            except OSError:
                return
            if not data:
                return
            fds += got
            buf += data
            while len(buf) >= 8:
                obj, header = struct.unpack_from("=II", buf)
                size, opcode = header >> 16, header & 0xFFFF
                if len(buf) < size:
                    break
                body, buf = buf[8:size], buf[size:]
                try:
                    self._request(conn, objects, fds, obj, opcode, body)
                except OSError:  # the stub went away while it was answered
                    return

    def _request(self, conn, objects, fds, obj, opcode, body):
        kind = objects.get(obj, "?")
        if kind == "wl_display" and opcode == 1:  # get_registry
            registry = struct.unpack_from("=I", body)[0]
            objects[registry] = "wl_registry"
            for name, iface, version in self.globals:
                text = iface.encode() + b"\0"
                text += b"\0" * (-len(text) % 4)
                self._event(conn, registry, 0, words(name, len(iface) + 1) + text + words(version))
        elif kind == "wl_display" and opcode == 0:  # sync
            callback = struct.unpack_from("=I", body)[0]
            self._event(conn, callback, 0, words(0))
            self._event(conn, 1, 1, words(callback))  # delete_id
        elif kind == "wl_registry" and opcode == 0:  # bind(name, interface, version, id)
            name = struct.unpack_from("=I", body)[0]
            iface, at = read_string(body, 4)
            version, new = struct.unpack_from("=II", body, at)
            objects[new] = iface
            self.log.append(("bind", iface, version, name))
        elif kind == "wl_compositor" and opcode == 0:
            objects[struct.unpack_from("=I", body)[0]] = "wl_surface"
        elif kind == "wl_webos_shell" and opcode == 1:  # get_shell_surface(id, surface)
            ident, surface = struct.unpack_from("=II", body)
            objects[ident] = "wl_webos_shell_surface"
            self.shell_surface[conn] = ident
            self.log.append(("shell_surface", objects.get(surface)))
        elif kind == "wl_webos_shell_surface" and opcode == 1:
            self.log.append(("state", struct.unpack_from("=I", body)[0]))
        elif kind == "wl_webos_shell_surface" and opcode == 2:
            name, at = read_string(body, 0)
            value, _ = read_string(body, at)
            self.log.append(("property", name, value))
        elif kind == "wl_shm" and opcode == 0:  # create_pool(id, fd, size)
            ident, size = struct.unpack_from("=Ii", body)
            objects[ident] = "wl_shm_pool"
            fd = fds.pop(0)
            self.log.append(("pool", size, os.fstat(fd).st_size, os.pread(fd, 8, 0)))
            os.close(fd)
        elif kind == "wl_shm_pool" and opcode == 0:  # create_buffer(id, offset, width, height, stride, format)
            ident, *rest = struct.unpack_from("=Iiiiii", body)
            objects[ident] = "wl_buffer"
            self.log.append(("buffer", *rest))
        elif kind == "wl_shm_pool" and opcode == 1:
            self.log.append(("pool_destroy",))
        elif kind == "wl_surface" and opcode == 1:
            buffer, x, y = struct.unpack_from("=Iii", body)
            self.log.append(("attach", objects.get(buffer), x, y))
        elif kind == "wl_surface" and opcode == 2:
            self.log.append(("damage", *struct.unpack_from("=iiii", body)))
        elif kind == "wl_surface" and opcode == 6:
            self.log.append(("commit",))
            self.mapped.set()


def vm_kb(pid, field="VmRSS"):
    for line in Path("/proc/%d/status" % pid).read_text().splitlines():
        if line.startswith(field + ":"):
            return int(line.split()[1])


def single_instance_held(name):
    """Whether a program listens on the abstract name the stub keeps its one card with (the kernel lists it as @name, state LISTEN)."""
    for line in Path("/proc/net/unix").read_text().splitlines():
        fields = line.split()
        if fields and fields[-1] == "@" + name and fields[3] == "00010000":
            return True
    return False


def cpu_ticks(pid):
    fields = Path("/proc/%d/stat" % pid).read_text().rsplit(")", 1)[1].split()
    return int(fields[11]) + int(fields[12])


class StubTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        if not shutil.which("zig") and not (HOST.exists() and ARM64.exists()):
            raise unittest.SkipTest("zig builds the stub, and there is no build")
        build_if_needed()

    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="homestub-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        # the lock's name is machine-wide: one of its own, so that tests here, the compositor's and a set's own stub never end each other's
        self.single = "lumatv-homestub-test-" + os.path.basename(self.tmp)
        self.procs = []
        self.addCleanup(self.kill_all)
        self.servers = []
        self.addCleanup(lambda: [s.close() for s in self.servers])

    def compositor(self, **kw):
        server = StandInCompositor(self.tmp, **kw)
        self.servers.append(server)
        return server

    def stub(self, binary=HOST, *args, **env):
        cmd = ([QEMU] if binary == ARM64 else []) + [str(binary), *args]
        proc = subprocess.Popen(cmd, env=dict(PATH=os.environ["PATH"], XDG_RUNTIME_DIR=self.tmp, WAYLAND_DISPLAY="wl-test", LUMATV_HOMESTUB_SINGLE=self.single, **env),
                                stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        self.procs.append(proc)
        return proc

    def kill_all(self):
        for p in self.procs:
            if p.poll() is None:
                p.kill()
            p.wait()
            p.stderr.close()

    def settled(self, server, proc):
        self.assertTrue(server.mapped.wait(3), "the card was not mapped: %r" % server.log)
        # what follows the commit is the idle, once it holds its name: a moment for anything that would end it
        end = time.time() + 3
        while not single_instance_held(self.single) and time.time() < end:
            time.sleep(0.002)
        time.sleep(0.05)
        if proc.poll() is not None:
            self.fail("it ended with %s: %r" % (proc.returncode, proc.stderr.read()))

    # ------------------------------------------------------------------------------------------------ the card

    def check_the_card(self, binary, expect_front=True):
        server = self.compositor()
        proc = self.stub(binary)
        self.settled(server, proc)
        log = server.log
        self.assertEqual([e for e in log if e[0] == "bind"],
                         [("bind", "wl_compositor", 1, 1), ("bind", "wl_shm", 1, 2), ("bind", "wl_webos_shell", 1, 4)],
                         "the three it needs, each at version 1 (wl_webos_shell is version 1 on the set) and nothing else")
        self.assertIn(("shell_surface", "wl_surface"), log)
        props = [e[1:] for e in log if e[0] == "property"]
        self.assertEqual(props, [("_WEBOS_WINDOW_TYPE", "_WEBOS_WINDOW_TYPE_CARD"), ("appId", "com.webos.app.home"), ("title", "Home"),
                                 ("lumatv.homeStub", "native")], "the last one tells the compositor this is the stub and not LG's Home")
        # a pixel of zeros in ARGB8888: the pool is four bytes of memory the stub made, the buffer is 1x1 at stride 4 and format 0
        self.assertIn(("pool", 4, 4, b"\0\0\0\0"), log)
        self.assertIn(("buffer", 0, 1, 1, 4, 0), log)
        self.assertEqual([e for e in log if e[0] == "state"], [("state", 3)] if expect_front else [], "fullscreen is the front")
        # created and described before it is shown: the first commit is the last thing it says
        self.assertEqual(log[-3:], [("attach", "wl_buffer", 0, 0), ("damage", 0, 0, 1, 1), ("commit",)])
        return server, proc

    def test_it_maps_a_transparent_card_that_is_lgs_home(self):
        self.check_the_card(HOST)

    @unittest.skipUnless(QEMU and ARM64.exists(), "the set's binary runs under qemu-user")
    def test_the_binary_of_the_set_does_the_same(self):
        self.check_the_card(ARM64)

    def test_a_preload_makes_the_window_and_maps_nothing(self):
        # a card with a buffer comes to the front in LumaTV's compositor and in LG's, so a preload of Home (the preload manager loads it again
        # when it is not running) must not have one: it would take the screen from the app on it
        for argument in ('{"@system_native_app":true,"reason":"preload","interfaceVersion":2,"appId":"com.webos.app.home"}',
                         '{"event":"launch","reason":"undefined","parameters":{"snapshot":true}}',
                         '{"parameters":{"preload":"full"},"appId":"com.webos.app.home"}'):
            with self.subTest(argument=argument):
                server = self.compositor(name="wl-test")
                proc = self.stub(HOST, argument)
                self.assertTrue(server.wait_for(("shell_surface", "wl_surface")), server.log)
                end = time.time() + 3
                while not single_instance_held(self.single) and time.time() < end:
                    time.sleep(0.002)
                time.sleep(0.05)
                self.assertIsNone(proc.poll(), "it stays, waiting to be launched for real")
                self.assertIn(("property", "appId", "com.webos.app.home"), server.log, "the compositor knows whose window it is")
                self.assertFalse([e for e in server.log if e[0] in ("state", "commit", "attach", "pool", "buffer")], server.log)
                self.assertFalse(server.mapped.is_set())
                proc.kill()
                proc.wait()
                server.close()
                os.unlink(server.path)

    def test_a_launch_that_is_no_preload_asks_for_the_front(self):
        server = self.compositor()
        proc = self.stub(HOST, '{"@system_native_app":true,"reason":"undefined","preloadMode":"criu","appDesc":{"title":"Home"}}')
        self.settled(server, proc)
        self.assertIn(("state", 3), server.log)

    def test_it_idles_in_poll_in_a_small_footprint(self):
        server = self.compositor()
        proc = self.stub(HOST)
        self.settled(server, proc)
        before = cpu_ticks(proc.pid)
        time.sleep(0.2)
        self.assertEqual(cpu_ticks(proc.pid) - before, 0, "nothing runs while it waits")
        status = Path("/proc/%d/stat" % proc.pid).read_text().rsplit(")", 1)[1].split()[0]
        self.assertEqual(status, "S", "asleep in poll")
        # the owner's target for the stub is 1 to 2 MB resident; a static program with no heap is a tenth of the lower figure
        self.assertLess(vm_kb(proc.pid), 256)
        self.assertLess(vm_kb(proc.pid, "VmHWM"), 256)
        self.assertEqual(len(os.listdir("/proc/%d/fd" % proc.pid)), 5, "stdio, the compositor and the listener that keeps one card: the pixel's memory is closed")

    # ---------------------------------------------------------------------------------------------- the end

    def test_the_compositors_close_ends_it_cleanly(self):
        server = self.compositor()
        proc = self.stub(HOST)
        self.settled(server, proc)
        server.send_close()
        self.assertEqual(proc.wait(3), 0)

    def test_sams_signal_ends_it_at_once(self):
        server = self.compositor()
        proc = self.stub(HOST)
        self.settled(server, proc)
        proc.send_signal(signal.SIGTERM)
        self.assertEqual(proc.wait(3), 0, "SAM closes a native app with the signal, and the next launch starts a new one")

    def test_a_compositor_that_goes_away_ends_it(self):
        server = self.compositor()
        proc = self.stub(HOST)
        self.settled(server, proc)
        server.drop_clients()
        self.assertEqual(proc.wait(3), 0, "exit 0: Home is launched again after the restart, by SAM and by the boot flow")

    # ------------------------------------------------------------------------------- a compositor that is not up yet

    def test_it_waits_for_a_compositor_that_is_restarting(self):
        proc = self.stub(HOST)
        time.sleep(0.25)
        self.assertIsNone(proc.poll(), "no socket yet: it waits")
        server = self.compositor()
        self.assertTrue(server.mapped.wait(3), "it connected once the socket was there")
        self.assertIsNone(proc.poll())

    def test_a_compositor_that_does_not_advertise_the_shell_yet_is_asked_again(self):
        server = self.compositor(globals_=[(1, "wl_compositor", 4), (2, "wl_shm", 1)])
        proc = self.stub(HOST)
        deadline = time.time() + 3
        while server.connections < 2 and time.time() < deadline:
            time.sleep(0.02)
        self.assertGreaterEqual(server.connections, 2, "a second connection: the first found no wl_webos_shell")
        self.assertFalse(server.mapped.is_set())
        self.assertIsNone(proc.poll())
        server.globals = StandInCompositor.GLOBALS
        self.assertTrue(server.mapped.wait(3))

    # ---------------------------------------------------------------------------------------------- one card

    def test_the_newest_of_two_keeps_the_screen_and_the_older_goes_after_the_newer_is_mapped(self):
        server = self.compositor()
        first = self.stub(HOST)
        self.settled(server, first)
        server.mapped.clear()
        second = self.stub(HOST)
        self.assertTrue(server.mapped.wait(3), "the newer one is mapped first, so that the card it replaces is not missed")
        self.assertEqual(first.wait(3), 0, "and the older goes when it is asked")
        end = time.time() + 3
        while not single_instance_held(self.single) and time.time() < end:
            time.sleep(0.002)
        self.assertIsNone(second.poll(), "the newer stays")
        # a third replaces the second the same way
        server.mapped.clear()
        third = self.stub(HOST)
        self.assertTrue(server.mapped.wait(3))
        self.assertEqual(second.wait(3), 0)
        self.assertIsNone(third.poll())

    # ---------------------------------------------------------------------------------------- the build, the manifest

    def test_the_binary_of_the_set_is_a_small_static_aarch64_program(self):
        data = ARM64.read_bytes()
        self.assertEqual(data[:4], b"\x7fELF")
        self.assertEqual(data[4], 2, "64-bit")
        self.assertEqual(struct.unpack_from("<H", data, 18)[0], 183, "EM_AARCH64")
        phoff = struct.unpack_from("<Q", data, 32)[0]
        phentsize, phnum = struct.unpack_from("<HH", data, 54)
        kinds = [struct.unpack_from("<I", data, phoff + i * phentsize)[0] for i in range(phnum)]
        self.assertNotIn(3, kinds, "no PT_INTERP: no loader, none of the set's 32-bit libraries")
        self.assertNotIn(2, kinds, "no PT_DYNAMIC: nothing is linked at run time")
        self.assertLess(len(data), MAX_BYTES)


class Manifest(unittest.TestCase):
    def setUp(self):
        self.native = json.loads(NATIVE_APPINFO.read_text())
        self.web = json.loads(WEB_APPINFO.read_text())

    def test_it_is_homes_manifest_with_a_native_type_and_the_stubs_program(self):
        self.assertEqual(self.native["id"], "com.webos.app.home")
        self.assertEqual(self.native["type"], "native")
        self.assertEqual(self.native["main"], "lumatv-homestub")
        self.assertEqual(self.native["appCategory"], ["home"], "what launchDefaultApp's home category finds")

    def test_everything_but_the_type_and_the_program_is_the_web_stubs(self):
        # the identity SAM, bootd, the wake and the saver read is the same whichever kind answers
        differ = {k for k in set(self.native) | set(self.web) if self.native.get(k) != self.web.get(k)}
        self.assertEqual(differ, {"type", "main"})

    def test_no_lifecycle_interface_is_declared_and_no_memory_is_asked_for(self):
        # a version 2 app is expected to registerApp, which needs a bus name that Home's role gives to Flutter's client only; the dev apps of the
        # set that never register (Moonlight, Punktfunk) name no version. A required memory would be reclaimed from other apps at each launch.
        for key in ("nativeLifeCycleInterfaceVersion", "requiredMemory", "handlesRelaunch"):
            self.assertNotIn(key, self.native)

    def test_it_is_never_listed_or_drawn_by_sam(self):
        self.assertIs(self.native["visible"], False)
        self.assertEqual(self.native["class"], {"hidden": True})
        self.assertIs(self.native["noSplashOnLaunch"], True)
        self.assertIs(self.native["transparent"], True)
        self.assertIs(self.native["lockable"], False)

    def test_the_icon_it_names_is_the_web_stubs(self):
        self.assertEqual(self.native["icon"], self.web["icon"])
        self.assertTrue((WEB_APPINFO.parent / self.native["icon"]).is_file())


if __name__ == "__main__":
    unittest.main()
