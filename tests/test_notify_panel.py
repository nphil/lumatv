"""Run with: cd tests && python3 -m unittest test_notify_panel   (needs Xvfb and PyQt5; skipped without them)

LumaTV's own notification panel (payload/compositor/lumatv/panels/Notify.qml), the one the shell's system layer draws under
LumaTV's compositor: a toast shows and goes with its timeout and takes no key; an alert takes the keys, its focused button
answers with the action the service attached and Back runs the close action; a warning alert without buttons is drawn and
never filtered; the input popup switches or stays; the PIN prompt types four digits and relays them, or cancels.
"""
import json
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

READY = 1500
LAUNCH = {"serviceURI": "luna://com.webos.applicationManager", "serviceMethod": "/launch", "launchParams": {"id": "com.webos.app.hdmi3"}}


def state_js():
    return ("JSON.stringify({toasts: root.toasts.map(function(t) { return [t.title, t.body] }), alerts: root.alerts.length, "
            "alertIndex: root.alerts.length ? root.alerts[0].index : -1, input: !!root.inputAlert, pin: root.pin ? root.pin.typed : null, "
            "pinIndex: root.pin ? root.pin.index : -1, wants: root.wants, live: root.live, "
            "calls: acts(root).map(function(c) { return [c.uri + c.method, JSON.parse(c.payload)] })})")


# The panel's own calls: what it did, without the four feed subscriptions it opens.
ACTS = "function acts(r) { return r.lunaCalls.filter(function(c) { return c.method.indexOf('/get') !== 0 }) }"


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class NotifyPanel(HostCase):
    def run_panel(self, steps, wait):
        args = []
        for at, js in steps:
            args += ["--eval", "%d:(function(){%s %s return 1})()" % (at, ACTS, js)]
        args += ["--eval", "%d:(function(){%s return %s})()" % (wait - 200, ACTS, state_js()), "--wait", str(wait)]
        rc, out = self.host(*args, surface="panels/Notify.qml")
        docs = self.docs(out)
        self.assertTrue(docs, out[-2000:])
        return docs[-1], out

    def test_a_toast_shows_its_title_and_subtitle_and_goes_with_its_timeout_taking_no_key(self):
        steps = [(READY, "root.toast({message: 'Update ready<br>Version 4.07.46 is downloaded', sourceId: 'com.webos.app.update', timeout: 1});"
                         "root.toast({message: 'Remote battery low', sourceId: 'lg', timeout: 20});"
                         "console.info('[t1] ' + JSON.stringify({n: root.toasts.length, wants: root.wants, live: root.live, first: [root.toasts[0].title, root.toasts[0].body]}));")]
        doc, out = self.run_panel(steps, READY + 2200)
        t1 = self.tagged(out, "t1")[0]
        self.assertEqual(t1["n"], 2)
        self.assertEqual(t1["first"], ["Remote battery low", ""])
        self.assertFalse(t1["wants"])
        self.assertTrue(t1["live"])
        self.assertEqual(doc["toasts"], [["Remote battery low", ""]], "the one-second toast is gone, the other stays")

    def test_an_alert_takes_the_keys_and_its_button_runs_the_attached_action(self):
        alert = {"alertAction": "open", "alertInfo": {"sourceId": "com.webos.app.home", "title": "Uninstall Crunchyroll?", "message": "The app is removed.",
                                                      "modal": True, "timeout": 0, "timestamp": 1,
                                                      "buttons": [{"label": "Keep it", "focus": True}, {"label": "Uninstall", "action": LAUNCH}],
                                                      "onCloseAction": {"serviceURI": "luna://com.webos.notification", "serviceMethod": "/closeAlert", "launchParams": {"timestamp": 1}}}}
        steps = [(READY, "root.alert(%s); console.info('[a1] ' + JSON.stringify({up: root.alertUp, wants: root.wants, index: root.alerts[0].index}));" % json.dumps(alert)),
                 (READY + 300, "root.key('right'); root.key('ok');"),
                 (READY + 600, "var a = %s; a.alertInfo.timestamp = 2; a.alertInfo.buttons = []; a.alertInfo.type = 'warning'; root.alert(a); root.key('back');" % json.dumps(alert))]
        doc, out = self.run_panel(steps, READY + 1400)
        a1 = self.tagged(out, "a1")[0]
        self.assertTrue(a1["up"] and a1["wants"])
        self.assertEqual(a1["index"], 0, "the button with focus: true starts focused")
        self.assertEqual(doc["alerts"], 0)
        calls = doc["calls"]
        self.assertIn(["luna://com.webos.applicationManager/launch", {"id": "com.webos.app.hdmi3"}], calls, "OK on Uninstall ran its action")
        self.assertIn(["luna://com.webos.notification/closeAlert", {"timestamp": 1}], calls, "the warning closed by Back ran its close action")

    def test_the_input_popup_switches_or_stays(self):
        popup = {"alertAction": "open", "alertInfo": {"sourceId": "com.webos.app.hdmi3", "timeout": 15, "timestamp": 2, "portName": "HDMI 3", "action": LAUNCH, "appId": "com.webos.app.hdmi3"}}
        steps = [(READY, "root.input(%s); root.key('right'); root.key('ok'); console.info('[i1] ' + JSON.stringify({gone: !root.inputAlert, launches: acts(root).length}));" % json.dumps(popup)),
                 (READY + 300, "root.input(%s); root.key('ok');" % json.dumps(popup))]
        doc, out = self.run_panel(steps, READY + 900)
        i1 = self.tagged(out, "i1")[0]
        self.assertTrue(i1["gone"])
        self.assertEqual(i1["launches"], 0, "Stay here launches nothing")
        self.assertFalse(doc["input"])
        self.assertEqual(doc["calls"], [["luna://com.webos.applicationManager/launch", {"id": "com.webos.app.hdmi3"}]])

    def test_the_pin_prompt_relays_four_digits_and_cancels_on_back(self):
        prompt = {"pincodePromptAction": "open", "pincodePromptInfo": {"appId": "com.webos.app.hdmi3", "title": "Enter the TV PIN", "message": "HDMI 3 is locked.", "promptType": "parental", "retry": 0, "timestamp": 3}}
        steps = [(READY, "root.prompt(%s); root.digit(1); root.digit(2); root.key('down'); root.key('down'); root.key('down'); root.key('down'); root.key('ok');"
                         "console.info('[p1] ' + JSON.stringify({typed: root.pin.typed, index: root.pin.index, calls: acts(root).length}));" % json.dumps(prompt)),
                 (READY + 300, "root.digit(3); root.digit(4); root.key('ok');"),
                 (READY + 600, "root.prompt(%s); root.key('back');" % json.dumps(prompt))]
        doc, out = self.run_panel(steps, READY + 1200)
        p1 = self.tagged(out, "p1")[0]
        self.assertEqual(p1["typed"], "12")
        self.assertEqual(p1["index"], 12, "Down from the pad lands on OK")
        self.assertEqual(p1["calls"], 0, "OK with two digits relays nothing")
        self.assertIsNone(doc["pin"], "cancelled")
        self.assertEqual(doc["calls"], [["luna://com.webos.notification/closePincodePrompt", {"pincode": "1234", "closeType": "relay", "mode": "", "appId": "com.webos.app.hdmi3"}],
                                        ["luna://com.webos.notification/closePincodePrompt", {"closeType": "close"}]])


if __name__ == "__main__":
    unittest.main()
