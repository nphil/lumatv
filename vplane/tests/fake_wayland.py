"""A small Wayland server for the player's host tests: just enough of the core protocol and of LG's webOS protocols (the tables in
compositor/protocol/lg-tables.txt) to take one window with an exported video element, to keep every request in order, and to let a
test read the window's pixels, move the window, close it or break the connection.

It is a second reading of the wire by the same author as the client, so it proves the player's behaviour (what it asks for and in what
order, what it draws, what it does when the compositor closes the window), not that LG's compositor agrees. The Rust compositor's own
harness runs the same client against the real server library (compositor/crates/lumatv-compositor/tests/vplane_host.rs).
"""
import mmap
import os
import socket
import struct
import tempfile
import threading
import time

# (interface, opcode) -> request name and its argument letters; the letters are libwayland's (u i s o n a h = fd)
REQUESTS = {
    ("wl_display", 0): ("sync", "n"), ("wl_display", 1): ("get_registry", "n"),
    ("wl_registry", 0): ("bind", "usun"),
    ("wl_compositor", 0): ("create_surface", "n"), ("wl_compositor", 1): ("create_region", "n"),
    ("wl_region", 0): ("destroy", ""), ("wl_region", 1): ("add", "iiii"), ("wl_region", 2): ("subtract", "iiii"),
    ("wl_shm", 0): ("create_pool", "nhi"),
    ("wl_shm_pool", 0): ("create_buffer", "niiiiu"), ("wl_shm_pool", 1): ("destroy", ""), ("wl_shm_pool", 2): ("resize", "i"),
    ("wl_buffer", 0): ("destroy", ""),
    ("wl_surface", 0): ("destroy", ""), ("wl_surface", 1): ("attach", "oii"), ("wl_surface", 2): ("damage", "iiii"),
    ("wl_surface", 3): ("frame", "n"), ("wl_surface", 4): ("set_opaque_region", "o"), ("wl_surface", 5): ("set_input_region", "o"),
    ("wl_surface", 6): ("commit", ""), ("wl_surface", 7): ("set_buffer_transform", "i"), ("wl_surface", 8): ("set_buffer_scale", "i"),
    ("wl_webos_shell", 0): ("get_system_pip", "n"), ("wl_webos_shell", 1): ("get_shell_surface", "no"),
    ("wl_webos_shell_surface", 0): ("set_location_hint", "u"), ("wl_webos_shell_surface", 1): ("set_state", "u"),
    ("wl_webos_shell_surface", 2): ("set_property", "ss"), ("wl_webos_shell_surface", 3): ("set_key_mask", "u"),
    ("wl_webos_shell_surface", 4): ("set_size", "ii"),
    ("wl_webos_foreign", 0): ("destroy", ""), ("wl_webos_foreign", 1): ("export_element", "nou"), ("wl_webos_foreign", 2): ("import_element", "nsu"),
    ("wl_webos_exported", 0): ("destroy", ""), ("wl_webos_exported", 1): ("set_exported_window", "oo"),
    ("wl_webos_exported", 2): ("set_crop_region", "ooo"), ("wl_webos_exported", 3): ("set_property", "ss"),
}

GLOBALS = [("wl_compositor", 3), ("wl_shm", 1), ("wl_webos_shell", 1), ("wl_webos_foreign", 1), ("wl_subcompositor", 1)]


class Conn:
    def __init__(self, server, sock):
        self.server, self.sock = server, sock
        self.objects = {1: "wl_display"}
        self.buf = b""
        self.fds = []
        self.regions = {}                  # id -> list of rects
        self.pools = {}                    # id -> (mmap, size)
        self.buffers = {}                  # id -> (pool id, offset, w, h, stride)
        self.surfaces = {}                 # id -> {"buffer": id, "committed": id}
        self.window_id = None
        self.exported_surface = {}
        self.alive = True

    def send(self, obj, op, payload=b""):
        try:
            self.sock.sendall(struct.pack("<II", obj, (8 + len(payload)) << 16 | op) + payload)
        except OSError:
            self.alive = False              # the client has gone: a reply to it has nowhere to land

    @staticmethod
    def string(s):
        raw = s.encode() + b"\0"
        return struct.pack("<I", len(raw)) + raw + b"\0" * (-len(raw) % 4)

    def read_string(self, data, pos):
        (n,) = struct.unpack_from("<I", data, pos)
        s = data[pos + 4:pos + 4 + n - 1].decode()
        return s, pos + 4 + ((n + 3) & ~3)

    def parse(self, iface, letters, data):
        pos, out = 0, []
        for ch in letters:
            if ch in "uio":
                (v,) = struct.unpack_from("<i" if ch == "i" else "<I", data, pos)
                pos += 4
                out.append(v)
            elif ch == "n":
                (v,) = struct.unpack_from("<I", data, pos)
                pos += 4
                out.append(v)
            elif ch == "s":
                v, pos = self.read_string(data, pos)
                out.append(v)
            elif ch == "h":
                out.append(self.fds.pop(0))
        return out

    def handle(self, obj, op, data):
        iface = self.objects.get(obj)
        if iface is None:
            self.server.note("error", f"request on unknown object {obj}")
            return
        spec = REQUESTS.get((iface, op))
        if spec is None:
            self.server.note("unknown", f"{iface}#{obj} opcode {op}")
            return
        name, letters = spec
        if iface == "wl_registry" and name == "bind":      # name, interface, version, new_id
            (gname,) = struct.unpack_from("<I", data, 0)
            s, pos = self.read_string(data, 4)
            ver, nid = struct.unpack_from("<II", data, pos)
            self.objects[nid] = s
            self.server.note("bind", f"{s} v{ver} as {nid}")
            return
        args = self.parse(iface, letters, data)
        self.server.note(f"{iface}.{name}", args)
        if iface == "wl_display" and name == "sync":
            self.objects[args[0]] = "wl_callback"
            self.send(args[0], 0, struct.pack("<I", 1))
            self.send(1, 1, struct.pack("<I", args[0]))
        elif iface == "wl_display" and name == "get_registry":
            self.objects[args[0]] = "wl_registry"
            for i, (g, v) in enumerate(GLOBALS, 1):
                self.send(args[0], 0, struct.pack("<I", i) + self.string(g) + struct.pack("<I", v))
        elif iface == "wl_compositor" and name == "create_surface":
            self.objects[args[0]] = "wl_surface"
            self.surfaces[args[0]] = {"buffer": None, "committed": None, "props": {}}
        elif iface == "wl_compositor" and name == "create_region":
            self.objects[args[0]] = "wl_region"
            self.regions[args[0]] = []
        elif iface == "wl_region" and name == "add":
            self.regions[obj].append(tuple(args))
        elif iface == "wl_shm" and name == "create_pool":
            nid, fd, size = args
            self.objects[nid] = "wl_shm_pool"
            self.pools[nid] = (mmap.mmap(fd, size, prot=mmap.PROT_READ), size)
            os.close(fd)
        elif iface == "wl_shm_pool" and name == "create_buffer":
            nid, off, w, h, stride, fmt = args
            self.objects[nid] = "wl_buffer"
            self.buffers[nid] = (obj, off, w, h, stride)
        elif iface == "wl_surface" and name == "attach":
            self.surfaces[obj]["buffer"] = args[0]
        elif iface == "wl_surface" and name == "commit":
            s = self.surfaces[obj]
            s["committed"] = s["buffer"]
            self.server.commit(self, obj)
        elif iface == "wl_webos_shell" and name == "get_shell_surface":
            self.objects[args[0]] = "wl_webos_shell_surface"
            self.server.shell_surface = (self, args[0], args[1])
        elif iface == "wl_webos_shell_surface" and name == "set_property":
            self.server.props[args[0]] = args[1]
        elif iface == "wl_webos_shell_surface" and name == "set_state":
            self.server.state = args[0]
        elif iface == "wl_webos_foreign" and name == "export_element":
            nid, surface, etype = args
            self.objects[nid] = "wl_webos_exported"
            self.server.export_count += 1
            wid = "_Window_Id_%d" % self.server.export_count
            self.window_id = wid
            self.exported_surface[nid] = surface
            if not self.server.hold_window_id:
                self.send(nid, 0, self.string(wid) + struct.pack("<I", etype))
        elif iface == "wl_webos_exported" and name == "set_exported_window":
            src = self.regions.get(args[0], [])
            dst = self.regions.get(args[1], [])
            self.server.video = {"src": src[0] if src else None, "dst": dst[0] if dst else None}
            self.server.video_history.append(dict(self.server.video))

    def pump(self):
        try:
            data, fds, _, _ = socket.recv_fds(self.sock, 65536, 8)
        except OSError:
            return False
        if not data and not fds:
            return False
        self.fds += fds
        self.buf += data
        while len(self.buf) >= 8:
            obj, so = struct.unpack_from("<II", self.buf, 0)
            size, op = so >> 16, so & 0xFFFF
            if len(self.buf) < size:
                break
            body = self.buf[8:size]
            self.buf = self.buf[size:]
            try:
                self.handle(obj, op, body)
            except Exception as e:           # a bad request must not end the server: the test reads what was noted
                self.server.note("server-error", repr(e))
        return True

    def pixels(self, surface_id):
        """The committed buffer of a surface as bytes (width, height, data)."""
        s = self.surfaces.get(surface_id)
        if not s or s["committed"] is None:
            return None
        pool, off, w, h, stride = self.buffers[s["committed"]]
        m, _ = self.pools[pool]
        return w, h, bytes(m[off:off + stride * h])


class FakeCompositor:
    def __init__(self, hold_window_id=False):
        self.dir = tempfile.mkdtemp(prefix="vplane-wl-")
        self.path = os.path.join(self.dir, "wayland-test")
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(self.path)
        self.listener.listen(4)
        self.listener.settimeout(0.1)
        self.lock = threading.Lock()
        self.events = []
        self.props = {}
        self.state = None
        self.shell_surface = None
        self.export_count = 0
        self.video = None
        self.video_history = []
        self.hold_window_id = hold_window_id
        self.conns = []
        self.commits = []
        self.running = True
        self.thread = threading.Thread(target=self.loop, daemon=True)
        self.thread.start()

    def note(self, what, args):
        with self.lock:
            self.events.append((time.monotonic(), what, args))

    def commit(self, conn, surface):
        got = conn.pixels(surface)
        with self.lock:
            self.commits.append((time.monotonic(), surface, got))

    def env(self):
        return {"XDG_RUNTIME_DIR": self.dir, "WAYLAND_DISPLAY": "wayland-test"}

    def loop(self):
        import select
        while self.running:
            try:
                s, _ = self.listener.accept()
                self.conns.append(Conn(self, s))
            except (socket.timeout, OSError):
                pass
            socks = [c.sock for c in self.conns if c.alive]
            if not socks:
                continue
            r, _, _ = select.select(socks, [], [], 0.02)
            for c in list(self.conns):
                if c.sock in r and not c.pump():
                    c.alive = False
                    self.note("disconnect", None)

    # ---- what a test asks
    def requests(self):
        with self.lock:
            return [(w, a) for _, w, a in self.events]

    def names(self):
        return [w for w, _ in self.requests()]

    def wait_for(self, what, timeout=5.0):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            if what in self.names():
                return True
            time.sleep(0.01)
        return False

    def settle(self, quiet=0.1, timeout=3.0):
        """Waits until the server has handled everything the client has sent (no new request for `quiet` seconds)."""
        end = time.monotonic() + timeout
        last, since = -1, time.monotonic()
        while time.monotonic() < end:
            with self.lock:
                n = len(self.events)
            if n != last:
                last, since = n, time.monotonic()
            elif time.monotonic() - since >= quiet:
                return
            time.sleep(0.01)

    def count(self, what):
        return self.names().count(what)

    def last_commit(self):
        with self.lock:
            return self.commits[-1] if self.commits else None

    def alive(self):
        return any(c.alive for c in self.conns)

    def send_close(self):
        conn, sid, _ = self.shell_surface
        conn.send(sid, 2)

    def break_protocol(self):
        conn, sid, _ = self.shell_surface
        conn.send(1, 0, struct.pack("<II", sid, 7) + Conn.string("test error"))

    def drop(self):
        for c in self.conns:
            try:
                c.sock.shutdown(socket.SHUT_RDWR)
                c.sock.close()
            except OSError:
                pass

    def release_window_id(self):
        for c in self.conns:
            for nid, _ in c.exported_surface.items():
                c.send(nid, 0, Conn.string(c.window_id) + struct.pack("<I", 0))

    def close(self):
        self.running = False
        self.thread.join(2)
        self.drop()
        try:
            self.listener.close()
            os.unlink(self.path)
            os.rmdir(self.dir)
        except OSError:
            pass


def argb(data, w, x, y):
    """The (a, r, g, b) of one pixel of a premultiplied ARGB8888 buffer."""
    (v,) = struct.unpack_from("<I", data, (y * w + x) * 4)
    return (v >> 24, (v >> 16) & 255, (v >> 8) & 255, v & 255)
