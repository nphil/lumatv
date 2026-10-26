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
OWN = ["system", "captions", "audioguide", "audiooptions", "clock", "poweron", "poweroff", "btaudio", "bthid", "wifi", "ipsettings", "network", "inputs", "picture",
       "equalizer", "sound", "hdmiaudio", "language", "support", "channels", "whitebalance", "finetune", "signalling", "emergency", "permissions"]       # the equalizer first: it answers in the Standard mode, which the sound page's own walk of the modes leaves
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

# the HDMI rows name their port in the label (the second line is the reason a row is dimmed, so a port named there was lost on the set)
probe("hdmi_labels", "var p=root.pageItem('inputs');return p.rows.map(function(r){return p.ev(r.label)}).filter(function(l){return /4:4:4/.test(l)})")
# the Time & Date row shows the clock of the moment the page is shown: the page is built at start and the probes come seconds later
probe("system_open", "root.openPage('system');return 1", wait=400)
probe("system_clock", "var p=root.pageItem('system');return {age:Date.now()-p.now.getTime(),text:p.clockText()}")

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
probe("bt_playback_locked", FIND % "btaudio" + "var o=R('Playback Option');return {enabled:p.ev(o.enabled)!==false,detail:p.ev(o.detail),value:p.ev(o.value),volume:!!R('Bluetooth Speaker Volume')}")
probe("bt_audio_search", FIND % "btaudio" + "R('Search').act();return 1", wait=400)
probe("bt_audio_found", FIND % "btaudio" + (ROWS % "btaudio").replace("var p=root.pageItem('btaudio');", "") + "R('Kitchen Speaker').act();return out", wait=400)
probe("bt_audio_paired", (ROWS % "btaudio") + "return out")
probe("bt_playback", FIND % "btaudio" + "var o=R('Playback Option');var en=p.ev(o.enabled)!==false;var opts=o.options().map(function(x){return x.value+':'+x.label});o.pick(0);"
      "var v=R('Bluetooth Speaker Volume');var before=p.ev(v.valueText);v.act(7);return {enabled:en,opts:opts,mode:p.ev(o.value),before:before,after:p.ev(v.valueText)}", wait=300)
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

# ---- Sound: the rows of each tab, the chooser on the TV's own list, what the sound out changes, the equalizer, the audio format per input, Apply and Reset
SOUND_TABS = ("var p=root.pageItem('sound');var out=[[],[],[],[]];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];"
              "if(p.ev(r.visible)===false||r.label===undefined)continue;out[r.tab||0].push(String(p.ev(r.label)))}return out")
SOUND_STATE = ("function S(l){var r=R(l);return {on:p.ev(r.enabled)!==false,detail:String(p.ev(r.detail)||'')}}"
               "return {balance:S('Balance'),eq:S('Equalizer'),install:S('Installation Type'),acoustic:S('AI Acoustic Tuning Options'),speakersToo:S('TV speakers too'),"
               "atmos:S('Dolby Atmos'),lgsync:S('LG Sound Sync'),share:S('TV Sound Mode Share'),reset:S('Reset')}")
probe("sound_tabs", SOUND_TABS)
probe("sound_state_receiver", FIND % "sound" + SOUND_STATE)
probe("sound_out_options", FIND % "sound" + "var row=R('Sound out');return {now:row.current(),value:p.ev(row.value),options:row.options().map(function(o){return o.value+':'+o.label})}")
probe("sound_out_open", "root.goTo('sound');root.key('ok');return {at:root.current}", wait=300)
probe("sound_out_chooser", "var p=root.pageItem('choice');return {title:p.title,rows:p.rows.map(function(r){return p.ev(r.label)}),stop:p.stop,back:p.back}")
probe("sound_out_picked", "root.key('up');root.key('ok');return {at:root.current,out:root.pageItem('sound').output}", wait=400)
probe("sound_tabs_tv", SOUND_TABS)
probe("sound_state_tv", FIND % "sound" + SOUND_STATE)
probe("eq_open", "root.openPage('equalizer');return 1", wait=400)
probe("eq_rows", "var p=root.pageItem('equalizer');return p.rows.map(function(r){return [p.ev(r.label),p.ev(r.valueText),p.ev(r.enabled)!==false]})")
probe("eq_band", FIND % "equalizer" + "R('1 kHz').act(5);R('100 Hz').act(-3);return {text:p.ev(R('1 kHz').valueText),bass:p.ev(R('100 Hz').valueText),levels:p.levels,min:p.ev(R('1 kHz').min),max:p.ev(R('1 kHz').max)}", wait=300)
probe("hdmi_audio_open", "root.openPage('hdmiaudio');return 1", wait=400)
probe("hdmi_audio_rows", "var p=root.pageItem('hdmiaudio');var out=[];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
      "var o=p.ev(r.options);out.push([p.ev(r.label),o.join('/'),o[p.ev(r.index)]])}return out")
probe("hdmi_audio_pick", FIND % "hdmiaudio" + "R('HDMI 3').act(0);R('HDMI 1').act(1);return 1", wait=300)
probe("sound_apply", FIND % "sound" + "R('Apply to All Inputs').act();return {asked:root.confirmOpen}", wait=250)
probe("sound_apply_yes", "root.key('right');root.key('ok');return 1", wait=250)
probe("sound_reset", FIND % "sound" + "R('Balance').act(4);var before=p.ev(R('Balance').num);R('Reset').act();return {asked:root.confirmOpen,danger:!!p.ev(R('Reset').danger),before:before}", wait=250)
probe("sound_reset_no", "root.key('ok');return 1", wait=250)
probe("sound_reset_go", FIND % "sound" + "R('Reset').act();root.key('right');root.key('ok');return 1", wait=400)
probe("sound_reset_after", FIND % "sound" + "return {balance:p.ev(R('Balance').num)}")
probe("sound_movie", SET + "s.set('sound', {soundMode: 'movie'});root.pageItem('sound').refresh();return 1", wait=400)
probe("sound_reset_movie", FIND % "sound" + "var r=R('Reset');return {on:p.ev(r.enabled)!==false,detail:p.ev(r.detail),eq:p.ev(R('Equalizer').detail)}")
probe("sound_standard", SET + "s.set('sound', {soundMode: 'standard'});root.pageItem('sound').refresh();return 1", wait=300)
probe("sound_optical", SET + "s.set('sound', {soundOutput: 'external_optical'});root.pageItem('sound').refresh();return 1", wait=400)
probe("sound_lgsync", FIND % "sound" + "var r=R('LG Sound Sync');var on=p.ev(r.enabled)!==false;r.act(true);return {on:on,now:p.ev(r.on),balance:p.ev(R('Balance').detail)}", wait=300)
probe("sound_arc", SET + "s.set('sound', {soundOutput: 'external_arc'});root.pageItem('sound').refresh();return 1", wait=400)
probe("sound_share", FIND % "sound" + "var r=R('TV Sound Mode Share');var on=p.ev(r.enabled)!==false;r.act(true);return {on:on,now:p.ev(r.on)}", wait=300)
probe("sound_tv_again", SET + "s.set('sound', {soundOutput: 'tv_speaker'});root.pageItem('sound').refresh();return 1", wait=400)

# ---- Language and location: the lists the TV gives, the menu language (asks first), the audio language, the keyboards, the country switch, the zip code
LANG_TABS = ("var p=root.pageItem('language');var out=[[],[],[]];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
             "out[r.tab||0].push(String(p.ev(r.label)))}return out")
KEYBOARD = "function K(l){var r=R(l);return {on:p.ev(r.on),enabled:p.ev(r.enabled)!==false,detail:p.ev(r.detail)}}"
probe("lang_open", "root.openPage('language');return 1", wait=600)
probe("lang_tabs", LANG_TABS)
probe("lang_menu_options", FIND % "language" + "var row=R('Menu Language');return {now:row.current(),value:p.ev(row.value),options:row.options().map(function(o){return o.value+':'+o.label})}")
probe("lang_menu_ask", FIND % "language" + "var row=R('Menu Language');row.pick(row.options().map(function(o){return o.value}).indexOf('es-US'));return {asked:root.confirmOpen,value:p.ev(row.value)}", wait=250)
probe("lang_menu_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("lang_menu_yes", FIND % "language" + "var row=R('Menu Language');row.pick(row.options().map(function(o){return o.value}).indexOf('es-US'));root.key('right');root.key('ok');return 1", wait=400)
probe("lang_menu_after", FIND % "language" + "return {info:p.info,value:p.ev(R('Menu Language').value)}")
probe("lang_keyboards", FIND % "language" + KEYBOARD + "return {en:K('English'),es:K('Espa\\u00f1ol'),fr:K('Fran\\u00e7ais')}")
probe("lang_keyboard_fr", FIND % "language" + KEYBOARD + "R('Fran\\u00e7ais').act(true);return K('Fran\\u00e7ais')", wait=300)
probe("lang_audio", FIND % "language" + "var row=R('Audio Language');var o=row.options();row.pick(o.map(function(x){return x.value}).indexOf('fr-FR'));return {value:p.ev(row.value),asked:root.confirmOpen,options:o.map(function(x){return x.value+':'+x.label})}", wait=300)
probe("lang_location", FIND % "language" + "return {country:p.ev(R('Broadcast Country').value),svc:p.ev(R('LG Services Country').on),zip:p.ev(R('Zip Code').value)}")
probe("lang_svc_ask", FIND % "language" + "R('LG Services Country').act(false);return {asked:root.confirmOpen,on:p.ev(R('LG Services Country').on)}", wait=250)
probe("lang_svc_off", "root.key('right');root.key('ok');return 1", wait=300)
probe("lang_svc_on", FIND % "language" + "R('LG Services Country').act(true);root.key('right');root.key('ok');return 1", wait=400)
probe("lang_svc_after", FIND % "language" + "return {on:p.ev(R('LG Services Country').on)}")
probe("lang_zip_bad", FIND % "language" + "R('Zip Code').act();" + (TYPE % "'123'") + "return 1", wait=200)
probe("lang_zip_bad_state", FIND % "language" + "return {detail:p.ev(R('Zip Code').detail),value:p.ev(R('Zip Code').value)}")
probe("lang_zip", FIND % "language" + "R('Zip Code').act();" + (TYPE % "'90210'") + "return 1", wait=200)
probe("lang_zip_after", FIND % "language" + "return {detail:p.ev(R('Zip Code').detail),value:p.ev(R('Zip Code').value)}")

# ---- Support: what LG's TV Information shows, the update state, Auto Update (LG's terms first), privacy, the tools, and the reset behind its questions
SUPPORT_TABS = ("var p=root.pageItem('support');var out=[[],[],[],[],[]];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
                "out[r.tab||0].push(String(p.ev(r.label)))}return out")
SUPPORT_VALUES = ("var p=root.pageItem('support');var out={};for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false||r.kind!=='value')continue;"
                  "out[String(p.ev(r.label))]=String(p.ev(r.value))}return out")
probe("support_open", "root.openPage('support');return 1", wait=900)
probe("support_tabs", SUPPORT_TABS)
probe("support_info", SUPPORT_VALUES)
probe("support_auto_state", FIND % "support" + "var r=R('Auto Update');return {on:p.ev(r.on),enabled:p.ev(r.enabled)!==false,detail:p.ev(r.detail)}")
probe("support_auto_on", FIND % "support" + "R('Auto Update').act(true);return p.ev(R('Auto Update').on)", wait=250)
probe("support_no_terms", SET + "s.set('', {eulaStatus: {generalTermsAllowed: false}});s.set('support', {automaticUpdate: 'off'});root.pageItem('support').refresh();return 1", wait=500)
probe("support_auto_locked", FIND % "support" + "var r=R('Auto Update');return {on:p.ev(r.on),enabled:p.ev(r.enabled)!==false,detail:p.ev(r.detail)}")
probe("support_terms_back", SET + "s.set('', {eulaStatus: {generalTermsAllowed: true, voiceAllowed: true}});root.pageItem('support').refresh();return 1", wait=500)
probe("support_dns_ask", FIND % "support" + "R('Do Not Sell My Personal Information').act(true);return {asked:root.confirmOpen,on:p.ev(R('Do Not Sell My Personal Information').on)}", wait=250)
probe("support_dns_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("support_dns_yes", FIND % "support" + "R('Do Not Sell My Personal Information').act(true);root.key('right');root.key('ok');return 1", wait=1200)
probe("support_dns_after", FIND % "support" + "return {on:p.ev(R('Do Not Sell My Personal Information').on)}")
probe("support_lmt", FIND % "support" + "R('Limit AD Tracking').act(true);return p.ev(R('Limit AD Tracking').on)", wait=250)
probe("support_ad_id", FIND % "support" + "return {id:p.ev(R('AD ID').value)}")
probe("support_ad_reset", FIND % "support" + "R('Reset AD ID').act();return {asked:root.confirmOpen}", wait=250)
probe("support_ad_reset_yes", "root.key('right');root.key('ok');return 1", wait=1300)
probe("support_ad_after", FIND % "support" + "return {id:p.ev(R('AD ID').value)}")
probe("support_memory", FIND % "support" + "R('Memory Optimizer').act();var asked=root.confirmOpen;root.key('right');root.key('ok');return {asked:asked,button:p.ev(R('Memory Optimizer').button)}", wait=300)
probe("support_tools", FIND % "support" + "return p.rows.filter(function(r){return r.tab===3}).map(function(r){return [p.ev(r.label),r.app||'',r.target||'']})")
probe("support_reset_first", FIND % "support" + "R('Reset All').act();return {asked:root.confirmOpen}", wait=250)
probe("support_reset_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("support_reset_continue", FIND % "support" + "R('Reset All').act();root.key('right');root.key('ok');return 1", wait=900)
probe("support_reset_pin", "var pins=L.calls.filter(function(x){return x.method==='/createPincodePrompt'}).map(function(x){return JSON.parse(x.payload)});"
      "return {second:root.confirmOpen,pins:pins,resets:L.calls.filter(function(x){return x.method==='/doReset'}).length}", wait=250)
probe("support_reset_second_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("support_reset_again", FIND % "support" + "R('Reset All Except Network Settings').act();root.key('right');root.key('ok');return 1", wait=900)
probe("support_reset_second_yes", "var second=root.confirmOpen;root.key('right');root.key('ok');return {second:second}", wait=300)

# ---- Safety: the locks' state, read only
probe("safety_open", "root.openPage('safety');return 1", wait=700)
probe("safety_rows", "var p=root.pageItem('safety');var out=[];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
      "out.push([String(p.ev(r.label)),String(p.ev(r.value)||''),!!r.act&&!r.app])}return out")
probe("safety_locked", SET + "s.set('lock', {parentalControl: 'true', movieRating: 'pg13', tvRatingGeneralAge: 'pg', inputBlock: '5', applockPerApp: 'true'});root.pageItem('safety').refresh();return 1", wait=600)
probe("safety_rows_locked", "var p=root.pageItem('safety');var out={};for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false||r.kind!=='value'||r.app)continue;out[String(p.ev(r.label))]=String(p.ev(r.value))}return out")
probe("safety_unlock", SET + "s.set('lock', {parentalControl: 'false', movieRating: 'off', tvRatingGeneralAge: 'off', inputBlock: '0', applockPerApp: 'false'});return 1")

# ---- Channels
probe("channels_open", "root.openPage('channels');return 1", wait=700)
probe("channels_rows", "var p=root.pageItem('channels');var out=[];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
      "out.push([String(p.ev(r.label)),String(p.ev(r.value)||''),p.ev(r.enabled)===false?'dim':'',String(p.ev(r.detail)||'')])}return out")
probe("channels_interactive", FIND % "channels" + "var a=R('Interactive Service'),b=R('Interactive Service (HDMI)');var was=[p.ev(a.on),p.ev(b.on)];a.act(true);b.act(false);return {was:was,now:[p.ev(a.on),p.ev(b.on)]}", wait=300)
probe("channels_scroll", FIND % "channels" + "var row=R('Channel Scrolling Options');var o=row.options();row.pick(o.map(function(x){return x.value}).indexOf('all_channel'));return {options:o.map(function(x){return x.value+':'+x.label}),value:p.ev(row.value)}", wait=300)

# ---- Picture: White balance, Fine Tune, Video Range and Minimum Brightness
WB = "function S(l){var r=R(l);return {on:p.ev(r.enabled)!==false,v:String(p.ev(r.valueText)!==undefined?p.ev(r.valueText):p.ev(r.value)),detail:String(p.ev(r.detail)||'')}}"
WB_ROWS = ("var p=root.pageItem('whitebalance');var out=[];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
           "out.push(String(p.ev(r.label)))}return out")
probe("wb_open", "root.openPage('whitebalance');return 1", wait=900)
probe("wb_rows_22", WB_ROWS)
probe("wb_level_state", FIND % "whitebalance" + WB + "return {level:S('Signal Level(%)').v,red:S('Red').v,lum:S('Brightness at 100%')}")
probe("wb_set_22", FIND % "whitebalance" + "R('Red').act(7);R('Signal Level(%)').act(-1);var lv=p.ev(R('Signal Level(%)').value);return {level:lv,red:p.ev(R('Red').valueText)}", wait=400)
probe("wb_after_level", FIND % "whitebalance" + WB + "R('Luminance').act(-4);return {lum:S('Brightness at 100%'),lumv:S('Luminance').v}", wait=300)
probe("wb_method_10", SET + "s.set('picture', {whiteBalanceMethod: '10'});root.pageItem('whitebalance').refresh();return 1", wait=500)
probe("wb_rows_10", WB_ROWS)
probe("wb_set_10", FIND % "whitebalance" + "R('Green').act(-9);return {v:p.ev(R('Green').valueText)}", wait=300)
probe("wb_method_2", SET + "s.set('picture', {whiteBalanceMethod: '2', whiteBalancePoint: 'low'});root.pageItem('whitebalance').refresh();return 1", wait=500)
probe("wb_rows_2", WB_ROWS)
probe("wb_set_2", FIND % "whitebalance" + "R('Blue').act(12);return {v:p.ev(R('Blue').valueText)}", wait=300)
probe("wb_point_high", FIND % "whitebalance" + "R('Point').act(1);return 1", wait=300)
probe("wb_set_2_high", FIND % "whitebalance" + "R('Red').act(-2);return {v:p.ev(R('Red').valueText)}", wait=300)
probe("wb_reset", FIND % "whitebalance" + "R('Reset').act();return {asked:root.confirmOpen,danger:!!p.ev(R('Reset').danger)}", wait=250)
probe("wb_reset_no", "root.key('ok');return 1", wait=250)
probe("wb_reset_yes", FIND % "whitebalance" + "R('Reset').act();root.key('right');root.key('ok');return 1", wait=500)
probe("wb_back_22", SET + "s.set('picture', {whiteBalanceMethod: '22'});root.pageItem('whitebalance').refresh();return 1", wait=500)
probe("wb_after_reset", FIND % "whitebalance" + "return {red:p.ev(R('Red').valueText),method:p.ev(R('Method').value)}")
FT = "function S(l){var r=R(l);return {on:p.ev(r.enabled)!==false,v:String(p.ev(r.valueText)!==undefined?p.ev(r.valueText):p.ev(r.value)),detail:String(p.ev(r.detail)||'')}}"
probe("ft_open", "root.openPage('finetune');return 1", wait=900)
probe("ft_locked", FIND % "finetune" + FT + "return {sel:S('Select Color'),sat:S('Adjust Saturation')}")
probe("ft_user", SET + "s.set('picture', {dynamicColor: 'user'});root.pageItem('finetune').refresh();return 1", wait=500)
probe("ft_unlocked", FIND % "finetune" + FT + "return {sel:S('Select Color'),sat:S('Adjust Saturation')}")
probe("ft_set_red", FIND % "finetune" + "R('Adjust Saturation').act(10);R('Adjust Luminance').act(-3);return {sat:p.ev(R('Adjust Saturation').valueText),min:p.ev(R('Adjust Tint').min),max:p.ev(R('Adjust Tint').max)}", wait=300)
probe("ft_choose_cyan", FIND % "finetune" + "var row=R('Select Color');var o=row.options();row.pick(o.map(function(x){return x.value}).indexOf('cyan'));return {options:o.map(function(x){return x.value+':'+x.label}),value:p.ev(row.value)}", wait=400)
probe("ft_set_cyan", FIND % "finetune" + FT + "R('Adjust Tint').act(-8);return {tint:S('Adjust Tint').v,sat:S('Adjust Saturation').v}", wait=300)
probe("ft_reset", FIND % "finetune" + "R('Reset').act();return {asked:root.confirmOpen}", wait=250)
probe("ft_reset_yes", "root.key('right');root.key('ok');return 1", wait=500)
probe("ft_after_reset", FIND % "finetune" + FT + "return {sat:S('Adjust Saturation').v,tint:S('Adjust Tint').v}")
probe("ft_off", SET + "s.set('picture', {dynamicColor: 'off'});return 1")
probe("picture_range", FIND % "picture" + "var vr=R('Video Range');var mb=R('Minimum Brightness');return {vr:vr.options(),mbOn:p.ev(mb.enabled)!==false,mbDetail:p.ev(mb.detail)}")
probe("picture_range_auto", SET + "s.set('picture', {energySaving: 'auto'});root.pageItem('picture').refresh();return 1", wait=500)
probe("picture_min_brightness", FIND % "picture" + "var mb=R('Minimum Brightness');var on=p.ev(mb.enabled)!==false;mb.act(4);return {on:on,min:p.ev(mb.min),max:p.ev(mb.max),text:p.ev(mb.valueText)}", wait=300)
probe("picture_video_range", FIND % "picture" + "var vr=R('Video Range');vr.act(2);return 1", wait=300)
probe("picture_energy_back", SET + "s.set('picture', {energySaving: 'off'});return 1")

# ---- the one-off rows: Always Ready, AI recommendations, extended press time, OLED care, Home Settings, LG's screen saver, Live Plus, Emergency Information
probe("look_tv_rows", "var p=root.pageItem('look');var out=[];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false||r.tab<1)continue;"
      "out.push([r.tab,String(p.ev(r.label)),p.ev(r.enabled)===false?'dim':'',String(p.ev(r.detail)||'')])}return out")
probe("ready_on", FIND % "look" + "R('Always Ready').act(true);return {on:p.ev(R('Always Ready').on),sub:p.ev(R('Always Ready motion sensor').enabled)!==false}", wait=300)
probe("ready_subs", FIND % "look" + "R('Always Ready energy saving').act(1);R('Always Ready notifications').act(false);R('Always Ready motion sensor').act(true);"
      "return {energy:R('Always Ready energy saving').options()}", wait=300)
probe("ready_off_ask", FIND % "look" + "R('Always Ready').act(false);return {asked:root.confirmOpen,on:p.ev(R('Always Ready').on)}", wait=250)
probe("ready_off_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("ready_off_yes", FIND % "look" + "R('Always Ready').act(false);root.key('right');root.key('ok');return 1", wait=400)
probe("ready_off_after", FIND % "look" + "return {on:p.ev(R('Always Ready').on)}")
probe("ai_tips", FIND % "look" + "R('Smart Tips').act(false);R('Smart Tips in Settings').act(false);return {w:!!R('Who.Where.What?')}", wait=250)
probe("remote_slow_locked", FIND % "remote" + "var a=R('Activate time for long press');return {on:p.ev(a.enabled)!==false,detail:p.ev(a.detail),value:p.ev(a.value)}")
probe("remote_slow_on", FIND % "remote" + "R('Extend Pressing Time on Remote').act(true);return 1", wait=300)
probe("remote_slow_pick", FIND % "remote" + "var a=R('Activate time for long press'),b=R('Interval time for repeated actions');var oa=a.options(),ob=b.options();"
      "a.pick(oa.map(function(x){return x.value}).indexOf('1.5'));b.pick(ob.map(function(x){return x.value}).indexOf('0.5'));"
      "return {on:p.ev(a.enabled)!==false,a:oa.map(function(x){return x.value+':'+x.label}),b:ob.map(function(x){return x.value+':'+x.label}),va:p.ev(a.value),vb:p.ev(b.value)}", wait=300)
probe("oled_care", FIND % "system" + "var row=R('Care Picture Settings');var o=row.options();row.pick(o.map(function(x){return x.value}).indexOf('comfortCare'));"
      "R('Care Recommendations').act(false);return {options:o.map(function(x){return x.value+':'+x.label}),value:p.ev(row.value)}", wait=300)
probe("pixel_ask", FIND % "system" + "R('Pixel Cleaning').act(true);return {asked:root.confirmOpen,on:p.ev(R('Pixel Cleaning').on)}", wait=250)
probe("pixel_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("pixel_yes", FIND % "system" + "R('Pixel Cleaning').act(true);root.key('right');root.key('ok');return 1", wait=400)
probe("pixel_off", FIND % "system" + "R('Pixel Cleaning').act(false);return p.ev(R('Pixel Cleaning').on)===false", wait=250)
probe("home_power_on", FIND % "look" + "var row=R('Power On Screen');var now=p.ev(row.index);row.act(1);var a=p.ev(row.index);row.act(0);return {now:now,a:a,b:p.ev(row.index),options:row.options}", wait=300)
probe("home_switches", FIND % "look" + "R('Home Promotion').act(false);R('Content Recommendation').act(false);return {a:p.ev(R('Home Promotion').on),b:p.ev(R('Content Recommendation').on)}", wait=250)
probe("saver_lg", FIND % "look" + "var row=R('Screen Saver Standby Time');var o=row.options();var now=p.ev(row.value);row.pick(o.map(function(x){return x.value}).indexOf('30'));"
      "R('Screen Saver Promotion').act(false);return {now:now,options:o.map(function(x){return x.value+':'+x.label}),value:p.ev(row.value),ad:p.ev(R('Screen Saver Promotion').on)}", wait=300)
probe("live_locked", FIND % "support" + "var a=R('Live Plus'),b=R('Live Promotion');return {on:p.ev(a.on),sub:p.ev(b.enabled)!==false,detail:p.ev(b.detail)}")
probe("live_ask", FIND % "support" + "R('Live Plus').act(true);return {asked:root.confirmOpen,on:p.ev(R('Live Plus').on)}", wait=250)
probe("live_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("live_yes", FIND % "support" + "R('Live Plus').act(true);root.key('right');root.key('ok');return 1", wait=400)
probe("live_after", FIND % "support" + "var b=R('Live Promotion');b.act(true);return {on:p.ev(R('Live Plus').on),sub:p.ev(b.enabled)!==false,promo:p.ev(b.on)}", wait=300)
probe("live_off", FIND % "support" + "R('Live Plus').act(false);return {asked:root.confirmOpen,on:p.ev(R('Live Plus').on)}", wait=300)
probe("emergency_row", FIND % "system" + "var r=R('Emergency Information');return {shown:!!r&&p.ev(r.visible)!==false,atsc3:p.atsc3}", wait=300)
probe("emergency_none", FIND % "system" + "p.atsc3=false;var r=R('Emergency Information');var hidden=p.ev(r.visible)===false;p.atsc3=true;return {hidden:hidden}")
probe("emergency_page", FIND % "emergency" + "R('Enable the function').act(true);var row=R('Audio Language');var o=row.options();row.pick(o.map(function(x){return x.value}).indexOf('spa'));"
      "return {title:p.title,options:o.map(function(x){return x.value+':'+x.label}),value:p.ev(row.value),on:p.ev(R('Enable the function').on)}", wait=300)

# ---- HDMI Signalling Override
probe("sig_link", FIND % "picture" + "var r=R('HDMI Signalling');return {shown:!!r&&p.ev(r.visible)!==false,tab:r&&r.tab}")
probe("sig_open", "root.openPage('signalling');return 1", wait=500)
probe("sig_rows", (ROWS % "signalling") + "return out")
probe("sig_set", FIND % "signalling" + "var a=R('Colorimetry'),b=R('EOTF'),c=R('Mastering Peak'),d=R('Dolby VSVDB');var oa=a.options(),oc=c.options();"
      "function at(o,v){return o.map(function(x){return x.value}).indexOf(v)}a.pick(at(oa,'bt2020'));b.pick(at(b.options(),'hlg'));c.pick(at(oc,'1000'));d.pick(at(d.options(),'v2'));"
      "return {colorimetry:oa.map(function(x){return x.value+':'+x.label}),peak:oc.map(function(x){return x.value+':'+x.label}),values:[a,b,c,d].map(function(r){return p.ev(r.value)})}", wait=300)
probe("sig_reset_ask", FIND % "signalling" + "R('Reset').act();return {asked:root.confirmOpen}", wait=250)
probe("sig_reset_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("sig_reset_yes", FIND % "signalling" + "R('Reset').act();root.key('right');root.key('ok');return 1", wait=900)
probe("sig_after_reset", (ROWS % "signalling") + "return out")

# ---- Delete Connection History
probe("history_ask", FIND % "inputs" + "R('Delete Connection History').act();return {asked:root.confirmOpen,button:p.ev(R('Delete Connection History').button)}", wait=250)
probe("history_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("history_yes", FIND % "inputs" + "R('Delete Connection History').act();root.key('right');root.key('ok');return 1", wait=500)
probe("history_after", FIND % "inputs" + "return {button:p.ev(R('Delete Connection History').button)}")

# ---- App Permissions
PERM = ("var p=root.pageItem('permissions');function RT(t,l){for(var i=0;i<p.rows.length;i++)if(p.rows[i].tab===t&&p.ev(p.rows[i].label)===l)return p.rows[i]};")
probe("perm_open", "root.openPage('permissions');return 1", wait=700)
probe("perm_rows", PERM + "var out=[[],[],[],[]];for(var i=0;i<p.rows.length;i++){var r=p.rows[i];if(p.ev(r.visible)===false)continue;"
      "out[r.tab].push([String(p.ev(r.label)||p.ev(r.text)),p.ev(r.on)===true])}return out")
probe("perm_camera_ask", PERM + "var r=RT(0,'Netflix');r.act(true);return {asked:root.confirmOpen,on:p.ev(r.on)}", wait=250)
probe("perm_camera_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("perm_camera_yes", PERM + "RT(0,'Netflix').act(true);root.key('right');root.key('ok');return 1", wait=400)
probe("perm_camera_off", PERM + "RT(0,'Camera').act(false);return {camera:p.ev(RT(0,'Camera').on),netflix:p.ev(RT(0,'Netflix').on)}", wait=300)
probe("perm_mic_on", PERM + "RT(1,'Camera').act(true);root.key('right');root.key('ok');return 1", wait=1300)
probe("perm_mic_after", PERM + "return {on:p.ev(RT(1,'Camera').on)}")
probe("perm_ble_off", PERM + "RT(2,'Hue Sync').act(false);return p.ev(RT(2,'Hue Sync').on)", wait=300)
probe("perm_file_off", PERM + "RT(3,'Netflix').act(false);return 1", wait=1300)
probe("perm_file_after", PERM + "return {on:p.ev(RT(3,'Netflix').on)}")

# ---- Delete My Personal Information
probe("del_open", "root.openPage('deletedata');return 1", wait=700)
probe("del_state", FIND % "deletedata" + "var a=R('Delete Viewing Information'),b=R('Delete Voice Information');return {a:p.ev(a.enabled)!==false,da:p.ev(a.detail)||'',b:p.ev(b.enabled)!==false,db:p.ev(b.detail)||''}")
probe("del_view_ask", FIND % "deletedata" + "R('Delete Viewing Information').act();return {asked:root.confirmOpen}", wait=250)
probe("del_view_no", "root.key('ok');return {open:root.confirmOpen}", wait=250)
probe("del_view_yes", FIND % "deletedata" + "R('Delete Viewing Information').act();root.key('right');root.key('ok');return 1", wait=900)
probe("del_view_sent", "return L.calls.filter(function(x){return x.method==='/reset'}).map(function(x){return JSON.parse(x.payload)})")
probe("del_voice_yes", FIND % "deletedata" + "R('Delete Voice Information').act();root.key('right');root.key('ok');return 1", wait=500)
probe("del_after", FIND % "deletedata" + "return {a:p.ev(R('Delete Viewing Information').button),b:p.ev(R('Delete Voice Information').button)}")
probe("del_no_voice_terms", SET + "s.set('', {eulaStatus: {generalTermsAllowed: true, voiceAllowed: false}});root.pageItem('deletedata').refresh();return 1", wait=600)
probe("del_voice_locked", FIND % "deletedata" + "var a=R('Delete Viewing Information'),b=R('Delete Voice Information');return {a:p.ev(a.enabled)!==false,b:p.ev(b.enabled)!==false,db:p.ev(b.detail)}")

# ---- USB Device (the way to it is on the Sound page's Output tab; the pick changes the sound out, which the last probe puts back)
probe("usb_link", FIND % "sound" + "var r=R('USB Device');return {shown:!!r&&p.ev(r.visible)!==false,value:r?p.ev(r.value):null}")
probe("usb_open", "root.openPage('usbsound');return 1", wait=600)
probe("usb_rows", (ROWS % "usbsound") + "return out")
probe("usb_pick", FIND % "usbsound" + "R('USB Audio Device').act();return 1", wait=1200)
probe("usb_after", (ROWS % "usbsound") + "return out")
probe("usb_restore", SET + "s.set('sound', {soundOutput: 'tv_speaker'});root.pageItem('sound').refresh();return 1", wait=500)

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
                "truMotionMode", "truMotionJudder", "soundOutput", "avSyncBypassInput", "supportATMOS", "audioBalance", "autoVolume", "tvSetupConfiguration",
                "magicSpaceSound", "tvOnSound", "digitalAudioPriority", "inputAudioFormatHDMI1", "soundOutputDigital", "userEqualizerValues", "soundModeModified", "localeInfo", "automaticUpdate", "lmt", "interactive-service", "interactive-service-hdmi",
                "autoscanChannelGroup", "colorimetry", "eotf", "masteringPeak", "masteringColor", "maxCLL", "maxFALL", "dolbyVSVDBVer", "emergencyInformationAtsc30",
                "emergencyInformationLanguageAtsc30", "bluetoothMode", "cameraResourcePermission", "nearbyDevicePermission"}
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
    def test_the_hdmi_rows_name_their_port_and_the_empty_bluetooth_slots_are_not_devices(self):
        self.assertEqual(self.probe("hdmi_labels")["res"], ["4:4:4 Pass Through (HDMI %d)" % n for n in (1, 2, 3, 4)])

    def test_the_time_and_date_row_reads_the_clock_when_the_page_is_shown(self):
        d = self.probe("system_clock")["res"]
        self.assertLess(d["age"], 2000, "the row showed the clock of an earlier moment: %s" % d)

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
        self.assertEqual(self.probe("bt_audio_rows")["res"], [["Speaker Connection", "1 device"], ["Playback Option", "Simultaneous Sound Out"], ["Registered Devices (1)", ""],
                                                              ["Living Room Headphones", ""], ["Connectable Devices (0)", ""], ["Search", ""]])
        found = self.probe("bt_audio_found")
        self.assertEqual(found["res"][-3:], [["Connectable Devices (1)", ""], ["Kitchen Speaker", ""], ["Searching", ""]])
        self.assertEqual([(c["method"], c["payload"]) for c in found["calls"] if c["method"] == "/connect"], [("/connect", {"address": "AA:BB:CC:00:00:02"})])
        self.assertIn(["Kitchen Speaker", "Connected"], self.probe("bt_audio_paired")["res"])                    # read again after the connection
        self.assertIn(["Connectable Devices (0)", ""], self.probe("bt_audio_paired")["res"])

    def test_the_playback_option_waits_for_a_connected_speaker_and_the_volume_is_the_services_own(self):
        self.assertEqual(self.probe("bt_playback_locked")["res"], {"enabled": False, "detail": "No speaker connected", "value": "Simultaneous Sound Out", "volume": False})
        d = self.probe("bt_playback")
        self.assertEqual(d["res"], {"enabled": True, "opts": ["surroundMode:Surround Effect", "speakerMode:Simultaneous Sound Out"], "mode": "Surround Effect", "before": "5", "after": "7"})
        self.assertEqual([c["payload"]["settings"] for c in d["calls"] if c["method"] == "/setSystemSettings"], [{"bluetoothMode": "surroundMode"}])
        self.assertEqual([(c["method"], c["payload"]) for c in d["calls"] if c["method"] == "/setVolumeBalancing"], [("/setVolumeBalancing", {"tvspeaker_bluetooth": 7})])

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
            ["OLED pixel brightness", "AI Picture Pro", "Aspect ratio", "Energy saving", "Minimum Brightness", "Just Scan"],
            ["Contrast", "Brightness", "Eye comfort", "Auto Dynamic Contrast", "Peak Brightness", "Gamma", "Video Range", "Motion Eye Care"],
            ["Colour", "Tint", "Color Temperature", "Color Gamut", "Color Adjustment", "Fine Tune", "White Balance"],
            ["Sharpness", "Super Resolution", "Noise Reduction", "MPEG Noise Reduction", "Smooth Gradation", "Real Cinema"],
            ["TruMotion", "OLED Motion Pro"],
            ["Filmmaker Auto Start", "AI genre selection", "AI brightness", "HDMI Signalling", "Apply to All Inputs", "Reset"]])

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

    # ---- Sound
    def written(self, name):
        """The settings the probe wrote, each as the service got them."""
        return [c["payload"]["settings"] for c in self.probe(name)["calls"] if c["method"] == "/setSystemSettings"]

    OUTPUTS = ["tv_speaker:TV Speaker", "external_arc:HDMI(ARC) Device", "external_optical:Optical Out Device", "lineout:Audio Out Device", "headphone:Wired Headphones",
               "tv_speaker_external_arc:HDMI(ARC) Device + TV Speaker", "tv_external_speaker:Optical Out Device + TV Speaker", "tv_speaker_headphone:Wired Headphones + TV Speaker"]

    def test_the_sound_tabs_list_what_LG_lists_for_the_sound_out_in_force(self):
        self.assertEqual(self.probe("sound_tabs")["res"], [
            ["Sound out", "Receiver", "Receiver volume", "Sound mode", "Match screen and sound", "Delay bypass", "Digital sound output", "TV speakers too", "Bluetooth Speakers", "USB Device"], [],
            ["Dolby Atmos", "Balance", "Equalizer", "Automatic volume", "Installation Type", "AI Acoustic Tuning Options", "TV Power Sound Volume", "Apply to All Inputs", "Reset"],
            ["Select HDMI Input Audio Format", "LG Sound Sync", "TV Sound Mode Share", "DTV Audio Setting"]])         # the TV hides eARC Support
        self.assertEqual(self.probe("sound_tabs_tv")["res"][0][:2], ["Sound out", "Receiver volume"])                # no receiver, no row for it

    def test_sound_out_is_LGs_chooser_on_the_tvs_list_in_LGs_names(self):
        d = self.probe("sound_out_options")["res"]
        self.assertEqual((d["now"], d["value"], d["options"]), ("external_arc", "HDMI(ARC) Device", self.OUTPUTS))      # Bluetooth, USB and the wireless ones are not offered here
        d = self.probe("sound_out_chooser")["res"]
        self.assertEqual((d["title"], d["rows"], d["stop"], d["back"]), ("Sound out", [o.split(":", 1)[1] for o in self.OUTPUTS], 1, "Sound"))   # the focus starts on the one in force
        self.assertEqual(self.probe("sound_out_open")["res"], {"at": "choice"})
        done = self.probe("sound_out_picked")
        self.assertEqual(done["res"], {"at": "sound", "out": "tv_speaker"})
        self.assertEqual(self.written("sound_out_picked"), [{"soundOutput": "tv_speaker"}])
        self.assertEqual([c["error"] for c in done["calls"]], [""] * len(done["calls"]))

    def test_the_tvs_own_sound_settings_wait_for_the_sound_out_LG_waits_for(self):
        r = self.probe("sound_state_receiver")["res"]
        self.assertEqual({k: v["on"] for k, v in r.items()}, {"balance": False, "eq": False, "install": False, "acoustic": False, "speakersToo": True, "atmos": True,
                                                              "lgsync": False, "share": True, "reset": True})
        self.assertEqual((r["balance"]["detail"], r["lgsync"]["detail"]), ("Needs the TV speakers", "Needs the optical out"))
        t = self.probe("sound_state_tv")["res"]
        self.assertEqual({k: v["on"] for k, v in t.items()}, {"balance": True, "eq": True, "install": True, "acoustic": True, "speakersToo": False, "atmos": True,
                                                              "lgsync": False, "share": False, "reset": True})
        self.assertEqual((t["speakersToo"]["detail"], t["share"]["detail"]), ("Needs the receiver", "Needs HDMI(ARC)"))
        d = self.probe("sound_lgsync")["res"]
        self.assertEqual((d["on"], d["now"], d["balance"]), (True, True, "Needs the TV speakers"))                      # the optical out: Sound Sync answers, the TV's balance does not
        self.assertEqual(self.written("sound_lgsync"), [{"lgSoundSync": "on"}])
        d = self.probe("sound_share")["res"]
        self.assertEqual((d["on"], d["now"]), (True, True))
        self.assertEqual(self.written("sound_share"), [{"soundAlive": "on"}])

    def test_the_equalizer_sends_all_five_levels_with_the_mode_marked_modified(self):
        self.assertEqual(self.probe("eq_rows")["res"], [[b, "0", True] for b in ("100 Hz", "300 Hz", "1 kHz", "3 kHz", "10 kHz")])
        d = self.probe("eq_band")["res"]
        self.assertEqual((d["text"], d["bass"], d["levels"], d["min"], d["max"]), ("+5", "-3", [-3, 0, 5, 0, 0], -10, 10))
        self.assertEqual(self.written("eq_band"), [{"userEqualizerValues": [0, 0, 5, 0, 0], "soundModeModified": "true"}, {"userEqualizerValues": [-3, 0, 5, 0, 0], "soundModeModified": "true"}])
        self.assertEqual([c["error"] for c in self.probe("eq_band")["calls"]], ["", ""])

    def test_the_audio_format_is_chosen_for_each_hdmi_input_the_tv_shows(self):
        self.assertEqual(self.probe("hdmi_audio_rows")["res"], [["HDMI 1", "Bitstream/PCM", "Bitstream"], ["HDMI 2", "Bitstream/PCM", "Bitstream"],
                                                                ["HDMI 3", "Bitstream/PCM", "PCM"], ["HDMI 4", "Bitstream/PCM", "PCM"]])       # DisplayPort and USB-C are hidden
        self.assertEqual(self.written("hdmi_audio_pick"), [{"inputAudioFormatHDMI3": "bitstream"}, {"inputAudioFormatHDMI1": "pcm"}])

    def test_sound_apply_to_all_inputs_asks_and_reset_asks_in_the_danger_style_and_sends_the_keys_LG_sends(self):
        self.assertTrue(self.probe("sound_apply")["res"]["asked"])
        self.assertEqual(self.written("sound_apply_yes"), [{"applyToAllInputSSM": "sound"}])
        d = self.probe("sound_reset")["res"]
        self.assertEqual((d["asked"], d["danger"], d["before"]), (True, True, 4))
        self.assertEqual([c for c in self.probe("sound_reset_no")["calls"] if c["method"] == "/resetSystemSettings"], [])
        done = [c for c in self.probe("sound_reset_go")["calls"] if c["method"] == "/resetSystemSettings"]
        self.assertEqual([c["payload"] for c in done], [{"category": "sound", "keys": ["audioBalance", "userEqualizerValues", "soundModeModified"]}])
        self.assertEqual([c["error"] for c in done], [""])
        self.assertEqual(self.probe("sound_reset_after")["res"], {"balance": 0})                                       # the TV put the balance back
        d = self.probe("sound_reset_movie")["res"]
        self.assertEqual((d["on"], d["detail"], d["eq"]), (False, "Standard mode only", "Standard mode and TV speakers"))   # LG's Reset is Standard's, without the sound wizard

    # ---- Language and location
    def test_the_language_page_has_the_tvs_lists_and_the_rows_LG_has(self):
        self.assertEqual(self.probe("lang_tabs")["res"], [["Menu Language", "Audio Language"], ["English", "Espa\u00f1ol", "Fran\u00e7ais"], ["Broadcast Country", "LG Services Country", "Zip Code"]])
        d = self.probe("lang_menu_options")["res"]
        self.assertEqual((d["now"], d["value"]), ("en-US", "English - United States"))
        self.assertEqual(d["options"], ["en-US:English - United States", "en-GB:English - United Kingdom", "es-US:Espa\u00f1ol - Estados Unidos", "fr-FR:Fran\u00e7ais - France"])

    def test_the_menu_language_asks_first_and_writes_the_locales_and_keyboards_LG_writes(self):
        d = self.probe("lang_menu_ask")["res"]
        self.assertEqual((d["asked"], d["value"]), (True, "English - United States"))                               # nothing changes before the answer
        self.assertEqual([c for c in self.probe("lang_menu_no")["calls"] if c["method"] == "/setSystemSettings"], [])      # Cancel, the safe answer, changes nothing
        sent = self.written("lang_menu_yes")
        self.assertEqual(len(sent), 1)
        self.assertEqual(sent[0]["localeInfo"], {"keyboards": ["en", "es"], "locales": {"UI": "es-US", "FMT": "es-US", "TV": "es-US", "STT": "en-US"}, "clock": "locale", "timezone": ""})
        self.assertEqual([c for c in self.probe("lang_menu_yes")["calls"] if c["method"] == "/setSystemSettings"][0].get("payload", {}).get("category"), None)    # no category, as LG's own
        self.assertEqual(self.probe("lang_menu_after")["res"]["value"], "Espa\u00f1ol - Estados Unidos")

    def test_keyboards_of_english_and_the_menu_language_stay_and_the_others_are_added_and_removed(self):
        d = self.probe("lang_keyboards")["res"]
        self.assertEqual((d["en"], d["es"]), ({"on": True, "enabled": False, "detail": "In use"}, {"on": True, "enabled": False, "detail": "In use"}))
        self.assertEqual(d["fr"], {"on": False, "enabled": True, "detail": ""})
        self.assertEqual(self.probe("lang_keyboard_fr")["res"]["on"], True)
        self.assertEqual(self.written("lang_keyboard_fr")[0]["localeInfo"]["keyboards"], ["en", "es", "fr"])

    def test_the_audio_language_is_written_at_once_without_a_question(self):
        d = self.probe("lang_audio")["res"]
        self.assertEqual((d["value"], d["asked"]), ("French", False))
        self.assertEqual(d["options"], ["en-US:English", "es-US:Spanish", "fr-FR:French"])
        self.assertEqual(self.written("lang_audio")[0]["localeInfo"]["locales"]["AUD"], "fr-FR")

    def test_location_shows_the_country_and_asks_before_the_services_country_changes(self):
        self.assertEqual(self.probe("lang_location")["res"], {"country": "United States", "svc": True, "zip": "94105"})
        d = self.probe("lang_svc_ask")["res"]
        self.assertEqual((d["asked"], d["on"]), (True, True))
        self.assertEqual(self.written("lang_svc_off"), [{"autoSmartServiceCountry": "off"}])
        done = self.probe("lang_svc_on")
        self.assertEqual(self.written("lang_svc_on"), [{"autoSmartServiceCountry": "on"}])
        self.assertEqual([(c["method"], c["payload"]) for c in done["calls"] if c["method"] == "/setCountrySettingByAuto"], [("/setCountrySettingByAuto", {"uniqueIndex": "2013"})])
        self.assertEqual(self.probe("lang_svc_after")["res"], {"on": True})

    def test_a_zip_code_is_five_digits_and_nothing_else_is_written(self):
        self.assertEqual(self.written("lang_zip_bad"), [])
        self.assertEqual(self.probe("lang_zip_bad_state")["res"], {"detail": "Needs 5 digits", "value": "94105"})
        self.assertEqual(self.written("lang_zip"), [{"zipcode": "90210"}])
        self.assertEqual(self.probe("lang_zip_after")["res"], {"detail": "", "value": "90210"})

    # ---- Support
    def test_support_has_the_rows_of_LGs_tv_information_update_privacy_tools_and_reset(self):
        self.assertEqual(self.probe("support_tabs")["res"], [
            ["Device Name", "Model", "Model Code", "Serial Number", "webOS TV Version", "Total Power On Time", "Available Internal Storage", "MAC Address (Wired)",
             "MAC Address (Wireless)", "ESN", "Widevine", "Dolby Vision"],                                   # VSN is hidden by the TV
            ["Software Version", "Update status", "Auto Update", "Check for Updates"],
            ["Do Not Sell My Personal Information", "Limit AD Tracking", "AD ID", "Reset AD ID", "Live Plus", "Live Promotion", "App Permissions", "Delete My Personal Information"],
            ["Screen Self-diagnosis", "Sound Self-diagnosis", "Memory Optimizer"],
            ["Reset All", "Reset All Except Network Settings"]])

    def test_the_information_is_what_the_tvs_services_answered_in_LGs_forms(self):
        v = self.probe("support_info")["res"]
        self.assertEqual((v["Model"], v["Model Code"], v["Serial Number"]), ("OLED65C4PUA", "OLED65C4PUA.AUSQLJR", "FAKE-SERIAL-0001"))
        self.assertEqual((v["webOS TV Version"], v["Software Version"]), ("webOS24 / 9.1.2", "webOS24 / 9.1.2"))            # LG: webOS (major + 15) / version
        self.assertEqual((v["Available Internal Storage"], v["MAC Address (Wired)"], v["MAC Address (Wireless)"]), ("1.22/2.83GB", "AA:BB:CC:11:22:33", "AA:BB:CC:44:55:66"))
        self.assertEqual((v["ESN"], v["Widevine"], v["Dolby Vision"], v["Total Power On Time"]), ("FAKE-ESN-0001", "FAKE-WV-0001", "4.0.1", "1,705 h"))
        self.assertEqual((v["Device Name"], v["Update status"]), ("Den TV", "No update running"))

    def test_the_update_is_shown_not_run_and_auto_update_needs_LGs_terms_to_turn_on(self):
        self.assertEqual(self.probe("support_auto_state")["res"], {"on": False, "enabled": True, "detail": ""})
        self.assertEqual(self.written("support_auto_on"), [{"automaticUpdate": "on"}])
        d = self.probe("support_auto_locked")["res"]
        self.assertEqual(d, {"on": False, "enabled": False, "detail": "Needs LG's terms"})
        for name in ("support_open", "support_auto_state", "support_auto_on", "support_no_terms"):
            self.assertEqual([c["method"] for c in self.probe(name)["calls"] if "update" in c["method"].lower() and c["method"] != "/getStatus"], [])      # no check, no start

    def test_do_not_sell_asks_then_tells_the_ad_service_and_the_setting_follows(self):
        d = self.probe("support_dns_ask")["res"]
        self.assertEqual((d["asked"], d["on"]), (True, False))
        self.assertEqual([c for c in self.probe("support_dns_no")["calls"] if c["method"] == "/updateDNS"], [])
        done = self.probe("support_dns_yes")
        self.assertEqual([c["payload"] for c in done["calls"] if c["method"] == "/updateDNS"], [{"status": "on"}])
        self.assertEqual(self.probe("support_dns_after")["res"], {"on": True})
        self.assertEqual(self.written("support_lmt"), [{"lmt": "on"}])

    def test_the_ad_id_is_shown_and_reset_after_a_question(self):
        self.assertEqual(self.probe("support_ad_id")["res"], {"id": "00000000-0000-4000-8000-000000000001"})
        self.assertTrue(self.probe("support_ad_reset")["res"]["asked"])
        self.assertEqual([c["method"] for c in self.probe("support_ad_reset_yes")["calls"] if c["method"] == "/resetIFA"], ["/resetIFA"])
        self.assertEqual(self.probe("support_ad_after")["res"], {"id": "00000000-0000-4000-8000-000000000002"})

    def test_the_tools_ask_first_and_the_self_diagnosis_opens_LGs_customer_support_app(self):
        self.assertEqual(self.probe("support_tools")["res"], [["Screen Self-diagnosis", "com.webos.app.customersupport", "pictureTest"],
                                                              ["Sound Self-diagnosis", "com.webos.app.customersupport", "soundTest"], ["Memory Optimizer", "", ""]])
        d = self.probe("support_memory")
        self.assertTrue(d["res"]["asked"])
        self.assertEqual([c["method"] for c in d["calls"] if c["method"] == "/removeAllRecents"], ["/removeAllRecents"])

    def test_the_reset_asks_then_the_pin_prompt_then_asks_again_and_nothing_is_sent_before_the_last_yes(self):
        self.assertTrue(self.probe("support_reset_first")["res"]["asked"])
        self.assertEqual([c["method"] for c in self.probe("support_reset_no")["calls"] if c["method"] in ("/createPincodePrompt", "/doReset")], [])        # Cancel stops it
        self.assertEqual([c for c in self.probe("support_reset_continue")["calls"] if c["method"] == "/doReset"], [])
        d = self.probe("support_reset_pin")["res"]
        self.assertEqual((d["pins"], d["resets"], d["second"]), ([{"promptType": "parental"}], 0, True))     # LG's prompt asked once, answered: the second question is up, nothing sent
        self.assertEqual([c for c in self.probe("support_reset_second_no")["calls"] if c["method"] == "/doReset"], [])        # its safe answer resets nothing
        self.assertEqual([c for c in self.probe("support_reset_again")["calls"] if c["method"] == "/doReset"], [])
        done = self.probe("support_reset_second_yes")
        self.assertTrue(done["res"]["second"])
        self.assertEqual([(c["payload"], c["error"]) for c in done["calls"] if c["method"] == "/doReset"], [({"resetType": "userKeepNetwork"}, "")])

    # ---- Safety
    def test_safety_shows_the_locks_and_has_no_row_that_writes_one(self):
        rows = self.probe("safety_rows")["res"]
        self.assertEqual([r[:2] for r in rows], [["Lock System", "Off"], ["TV Program Locks", "Off"], ["Movie Lock", "Off"], ["Application Locks", "Off"], ["Input Locks", "Off"],
                                                 ["Change locks", ""]])
        self.assertEqual([r[0] for r in rows if r[2]], [])                                                                      # only LG's own screen changes a lock
        self.assertEqual(self.probe("safety_rows_locked")["res"], {"Lock System": "On", "TV Program Locks": "On", "Movie Lock": "PG-13", "Application Locks": "On", "Input Locks": "On"})
        for name in ("safety_open", "safety_rows", "safety_rows_locked"):
            self.assertEqual([c for c in self.probe(name)["calls"] if c["method"] == "/setSystemSettings"], [])

    # ---- Channels
    def test_channels_lists_the_settings_the_tv_shows_and_writes_LGs_values(self):
        rows = self.probe("channels_rows")["res"]
        self.assertEqual(rows, [["Channel Scrolling Options", "Primary Channels", "", ""], ["System Color", "Multi", "dim", "Live TV only"], ["Interactive Service", "", "", ""],
                                ["Interactive Service (HDMI)", "", "", ""], ["Channel Tuning & Settings", "", "", ""]])
        d = self.probe("channels_interactive")
        self.assertEqual(d["res"], {"was": [False, True], "now": [True, False]})
        self.assertEqual(self.written("channels_interactive"), [{"interactive-service": "onByUser"}, {"interactive-service-hdmi": "offByUser"}])      # LG's own switch values
        s = self.probe("channels_scroll")["res"]
        self.assertEqual((s["options"], s["value"]), (["primary_channel:Primary Channels", "all_channel:All Channels"], "All Channels"))

    # ---- Picture: White balance, Fine Tune, Video Range, Minimum Brightness
    def test_white_balance_offers_the_rows_of_the_method_in_force_and_writes_LGs_keys(self):
        full = ["Method", "Signal Level(%)", "Luminance", "Red", "Green", "Blue", "Brightness at 100%", "Reset"]
        self.assertEqual(self.probe("wb_rows_22")["res"], full)
        self.assertEqual(self.probe("wb_rows_10")["res"], full)
        self.assertEqual(self.probe("wb_rows_2")["res"], ["Method", "Point", "Red", "Green", "Blue", "Reset"])
        d = self.probe("wb_level_state")["res"]
        self.assertEqual((d["level"], d["red"], d["lum"]), ("100%", "0", {"on": True, "v": "100", "detail": ""}))
        d = self.probe("wb_set_22")
        self.assertEqual(d["res"], {"level": "95%", "red": "0"})
        sent = self.written("wb_set_22")
        self.assertEqual(sent[0], {"whiteBalanceRed": [0] * 21 + [7], "pictureSettingModified": {"filmMaker": True}})      # the whole list, the level's number changed, the mode marked
        self.assertEqual(sent[1], {"whiteBalanceIre": "95"})
        self.assertEqual([c["error"] for c in d["calls"]], [""] * len(d["calls"]))
        d = self.probe("wb_after_level")["res"]
        self.assertEqual((d["lum"], d["lumv"]), ({"on": False, "v": "100", "detail": "Only at 100%"}, "-4"))
        self.assertEqual(self.written("wb_after_level"), [{"adjustingLuminance": [0] * 20 + [-4, 0]}])
        self.assertEqual(self.written("wb_set_10"), [{"whiteBalanceGreen10pt": [0] * 9 + [-9]}])
        self.assertEqual(self.probe("wb_set_10")["res"], {"v": "-9"})
        self.assertEqual(self.written("wb_set_2"), [{"whiteBalanceBlueOffset": "12"}])                                      # two points: low is the offset, high the gain
        self.assertEqual(self.written("wb_point_high"), [{"whiteBalancePoint": "high"}])
        self.assertEqual(self.written("wb_set_2_high"), [{"whiteBalanceRedGain": "-2"}])

    def test_the_white_balance_reset_asks_and_sends_the_keys_the_tv_lists(self):
        d = self.probe("wb_reset")["res"]
        self.assertEqual((d["asked"], d["danger"]), (True, True))
        self.assertEqual([c for c in self.probe("wb_reset_no")["calls"] if c["method"] == "/resetSystemSettings"], [])
        done = [c for c in self.probe("wb_reset_yes")["calls"] if c["method"] == "/resetSystemSettings"]
        self.assertEqual(len(done), 1)
        self.assertEqual((done[0]["payload"]["category"], "whiteBalanceRedGain" in done[0]["payload"]["keys"], done[0]["error"]), ("picture", True, ""))
        self.assertEqual(self.probe("wb_after_reset")["res"], {"red": "0", "method": "22 Points Signal Level(%)"})

    def test_fine_tune_answers_only_in_user_mode_and_writes_the_chosen_colours_keys(self):
        d = self.probe("ft_locked")["res"]
        self.assertEqual((d["sel"]["on"], d["sel"]["detail"], d["sat"]["on"]), (False, "Set Color Adjustment to User", False))
        d = self.probe("ft_unlocked")["res"]
        self.assertEqual((d["sel"]["on"], d["sat"]["on"], d["sat"]["detail"]), (True, True, ""))
        d = self.probe("ft_set_red")["res"]
        self.assertEqual((d["sat"], d["min"], d["max"]), ("10", -30, 30))
        self.assertEqual(self.written("ft_set_red"), [{"colorManagementSaturationRed": "10"}, {"colorManagementLuminanceRed": "-3"}])
        d = self.probe("ft_choose_cyan")["res"]
        self.assertEqual((d["options"], d["value"]), (["red:Red", "green:Green", "blue:Blue", "cyan:Cyan", "magenta:Magenta", "yellow:Yellow"], "Cyan"))
        self.assertEqual(self.written("ft_choose_cyan"), [{"colorManagementColorSystem": "cyan"}])
        self.assertEqual(self.probe("ft_set_cyan")["res"], {"tint": "-8", "sat": "0"})                                          # the rows follow the colour: Cyan's own keys
        self.assertEqual(self.written("ft_set_cyan"), [{"colorManagementHueCyan": "-8"}])
        self.assertTrue(self.probe("ft_reset")["res"]["asked"])
        done = [c for c in self.probe("ft_reset_yes")["calls"] if c["method"] == "/resetSystemSettings"]
        self.assertEqual((len(done), len(done[0]["payload"]["keys"]), done[0]["error"]), (1, 18, ""))                           # LG's reset names the 18 keys the TV lists
        self.assertEqual(self.probe("ft_after_reset")["res"], {"sat": "0", "tint": "0"})

    def test_video_range_is_LGs_three_and_minimum_brightness_waits_for_auto_energy_saving(self):
        d = self.probe("picture_range")["res"]
        self.assertEqual((d["vr"], d["mbOn"], d["mbDetail"]), (["Auto", "Limited", "Full"], False, "Energy saving is not Auto"))
        d = self.probe("picture_min_brightness")["res"]
        self.assertEqual(d, {"on": True, "min": 0, "max": 10, "text": "4"})
        self.assertEqual(self.written("picture_min_brightness"), [{"energySavingAutoMin": "4"}])
        self.assertEqual(self.written("picture_video_range"), [{"blackLevel": "high"}])

    # ---- the one-off rows
    def test_always_ready_is_LGs_switch_with_its_three_options_and_asks_before_it_goes_off(self):
        rows = self.probe("look_tv_rows")["res"]
        self.assertEqual([r[1:] for r in rows if r[0] == 3][-4:], [["Always Ready", "", ""], ["Always Ready energy saving", "dim", "Turn Always Ready on"],
                                                                    ["Always Ready notifications", "dim", "Turn Always Ready on"], ["Always Ready motion sensor", "dim", "Turn Always Ready on"]])
        self.assertEqual(self.probe("ready_on")["res"], {"on": True, "sub": True})
        self.assertEqual(self.written("ready_on"), [{"lifeOnScreenMode": "allEnabled"}])                                      # LG's own value for on
        self.assertEqual(self.written("ready_subs"), [{"lifeOnScreenEnergySaving": "off"}, {"lifeOnScreenNotification": "false"}, {"lifeOnScreenUsingMotionSensor": "true"}])
        d = self.probe("ready_off_ask")["res"]
        self.assertEqual((d["asked"], d["on"]), (True, True))
        self.assertEqual([c for c in self.probe("ready_off_no")["calls"] if c["method"] == "/setSystemSettings"], [])
        self.assertEqual(self.written("ready_off_yes"), [{"lifeOnScreenMode": "off"}])
        self.assertEqual(self.probe("ready_off_after")["res"], {"on": False})

    def test_ai_recommendations_are_the_switches_the_tv_shows_and_the_one_it_hides_is_left_out(self):
        rows = [r[1] for r in self.probe("look_tv_rows")["res"] if r[0] == 1]
        self.assertIn("Smart Tips", rows)
        self.assertIn("Smart Tips in Settings", rows)
        self.assertNotIn("Who.Where.What?", rows)                                                                               # LG hides it on this set
        self.assertEqual(self.written("ai_tips"), [{"aiNudge": "off"}, {"aiSettingsNudge": "off"}])

    def test_extended_press_time_waits_for_its_switch_and_offers_the_tvs_own_times(self):
        self.assertEqual(self.probe("remote_slow_locked")["res"], {"on": False, "detail": "Turn Extend Pressing Time on", "value": "Basic"})
        self.assertEqual(self.written("remote_slow_on"), [{"remoteSlowInput": "on"}])
        d = self.probe("remote_slow_pick")["res"]
        self.assertTrue(d["on"])
        self.assertEqual(d["a"], ["basic:Basic", "1.5:1.5 s", "2:2 s", "2.5:2.5 s", "3:3 s", "3.5:3.5 s", "4:4 s", "4.5:4.5 s", "5:5 s"])
        self.assertEqual(d["b"][:3], ["basic:Basic", "0.5:0.5 s", "1:1 s"])
        self.assertEqual((d["va"], d["vb"]), ("1.5 s", "0.5 s"))
        self.assertEqual(self.written("remote_slow_pick"), [{"longPressInterval": "1.5"}, {"repeatInterval": "0.5"}])

    def test_oled_care_has_LGs_three_care_modes_pixel_cleaning_asks_to_turn_on_and_recommendations_switch(self):
        d = self.probe("oled_care")["res"]
        self.assertEqual(d["options"], ["off:Off", "comfortCare:Comfort Mode", "cinemaCare:Theater Mode", "kidsCare:Kids Care Mode"])
        self.assertEqual(d["value"], "Comfort Mode")
        self.assertEqual(self.written("oled_care"), [{"oledCareMode": "comfortCare"}, {"oledCareRecommendation": "off"}])
        d = self.probe("pixel_ask")["res"]
        self.assertEqual((d["asked"], d["on"]), (True, False))
        self.assertEqual([c for c in self.probe("pixel_no")["calls"] if c["method"] == "/setSystemSettings"], [])
        self.assertEqual(self.written("pixel_yes"), [{"scheduledPixelCleaning": "on"}])
        self.assertEqual(self.written("pixel_off"), [{"scheduledPixelCleaning": "off"}])                                      # off needs no question

    def test_home_settings_are_LGs_power_on_screen_and_its_two_switches(self):
        rows = [r for r in self.probe("look_tv_rows")["res"] if r[0] == 1]
        self.assertEqual([r[1] for r in rows][-5:], ["Power On Screen", "Home Promotion", "Content Recommendation", "Smart Tips", "Smart Tips in Settings"])
        self.assertEqual([r[2:] for r in rows][-5:], [["", ""]] * 5)
        d = self.probe("home_power_on")["res"]
        self.assertEqual((d["now"], d["a"], d["b"], d["options"]), (0, 1, 0, ["Home", "Recent Input"]))               # the TV starts on Home
        self.assertEqual(self.written("home_power_on"), [{"homeAutoLaunch": "off", "selectedInputApp": "off", "checkHomeAutoLaunchValue": "user"},
                                                           {"homeAutoLaunch": "on", "selectedInputApp": "off", "checkHomeAutoLaunchValue": "user"}])      # as LG's row writes it
        self.assertEqual(self.probe("home_switches")["res"], {"a": False, "b": False})
        self.assertEqual(self.written("home_switches"), [{"homePromotion": "off"}, {"contentRecommendation": "off"}])
        for name in ("home_power_on", "home_switches"):
            self.assertEqual([c for c in self.probe(name)["calls"] if c["error"]], [])

    def test_the_screen_savers_standby_time_is_LGs_four_choices_and_its_promotion_a_switch(self):
        rows = [r[1] for r in self.probe("look_tv_rows")["res"] if r[0] == 2]
        self.assertEqual(rows[-2:], ["Screen Saver Standby Time", "Screen Saver Promotion"])
        d = self.probe("saver_lg")["res"]
        self.assertEqual((d["now"], d["value"], d["ad"]), ("10 minutes", "30 minutes", False))
        self.assertEqual(d["options"], ["3:3 minutes", "10:10 minutes", "20:20 minutes", "30:30 minutes"])
        self.assertEqual(self.written("saver_lg"), [{"screenSaverTimer": "30", "checkChangedScreenSaverTimerByUser": "user"}, {"screenSaverAd": "off"}])
        self.assertEqual([c for c in self.probe("saver_lg")["calls"] if c["error"]], [])

    def test_live_plus_asks_to_turn_on_and_live_promotion_waits_for_it(self):
        self.assertEqual(self.probe("live_locked")["res"], {"on": False, "sub": False, "detail": "Turn Live Plus on"})
        d = self.probe("live_ask")["res"]
        self.assertEqual((d["asked"], d["on"]), (True, False))
        self.assertEqual([c for c in self.probe("live_no")["calls"] if c["method"] == "/setSystemSettings"], [])
        self.assertEqual(self.written("live_yes"), [{"livePlus": "on"}])
        self.assertEqual(self.probe("live_after")["res"], {"on": True, "sub": True, "promo": True})
        self.assertEqual(self.written("live_after"), [{"livePromotion": "on"}])
        d = self.probe("live_off")
        self.assertEqual(d["res"], {"asked": False, "on": False})                                                            # off needs no question
        self.assertEqual([c["payload"]["settings"] for c in d["calls"] if c["method"] == "/setSystemSettings"], [{"livePlus": "off"}])

    def test_emergency_information_waits_for_an_atsc3_tuner_and_is_LGs_switch_and_language(self):
        self.assertEqual(self.probe("emergency_row")["res"], {"shown": True, "atsc3": True})
        self.assertEqual(self.probe("emergency_none")["res"], {"hidden": True})                                              # no tuner, no row
        d = self.probe("emergency_page")
        self.assertEqual(d["res"], {"title": "Emergency Information", "options": ["eng:English", "spa:Spanish", "fre:French"], "value": "Spanish", "on": True})
        self.assertEqual([c["payload"]["settings"] for c in d["calls"] if c["method"] == "/setSystemSettings"],
                         [{"emergencyInformationAtsc30": "on"}, {"emergencyInformationLanguageAtsc30": "spa"}])

    SIGNALS = ["Colorimetry", "EOTF", "Mastering Peak", "Mastering Color", "MaxCLL", "MaxFALL", "Dolby VSVDB", "Reset"]

    def test_hdmi_signalling_is_LGs_seven_overrides_on_the_tvs_lists_and_a_reset_that_asks(self):
        self.assertEqual(self.probe("sig_link")["res"], {"shown": True, "tab": 5})
        rows = self.probe("sig_rows")["res"]
        self.assertEqual([r[0] for r in rows], self.SIGNALS)
        self.assertEqual([r[1] for r in rows][:7], ["AUTO", "AUTO", "AUTO", "AUTO", "AUTO", "AUTO", "V1"])                    # LG's names, the TV starts on its own
        d = self.probe("sig_set")["res"]
        self.assertEqual(d["colorimetry"], ["auto:AUTO", "bt709:BT709", "bt601:BT601", "bt2020:BT2020", "p3D65:P3 D65"])
        self.assertEqual(d["peak"], ["auto:AUTO", "0:0", "400:400", "540:540", "700:700", "1000:1000", "2000:2000", "3000:3000", "4000:4000", "10000:10000"])
        self.assertEqual(d["values"], ["BT2020", "HLG", "1000", "V2"])
        self.assertEqual(self.written("sig_set"), [{"colorimetry": "bt2020"}, {"eotf": "hlg"}, {"masteringPeak": "1000"}, {"dolbyVSVDBVer": "v2"}])
        self.assertEqual([c for c in self.probe("sig_set")["calls"] if c["error"]], [])
        self.assertTrue(self.probe("sig_reset_ask")["res"]["asked"])
        self.assertEqual([c for c in self.probe("sig_reset_no")["calls"] if c["method"] == "/resetSystemSettings"], [])
        done = [c for c in self.probe("sig_reset_yes")["calls"] if c["method"] == "/resetSystemSettings"]
        self.assertEqual([c["payload"] for c in done], [{"category": "other", "keys": ["colorimetry", "eotf", "masteringPeak", "masteringColor", "maxCLL", "maxFALL", "dolbyVSVDBVer"]}])
        self.assertEqual([r[1] for r in self.probe("sig_after_reset")["res"]][:7], ["AUTO", "AUTO", "AUTO", "AUTO", "AUTO", "AUTO", "V1"])

    def test_delete_connection_history_asks_then_tells_the_five_services_LGs_row_tells(self):
        d = self.probe("history_ask")["res"]
        self.assertEqual((d["asked"], d["button"]), (True, "Delete"))
        self.assertEqual([c["method"] for c in self.probe("history_no")["calls"]], [])                                       # Cancel forgets nothing
        done = self.probe("history_yes")["calls"]
        self.assertEqual([c["method"] for c in done], ["/resetWoBLE", "/deletePairedDeviceList", "/removePairedDevices", "/deleteIPConnectionHistory", "/removeBlockedDeviceList"])
        self.assertEqual([c["payload"] for c in done], [{}] * 5)
        self.assertEqual([c["error"] for c in done], [""] * 5)
        self.assertEqual(self.probe("history_after")["res"], {"button": "Done"})

    def test_app_permissions_are_LGs_four_lists_and_an_app_is_allowed_only_after_the_question(self):
        rows = self.probe("perm_rows")["res"]
        self.assertEqual(rows, [[["Camera", True], ["Netflix", False]], [["Camera", False], ["Netflix", True], ["Hue Sync", False]], [["Hue Sync", True]], [["Netflix", True]]])
        d = self.probe("perm_camera_ask")["res"]
        self.assertEqual((d["asked"], d["on"]), (True, False))
        self.assertEqual([c for c in self.probe("perm_camera_no")["calls"] if c["method"] == "/setSystemSettings"], [])           # Block leaves it blocked
        cam = {"cameraResourcePermission": [{"appId": "com.webos.app.camera", "permissions": [{"resource": "camera", "permission": True}]},
                                             {"appId": "netflix", "permissions": [{"resource": "camera", "permission": True}]}]}
        self.assertEqual(self.written("perm_camera_yes"), [cam])                                                              # the whole table, one flag changed
        self.assertEqual(self.probe("perm_camera_off")["res"], {"camera": False, "netflix": True})
        cam["cameraResourcePermission"][0]["permissions"][0]["permission"] = False
        self.assertEqual(self.written("perm_camera_off"), [cam])
        self.assertEqual(self.written("perm_ble_off"), [{"nearbyDevicePermission": [{"appId": "com.lighting.huesync", "permissions": [{"resource": "ble", "permission": False}]}]}])
        self.assertEqual([c["payload"] for c in self.probe("perm_mic_on")["calls"] if c["method"] == "/setPermission"],
                         [{"appId": "com.webos.app.camera", "privacyGroup": "microphone.privacy", "status": "allow"}])
        self.assertEqual(self.probe("perm_mic_after")["res"], {"on": True})                                                   # read back from the service
        self.assertEqual([c["payload"] for c in self.probe("perm_file_off")["calls"] if c["method"] == "/setPermission"],
                         [{"appId": "netflix", "privacyGroup": "usbFileAccess.privacy", "status": "deny"}])
        self.assertEqual(self.probe("perm_file_after")["res"], {"on": False})
        for name in ("perm_camera_yes", "perm_camera_off", "perm_mic_on", "perm_ble_off", "perm_file_off"):
            self.assertEqual([c for c in self.probe(name)["calls"] if c["error"]], [])

    def test_delete_my_personal_information_waits_for_the_terms_and_the_network_and_asks_before_it_tells_LGs_services(self):
        self.assertEqual(self.probe("del_state")["res"], {"a": True, "da": "", "b": True, "db": ""})
        self.assertTrue(self.probe("del_view_ask")["res"]["asked"])
        self.assertEqual([c for c in self.probe("del_view_no")["calls"] if c["method"] in ("/reset", "/deletePrivacyData")], [])
        done = self.probe("del_view_yes")["calls"]
        self.assertEqual([(c["method"], c["payload"]) for c in done], [("/getIDs", {"idType": ["LGUDID"]}), ("/reset", {"target": "viewingInfo", "toast": False})])
        self.assertEqual(self.probe("del_view_sent")["res"], [{"target": "viewingInfo", "toast": False},
                                                                {"toast": True, "bodyData": {"isDeleteAccountAllData": True, "lgudid": "FAKE-LGUDID-0001"}}])      # after the id came back
        done = self.probe("del_voice_yes")["calls"]
        self.assertEqual([(c["method"], c["payload"]) for c in done], [("/deletePrivacyData", {}), ("/reset", {"target": "voiceInfo", "toast": False})])
        self.assertEqual(self.probe("del_after")["res"], {"a": "Done", "b": "Done"})
        self.assertEqual(self.probe("del_voice_locked")["res"], {"a": True, "b": False, "db": "Needs LG's terms"})        # the voice also needs the voice terms

    def test_usb_device_lists_the_sound_cards_and_picks_one_the_way_LGs_list_does(self):
        self.assertEqual(self.probe("usb_link")["res"], {"shown": True, "value": ""})
        self.assertEqual(self.probe("usb_rows")["res"], [["Device List", ""], ["USB Audio Device", ""], ["Gaming Headset", "In use"]])
        d = self.probe("usb_pick")
        self.assertEqual([(c["method"], c["payload"]) for c in d["calls"] if c["method"] == "/setSoundOutput"], [("/setSoundOutput", {"soundOutput": "usb_speaker", "cardNumber": 1})])
        self.assertEqual([c for c in d["calls"] if c["error"]], [])
        self.assertEqual(self.probe("usb_after")["res"], [["Device List", ""], ["USB Audio Device", "In use"], ["Gaming Headset", ""]])


if __name__ == "__main__":
    unittest.main()
