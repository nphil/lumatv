"""Run with: python3 -m unittest tests.test_qml_compile   (needs PyQt5; no display)

Every QML file under payload/compositor/lumatv compiles with the set's import layout (tests/qmlhost/compile_all.py), and the check
fails on the faults that have stopped a surface from building on the TV: an object after the root's closing brace (Settings.qml, which
left the TV on a bare wallpaper), an import of a module the set does not have, and an object a type cannot hold.
"""
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
COMPILE = os.path.join(HERE, "qmlhost", "compile_all.py")
SRC = os.path.join(ROOT, "payload", "compositor", "lumatv")

try:
    import PyQt5.QtQml  # noqa: F401
    HAVE_QT = True
except ImportError:
    HAVE_QT = False


def compile_tree(src):
    r = subprocess.run([sys.executable, COMPILE, "--src", src], capture_output=True, text=True, timeout=120)
    return r.returncode, r.stdout + r.stderr


@unittest.skipUnless(HAVE_QT, "needs PyQt5")
class QmlCompile(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="qmlcompile-test-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.src = os.path.join(self.tmp, "lumatv")
        shutil.copytree(SRC, self.src)

    def append(self, rel, text):
        with open(os.path.join(self.src, rel), "a", encoding="utf-8") as f:
            f.write(text)

    def test_the_surface_compiles(self):
        rc, out = compile_tree(SRC)
        self.assertEqual(rc, 0, out[-3000:])
        self.assertRegex(out, r"\d{2,} QML file\(s\) compiled, 0 failed")

    def test_an_object_after_the_roots_closing_brace_fails(self):
        self.append("Settings.qml", "\nLuHelp { id: helpSheet }\n")
        rc, out = compile_tree(self.src)
        self.assertEqual(rc, 1, out[-1500:])
        self.assertIn("FAIL Settings.qml", out)

    def test_an_import_of_a_module_the_set_lacks_fails(self):
        with open(os.path.join(self.src, "kit", "Apps.qml"), encoding="utf-8") as f:
            text = f.read()
        with open(os.path.join(self.src, "kit", "Apps.qml"), "w", encoding="utf-8") as f:
            f.write(text.replace("import QtQuick 2.4\n", "import QtQuick 2.4\nimport NoSuchModule 1.0\n", 1))
        rc, out = compile_tree(self.src)
        self.assertEqual(rc, 1, out[-1500:])
        self.assertIn("FAIL kit/Apps.qml", out)

    def test_an_object_the_type_cannot_hold_fails(self):
        with open(os.path.join(self.src, "kit", "Apps.qml"), encoding="utf-8") as f:
            text = f.read()
        body = text.rindex("}")
        with open(os.path.join(self.src, "kit", "Apps.qml"), "w", encoding="utf-8") as f:
            f.write(text[:body] + "    Item { }\n" + text[body:])
        rc, out = compile_tree(self.src)
        self.assertEqual(rc, 1, out[-1500:])
        self.assertIn("FAIL kit/Apps.qml", out)
        self.assertIn("non-existent", out)


if __name__ == "__main__":
    unittest.main()
