"""Run with: cd tests && python3 -m unittest test_rust_settings   (needs rustui/target/debug/lumatv-ui and Mesa; skipped without them)

The pages of the Rust shell's Settings that the Rust unit tests cannot draw (their documents are the fake helper's and LG's, in
render_rust_settings.py) read clean to the host's layout and contrast lint, and a focused row is not cut by its list's clip, at the
larger text in the light look: what a key sends the helper is tested in rustui/ui/src/settings/pages/{ai,privacy,homeassistant,cameras}.rs.
"""
import concurrent.futures
import json
import os
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import render_rust_settings as rr  # noqa: E402
import rust_settings_host as host  # noqa: E402

# one scene of each page that has no lint in cargo test: AI access (tabs and the assistants list), Home Assistant, a camera's kinds, Privacy
PAGES = ['ai', 'ai-assistants', 'homeassistant', 'cameras-notify', 'camera-kinds', 'privacy-prime']


@unittest.skipUnless(host.available(), 'needs rustui/target/debug/lumatv-ui and Mesa')
class RustSettings(unittest.TestCase):
    def test_the_pages_without_a_cargo_lint_read_clean_with_the_focused_row_whole_at_the_larger_text_in_the_light_look(self):
        def one(name):
            rc, text, findings, png = rr.render(rr.BY_NAME[name], 'light', 2.0)
            return name, rc, findings, png, text

        with concurrent.futures.ThreadPoolExecutor(len(PAGES)) as pool:
            for name, rc, findings, png, text in pool.map(one, PAGES):
                self.assertEqual(rc, 0, '%s\n%s' % (name, text[-1500:]))
                self.assertIsNotNone(png, name)
                self.assertEqual(findings, [], '%s: %s' % (name, json.dumps(findings)))


if __name__ == '__main__':
    unittest.main()
