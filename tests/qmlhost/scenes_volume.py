"""Scenes of the volume and mute display (mock 11): the slim pane at the right edge and the mark that stays while the sound is muted, over a
picture standing in for video (an app in front: the glass is see-through to Lu.readingAlpha) and over Home, dark and light.

    python3 tests/qmlhost/scenes.py --only 11-volume-osd,11-volume-osd-muted --out /tmp/volume-shots

Each is `LumaOverlay.qml` beside Home as the fullscreen container loads it (qmlhost.py `--overlay`), where the display lives. The receiver is the helper's
`receiver` topic as the set publishes it (the Denon over eARC, counting 0 to 98); the TV's own volume is the host's stand-in audio service
(`LS.audioService`, tests/qmlhost/stubs/WebOSServices/LS.qml), which a scene tells where the sound goes and then makes announce a change, as a volume key does.
"""
import json
import os
import subprocess
import sys

from scenes import HERE, NOW, js, png_path, scene
import scenes_host
import xdisplay

# what the helper publishes for the Denon on the set (read 2026-10-25): absolute scale, input "TV"
RX = {"enabled": True, "error": None, "fresh": True, "host": "192.168.1.93", "input": "TV", "mute": False, "polling": True, "power": "on",
      "reachable": True, "volumeAbs": 46.0, "volumeDb": -34.0, "volumeDisplay": "absolute", "volumeText": "46"}
# the same receiver on the decibel scale
RX_DB = dict(RX, volumeAbs=45.5, volumeDb=-34.5, volumeDisplay="relative", volumeText="-34.5 dB")
GONE = {"enabled": True, "error": None, "fresh": False, "host": "192.168.1.93", "polling": True, "reachable": False}


def receiver_at(at, doc):
    """The receiver published from inside the host at `at` ms, with its own clock: a reading's stamp is what ages it."""
    code = ("(function(){var d=%s; d.updatedAt=Date.now(); var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/receiver');"
            "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d)); return 1})()") % json.dumps(doc)
    return (at, code)


def audio_at(at, body):
    """`body` (JavaScript, double quotes only) run against the host's stand-in audio service at `at` ms."""
    assert "'" not in body
    return (at, "(function(){ Qt.createQmlObject('import QtQuick 2.4\\nimport WebOSServices 1.0\\nQtObject { Component.onCompleted: { %s } }', overlay, 'audio'); return 1 })()" % body)


def osd_at(at, body):
    """`body` run with `o`, the display (the item of the overlay that has `staleMs`), at `at` ms."""
    return (at, "(function(){ var f=function(p){if(p.staleMs!==undefined)return p;for(var i=0;i<p.children.length;i++){var r=f(p.children[i]);if(r)return r}return null}; var o=f(overlay); %s; return 1 })()" % body)


def sound_out(at, output="external_arc", volume=31, syncable=False):
    return audio_at(at, 'LS.audioService.soundOutput = "%s"; LS.audioService.volume = %d; LS.audioService.volumeSyncable = %s' % (output, volume, "true" if syncable else "false"))


def key_at(at, why="volumeUp", volume=None, muted=None):
    """What a volume key does on the set: the audio service changes and says so."""
    return audio_at(at, "LS.audioService.report(%s, %s, %s)" % (json.dumps(why), "null" if volume is None else volume, "null" if muted is None else ("true" if muted else "false")))


def render(name, out_dir, material, steps, snap_at, over="video", bg=None, more_snaps=()):
    """steps: [(ms, js)] run in the host. `over` is "video" (an app in front, from 800 ms, `bg` the picture behind: a stand-in by default) or "home"
    (Home in front); one grab at `snap_at`, and `more_snaps` [(ms, path)] besides. Returns (exit code, warnings, the host's whole output)."""
    path = png_path(out_dir, name, material)
    if os.path.exists(path):
        os.remove(path)
    user = {"perf": {"syncResidents": True}, "material": material}
    args = [sys.executable, os.path.join(HERE, "qmlhost.py"), "LumaHome.qml", "--fake-helper", "--overlay", "--theme", json.dumps(user),
            "--eval", "100:overlay.warm = true"]
    if over == "video":
        args += ["--bgimage", bg or scenes_host.video_png(), "--prop", "hostActive=false", "--eval", "800:overlay.appFront = true"]
    else:
        args += ["--prop", "hostActive=true"]
    for ms, code in steps:
        args += ["--eval", "%d:%s" % (ms, code)]
    for ms, other in more_snaps:
        args += ["--snap", "%d:%s" % (ms, other)]
    args += ["--snap", "%d:%s" % (snap_at, path), "--wait", str(max([snap_at] + [ms for ms, _ in more_snaps]) + 600)]
    r = subprocess.run(args, env=dict(os.environ, DISPLAY=xdisplay.DISPLAY), capture_output=True, text=True, timeout=240)
    text = r.stdout + r.stderr
    warn = [ln for ln in text.splitlines() if ln.startswith("[warning]") and "Implicitly defined onFoo" not in ln]
    if not os.path.exists(path):
        warn.append("[snap] not written: %s" % path)
    return (r.returncode or (1 if not os.path.exists(path) else 0)), warn, text


# The fake helper is up and has synced about a second after the host starts, and a reading published before that reaches the display with the
# sync: everything a scene needs from the receiver comes from here on, and the grab a second and more after the key.
B = 1500


# ---- over a picture standing in for video: the glass lets it through
@scene("11-volume-osd", runner=True)
def _(out_dir, material):
    """The Denon on the set (0 to 98) at 46, a Volume+ just pressed: the number, what it counts, the level and the speaker."""
    return render("11-volume-osd", out_dir, material, [sound_out(B), receiver_at(B + 200, RX), key_at(B + 2000, "volumeUp", 32)], B + 2700)


@scene("11-volume-osd-db", runner=True)
def _(out_dir, material):
    """The Denon on the decibel scale."""
    return render("11-volume-osd-db", out_dir, material, [sound_out(B), receiver_at(B + 200, RX_DB), key_at(B + 2000, "volumeUp", 32)], B + 2700)


@scene("11-volume-osd-muted", runner=True)
def _(out_dir, material):
    """The mute key: the word, what it was, the level dimmed."""
    return render("11-volume-osd-muted", out_dir, material,
                  [sound_out(B), receiver_at(B + 200, RX), receiver_at(B + 2000, dict(RX, mute=True)), key_at(B + 2020, "mute", None, True)], B + 3300)


@scene("11-volume-osd-mark", runner=True)
def _(out_dir, material):
    """The pane has gone; the sound is still muted, and the small mark stays (it moves a little every ten seconds)."""
    return render("11-volume-osd-mark", out_dir, material,
                  [sound_out(B), receiver_at(B + 200, dict(RX, mute=True)), osd_at(B + 300, "o.holdMs = 300"), key_at(B + 2000, "mute", None, True)], B + 4500)


@scene("11-volume-osd-tv", runner=True)
def _(out_dir, material):
    """No receiver (it does not answer): the TV's own volume, which is what the keys changed."""
    return render("11-volume-osd-tv", out_dir, material, [sound_out(B, "tv_speaker", 18, True), receiver_at(B + 200, GONE), key_at(B + 2000, "volumeUp", 19)], B + 2700)


@scene("11-volume-osd-tv-arc", runner=True)
def _(out_dir, material):
    """The sound goes out over ARC and the receiver is off: the TV's step count, said to be the ARC's."""
    return render("11-volume-osd-tv-arc", out_dir, material,
                  [sound_out(B), receiver_at(B + 200, dict(RX, power="standby")), key_at(B + 2000, "volumeUp", 32)], B + 2700)


@scene("11-volume-osd-offline", runner=True)
def _(out_dir, material):
    """The TV has not said where the sound goes and the receiver does not answer: nothing to read."""
    return render("11-volume-osd-offline", out_dir, material, [receiver_at(B + 200, GONE), osd_at(B + 2000, "o.show()")], B + 2700)


# ---- over Home: the glass is the frosted wallpaper's, opaque
@scene("11-volume-osd-home", runner=True)
def _(out_dir, material):
    return render("11-volume-osd-home", out_dir, material, [sound_out(B), receiver_at(B + 200, RX), key_at(B + 2000, "volumeUp", 32)], B + 2700, over="home")
