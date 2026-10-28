"""Run with: python3 -m unittest tests.test_notification_patch

tools/notification_patch.py against a copy of LG's notification tree (the census copy of the set's
/usr/lib/qt5/qml/SystemUI/Notification; skipped when it is not on this workstation): what it
changes, that it changes nothing else, that a missing or doubled anchor is fatal, and (with Xvfb and
PyQt5) that the patched view loads and draws its toast and alert through the offline host.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
TOOL = os.path.join(ROOT, "tools", "notification_patch.py")
TOKENS = os.path.join(ROOT, "tools", "tokens.py")
SHIPPED = os.path.join(ROOT, "payload", "theme", "theme.json")
OVERLAY = os.path.join(ROOT, "payload", "notification")
HOST = os.path.join(HERE, "qmlhost", "notify.py")
CENSUS = "/data/home/lgtv/uiinv/qml/qml/SystemUI/Notification"

sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402


sys.path.insert(0, os.path.join(ROOT, "tools"))
from notification_patch import opaque  # noqa: E402


def palette():
    """The palette the view is drawn with: tokens.py's, with the sheet opaque and a backdrop for a modal question that hides the page."""
    out = subprocess.run([sys.executable, TOKENS, SHIPPED, os.devnull, "qml"], capture_output=True, text=True, check=True).stdout
    return opaque(json.loads(out))


@unittest.skipUnless(os.path.isdir(CENSUS), "needs the census copy of LG's notification tree")
class NotificationPatch(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = os.path.join(self.tmp.name, "Notification")
        shutil.copytree(CENSUS, self.dir)
        self.pal = palette()

    def tearDown(self):
        self.tmp.cleanup()

    def run_tool(self, pal=None, overlay=OVERLAY):
        args = [sys.executable, TOOL, self.dir, json.dumps(pal or self.pal)] + ([overlay] if overlay else [])
        r = subprocess.run(args, capture_output=True, text=True)
        return r.returncode, r.stderr

    def read(self, name):
        with open(os.path.join(self.dir, name)) as f:
            return f.read()

    def write(self, name, text):
        with open(os.path.join(self.dir, name), "w") as f:
            f.write(text)

    def stock(self, name):
        with open(os.path.join(CENSUS, name)) as f:
            return f.read()

    def test_patches_the_real_tree(self):
        rc, err = self.run_tool()
        self.assertEqual(rc, 0, err)
        style = self.read("NotificationStyle.qml")
        self.assertTrue(style.startswith("// Luma: patched"))
        self.assertNotRegex(style, r"root\.languageStyle\.(normal|cond)\d+\.fontFamily\[0\]")
        self.assertGreaterEqual(style.count('"Manrope"'), 6)
        self.assertIn('property color defaultFocusColor: "%s"' % self.pal["accent"], style)
        self.assertIn('property string scrimColor: "%s"' % self.pal["scrimHide"], style)
        self.assertNotIn("#cf0652", style.split("property QtObject alert")[0])

    def test_the_alert_and_its_defaultview_twin_are_both_in_the_palette_and_partialview_is_left(self):
        self.run_tool()
        style = self.read("NotificationStyle.qml")
        heads = [m.start() for m in re.finditer(r"property QtObject alert: QtObject \{", style)]
        self.assertEqual(len(heads), 3)
        for i, want in enumerate((True, True, False)):
            body = style[heads[i]:heads[i] + 6000].split("property QtObject button")[0]
            self.assertEqual(self.pal["sheetBottom"] in body, want, "alert object %d" % i)
            self.assertEqual(self.pal["hairline"] in body, want)
            if want:
                self.assertNotIn("#CACBCC", body)
            else:
                self.assertIn('"#000000"', body)       # the partial-screen layout keeps LG's black

    def test_toast_is_ours_and_in_the_palette(self):
        self.run_tool()
        toast = self.read("Toast.qml")
        self.assertIn("LumaTV's toast", toast)
        for name in ("sheetBottom", "ink", "inkDim", "hairline", "row", "accent"):
            self.assertIn(self.pal[name], toast)
        self.assertNotRegex(toast, r"@@\w+@@")
        code = "\n".join(ln for ln in toast.splitlines() if not ln.lstrip().startswith("//"))
        self.assertNotRegex(code, r"layer\.enabled|DropShadow")
        # the interface ToastView reads of it
        for member in ("property string iconPath", "property string message", "property string readable", "property string type",
                       "property bool containsMouse", "signal activated()", "signal closed()", "signal entered()",
                       "property var userdataId", "function rotateToastView()"):
            self.assertIn(member, toast)

    def test_toast_goes_where_the_mock_puts_it(self):
        self.run_tool()
        view = self.read("ToastView.qml")
        self.assertIn("anchors.topMargin: root.style.getRValue(137)", view)
        self.assertIn("anchors.rightMargin: root.style.getRValue(64)", view)
        self.assertIn("x: toastStack.width - width", view)
        self.assertNotIn("(root.width - style.toast.width) / 2", view)

    def test_alert_is_centred_and_buttons_are_in_the_palette(self):
        self.run_tool(overlay=None)       # the overlay replaces Alert.qml; this is the patch on LG's own
        alert = self.read("Alert.qml")
        self.assertIn("anchors.verticalCenter: isFullAlert ? undefined : isPartialScreen ? undefined : parent.verticalCenter", alert)
        self.assertNotIn("getRValue(180)", alert.split("MoonstoneShadow")[0])
        button = self.read("SimpleButton.qml")
        for name in ("ink", "focus", "row", "focusEdge"):
            self.assertIn(self.pal[name], button)
        self.assertNotIn("#E6E6E6", button)
        self.assertNotIn("#4C5059", button)

    def test_shadows_are_off_in_the_files_that_have_one(self):
        self.run_tool(overlay=None)       # the overlay replaces the pad key; this is the patch on LG's own
        for name in ("SimpleButton.qml", "PincodePromptNumButton.qml"):
            text = self.read(name)
            self.assertNotRegex(text, r"layer\.enabled:\s*(true|root\.activeFocus)\s*layer\.effect: DropShadow")
            self.assertRegex(text, r"layer\.enabled: false\s*layer\.effect: DropShadow")

    def test_changes_nothing_but_what_it_names(self):
        self.run_tool()
        named = ("NotificationStyle.qml", "Toast.qml", "ToastView.qml", "Alert.qml", "SimpleButton.qml", "PincodePromptNumButton.qml",
                 "NotificationView.qml", "AdvancedAlertView.qml", "AdvancedAlertDelegate.qml", "PincodePromptView.qml", "PincodePrompt.qml",
                 "PincodePromptPasswordButton.qml")
        for name in sorted(os.listdir(CENSUS)):
            p = os.path.join(CENSUS, name)
            if not os.path.isfile(p) or name in named:
                continue
            with open(p, "rb") as a, open(os.path.join(self.dir, name), "rb") as b:
                self.assertEqual(a.read(), b.read(), name)
        self.assertEqual(sorted(os.listdir(self.dir)), sorted(os.listdir(CENSUS)))

    def test_the_shadow_blocks_stay_and_only_the_switch_changes(self):
        self.run_tool(overlay=None)
        for name in ("SimpleButton.qml", "PincodePromptNumButton.qml"):
            after = self.read(name)
            before = self.stock(name)
            self.assertEqual(after.count("layer.enabled: false"), len(re.findall(r"layer\.enabled:\s*(?:true|root\.activeFocus)", before)))
            self.assertEqual(len(after.split("DropShadow")), len(before.split("DropShadow")))

    def test_the_overlay_files_are_in_the_palette_and_keep_the_interface_their_views_read(self):
        self.run_tool()
        want = {
            "Alert.qml": ("property string title", "property string message", "property bool modal", "property alias buttonModel",
                          "property bool isFullAlert", "property bool isCheckBoxAlert", "property alias checkValue", "property bool isProgressAlert",
                          "property bool isSpinnerAlert", "property bool showTimeoutValue", "property int progressValue",
                          "property alias discardTimerInterval", "signal buttonClicked(int btnIndex)", "signal closeClicked()", "signal discard()"),
            "AdvancedAlertDelegate.qml": ("property alias discardTimerInterval", "property alias alertCardContainer", "property bool deviceReady",
                                          "property bool portReady", "function animateButtonClose()", "signal discard()", "signal discardAndActivate(int cardId)"),
            "PincodePrompt.qml": ("property string title", "property string message", "property var retry", "property bool isFirstUse", "property bool aboutToClose",
                                  "property string password", "signal postPassword()", "signal postExitSignal()", "function getPromptTitle(", "function getPromptMessage("),
        }
        for name, members in want.items():
            text = self.read(name)
            self.assertNotRegex(text, r"@@\w+@@", name)
            code = "\n".join(ln for ln in text.splitlines() if not ln.lstrip().startswith("//"))
            self.assertNotRegex(code, r"layer\.enabled|DropShadow|ConicalGradient", name)
            for member in members:
                self.assertIn(member, text, name + ": " + member)
        for name in ("PincodePromptNumButton.qml", "PincodePromptPasswordButton.qml"):
            self.assertNotRegex(self.read(name), r"@@\w+@@", name)
        self.assertIn(self.pal["danger"], self.read("Alert.qml"))

    def test_the_input_popup_goes_to_the_toast_corner_with_no_black_scrim_and_the_pin_page_is_clear(self):
        self.run_tool()
        view = self.read("NotificationView.qml")
        self.assertIn("topMargin: style.getRValue(137)", view)
        self.assertIn("rightMargin: style.getRValue(64)", view)
        self.assertNotIn("bottomMargin: style.inputPopupItem.bottomMargin", view)
        self.assertNotIn("opacity: 0.6", view)
        self.assertIn('color: "transparent"', self.read("PincodePromptView.qml"))

    def test_a_changed_input_popup_or_pin_anchor_is_fatal(self):
        for name, old in (("NotificationView.qml", "bottomMargin: style.inputPopupItem.bottomMargin"),
                          ("AdvancedAlertView.qml", "from: root.style.inputPopupItem.height + root.style.inputPopupItem.bottomMargin"),
                          ("PincodePromptView.qml", "color: style.pincodePrompt.backgroundColor")):
            shutil.rmtree(self.dir)
            shutil.copytree(CENSUS, self.dir)
            self.write(name, self.stock(name).replace(old, old + " + 1"))
            rc, err = self.run_tool()
            self.assertEqual(rc, 1, name)
            self.assertIn(name, err)

    def test_without_an_overlay_the_tree_keeps_LGs_toast(self):
        rc, err = self.run_tool(overlay=None)
        self.assertEqual(rc, 0, err)
        self.assertEqual(self.read("Toast.qml"), self.stock("Toast.qml"))

    def test_a_missing_anchor_is_fatal_and_says_which(self):
        self.write("NotificationStyle.qml", self.stock("NotificationStyle.qml").replace('"#CACBCC"', '"#111111"'))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("backgroundColor", err)
        self.assertIn("found 0 times", err)

    def test_a_doubled_anchor_is_fatal_too(self):
        text = self.stock("ToastView.qml")
        self.write("ToastView.qml", text + "\n// anchors.topMargin: root.style.toast.viewMargins\n")
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("found 2 times", err)

    def test_a_changed_alert_placement_is_fatal(self):
        self.write("Alert.qml", self.stock("Alert.qml").replace("getRValue(180)", "getRValue(190)"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("Alert.qml", err)

    def test_a_tree_with_no_shadow_to_switch_off_is_fatal(self):
        self.write("SimpleButton.qml", re.sub(r"layer\.enabled:\s*(true|root\.activeFocus)", "layer.enabled: false", self.stock("SimpleButton.qml")))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("SimpleButton.qml", err)

    def test_a_tree_whose_fonts_were_renamed_is_fatal(self):
        self.write("NotificationStyle.qml", self.stock("NotificationStyle.qml").replace("languageStyle", "langStyle"))
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("font sites", err)

    def test_an_overlay_file_with_nothing_to_replace_is_fatal(self):
        with tempfile.TemporaryDirectory() as ov:
            with open(os.path.join(ov, "NoSuchThing.qml"), "w") as f:
                f.write("import QtQuick 2.4\nItem {}\n")
            rc, err = self.run_tool(overlay=ov)
        self.assertEqual(rc, 1)
        self.assertIn("no such file to replace", err)

    def test_an_overlay_marker_the_palette_lacks_is_fatal(self):
        with tempfile.TemporaryDirectory() as ov:
            with open(os.path.join(ov, "Toast.qml"), "w") as f:
                f.write('import QtQuick 2.4\nRectangle { color: "@@noSuchColour@@" }\n')
            rc, err = self.run_tool(overlay=ov)
        self.assertEqual(rc, 1)
        self.assertIn("noSuchColour", err)

    def test_patching_twice_is_refused(self):
        self.assertEqual(self.run_tool()[0], 0)
        rc, err = self.run_tool()
        self.assertEqual(rc, 1)
        self.assertIn("already patched", err)

    def test_a_palette_without_what_it_needs_is_refused(self):
        rc, err = self.run_tool({"ink": "#ffffff"})
        self.assertEqual(rc, 1)
        self.assertIn("palette has no", err)
        pal = dict(self.pal)
        del pal["danger"]
        rc, err = self.run_tool(pal)
        self.assertEqual(rc, 1)
        self.assertIn("danger", err)

    def test_the_patched_tree_still_has_the_files_the_view_imports(self):
        self.run_tool()
        for name in ("NotificationView.qml", "Services.qml", "NLocaleService/qmldir", "Utils.js"):
            self.assertTrue(os.path.exists(os.path.join(self.dir, name)), name)


@unittest.skipUnless(os.path.isdir(CENSUS) and DISPLAY and HAVE_QT, "needs the census copy of LG's tree, Xvfb and PyQt5")
class PatchedViewInTheHost(unittest.TestCase):
    """The patched view, with LG's own data layer behind stand-ins for what is native, in Qt 5.15 on
    software GL: it must load, take each kind of notification through LG's models and warn of nothing."""

    def run_host(self, *args):
        r = subprocess.run([sys.executable, HOST, "--patched", *args], env=dict(os.environ, DISPLAY=DISPLAY),
                           capture_output=True, text=True, timeout=120)
        return r.returncode, r.stdout + r.stderr

    def states(self, out):
        return [json.loads(m.group(1)) for m in re.finditer(r"\[state\] (\{.*\})", out)]

    def test_a_toast_and_an_alert_arrive_together(self):
        rc, out = self.run_host("--toast", "300", "--alert", "600", "--wait", "1800")
        self.assertEqual(rc, 0, out[-1500:])
        last = self.states(out)[-1]
        self.assertEqual((last["toasts"], last["alerts"]), (1, 1), out[-800:])

    def test_an_input_popup_and_a_pin_prompt_arrive_when_no_modal_alert_is_up(self):
        # LG's own rule (NotificationView.acceptAdvancedAlerts / acceptPincodePrompts): a modal alert refuses both.
        for kind, key in (("--input", "advanced"), ("--pin", "pin")):
            rc, out = self.run_host(kind, "300", "--wait", "1500")
            self.assertEqual(rc, 0, kind + out[-1500:])
            self.assertEqual(self.states(out)[-1][key], 1, kind + out[-800:])
        rc, out = self.run_host("--alert", "300", "--input", "700", "--pin", "900", "--wait", "1800")
        last = self.states(out)[-1]
        self.assertEqual((last["alerts"], last["advanced"], last["pin"]), (1, 0, 0), out[-800:])

    def test_a_two_line_toast_with_an_icon_and_a_link_loads_clean(self):
        doc = json.dumps({"sourceId": "x", "message": "Volume 24 \u00b7 set by Cody\nAllowed by your AI access settings", "type": "advanced",
                          "iconPath": os.path.join(ROOT, "assets", "icon160.png"), "timeout": 8, "userData": {"id": "t"}, "readable": "",
                          "action": {"serviceURI": "luna://x", "serviceMethod": "y", "launchParams": {}}, "actionLabel": "Undo"})
        rc, out = self.run_host("--toast", "300:" + doc, "--wait", "1500")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.states(out)[-1]["toasts"], 1)

    def test_toast_light_and_standard_types_load_clean(self):
        for kind in ("light", "standard"):
            doc = json.dumps({"sourceId": "x", "message": "A long message " * 12, "type": kind, "iconPath": "", "timeout": 8,
                              "userData": {"id": kind}, "readable": ""})
            rc, out = self.run_host("--toast", "300:" + doc, "--wait", "1200")
            self.assertEqual(rc, 0, kind + out[-1200:])

    def test_only_a_generic_icon_file_is_replaced_by_lumatvs_mark(self):
        # An app's own icon survives a path that merely contains "info" or "default"; the generic file name, or no icon, gets the mark.
        with tempfile.TemporaryDirectory() as tmp:
            def source(rel):
                path = ""
                if rel:
                    path = os.path.join(tmp, rel)
                    os.makedirs(os.path.dirname(path), exist_ok=True)
                    shutil.copy(os.path.join(ROOT, "assets", "icon160.png"), path)
                doc = json.dumps({"sourceId": "x", "message": "hi", "type": "advanced", "iconPath": path, "readable": "", "timeout": 8, "userData": {"id": "t"}})
                rc, out = self.run_host("--toast", "300:" + doc, "--probe", "900:toastIcon:source", "--wait", "1200")
                self.assertEqual(rc, 0, out[-1200:])
                return path, re.search(r"\[probe\] toastIcon\.source=(\S+)", out).group(1)
            for own in ("x/default_icon.png", "com.info.app/icon.png", "notice-board/logo.png"):
                path, src = source(own)
                self.assertEqual(src, "file://" + path, own)
            for generic in ("", "x/info.png", "x/Default.PNG"):
                self.assertTrue(source(generic)[1].endswith("/images/luma-info.png"), generic)

    ALERT = {"alertInfo": {"sourceId": "x", "title": "Uninstall?", "message": "Sure?", "readable": "", "showTimeout": False, "modal": True, "timeout": 0,
                           "timestamp": 5, "buttons": [{"label": "Keep it", "focus": True, "action": {"serviceURI": "luna://k", "serviceMethod": "/keep", "launchParams": {}}},
                                                       {"label": "Uninstall", "danger": True, "action": {"serviceURI": "luna://u", "serviceMethod": "/un", "launchParams": {"a": 1}}}]}}

    def actions(self, out):
        """The luna calls the view made that are not its own subscriptions: (uri, method, payload)."""
        line = re.search(r"^\[calls\] (.*)$", out, re.M)
        self.assertIsNotNone(line, out[-800:])
        return [(c["uri"], c["method"], c["payload"]) for c in json.loads(line.group(1)) if not c["method"].startswith(("get", "signal/"))]

    def test_the_alert_answers_from_the_keys_only_and_each_button_calls_its_own_action(self):
        rc, out = self.run_host("--alert", "300:" + json.dumps(self.ALERT), "--press", "1200:OK", "--calls", "--wait", "2200")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.actions(out), [("luna://k", "/keep", "{}")])
        self.assertEqual(self.states(out)[-1]["alerts"], 0)
        rc, out = self.run_host("--alert", "300:" + json.dumps(self.ALERT), "--press", "900:RIGHT", "--press", "1300:OK", "--calls", "--wait", "2200")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.actions(out), [("luna://u", "/un", '{"a":1}')])

    def test_a_warning_alert_is_drawn_whole_and_never_filtered(self):
        # LG's protected kinds (emergency alerts, thermal shutdown, Magic Remote battery) arrive as alerts or toasts like any other;
        # LumaTV's view draws every one and closes none of them by itself.
        warn = {"alertInfo": {"sourceId": "x", "type": "warning", "title": "The TV is too hot", "message": "It will turn off to cool down.", "readable": "",
                              "showTimeout": False, "modal": True, "timeout": 0, "timestamp": 9,
                              "buttons": [{"label": "OK", "focus": True, "action": {"serviceURI": "luna://k", "serviceMethod": "/ok", "launchParams": {}}}]}}
        rc, out = self.run_host("--alert", "300:" + json.dumps(warn), "--wait", "3000", "--probe", "2900:alertTitle:text")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.states(out)[-1]["alerts"], 1)
        self.assertIn("[probe] alertTitle.text=The TV is too hot", out)
        rc, out = self.run_host("--toast", "300", "--alert", "400:" + json.dumps(warn), "--wait", "1500")
        self.assertEqual((self.states(out)[-1]["toasts"], self.states(out)[-1]["alerts"]), (1, 1))

    def test_the_pin_goes_out_only_when_four_digits_are_in_and_OK_is_pressed(self):
        rc, out = self.run_host("--pin", "300", "--press", "800:1,2,3", "--press", "1200:OK", "--calls", "--wait", "1800")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.actions(out), [])
        rc, out = self.run_host("--pin", "300", "--press", "800:1,2", "--press", "1000:BACKSPACE", "--press", "1100:3,4,5", "--press", "1500:OK", "--calls", "--wait", "2100")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.actions(out), [("luna://com.webos.notification", "/closePincodePrompt",
                                               '{"pincode":"1345","closeType":"relay","mode":"","appId":"com.webos.app.hdmi3"}')])

    def test_back_on_an_empty_pin_closes_it_and_sends_no_pin(self):
        rc, out = self.run_host("--pin", "300", "--press", "900:BACKSPACE", "--calls", "--wait", "1800")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.actions(out), [("luna://com.webos.notification", "/closePincodePrompt", '{"closeType":"close"}')])

    def test_the_pin_pad_moves_with_the_direction_keys_and_the_pad_keys_enter_digits(self):
        # focus starts on 5; Down 8, Down 0, Left Clear; Clear with nothing in is inert; Up from 0 is 8
        rc, out = self.run_host("--pin", "300", "--press", "800:DOWN,DOWN,RIGHT,LEFT,UP,OK", "--calls", "--wait", "1800", "--probe", "1600:pincodePromptPasswordButton:text")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.actions(out), [])
        self.assertNotIn("TypeError", out)

    def test_the_input_popup_switches_on_OK_and_stays_on_the_other_button(self):
        rc, out = self.run_host("--input", "300", "--press", "1200:OK", "--calls", "--wait", "2200")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.actions(out), [("luna://com.webos.applicationManager", "/launch", '{"id":"com.webos.app.hdmi3"}')])
        rc, out = self.run_host("--input", "300", "--press", "1000:RIGHT", "--press", "1300:OK", "--calls", "--wait", "2200")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertEqual(self.actions(out), [])
        self.assertEqual(self.states(out)[-1]["advanced"], 0)

    def test_the_alert_keeps_focus_on_its_first_button_and_moves_with_the_keys(self):
        rc, out = self.run_host("--alert", "300", "--press", "900:DOWN", "--wait", "1500")
        self.assertEqual(rc, 0, out[-1500:])
        self.assertIn("requestFocus alert", out)

    def test_stock_tree_loads_in_the_host_too(self):
        r = subprocess.run([sys.executable, HOST, "--toast", "300", "--alert", "600", "--wait", "1500"], env=dict(os.environ, DISPLAY=DISPLAY),
                           capture_output=True, text=True, timeout=120)
        self.assertEqual(r.returncode, 0, (r.stdout + r.stderr)[-1500:])


if __name__ == "__main__":
    unittest.main()
