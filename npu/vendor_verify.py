#!/usr/bin/env python3
"""Execute LG's unmodified ARM package verifier in isolated CPU emulation.

No dynamic loader, mocked inference, host syscalls, NPU or hardware simulation.
Only verifies the serialization envelope, never the DNE command semantics.
The private function address is valid exclusively for the hash-pinned TV library.
"""
import argparse
import hashlib
import struct
from pathlib import Path

from elftools.elf.elffile import ELFFile
from unicorn import Uc, UC_ARCH_ARM, UC_MODE_THUMB, UC_PROT_READ, UC_PROT_EXEC, UC_HOOK_INTR
from unicorn.arm_const import UC_ARM_REG_R0, UC_ARM_REG_R1, UC_ARM_REG_SP, UC_ARM_REG_LR, UC_ARM_REG_PC

LIBRARY_SHA256 = '23f1b083aa1763a7979e6995e326c5e82f36cb2e738ac91c25890fd607c2bdc3'
VERIFY_ADDRESS = 0x15C2D
MAX_BYTES = 64 * 1024 * 1024


def verify(library, data):
    image = Path(library).read_bytes()
    if hashlib.sha256(image).hexdigest() != LIBRARY_SHA256:
        raise ValueError('unknown libnpu build: refusing private function address')
    if not 8 <= len(data) <= MAX_BYTES:
        return False
    root = struct.unpack_from('<I', data)[0]
    if root < 4 or root % 4 or root > len(data) - 4:
        return False
    uc = Uc(UC_ARCH_ARM, UC_MODE_THUMB)
    uc.mem_map(0x10000, 0x50000)
    with Path(library).open('rb') as f:
        elf = ELFFile(f)
        for seg in elf.iter_segments():
            if seg['p_type'] != 'PT_LOAD':
                continue
            addr, size, off = seg['p_vaddr'], seg['p_filesz'], seg['p_offset']
            lo, hi = max(addr, 0x10000), min(addr + size, 0x60000)
            if hi > lo:
                uc.mem_write(lo, image[off + lo - addr:off + hi - addr])
    uc.mem_protect(0x10000, 0x50000, UC_PROT_READ | UC_PROT_EXEC)
    base, state, stack, stop = 0x1000000, 0x20000000, 0x30000000, 0x40000000
    uc.mem_map(base, (len(data) + 4095) & ~4095)
    uc.mem_write(base, data)
    uc.mem_protect(base, (len(data) + 4095) & ~4095, UC_PROT_READ)
    uc.mem_map(state, 4096)
    uc.mem_write(state, struct.pack('<7IB3x', base, len(data), 0, 64, 0, 1000000, 0, 1))
    uc.mem_map(stack, 65536)
    uc.mem_map(stop, 4096, UC_PROT_READ | UC_PROT_EXEC)

    def reject_interrupt(_uc, number, _data):
        raise RuntimeError(f'unexpected ARM interrupt/syscall {number}')

    uc.hook_add(UC_HOOK_INTR, reject_interrupt)
    uc.reg_write(UC_ARM_REG_R0, base + root)
    uc.reg_write(UC_ARM_REG_R1, state)
    uc.reg_write(UC_ARM_REG_SP, stack + 65520)
    uc.reg_write(UC_ARM_REG_LR, stop | 1)
    uc.emu_start(VERIFY_ADDRESS, stop, timeout=5_000_000, count=2_000_000)
    if uc.reg_read(UC_ARM_REG_PC) != stop:
        raise RuntimeError('verifier did not return within emulation bounds')
    return bool(uc.reg_read(UC_ARM_REG_R0))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('library', type=Path)
    parser.add_argument('package', type=Path)
    args = parser.parse_args()
    passed = verify(args.library, args.package.read_bytes())
    print('libnpu_sha256=' + hashlib.sha256(args.library.read_bytes()).hexdigest())
    print('LG_ARM_package_verifier=' + ('PASS' if passed else 'FAIL'))
    print('hardware_execution_verified=false')
    raise SystemExit(0 if passed else 1)


if __name__ == '__main__':
    main()
