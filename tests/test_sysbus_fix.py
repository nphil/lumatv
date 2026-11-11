"""The repaired Luna binding's builder: only the known files are taken, and what it writes is whole or not there.

LG's module is not in the repository, so the builds here run on a stand-in with the expected hashes swapped."""
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "sysbus_fix.py")
sys.path.insert(0, os.path.join(ROOT, "tools"))

import sysbus_fix  # noqa: E402

NAMES = sorted((sysbus_fix.MODULE, sysbus_fix.LOADER, sysbus_fix.MANIFEST))
MODULE = bytes(range(256)) * 0x81
LOADER = b"module.exports = require('./webos-sysbus.node');\n"


def patched(data):
    out = bytearray(data)
    for offset, new in sysbus_fix.PATCHES:
        out[offset:offset + len(new)] = new
    return bytes(out)


def source(d, module=MODULE, loader=LOADER):
    src = os.path.join(d, "src")
    os.makedirs(src, exist_ok=True)
    for name, data in ((sysbus_fix.MODULE, module), (sysbus_fix.LOADER, loader)):
        with open(os.path.join(src, name), "wb") as f:
            f.write(data)
    return src


def run(*args):
    return subprocess.run([sys.executable, TOOL] + list(args), capture_output=True, text=True)


class RealHashes(unittest.TestCase):
    def test_other_files_are_refused(self):
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "out")
            r = run("build", source(d), out)
            self.assertEqual(r.returncode, 1)
            self.assertIn("sysbus_fix:", r.stderr)
            self.assertFalse(os.path.exists(out))
            self.assertEqual(run("verify", out).returncode, 1)


class StandIn(unittest.TestCase):
    def setUp(self):
        hashes = mock.patch.multiple(sysbus_fix, SOURCE=sysbus_fix.sha(MODULE), FIXED=sysbus_fix.sha(patched(MODULE)),
                                     LOADER_SHA=sysbus_fix.sha(LOADER))
        hashes.start()
        self.addCleanup(hashes.stop)
        d = tempfile.TemporaryDirectory()
        self.addCleanup(d.cleanup)
        self.dir = d.name
        self.out = os.path.join(self.dir, "out")

    def test_any_changed_byte_fails_verify_and_a_rebuild_restores_it(self):
        src = source(self.dir)
        sysbus_fix.build(src, self.out)
        for name in NAMES:
            path = os.path.join(self.out, name)
            with open(path, "r+b") as f:
                first = f.read(1)
                f.seek(0)
                f.write(bytes([first[0] ^ 1]))
            with self.assertRaises(ValueError, msg=name):
                sysbus_fix.verify(self.out)
            sysbus_fix.build(src, self.out)
            sysbus_fix.verify(self.out)
        self.assertEqual(sorted(os.listdir(self.out)), NAMES)

    def test_a_missing_file_fails_verify(self):
        sysbus_fix.build(source(self.dir), self.out)
        os.remove(os.path.join(self.out, sysbus_fix.MANIFEST))
        with self.assertRaises(OSError):
            sysbus_fix.verify(self.out)

    def test_a_refused_build_leaves_the_last_good_copy(self):
        sysbus_fix.build(source(self.dir), self.out)
        for other in ({"module": MODULE + b"\0"}, {"loader": LOADER + b"\n"}):
            with self.assertRaises(ValueError):
                sysbus_fix.build(source(self.dir, **other), self.out)
            sysbus_fix.verify(self.out)
            self.assertEqual(sorted(os.listdir(self.out)), NAMES)

    def test_a_result_with_another_hash_is_not_written(self):
        with mock.patch.object(sysbus_fix, "FIXED", sysbus_fix.sha(MODULE)):
            with self.assertRaises(ValueError):
                sysbus_fix.build(source(self.dir), self.out)
        self.assertFalse(os.path.exists(self.out))


if __name__ == "__main__":
    unittest.main()
