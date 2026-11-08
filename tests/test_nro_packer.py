"""Exercise packaging without requiring the Switch compiler."""
import pathlib
import struct
import sys
import unittest
sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / 'tools'))
from nro_packer import pack


class PackerTest(unittest.TestCase):
    def test_dummy_segments_and_mod_offsets(self):
        elf = bytearray(0x3400)
        elf[:6] = b'\x7fELF\x02\x01'
        struct.pack_into('<H', elf, 18, 183)
        struct.pack_into('<Q', elf, 32, 64)
        struct.pack_into('<HH', elf, 54, 56, 3)
        for i, (size, mem, flags) in enumerate([(0x180, 0x180, 5), (17, 17, 4), (33, 0x2500, 6)]):
            struct.pack_into('<II6Q', elf, 64 + i * 56, 1, flags, 0x1000 * (i+1), 0x1000*i, 0, size, mem, 0x1000)
        struct.pack_into('<II', elf, 0x1000, 0x14000020, 0x100)
        elf[0x1100:0x1104] = b'MOD0'
        struct.pack_into('<6I', elf, 0x1104, 0x1f00, 0x2000, 0x4400, 0, 0, 0)
        nro = pack(elf)
        self.assertEqual(nro[0x10:0x14], b'NRO0')
        self.assertEqual(len(nro), 0x3000)
        self.assertEqual(struct.unpack_from('<I', nro, 0x18)[0], len(nro))
        self.assertEqual(struct.unpack_from('<7I', nro, 0x20), (0, 0x1000, 0x1000, 0x1000, 0x2000, 0x1000, 0x2000))
        self.assertEqual(nro[:16], elf[0x1000:0x1010])
        self.assertEqual(nro[0x100:0x11c], elf[0x1100:0x111c])
        with self.assertRaises(ValueError):
            pack(b'bad')


if __name__ == '__main__':
    unittest.main()
