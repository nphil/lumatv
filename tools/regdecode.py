#!/usr/bin/env python3
"""Decode and diff /proc/lg/fbdev/regdump captures against LG's OSD register headers. Reads files only.

The C4's `cat /proc/lg/fbdev/regdump` prints the OSD, TOP, AFBC0/1, SHP0/1 and MIF0/1 register blocks as
`0xADDR (+0xOFF) = VALUE` lines. LG's GPL kernel tree names those registers and their bit fields
(kdriver/chip/o22/fbdev/osd_{reg,top,afbc,shp}_reg_o22.h); this tool joins the two. The headers are not in this
repository: point --kdriver (or $LG_KDRIVER) at the tree's kdriver/ directory. MIF has no header there and stays raw.

  regdecode.py decode DUMP [--all] [--block OSD] [--grep REGEX] [--unknown]
  regdecode.py diff A B [--noise N1 N2] [--ignore REGEX] [--block OSD] [--grep REGEX]
  regdecode.py find REGEX                      where a register sits (absolute addresses, default block bases)

DUMP is the regdump text or a file that contains it (other lines are skipped), or lines of `0xADDR 0xVALUE`
(devmem reads), which are assigned to a block by address. If a file holds several regdumps, the last one wins.
`diff` exits 1 when registers differ, like diff(1). A noise pair (two dumps with nothing written, as in
docs/research/compositor/osd01-probe-2026-10-29/noise-dry-run) removes the registers that change by themselves.

The OSD block is not a flat list: osd_reg_o22.h declares it as the C struct OSD_O22_REG_T (status and common, the
three mixers, four 256-byte layer windows, HDR-UI), and the offsets in the register comments restart inside every
window. The tool lays that struct out the way a compiler would, so a register is named by its window: OSD0, OSD0_1,
OSD1 (removed in hardware) or CURSOR.
"""
import argparse
import os
import re
import signal
import sys

DEFAULT_KDRIVER = "/data/home/lgtv/research/lgsrc/k/soc_GO/linux-and-kdriver_paluma-5/kdriver"
HEADER_DIR = os.path.join("chip", "o22", "fbdev")

# Dump block -> (header file, base seen on the C4, size in bytes). The comments in the headers carry stale 0xC901xxxx
# bases; offsets inside a header are relative to the block base the dump prints.
BLOCKS = {
    "TOP": ("osd_top_reg_o22.h", 0xCC480000, 256),
    "OSD": ("osd_reg_o22.h", 0xCC480100, 1756),
    "AFBC0": ("osd_afbc_reg_o22.h", 0xCC480800, 256),
    "AFBC1": ("osd_afbc_reg_o22.h", 0xCC482700, 256),
    "SHP0": ("osd_shp_reg_o22.h", 0xCC481C00, 768),
    "SHP1": ("osd_shp_reg_o22.h", 0xCC482300, 768),
    "MIF0": (None, 0xCC480C00, 192),
    "MIF1": (None, 0xCC482800, 192),
}

WINDOWS = ("OSD0", "OSD0_1", "OSD1", "CURSOR")
# osdx_ctrl[i] is a union of four member structs (osd_reg_o22.h, OSDX_CTRL): 0 the registers every plane has, 1 the
# OSD0-only extras, 2 the OSD0_1-only 4K registers, 3 the cursor-only ones. LG's comments pin each to its plane
# ("should be access by osdx[1]"); OSD0_1 and OSD1 have no scaler, scaler info or path register ("blank").
WINDOW_ALTS = {0: (0, 1), 1: (0, 2), 2: (0,), 3: (0, 3)}
NO_SCALER = {"osd_ctrl_scaler", "osd_info_scaler", "osd_ctrl_path"}
PADDING = re.compile(r"^__")  # __rsvd_NN, __skip_NN, __skp_NN, __sep_N: gaps, not registers

FIELD_RE = re.compile(r"^[ \t]*(\w*)[ \t]*:[ \t]*(\d+)[ \t]*[,;][ \t]*//[ \t]*(\d+)(?:[ \t]*:[ \t]*(\d+))?(.*)$")
FLAT_REG_RE = re.compile(r"^[ \t]*0x([0-9a-fA-F]{4,5})[ \t]+(\w+)[ \t]+'[^']*'[ \t]*$", re.M)
SECTION_RE = re.compile(r"^FBDEV \((\w+)\) : 0x([0-9a-fA-F]+) \(size=(\d+)\)")
WORD_RE = re.compile(r"^\s*0x([0-9a-fA-F]+) \(\+0x([0-9a-fA-F]+)\) = ([0-9a-fA-F]+)\s*$")
ABS_RE = re.compile(r"^\s*0x([0-9a-fA-F]+)\s*[:=]?\s*0x([0-9a-fA-F]+)\s*$")
BITFIELD_RE = re.compile(r"\w*\s*:\s*\d+\s*[,;]")
TOKEN_RE = re.compile(r"[A-Za-z_]\w*|0[xX][0-9a-fA-F]+|\d+|[{}\[\];]|\S")


class Slot:
    """One register at one offset: a label, its C type, and its field layouts (one, or two for a union of structs)."""

    def __init__(self, label, offset, type_name, alts, window=None):
        self.label = label
        self.offset = offset
        self.type_name = type_name
        self.alts = alts or [[]]
        self.window = window

    @property
    def fields(self):
        return self.alts[1 if self.window == 3 and len(self.alts) > 1 else 0]

    def split(self, value):
        """The named fields of value, in bit order: [(name, low bit, width, field value)]."""
        return [(n, lo, w, (value >> lo) & ((1 << w) - 1)) for n, lo, w in self.fields if n]


def strip_comments(text):
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", "", text)


def preprocess(text):
    """Keep the live branch of `#if 0|1 / #else / #endif`; other directives and macro bodies are dropped."""
    out, stack, lines = [], [], text.splitlines()
    i = 0
    while i < len(lines):
        line = lines[i]
        s = line.strip()
        if s.startswith("#"):
            while lines[i].rstrip().endswith("\\") and i + 1 < len(lines):
                i += 1  # a macro body continues on the next line
            m = re.match(r"#\s*if\s+([01])\b", s)
            if m:
                stack.append([m.group(1) == "1"])
            elif re.match(r"#\s*(ifdef|ifndef|if)\b", s):
                stack.append([True])  # include guards and `defined()` tests: keep the block
            elif re.match(r"#\s*else\b", s) and stack:
                stack[-1][0] = not stack[-1][0]
            elif re.match(r"#\s*endif\b", s) and stack:
                stack.pop()
        elif all(live for live, in stack):
            out.append(line)
        i += 1
    return "\n".join(out)


def parse_fields(raw):
    """[(name, low bit, width)] from a bit-field struct body; positions come from LG's `lo:hi` comments."""
    fields, cursor = [], 0
    for line in raw.splitlines():
        f = FIELD_RE.match(line)
        if not f:
            continue
        name, width, lo, hi, rest = f.group(1), int(f.group(2)), int(f.group(3)), f.group(4), f.group(5)
        if hi is not None and int(hi) - lo + 1 != width:
            lo = cursor  # the comment and the width disagree: trust the running sum
        fields.append(("" if "reserved" in rest else name, lo, width))
        cursor = lo + width
    return fields


def typedef_bodies(text):
    """[(typedef kind, name, body)] for every `typedef struct|union { ... } NAME;`, braces balanced."""
    out = []
    for m in re.finditer(r"typedef\s+(struct|union)\s*\{", text):
        depth, i = 1, m.end()
        while i < len(text) and depth:
            depth += {"{": 1, "}": -1}.get(text[i], 0)
            i += 1
        tail = re.match(r"\s*(\w+)\s*;", text[i:])
        if depth == 0 and tail:
            out.append((m.group(1), tail.group(1), text[m.end():i - 1]))
    return out


class Layout:
    """The register types of one header and, if it declares a master struct, the C layout of that struct."""

    def __init__(self, text):
        self.regs = {}  # type name -> [field list] (a union of bit-field structs has one list per struct)
        self.aggs = {}  # type name -> (kind, member list)
        clean = preprocess(text)
        for kind, name, body in typedef_bodies(clean):
            if BITFIELD_RE.search(strip_comments(body)):
                blocks = re.findall(r"struct\s*\{(.*?)\}\s*;", body, re.S)
                self.regs[name] = [parse_fields(b) for b in blocks] if blocks else [parse_fields(body)]
            else:
                self.aggs[name] = (kind, self.parse_members(TOKEN_RE.findall(strip_comments(body)), [0]))
        self.sizes = {}

    def parse_members(self, toks, pos):
        """Members of one brace level: (kind, type or sub-members, name, count)."""
        members = []
        while pos[0] < len(toks) and toks[pos[0]] != "}":
            t = toks[pos[0]]
            if t in ("struct", "union") and toks[pos[0] + 1] == "{":
                pos[0] += 2
                sub = self.parse_members(toks, pos)
                pos[0] += 1  # the closing brace
                kind, ref = t, sub
            else:
                kind, ref = "type", t
                pos[0] += 1
            name = None
            if toks[pos[0]] != ";" and toks[pos[0]] != "[":
                name = toks[pos[0]]
                pos[0] += 1
            count = 1
            while toks[pos[0]] == "[":
                count *= int(toks[pos[0] + 1], 0)
                pos[0] += 3
            pos[0] += 1  # the semicolon
            members.append((kind, ref, name, count))
        return members

    def size(self, kind, ref):
        if kind == "type":
            if ref in self.regs or ref == "UINT32":
                return 4
            if ref in self.aggs:
                return self.agg_size(ref)
            raise ValueError("unknown type " + ref)
        return self.body_size(kind, ref)

    def body_size(self, kind, members):
        sizes = [self.size(k, r) * c for k, r, _, c in members]
        return max(sizes or [0]) if kind == "union" else sum(sizes)

    def agg_size(self, name):
        if name not in self.sizes:
            kind, members = self.aggs[name]
            self.sizes[name] = self.body_size(kind, members)
        return self.sizes[name]

    def flatten(self, root):
        """[Slot] for a typedef: every register in it at its C offset (a union's members share the offset)."""
        out = []
        kind, members = self.aggs[root]
        self.walk(kind, members, 0, "", [], out)
        return out

    def walk(self, kind, members, base, path, ctx, out):
        off = base
        for idx, (k, ref, name, count) in enumerate(members):
            here = base if kind == "union" else off
            tag = ctx + [("alt", idx)] if kind == "union" else ctx
            width = self.size(k, ref)
            if name and PADDING.match(name):
                off += width * count
                continue
            for i in range(count):
                at = here + i * width
                label = path + ("." if path and name else "") + (name or "")
                label += "[%d]" % i if count > 1 else ""
                leaf_tag = tag + [("win", i)] if ref == "OSDX_CTRL" else tag
                if k == "type" and ref in self.aggs:
                    sub_kind, sub = self.aggs[ref]
                    self.walk(sub_kind, sub, at, label, leaf_tag, out)
                elif k in ("struct", "union"):
                    self.walk(k, ref, at, label, leaf_tag, out)
                else:
                    out.append((label, at, ref, self.regs.get(ref), leaf_tag))
            off += width * count


def osd_slots(layout):
    """{offset: [Slot]} for OSD_O22_REG_T, one slot per register that exists in its window."""
    slots = {}
    for label, off, type_name, alts, ctx in layout.flatten("OSD_O22_REG_T"):
        win = next((v for t, v in ctx if t == "win"), None)
        if win is not None:
            alt = next(k for t, k in ctx[ctx.index(("win", win)) + 1:] if t == "alt")
            member = label.rsplit(".", 1)[-1]
            if alt not in WINDOW_ALTS[win] or (win in (1, 2) and member in NO_SCALER):
                continue
            label = "%s.%s" % (WINDOWS[win], member)
        slots.setdefault(off, []).append(Slot(label, off, type_name, alts, win))
    return slots


def flat_slots(text):
    """{offset: [Slot]} for the single-frame headers: the comment `0xOFFSET name ''` gives the offset."""
    clean = preprocess(text)
    layout = Layout(clean)
    found = list(FLAT_REG_RE.finditer(clean))
    slots = {}
    for i, m in enumerate(found):
        end = found[i + 1].start() if i + 1 < len(found) else len(clean)
        body = re.search(r"typedef\s+struct\s*\{(.*?)\}\s*(\w+)\s*;", clean[m.end():end], re.S)
        alts = layout.regs.get(body.group(2)) if body else None
        off = int(m.group(1), 16)
        slots.setdefault(off, []).append(Slot(m.group(2), off, body.group(2) if body else "", alts))
    return slots


def load_headers(kdriver):
    """{header file: {offset: [Slot]}} for the files BLOCKS names. A missing tree is an error naming the fix."""
    root = os.path.join(kdriver, HEADER_DIR)
    if not os.path.isdir(root):
        raise SystemExit("no LG headers under %s: pass --kdriver or set LG_KDRIVER to the kdriver/ directory of "
                         "LG's GPL tree" % root)
    out = {}
    for header, _, _ in BLOCKS.values():
        if header and header not in out:
            with open(os.path.join(root, header), encoding="latin-1") as f:
                text = f.read()
            out[header] = osd_slots(Layout(text)) if header == "osd_reg_o22.h" else flat_slots(text)
    return out


def block_at(addr):
    for name, (_, base, size) in BLOCKS.items():
        if base <= addr < base + size:
            return name, addr - base
    return None, None


def parse_dump(text):
    """{block: {offset: value}} from regdump text and from `0xADDR 0xVALUE` lines."""
    dump = {}
    cur = None
    for line in text.splitlines():
        if line.startswith("####"):
            cur = None
            continue
        m = SECTION_RE.match(line)
        if m:
            cur = m.group(1)
            dump[cur] = {}  # a later capture of the same block replaces the earlier one
            continue
        m = WORD_RE.match(line)
        if m and cur:
            dump[cur][int(m.group(2), 16)] = int(m.group(3), 16)
            continue
        m = ABS_RE.match(line)
        if m:
            name, off = block_at(int(m.group(1), 16))
            if name:
                dump.setdefault(name, {})[off] = int(m.group(2), 16)
    return {b: w for b, w in dump.items() if b in BLOCKS and w}


def read_dump(path):
    with open(path, encoding="latin-1") as f:
        dump = parse_dump(f.read())
    if not dump:
        raise SystemExit("%s: no regdump or `0xADDR 0xVALUE` lines found" % path)
    return dump


def slots_at(headers, block, offset):
    header = BLOCKS[block][0]
    return headers[header].get(offset, []) if header else []


def fmt_value(width, value):
    return str(value) if width <= 4 or value <= 9 else "0x%x(%d)" % (value, value)


def fmt_fields(slot, value, only_nonzero=True):
    return " ".join("%s=%s" % (n, fmt_value(w, v)) for n, _, w, v in slot.split(value) if v or not only_nonzero)


def addr_of(block, offset):
    return BLOCKS[block][1] + offset


def wanted(block, names, args):
    if args.block and block.upper() != args.block.upper():
        return False
    return not args.grep or any(re.search(args.grep, n, re.I) for n in names)


def label_of(slots):
    return "/".join(s.label for s in slots) or "?"


def cmd_decode(args, headers):
    dump = read_dump(args.dump)
    for block in sorted(dump, key=lambda b: BLOCKS[b][1]):
        for off in sorted(dump[block]):
            value = dump[block][off]
            if not value and not args.all:
                continue
            slots = slots_at(headers, block, off)
            if not wanted(block, [s.label for s in slots] or ["?"], args) or (not slots and not args.unknown):
                continue
            print("%-5s +0x%04x 0x%08x  %-38s 0x%08x" % (block, off, addr_of(block, off), label_of(slots), value))
            for s in slots:
                text = fmt_fields(s, value, not args.all)
                if text:
                    print("      " + text)
    return 0


def noise_offsets(a, b):
    return {(blk, off) for blk in set(a) | set(b) for off in set(a.get(blk, {})) | set(b.get(blk, {}))
            if a.get(blk, {}).get(off) != b.get(blk, {}).get(off)}


def cmd_diff(args, headers):
    a, b = read_dump(args.a), read_dump(args.b)
    noise = noise_offsets(read_dump(args.noise[0]), read_dump(args.noise[1])) if args.noise else set()
    changed = hidden = 0
    for block in sorted(set(a) | set(b), key=lambda k: BLOCKS[k][1]):
        for off in sorted(set(a.get(block, {})) | set(b.get(block, {}))):
            va, vb = a.get(block, {}).get(off), b.get(block, {}).get(off)
            if va == vb:
                continue
            slots = slots_at(headers, block, off)
            names = [s.label for s in slots] or ["?"]
            if (block, off) in noise or (args.ignore and any(re.search(args.ignore, n, re.I) for n in names)):
                hidden += 1
                continue
            if not wanted(block, names, args):
                continue
            changed += 1
            sa = "--------" if va is None else "%08x" % va
            sb = "--------" if vb is None else "%08x" % vb
            print("%-5s +0x%04x 0x%08x  %-38s %s -> %s" % (block, off, addr_of(block, off), label_of(slots), sa, sb))
            if va is not None and vb is not None:
                for s in slots:
                    deltas = [(n, w, x, y) for (n, _, w, x), (_, _, _, y) in zip(s.split(va), s.split(vb)) if x != y]
                    if deltas:
                        print("      " + " ".join("%s %s->%s" % (n, fmt_value(w, x), fmt_value(w, y))
                                                   for n, w, x, y in deltas))
    if hidden:
        print("(%d changed registers hidden by --noise/--ignore)" % hidden, file=sys.stderr)
    return 1 if changed else 0


def cmd_find(args, headers):
    hits = 0
    for block, (header, base, _) in BLOCKS.items():
        if not header:
            continue
        for off, slots in sorted(headers[header].items()):
            for s in slots:
                if re.search(args.pattern, s.label, re.I) or any(re.search(args.pattern, n, re.I) for n, _, _ in s.fields):
                    hits += 1
                    print("%-5s +0x%04x 0x%08x  %s" % (block, off, base + off, s.label))
    return 0 if hits else 1


def main(argv=None):
    if hasattr(signal, "SIGPIPE"):
        signal.signal(signal.SIGPIPE, signal.SIG_DFL)  # `| head` is a normal way to read this
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--kdriver", default=os.environ.get("LG_KDRIVER", DEFAULT_KDRIVER),
                   help="kdriver/ directory of LG's GPL tree (default $LG_KDRIVER, then the research checkout)")
    sub = p.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("decode", help="name and split every register of a dump")
    d.add_argument("dump")
    d.add_argument("--all", action="store_true", help="include registers and fields that are 0")
    d.add_argument("--unknown", action="store_true", help="include words no header names")
    d.add_argument("--block", help="OSD, TOP, AFBC0, AFBC1, SHP0, SHP1, MIF0 or MIF1")
    d.add_argument("--grep", help="register names matching this regex")
    f = sub.add_parser("diff", help="the registers and fields that differ between two dumps")
    f.add_argument("a")
    f.add_argument("b")
    f.add_argument("--noise", nargs=2, metavar=("N1", "N2"), help="two dumps with nothing written")
    f.add_argument("--ignore", help="hide registers whose name matches this regex")
    f.add_argument("--block")
    f.add_argument("--grep")
    s = sub.add_parser("find", help="registers whose name or field name matches")
    s.add_argument("pattern")
    args = p.parse_args(argv)
    headers = load_headers(args.kdriver)
    return {"decode": cmd_decode, "diff": cmd_diff, "find": cmd_find}[args.cmd](args, headers)


if __name__ == "__main__":
    sys.exit(main())
