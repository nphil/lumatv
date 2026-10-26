"""Run with: python3 -m unittest tests.test_settings_coverage   (needs Xvfb and PyQt5; skipped without them)

The pages that cover the rest of LG's Settings (docs/research/settings-coverage.md), through the offline host on Settings.qml alone:
every row of every page against the offline TV's refusals (its key lists and value lists are the owner's TV's own answers, LgDesc.js),
the keys a row writes together, the child pages and the way back, and the question that guards what cannot be undone.

The host takes a while to start, so one run answers every probe: each is a piece of JS the host evaluates on Settings.qml in turn
(pages are built with Settings, so by 5 s they all exist; a key press is `root.key(dir)`, as Home's key filter hands it over), and what it
returns, with the luna calls it caused, comes back under its name. A probe that changes the TV's state goes after the ones that read
it; the walk of every row is the last.
"""
import json
import os
import subprocess
import sys
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from xdisplay import DISPLAY, HAVE_QT  # noqa: E402
from test_settings import HOST, HostCase  # noqa: E402

AT = 5000
PROBES = []          # (name, js body that returns a value, wait after in ms)

# JS that finds the row of `page` labelled `label` and leaves it in `row` (with `p` the page).
FIND = "var p=root.pageItem('%s');function R(l){for(var i=0;i<p.rows.length;i++)if(p.ev(p.rows[i].label)===l)return p.rows[i]}"
# Every row of every page, as the offline TV can take it: a switch both ways, each value of a list, a slider at both ends. A row that is dimmed
# or left out is skipped, as a person could not press it. A switch ends on, so the rows that wait for it (Audio Guidance's speed) are walked too.
# the pages of this slice and the one it extends (others' pages are walked by their own tests)
OWN = ["system", "captions", "audioguide", "audiooptions", "clock", "poweron", "poweroff", "btaudio", "bthid", "wifi", "ipsettings", "network", "inputs", "picture"]
WALK = ("var n=0;var ids=" + json.dumps(OWN) + ";for(var ii=0;ii<ids.length;ii++){var p=root.pageItem(ids[ii]);if(!p||!p.rows)continue;"
        "for(var r=0;r<p.rows.length;r++){var row=p.rows[r];if(!row.act||row.info===true)continue;"
        "if(p.ev(row.enabled)===false||p.ev(row.visible)===false)continue;var k=row.kind;"
        "if(k==='toggle'){row.act(false);row.act(true);n+=2}"
        "else if(k==='seg'||k==='cards'){var c=p.count(r);for(var j=0;j<c;j++){row.act(j);n++}}"
        "else if(k==='slider'){row.act(p.ev(row.min));row.act(p.ev(row.max));n+=2}"
        "else if(k==='nudge'){row.act(1);row.act(-1);n+=2}"
        "else if(k==='value'&&row.options&&row.pick){var o=row.options();for(var q=0;q<o.length;q++){row.pick(q);n++}}}}return n")
SET = "var s=Qt.createQmlObject('import QtQuick 2.4; import WebOSCompositorBase 1.0; import WebOSServices 1.0; QtObject { function set(c, v) { LS.adhoc.call(\"luna://com.webos.service.settings\", \"/setSystemSettings\", JSON.stringify({category: c, settings: v})) } }', root);"


def probe(name, body, wait=250):
    PROBES.append((name, body, wait))


def wrapped(name, body):
    """The probe as one expression: its value, and every luna call it made, as a JSON string tagged with its name."""
    return ("(function(){var L=Qt.createQmlObject('import QtQuick 2.4; import WebOSCompositorBase 1.0; import WebOSServices 1.0; QtObject { property var calls: LS.calls }', root);"
            "var n0=L.calls.length;var res;try{res=(function(){%s})()}catch(e){res={error:String(e)}}"
            "var c=L.calls.slice(n0).filter(function(x){return !/^\\/getSystemSetting/.test(x.method)}).map(function(x){var p=null;try{p=JSON.parse(x.payload)}catch(e){}return {method:x.method,payload:p,error:x.error}});"
            "return JSON.stringify({tag:'%s',res:res,calls:c})})()" % (body, name))


# the pages that read a luna service when they are opened, opened first so that the reads have come back by the probes below
probe("open_clock", "root.openPage('clock');return 1", wait=400)

# ---- Accessibility and captions
probe("display_switches",
      FIND % "system" + "R('Text with High Contrast').act(true);R('Grayscale').act(true);var g1=p.ev(R('Grayscale').on);R('Invert Colors').act(true);"
      "var g2=p.ev(R('Grayscale').on),i2=p.ev(R('Invert Colors').on);R('Screen Zoom').act(true);"
      "return {g1:g1,g2:g2,i2:i2,zoom:p.ev(R('Screen Zoom').on),i3:p.ev(R('Invert Colors').on)}")
probe("choice_flow",
      "var out={};" + (FIND % "captions") + "out.options=R('Analog Mode').options().map(function(x){return x.value+':'+x.label});out.now=R('Analog Mode').current();"
      "out.value=p.ev(R('Analog Mode').value);root.goTo('captions');root.key('down');root.key('ok');out.at=root.current;var c=root.pageItem('choice');"
      "out.title=c.title;out.rows=c.rows.map(function(r){return c.ev(r.label)});out.stop=c.stop;root.key('down');root.key('ok');out.back=root.current;"
      "out.trail=root.trail;return out")
probe("back_trail",
      "var seen=[];root.goTo('captions');seen.push(root.current);root.key('down');root.key('ok');seen.push(root.current);root.key('back');seen.push(root.current);"
      "root.key('back');seen.push(root.current);seen.push(root.zone);return seen")
probe("captions_rows",
      "var p=root.pageItem('captions');var out=[];for(var i=0;i<p.rows.length;i++)if(p.ev(p.rows[i].visible)!==false)out.push(p.ev(p.rows[i].label)||'');return out")

# ---- the question
_Q = "{title:'Reset?',body:'',answers:[{id:'no',label:'Cancel'},{id:'yes',label:'Reset',danger:true}],focus:0}"
probe("question",
      "var a=[];root.confirm(%s,function(id){a.push(id)});var open=root.confirmOpen;root.key('right');root.key('ok');"
      "root.confirm(%s,function(id){a.push(id)});root.key('back');root.confirm(%s,function(id){a.push(id)});root.key('ok');return {a:a,open:open}" % (_Q, _Q, _Q),
      wait=400)
probe("question_keys",
      "root.goTo('captions');var before=root.pageItem('captions').stop;root.confirm({title:'x',answers:[{id:'no',label:'No'}],focus:0},function(){});"
      "root.key('down');root.key('down');return {before:before,after:root.pageItem('captions').stop,open:root.confirmOpen}", wait=400)

# ---- Time & power
probe("timer_writes",
      FIND % "poweron" + "R('Timer').act(true);R('Hour').act(1);R('Minute').act(1);R('Sunday').act(true);R('Tuesday').act(true);R('Sunday').act(false);"
      "var o=root.pageItem('poweroff');var inputs=o.rows.filter(function(x){return o.ev(x.visible)!==false&&o.ev(x.label)==='Input'}).length;"
      "return {mask:p.mask,hour:p.ev(R('Hour').value),min:p.ev(R('Minute').value),title:p.title,off:o.title,offInput:inputs}")
probe("timer_inputs",
      FIND % "poweron" + "var row=R('Input');return {now:p.ev(row.value),options:row.options().map(function(o){return o.label})}")
probe("zone_list",
      FIND % "clock" + "var row=R('Time Zone');var o=row.options().map(function(x){return x.label});var now=p.ev(row.value);row.pick(1);"
      "return {o:o,now:now,after:p.ev(row.value),cur:row.current()}", wait=300)
probe("time_and_power_rows",
      "var p=root.pageItem('system');var out=[];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(r.tab===1&&p.ev(r.visible)!==false)out.push(p.ev(r.label)||'')}return out")
probe("clock_by_hand",
      FIND % "clock" + "var locked=p.ev(R('Hour').enabled);var before=p.draft.getTime();R('Set Automatically').act(false);var open=p.ev(R('Hour').enabled);"
      "R('Hour').act(1);R('Hour').act(1);return {locked:locked,open:open,before:before/1000,after:p.draft.getTime()/1000}", wait=1500)
probe("clock_sent", "return L.calls.filter(function(x){return /setSystemTime|setPreferences/.test(x.method)}).map(function(x){return {method:x.method,payload:JSON.parse(x.payload)}})")

# ---- Bluetooth: speakers (btaudiosrc) and controllers (bthidmanager)
ROWS = ("var p=root.pageItem('%s');var out=[];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
        "out.push([p.ev(r.label)||p.ev(r.text)||'',String(p.ev(r.value)||'')])}")
probe("bt_audio_open", "root.openPage('btaudio');return 1", wait=400)
probe("bt_audio_rows", (ROWS % "btaudio") + "return out")
probe("bt_audio_search", FIND % "btaudio" + "R('Search').act();return 1", wait=400)
probe("bt_audio_found", FIND % "btaudio" + (ROWS % "btaudio").replace("var p=root.pageItem('btaudio');", "") + "R('Kitchen Speaker').act();return out", wait=400)
probe("bt_audio_paired", (ROWS % "btaudio") + "return out")
probe("bt_hid_open", "root.openPage('bthid');return 1", wait=400)
probe("bt_hid_rows", (ROWS % "bthid") + "return out")
probe("bt_hid_search", FIND % "bthid" + "R('Search').act();return 1", wait=400)
probe("bt_hid_found", FIND % "bthid" + "R('Keyboard K380').act();return 1", wait=400)
probe("bt_hid_paired", (ROWS % "bthid") + "return out")
probe("bt_device", "root.openPage('btaudio');root.openPage('btdevice',{kind:'audio',address:'AA:BB:CC:00:00:01',name:'Living Room Headphones',connected:false,back:'Bluetooth Speakers',backId:'btaudio'});"
      + (ROWS % "btdevice") + "return {title:root.pageItem('btdevice').title,rows:out,at:root.current}")
probe("bt_device_delete", FIND % "btdevice" + "R('Delete Device').act();var asked=root.confirmOpen;root.key('right');root.key('ok');return {asked:asked,at:root.current}", wait=400)
probe("bt_audio_gone", (ROWS % "btaudio") + "return out")

# ---- Network: joining, the address, the TV's name (typed on the keyboard: the probe types into it, then presses Done)
TYPE = "root.entryItem.text=%s;root.entryItem.press('Done');"
probe("wifi_open", "root.openPage('wifi');return 1", wait=400)
probe("wifi_rows", (ROWS % "wifi") + "return out")
probe("wifi_open_join", FIND % "wifi" + "R('Guest').act();return {entry:root.entryOpen,confirm:root.confirmOpen}", wait=150)
probe("wifi_open_confirm", "var asked=root.confirmOpen;root.key('right');root.key('ok');return {asked:asked}", wait=300)
probe("wifi_secured", FIND % "wifi" + "R('Neighbour 5G').act();var o={entry:root.entryOpen,secret:root.entryItem.secret,prompt:root.entryItem.prompt};"
      + (TYPE % "'hunter22'") + "return o", wait=150)
probe("wifi_secured_confirm", "var asked=root.confirmOpen;root.key('right');root.key('ok');return {asked:asked}", wait=300)
probe("ip_open", "root.openPage('ipsettings');return 1", wait=400)
probe("ip_rows", (ROWS % "ipsettings") + "return out")
probe("ip_bad_address", FIND % "ipsettings" + "R('IP Address Setting').act(1);R('IP Address').act();" + (TYPE % "'999.1.1.1'")
      + "return {ready:p.ready,apply:p.ev(R('Apply').enabled),detail:p.ev(R('Apply').detail)}", wait=200)
probe("ip_apply", FIND % "ipsettings" + "R('IP Address').act();" + (TYPE % "'192.168.1.50'") + "return {ready:p.ready}", wait=200)
probe("ip_apply_ask", FIND % "ipsettings" + "R('Apply').act();var asked=root.confirmOpen;root.key('right');root.key('ok');return {asked:asked}", wait=300)
probe("tv_name", "var p=root.pageItem('network');function R(l){for(var i=0;i<p.rows.length;i++)if(p.ev(p.rows[i].label)===l)return p.rows[i]}"
      "var before=p.ev(R('TV Name').value);R('TV Name').act();" + (TYPE % "'Den TV'") + "return {before:before}", wait=200)
probe("tv_name_after", "var p=root.pageItem('network');for(var i=0;i<p.rows.length;i++)if(p.ev(p.rows[i].label)==='TV Name')return {name:p.ev(p.rows[i].value)}")

# ---- Picture: the rows of each tab (the ones the TV hides are left out), the user TruMotion mode's two sliders, Apply to All Inputs and Reset
PICTURE_TABS = ("var p=root.pageItem('picture');var out=[[],[],[],[],[],[]];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];"
                "if(p.ev(r.visible)===false||r.label===undefined)continue;out[r.tab||0].push(String(p.ev(r.label)))}return out")
probe("picture_tabs", PICTURE_TABS)
probe("picture_user_motion", FIND % "picture" + "var o=R('TruMotion').options().map(function(x){return x.value});R('TruMotion').pick(o.indexOf('user'));return {options:o}", wait=200)
probe("picture_tabs_user", PICTURE_TABS)
probe("picture_apply", FIND % "picture" + "R('Apply to All Inputs').act();return {asked:root.confirmOpen}", wait=250)
probe("picture_apply_yes", "root.key('right');root.key('ok');return 1", wait=250)
probe("picture_reset", FIND % "picture" + "R('Reset').act();return {asked:root.confirmOpen,danger:!!p.ev(R('Reset').danger)}", wait=250)
probe("picture_reset_no", "root.key('ok');return 1", wait=250)
probe("picture_reset_yes", FIND % "picture" + "R('Reset').act();return 1", wait=250)
probe("picture_reset_go", "root.key('right');root.key('ok');return 1", wait=250)

# ---- every row against the TV's refusals (the last, it changes what the pages show)
probe("walk", "var s=Qt.createQmlObject('import QtQuick 2.4; import WebOSCompositorBase 1.0; import WebOSServices 1.0; QtObject { function set(c, v) { LS.adhoc.call(\"luna://com.webos.service.settings\", "
      "\"/setSystemSettings\", JSON.stringify({category: c, settings: v})) } }', root);s.set('caption', {captionStyle: 'manual'});root.pageItem('captions').refresh();"
      "return 0", wait=400)
probe("walk_rows", WALK, wait=500)
probe("refusals",
      SET + "s.set('caption', {captionSize: 'huge'});s.set('caption', {captionTextColor: 'white'});s.set('caption', {noSuchKey: 'x'});return 0")


def run_probes():
    args = []
    at = AT
    for name, body, wait in PROBES:
        args += ["--eval", "%d:%s" % (at, wrapped(name, body))]
        at += wait
    # the shared helper gives a host 180 s; this one has every probe to answer, and the machine that runs it may be busy
    cmd = [sys.executable, HOST, "Settings.qml", "--fake-helper", "--prop", "hostActive=true", "--theme", '{"perf":{"syncResidents":true}}', *args, "--wait", str(at + 600)]
    r = subprocess.run(cmd, env=dict(os.environ, DISPLAY=DISPLAY), capture_output=True, text=True, timeout=900)
    rc, out = r.returncode, r.stdout + r.stderr
    found = {}
    for doc in HostCase.docs(None, out):
        if isinstance(doc, dict) and "tag" in doc:
            found[doc["tag"]] = doc
    return rc, out, found


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class Coverage(unittest.TestCase):
    found = {}
    out = ""

    @classmethod
    def setUpClass(cls):
        rc, cls.out, cls.found = run_probes()
        cls.rc = rc

    def probe(self, name):
        self.assertEqual(self.rc, 0, self.out[-1500:])
        self.assertNotIn("did not load", self.out)
        self.assertIn(name, self.found, "the probe did not answer: %s" % self.out[-1500:])
        doc = self.found[name]
        self.assertNotIsInstance(doc["res"], str, doc)
        self.assertNotIn("error", doc["res"] if isinstance(doc["res"], dict) else {}, doc)
        return doc

    def test_every_probe_answered(self):
        self.assertEqual(sorted(self.found), sorted(n for n, _, _ in PROBES), self.out[-1500:])

    # ---- every row against the TV's own lists
    def test_no_row_writes_a_key_or_value_the_tv_does_not_list(self):
        doc = self.probe("walk_rows")
        self.assertGreater(doc["res"], 60, "the walk found too few rows")
        self.assertEqual([c for c in doc["calls"] if c["error"]], [])
        written = {k for c in doc["calls"] if c["payload"] for k in c["payload"].get("settings", {})}
        want = {"captionAnalog", "captionDigital", "captionSize", "captionFont", "captionTextColor", "captionBgColor", "captionWindowOpacity",
                "subtitleLanguageFirst", "audioGuidanceSpeed", "supplementaryAudio", "hearingAid", "dialogEnhancementVolumeAtsc30",
                "highContrast", "adjustments", "onTimerEnable", "onTimerHour", "onTimerMinute", "onTimerWeekday", "onTimerSource", "onTimerVolume",
                "offTimerEnable", "autoClock", "qmsVrr", "uhdDeepColor", "444BypassHDMI1", "444BypassHDMI4", "enableDolbyVisionPC",
                "ipv6Enable", "bleAdvertisingOnOff", "dynamicContrast", "peakBrightness", "gamma", "motionEyeCare", "colorTemperature", "colorGamut",
                "dynamicColor", "superResolution", "noiseReduction", "mpegNoiseReduction", "smoothGradation", "realCinema", "motionPro", "filmMakerMode", "justScan",
                "truMotionMode", "truMotionJudder"}
        self.assertEqual(want - written, set())
        self.assertEqual(written & {"nearBlackDetail", "expressionEnhancer", "dolbyPrecisionDetail", "hdrDynamicToneMapping"}, set())       # the TV hides them

    def test_the_stub_refuses_a_caption_value_the_tv_does_not_list(self):
        doc = self.probe("refusals")
        self.assertEqual([bool(c["error"]) for c in doc["calls"]], [True, False, True])

    # ---- the five display switches write what LG's own rows write together
    def test_display_accessibility_switches_write_the_keys_LG_writes_with_them(self):
        doc = self.probe("display_switches")
        sent = [c["payload"] for c in doc["calls"]]
        off = {"blackAndWhite": "off", "colorInversion": "off"}
        self.assertEqual([c["category"] for c in sent], ["option"] * 6)
        self.assertEqual(sent[0]["settings"], {"highContrast": "on", "adjustments": off})                                   # high contrast clears the pair
        self.assertEqual(sent[1]["settings"], {"screenMagnification": "off"})                                                # grayscale: the zoom off first,
        self.assertEqual(sent[2]["settings"], {"adjustments": {"blackAndWhite": "on", "colorInversion": "off"}, "highContrast": "off", "focusEmphasis": "off"})
        self.assertEqual(sent[4]["settings"], {"adjustments": {"blackAndWhite": "off", "colorInversion": "on"}, "highContrast": "off", "focusEmphasis": "off"})
        self.assertEqual(sent[5]["settings"], {"screenMagnification": "on", "adjustments": off})
        self.assertEqual([c["error"] for c in doc["calls"]], [""] * 6)
        self.assertEqual(doc["res"], {"g1": True, "g2": False, "i2": True, "zoom": True, "i3": False})                       # the one in force last wins

    # ---- the chooser and the way back
    def test_a_choice_row_offers_the_tvs_list_in_LGs_names_and_writes_one(self):
        doc = self.probe("choice_flow")
        d = doc["res"]
        self.assertEqual(d["options"], ["cc1:CC 1", "cc2:CC 2", "cc3:CC 3", "cc4:CC 4", "t1:Text 1", "t2:Text 2", "t3:Text 3", "t4:Text 4"])   # eng and kor are hidden
        self.assertEqual((d["now"], d["value"], d["at"], d["title"]), ("cc1", "CC 1", "choice", "Analog Mode"))
        self.assertEqual(d["rows"], ["CC 1", "CC 2", "CC 3", "CC 4", "Text 1", "Text 2", "Text 3", "Text 4"])
        self.assertEqual(d["stop"], 0)                                           # the focus starts on the value in force
        self.assertEqual((d["back"], d["trail"]), ("captions", []))              # a pick goes back to the page that opened the list
        self.assertEqual([c["payload"]["settings"] for c in doc["calls"]], [{"captionAnalog": "cc2"}])

    def test_back_goes_to_the_page_a_child_was_opened_from_at_any_depth(self):
        self.assertEqual(self.probe("back_trail")["res"], ["captions", "choice", "captions", "system", "page"])

    def test_a_row_the_tv_hides_is_left_out(self):
        shown = self.probe("captions_rows")["res"]
        self.assertIn("Hard of Hearing", shown)
        self.assertNotIn("AI Caption", shown)                                    # LG hides it on this set

    # ---- the question
    def test_the_question_has_the_safe_answer_first_and_back_means_it(self):
        d = self.probe("question")["res"]
        self.assertEqual(d["a"], ["yes", "no", "no"])                           # Right then OK is the doing answer; Back, and OK alone, the safe one
        self.assertTrue(d["open"])

    def test_keys_go_to_the_question_while_it_is_up(self):
        d = self.probe("question_keys")["res"]
        self.assertEqual(d["before"], d["after"])
        self.assertTrue(d["open"])

    # ---- Time & power: the timers and the clock
    def test_a_timer_writes_LGs_hour_minute_and_day_mask(self):
        doc = self.probe("timer_writes")
        d = doc["res"]
        self.assertEqual((d["mask"], d["hour"], d["min"], d["title"], d["off"], d["offInput"]), (4, "8 AM", "31", "Power On Timer", "Power Off Timer", 0))
        self.assertEqual([c["payload"]["settings"] for c in doc["calls"]],
                         [{"onTimerEnable": "on"}, {"onTimerHour": "8"}, {"onTimerMinute": "31"}, {"onTimerWeekday": "1"}, {"onTimerWeekday": "5"}, {"onTimerWeekday": "4"}])
        self.assertEqual([c["error"] for c in doc["calls"]], [""] * 6)

    def test_the_power_on_timer_offers_the_hdmi_inputs_the_tv_lists(self):
        self.assertEqual(self.probe("timer_inputs")["res"], {"now": "HDMI 1", "options": ["HDMI 1", "HDMI 2", "HDMI 3", "HDMI 4"]})

    def test_the_time_zone_list_is_the_tvs_for_the_country_of_the_zone_in_force(self):
        doc = self.probe("zone_list")
        d = doc["res"]
        self.assertEqual((d["o"], d["now"], d["after"], d["cur"]), (["New York", "Chicago", "Denver", "Los Angeles"], "New York", "Chicago", "America/Chicago"))
        settings = [c for c in doc["calls"] if c["method"] == "/setSystemSettings"]
        self.assertEqual([c["payload"]["settings"]["timeZone"]["ZoneID"] for c in settings], ["America/Chicago"])
        self.assertEqual([c["payload"] for c in doc["calls"] if c["method"] == "/setPreferences"][0]["timeZone"]["ZoneID"], "America/Chicago")   # and the preference, by luna
        self.assertEqual([c["error"] for c in doc["calls"]], [""] * len(doc["calls"]))

    def test_time_and_power_lists_what_LG_lists_and_leaves_out_what_the_tv_hides(self):
        self.assertEqual(self.probe("time_and_power_rows")["res"],
                         ["Sleep Timer", "Time & Date", "Power On Timer", "Power Off Timer", "No Signal Auto Off", "Auto Power Off after 4 Hours", "Screen Off", "Quick Start+"])

    def test_the_clock_is_sent_once_a_second_after_the_last_press_and_only_by_hand(self):
        d = self.probe("clock_by_hand")["res"]
        self.assertEqual((d["locked"], d["open"]), (False, True))                 # the rows answer only once Set Automatically is off
        self.assertEqual(d["after"] - d["before"], 7200)
        sent = self.probe("clock_sent")["res"][-2:]                               # after the zone's own preference from the earlier probe
        self.assertEqual([c["method"] for c in sent], ["/setPreferences", "/time/setSystemTime"])           # two presses, one send
        self.assertEqual(sent[0]["payload"], {"useNetworkTime": False})
        self.assertEqual(sent[1]["payload"]["utc"], d["after"])

    # ---- Bluetooth
    def test_the_speakers_page_lists_what_is_paired_and_finds_more(self):
        self.assertEqual(self.probe("bt_audio_rows")["res"], [["Speaker Connection", "1 device"], ["Registered Devices (1)", ""], ["Living Room Headphones", ""],
                                                              ["Connectable Devices (0)", ""], ["Search", ""]])
        found = self.probe("bt_audio_found")
        self.assertEqual(found["res"][-3:], [["Connectable Devices (1)", ""], ["Kitchen Speaker", ""], ["Searching", ""]])
        self.assertEqual([(c["method"], c["payload"]) for c in found["calls"] if c["method"] == "/connect"], [("/connect", {"address": "AA:BB:CC:00:00:02"})])
        self.assertIn(["Kitchen Speaker", "Connected"], self.probe("bt_audio_paired")["res"])                    # read again after the connection
        self.assertIn(["Connectable Devices (0)", ""], self.probe("bt_audio_paired")["res"])

    def test_the_controller_page_pairs_with_the_name_and_address_LG_sends(self):
        self.assertEqual(self.probe("bt_hid_rows")["res"], [["Registered Devices (1)", ""], ["Wireless Controller", ""], ["Connectable Devices (0)", ""], ["Search", ""]])
        found = self.probe("bt_hid_found")
        self.assertEqual([c["payload"] for c in found["calls"] if c["method"] == "/requestConnect"], [{"name": "Keyboard K380", "address": "11:22:33:00:00:02"}])
        self.assertIn(["Keyboard K380", "Connected"], self.probe("bt_hid_paired")["res"])

    def test_a_device_is_deleted_only_after_the_question_is_answered_yes(self):
        d = self.probe("bt_device")["res"]
        self.assertEqual((d["title"], d["rows"], d["at"]), ("Living Room Headphones", [["Connect", ""], ["Delete Device", ""]], "btdevice"))
        doc = self.probe("bt_device_delete")
        self.assertEqual(doc["res"], {"asked": True, "at": "btaudio"})                                          # then back to the list it came from
        self.assertEqual([c["payload"] for c in doc["calls"] if c["method"] == "/removePairedDevice"], [{"address": "AA:BB:CC:00:00:01"}])
        self.assertNotIn(["Living Room Headphones", ""], self.probe("bt_audio_gone")["res"])

    # ---- Network
    def test_join_network_lists_what_the_wifi_service_found_and_joins_after_the_question(self):
        self.assertEqual(self.probe("wifi_rows")["res"], [["Nate's Home", "Strong \u00b7 Secured"], ["Guest", "Good"], ["Neighbour 5G", "Weak \u00b7 Secured"],
                                                          ["Other Network", ""], ["Search Again", ""]])
        self.assertEqual(self.probe("wifi_open_join")["res"], {"entry": False, "confirm": True})               # an open network needs no password, only the question
        joined = self.probe("wifi_open_confirm")
        self.assertEqual([c["payload"] for c in joined["calls"] if c["method"] == "/connect"], [{"ssid": "Guest"}])

    def test_a_password_is_typed_hidden_and_goes_straight_to_the_wifi_service(self):
        d = self.probe("wifi_secured")["res"]
        self.assertEqual((d["entry"], d["secret"], d["prompt"]), (True, True, "Password for Neighbour 5G"))
        doc = self.probe("wifi_secured_confirm")
        self.assertTrue(doc["res"]["asked"])                                                                   # the question comes after the password
        self.assertEqual([c["payload"] for c in doc["calls"] if c["method"] == "/connect"],
                         [{"ssid": "Neighbour 5G", "security": {"securityType": "psk", "simpleSecurity": {"passKey": "hunter22"}}}])

    def test_ip_settings_read_the_link_and_apply_only_valid_addresses_after_the_question(self):
        rows = self.probe("ip_rows")["res"]
        self.assertEqual(rows, [["IP Address Setting", ""], ["IP Address", "192.168.1.146"], ["Subnet Mask", "255.255.255.0"], ["Gateway", "192.168.1.1"],
                                ["DNS Server", "192.168.1.1"], ["Apply", ""]])
        bad = self.probe("ip_bad_address")["res"]
        self.assertEqual((bad["ready"], bad["apply"], bad["detail"]), (False, False, "Check the addresses"))
        self.assertEqual(self.probe("ip_apply")["res"], {"ready": True})
        doc = self.probe("ip_apply_ask")
        self.assertTrue(doc["res"]["asked"])
        self.assertEqual([c["payload"] for c in doc["calls"] if c["method"] == "/setipv4"],
                         [{"method": "manual", "ssid": "Neighbour 5G", "address": "192.168.1.50", "netmask": "255.255.255.0", "gateway": "192.168.1.1"}])
        self.assertEqual([c["payload"] for c in doc["calls"] if c["method"] == "/setdns"], [{"dns": ["192.168.1.1"], "ssid": "Neighbour 5G"}])
        self.assertEqual([c["error"] for c in doc["calls"] if c["method"] in ("/setipv4", "/setdns")], ["", ""])

    def test_the_tvs_name_is_typed_and_written_as_the_setting(self):
        self.assertEqual(self.probe("tv_name")["res"]["before"], "Living Room TV")
        self.assertEqual([c["payload"]["settings"] for c in self.probe("tv_name")["calls"] if c["method"] == "/setSystemSettings"], [{"deviceName": "Den TV"}])
        self.assertEqual(self.probe("tv_name_after")["res"], {"name": "Den TV"})

    # ---- Picture
    def test_the_picture_tabs_list_what_LG_lists_and_leave_out_what_the_tv_hides(self):
        self.assertEqual(self.probe("picture_tabs")["res"], [
            ["OLED pixel brightness", "AI Picture Pro", "Aspect ratio", "Energy saving", "Just Scan", "More in LG settings"],
            ["Contrast", "Brightness", "Eye comfort", "Auto Dynamic Contrast", "Peak Brightness", "Gamma", "Motion Eye Care", "More in LG settings"],
            ["Colour", "Tint", "Color Temperature", "Color Gamut", "Color Adjustment", "More in LG settings"],
            ["Sharpness", "Super Resolution", "Noise Reduction", "MPEG Noise Reduction", "Smooth Gradation", "Real Cinema"],
            ["TruMotion", "OLED Motion Pro"],
            ["Filmmaker Auto Start", "AI genre selection", "AI brightness", "Apply to All Inputs", "Reset"]])

    def test_de_judder_and_de_blur_wait_for_the_user_motion_mode(self):
        self.assertEqual(self.probe("picture_user_motion")["res"]["options"], ["off", "natural", "smooth", "clear", "user"])
        self.assertEqual(self.probe("picture_tabs_user")["res"][4], ["TruMotion", "De-Judder", "De-Blur", "OLED Motion Pro"])
        sent = [c["payload"] for c in self.probe("picture_user_motion")["calls"] if c["method"] == "/setSystemSettings"]
        self.assertEqual([(p["settings"], p["app_id"]) for p in sent], [({"truMotionMode": "user"}, "com.webos.app.hdmi2")])

    def test_apply_to_all_inputs_asks_first_and_then_writes_the_key_LG_writes(self):
        self.assertTrue(self.probe("picture_apply")["res"]["asked"])
        self.assertEqual([c["payload"]["settings"] for c in self.probe("picture_apply_yes")["calls"] if c["method"] == "/setSystemSettings"], [{"applyToAllInput": "picture"}])

    def test_reset_asks_in_the_danger_style_and_names_the_input_and_the_mode_it_resets(self):
        d = self.probe("picture_reset")["res"]
        self.assertEqual((d["asked"], d["danger"]), (True, True))
        self.assertEqual([c for c in self.probe("picture_reset_no")["calls"] if c["method"] == "/resetSystemSettings"], [])        # Back or Cancel resets nothing
        done = [c for c in self.probe("picture_reset_go")["calls"] if c["method"] == "/resetSystemSettings"]
        self.assertEqual([c["payload"] for c in done], [{"category": "picture", "dimension": {"input": "default", "pictureMode": "filmMaker"}}])
        self.assertEqual([c["error"] for c in done], [""])


if __name__ == "__main__":
    unittest.main()
