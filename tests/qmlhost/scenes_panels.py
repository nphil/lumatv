"""Scenes of PanelsSlice: the input picker (08), the quick panel (10) and the receiver page (24). The volume display (11) is in scenes_volume.py.

A receiver reading is old after 6 s (kit/ReceiverGate), so every scene publishes the receiver again just before it
looks, the way the helper does while the screen is on.

Scenes 24-* publish `receiver(controls=True)`: sound modes, Audyssey, levels, Quick Select and Zone 2 are FIXTURES of the
contract the page is written to (docs/mocks/24-*), not what the helper publishes today. The helper reads power, input,
volume and mute over the receiver's HTTP port and has no command route; `24-settings-receiver-today` is the page against
what it really publishes.
"""
import json

from scenes import scene, k, READY, receiver, settings_to

AMBIENT_ROSTER = {"plugins": [{"id": "ambient", "name": "Ambient light", "state": "connected"}]}
AMBIENT_TILES = {"items": [{"id": "ambient", "label": "Ambient light", "icon": "base.icons/lamp", "state": {"type": "on"},
                            "action": "toggle", "hint": "Follows the picture \u00b7 70%"}]}


def again(at, **kw):
    """The receiver as read at `at`: published from inside the host with its own clock, since a reading's stamp is what
    ages it and the host starts several seconds after the scene's list is built."""
    doc = json.dumps(receiver(**kw))
    js = ("(function(){var d=%s; d.updatedAt=Date.now(); var x=new XMLHttpRequest(); x.open('POST','http://127.0.0.1:18731/__publish/receiver');"
          "x.setRequestHeader('Content-Type','application/json'); x.send(JSON.stringify(d)); return 1})()") % doc
    return ("eval", at, js)


@scene("08-input-picker")
def _():
    # HDMI 4 is set to Always, so the tile that says nothing is connected shows, as in the mock
    return [("theme", 0, {"inputs": {"hdmi4": "always"}}),
            ("eval", READY - 500, "root.openLayer('picker')"), ("keys", READY + 300, "right"), ("snap", READY + 1500, "08-input-picker")]


@scene("10-quick-panel")
def _():
    from scenes import AI
    return [("publish", 300, "ai", AI), ("publish", 300, "plugins", AMBIENT_ROSTER), ("publish", 300, "plugin.ambient.tiles", AMBIENT_TILES),
            again(READY - 800, controls=True), ("eval", READY - 500, "root.openLayer('quick')"), ("snap", READY + 1500, "10-quick-panel")]


# the values of mock 24's other tabs
MOCK24 = dict(controls=True, soundModeId="movie", levels={"dialog": 2, "subwoofer": -1, "bass": 0, "treble": 1}, cinemaEq=False, eco="auto",
              zone2={"name": "patio speakers", "power": False, "volumeDb": -50, "volumeText": "-50 dB", "input": "HEOS Music"})


def receiver_page(*after):
    """Settings on Sound, OK on the Receiver row (the one under Sound out) opens the receiver's page; `after` is (ms offset, keys, snapshot name)."""
    steps = [again(300, **MOCK24)] + settings_to("sound", "right", "down", "ok")
    for at, keys, name in after:
        steps.append(again(READY + at - 700, **MOCK24))
        if keys:
            steps.append(("keys", READY + at - 600, keys))
        steps.append(("snap", READY + at, name))
    return steps


@scene("24-settings-receiver")
def _():
    return receiver_page((3000, "down down", "24-settings-receiver"))


@scene("24-settings-receiver-levels")
def _():
    return receiver_page((3000, "up right down", "24-settings-receiver-levels"))


@scene("24-settings-receiver-audyssey")
def _():
    return receiver_page((3000, "up right right down", "24-settings-receiver-audyssey"))


@scene("24-settings-receiver-zones")
def _():
    return receiver_page((3000, "up right right right down", "24-settings-receiver-zones"))


@scene("24-settings-receiver-today")
def _():
    return [again(300)] + settings_to("sound", "right", "down", "ok") + [again(READY + 2300), ("snap", READY + 3000, "24-settings-receiver-today")]
