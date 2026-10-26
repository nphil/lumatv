"""Run with: python3 -m unittest tests.test_ls_import   (the host check needs Xvfb and PyQt5; skipped without them)

On the set `LS` (LG's luna services object) is a singleton of the QML module `WebOSCompositorBase`; `WebOSServices` has none, so a file that
imports only WebOSServices and uses `LS` fails at runtime with "LS is not defined". The offline host's stand-in therefore puts `LS` in
WebOSCompositorBase too, and this holds both ends: no compositor file uses `LS` without the import, and a file that does fails on the host.
"""
import os
import re
import subprocess
import sys
import unittest

from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
COMPOSITOR = os.path.join(ROOT, "payload", "compositor")
STUBS = os.path.join(HERE, "qmlhost", "stubs")
HOST = os.path.join(HERE, "qmlhost", "qmlhost.py")

IMPORT = re.compile(r"^import WebOSCompositorBase \d+\.\d+\s*$", re.M)
# a comment or a string is not a use: `"LS.adhoc"` in a log line and the notes above a function name `LS` without calling it
NOT_CODE = re.compile(r"//[^\n]*|/\*.*?\*/|\"(?:\\.|[^\"\\\n])*\"|'(?:\\.|[^'\\\n])*'", re.S)
USE = re.compile(r"(?<![\w.])LS\s*[.\[]")


def qml_files():
    for dirpath, _, names in os.walk(COMPOSITOR):
        for n in sorted(names):
            if n.endswith((".qml", ".js")):
                yield os.path.join(dirpath, n)


def uses_ls(path):
    with open(path, encoding="utf-8") as f:
        return bool(USE.search(NOT_CODE.sub("", f.read())))


class LsImport(unittest.TestCase):
    def test_every_file_that_uses_ls_imports_webosCompositorBase(self):
        users = [p for p in qml_files() if uses_ls(p)]
        self.assertGreater(len(users), 10, "the scan found almost no use of LS: the pattern is wrong")
        bad = []
        for p in users:
            if p.endswith(".js"):
                bad.append(p)               # a script library sees only what it imports itself; LS belongs to the QML file that calls it
                continue
            with open(p, encoding="utf-8") as f:
                if not IMPORT.search(f.read()):
                    bad.append(p)
        self.assertEqual([os.path.relpath(p, ROOT) for p in bad], [], "use LS without `import WebOSCompositorBase 1.0`")

    def test_the_host_has_ls_in_webosCompositorBase_and_not_in_webosServices(self):
        with open(os.path.join(STUBS, "WebOSCompositorBase", "qmldir")) as f:
            self.assertRegex(f.read(), r"(?m)^singleton LS \d+\.\d+ \S*LS\.qml$")
        with open(os.path.join(STUBS, "WebOSServices", "qmldir")) as f:
            self.assertNotRegex(f.read(), r"(?m)^(singleton )?LS ")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class LsMissingImportFailsOnTheHost(unittest.TestCase):
    def host(self, *args):
        r = subprocess.run([sys.executable, HOST, "panels/Drawer.qml", "--wait", "1500", *args],
                           env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    def test_a_file_that_lost_the_import_warns_that_ls_is_not_defined(self):
        # The Notification Centre builds kit/NoteLog.qml, which binds LS.appId as it loads (the line the set's log names)
        rc, out = self.host()
        self.assertEqual(rc, 0, out[-1500:])
        rc, out = self.host("--drop-import", "kit/NoteLog.qml:WebOSCompositorBase")
        self.assertIn("NoteLog.qml:57", out)
        self.assertIn("LS is not defined", out)
        self.assertNotEqual(rc, 0, out[-1500:])


if __name__ == "__main__":
    unittest.main()
