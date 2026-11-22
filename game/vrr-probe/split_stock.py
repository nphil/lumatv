"""Offline interpreter for channel 0's two stock owners. No device backend or arbitrary entry point."""
import ctypes
import hashlib
import re
import struct
import subprocess
import tempfile

IMAGE_SHA = 'bc5aa595f350a0dc89f6f314c8cfe6977863ab021f7cda133bdf0795bbefd6de'
BOOT_SHA = '3e13cfe557ad8037972c77f03bd4d8d226e2f58eee0127fd38a98ae23fa52ca8'


def load_image(path):
    if not path.is_file() or path.stat().st_size not in (1821632, 21106355):
        raise ValueError('expected the local retail image 0 or boot snapshot')
    data = path.read_bytes()
    source = hashlib.sha256(data).hexdigest()
    if source == IMAGE_SHA:
        return data
    if source != BOOT_SHA:
        raise ValueError('wrong boot snapshot or image 0')
    # Only decompress blocks intersecting the embedded image. The preceding digest pins the layout.
    start, end = 0x1f71028, 0x1f71028 + 1821632
    block, count = struct.unpack_from('<II', data, 12)
    sizes = struct.unpack_from(f'<{count}I', data, 32)
    offset = 32 + 4 * count
    result = bytearray()
    decoder = ctypes.CDLL('liblz4.so.1').LZ4_decompress_safe
    decoder.argtypes = (ctypes.c_char_p, ctypes.c_void_p, ctypes.c_int, ctypes.c_int)
    for i, n in enumerate(sizes):
        lo = block * i
        if lo >= end: break
        if lo + block > start:
            buf = ctypes.create_string_buffer(block)
            if decoder(data[offset:offset+n], buf, n, block) != block:
                raise ValueError('unexpected LZ4P block size')
            result.extend(buf[max(0,start-lo):min(block,end-lo)])
        offset += n
    return bytes(result)


CACHE, PULSE, PIXEL, TIMER = 0x50395db0, 0xcc8c0020, 0xcc8c00a4, 0xcc8c0080
SELECT, PERIOD = 0x4000d4e0, 0x502b5284
MASK32 = 0xffffffff
# Only the reachable instructions for requests 0x20/0x50, main window, diagnostics off.
BLOCKS = ((0xd4e0,0xd4ea), (0xd4ed,0xd502), (0xd505,0xd510), (0xd56b,0xd571),
          (0xd574,0xd58f), (0xd599,0xd59e), (0xd5c9,0xd5d0), (0xd635,0xd653),
          (0xd658,0xd65d), (0xd660,0xd6b4), (0xd832,0xd838))
SEGMENTS = ((0x3ffe002c,0x3ffe23d0,0x368), (0x400003c8,0x4001fc8c,0x3d60),
            (0x50200000,0x502339bc,0x23624), (0x502542b0,0x5032c6b4,0x778d4),
            (0x50395320,0x5039959c,0x1b8944))


def image_bytes(image, address, size):
    for lo, hi, offset in SEGMENTS:
        if lo <= address and address + size <= hi:
            start = offset + address - lo
            return image[start:start + size]
    raise ValueError(f'unmapped image read {address:#x}')


def decode(image, objdump):
    if hashlib.sha256(image).hexdigest() != IMAGE_SHA:
        raise ValueError('wrong retail image 0')
    ops = {}
    blocks = [(0x40000000 + lo, 0x40000000 + hi) for lo, hi in BLOCKS] + [(PERIOD, 0x502b52ab)]
    for lo, hi in blocks:
        # Disassemble at known instruction boundaries, not across Xtensa code/data padding.
        with tempfile.NamedTemporaryFile() as fp:
            fp.write(image_bytes(image, lo, hi - lo)); fp.flush()
            text = subprocess.check_output([objdump, '-D', '-b', 'binary', '-m', 'xtensa', '-EL',
                f'--adjust-vma={lo}', fp.name], text=True, timeout=2)
        pc = lo
        for addr, bits, op, args in re.findall(r'^\s*([0-9a-f]+):\s+([0-9a-f]{4,6})\s+(\S+)[ \t]*([^\n]*)$', text, re.M):
            if int(addr, 16) != pc:
                raise ValueError(f'misaligned instruction at {addr}')
            size = len(bits) // 2
            operands = [s.strip() for s in args.split(',')] if args.strip() else []
            ops[pc] = (size, op.removesuffix('.n'), operands)
            pc += size
        if pc != hi:
            raise ValueError(f'incomplete disassembly at {pc:#x}')
    return ops


class StockOwners:
    def __init__(self, image, ops, original):
        self.image, self.ops = image, ops
        self.ram = {CACHE: original['cache'], PULSE: original['pulse'], PIXEL: original['pixel'], TIMER: original['timer'],
                    0x3fffa9d8 + 52: 0, 0x3fffa9d8 + 72: 0}

    def words(self):
        return dict(zip(('cache','pulse','pixel','timer'), (self.ram[k] for k in (CACHE,PULSE,PIXEL,TIMER))))

    def _run(self, pc, args, store):
        sp = 0x01000000
        regs = {'a1': sp, **{f'a{i}': n for i,n in enumerate(args, 2)}}
        stack = {}
        for _ in range(96):
            size, op, operands = self.ops[pc]
            here, pc = pc, pc + size
            def v(i):
                s = operands[i]
                return regs[s] if s.startswith('a') else int(s, 0)
            dst = operands[0] if operands else None
            if op == 'retw':
                if regs['a2'] != 0: raise ValueError('nonzero stock return')
                return
            if op == 'entry': regs['a1'] -= v(1)
            elif op == 'memw': pass  # Ordered single-threaded host memory only.
            elif op in ('mov','movi'): regs[dst] = v(1) & MASK32
            elif op == 'l32r': regs[dst] = int.from_bytes(image_bytes(self.image, v(1), 4), 'little')
            elif op in ('l32i','l16ui'):
                a = v(1) + v(2); n = 4 if op == 'l32i' else 2
                if a in stack: x = stack[a]
                elif a in self.ram: x = self.ram[a]
                else: x = int.from_bytes(image_bytes(self.image, a, n), 'little')
                regs[dst] = x & ((1 << (8*n)) - 1)
            elif op == 's32i':
                a, x = v(1) + v(2), v(0)
                if a in (CACHE, PULSE, TIMER): self.ram[a] = x
                elif regs['a1'] <= a <= sp - 4: stack[a] = x
                else: raise ValueError(f'unexpected stock write {here:#x} -> {a:#x}')
                store(here, a, x)
            elif op in ('movnez','moveqz'):
                if (v(2) != 0) == (op == 'movnez'): regs[dst] = v(1)
            elif op == 'extui': regs[dst] = (v(1) >> v(2)) & ((1 << v(3)) - 1)
            elif op == 'slli': regs[dst] = (v(1) << v(2)) & MASK32
            elif op == 'add': regs[dst] = (v(1) + v(2)) & MASK32
            elif op == 'mull': regs[dst] = (v(1) * v(2)) & MASK32
            elif op == 'quou': regs[dst] = v(1) // v(2)
            elif op == 'and': regs[dst] = v(1) & v(2)
            elif op == 'or': regs[dst] = v(1) | v(2)
            elif op in ('j','jx'): pc = v(0)
            elif op in ('bgeu','bne','bnei','bltu','beq','bany'):
                x,y = v(0), v(1)
                if {'bgeu':x>=y, 'bne':x!=y, 'bnei':x!=y, 'bltu':x<y, 'beq':x==y, 'bany':bool(x&y)}[op]: pc = v(2)
            else: raise ValueError(f'unsupported instruction {here:#x}: {op}')
        raise ValueError('stock instruction bound exceeded')

    def pulse(self, request, store):
        if request not in (0x20,0x50): raise ValueError('not a split owner argument')
        self._run(SELECT, (0,request), store)

    def period(self, numerator, denominator, store):
        if not (0 < numerator <= MASK32 and 0 < denominator <= MASK32): raise ValueError('invalid timer owner arguments')
        self._run(PERIOD, (0,numerator,denominator), store)
