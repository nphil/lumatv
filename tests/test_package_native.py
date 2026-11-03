"""The native package must not ship the retired project QML frontend."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools" / "build-ipk.sh"


class NativePackage(unittest.TestCase):
    def test_payload_staging_contains_no_project_qml(self):
        script = SCRIPT.read_text()
        stage = script[script.index('# Native UI assets are bundled'):script.index('cp tools/brand.sh "$DATA_ROOT/tools/"')]
        with tempfile.TemporaryDirectory() as temp:
            env = dict(os.environ, DATA_ROOT=temp, BRAND_QML_MODULE="lumatv")
            subprocess.run(['sh', '-e', '-c', stage], cwd=ROOT, env=env,
                           check=True, capture_output=True, text=True)
            payload = Path(temp) / 'payload'
            self.assertEqual(list(payload.rglob('*.qml')), [])
            self.assertFalse((payload / 'home').exists())
            self.assertTrue((payload / "compositor/lumatv/logo/arc.png").is_file())
            self.assertTrue((payload / "compositor/lumatv/icons/audio/index.json").is_file())


if __name__ == '__main__':
    unittest.main()
