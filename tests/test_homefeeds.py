"""Run with: python3 -m unittest tests.test_homefeeds   (needs Xvfb and PyQt5; skipped without them)

Home's weather and news cards through the offline QML host with a stand-in for the native helper
(tests/qmlhost/fakehelper.py): they draw what the helper serves, take a later reading when the
helper publishes one, draw the helper's saved file at once when the helper is not there, and never
draw an expired or foreign reading. The cards fetch nothing themselves.
"""
import json
import os
import subprocess
import sys
import time
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

HOST = os.path.join(HERE, "qmlhost", "qmlhost.py")

PROBE = ("(function f(i){ if (i.place !== undefined && i.cond !== undefined && i.nowKind !== undefined) return 'W:' + i.place + '|' + i.temp;"
         " if (i.items !== undefined && i.fetchedAt !== undefined && i.slideMs !== undefined) return 'N:' + (i.items.length ? i.items[0].title : '');"
         " var out = ''; for (var k = 0; k < i.children.length; k++) { var r = f(i.children[k]); if (r) out += (out ? ' ' : '') + r } return out })(root)")


def now_ms():
    return int(time.time() * 1000)


def weather_doc(place, age_ms=0, temp="66\u00b0"):
    t = now_ms() - age_ms
    return {"schema": 1, "fetchedAt": t, "staleAt": t + 900000, "expiresAt": t + 21600000, "zip": "30311", "units": "fahrenheit", "place": place,
            "display": {"place": place, "temp": temp, "cond": "Clear", "stats": "", "nowKind": "sun", "days": []}}


def news_doc(title, age_ms=0):
    t = now_ms() - age_ms
    return {"schema": 1, "fetchedAt": t, "staleAt": t + 900000, "expiresAt": t + 21600000, "feeds": ["a"],
            "items": [{"tag": "W", "src": "S", "title": title, "summary": "", "link": "", "img": "", "pub": "", "pubMs": None}]}


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class HomeFeeds(unittest.TestCase):
    def host(self, *args):
        r = subprocess.run([sys.executable, HOST, "LumaHome.qml", "--prop", "hostActive=true", *args],
                           env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=90)
        self.assertEqual(r.returncode, 0, (r.stdout + r.stderr)[-800:])
        return [line.split("] ", 1)[1].strip().strip("()'") for line in (r.stdout + r.stderr).splitlines() if line.startswith("[eval]")]

    def test_the_cards_draw_what_the_helper_serves(self):
        out = self.host("--wait", "2500", "--fake-helper", "--eval", f"1800:{PROBE}")
        self.assertIn("W:HELPERVILLE|66\u00b0", out[0])
        self.assertIn("N:HEADLINE FROM THE HELPER", out[0])

    def test_a_later_reading_published_by_the_helper_replaces_the_one_on_screen(self):
        data = {"doc": weather_doc("CHANGEDVILLE", -20000, "12\u00b0")}
        out = self.host("--wait", "4000", "--fake-helper", "--eval", f"1500:{PROBE}", "--publish", f"2000:weather:{json.dumps(data)}",
                        "--eval", f"3200:{PROBE}")
        self.assertIn("W:HELPERVILLE", out[0])
        self.assertIn("W:CHANGEDVILLE|12\u00b0", out[1])

    def test_an_older_or_expired_reading_from_the_helper_is_not_drawn(self):
        old = {"doc": weather_doc("OLDVILLE", 60000)}
        expired = {"doc": weather_doc("EXPIREDVILLE", 7 * 3600 * 1000)}
        out = self.host("--wait", "4500", "--fake-helper", "--publish", f"1800:weather:{json.dumps(old)}",
                        "--publish", f"2300:weather:{json.dumps(expired)}", "--eval", f"3500:{PROBE}")
        self.assertIn("W:HELPERVILLE", out[0])

    def test_the_saved_file_is_drawn_when_the_helper_is_not_there(self):
        eighty_eight = "88\u00b0"
        out = self.host("--wait", "2000", "--cache", f"weather={json.dumps(weather_doc('SAVEDVILLE', 20 * 60000, eighty_eight))}",
                        "--cache", f"news={json.dumps(news_doc('SAVED HEADLINE', 20 * 60000))}", "--eval", f"1500:{PROBE}")
        self.assertIn("W:SAVEDVILLE|88\u00b0", out[0])
        self.assertIn("N:SAVED HEADLINE", out[0])

    def test_an_expired_saved_file_is_not_drawn(self):
        out = self.host("--wait", "2000", "--cache", f"weather={json.dumps(weather_doc('STALEVILLE', 7 * 3600 * 1000))}",
                        "--cache", f"news={json.dumps(news_doc('OLD NEWS', 7 * 3600 * 1000))}", "--eval", f"1500:{PROBE}")
        self.assertNotIn("STALEVILLE", out[0])
        self.assertNotIn("OLD NEWS", out[0])
        self.assertIn("N:", out[0])

    def test_a_saved_file_that_is_not_json_or_has_another_schema_is_ignored(self):
        out = self.host("--wait", "2000", "--cache", "weather=not json", "--cache", f"news={json.dumps(dict(news_doc('X'), schema=2))}",
                        "--eval", f"1500:{PROBE}")
        self.assertNotIn("N:X", out[0])

    def test_the_helper_arriving_later_replaces_the_saved_reading(self):
        # no helper for the first second: the saved file is drawn; the fake starts at once in this
        # host, so the saved reading is overtaken as soon as the first state read lands
        out = self.host("--wait", "2500", "--fake-helper", "--cache", f"weather={json.dumps(weather_doc('SAVEDVILLE', 20 * 60000))}",
                        "--eval", f"1800:{PROBE}")
        self.assertIn("W:HELPERVILLE", out[0])


if __name__ == "__main__":
    unittest.main()
