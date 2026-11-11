#!/usr/bin/env python3
"""Builds a repaired copy of LG's Luna binding for node (`/usr/lib/nodejs/webos-sysbus.node`, webOS 10.2.1,
node 16.20.2) with the `palmbus.js` that loads it. Only the known files are accepted and the result is checked
by hash.

    sysbus_fix.py build SRC_DIR OUT_DIR    SRC_DIR holds LG's webos-sysbus.node and palmbus.js
    sysbus_fix.py verify OUT_DIR           exit 0 when OUT_DIR is exactly what build writes

The binding polls GLib's descriptors through libuv and passed only READABLE and WRITABLE on. libuv 1.43 reports
a socket in error as status UV_EBADF with no events, and luna-service2 asks for G_IO_IN alone, so such a
connection is never read again and never closed, while epoll reports it on every turn of the loop.

Measured on the TV: `ss.apiadapter` held ten Luna sockets without a peer; epoll reported exactly those on every
turn and nothing read them, at 21 to 23 % of one core (about 650 turns a second, 160 epoll_ctl calls each) with
no bus traffic at all. Another service on the same module, with no such socket, idles at 0.06 %. That the ten
were in the error state, and how their peers went, is inferred from those traces and libuv's source, not
observed; so is luna-service2 closing a connection once its read fails.
"""
import hashlib
import json
import os
import struct
import sys

MODULE = "webos-sysbus.node"
SOURCE = "cec11e7882477112a515523b3f42a4169aa7a9ce0c5faeab1d1e722903cf3686"
FIXED = "e8b2a6719af992df973ab243da11e4a0d49e6dd861e9a61677cacb45ac750717"
# Copied, not linked: it requires `./webos-sysbus.node` and node resolves a link back to /usr/lib/nodejs.
LOADER = "palmbus.js"
LOADER_SHA = "072a5e7a37db2f4ef858c0665c4af829103122bb45012ed1cb8749c600c78f41"
MANIFEST = "manifest.json"

# (file offset, new bytes); both keep their length.
PATCHES = (
    # Poll callback, the mask applied to each GPollFD's requested events. READABLE and WRITABLE map to G_IO_IN
    # and G_IO_OUT as before; no events at all (libuv's error report) now stands for every requested condition
    # plus G_IO_ERR and G_IO_HUP, which is what poll() answers for such a socket, so the owner is told to read
    # it and meets the error.
    #   orr.w r4, r4, r4, lsl #1 ; ands r4, r4, #5 ; it eq ; moveq r4, #0x1d
    (0x7CCE, bytes.fromhex("44ea440414f0050408bf1d24")),
    # GLib's timeout and uv_timer_start are both in milliseconds; the factor was 0.001 (libev took seconds), so
    # every GLib timer under a second became 0 ms.
    (0x8090, struct.pack("<d", 1.0)),
)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def read(path):
    with open(path, "rb") as f:
        return f.read()


def fix(data):
    if sha(data) != SOURCE:
        raise ValueError("not the %s this patch was made for" % MODULE)
    out = bytearray(data)
    for offset, new in PATCHES:
        out[offset:offset + len(new)] = new
    if sha(out) != FIXED:
        raise ValueError("the patched %s does not have the expected hash" % MODULE)
    return bytes(out)


def manifest():
    text = json.dumps({"source": {MODULE: SOURCE, LOADER: LOADER_SHA}, "files": {MODULE: FIXED, LOADER: LOADER_SHA}},
                      indent=1, sort_keys=True)
    return (text + "\n").encode()


def build(src, out):
    loader = read(os.path.join(src, LOADER))
    if sha(loader) != LOADER_SHA:
        raise ValueError("not the %s this patch was made for" % LOADER)
    files = ((MODULE, fix(read(os.path.join(src, MODULE)))), (LOADER, loader), (MANIFEST, manifest()))
    os.makedirs(out, exist_ok=True)
    for name, data in files:
        tmp = os.path.join(out, name + ".tmp")
        with open(tmp, "wb") as f:
            f.write(data)
        os.chmod(tmp, 0o755 if name == MODULE else 0o644)
        os.replace(tmp, os.path.join(out, name))


def verify(out):
    for name, want in ((MODULE, FIXED), (LOADER, LOADER_SHA), (MANIFEST, sha(manifest()))):
        if sha(read(os.path.join(out, name))) != want:
            raise ValueError("%s is not what build writes" % name)


def main(argv):
    try:
        if len(argv) == 4 and argv[1] == "build":
            build(argv[2], argv[3])
        elif len(argv) == 3 and argv[1] == "verify":
            verify(argv[2])
        else:
            sys.exit("usage: sysbus_fix.py build SRC_DIR OUT_DIR | verify OUT_DIR")
    except (OSError, ValueError) as e:
        sys.exit("sysbus_fix: %s" % e)


if __name__ == "__main__":
    main(sys.argv)
