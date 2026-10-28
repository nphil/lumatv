"""Run with: python3 -m unittest tests.test_volume_osd   (needs Xvfb and PyQt5; skipped without them)

The volume and mute display (panels/VolumeOsd.qml, held by LumaOverlay.qml) through the offline QML host, the fake helper and the host's stand-in
audio service (`LS.audioService`, which a test tells where the sound goes and makes announce a change, as a volume key does):

- whose volume it shows: the receiver's own, in its own units, while the sound goes over ARC to a receiver that is on and answering; the TV's
  own where the TV's speakers are the output or the receiver is off, unreachable or switched off in Settings; a receiver that may answer and has
  not yet is waited for (`decideMs`), not replaced by the TV's counter and then corrected;
- that a reading that is not in date is not shown, and that a silent helper loses a mute that stayed;
- the mute: the word, then a mark that stays and moves, from the receiver's push or the TV's mute;
- that it is up over Home as well as over an app, takes no key and no focus, has the glass see-through only over an app's picture, and that
  Home's own layers no longer hold a second one.

The host runs on a shared, loaded machine in software GL: a reading the fake helper publishes reaches the display after the helper's first sync
(about a second after the start) and a long poll later, so every check comes a second and more after what it checks, and the display's own
timers are lengthened where a test has to catch them mid-way.
"""
import json
import os
import re
import sys
import unittest

from test_settings import HostCase
from xdisplay import DISPLAY, HAVE_QT

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "qmlhost"))

FIND = ("var f=function(p){if(p.staleMs!==undefined)return p;for(var i=0;i<p.children.length;i++){var r=f(p.children[i]);if(r)return r}return null};"
        "var o=f(overlay); if(!o) throw Error('volume display not built');")
PLAIN = '{"perf":{"syncResidents":true}}'
B = 1500                # the helper is up and has synced from here on

# what the helper publishes for the Denon on the set: the absolute scale, input "TV"
RX = {"enabled": True, "error": None, "fresh": True, "host": "192.168.1.93", "input": "TV", "mute": False, "polling": True, "power": "on",
      "reachable": True, "volumeAbs": 46.0, "volumeDb": -34.0, "volumeDisplay": "absolute", "volumeText": "46"}
RX_DB = dict(RX, volumeAbs=45.5, volumeDb=-34.5, volumeDisplay="relative", volumeText="-34.5 dB")
GONE = {"enabled": True, "error": None, "fresh": False, "host": "192.168.1.93", "polling": True, "reachable": False}
OFF = {"enabled": False, "error": None, "fresh": False, "host": "192.168.1.93", "polling": False, "reachable": None}


def on_osd(body):
    """An expression over `o`, the display, found in the overlay."""
    return "(function(){ %s return (%s) })()" % (FIND, body)


def do(body):
    return "(function(){ %s %s; return 1 })()" % (FIND, body)


def rx(doc, age=0):
    """The receiver published from inside the host with its own clock (a reading's stamp is what ages it); `age` ms old."""
    return ("(function(){var d=%s; d.updatedAt=Date.now()-%d; var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/receiver');"
            "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d)); return 1})()" % (json.dumps(doc), age))


def quiet(doc):
    """A newer stamp the helper holds without an event: what the display gets only by asking."""
    return ("(function(){var d=%s; d.updatedAt=Date.now(); var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__quiet/receiver');"
            "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d)); return 1})()" % json.dumps(doc))


def audio(body):
    assert "'" not in body
    # LS is in WebOSCompositorBase on the set and in the host's stub of it; the second import is the older stub's home for it
    return ("(function(){ Qt.createQmlObject('import QtQuick 2.4\\nimport WebOSCompositorBase 1.0\\nimport WebOSServices 1.0\\nQtObject { Component.onCompleted: { %s } }', overlay, 'audio'); return 1 })()" % body)


def sound_out(output="external_arc", volume=31, syncable=False):
    return audio('LS.audioService.soundOutput = "%s"; LS.audioService.volume = %d; LS.audioService.volumeSyncable = %s' % (output, volume, "true" if syncable else "false"))


def press(why="volumeUp", volume=None, muted=None):
    return audio("LS.audioService.report(%s, %s, %s)" % (json.dumps(why), "null" if volume is None else volume, "null" if muted is None else ("true" if muted else "false")))


SHOWN = on_osd("[o.pill, o.paneUp, o.source, o.level, o.muted, o.markUp].join('|')")
# the mark at the bar's right end: [its key, its words, drawn, its accessible name, its size, the bar's width]
LOGO = on_osd("(function(){ var g=function(p){if(p.aspect!==undefined)return p;for(var i=0;i<p.children.length;i++){var r=g(p.children[i]);if(r)return r}return null}; "
              "var l=g(o); return [o.logoKey, o.format, l.visible && l.ready, l.label, l.width + 'x' + l.height, l.parent.width].join('|') })()")
POSTS = ("(function(){var x=new XMLHttpRequest(); x.onreadystatechange=function(){if(x.readyState===4) console.info('[posts] '+x.responseText)};"
         "x.open('GET','http://127.0.0.1:18731/__posts'); x.send(); return 1})()")

# A warning from these files is the display's; one from any other surface is work in progress elsewhere in the tree and is not judged here.
OWN = re.compile(r"VolumeOsd|TvVolume|LumaOverlay|Glass\.qml|LuPane|kit/Lu\.qml|ReceiverGate|StarfishFullscreenContainer|overlay volume")


@unittest.skipUnless(DISPLAY and HAVE_QT, "needs Xvfb and PyQt5")
class VolumeOsd(HostCase):
    def clean(self, rc, out):
        """The host exits 0 when nothing in the tree warned. Nothing of the display's may have, and with nothing else warned it is 0."""
        stray = [ln for ln in out.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
        self.assertEqual([ln for ln in stray if OWN.search(ln)], [], out[-2000:])
        if not stray:
            self.assertEqual(rc, 0, out[-2000:])

    def run_osd(self, steps, wait, home=False, theme=PLAIN):
        """steps: (ms, js, label); each `label=value` is read back. The overlay is built at once; an app is in front from 800 ms unless `home`.
        Returns ({label: value}, the host's whole output)."""
        argv = ["--eval", "100:(function(){ overlay.warm = true; return 1 })()"]
        if not home:
            argv += ["--eval", "800:(function(){ overlay.appFront = true; return 1 })()"]
        for ms, expr, label in steps:
            argv += ["--eval", "%d:(function(){ return %s + (%s) })()" % (ms, json.dumps(label + "="), expr)]
        rc, out = self.host(*argv, "--overlay", "--prop", "hostActive=" + ("true" if home else "false"), "--theme", theme, "--wait", str(wait))
        self.clean(rc, out)
        got = {}
        for v in self.values(out):
            m = re.match(r"^([a-z0-9_]+)=(.*)$", v, re.S)
            if m:
                got[m.group(1)] = m.group(2)
        return got, out

    def test_the_receiver_is_read_while_the_sound_goes_to_it_and_the_keys_are_asked_about(self):
        got, out = self.run_osd([
            (300, do("o.holdMs = 3000"), "hold"), (B, sound_out(), "out"), (B + 200, rx(RX), "rx"), (B + 1800, SHOWN, "idle"),
            (B + 2000, press("volumeUp", 32), "key"), (B + 2800, SHOWN, "key_state"),
            # the receiver's own push, a moment later: the number changes in place and the pane stays
            (B + 3000, rx(dict(RX, volumeAbs=46.5, volumeDb=-33.5, volumeText="46.5")), "push"), (B + 4000, SHOWN, "pushed"),
            (B + 4200, POSTS, "posts"), (B + 4300, on_osd("o.fraction"), "fraction"),
            (B + 8000, SHOWN, "later"),
        ], B + 8600)
        self.assertEqual(got["idle"], "false|false|receiver|46|false|false", "nothing is on screen before a key, the receiver is the source")
        self.assertEqual(got["key_state"], "true|true|receiver|47|false|false", "a key moves the receiver's own level at once, one step, before it says anything")
        self.assertEqual(got["pushed"], "true|true|receiver|47|false|false", "the receiver's push changes the number in place")
        self.assertAlmostEqual(float(got["fraction"]), 46.5 / 98, places=3)
        self.assertEqual(got["later"], "false|false|receiver|47|false|false", "gone after the hold")
        posts = json.loads(re.findall(r"\[posts\] (.*)", out)[-1])["posts"]
        self.assertIn(["/v1/receiver/nudge", {}], posts, "a key asks the helper to read the receiver now")

    def test_a_receiver_set_to_decibels_still_shows_the_whole_number_level(self):
        got, _ = self.run_osd([(B, sound_out(), "out"), (B + 200, rx(RX_DB), "rx"), (B + 2000, press("volumeDown", 30), "key"),
                               (B + 2800, on_osd("o.level"), "db")], B + 3400)
        self.assertEqual(got["db"], "45", "a receiver showing decibels still gives the whole-number level")

    def test_the_format_is_a_mark_at_the_end_of_the_bar_named_in_words_and_a_format_without_one_shows_nothing(self):
        atmos = dict(RX, soundMode="DOLBY ATMOS", signal="Dolby Atmos - DD+")
        dolby = dict(RX, soundMode="DOLBY AUDIO-DD+DSUR", signal="DOLBY D+ 5.1")
        dtsx = dict(RX, soundMode="DTS:X", signal="DTS:X")
        pcm = dict(RX, soundMode="STEREO", signal="PCM 2.0")
        got, _ = self.run_osd([
            (300, do("o.holdMs = 30000"), "hold"), (B, sound_out(), "out"),
            (B + 200, rx(pcm), "rx"), (B + 1800, press("volumeUp", 32), "key"), (B + 2600, LOGO, "pcm"),
            (B + 3000, rx(atmos), "r2"), (B + 4200, LOGO, "atmos"),
            (B + 4400, rx(dolby), "r3"), (B + 5600, LOGO, "dolby"),
            (B + 5800, rx(dtsx), "r4"), (B + 7000, LOGO, "dtsx"),
            (B + 7200, rx(dict(dtsx, mute=True)), "r5"), (B + 8400, LOGO, "muted"),
        ], B + 9000)
        key, words, drawn, label, size, width = got["pcm"].split("|")
        self.assertEqual((key, words, drawn, label), ("", "", "false", ""), "PCM stereo: no mark and no word")
        self.assertEqual(width, "440", "the bar is its plain width with nothing at its end")
        for state, want, name in (("atmos", "atmos", "Dolby Atmos"), ("dolby", "dolby", "Dolby Digital+ \u00b7 Surround"), ("dtsx", "dtsx", "DTS:X")):
            key, words, drawn, label, size, width = got[state].split("|")
            self.assertEqual((key, words, drawn, label), (want, name, "true", name), state)
            w, h = (int(v) for v in size.split("x"))
            self.assertTrue(20 <= h <= 28 and 0 < w <= 7 * 28, "%s: drawn %s, as tall as the number's text and no wider than 7 times that" % (state, size))
            self.assertGreaterEqual(int(width), 2 * 24 + 32 + 20 + 140 + 20 + w, state + ": the bar's width counts the mark")
        self.assertEqual(got["muted"].split("|")[:3], ["", "", "false"], "muted, the slashed speaker is the only mark")

    def test_a_key_moves_the_level_at_once_and_an_old_reading_does_not_move_it_back(self):
        got, _ = self.run_osd([
            (300, do("o.holdMs = 7000; o.settleMs = 1200"), "cfg"), (B, sound_out(), "out"), (B + 200, rx(RX), "rx"),
            # five presses of a held key, 100 ms apart: the level has moved five steps before any reading, and the fill with it
            (B + 2000, press("volumeUp", 32), "k0"), (B + 2100, press("volumeUp", 33), "k1"), (B + 2200, press("volumeUp", 34), "k2"),
            (B + 2300, press("volumeUp", 35), "k3"), (B + 2400, press("volumeUp", 36), "k4"),
            (B + 2500, on_osd("[o.level, o.fraction.toFixed(3)].join('|')"), "keyed"),
            # a reading from before the keys reaches the display: it does not take the level back
            (B + 2600, rx(RX), "old"), (B + 3000, on_osd("o.level"), "kept"),
            # the receiver's own, after the last key: the level it reports is the one shown once the keys' answer has run out
            (B + 3100, rx(dict(RX, volumeAbs=48.5, volumeDb=-31.5, volumeText="48.5")), "reported"),
            (B + 5000, on_osd("[o.level, o.keyed].join('|')"), "reconciled"),
        ], B + 5600)
        self.assertEqual(got["keyed"].split("|")[0], "49", "five keys, five steps of 0.5, before the receiver says anything")
        self.assertAlmostEqual(float(got["keyed"].split("|")[1]), 48.5 / 98, places=3)
        self.assertEqual(got["kept"], "49", "an old reading does not move it back")
        self.assertEqual(got["reconciled"], "49|false", "then the reading is taken")

    def test_a_volume_key_over_a_muted_receiver_shows_it_unmuted_at_once_as_the_denon_cancels_its_mute(self):
        got, _ = self.run_osd([
            (300, do("o.holdMs = 4000; o.staleMs = 60000"), "cfg"), (B, sound_out(), "out"), (B + 200, rx(dict(RX, mute=True)), "muted"),
            (B + 2000, press("volumeUp", 32), "key"), (B + 2300, SHOWN, "after_key"),
        ], B + 2900)
        self.assertEqual(got["after_key"], "true|true|receiver|47|false|false", "Volume+ over a muted Denon: the number, not the muted form")

    def test_the_mute_key_has_no_direction_but_the_tvs_mute_flag_moves_and_the_bar_shows_it_at_once(self):
        got, _ = self.run_osd([
            (300, do("o.holdMs = 4000; o.staleMs = 60000"), "cfg"), (B, sound_out(), "out"), (B + 200, rx(RX), "rx"),
            (B + 2000, press("", None, True), "key"), (B + 2300, SHOWN, "after_key"),
            # the Denon's own push arrives later and agrees: nothing changes
            (B + 2600, rx(dict(RX, mute=True)), "push"), (B + 3000, SHOWN, "after_push"),
        ], B + 3600)
        self.assertEqual(got["after_key"], "true|true|receiver|46|true|false", "the mute key: the muted form before the receiver says anything")
        self.assertEqual(got["after_push"], "true|true|receiver|46|true|false")

    def test_without_a_receiver_it_is_the_tvs_own_volume(self):
        cases = [("unreachable", GONE), ("switched off in Settings", OFF), ("in standby", dict(RX, power="standby"))]
        steps = []
        for i, (_, doc) in enumerate(cases):
            t = B + i * 3500
            steps += [(t, rx(doc), "rx%d" % i), (t + 200, sound_out("external_arc", 31 + i), "out%d" % i), (t + 1800, press("volumeUp", 32 + i), "key%d" % i),
                      (t + 2600, SHOWN, "state%d" % i), (t + 2650, on_osd("o.fraction"), "fraction%d" % i), (t + 2700, do("o.pill = false"), "off%d" % i)]
        got, _ = self.run_osd(steps, B + 3500 * len(cases) + 500)
        for i, (name, _) in enumerate(cases):
            self.assertEqual(got["state%d" % i], "true|true|tv|%d|false|false" % (32 + i), name)
            self.assertAlmostEqual(float(got["fraction%d" % i]), (32 + i) / 100, places=3)

    def test_the_tvs_speakers_are_the_tvs_volume_even_with_the_receiver_on_and_a_receiver_change_is_not_shown(self):
        got, _ = self.run_osd([
            (B, sound_out("tv_speaker", 18, True), "out"), (B + 200, rx(RX), "rx"), (B + 2000, press("volumeUp", 19), "key"), (B + 2800, SHOWN, "key_state"),
            (B + 2900, do("o.pill = false"), "off"),
            (B + 3500, rx(dict(RX, volumeAbs=50.0, volumeText="50")), "push"), (B + 5000, SHOWN, "after_push"),
            # the headphones, Bluetooth: the same
            (B + 5200, sound_out("headphone", 7, True), "hp"), (B + 5600, press("volumeDown", 6), "hp_key"), (B + 6400, SHOWN, "hp_state"),
        ], B + 7000)
        self.assertEqual(got["key_state"], "true|true|tv|19|false|false")
        self.assertEqual(got["after_push"], "false|false|tv|19|false|false", "the receiver is not the output: its own level is not what the keys changed")
        self.assertEqual(got["hp_state"], "true|true|tv|6|false|false")

    def test_a_receiver_that_may_answer_is_waited_for_and_the_tvs_counter_is_the_answer_only_when_it_does_not(self):
        got, _ = self.run_osd([
            (300, do("o.holdMs = 4000; o.decideMs = 1500"), "hold"), (B, sound_out(), "out"),
            # the reading on the display is 8 s old: the helper still says it is reachable
            (B + 200, rx(RX, 8000), "old"), (B + 1800, on_osd("[o.fresh, o.stale].join('|')"), "aged"),
            # nothing answers: held back for the decision window, then the TV's own volume
            (B + 2000, press("volumeUp", 32), "key"), (B + 2500, SHOWN, "waiting"), (B + 4000, SHOWN, "settled"),
            (B + 4200, do("o.pill = false"), "off"),
            # the helper has a newer reading it did not announce: the key's question gets it, and the receiver's level is what shows
            (B + 5000, quiet(RX), "quiet"), (B + 6000, press("volumeUp", 33), "key2"), (B + 7500, SHOWN, "answered"),
        ], B + 8200)
        self.assertEqual(got["aged"], "false|true", "the reading is not in date")
        self.assertEqual(got["waiting"], "true|false|wait||false|false", "a key waits for a receiver that may answer: nothing is drawn yet")
        self.assertEqual(got["settled"], "true|true|tv|32|false|false", "no answer: the TV's own volume")
        self.assertEqual(got["answered"], "true|true|receiver|46|false|false", "answered: the receiver's own level, never the counter first")

    def test_the_mute_is_a_word_then_a_mark_that_stays_and_moves_until_the_sound_is_back(self):
        steps = [(300, do("o.holdMs = 1500; o.hopMs = 1500; o.staleMs = 60000"), "hold"), (B, sound_out(), "out"), (B + 200, rx(RX), "rx"),
                 (B + 2000, rx(dict(RX, mute=True)), "mute"), (B + 2900, SHOWN, "pane"), (B + 5000, SHOWN, "mark")]
        steps += [(B + 5100 + i * 1000, on_osd("[o.markPlace, o.markX, o.markY].join('|')"), "place%d" % i) for i in range(6)]
        steps += [(B + 11500, rx(dict(RX, mute=False)), "unmute"), (B + 12800, SHOWN, "back"), (B + 16000, SHOWN, "after")]
        got, _ = self.run_osd(steps, B + 16600)
        self.assertEqual(got["pane"], "true|true|receiver|46|true|false", "a mute from the receiver shows the pane in its muted form")
        self.assertEqual(got["mark"], "false|false|receiver|46|true|true", "after the hold the mark stays")
        spots = [tuple(int(v) for v in got["place%d" % i].split("|")) for i in range(6)]
        places = [s[0] for s in spots]
        self.assertGreaterEqual(len(set(places)), 2, "the mark moves: %s" % places)
        # the top corners only: inside the 5 % safe area, below Home's status bar, above the lower third and away from the middle
        layout = json.load(open(os.path.join(os.path.dirname(HERE), "payload", "theme", "layout.json")))
        bar_bottom = layout["statusBar"]["height"] + layout["safe"]["y"]
        for _, x, y in spots:
            self.assertTrue(96 <= x and x + 64 <= 1920 - 96, "inside the safe width: %d" % x)
            self.assertTrue(y >= bar_bottom and y + 64 <= 1080 // 3, "in the top third and clear of the status bar: %d" % y)
            self.assertTrue(x + 64 <= 1920 * 0.3 or x >= 1920 * 0.7, "in a corner, not the middle: %d" % x)
        self.assertEqual(got["back"].split("|")[0], "true", "the receiver's unmute shows the pane again")
        self.assertEqual(got["after"], "false|false|receiver|46|false|false", "unmuted: nothing stays")

    def test_the_mark_holds_still_while_a_panel_is_up_and_moves_on_after(self):
        # its hop is a fade out and in, frames that chained onto a panel's slide over the film (round 16: picker bursts of 292 and 318 ms)
        steps = [(300, do("o.holdMs = 1500; o.hopMs = 1500; o.staleMs = 60000"), "hold"), (B, sound_out(), "out"), (B + 200, rx(RX), "rx"),
                 (B + 2000, rx(dict(RX, mute=True)), "mute"), (B + 4000, do("o.still = true"), "still")]
        steps += [(B + 4100 + i * 1000, on_osd("[o.markPlace, o.markUp].join('|')"), "held%d" % i) for i in range(4)]
        steps += [(B + 8000, do("o.still = false"), "free")]
        steps += [(B + 8100 + i * 1000, on_osd("o.markPlace"), "after%d" % i) for i in range(4)]
        got, _ = self.run_osd(steps, B + 12500)
        held = [got["held%d" % i] for i in range(4)]
        self.assertEqual(len(set(held)), 1, "the mark hopped while a panel was up: %s" % held)
        self.assertTrue(held[0].endswith("|true"), held)
        self.assertGreaterEqual(len({got["after%d" % i] for i in range(4)}), 2, "the mark stopped hopping for good")

    def test_over_home_the_mark_hops_along_the_status_bars_row_between_its_groups(self):
        # Home's corners are widgets (the clock top left, the weather top right) and the saver's clock roams the safe area: only the top edge is free
        steps = [(300, do("o.holdMs = 1500; o.hopMs = 1500; o.staleMs = 60000"), "hold"), (B, sound_out(), "out"), (B + 200, rx(RX), "rx"),
                 (B + 2000, rx(dict(RX, mute=True)), "mute")]
        steps += [(B + 5100 + i * 1000, on_osd("[o.markPlace, o.markX, o.markY].join('|')"), "place%d" % i) for i in range(6)]
        got, _ = self.run_osd(steps, B + 11500, home=True)
        layout = json.load(open(os.path.join(os.path.dirname(HERE), "payload", "theme", "layout.json")))
        top, bottom = layout["safe"]["y"], layout["safe"]["y"] + layout["statusBar"]["height"]
        spots = [tuple(int(v) for v in got["place%d" % i].split("|")) for i in range(6)]
        self.assertGreaterEqual(len(set(s[0] for s in spots)), 2, "the mark moves: %s" % spots)
        for _, x, y in spots:
            self.assertTrue(top - 8 <= y and y + 64 <= bottom + 8, "in the status bar's row: %d" % y)
            self.assertTrue(400 <= x and x + 64 <= 1500, "between the profile on the left and the three icons on the right: %d" % x)

    def test_the_tvs_own_mute_shows_and_stays_when_there_is_no_receiver(self):
        got, _ = self.run_osd([
            (300, do("o.holdMs = 1500"), "hold"), (B, sound_out("tv_speaker", 18, True), "out"), (B + 200, rx(GONE), "rx"),
            (B + 2000, press("mute", None, True), "key"), (B + 2800, on_osd("[o.pill, o.source, o.muted].join('|')"), "pane"),
            (B + 5000, SHOWN, "mark"), (B + 6000, press("mute", None, False), "unmute"), (B + 7000, on_osd("[o.markUp, o.muted].join('|')"), "after"),
        ], B + 7600)
        self.assertEqual(got["pane"], "true|tv|true", "the TV's mute: the word")
        self.assertEqual(got["mark"], "false|false|tv|18|true|true")
        self.assertEqual(got["after"], "false|false")

    def test_a_mute_that_is_in_date_stays_while_the_helper_keeps_reading_and_goes_when_it_stops(self):
        # the helper moves a reading's stamp quietly: the display asks for it each time the copy it has ages out, as long as the mute is on screen
        steps = [(300, do("o.holdMs = 300; o.staleMs = 2500; o.graceMs = 1000"), "cfg"), (B, sound_out(), "out"), (B + 200, rx(dict(RX, mute=True)), "mute")]
        for i in range(9):
            steps.append((B + 1000 + i * 1000, quiet(dict(RX, mute=True)), "q%d" % i))
        steps += [(B + 2500, on_osd("o.markUp"), "kept1"), (B + 6000, on_osd("o.markUp"), "kept2"), (B + 9500, on_osd("o.markUp"), "kept3"),
                  (B + 15500, on_osd("o.markUp"), "lost")]
        got, _ = self.run_osd(steps, B + 16000)
        self.assertEqual((got["kept1"], got["kept2"], got["kept3"]), ("true", "true", "true"), "a helper that keeps reading keeps the mark")
        self.assertEqual(got["lost"], "false", "a helper that has stopped does not leave a mute up that is no longer known")

    def test_a_mute_from_before_a_restart_comes_back_when_the_helper_holds_a_newer_stamp(self):
        # the first copy is old (the helper moved the stamp without an event): the display asks for the newest, with no mute on screen yet, until the helper has moved the stamp
        steps = [(300, do("o.holdMs = 300; o.staleMs = 6000"), "cfg"), (B, sound_out(), "out"),
                 (B + 100, rx(dict(RX, mute=True), 60000), "old"), (B + 1000, quiet(dict(RX, mute=True)), "quiet"),
                 (B + 5000, on_osd("[o.markUp, o.muted].join('|')"), "mark")]
        got, _ = self.run_osd(steps, B + 5600, home=True)
        self.assertEqual(got["mark"], "true|true", "the mark is up from the copy the helper holds, not only after the next change")

    def test_a_reading_that_is_not_in_date_is_not_shown_and_the_next_is(self):
        got, _ = self.run_osd([
            (300, do("o.staleMs = 4000; o.holdMs = 15000"), "cfg"), (B + 200, rx(RX), "rx"), (B + 1000, do("o.show()"), "show"), (B + 1800, on_osd("[o.fresh, o.level].join('|')"), "fresh"),
            (B + 6000, on_osd("[o.fresh, o.stale, o.level, o.offline, o.source].join('|')"), "old"),
            (B + 6200, rx(RX), "again"), (B + 8000, on_osd("[o.fresh, o.level, o.offline].join('|')"), "back"),
        ], B + 8600)
        self.assertEqual(got["fresh"], "true|46")
        self.assertEqual(got["old"], "false|true||true|none", "the number of a reading that has aged out is not drawn: the display says it has nothing")
        self.assertEqual(got["back"], "true|46|false")

    def test_over_an_apps_picture_the_glass_is_see_through_and_over_home_it_is_not(self):
        got, _ = self.run_osd([(B, sound_out(), "out"), (B + 200, rx(RX), "rx"), (B + 2000, press("volumeUp", 32), "key"),
                               (B + 2800, on_osd("[o.overVideo, o.paneAlpha].join('|')"), "app"),
                               (B + 3000, do("overlay.appFront = false"), "home"), (B + 3800, on_osd("[o.overVideo, o.paneAlpha].join('|')"), "home_state")], B + 4400)
        theme = json.load(open(os.path.join(os.path.dirname(HERE), "payload", "theme", "theme.json")))
        alpha = theme["dark"]["readingAlpha"]
        self.assertEqual(got["app"], "true|%s" % alpha)
        self.assertLess(alpha, 0.96, "less opaque than Lucent's overlay floor, as asked, with the contrast measured (test_osd_contrast)")
        self.assertEqual(got["home_state"], "false|1")

    def test_the_bar_rides_120_px_higher_over_an_apps_picture_and_stays_put_over_home(self):
        pos = on_osd("o.bottomGap + '|' + Math.round(o.raise)")
        got, _ = self.run_osd([(B, sound_out(), "out"), (B + 200, rx(RX), "rx"), (B + 2000, press("volumeUp", 32), "key"),
                               (B + 2900, pos, "video"),
                               (B + 3000, do("overlay.appFront = false"), "home"), (B + 5600, press("volumeDown", 31), "key2"), (B + 6300, pos, "home_state")], B + 7000)
        self.assertEqual(got["video"], "184|120", "64 px + 120 px from the bottom edge over a picture")
        # an app that goes while the bar shows: it glides down (180 ms) instead of jumping
        moving, _ = self.run_osd([(300, do("o.holdMs = 7000"), "cfg"), (B, sound_out(), "out"), (B + 200, rx(RX), "rx"), (B + 1500, press("volumeUp", 32), "key"),
                                  (B + 3200, do("overlay.appFront = false"), "home"), (B + 3220, on_osd("[o.raise, o.pill, o.overVideo, o.paneUp].join('|')"), "mid"), (B + 3900, on_osd("o.raise"), "end")], B + 4300)
        self.assertTrue(0 < float(moving["mid"].split("|")[0]) < 120, "mid-glide, not jumped: %s" % moving["mid"])
        self.assertEqual(float(moving["end"]), 0.0)
        self.assertEqual(got["home_state"], "64|0", "64 px from the bottom edge over Home")

    def test_it_is_up_over_home_too_takes_no_key_and_no_focus_and_the_layer_is_drawn_only_while_it_has_something(self):
        got, _ = self.run_osd([
            (300, do("o.holdMs = 1500"), "cfg"), (B, sound_out(), "out"), (B + 200, rx(RX), "rx"),
            (B + 1800, "[overlay.visible, overlay.up].join('|')", "idle"),
            (B + 2000, press("volumeUp", 32), "key"), (B + 2700, "[overlay.visible, overlay.up, overlay.appFront].join('|')", "layer"),
            (B + 2800, on_osd("[o.activeFocus, o.focus].join('|')"), "focus"),
            (B + 2900, "[overlay.key(Qt.Key_VolumeUp, true, false, 0), overlay.key(Qt.Key_Return, true, false, 0), overlay.wants].join('|')", "keys"),
            (B + 7000, "overlay.visible", "gone"),
        ], B + 7600, home=True)
        self.assertEqual(got["idle"], "false|false", "the layer is not drawn for nothing")
        self.assertEqual(got["layer"], "true|false|false", "over Home, nothing but the display is up in the layer")
        self.assertEqual(got["focus"], "false|false")
        self.assertEqual(got["keys"], "false|false|false", "the volume keys are the app's and OK is nobody's here")
        self.assertEqual(got["gone"], "false")

    def test_home_holds_no_second_display(self):
        rc, out = self.host("--eval", "7000:(function(){var f=function(p){if(p.staleMs!==undefined)return p;for(var i=0;i<p.children.length;i++){var r=f(p.children[i]);if(r)return r}return null};"
                            "return 'osd=' + (f(root) ? 'found' : 'none')})()", "--wait", "7600")
        self.clean(rc, out)
        self.assertIn("osd=none", out, "a display in Home's layers as well as in the overlay would answer every key twice")


if __name__ == "__main__":
    unittest.main()
