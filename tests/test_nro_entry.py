"""Check the actual SA_NRO-preprocessed entry without executing Switch code."""
import os
import pathlib
import re
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]


def check_entry(source):
    source = re.sub(r'/\*.*?\*/|//[^\n]*', '', source, flags=re.S)
    entry = re.search(r'\bint\s+main\s*\(\s*void\s*\)\s*\{', source)
    assert entry, 'NRO main missing'
    body = source[entry.end():]
    assert re.match(r'\s*consoleInit\s*\(\s*(?:NULL|\(\(void\s*\*\)\s*0\))\s*\)\s*;', body), 'consoleInit(NULL) must be the first NRO statement'


class NroEntryTest(unittest.TestCase):
    def test_built_nro_source(self):
        devkit = pathlib.Path(os.environ.get('DEVKITPRO', pathlib.Path.home() / 'devkitpro'))
        source = subprocess.check_output([
            devkit / 'devkitA64/bin/aarch64-none-elf-gcc',
            '-E', '-P', '-DSA_NRO', '-D__SWITCH__',
            '-I' + str(devkit / 'libnx/include'), str(ROOT / 'native/shim.c'),
        ], text=True)
        check_entry(source)
        main = source[source.index('int main(void) {'):source.index('size_t sa_log_tail')]
        for output in re.finditer(r'\bprintf\((?:"(?:\\.|[^"\\])*"|[^";])*;' , main):
            self.assertRegex(main[output.end():], r'^\s*consoleUpdate\(\(\(void \*\)0\)\)|^\s*consoleUpdate\(NULL\)')
        self.assertIn('SWAgent %s', main)
        self.assertIn('port 6000 busy - sysmodule already running', main)
        self.assertIn('press HOME to exit', main)
        self.assertIn('svcSleepThread(100000000)', main)
        self.assertIn('HidNpadButton_Plus', main)

    def test_rejects_output_or_worker_before_init(self):
        for call in ('printf("boot");', 'puts("boot");', 'write(1, p, n);', 'start_worker();'):
            with self.subTest(call=call), self.assertRaises(AssertionError):
                check_entry('int main(void) { ' + call + ' consoleInit(NULL); }')

    def test_rejects_missing_init(self):
        with self.assertRaises(AssertionError):
            check_entry('int main(void) { puts("boot"); }')


if __name__ == '__main__':
    unittest.main()
