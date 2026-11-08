"""Execute the production NRO lifecycle with host thread/console adapters."""
import pathlib
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SOURCE = (ROOT / 'native/shim.c').read_text()


def function(signature):
    start = SOURCE.index(signature + ' {')
    end = SOURCE.index('{', start) + 1
    depth = 1
    while depth:
        depth += (SOURCE[end] == '{') - (SOURCE[end] == '}')
        end += 1
    return SOURCE[start:end]


class ShutdownTest(unittest.TestCase):
    def test_main_never_returns_or_shuts_down(self):
        main = function('int main(void)')
        self.assertIn('for (;;) {', main)
        self.assertNotIn('while (appletMainLoop()', main)
        for forbidden in ('return', 'break;', 'consoleExit(', 'nro_shutdown(', 'sa_quit(', 'close_all_sockets('):
            self.assertNotIn(forbidden, main)
        for forbidden in ('nro_shutdown', 'join_until', 'threadWaitForExit', 'close_all_sockets'):
            self.assertNotIn(forbidden, SOURCE)

    def test_plus_only_displays_home_instruction(self):
        main = function('int main(void)')
        start = main.index('if (padGetButtonsDown(&pad) & HidNpadButton_Plus) {')
        handler = main[start:main.index('}', start)]
        self.assertIn('printf("press HOME to exit\\n");', handler)
        self.assertIn('consoleUpdate(NULL);', handler)
        self.assertEqual(handler.count(';'), 2)
        self.assertIn('svcSleepThread(100000000);', main[start:])
        self.assertIn('int sa_stopping(void) { return 0; }', SOURCE)


if __name__ == '__main__':
    unittest.main()
