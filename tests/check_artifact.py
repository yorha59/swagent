"""Validate the deliverable container, NPDM identity and linked target."""
import hashlib, os, pathlib, re, struct, subprocess, sys
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
assert b'swagent-1.30-rs' in elf
assert (root/'build/swagent.nsp').read_bytes() == p
assert (artifact.parent/'flags/boot2.flag').exists()
print(f'NSP: PFS0 + NSO + NPDM title ID + AArch64 ELF verified; {len(p):,} bytes')
print(f'SHA256: {hashlib.sha256(p).hexdigest()}')

if '--sysmodule-only' in sys.argv:
    sys.exit(0)

nro = (root/'dist/switch/swagent.nro').read_bytes()
assert nro[0x10:0x14] == b'NRO0'
nro_size = struct.unpack_from('<I', nro, 0x18)[0]
assert nro_size <= len(nro)
if nro_size < len(nro):
    assert nro[nro_size:nro_size+4] == b'ASET'
    for i in range(3):
        offset, size = struct.unpack_from('<QQ', nro, nro_size+8+i*16)
        assert nro_size+offset+size <= len(nro)
segments = [struct.unpack_from('<II', nro, 0x20+i*8) for i in range(3)]
assert segments[0][0] == 0
assert all(off % 4096 == size % 4096 == 0 for off, size in segments)
assert all(segments[i][0]+segments[i][1] == segments[i+1][0] for i in range(2))
assert sum(size for off, size in segments) == nro_size
mod, = struct.unpack_from('<I', nro, 4)
assert nro[mod:mod+4] == b'MOD0'
_, bss_start, bss_end = struct.unpack_from('<III', nro, mod+4)
assert nro_size <= mod+bss_end <= nro_size+struct.unpack_from('<I', nro, 0x38)[0]
assert mod+bss_start <= mod+bss_end
assert b'swagent-1.30-rs' in nro and b'listening %s:6000' in nro
assert b'port 6000 busy - sysmodule already running' in nro
assert b'press HOME to exit' in nro and b'SWAgent %s' in nro
print(f'NRO: NRO0 + page segments + MOD0/BSS verified; {len(nro):,} bytes')

# Verify hbmenu uses libnx crt0/default runtime rather than resident overrides.
nro_elf = root/'build/swagent-nro.elf'
nro_symbols = subprocess.check_output([nm, nro_elf], text=True)
for symbol in ('__appInit', '__libnx_initheap', '__appExit'):
    assert re.search(r' W ' + symbol + r'$', nro_symbols, re.M), symbol
assert re.search(r'^0+ T _start$', nro_symbols, re.M)
assert re.search(r' T agent_nro_main$', nro_symbols, re.M)
assert not re.search(r' T agent_main$', nro_symbols, re.M)
objdump = nm.with_name('aarch64-none-elf-objdump')
def disassemble(symbol):
    return subprocess.check_output([objdump, '-d', '--disassemble='+symbol, nro_elf], text=True)
init = disassemble('sa_init')
assert not any(name in init for name in ('smInitialize', 'fsInitialize', 'setsysInitialize', 'log_writer'))
main = disassemble('main')
calls = re.findall(r'\bbl\s+[0-9a-f]+\s+<([^>]+)>', main)
assert calls[0] == 'consoleInit', calls
assert main.index('<consoleInit>') < main.index('<threadStart>')
for output in ('printf', 'puts', 'vfprintf', 'fwrite', 'write'):
    if '<'+output+'>' in main:
        assert main.index('<consoleInit>') < main.index('<'+output+'>')
assert '<svcExitProcess>' not in main
assert '<consoleExit>' not in main
assert not re.search(r'\bret\b', main)
assert 'nro_shutdown' not in nro_symbols
assert 'join_until' not in nro_symbols
assert '<sleep_guard' not in disassemble('agent_nro_main')
assert re.search(r' T __libnx_initheap$', symbols, re.M)
print('NRO entry: libnx crt0/heap/app init + console-before-worker + isolated network init verified')
