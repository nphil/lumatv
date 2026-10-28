#!/usr/bin/env python3
"""tv-wlprobe.py [socket-path] [seconds]: a read-only Wayland client in plain Python (no libwayland; LG's ignores WAYLAND_DEBUG), run
on the set against LG's compositor or LumaTV's. Connects, lists every global with its version, binds the seats, output, shm, webos
input manager (+ a webos seat per wl_seat), starfish pointer (+ get timer), makes a surface with a webOS shell surface (never
mapped: no buffer), and prints every event it receives, decoded where the signature is known, for N seconds. Nothing is changed on
the compositor. The run against LG's of 2026-10-28 is docs/research/compositor/lg-wlprobe-2026-10-28.txt; `gate.sh evidence` keeps
one against LumaTV's beside it.
Default socket: $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY, else /tmp/xdg/wayland-0."""
import os, socket, struct, sys, time, array

sock_path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.environ.get("XDG_RUNTIME_DIR", "/tmp/xdg"), os.environ.get("WAYLAND_DISPLAY", "wayland-0"))
seconds = float(sys.argv[2]) if len(sys.argv) > 2 else 3.0

# event signatures (per interface, by opcode); '?' nullable, letters as libwayland (i u f s o n a h)
EVENTS = {
    "wl_display": ["error:ous", "delete_id:u"],
    "wl_registry": ["global:usu", "global_remove:u"],
    "wl_callback": ["done:u"],
    "wl_seat": ["capabilities:u", "name:s"],
    "wl_keyboard": ["keymap:uhu", "enter:uoa", "leave:uo", "key:uuuu", "modifiers:uuuuu", "repeat_info:ii"],
    "wl_pointer": ["enter:uoff", "leave:uo", "motion:uff", "button:uuuu", "axis:uuf", "frame:", "axis_source:u", "axis_stop:uu", "axis_discrete:ui"],
    "wl_touch": ["down:uuoiff", "up:uui", "motion:uiff", "frame:", "cancel:", "shape:iff", "orientation:if"],
    "wl_output": ["geometry:iiiiissi", "mode:uiii", "done:", "scale:i", "name:s", "description:s"],
    "wl_shm": ["format:u"],
    "wl_surface": ["enter:o", "leave:o"],
    "wl_shell_surface": ["ping:u", "configure:uii", "popup_done:"],
    "wl_webos_shell_surface": ["state_changed:u", "position_changed:ii", "close:", "exposed:a", "state_about_to_change:u"],
    "wl_webos_input_manager": ["cursor_visibility:uo"],
    "wl_webos_seat": ["info:usuu"],
    "wl_starfish_pointer": ["timer_value:u"],
    "wl_starfish_output": ["stereoscope_hint:u"],
    "wl_webos_exported": ["window_id_assigned:su"],
    "zwp_linux_dmabuf_v1": ["format:u", "modifier:uuu"],
}

s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.connect(sock_path)
s.setblocking(False)
objects = {1: "wl_display"}
next_id = [2]
globals_seen = []
fds_in = []
log = []

def now():
    return "%7.3f" % (time.monotonic() - T0)

def new_id(iface):
    i = next_id[0]; next_id[0] += 1; objects[i] = iface; return i

def msg(obj, opcode, *args):
    body = b""
    for a in args:
        if isinstance(a, int):
            body += struct.pack("<I", a & 0xffffffff)
        elif isinstance(a, str):
            b = a.encode() + b"\0"
            body += struct.pack("<I", len(b)) + b + b"\0" * ((4 - len(b) % 4) % 4)
    size = 8 + len(body)
    s.sendall(struct.pack("<II", obj, (size << 16) | opcode) + body)

def send_sync():
    cb = new_id("wl_callback"); msg(1, 0, cb); return cb

buf = b""
def read_some(timeout):
    global buf
    import select
    r, _, _ = select.select([s], [], [], timeout)
    if not r:
        return False
    try:
        data, anc, _, _ = s.recvmsg(65536, socket.CMSG_SPACE(16 * 4))
    except BlockingIOError:
        return False
    if not data:
        raise EOFError("compositor closed the connection")
    for level, typ, d in anc:
        if level == socket.SOL_SOCKET and typ == socket.SCM_RIGHTS:
            a = array.array("i"); a.frombytes(d[: len(d) - len(d) % 4]); fds_in.extend(a.tolist())
    buf += data
    return True

def decode(iface, opcode, body):
    sigs = EVENTS.get(iface)
    if not sigs or opcode >= len(sigs):
        return None, ["0x%08x" % w for w in struct.unpack("<%dI" % (len(body) // 4), body[: len(body) // 4 * 4])]
    name, sig = sigs[opcode].split(":")
    out = []; p = 0
    for c in sig:
        if c in "iufon":
            (w,) = struct.unpack_from("<I", body, p); p += 4
            if c == "i": w = struct.unpack("<i", struct.pack("<I", w))[0]
            if c == "f": w = struct.unpack("<i", struct.pack("<I", w))[0] / 256.0
            if c == "o": w = "%s@%d" % (objects.get(w, "?"), w) if w else "null"
            out.append(w)
        elif c == "s":
            (n,) = struct.unpack_from("<I", body, p); p += 4
            out.append(body[p : p + max(n - 1, 0)].decode(errors="replace")); p += (n + 3) & ~3
        elif c == "a":
            (n,) = struct.unpack_from("<I", body, p); p += 4
            out.append(body[p : p + n].hex()); p += (n + 3) & ~3
        elif c == "h":
            out.append("fd:%s" % (fds_in.pop(0) if fds_in else "?"))
    return name, out

def dispatch(stop_on_cb=None):
    """handles every complete message in buf; returns True when stop_on_cb's done arrived"""
    global buf
    done = False
    while len(buf) >= 8:
        obj, so = struct.unpack_from("<II", buf, 0)
        size, opcode = so >> 16, so & 0xffff
        if len(buf) < size:
            break
        body = buf[8:size]; buf = buf[size:]
        iface = objects.get(obj, "?")
        name, args = decode(iface, opcode, body)
        line = "%s %s@%d.%s%s" % (now(), iface, obj, name or ("op%d" % opcode), tuple(args))
        print(line); log.append(line)
        if iface == "wl_registry" and opcode == 0:
            globals_seen.append((args[0], args[1], args[2]))
        if iface == "wl_display" and opcode == 1:
            objects.pop(args[0], None)
        if iface == "wl_callback" and obj == stop_on_cb:
            done = True
    return done

def roundtrip():
    cb = send_sync()
    t = time.monotonic()
    while time.monotonic() - t < 5:
        read_some(0.2)
        if dispatch(cb):
            return
    print("roundtrip timed out")

T0 = time.monotonic()
print("connected to", sock_path)
registry = new_id("wl_registry"); msg(1, 1, registry)
roundtrip()
print("--- %d globals ---" % len(globals_seen))
for name, iface, ver in globals_seen:
    print("  global %3d %-40s v%d" % (name, iface, ver))

def bind(iface_name, version, limit=None):
    ids = []
    for name, iface, ver in globals_seen:
        if iface == iface_name and (limit is None or len(ids) < limit):
            i = new_id(iface); msg(registry, 0, name, iface, min(version, ver), i); ids.append(i)
    return ids

compositor = bind("wl_compositor", 4)
shm = bind("wl_shm", 1)
outputs = bind("wl_output", 2)
seats = bind("wl_seat", 5)
im = bind("wl_webos_input_manager", 1)
sp = bind("wl_starfish_pointer", 1)
so = bind("wl_starfish_output", 1)
wshell = bind("wl_webos_shell", 1)
shell = bind("wl_shell", 1)
roundtrip()
print("--- seats: %d, outputs: %d, input manager: %s, starfish pointer: %s ---" % (len(seats), len(outputs), bool(im), bool(sp)))
for seat in seats:
    kb = new_id("wl_keyboard"); msg(seat, 1, kb)
    pt = new_id("wl_pointer"); msg(seat, 0, pt)
    if im:
        ws = new_id("wl_webos_seat"); msg(im[0], 1, ws, seat)
if sp:
    msg(sp[0], 1)  # get_mrcu_standby_timer -> timer_value
if compositor and wshell:
    surf = new_id("wl_surface"); msg(compositor[0], 0, surf)
    wss = new_id("wl_webos_shell_surface"); msg(wshell[0], 1, wss, surf)
    msg(wss, 2, "appId", "org.nphil.wlprobe"); msg(wss, 2, "_WEBOS_WINDOW_TYPE", "_WEBOS_WINDOW_TYPE_CARD")
    if shell:
        ss = new_id("wl_shell_surface"); msg(shell[0], 0, ss, surf); msg(ss, 3)  # set_toplevel
    msg(surf, 6)  # commit without a buffer
roundtrip()
print("--- listening %.1fs ---" % seconds)
t = time.monotonic()
while time.monotonic() - t < seconds:
    if read_some(0.2):
        dispatch()
print("--- done; objects: %s" % ", ".join("%s@%d" % (v, k) for k, v in sorted(objects.items())))
