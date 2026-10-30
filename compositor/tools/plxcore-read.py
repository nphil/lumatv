#!/usr/bin/env python3
"""Reads a PlxNative core (ARM32, from tv-plxcore.sh): the crashing thread, its frames, the proxy it marshals on, and what the window's surface is.

usage: plxcore-read.py CORE [THREADS.txt]    (THREADS.txt is the threads.N.txt next to the core: tid and thread name per line)
Reads libmali.so.0.46.0, libSDL2-2.0.so.0.14.0 and plxnative from the directory in PLX_LIBS (default /tmp/plexdeep), copies from the set
(/usr/lib, /media/developer/apps/usr/palm/applications/com.beb.plxnative), and libwayland-client from docs/research/arch/sysroot32.
"""
import bisect
import os
import struct
import sys

from elftools.elf.elffile import ELFFile

HERE = os.path.dirname(os.path.abspath(__file__))
SYSROOT = os.path.join(HERE, "..", "..", "docs", "research", "arch", "sysroot32")
COPIES = os.environ.get("PLX_LIBS", "/tmp/plexdeep")
LIBS = {
    "libmali.so.0.46.0": os.path.join(COPIES, "libmali.so.0.46.0"),
    "libSDL2-2.0.so.0.14.0": os.path.join(COPIES, "libSDL2-2.0.so.0.14.0"),
    "libwayland-client.so.0.20.0": os.path.join(SYSROOT, "libwayland-client.so.0.20.0"),
    "plxnative": os.path.join(COPIES, "plxnative"),
}


class Core:
    def __init__(self, path):
        self.f = open(path, "rb")
        self.elf = ELFFile(self.f)
        self.loads = [(s["p_vaddr"], s["p_vaddr"] + s["p_filesz"], s["p_offset"]) for s in self.elf.iter_segments() if s["p_type"] == "PT_LOAD"]
        self.threads = []  # (tid, signo, regs[18])
        self.files = []  # (start, end, offset, name)
        for seg in self.elf.iter_segments():
            if seg["p_type"] != "PT_NOTE":
                continue
            data = seg.data()
            off = 0
            while off + 12 <= len(data):
                namesz, descsz, ntype = struct.unpack_from("<III", data, off)
                off += 12
                name = data[off:off + namesz].rstrip(b"\0")
                off += (namesz + 3) & ~3
                desc = data[off:off + descsz]
                off += (descsz + 3) & ~3
                if name == b"CORE" and ntype == 1 and len(desc) >= 144:  # NT_PRSTATUS
                    signo = struct.unpack_from("<i", desc, 0)[0]
                    tid = struct.unpack_from("<i", desc, 24)[0]
                    regs = struct.unpack_from("<18I", desc, 72)
                    self.threads.append((tid, signo, regs))
                if name == b"CORE" and ntype == 0x46494C45:  # NT_FILE
                    count, page = struct.unpack_from("<II", desc, 0)
                    p = 8
                    ents = []
                    for _ in range(count):
                        ents.append(struct.unpack_from("<III", desc, p))
                        p += 12
                    for s, e, o in ents:
                        end = desc.index(b"\0", p)
                        self.files.append((s, e, o * page, desc[p:end].decode()))
                        p = end + 1

    def read(self, addr, n):
        for lo, hi, off in self.loads:
            if lo <= addr and addr + n <= hi:
                self.f.seek(off + addr - lo)
                return self.f.read(n)
        return None

    def u32(self, addr):
        b = self.read(addr, 4)
        return None if b is None else struct.unpack("<I", b)[0]

    def cstr(self, addr, n=48):
        b = self.read(addr, n) if addr else None
        if b is None:
            return None
        return b.split(b"\0")[0].decode("latin1")

    def module(self, addr):
        """(file name, offset in the file's image) of an address, from the mappings."""
        best = None
        for s, e, o, name in self.files:
            if s <= addr < e:
                best = (name.split("/")[-1], addr - s + o)
        return best


def symbolizer():
    tables = {}
    for name, path in LIBS.items():
        try:
            elf = ELFFile(open(path, "rb"))
        except OSError:
            continue
        syms = []
        for secname in (".symtab", ".dynsym"):
            sec = elf.get_section_by_name(secname)
            if sec is None:
                continue
            for s in sec.iter_symbols():
                if s["st_info"]["type"] == "STT_FUNC" and s["st_value"]:
                    syms.append((s["st_value"] & ~1, s["st_size"], s.name))
        syms.sort()
        tables[name] = (syms, [a for a, _, _ in syms])
    return tables


def sym(tables, mod):
    if mod is None or mod[0] not in tables:
        return ""
    syms, addrs = tables[mod[0]]
    # a shared object's file offset equals its vaddr for the text of these libraries; the executable is linked at 0x10000
    addr = mod[1] + (0x10000 if mod[0] == "plxnative" else 0)
    i = bisect.bisect_right(addrs, addr) - 1
    if i < 0:
        return ""
    a, sz, name = syms[i]
    if sz and addr - a >= sz:
        return f"(after {name}+{addr - a:#x}, not a named function)"
    return f"{name}+{addr - a:#x}"


def main():
    core = Core(sys.argv[1])
    names = {}
    if len(sys.argv) > 2:
        for line in open(sys.argv[2]):
            tid, _, comm = line.strip().partition(" ")
            names[int(tid)] = comm
    tables = symbolizer()
    print("threads:", len(core.threads))
    crash = None
    for tid, signo, regs in core.threads:
        mark = ""
        if signo == 11:
            crash = (tid, regs)
            mark = "  <== SIGSEGV"
        pc = regs[15]
        mod = core.module(pc)
        print(f"  tid {tid} {names.get(tid, '?'):16s} signal {signo:2d} pc {pc:#010x} {mod[0] if mod else '?'}+{mod[1]:#x} {sym(tables, mod)}{mark}" if mod else
              f"  tid {tid} {names.get(tid, '?'):16s} signal {signo:2d} pc {pc:#010x}{mark}")
    if crash is None:
        print("no SIGSEGV thread")
        return
    tid, r = crash
    sp, pc, lr = r[13], r[15], r[14]
    print(f"\ncrashing thread {tid} ({names.get(tid, '?')}): pc {pc:#x} lr {lr:#x} sp {sp:#x}  r0-r12 {[hex(x) for x in r[:13]]}")
    # frame of wl_argument_from_va_list: push {r4-r7, lr} then sub sp, 0xc  => saved r4..r7, lr at sp+0xc
    saved = [core.u32(sp + 0xC + 4 * i) for i in range(5)]
    opcode, proxy, args, iface_param, ret = saved
    print(f"marshal_flags state: opcode {opcode}, proxy {proxy:#x}, interface arg {iface_param:#x}, return {ret:#x}")

    def dump_proxy(label, p):
        raw = core.read(p, 0x2C) if p else None
        if raw is None:
            print(f"  {label} {p:#x}: unreadable")
            return
        w = struct.unpack("<11I", raw)
        itf = w[0]
        name = core.cstr(core.u32(itf) if itf and core.u32(itf) else 0) if itf else None
        print(f"  {label} {p:#x}: words {[hex(x) for x in w]}")
        print(f"     object.interface {itf:#x} -> name {name!r}; id {w[2]}; display {w[3]:#x}; queue {w[4]:#x}; flags {w[5]:#x}; refcount {w[6]}; version {w[9]}")

    dump_proxy("the proxy", proxy)
    # wl_proxy_marshal_flags frame: saved r4,r5,r6,r7,r8,sb,lr of its caller at sp+0x20+0x64
    base = sp + 0x20 + 0x64
    cr = [core.u32(base + 4 * i) for i in range(7)]
    print("caller's saved r4 r5 r6 r7 r8 sb lr:", [hex(x) if x is not None else None for x in cr])
    callmod = core.module(cr[6] & ~1) if cr[6] else None
    print("  return into:", callmod, sym(tables, callmod))
    ws = cr[0]
    if ws:
        raw = core.read(ws, 0x60)
        if raw:
            w = struct.unpack("<24I", raw)
            print(f"  display_window_buffer's r4 (window_surface) {ws:#x}: window {w[0]:#x}, queue_a {w[1]:#x}, queue_b {w[2]:#x}, frame_callback {w[3]:#x}, sync {w[4]:#x}, swap_interval {w[22]}")
            win = w[0]
            wraw = core.read(win, 0x2C) if win else None
            if wraw:
                ww = struct.unpack("<11I", wraw)
                print(f"  window (wl_egl_window) {win:#x}: version {ww[0]:#x} size {ww[1]}x{ww[2]} attached {ww[5]}x{ww[6]} driver_private {ww[7]:#x} resize {ww[8]:#x} destroy_cb {ww[9]:#x} surface {ww[10]:#x}")
                dump_proxy("window->surface", ww[10])
            else:
                print(f"  window {win:#x}: not in the core")
    # stack scan of the crashing thread: words that point into executable mappings
    print("\nstack scan (return addresses):")
    seen = 0
    for i in range(0, 0x600, 4):
        v = core.u32(sp + i)
        if v is None:
            break
        m = core.module(v & ~1)
        if m and m[0] in tables:
            s = sym(tables, m)
            if s:
                print(f"  sp+{i:#05x} {v:#010x} {m[0]} {s}")
                seen += 1
                if seen > 40:
                    break


if __name__ == "__main__":
    main()
