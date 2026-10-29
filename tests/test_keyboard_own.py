"""Run with: cd tests && python3 -m unittest test_keyboard_own   (needs Xvfb and PyQt5; skipped without them)

LumaTV's own on-screen keyboard (payload/compositor/lumatv/panels/Keyboard.qml) as the shell client runs it (compositor/shell/Shell.qml),
against the host's stand-in of the compositor's UI channel (tests/qmlhost/shellhost.py): the compositor "activates" a field by publishing the
`ime` topic, "presses" the remote by sending `imekey` events, and the keyboard answers with `POST /v1/ime` (printed as `[ime] {...}`). What the
real compositor does with those requests, and what it does with the field, is tested in compositor/crates/lumatv-compositor/tests/ime_own.rs;
the layouts and the D-pad over them in tests/keyboard.test.js. Nothing here measures time: the set's gate does.

A host run takes about fifteen seconds, so each test is one run that walks through a sequence.
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
READY = 8000                                      # Home has loaded and the system layer is warm

FIELD = {"active": True, "show": True, "model": 3, "app": "com.webos.app.voice", "hint": 7, "purpose": 0, "enter": 5, "max": 0, "text": False}
GONE = {"active": False, "show": False}


def push(at, topic, doc):
    return ["--push", "%d:%s:%s" % (at, topic, json.dumps(doc))]


class Script:
    """The remote's presses as `imekey` events, one every `gap` ms from `start`, and the state of the keyboard read after each step."""

    def __init__(self, start, gap=150):
        self.at = start
        self.gap = gap
        self.args = []
        self.reads = []

    def key(self, name):
        self.args += push(self.at, "imekey", {"k": name, "d": True, "r": False}) + push(self.at + 40, "imekey", {"k": name, "d": False, "r": False})
        self.at += self.gap

    def keys(self, *names):
        for n in names:
            self.key(n)

    def read(self, label):
        """The keyboard's state as the shell reports it, 60 ms after the last key."""
        self.args += ["--eval", "%d:JSON.stringify({label: %s, keyboard: home.state().keyboard})" % (self.at - self.gap + 70, json.dumps(label))]
        self.reads.append(label)


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class OwnKeyboard(unittest.TestCase):
    def host(self, *args, wait):
        env = dict(os.environ, DISPLAY=DISPLAY, QT_QPA_PLATFORM="offscreen")
        cmd = [sys.executable, HOST, "--wait", str(wait), "--push", "500:foreground:" + APP, "--push", "500:screen:" + json.dumps({"on": True}), *args]
        r = subprocess.run(cmd, env=env, capture_output=True, text=True, timeout=int(os.environ.get("HOST_TIMEOUT", "240")))
        return r.returncode, r.stdout + r.stderr

    def lines(self, out, tag):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[%s\] (.*)$" % tag, out, re.M)]

    def evals(self, out):
        """The states the script read, by label."""
        got = {}
        for m in re.finditer(r"\[eval\] \('(.*)', (?:True|False)\)$", out, re.M):
            doc = json.loads(m.group(1).replace("\\'", "'"))          # the host prints a Python repr: an apostrophe is escaped
            got[doc["label"]] = doc["keyboard"]
        return got

    def clean(self, rc, out):
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") or ln.startswith("[critical]")]
        stray = [ln for ln in stray if "Implicitly defined onFoo" not in ln]
        self.assertEqual(rc, 0, "\n".join(stray[:20]) + "\n" + out[-1500:])

    def test_the_client_offers_a_keyboard_and_the_remote_types_on_it(self):
        s = Script(READY + 1000)
        s.key("ok")                                          # g
        s.key("right"); s.key("ok")                          # h
        s.key("up"); s.key("ok")                             # y
        s.keys("down", "down", "ok")                         # n
        s.keys("up", "up", "left", "left", "left", "left", "left", "left")      # along the second row to Shift
        s.read("on_shift")
        s.key("ok")                                          # one capital
        s.read("shifted")
        s.keys("right", "ok")                                # Q, and the capital is used up
        s.read("after_q")
        s.keys("up")
        s.keys(*["right"] * 10)                              # along the number row to Backspace
        s.read("on_backspace")
        s.key("ok")
        s.keys("down", "down", "down")                       # Enter, the key that puts the keyboard away, Clear
        s.read("on_clear")
        s.key("ok")                                          # the field has no text: nothing happens
        clear_at = s.at
        s.args += push(clear_at, "ime", dict(FIELD, text=True))
        s.at += 200
        s.key("ok")                                          # now it does
        s.keys("up", "up")
        s.read("on_enter")
        s.key("ok")                                          # Search: Enter, and the keyboard goes away
        s.args += push(s.at, "ime", dict(FIELD, show=False))   # the compositor's answer to the hide
        s.at += 600
        s.args += push(s.at, "ime", FIELD)                   # the app asks again: a new session
        s.at += 600
        s.key("back")                                        # Back puts the keyboard away and the field stays
        s.read("after_back")
        end = s.at + 600
        rc, out = self.host("--push", "%d:ime:%s" % (READY, json.dumps(FIELD)), *s.args,
                            "--eval", "%d:JSON.stringify({label: 'early', keyboard: home.state().keyboard})" % (READY + 300), wait=end)
        self.clean(rc, out)
        reports = self.lines(out, "report")
        self.assertTrue(any("keyboard" in r["can"] for r in reports), "the client said it can show a keyboard: %s" % [r["can"] for r in reports][:4])
        posts = [{k: v for k, v in p.items() if k != "model"} for p in self.lines(out, "ime")]
        self.assertTrue(all(p.get("model") in (None, 3) for p in self.lines(out, "ime")), "every request names the field it is for")
        shown = [p for p in posts if p["op"] == "shown"]
        self.assertEqual(shown[0], {"op": "shown", "x": 0, "y": 626, "w": 1920, "h": 454}, "the rectangle the field is told")
        typed = [p["text"] for p in posts if p["op"] == "text"]
        self.assertEqual(typed, ["g", "h", "y", "n", "Q"])
        ops = [(p["op"], p.get("key")) for p in posts if p["op"] != "text" and p["op"] != "shown"]
        self.assertEqual(ops, [("key", "backspace"), ("clear", None), ("key", "enter"), ("hide", None), ("hide", None)],
                         "Backspace, Clear once the field has text and not before, Enter and then the keyboard going away (Search), and Back")
        got = self.evals(out)
        self.assertEqual((got["on_shift"]["key"], got["on_shift"]["shift"]), ("shift", 0))
        self.assertEqual((got["shifted"]["key"], got["shifted"]["shift"]), ("shift", 1), "Shift once is one capital")
        self.assertEqual((got["after_q"]["key"], got["after_q"]["shift"]), ("q", 0), "the capital is used up")
        self.assertEqual(got["on_backspace"]["key"], "backspace")
        self.assertEqual(got["on_clear"]["key"], "clear")
        self.assertEqual(got["on_enter"]["key"], "enter")
        self.assertEqual(got["after_back"]["up"], True, "the topic still says the field wants it: the keyboard is the shell's to take away")

    def test_the_fields_content_type_picks_the_layout_and_where_the_focus_starts(self):
        cases = [(6, 0, 0, "email", "g", 0), (5, 0, 0, "url", "g", 0), (3, 0, 2, "pad-number", "5", 0), (2, 0xc0, 0, "pad-digits", "5", 0),
                 (4, 0, 0, "pad-phone", "5", 0), (8, 0xc0, 0, "letters", "g", 0), (0, 0x10, 0, "letters", "g", 2), (7, 0x7, 0, "letters", "g", 1),
                 (0, 0x7, 0, "letters", "g", 0)]
        args = []
        at = READY
        for i, (purpose, hint, enter, *_rest) in enumerate(cases):
            field = dict(FIELD, model=10 + i, purpose=purpose, hint=hint, enter=enter)
            args += push(at, "ime", field)
            args += ["--eval", "%d:JSON.stringify({label: 'c%d', keyboard: home.state().keyboard})" % (at + 450, i)]
            args += push(at + 600, "ime", GONE)
            at += 1000
        rc, out = self.host(*args, wait=at + 400)
        self.clean(rc, out)
        got = self.evals(out)
        for i, (purpose, hint, enter, layout, key, shift) in enumerate(cases):
            k = got["c%d" % i]
            self.assertEqual((k["layout"], k["key"], k["shift"], k["up"]), (layout, key, shift, True), "purpose %d hint %#x" % (purpose, hint))
        shown = [p for p in self.lines(out, "ime") if p["op"] == "shown"]
        self.assertEqual(len(shown), len(cases))
        pads = {p["model"]: p for p in shown if p["model"] in (12, 13, 14)}
        self.assertTrue(all(p["w"] < 1920 for p in pads.values()), "a pad is a sheet of its own width, not the whole screen's: %s" % pads)
        self.assertTrue(all(p["w"] == 1920 for p in shown if p["model"] not in (12, 13, 14)))

    def test_hidden_the_keyboard_runs_nothing_and_the_system_layer_draws_no_frame(self):
        args = push(READY, "ime", FIELD) + push(READY + 1800, "ime", GONE)
        # a key for a keyboard that is not wanted is nobody's
        s = Script(READY + 3200)
        s.keys("right", "ok")
        args += s.args
        args += ["--eval", "%d:JSON.stringify({label: 'up', keyboard: home.state().keyboard})" % (READY + 1000),
                 "--eval", "%d:JSON.stringify({label: 'hidden', keyboard: home.state().keyboard})" % (READY + 2600),
                 "--eval", "%d:JSON.stringify({label: 'after_keys', keyboard: home.state().keyboard})" % (READY + 4200),
                 "--frames-sys", "%d:a" % (READY + 2800), "--frames-sys", "%d:b" % (READY + 5200)]
        rc, out = self.host(*args, wait=READY + 5400)
        self.clean(rc, out)
        got = self.evals(out)
        self.assertEqual((got["up"]["up"], got["up"]["live"]), (True, True))
        self.assertEqual((got["hidden"]["up"], got["hidden"]["live"], got["hidden"]["busy"]), (False, False, False), "nothing of it runs once it has gone")
        self.assertEqual((got["after_keys"]["up"], got["after_keys"]["busy"], got["after_keys"]["focus"]), (False, False, got["hidden"]["focus"]),
                         "the remote's keys moved nothing of a keyboard that is not up")
        self.assertEqual([p for p in self.lines(out, "ime") if p["op"] in ("text", "key", "clear")], [], "and typed nothing")
        marks = {m.group(1): int(m.group(2)) for m in re.finditer(r"^\[frames\] (\w+) (\d+)$", out, re.M)}
        self.assertEqual(marks["b"] - marks["a"], 0, "2.4 s with the keyboard away: the system layer swapped no frame")
        reports = self.lines(out, "report")
        self.assertEqual(reports[-1]["live"], False, "and told the compositor it has nothing to show, so that the compositor draws none of it either")

    def test_the_microphone_hands_the_words_to_the_field_and_says_so_when_voice_is_not_there(self):
        has_voice = {"returnValue": True, "configs": {"system.supportVoiceRecognition": True}}
        s = Script(READY + 1000)
        s.keys(*["right"] * 6)                                # along the middle row to the microphone
        s.read("on_voice")
        s.key("ok")
        mic_at = s.at
        args = ["--luna", "2500:/getConfigs:" + json.dumps(has_voice), *push(READY, "ime", FIELD), *s.args,
                "--luna", "%d:/recognizeVoiceWithDetails:%s" % (mic_at + 300, json.dumps({"returnValue": True, "event": "sttStart", "voiceTicket": "T1"})),
                "--eval", "%d:JSON.stringify({label: 'listening', keyboard: home.state().keyboard, note: home.voiceNote})" % (mic_at + 600),
                "--luna", "%d:/recognizeVoiceWithDetails:%s" % (mic_at + 900, json.dumps({"returnValue": True, "event": "sttResult", "text": ["hello world", "hollow word"]})),
                "--eval", "%d:JSON.stringify({label: 'heard', keyboard: home.state().keyboard, note: home.voiceNote})" % (mic_at + 1300)]
        # a second try that fails: the service is not there (LG's voice services are stopped under the `lean` profile)
        again = mic_at + 1600
        args += ["--eval", "%d:home.keyboardItem.imeKey('ok', true, false)" % again,
                 "--luna", "%d:/recognizeVoiceWithDetails:%s" % (again + 400, json.dumps({"returnValue": False, "errorText": "Service does not exist"})),
                 "--eval", "%d:JSON.stringify({label: 'failed', keyboard: home.state().keyboard, note: home.voiceNote})" % (again + 800)]
        rc, out = self.host(*args, wait=again + 1200)
        self.clean(rc, out)
        luna = self.lines(out, "luna")
        calls = [c for c in luna if c["uri"].endswith("/recognizeVoiceWithDetails")]
        self.assertEqual(len(calls), 2)
        self.assertEqual((calls[0]["subscribe"], calls[0]["payload"]), (True, {"subscribe": True, "language": "en-US", "runVoiceUi": False}),
                         "LG's own dictation call: recognition with no voice UI of LG's, a subscription")
        self.assertTrue(any(c["uri"].endswith("/getConfigs") for c in luna), "the key is offered only where the TV says it has voice recognition")
        evals = {}
        for m in re.finditer(r"\[eval\] \('(.*)', (?:True|False)\)$", out, re.M):
            doc = json.loads(m.group(1).replace("\\'", "'"))
            if "label" in doc:
                evals[doc["label"]] = doc
        self.assertEqual((evals["on_voice"]["keyboard"]["key"], evals["on_voice"]["keyboard"]["voice"]), ("voice", True))
        self.assertEqual((evals["listening"]["keyboard"]["listening"], evals["listening"]["note"]), (True, "Listening"))
        self.assertEqual((evals["heard"]["keyboard"]["listening"], evals["heard"]["note"]), (False, ""))
        self.assertEqual([p["text"] for p in self.lines(out, "ime") if p["op"] == "text"], ["hello world"], "the first answer is typed into the field")
        self.assertEqual((evals["failed"]["keyboard"]["listening"], evals["failed"]["note"]), (False, "Voice isn't available"))
        cancels = self.lines(out, "cancel")
        self.assertGreaterEqual(len(cancels), 1, "a finished listening ends its subscription")

    def test_the_pointer_presses_a_key_and_types_it_on_release_over_the_same_key(self):
        # letters at 100 % text: a key is 104 x 68 with 10 between, the first at the grid's corner; h is the seventh of the third row
        h = (6 * 114 + 10, 2 * 78 + 10)
        j = (7 * 114 + 10, 2 * 78 + 10)
        steps = [push(READY, "ime", FIELD)]
        at = READY + 1000
        evs = []
        for label, expr in (("hover", "home.keyboardItem.pointerHover(%d, %d)" % h),
                            ("down_h", "home.keyboardItem.pointerDown(%d, %d)" % h),
                            ("up_h", "home.keyboardItem.pointerUp(%d, %d)" % h),
                            ("down_j", "home.keyboardItem.pointerDown(%d, %d)" % j),
                            ("up_h_again", "home.keyboardItem.pointerUp(%d, %d)" % h),
                            ("gap", "home.keyboardItem.pointerHover(%d, %d)" % (6 * 114 + 107, 2 * 78 + 10))):
            evs += ["--eval", "%d:(function() { %s; return 1 })()" % (at, expr)]
            at += 200
            evs += ["--eval", "%d:JSON.stringify({label: '%s', keyboard: home.state().keyboard})" % (at - 100, label)]
        rc, out = self.host(*steps[0], *evs, wait=at + 400)
        self.clean(rc, out)
        got = self.evals(out)
        self.assertNotEqual(got["hover"]["key"], "h", "hovering shows the key under the pointer and does not move the focus")
        self.assertEqual(got["down_h"]["key"], "h", "a press moves the focus to its key")
        self.assertEqual([p["text"] for p in self.lines(out, "ime") if p["op"] == "text"], ["h"], "a press let go on the same key types it, once")
        self.assertEqual(got["down_j"]["key"], "j", "a press moves the focus to its key")
        self.assertEqual(got["gap"]["key"], "j", "the gap between two keys is no key: the focus stays where it was")

    def test_with_reduced_motion_the_keyboard_fades_and_types_all_the_same(self):
        s = Script(READY + 1000)
        s.keys("right", "ok")
        s.read("typed")
        args = push(READY, "ime", FIELD) + s.args + push(READY + 2600, "ime", GONE)
        args += ["--eval", "%d:JSON.stringify({label: 'away', keyboard: home.state().keyboard})" % (READY + 3400)]
        rc, out = self.host("--theme", json.dumps({"motion": {"reduced": True}}), *args, wait=READY + 3800)
        self.clean(rc, out)
        got = self.evals(out)
        self.assertEqual((got["typed"]["key"], got["typed"]["up"]), ("h", True))
        self.assertEqual([p["text"] for p in self.lines(out, "ime") if p["op"] == "text"], ["h"])
        self.assertEqual((got["away"]["up"], got["away"]["live"], got["away"]["busy"]), (False, False, False), "the fade is over and nothing runs")


if __name__ == "__main__":
    unittest.main()
