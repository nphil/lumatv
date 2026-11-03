"""The native cutover retires LG QML restyles but keeps safe stock recovery."""
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "tools" / "lumatv"


def function(source, name):
    match = re.search(r"^" + re.escape(name) + r"\(\) \{.*?^\}", source, re.M | re.S)
    if match is None:
        raise AssertionError("missing shell function " + name)
    return match.group()


class NativeInputCutover(unittest.TestCase):
    def test_retired_restyle_commands_and_shadow_builder_are_not_dispatched(self):
        source = CLI.read_text()
        dispatch = source[source.rfind('\ncase "$VERB" in'):]
        for name in ("inputcommon", "voice", "keyboard"):
            self.assertNotRegex(dispatch, rf"(?m)^\s*{name}\)")
            self.assertNotRegex(source, rf"(?m)^verb_{name}\(\)")
        self.assertNotRegex(source, r"(?m)^build_shadow\(\)")

    def test_stock_recovery_attempts_both_legacy_locations_even_without_flags(self):
        with tempfile.TemporaryDirectory() as d:
            root = Path(d)
            (root / "stock-ui-held").touch()
            calls = root / "calls"
            source = CLI.read_text()
            script = '''
comp_mounted() { return 1; }
hs_wd_stop() { :; }
hs_mounted_ours() { return 1; }
hs_mode_set() { :; }
webglass_apps() { :; }
stock_ui_unmount_overlay() {
  printf '%s %s\\n' "$1" "$2" >> "$CALLS"
  [ "$1" != "${FAIL_TARGET:-}" ]
}
stock_ui_restore_env() { :; }
timeout() { case "$4" in status) printf '{}';; esac; }
emit_error() { printf 'failed\\n'; }
''' + function(source, "stock_ui_restore") + '\nstock_ui_restore\n'
            env = dict(os.environ, COMP=d, STATE=d, CALLS=str(calls),
                       ASSETS=str(root / "absent-assets"), STOCK_ASSETS=str(root / "absent-stock"),
                       WEBGLASS_STAGE=str(root / "webapps"), KEYFILTER_TOOL="keyfilter",
                       KEYFILTER_HANDLER="handler", KEYFILTER_DIR=d, SHORT_ID="lumatv",
                       SYSTEMUI_TOOL="systemui", SYSTEMUI_STAGE=str(root / "systemui"),
                       QS_APP="quick", LOGO_BOOT_APP="boot", LOGO_OFF_APP="off",
                       NOTI_STAGE=str(root / "noti"), NOTI_PERSIST=str(root / "persist"))
            targets = [f"{base}/com.webos.app.{app}/qml"
                       for app in ("inputcommon", "voice")
                       for base in ("/usr/palm/applications", "/usr/share")]
            for failed in (None, targets[0]):
                calls.unlink(missing_ok=True)
                result = subprocess.run(["sh", "-eu", "-c", script],
                                        env=dict(env, FAIL_TARGET=failed or ""),
                                        capture_output=True, text=True)
                self.assertEqual(result.returncode == 0, failed is None, result.stderr)
                entries = calls.read_text().splitlines()
                for target in targets:
                    app = "inputcommon" if "inputcommon" in target else "voice"
                    self.assertIn(f"{target} {d}/{app}/upper", entries)


if __name__ == "__main__":
    unittest.main()
