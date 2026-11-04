import struct, zlib
p = open('build/test.png', 'rb').read()
assert p[:8] == b'\x89PNG\r\n\x1a\n'
off, data = 8, b''
while off < len(p):
    n, = struct.unpack_from('>I', p, off)
    kind, body = p[off+4:off+8], p[off+8:off+8+n]
    crc, = struct.unpack_from('>I', p, off+8+n)
    assert zlib.crc32(kind + body) == crc
    if kind == b'IDAT': data += body
    if kind == b'IHDR': assert struct.unpack('>IIBBBBB', body) == (2, 2, 8, 2, 0, 0, 0)
    off += n+12
assert zlib.decompress(data) == bytes([0, 255,0,0, 0,255,0, 0, 0,0,255, 255,255,255])
print('PNG: independent CRC, zlib and pixel validation passed')
