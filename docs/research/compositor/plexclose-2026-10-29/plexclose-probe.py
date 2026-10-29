#!/usr/bin/env python3
"""plexclose-probe.py [socket-path]: a plain-Python Wayland client (no libwayland) that measures what a compositor does
with a surface's frame callbacks: commit->done latency on a full-screen card (phase A), and what arrives for a callback
whose surface is destroyed right after the commit (phases B, C): `done`, `delete_id`, or nothing. One run takes ~6 s.
Default socket: $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY, else /tmp/xdg/wayland-0. Prints one line per event with ms stamps."""
import os, socket, struct, sys, time, array, select, tempfile

sock_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp/xdg"), os.environ.get("WAYLAND_DISPLAY", "wayland-0"))
APPID = os.environ.get("PROBE_APPID", "org.nphil.lumatv.x1")
WTYPE = os.environ.get("PROBE_WTYPE", "_WEBOS_WINDOW_TYPE_CARD")
W, H = 1920, 1080
EVENTS = {
    "wl_display": ["error:ous", "delete_id:u"],
    "wl_registry": ["global:usu", "global_remove:u"],
    "wl_callback": ["done:u"],
    "wl_buffer": ["release:"],
    "wl_shm": ["format:u"],
    "wl_surface": ["enter:o", "leave:o"],
    "wl_shell_surface": ["ping:u", "configure:uii", "popup_done:"],
    "wl_webos_shell_surface": ["state_changed:u", "position_changed:ii", "close:", "exposed:a", "state_about_to_change:u"],
}
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sock_path)
s.setblocking(False)
T0 = time.monotonic()
objects = {1: "wl_display"}
next_id = [2]
globals_seen = []
buf = b""
log = []

def now():
    return "%8.3f" % ((time.monotonic() - T0) * 1000.0)

def new_id(iface):
    i = next_id[0]; next_id[0] += 1; objects[i] = iface; return i

def pack(obj, opcode, *args):
    body = b""
    for a in args:
        if isinstance(a, int):
            body += struct.pack("<I", a & 0xffffffff)
        elif isinstance(a, str):
            b = a.encode() + b"\0"
            body += struct.pack("<I", len(b)) + b + b"\0" * ((4 - len(b) % 4) % 4)
    size = 8 + len(body)
    return struct.pack("<II", obj, (size << 16) | opcode) + body

def msg(obj, opcode, *args):
    s.sendall(pack(obj, opcode, *args))

def msg_fd(fd, obj, opcode, *args):
    s.sendmsg([pack(obj, opcode, *args)], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", [fd]).tobytes())])

def read_some(timeout):
    global buf
    r, _, _ = select.select([s], [], [], timeout)
    if not r:
        return False
    try:
        data, anc, _, _ = s.recvmsg(65536, socket.CMSG_SPACE(16 * 4))
    except BlockingIOError:
        return False
    if not data:
        raise EOFError("compositor closed the connection")
    buf += data
    return True

def decode(iface, opcode, body):
    sigs = EVENTS.get(iface)
    if not sigs or opcode >= len(sigs):
        return None, (body.hex(),)
    name, sig = sigs[opcode].split(":")
    out = []; p = 0
    for c in sig:
        if c in "uoin":
            out.append(struct.unpack_from("<I", body, p)[0]); p += 4
        elif c == "i":
            out.append(struct.unpack_from("<i", body, p)[0]); p += 4
        elif c == "f":
            out.append(struct.unpack_from("<i", body, p)[0] / 256.0); p += 4
        elif c == "s":
            n = struct.unpack_from("<I", body, p)[0]; p += 4
            out.append(body[p:p + n - 1].decode(errors="replace")); p += (n + 3) & ~3
        elif c == "a":
            n = struct.unpack_from("<I", body, p)[0]; p += 4
            out.append("array[%d]" % n); p += (n + 3) & ~3
        elif c == "h":
            out.append("fd")
    return name, out

seen = []   # (ms, iface, obj, name, args)
def dispatch():
    global buf
    while len(buf) >= 8:
        obj, so = struct.unpack_from("<II", buf, 0)
        size, opcode = so >> 16, so & 0xffff
        if len(buf) < size:
            break
        body = buf[8:size]; buf = buf[size:]
        iface = objects.get(obj, "?")
        name, args = decode(iface, opcode, body)
        t = (time.monotonic() - T0) * 1000.0
        seen.append((t, iface, obj, name, args))
        line = "%s %s@%d.%s%s" % (now(), iface, obj, name or ("op%d" % opcode), tuple(args))
        print(line, flush=True); log.append(line)
        if iface == "wl_registry" and opcode == 0:
            globals_seen.append((args[0], args[1], args[2]))
        if iface == "wl_display" and opcode == 1:
            objects.pop(args[0], None)

def pump(seconds):
    t = time.monotonic()
    while time.monotonic() - t < seconds:
        if read_some(min(0.05, max(0.0, seconds - (time.monotonic() - t)))):
            dispatch()

def wait_for(pred, seconds):
    t = time.monotonic()
    while time.monotonic() - t < seconds:
        read_some(0.01); dispatch()
        if pred():
            return True
    return False

def roundtrip():
    cb = new_id("wl_callback"); msg(1, 0, cb)
    ok = wait_for(lambda: any(o == cb and n == "done" for _, _, o, n, _ in seen), 5)
    if not ok:
        print("roundtrip timed out", flush=True)

print("connected to", sock_path, "appId", APPID, flush=True)
registry = new_id("wl_registry"); msg(1, 1, registry)
roundtrip()
def bind(iface_name, version):
    for name, iface, ver in globals_seen:
        if iface == iface_name:
            i = new_id(iface); msg(registry, 0, name, iface, min(version, ver), i); return i
    return None
compositor = bind("wl_compositor", 4)
shm = bind("wl_shm", 1)
wshell = bind("wl_webos_shell", 1)
shell = bind("wl_shell", 1)
roundtrip()
print("globals: compositor %s shm %s webos_shell %s shell %s" % (compositor, shm, wshell, shell), flush=True)

# one shm pool, three buffers (as a Mali swapchain has)
stride = W * 4
size = stride * H
fd = -1
try:
    fd = os.memfd_create("probe")
except Exception:
    f = tempfile.TemporaryFile(dir="/tmp"); fd = os.dup(f.fileno())
os.ftruncate(fd, size * 3)
pool = new_id("wl_shm_pool"); msg_fd(fd, shm, 0, pool, size * 3)
buffers = []
for i in range(3):
    b = new_id("wl_buffer"); msg(pool, 0, b, i * size, W, H, stride, 1)  # 1 = XRGB8888
    buffers.append(b)
roundtrip()

def card(tag):
    surf = new_id("wl_surface"); msg(compositor, 0, surf)
    wss = new_id("wl_webos_shell_surface"); msg(wshell, 1, wss, surf)
    msg(wss, 2, "appId", APPID); msg(wss, 2, "_WEBOS_WINDOW_TYPE", WTYPE); msg(wss, 2, "title", tag)
    if shell:
        ss = new_id("wl_shell_surface"); msg(shell, 0, ss, surf); msg(ss, 3)  # set_toplevel
    msg(wss, 1, 3)  # set_state fullscreen
    return surf, wss

def present(surf, b):
    """attach, damage, frame, commit; returns the callback id"""
    cb = new_id("wl_callback")
    msg(surf, 1, b, 0, 0)          # attach
    msg(surf, 2, 0, 0, W, H)       # damage
    msg(surf, 3, cb)               # frame
    msg(surf, 6)                   # commit
    return cb

def fate(cb, seconds):
    """what arrived for callback cb within `seconds`: ('done', ms) or ('delete_id', ms) or ('nothing', None)"""
    t0 = time.monotonic()
    while time.monotonic() - t0 < seconds:
        read_some(0.01); dispatch()
        for t, iface, obj, name, args in seen:
            if iface == "wl_callback" and obj == cb and name == "done":
                return ("done", t)
            if iface == "wl_display" and name == "delete_id" and args[0] == cb:
                return ("delete_id", t)
    return ("nothing", None)

# ---- phase A: commit->done latency on a mapped full-screen card, at a few commit phases ----
print("--- phase A: latency commit->done on a mapped card (30 commits) ---", flush=True)
surf, wss = card("probe-A")
cb = present(surf, buffers[0])
wait_for(lambda: any(o == cb and n == "done" for _, _, o, n, _ in seen), 2)
pump(0.3)
lat = []
for i in range(30):
    time.sleep((i % 7) * 0.0025)   # spread the commit over the refresh
    b = buffers[(i + 1) % 3]
    ts = (time.monotonic() - T0) * 1000.0
    cb = present(surf, b)
    what, t = fate(cb, 1.5)
    lat.append((what, None if t is None else round(t - ts, 2)))
print("A latency ms (commit->done):", lat, flush=True)
got = [l for w, l in lat if w == "done"]
if got:
    print("A summary: done %d/30, min %.2f max %.2f mean %.2f" % (len(got), min(got), max(got), sum(got) / len(got)), flush=True)

# ---- phase B: commit+frame then destroy the surface in the same flush (Plex's close while a callback is pending) ----
print("--- phase B: commit+frame, then wl_surface.destroy at once ---", flush=True)
msg(surf, 0)   # destroy phase A's surface first
roundtrip(); pump(0.5)
surf, wss = card("probe-B")
cb0 = present(surf, buffers[0])
wait_for(lambda: any(o == cb0 and n == "done" for _, _, o, n, _ in seen), 2)
pump(0.3)
ts = (time.monotonic() - T0) * 1000.0
cb = present(surf, buffers[1])
msg(surf, 0)   # wl_surface.destroy, same flush
what, t = fate(cb, 1.5)
print("B: callback %d after destroy -> %s at +%s ms" % (cb, what, None if t is None else round(t - ts, 2)), flush=True)
pump(0.3)

# ---- phase C: commit+frame, 3 ms later destroy (SDL's order: the commit lands, the compositor may or may not have drawn) ----
print("--- phase C: commit+frame, 3 ms, then destroy ---", flush=True)
surf, wss = card("probe-C")
cb0 = present(surf, buffers[2])
wait_for(lambda: any(o == cb0 and n == "done" for _, _, o, n, _ in seen), 2)
pump(0.3)
ts = (time.monotonic() - T0) * 1000.0
cb = present(surf, buffers[0])
time.sleep(0.003)
msg(surf, 0)
what, t = fate(cb, 1.5)
print("C: callback %d after destroy(+3ms) -> %s at +%s ms" % (cb, what, None if t is None else round(t - ts, 2)), flush=True)
pump(0.3)

# ---- phase D: two commits back to back, the second's callback pending behind the first's, then destroy ----
print("--- phase D: commit+frame, commit+frame, destroy ---", flush=True)
surf, wss = card("probe-D")
cb0 = present(surf, buffers[1])
wait_for(lambda: any(o == cb0 and n == "done" for _, _, o, n, _ in seen), 2)
pump(0.3)
ts = (time.monotonic() - T0) * 1000.0
cb1 = present(surf, buffers[2])
cb2 = present(surf, buffers[0])
msg(surf, 0)
what1, t1 = fate(cb1, 1.5)
what2, t2 = fate(cb2, 0.5)
print("D: first %s at +%s ms, second %s at +%s ms" % (what1, None if t1 is None else round(t1 - ts, 2), what2, None if t2 is None else round(t2 - ts, 2)), flush=True)
pump(0.3)
print("--- done; %d events seen ---" % len(seen), flush=True)
