"""Run with: cd tests && python3 -m unittest test_shell_cards   (needs Xvfb and PyQt5; skipped without them)

The overlay system's cards through the shell client (compositor/shell/Shell.qml) against a stand-in of the compositor's UI channel
(tests/qmlhost/shellhost.py): while an app is in front and a card has been up long enough, the client tells the compositor it takes OK and Back
(`takes` in its report) and does not ask for the keyboard focus (`wants` stays false: the app keeps its arrows); a `syskey` event, which is what
the compositor sends for a key it took, is the card's key (Back dismisses it, and the report says it takes nothing any more); and with the
switch off the client never asks for a key.
"""
import json
import os
import re
import subprocess
import sys
import unittest

from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
HOST = os.path.join(HERE, "qmlhost", "shellhost.py")
APP = '{"app":"netflix","home":false}'
ON = '{"perf":{"syncResidents":true},"overlays":{"enabled":true,"armMs":300}}'
OFF = '{"perf":{"syncResidents":true}}'
ROSTER = {"plugins": [{"id": "ha", "name": "Home Assistant", "version": "0.2.0", "class": "L", "state": "connected", "registered": True}], "firstParty": [], "problem": None}
NOTE = {"id": "low", "channel": "ha", "channelTitle": "Home Assistant", "channelIcon": "base.icons/home", "revision": 1, "receivedAt": 0, "expiresAt": None,
        "post": {"id": "low", "channel": "ha", "title": "Low battery", "body": "Door sensor is at 5 %", "privacy": "public", "ongoing": False}}


def publish(topic, doc):
    return ("(function(){var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/%s');"
            "x.setRequestHeader('Content-Type','application/json'); x.send(%s); return 1})()" % (topic, json.dumps(json.dumps(doc))))


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class ShellCards(unittest.TestCase):
    def host(self, *args, wait):
        env = dict(os.environ, DISPLAY=DISPLAY, QT_QPA_PLATFORM="offscreen")
        cmd = [sys.executable, HOST, "--fake-helper", "--wait", str(wait), *args]
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get("HOST_TIMEOUT", "240")))
        return r.returncode, r.stdout + r.stderr

    def reports(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[report\] (.*)$", out, re.M)]

    def evals(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"\[eval\] \('(.*)', (?:True|False)\)$", out, re.M)]

    def run_cards(self, theme, *extra, wait=16000):
        steps = ["--push", "500:foreground:" + APP, "--push", "500:screen:{\"on\":true}", "--theme", theme,
                 "--eval", "2500:" + publish("plugins", ROSTER), "--eval", "2600:" + publish("plugin.ha.notifications", {"items": []}),
                 "--eval", "11500:" + publish("plugin.ha.notifications", {"items": [NOTE]}),
                 "--eval", "12400:JSON.stringify(home.state().cards)"]
        return self.host(*steps, *extra, wait=wait)

    def test_a_card_over_an_app_makes_the_client_ask_for_ok_and_back_and_never_for_the_focus(self):
        rc, out = self.run_cards(ON, "--eval", "13400:JSON.stringify(home.state().cards)")
        cards = self.evals(out)
        self.assertTrue(cards, out[-2500:])
        self.assertEqual(cards[0]["stack"], True, "the card is up: " + out[-1500:])
        self.assertTrue(cards[0]["on"])
        with_takes = [r for r in self.reports(out) if r.get("takes") == ["ok", "back"]]
        self.assertTrue(with_takes, "the client asked for OK and Back once the card had been up a moment")
        self.assertTrue(all(r["wants"] is False for r in with_takes), "and not for the keyboard focus")
        self.assertTrue(with_takes[0]["live"], "the layer is drawn for the card")
        # before it is armed (600 ms is the default, 300 here) the client asks for nothing
        first = [r for r in self.reports(out) if r["state"].get("cards") and r["state"]["cards"]["stack"]]
        self.assertTrue(first and first[0]["takes"] == [], "a press made as the card arrives is the app's: " + str(first[:1]))

    def test_a_syskey_event_is_the_cards_key_back_dismisses_and_the_client_lets_go_of_the_keys(self):
        rc, out = self.run_cards(ON, "--push", "13000:syskey:{\"k\":\"back\",\"d\":true,\"r\":false}", "--push", "13100:syskey:{\"k\":\"back\",\"d\":false,\"r\":false}",
                                 "--eval", "14500:JSON.stringify(home.state().cards)")
        cards = self.evals(out)
        self.assertEqual(len(cards), 2, out[-2500:])
        self.assertTrue(cards[0]["stack"])
        self.assertFalse(cards[1]["stack"], "Back, as the compositor sends it, dismissed the card")
        last = self.reports(out)[-1]
        self.assertEqual(last["takes"], [], "with no card left the client asks for nothing")
        self.assertEqual(last["state"]["dropped"], 1, "the release of a key that was taken is the only one nobody answered")

    def test_with_the_switch_off_the_client_never_asks_for_a_key(self):
        rc, out = self.run_cards(OFF, "--push", "13000:syskey:{\"k\":\"back\",\"d\":true,\"r\":false}")
        cards = self.evals(out)
        self.assertTrue(cards, out[-2500:])
        self.assertFalse(cards[0]["on"])
        self.assertFalse(cards[0]["stack"])
        self.assertTrue(all(r.get("takes") == [] for r in self.reports(out)))


if __name__ == "__main__":
    unittest.main()
