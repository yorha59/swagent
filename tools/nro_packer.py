#!/usr/bin/env python3
"""Pack a libnx AArch64 PIE as a bare hbmenu NRO (stdlib only).

NRO0 has a 0x10 startup stub followed by a 0x70 header. Virtual
addresses remain file offsets, preserving crt0's MOD0-relative references.
"""
import argparse
import pathlib
import struct


def align(n):
    return (n + 4095) & ~4095


def pack(elf):
    if len(elf) < 64 or elf[:6] != b'\x7fELF\x02\x01':
        raise ValueError('expected ELF64 little endian')
    if struct.unpack_from('<H', elf, 18)[0] != 183:
        raise ValueError('expected AArch64')
    phoff = struct.unpack_from('<Q', elf, 32)[0]
    entsize, count = struct.unpack_from('<HH', elf, 54)
    if entsize != 56 or phoff + count * entsize > len(elf):
        raise ValueError('invalid program headers')
    segments = []
    for i in range(count):
        kind, flags, off, addr, _, size, mem, _ = struct.unpack_from('<II6Q', elf, phoff + i * entsize)
        if kind == 1:
            if size > mem or off + size > len(elf):
                raise ValueError('invalid segment')
            segments.append((addr, off, size, mem, flags))
    if len(segments) != 3 or [s[4] for s in segments] != [5, 4, 6]:
        raise ValueError('expected text/rodata/data LOAD segments')
    end = 0
    for addr, off, size, mem, flags in segments:
        if addr != end:
            raise ValueError('expected contiguous page-aligned segments')
        end += align(size)
    if segments[0][2] < 0x80:
        raise ValueError('missing startup/header reservation')
    out = bytearray(end)
    for addr, off, size, mem, flags in segments:
        out[addr:addr + size] = elf[off:off + size]
    mod = struct.unpack_from('<I', out, 4)[0]
    if mod < 0x80 or mod + 28 > end or out[mod:mod + 4] != b'MOD0':
        raise ValueError('missing MOD0 startup offset')
    header = bytearray(0x70)
    struct.pack_into('<4sIII', header, 0, b'NRO0', 0, end, 0)
    for i, (addr, off, size, mem, flags) in enumerate(segments):
        struct.pack_into('<II', header, 0x10 + i * 8, addr, align(size))
    data = segments[2]
    struct.pack_into('<I', header, 0x28, align(max(0, data[3] - align(data[2]))))
    out[0x10:0x80] = header
    return bytes(out)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('elf', type=pathlib.Path)
    parser.add_argument('nro', type=pathlib.Path)
    args = parser.parse_args()
    args.nro.write_bytes(pack(args.elf.read_bytes()))
