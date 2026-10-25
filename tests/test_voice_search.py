"""Run with: python3 -m unittest discover -s tests -p 'test_voice_search.py'   (needs Xvfb and PyQt5; skipped without them)

Voice search on Home, through the offline host and its stand-in for the helper (tests/qmlhost/fakehelper.py, whose `voice` routes
answer as the helper's /v1/voice/* do): the mic key is Home's while the helper says voice search is ready and LG's otherwise,
pressing it opens the voice bar and starts the helper, releasing it ends the request, and the words land in Search's box and
nowhere else. Nothing here measures time: the set's gate does.
"""
import json
import re
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

import test_shell_surfaces as shell
import test_settings as settings

READY = shell.READY
MIC = 16777316                                 # the host's stand-in for WebOS.Key_webOS_Voice (stubs/WebOS/Global)
VOICE = {"state": "idle", "session": 0, "ready": True}
STATE = shell.STATE
# the voice bar's own state and what it shows, from the surface that has `micOpen`
BAR = shell.probe("micOpen", "{ state: s.state, text: s.text, mic: s.micOpen }")
# Search's typed text and whether it says something can hear (the mic hint)
SEARCH = shell.probe("symbols", "{ text: s.text, voice: s.voice }")
POSTS = ("(function(){ var x = new XMLHttpRequest(); x.open('GET', 'http://127.0.0.1:18731/__posts', false); x.send(); "
         "return JSON.stringify(JSON.parse(x.responseText).posts.map(function(p) { return p[0] })) })()")
LAUNCHES = "JSON.stringify(root.lunaCalls.filter(function(c) { return c.method === '/launch' || c.method === '/launchDefaultApp' }).map(function(c) { return c.payload }))"


def press(at, label, down=True, up=True):
    """The mic key as the key filter hands it over, a press and a release; what `root.key` answers to each (true: Home took it)."""
    body = ("r.push(String(root.key(%d,true,false,0)));" % MIC if down else "") + ("r.push(String(root.key(%d,false,false,0)));" % MIC if up else "")
    return ["--eval", "%d:(function(){ var r = []; %s return '%s=' + r.join(',') })()" % (at, body, label)]


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class VoiceSearch(HostCase):
    CODES = shell.KEYS

    def go(self, args, wait):
        """Runs the host with the fake helper and `args`; returns {label: value} of the `label=...` evals."""
        rc, out = self.host(*args, "--wait", str(wait))
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
        self.assertEqual(rc, 0, "\n".join(stray) + "\n" + out[-1500:])
        self.assertEqual(stray, [], out[-2000:])
        got = {}
        for v in self.values(out):
            m = re.match(r"^([a-z0-9_]+)=(.*)$", v, re.S)
            if m:
                got[m.group(1)] = m.group(2)
        return got

    def ev(self, at, label, expr):
        return ["--eval", "%d:(function(){ return %s + (%s) })()" % (at, json.dumps(label + "="), expr)]

    def publish(self, at, doc):
        return ["--publish", "%d:voice:%s" % (at, json.dumps(doc))]

    def test_the_words_fill_search_and_nothing_runs(self):
        # "open netflix" is a command in anyone's mouth; here it is only text in the search box
        say = dict(VOICE, say={"text": "open netflix", "delayMs": 600})
        args = [*self.publish(600, say), *press(READY, "down", up=False), *self.ev(READY + 400, "listening", BAR), *self.ev(READY + 400, "posts1", POSTS),
                *press(READY + 500, "up", down=False), *self.ev(READY + 800, "working", BAR), *self.ev(READY + 2500, "state", STATE),
                *self.ev(READY + 2500, "search", SEARCH), *self.ev(READY + 2500, "bar", BAR), *self.ev(READY + 2500, "launches", LAUNCHES),
                *self.ev(READY + 2500, "posts2", POSTS)]
        got = self.go(args, READY + 2900)
        self.assertEqual(got["down"], "true", "Home took the press")
        self.assertEqual(got["up"], "true", "and the release")
        self.assertEqual(json.loads(got["listening"]), {"state": "listening", "text": "", "mic": True}, "the bar and the mic indicator are up while the key is down")
        self.assertEqual(json.loads(got["posts1"]), ["/v1/voice/listen"])
        self.assertEqual(json.loads(got["working"])["state"], "working", "the key is up: the bar says it is working on it")
        self.assertEqual(json.loads(got["posts2"]), ["/v1/voice/listen", "/v1/voice/stop"])
        self.assertEqual(got["state"].split("/")[0], "search", "the words open Search")
        self.assertEqual(json.loads(got["search"]), {"text": "open netflix", "voice": True}, "the text is in the box, and the box says something can hear")
        self.assertEqual(json.loads(got["bar"]), {"state": "", "text": "", "mic": False}, "the bar and the mic indicator are gone")
        self.assertEqual(json.loads(got["launches"]), [], "voice ran nothing: no app was launched")

    def test_the_key_is_lgs_while_voice_search_is_not_ready(self):
        args = [*self.publish(600, dict(VOICE, ready=False, reason="voice_off")), *press(READY, "mic"), *self.ev(READY + 600, "state", STATE),
                *self.ev(READY + 600, "posts", POSTS)]
        got = self.go(args, READY + 900)
        self.assertEqual(got["mic"], "false,false", "Home did not take the key: LG's chain gets it")
        self.assertEqual(got["state"], "/dock/0/0/", "nothing opened")
        self.assertEqual(json.loads(got["posts"]), [], "the helper was not asked to listen")

    def test_nothing_heard_is_said_on_the_bar_and_the_bar_goes_by_itself(self):
        # the stand-in's default answer is "Nothing was heard"
        args = [*self.publish(600, VOICE), *press(READY, "down", up=False), *press(READY + 300, "up", down=False), *self.ev(READY + 900, "bar", BAR),
                *self.ev(READY + 900, "state", STATE), *self.ev(READY + 4000, "gone", BAR), *self.ev(READY + 4000, "state2", STATE)]
        got = self.go(args, READY + 4400)
        self.assertEqual(json.loads(got["bar"]), {"state": "failed", "text": "Nothing was heard", "mic": False})
        self.assertEqual(got["state"].split("/")[0], "voice")
        self.assertEqual(json.loads(got["gone"])["state"], "")
        self.assertEqual(got["state2"], "/dock/0/0/", "after the words are read the bar closes and Home has the keys again")

    def test_back_cancels_the_request_and_the_helper_is_told(self):
        say = dict(VOICE, say={"text": "never arrives", "delayMs": 30000})
        args = [*self.publish(600, say), *press(READY, "down", up=False), *press(READY + 300, "up", down=False), *self.ev(READY + 700, "working", BAR),
                *self.keys(READY + 800, "back"), *self.ev(READY + 1400, "state", STATE), *self.ev(READY + 1400, "bar", BAR),
                *self.ev(READY + 1400, "posts", POSTS)]
        got = self.go(args, READY + 1800)
        self.assertEqual(json.loads(got["working"])["state"], "working")
        self.assertEqual(got["state"], "/dock/0/0/")
        self.assertEqual(json.loads(got["bar"]), {"state": "", "text": "", "mic": False})
        self.assertEqual(json.loads(got["posts"]), ["/v1/voice/listen", "/v1/voice/stop", "/v1/voice/cancel"])

    def test_a_refusal_is_said_on_the_bar(self):
        # the topic says ready (so Home takes the key), the helper refuses the request: the bar says so instead of listening
        # (Qt's XMLHttpRequest gives Home no status and no body for a 409, as the host shows)
        refuse = dict(VOICE, refuse={"code": "busy", "message": "voice search is already listening"})
        args = [*self.publish(600, refuse), *press(READY, "down", up=False), *self.ev(READY + 500, "bar", BAR), *self.ev(READY + 500, "posts", POSTS)]
        got = self.go(args, READY + 900)
        self.assertEqual(json.loads(got["bar"]), {"state": "failed", "text": "Voice search is busy or not available", "mic": False})
        self.assertEqual(json.loads(got["posts"]), ["/v1/voice/listen"])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class VoiceSwitch(HostCase):
    """The Voice row of Settings, Remote & pointer: the helper's `base.voice` switch, and the helper's words for what is missing."""

    def page(self, *keys, publish=None, after=6600):
        rc, out = self.host("--eval", "%d:root.openSettings()" % settings.OPEN_AT,
                            *self.keys(settings.READY + 3000, *(["down"] * settings.RAIL["remote"])),
                            *self.keys(settings.READY + 4500, "right", *keys),
                            *(["--publish", "%d:voice:%s" % (settings.READY + 4000, json.dumps(publish))] if publish else []),
                            "--eval", "%d:%s" % (settings.READY + 5800, settings.POSTS),
                            "--eval", "%d:%s" % (settings.READY + 5900, NOTE),
                            "--wait", str(settings.READY + after))
        self.assertEqual(rc, 0, out[-1500:])
        return self.tagged(out, "posts")[-1]["posts"], self.docs(out)[-1]

    def test_the_switch_is_a_helper_setting_and_changes_nothing_else(self):
        # pointer, speed, size, number keys, then the voice switch (it is off by default)
        posts, _ = self.page("down", "down", "down", "down", "ok")
        self.assertEqual([p for p in posts if p[0] == "/v1/settings/base.voice"],
                         [["/v1/settings/base.voice", {"changes": [{"key": "enabled", "value": {"type": "bool", "value": True}}]}]])
        self.assertEqual([p[0] for p in posts if "voice" in p[0] and p[0] != "/v1/settings/base.voice"], [], "the page never starts a request")

    def test_what_is_missing_is_said_under_the_switch_once_it_is_on(self):
        posts, note = self.page("down", "down", "down", "down", "ok", publish={"state": "idle", "session": 0, "ready": False, "reason": "voice_unlinked"})
        self.assertIn("not linked", note["text"])
        self.assertTrue(note["shown"], "voice search is on and cannot listen: the page says why")


# the note under the switch, from the page that has `voiceNeeds`
NOTE = shell.probe("voiceNeeds", "{ text: s.voiceNeeds, shown: s.voiceOn && s.voiceNeeds !== '' }", panel=False)


if __name__ == "__main__":
    unittest.main()
