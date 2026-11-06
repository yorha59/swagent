"""Validate the deliverable container, NPDM identity and linked target."""
import hashlib, os, pathlib, re, struct, subprocess
root = pathlib.Path(__file__).resolve().parent.parent
artifact = root / 'dist/atmosphere/contents/430000000000000B/exefs.nsp'
p = artifact.read_bytes()
assert p[:4] == b'PFS0'
count, names_len = struct.unpack_from('<II', p, 4)
base = 16 + count*24 + names_len
names = p[16+count*24:base]
files = {}
for i in range(count):
    offset, size, name_offset = struct.unpack_from('<QQI', p, 16+i*24)
    name = names[name_offset:].split(b'\0')[0].decode()
    assert base+offset+size <= len(p)
    files[name] = p[base+offset:base+offset+size]
assert set(files) == {'main', 'main.npdm'}
assert files['main'][:4] == b'NSO0'
npdm = files['main.npdm']
assert npdm[:4] == b'META'
aci, = struct.unpack_from('<I', npdm, 0x70)
assert npdm[aci:aci+4] == b'ACI0'
assert struct.unpack_from('<Q', npdm, aci+0x10)[0] == 0x430000000000000B
elf = (root / 'build/swagent.elf').read_bytes()
assert elf[:4] == b'\x7fELF' and elf[4:6] == bytes([2,1])
assert struct.unpack_from('<H', elf, 18)[0] == 183  # AArch64
nm = pathlib.Path(os.environ.get('DEVKITPRO', str(pathlib.Path.home()/'devkitpro'))) / 'devkitA64/bin/aarch64-none-elf-nm'
symbols = subprocess.check_output([nm, root/'build/swagent.elf'], text=True)
assert re.search(r' T agent_main$', symbols, re.M)
assert not re.search(r' T (?:_exit|exit|abort)$', symbols, re.M)
assert not re.search(r'\b(?:sa_(?:hid_init|buttons|touch|capture|jpeg\w*)|hiddbg\w*|capssc\w*|tj\w*)$', symbols, re.M)
assert b'swagent-1.8-rs' in elf
assert (root/'build/swagent.nsp').read_bytes() == p
assert (artifact.parent/'flags/boot2.flag').exists()
print(f'NSP: PFS0 + NSO + NPDM title ID + AArch64 ELF verified; {len(p):,} bytes')
print(f'SHA256: {hashlib.sha256(p).hexdigest()}')
