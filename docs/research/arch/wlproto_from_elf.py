#!/usr/bin/env python3
"""Recover Wayland protocol tables (struct wl_interface / wl_message) from an
ELF shared object that exports them, e.g. mali_buffer_sharing_interface in
libmali. Reads only the headers, .dynsym/.dynstr and the referenced bytes
(a few hundred KB), never the whole file. ELF32 little-endian (REL, in-place
addends) and ELF64 (RELA: pointers in the file may be zero, then the
relocation addends are used).

Usage: wlproto_from_elf.py <lib.so> <symbol>... (default: every *_interface
symbol whose name does not start with wl_)
"""
import struct, sys

class Elf:
    def __init__(self, path):
        self.f = open(path, 'rb')
        ident = self.f.read(16)
        assert ident[:4] == b'\x7fELF'
        self.cls = ident[4]  # 1 = ELF32, 2 = ELF64
        self.f.seek(0)
        if self.cls == 1:
            hdr = struct.unpack('<16sHHIIIIIHHHHHH', self.f.read(52))
            (_, _, _, _, _, self.phoff, self.shoff, _, _, self.phentsize, self.phnum, self.shentsize, self.shnum, self.shstrndx) = hdr
            self.ptr = 4; self.pfmt = '<I'
        else:
            hdr = struct.unpack('<16sHHIQQQIHHHHHH', self.f.read(64))
            (_, _, _, _, _, self.phoff, self.shoff, _, _, self.phentsize, self.phnum, self.shentsize, self.shnum, self.shstrndx) = hdr
            self.ptr = 8; self.pfmt = '<Q'
        self.loads = []
        for i in range(self.phnum):
            self.f.seek(self.phoff + i * self.phentsize)
            if self.cls == 1:
                p_type, p_offset, p_vaddr, _, p_filesz, p_memsz, _, _ = struct.unpack('<IIIIIIII', self.f.read(32))
            else:
                p_type, _, p_offset, p_vaddr, _, p_filesz, p_memsz, _ = struct.unpack('<IIQQQQQQ', self.f.read(56))
            if p_type == 1:
                self.loads.append((p_vaddr, p_offset, p_filesz))
        self.sections = {}
        shdrs = []
        for i in range(self.shnum):
            self.f.seek(self.shoff + i * self.shentsize)
            if self.cls == 1:
                sh = struct.unpack('<IIIIIIIIII', self.f.read(40))
                name, typ, flags, addr, off, size, link, info, align, entsize = sh
            else:
                sh = struct.unpack('<IIQQQQIIQQ', self.f.read(64))
                name, typ, flags, addr, off, size, link, info, align, entsize = sh
            shdrs.append((name, typ, addr, off, size, link, entsize))
        shstr = shdrs[self.shstrndx]
        self.f.seek(shstr[3]); strtab = self.f.read(shstr[4])
        for name, typ, addr, off, size, link, entsize in shdrs:
            n = strtab[name:strtab.index(b'\0', name)].decode()
            self.sections[n] = (typ, addr, off, size, link, entsize)
        self.rela = {}
        for n, (typ, addr, off, size, link, entsize) in self.sections.items():
            if typ == 4 and self.cls == 2:  # SHT_RELA, collect R_*_RELATIVE addends by offset
                self.f.seek(off); data = self.f.read(size)
                for k in range(0, size, 24):
                    r_off, r_info, r_add = struct.unpack('<QQq', data[k:k+24])
                    if r_info & 0xffffffff == 1027:  # R_AARCH64_RELATIVE
                        self.rela[r_off] = r_add

    def off(self, vaddr):
        for va, off, fsz in self.loads:
            if va <= vaddr < va + fsz:
                return off + (vaddr - va)
        raise ValueError('vaddr 0x%x not in a PT_LOAD' % vaddr)

    def read(self, vaddr, n):
        self.f.seek(self.off(vaddr)); return self.f.read(n)

    def ptr_at(self, vaddr):
        v = struct.unpack(self.pfmt, self.read(vaddr, self.ptr))[0]
        if v == 0 and vaddr in self.rela:
            v = self.rela[vaddr]
        return v

    def cstr(self, vaddr):
        if not vaddr: return None
        out = b''
        while True:
            chunk = self.read(vaddr + len(out), 64)
            if b'\0' in chunk:
                out += chunk[:chunk.index(b'\0')]; return out.decode('latin1')
            out += chunk

    def symbols(self, secname):
        if secname not in self.sections: return
        typ, addr, off, size, link, entsize = self.sections[secname]
        # .dynsym -> .dynstr, .symtab -> .strtab
        styp, saddr, soff, ssize, slink, sentsize = self.sections['.dynstr' if secname == '.dynsym' else '.strtab']
        self.f.seek(soff); strtab = self.f.read(ssize)
        self.f.seek(off); data = self.f.read(size)
        for k in range(0, size, entsize):
            if self.cls == 1:
                st_name, st_value, st_size, st_info, st_other, st_shndx = struct.unpack('<IIIBBH', data[k:k+16])
            else:
                st_name, st_info, st_other, st_shndx, st_value, st_size = struct.unpack('<IBBHQQ', data[k:k+24])
            if st_shndx == 0 or st_name == 0: continue
            end = strtab.find(b'\0', st_name)
            name = strtab[st_name:end].decode('latin1')
            yield name, st_value, st_size

    def dynsyms(self):
        yield from self.symbols('.dynsym')
        yield from self.symbols('.symtab')

    def find_interface_by_name(self, iface_name):
        """Scan .rodata for the interface name string, then the data sections
        for a struct whose first field points at it."""
        needle = iface_name.encode() + b'\0'
        cands = []
        for n in ('.rodata', '.data.rel.ro', '.data', '.data.rel.ro.local'):
            if n not in self.sections: continue
            typ, addr, off, size, link, entsize = self.sections[n]
            self.f.seek(off); data = self.f.read(size)
            pos = data.find(needle)
            while pos != -1:
                if pos == 0 or data[pos-1] == 0:
                    cands.append(addr + pos)
                pos = data.find(needle, pos + 1)
        hits = []
        for n in ('.data.rel.ro', '.data', '.data.rel.ro.local', '.rodata'):
            if n not in self.sections: continue
            typ, addr, off, size, link, entsize = self.sections[n]
            self.f.seek(off); data = self.f.read(size)
            for sa in cands:
                pat = struct.pack(self.pfmt, sa)
                pos = data.find(pat)
                while pos != -1:
                    va = addr + pos
                    if va % self.ptr == 0:
                        try:
                            P = self.ptr; al = lambda o: (o + P - 1) // P * P
                            version = struct.unpack('<i', data[pos + P:pos + P + 4])[0]
                            mcount = struct.unpack('<i', data[pos + P + 4:pos + P + 8])[0]
                            oe = al(P + 8) + P
                            ecount = struct.unpack('<i', data[pos + oe:pos + oe + 4])[0]
                            if 0 <= version < 64 and 0 <= mcount < 64 and 0 <= ecount < 64:
                                hits.append(va)
                        except struct.error:
                            pass
                    pos = data.find(pat, pos + 1)
        return hits

    def message(self, vaddr):
        name = self.cstr(self.ptr_at(vaddr))
        sig = self.cstr(self.ptr_at(vaddr + self.ptr))
        types_p = self.ptr_at(vaddr + 2 * self.ptr)
        nargs = sum(1 for c in sig if c in 'iufsonah')  # digits are versions, '?' nullable
        types = []
        for a in range(nargs):
            tp = self.ptr_at(types_p + a * self.ptr) if types_p else 0
            types.append(self.cstr(self.ptr_at(tp)) if tp else None)  # wl_interface.name
        return name, sig, types

    def interface(self, vaddr):
        # struct wl_interface { const char *name; int version; int method_count; const struct wl_message *methods;
        #                       int event_count; const struct wl_message *events; } with natural alignment
        P = self.ptr
        al = lambda o: (o + P - 1) // P * P
        o_methods = al(P + 8); o_ecount = o_methods + P; o_events = al(o_ecount + 4)
        name = self.cstr(self.ptr_at(vaddr))
        version = struct.unpack('<i', self.read(vaddr + P, 4))[0]
        mcount = struct.unpack('<i', self.read(vaddr + P + 4, 4))[0]
        methods = self.ptr_at(vaddr + o_methods)
        ecount = struct.unpack('<i', self.read(vaddr + o_ecount, 4))[0]
        events = self.ptr_at(vaddr + o_events)
        msz = 3 * self.ptr
        return {
            'name': name, 'version': version,
            'requests': [self.message(methods + i * msz) for i in range(mcount)] if methods else [],
            'events': [self.message(events + i * msz) for i in range(ecount)] if events else [],
        }

def dump(i):
    print('interface %s version %d' % (i['name'], i['version']))
    for kind in ('requests', 'events'):
        for idx, (name, sig, types) in enumerate(i[kind]):
            args = ', '.join('%s:%s' % (c, t or '-') for c, t in zip([c for c in sig if c in 'iufsonah'], types))
            print('  %s[%d] %s(%s)  sig=%s' % (kind[:-1], idx, name, args, sig))

def main():
    e = Elf(sys.argv[1])
    wanted = sys.argv[2:]
    syms = {n: v for n, v, s in e.dynsyms() if n.endswith('_interface')}
    print('symbol tables: %d *_interface symbols (%s)' % (len(syms), ', '.join(s for s in ('.dynsym', '.symtab') if s in e.sections)))
    if not wanted:
        wanted = sorted(n for n in syms if not n.startswith('wl_') or n in ('wl_drm_interface',))
    for w in wanted:
        iname = w[:-len('_interface')] if w.endswith('_interface') else w
        addrs = [syms[w]] if w in syms else e.find_interface_by_name(iname)
        if not addrs:
            print('%s: not exported and no struct found by name scan' % w); continue
        for a in addrs:
            try:
                i = e.interface(a)
            except Exception as ex:
                print('%s @0x%x: %s' % (w, a, ex)); continue
            print('# %s at vaddr 0x%x (%s)' % (w, a, 'symbol' if w in syms else 'found by name scan'))
            dump(i)

if __name__ == '__main__':
    main()
