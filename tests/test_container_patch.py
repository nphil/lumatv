"""Run with: python3 -m unittest discover -s tests -p 'test_container_patch.py'

`payload/compositor/StarfishFullscreenContainer.qml` is LG's own file with LumaTV's layers loaded into it (LumaHome and the overlay over an
app). It is the one file of LG's whose breaking would leave no fullscreen view, so it is held to two rules without the set:

- it is LG's file and nothing else: every line of LG's copy is in it, in order, and what is added is LumaTV's (skipped where LG's copy
  is not on this workstation: `/data/home/lgtv/uiinv`, the firmware's QML as read from the set);
- it parses: the QML parser reports no syntax error (the modules LG's file imports are not here, so the load itself fails on those and
  on nothing else).
"""
import difflib
import os
import re
import unittest

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
PATCHED = os.path.join(ROOT, "payload", "compositor", "StarfishFullscreenContainer.qml")
STOCK = [p for p in ("/data/home/lgtv/uiinv/qml/qml/WebOSCompositor/views/fullscreen/StarfishFullscreenContainer.qml",
                     "/tmp/tvsrc/qml/WebOSCompositor/views/fullscreen/StarfishFullscreenContainer.qml") if os.path.exists(p)]

try:
    from PyQt5 import QtCore, QtQml
    HAVE_QT = True
except ImportError:
    HAVE_QT = False


def read(path):
    with open(path, encoding="utf-8") as f:
        return f.read()


class Container(unittest.TestCase):
    @unittest.skipUnless(STOCK, "LG's copy of the container is not on this machine")
    def test_it_is_LGs_file_with_only_LumaTVs_layers_added(self):
        stock, patched = read(STOCK[0]).splitlines(), read(PATCHED).splitlines()
        ops = [o for o in difflib.SequenceMatcher(None, stock, patched, autojunk=False).get_opcodes() if o[0] != "equal"]
        self.assertTrue(ops, "nothing is added: the layers are gone")
        for tag, a1, a2, b1, b2 in ops:
            self.assertEqual(tag, "insert", "a line of LG's was changed or removed: %r -> %r" % (stock[a1:a2][:2], patched[b1:b2][:2]))
            block = "\n".join(patched[b1:b2])
            self.assertRegex(block, r"(?i)luma|import TextFile", "an addition that is not LumaTV's: %r" % block[:120])

    @unittest.skipUnless(HAVE_QT, "needs PyQt5")
    def test_it_parses_and_loads_nothing_that_is_not_an_import_LGs_modules_lack_here(self):
        app = QtCore.QCoreApplication.instance() or QtCore.QCoreApplication([])
        text = re.sub(r"\{\{shortId\}\}", "lumatv", re.sub(r"\{\{qmlModule\}\}", "lumatv", read(PATCHED)))
        self.assertNotRegex(text, r"\{\{\w+\}\}", "a placeholder the build does not know")
        for name, source, expect_syntax_error in (("the file", text, False), ("a broken copy", text.replace("id: lumaOverlay", "id: lumaOverlay }"), True)):
            engine = QtQml.QQmlEngine()
            component = QtQml.QQmlComponent(engine)
            component.setData(source.encode("utf-8"), QtCore.QUrl("file:///tmp/StarfishFullscreenContainer.qml"))
            errors = [e.toString() for e in component.errors()]
            syntax = [e for e in errors if re.search(r"Syntax error|Expected token|Unexpected token|Unexpected end", e)]
            self.assertEqual(bool(syntax), expect_syntax_error, "%s: %r" % (name, errors[:3]))
            if not expect_syntax_error:
                self.assertTrue(errors, "it loaded without LG's modules: the check proves nothing")
                self.assertTrue(all(re.search(r"is not installed|no such directory|No such file", e) for e in errors), errors[:3])
        del app


if __name__ == "__main__":
    unittest.main()
