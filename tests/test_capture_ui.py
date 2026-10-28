"""Run with: cd tests && python3 -m unittest test_capture_ui   (needs Xvfb and PyQt5; skipped without them)

The capture surfaces (docs/research/capture-result.md) through the offline hosts:

* the shell client's recording dot and note (panels/CaptureMark.qml in compositor/shell/Shell.qml's system layer) against a stand-in of
  the compositor's UI channel: the dot is drawn while the `capture` topic says a recording runs, the shell says so in its state
  (`capture.dot`, which the compositor will not record without), the layer is live for it, a note follows a finished capture for a moment,
  and what happened before the shell began is not announced;
* the Settings page (settings/PageCapture.qml) against the fake helper: dimmed and saying so while the switch is off, the actions post to
  the helper's routes while it is on, and a refusal is said in words.
"""
import ast
import json
import os
import re
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402

SHELL_HOST = os.path.join(HERE, "qmlhost", "shellhost.py")
HOME_HOST = os.path.join(HERE, "qmlhost", "qmlhost.py")
HOME = '{"app":"com.webos.app.home","home":true}'


def topic(recording=False, n=2, last=None):
    return json.dumps({"recording": recording, "stage": "recording" if recording else "idle", "since": 2000 if recording else None, "n": n, "last": last})


SHOT = {"kind": "shot", "name": "shot-1.png", "bytes": 1, "seconds": 0, "ok": True, "note": "1920x1080", "at": 1000}
REC_OK = {"kind": "recording", "name": "rec-1.mp4", "bytes": 9, "seconds": 4.2, "ok": True, "note": "", "at": 3000}
REC_BAD = dict(REC_OK, ok=False, note="the encoder: it stopped", at=4000)
SHOT2 = dict(SHOT, name="shot-2.png", at=5000)


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class CaptureMark(unittest.TestCase):
    def host(self, *args, wait):
        env = dict(os.environ, DISPLAY=DISPLAY, QT_QPA_PLATFORM="offscreen")
        r = subprocess.run([sys.executable, SHELL_HOST, "--wait", str(wait), *args], env=env, capture_output=True, text=True,
                           timeout=int(os.environ.get("HOST_TIMEOUT", "240")))
        return r.returncode, r.stdout + r.stderr

    def evals(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"\[eval\] \('(.*)', (?:True|False)\)$", out, re.M)]

    def reports(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"^\[report\] (.*)$", out, re.M)]

    def test_the_dot_follows_the_recording_and_the_note_follows_a_capture(self):
        look = "JSON.stringify([home.state().capture.dot, home.captureLive, home.captureItem.note, home.captureItem.recording])"
        rc, out = self.host(
            "--push", "500:foreground:" + HOME, "--push", "500:screen:{\"on\":true}",
            # a capture from before this shell began: the first topic is the baseline, not news
            "--push", "3000:capture:" + topic(False, 2, SHOT), "--eval", "3600:" + look,
            # a recording: the dot, the layer live, the shell says so in its state
            "--push", "4000:capture:" + topic(True, 2, SHOT), "--eval", "4800:" + look,
            # it ends: the dot goes, the words stay for a moment
            "--push", "5200:capture:" + topic(False, 2, REC_OK), "--eval", "5800:" + look, "--eval", "9000:" + look,
            # a screenshot: the counter moves
            "--push", "9500:capture:" + topic(False, 3, SHOT2), "--eval", "10100:" + look,
            # a recording that failed: said as stopped
            "--push", "13000:capture:" + topic(False, 3, REC_BAD), "--eval", "13600:" + look, wait=14500)
        e = self.evals(out)
        self.assertEqual(len(e), 6, out[-3000:])
        self.assertEqual(e[0], [False, False, "", False], "what was there before the shell began is not announced")
        self.assertEqual(e[1], [True, True, "", True], "the dot is up, the layer is live for it, and the shell says the dot is up")
        self.assertEqual(e[2], [False, True, "Recording saved", False], "the dot is gone, the note is up and keeps the layer live")
        self.assertEqual(e[3], [False, False, "", False], "the note went by itself")
        self.assertEqual(e[4], [False, True, "Screenshot saved", False])
        self.assertEqual(e[5], [False, True, "Recording stopped", False])
        reports = self.reports(out)
        self.assertTrue(any(r["state"]["capture"]["dot"] is True and r["live"] is True for r in reports), "the compositor is told: %s" % [(r["live"], r["state"]["capture"]) for r in reports][-6:])
        self.assertTrue(all("capture" in r["state"] for r in reports))
        self.assertEqual(rc, 0, "no warning of the shell's own:\n" + "\n".join(l for l in out.splitlines() if l.startswith("[warning]") or l.startswith("[critical]"))[-2000:])


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class CapturePage(unittest.TestCase):
    PAGE = ("var p=root.pageItem('capture');var out=[];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
            "out.push({id:r.id||'',label:String(p.ev(r.label)||''),text:String(p.ev(r.text)||''),on:p.ev(r.on)===true,enabled:p.ev(r.enabled)!==false,"
            "detail:String(p.ev(r.detail)||''),value:String(p.ev(r.value)||''),button:String(p.ev(r.button)||'')})}return out")
    SET = "var x=new XMLHttpRequest();x.open('POST','http://127.0.0.1:18731/__capture');x.setRequestHeader('Content-Type','application/json');x.send(JSON.stringify(%s));return 1"
    POSTS = "var x=new XMLHttpRequest();x.open('GET','http://127.0.0.1:18731/__posts',false);x.send();console.info('[posts] '+x.responseText);return 1"

    def run_probes(self, *probes):
        """Each probe is (wait ms, JS body); returns what the ones that returned JSON said, and the helper's recorded POSTs."""
        args = []
        at = 2500
        for wait, body in probes:
            args += ["--eval", "%d:(function(){%s})()" % (at, body)]
            at += wait
        args += ["--eval", "%d:(function(){%s})()" % (at, self.POSTS), "--wait", str(at + 1500)]
        cmd = [sys.executable, HOME_HOST, "Settings.qml", "--fake-helper", "--prop", "hostActive=true", "--theme", '{"perf":{"syncResidents":true}}', *args]
        r = subprocess.run(cmd, env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=int(os.environ.get("HOST_TIMEOUT", "300")))
        out = r.stdout + r.stderr
        docs = []
        for line in out.splitlines():
            m = re.match(r"\[eval\] \((.*), (True|False)\)$", line)
            if m:
                v = ast.literal_eval(m.group(1))
                if isinstance(v, str) and v[:1] in "[{":
                    docs.append(json.loads(v))
        posts = [json.loads(m.group(1)) for m in re.finditer(r"\[posts\] (\{.*\})", out)]
        return docs, (posts[-1]["posts"] if posts else []), out

    def test_off_the_page_says_so_and_every_action_is_dimmed(self):
        docs, posts, out = self.run_probes((1500, "root.openPage('capture');root.pageItem('capture').refresh();return 1"), (800, "return JSON.stringify(%s)" % ("(function(){" + self.PAGE + "})()")))
        rows = docs[0]
        self.assertEqual(rows[0]["text"], "Screen capture is off on this TV.", out[-2500:])
        by = {r["id"]: r for r in rows if r["id"]}
        self.assertEqual([by[i]["enabled"] for i in ("shot", "later", "record")], [False, False, False])
        self.assertNotIn("files", by, "nothing is said about files while it is off")
        self.assertEqual(posts, [])

    def test_on_the_actions_post_to_the_helper_and_a_refusal_is_said(self):
        on = {"enabled": True, "available": True, "recording": False, "encoder": {"path": "/x", "available": True}, "captures": {"count": 3, "bytes": 5200000, "max": 268435456}}
        rows = "JSON.stringify((function(){" + self.PAGE + "})())"
        docs, posts, out = self.run_probes(
            (1200, self.SET % json.dumps(on)),
            (1200, "root.openPage('capture');root.pageItem('capture').refresh();return 1"),
            (900, "return " + rows),
            (1200, "var p=root.pageItem('capture');p.rows.filter(function(r){return r.id==='shot'})[0].act();return 1"),
            (1200, "return " + rows),
            (1200, "var p=root.pageItem('capture');p.rows.filter(function(r){return r.id==='record'})[0].act(true);return 1"),
            (1500, "return " + rows),
            (1200, "var p=root.pageItem('capture');p.rows.filter(function(r){return r.id==='record'})[0].act(false);return 1"),
            (1500, "return " + rows),
            (1200, self.SET % json.dumps(dict(on, refuse={"code": "no_indicator", "status": 409}))),
            (1200, "var p=root.pageItem('capture');p.rows.filter(function(r){return r.id==='record'})[0].act(true);return 1"),
            (1500, "return " + rows),
        )
        idle, shot, started, stopped, refused = docs
        by = lambda rs: {r["id"]: r for r in rs if r["id"]}  # noqa: E731
        self.assertEqual((by(idle)["shot"]["enabled"], by(idle)["record"]["enabled"], by(idle)["record"]["on"]), (True, True, False), out[-2500:])
        self.assertEqual(by(idle)["files"]["value"], "3 files, 5.2 MB")
        self.assertEqual([r["text"] for r in shot if r["text"]], ["Screenshot saved"])
        self.assertEqual((by(started)["record"]["on"], by(started)["record"]["detail"]), (True, "0:03"), "the recording runs, its clock is the helper's")
        self.assertEqual([r["text"] for r in stopped if r["text"]], ["Recording saved"])
        self.assertFalse(by(stopped)["record"]["on"])
        self.assertEqual([r["text"] for r in refused if r["text"]], ["The recording dot could not be shown"])
        self.assertEqual([p[0] for p in posts], ["/v1/capture/screenshot", "/v1/capture/record/start", "/v1/capture/record/stop", "/v1/capture/record/start"])


if __name__ == "__main__":
    unittest.main()
