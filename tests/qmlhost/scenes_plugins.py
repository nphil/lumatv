"""Scenes of PluginsSlice: the Notification Centre (13), plugin cards on Home, the screensaver (21) and art mode.

    python3 tests/qmlhost/scenes.py --only 13-notifications,36-home-plugin-cards,21-screensaver-clock --out /tmp/plugins-shots

What the helper would publish is published by the fake helper: the roster (`plugins`, with the helper's own plugins under
`firstParty`), a plugin's cards and notifications (`plugin.<id>.cards|notifications`), the AI access audit. What the TV itself said
(NoteLog) and the surfaces that Home's own wiring does not open yet are driven from inside the host: a panel is built with the
same component Layers builds it from, and NoteLog is posted to through a throwaway object in the host's engine (the singleton is
one per engine, so it is the one the panel reads).
"""
import json
import time

from scenes import AI, AUDIT, NOW, READY, js, scene

SECS = int(time.time())


def plugin(id, name, first_party=False, state="connected"):
    return {"id": id, "name": name, "version": "1", "class": "S", "state": state, "registered": state == "connected",
            **({"firstParty": True} if first_party else {})}


FIRST_PARTY = [plugin("weather", "Weather", True), plugin("news", "News", True), plugin("clock", "Clock", True),
               plugin("ambient", "Ambient light", True)]
HOME_ASSISTANT = plugin("homeassistant", "Home Assistant")
ROSTER = {"plugins": [HOME_ASSISTANT], "firstParty": FIRST_PARTY, "problem": None}


def note(id, title, body, ago_s, actions=(), icon="base.icons/message"):
    """A notification as the broker publishes it: the channel's icon and name beside the plugin's own post."""
    return {"id": id, "channel": "events", "channelTitle": "Events", "channelIcon": icon, "revision": 1, "receivedAt": SECS - ago_s, "expiresAt": None,
            "post": {"id": id, "channel": "events", "title": title, "body": body, "actions": list(actions)}}


HA_NOTES = {"items": [
    note("lock", "Front door locked", "Locked by the night routine.", 1500, icon="base.icons/lock"),
    note("driveway", "Driveway: person seen", "Kestrel saved a clip. Open the camera to watch it.", 5000, icon="base.icons/camera")]}


def card(id, title, facts=None, icon="base.icons/lamp", role="accent", size="half", actions=(), kind="facts", **extra):
    content = {"type": kind, "title": title, "icon": icon, "role": role}
    if kind == "facts":
        content["facts"] = [{"label": a, "value": b} for a, b in facts]
    content.update(extra)
    return {"id": id, "revision": 1, "createdAt": SECS - 600, "expiresAt": None,
            "card": {"id": id, "size": size, "priority": "new", "content": content, "open": {"view": "main"}, "actions": list(actions)}}


def posts_to_note_log(*items):
    """JS that posts to NoteLog from a throwaway object in the host's engine."""
    calls = "".join("NoteLog.post(%s);" % json.dumps(i) for i in items)
    return ("(function(){Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'kit\"\\nQtObject { Component.onCompleted: {%s} }', root); return 1})()"
            % calls.replace("\\", "\\\\").replace("'", "\\'"))


def panel(path, props="{z: 70}", after="o.open()"):
    """JS that builds `panels/<path>` from the module directory, the way Layers does, and starts it."""
    return ("(function(){var c = Qt.createComponent(root.moduleDirUrl + '%s'); var o = c.createObject(root, %s);"
            " if (!o) return c.errorString(); %s; return 'ok'})()" % (path, props, after))


AI_PAUSED = json.loads(json.dumps(AI))
AI_PAUSED["assistants"][0]["paused"] = True
TV_UPDATE = {"source": "TV", "icon": "download", "title": "TV update ready", "body": "webOS 10.2.3, 412 MB. Installs only when you say so, from Settings > System.",
             "time": NOW - 26 * 3600 * 1000}


def drawer_steps(extra=(), ready=READY):
    return [("publish", 300, "ai", AI_PAUSED), ("publish", 300, "__audit", AUDIT), ("publish", 400, "plugins", ROSTER),
            ("publish", 400, "plugin.homeassistant.notifications", HA_NOTES),
            ("eval", ready - 1500, posts_to_note_log(TV_UPDATE)), ("eval", ready - 900, panel("panels/Drawer.qml", "{z: 70, objectName: 'drawer'}")),
            *extra]


@scene("13-notifications")
def _():
    return drawer_steps() + [("snap", READY + 1200, "13-notifications")]


@scene("13-notifications-empty")
def _():
    return [("eval", READY - 900, panel("panels/Drawer.qml")), ("snap", READY + 800, "13-notifications-empty")]


# ---- what the plugins publish for Home and the screensaver
WEATHER_CARD = {"items": [card("weather", "Weather \u00b7 Atlanta", [("Now", "46\u00b0"), ("Conditions", "Clear"), ("Today", "61\u00b0 / 44\u00b0"),
                                                                    ("Details", "Feels like 44\u00b0 \u00b7 55% humidity \u00b7 6 mph")],
                                icon="base.icons/moon", role="info", actions=[{"action": "refresh", "label": "Refresh"}])]}
HOME_CARDS = {"items": [
    card("door", "Front door", [("State", "Locked")], icon="base.icons/lock", role="positive"),
    card("living", "Living room", [("Lights", "3 on")], icon="base.icons/lightbulb", role="warning", actions=[{"action": "off", "label": "Turn off"}]),
    card("climate", "72\u00b0 cooling", [("Set to", "70\u00b0")], icon="base.icons/sun", role="info"),
    card("kitchen", "Kitchen", [("Playing", "Sonos")], icon="base.icons/music", role="accent"),
    card("garage", "Garage door", [("State", "Open for 12 min")], icon="base.icons/info", role="danger", size="full"),
    card("pool", "Pool", [("Water", "81\u00b0")], icon="base.icons/leaf", role="positive")]}
SAVER_STATE = {"on": True, "state": "Screen Saver", "since": NOW, "source": "luna-subscribe", "error": None}


def plugin_topics(*, home=True, weather=True):
    steps = [("publish", 300, "plugins", ROSTER)]
    if weather:
        steps.append(("publish", 300, "plugin.weather.cards", WEATHER_CARD))
    if home:
        steps.append(("publish", 300, "plugin.homeassistant.cards", HOME_CARDS))
    return steps


def saver(style, preview, extra_theme=None, power_state=True):
    theme = {"screensaver": {"style": style, **(extra_theme or {})}}
    steps = [("theme", 0, theme)] + plugin_topics()
    if power_state:
        steps.append(("publish", 300, "screen", SAVER_STATE))
    steps.append(("eval", READY - 1500, panel("panels/Screensaver.qml", "{z: 95, objectName: 'saver'}", after=preview)))
    return steps


@scene("21-screensaver-clock")
def _():
    return saver("clock", "o.preview(0.36, 0.58)") + [("snap", READY + 1500, "21-screensaver-clock")]


@scene("21-screensaver-art")
def _():
    return saver("art", "o.preview(undefined, undefined, 'great-wave')") + [("snap", READY + 2500, "21-screensaver-art")]


@scene("21-screensaver-art-portrait")
def _():
    # a print that is not wider than it is tall fits by its height
    return saver("art", "o.preview(undefined, undefined, 'starry-night')", {"art": {"caption": True}}) + [("snap", READY + 2500, "21-screensaver-art-starry")]


@scene("21-screensaver-clock-bare")
def _():
    # the glance line off: the time and the date on black
    return saver("clock", "o.preview(0.70, 0.30)", {"glance": False}) + [("snap", READY + 1500, "21-screensaver-clock-bare")]


def home_at_home(extra=()):
    return plugin_topics() + [("eval", READY - 600, "root.widgetFocusId = 'home'; root.focusLayer = 'widgets'")] + list(extra)


@scene("36-home-plugin-cards")
def _():
    return plugin_topics() + [("snap", READY + 800, "36-home-plugin-cards")]


@scene("36-home-plugin-cards-focus")
def _():
    return home_at_home([("keys", READY, "ok right down"), ("snap", READY + 1400, "36-home-plugin-cards-focus")])
