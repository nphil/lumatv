"""The stock recovery puts the panel's brightness back from the compositor's journal, against a stand-in settings service."""
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
SCRIPT = (ROOT / "compositor" / "tools" / "tv-recover-home.sh").read_text()
LUNA_SEND = '''#!/bin/sh
echo "$*" >> "$FAKE/calls"
case "$*" in
*getSystemSettings*) [ -n "$FAKE_NOW" ] && echo "{\\"returnValue\\":true,\\"settings\\":{\\"backlight\\":\\"$FAKE_NOW\\"}}" ;;
*setSystemSettings*) if [ "$FAKE_REFUSE" = 1 ]; then echo '{"returnValue":false}'; else echo '{"returnValue":true}'; fi ;;
esac
'''


def function(name):
    match = re.search(r"^" + re.escape(name) + r"\(\) [\{\(].*?^[\}\)]|^" + re.escape(name) + r"\(\) \{.*?\}$", SCRIPT, re.M | re.S)
    if match is None:
        raise AssertionError("missing shell function " + name)
    return match.group()


class RecoverPanel(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="recover-panel-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        (self.tmp / "bin").mkdir()
        send = self.tmp / "bin" / "luna-send"
        send.write_text(LUNA_SEND)
        send.chmod(0o755)
        (self.tmp / "calls").write_text("")
        self.journal = self.tmp / "paneldim.json"
        self.journal.write_text(json.dumps({
            "at": 1, "dimension": {"input": "hdmi2", "pictureMode": "filmMaker"}, "orig": 75, "pid": 999999, "written": 30,
        }, sort_keys=True, separators=(",", ":")))

    def run_restore(self, now="30", refuse="0"):
        program = "\n".join([
            'say() { echo "$*" >> "$FAKE/said"; }',
            "ls_ok() { grep -q '\"returnValue\"[[:space:]]*:[[:space:]]*true'; }",
            "bounded() { shift; \"$@\"; }",
            'PANELDIM_JOURNAL="$LUMATV_PANELDIM_JOURNAL"',
            function("journal_num"), function("journal_str"), function("restore_panel"),
            "restore_panel",
        ])
        env = dict(os.environ, FAKE=str(self.tmp), FAKE_NOW=now, FAKE_REFUSE=refuse, LUMATV_PANELDIM_JOURNAL=str(self.journal),
                   PATH=str(self.tmp / "bin") + os.pathsep + os.environ["PATH"])
        subprocess.run(["sh", "-c", program], env=env, check=True, timeout=20)
        said = (self.tmp / "said").read_text() if (self.tmp / "said").exists() else ""
        return said, (self.tmp / "calls").read_text()

    def test_a_dim_left_by_a_killed_compositor_is_put_back_with_its_picture_mode(self):
        said, calls = self.run_restore()
        self.assertIn("put back to 75 from the dim's 30", said)
        self.assertIn('"backlight":"75"', calls)
        self.assertIn('"dimension":{"pictureMode":"filmMaker","input":"hdmi2"}', calls)
        self.assertFalse(self.journal.exists())

    def test_a_value_someone_else_changed_is_left_alone_and_the_journal_goes(self):
        said, calls = self.run_restore(now="60")
        self.assertIn("left alone", said)
        self.assertNotIn("setSystemSettings", calls)
        self.assertFalse(self.journal.exists())

    def test_a_running_compositor_owns_its_journal(self):
        live = subprocess.Popen(["bash", "-c", "exec -a lumatv-compositor sleep 30"])
        self.addCleanup(live.kill)
        data = json.loads(self.journal.read_text())
        data["pid"] = live.pid
        self.journal.write_text(json.dumps(data))
        said, calls = self.run_restore()
        self.assertEqual((said, calls), ("", ""))
        self.assertTrue(self.journal.exists())

    def test_a_setting_that_cannot_be_read_or_written_leaves_the_journal_for_the_helper(self):
        said, calls = self.run_restore(now="")
        self.assertIn("not read", said)
        self.assertTrue(self.journal.exists())
        (self.tmp / "said").unlink()
        said, calls = self.run_restore(refuse="1")
        self.assertIn("not put back", said)
        self.assertTrue(self.journal.exists())


if __name__ == "__main__":
    unittest.main()
