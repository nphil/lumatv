"""Scenes of the Home widgets: each card at the sizes a person can give it, Home in the arrangements the owner chose, Settings' Home
widgets page and the layout editor.

    python3 tests/qmlhost/scenes.py --only 37-home-weather-sizes,37-home-halves --out /tmp/widgets-shots

A gallery scene is Home with no dock and a layout of one kind of card at several sizes at once (`("layout", 0, {...})` is the owner's
layout.json over the shipped one), so one render shows how a card fills a narrow, a half-width, a wide, a short and a tall place. The
card ids name the size: `c6x4` is six columns by four rows of the sixteen. Every scene renders dark and light.
"""
import json
import time

from scenes import NOW, READY, k, scene

SECS = int(time.time())


def later():
    """A reading's stamp after the one the fake helper starts with, whenever this scene runs (a stamp up to five minutes ahead is taken as now)."""
    return int(time.time() * 1000) + 120000
GRID = {"rows": 16}                       # sixteen equal rows (the layout editor's grid); the shipped file's pixel rows are three


def card(type, col, row, span, rows, **extra):
    return {"id": "c%dx%d" % (span, rows) + ("" if not extra.get("n") else "-%d" % extra.pop("n")), "type": type, "col": col, "row": row, "span": span, "rows": rows, **extra}


def gallery(*cards):
    """The owner's layout: sixteen rows and these cards, no dock."""
    return {"grid": dict(GRID), "widgets": list(cards)}


# ---- readings the helper would publish
def weather_topic(place="Atlanta"):
    days = [("Today", "sun", 71, 51), ("Mon", "cloud", 73, 51), ("Tue", "rain", 62, 48), ("Wed", "partly", 66, 56), ("Thu", "drizzle", 59, 44),
            ("Fri", "sun", 68, 42), ("Sat", "storm", 74, 55)]
    doc = {"schema": 1, "fetchedAt": later(), "staleAt": later() + 900000, "expiresAt": later() + 21600000, "zip": "30311", "units": "fahrenheit", "place": place,
           "display": {"place": place, "temp": "61\u00b0", "cond": "Clear", "stats": "Feels like 57\u00b0 \u00b7 53% humidity \u00b7 6 mph", "nowKind": "moon",
                       "days": [{"day": d, "kind": kd, "hi": "%d\u00b0" % hi, "lo": "%d\u00b0" % lo} for d, kd, hi, lo in days]}}
    return {"doc": doc, "status": {"failures": 0, "error": None, "lastAttemptAt": NOW}, "state": "fresh", "ageMs": 0}


def news_topic():
    items = [{"tag": "WORLD", "src": "BBC News", "title": "Denmark says Russian warship fired flares at military helicopter",
              "summary": "The Danish prime minister has accused Russia of \"reckless\" behaviour, while the Kremlin is yet to comment. "
                         "The helicopter was on a routine patrol over the Baltic when the flares were fired, the Danish defence ministry said.",
              "link": "https://example.invalid/a", "img": "", "pub": "Sat, 24 Oct 2026 22:30:00 GMT", "pubMs": 1792884600000},
             {"tag": "U.S.", "src": "BBC News", "title": "A second story", "summary": "", "link": "", "img": "", "pub": "", "pubMs": None}]
    doc = {"schema": 1, "fetchedAt": later(), "staleAt": later() + 900000, "expiresAt": later() + 21600000, "feeds": ["x", "y"], "items": items}
    return {"doc": doc, "status": {"failures": 0, "error": None, "lastAttemptAt": NOW}, "state": "fresh", "ageMs": 0}


def plugin(id, name, first_party=False):
    return {"id": id, "name": name, "version": "1", "class": "S", "state": "connected", "registered": True, **({"firstParty": True} if first_party else {})}


ROSTER = {"plugins": [plugin("homeassistant", "Home Assistant")],
          "firstParty": [plugin("weather", "Weather", True), plugin("news", "News", True), plugin("clock", "Clock", True)], "problem": None}


def plugin_card(id, title, facts, icon, role, size="half", actions=()):
    content = {"type": "facts", "title": title, "icon": icon, "role": role, "facts": [{"label": a, "value": b} for a, b in facts]}
    return {"id": id, "revision": 1, "createdAt": SECS - 600, "expiresAt": None,
            "card": {"id": id, "size": size, "priority": "new", "content": content, "open": {"view": "main"}, "actions": list(actions)}}


HOME_CARDS = {"items": [
    plugin_card("living", "Living room", [("Lights", "3 on")], "base.icons/lightbulb", "warning", actions=[{"action": "off", "label": "Turn off"}]),
    plugin_card("climate", "72\u00b0 cooling", [("Set to", "70\u00b0")], "base.icons/sun", "info"),
    plugin_card("door", "Front door", [("State", "Locked")], "base.icons/lock", "positive"),
    plugin_card("kitchen", "Kitchen", [("Playing", "Sonos")], "base.icons/music", "accent"),
    plugin_card("garage", "Garage door", [("State", "Open for 12 min")], "base.icons/info", "danger", size="full"),
    plugin_card("pool", "Pool", [("Water", "81\u00b0")], "base.icons/leaf", "positive"),
    plugin_card("porch", "Porch light", [("Lights", "Off")], "base.icons/lightbulb", "accent"),
    plugin_card("mail", "Mailbox", [("State", "Opened 2 h ago")], "base.icons/info", "info")]}


def feeds(*extra):
    return [("publish", 300, "weather", weather_topic()), ("publish", 300, "news", news_topic()), ("publish", 300, "plugins", ROSTER),
            ("publish", 300, "plugin.homeassistant.cards", HOME_CARDS), *extra]


AT = 4200                                       # the cards are built and have their readings


def shot(name, layout, *extra, theme=None):
    steps = [("layout", 0, layout)] + ([("theme", 0, theme)] if theme else []) + feeds() + list(extra)
    return steps + [("snap", AT, name)]


# ---- weather: a narrow card, a half, a wide one, a short one, a tall one
@scene("37-home-weather-sizes")
def _():
    return shot("37-home-weather-sizes", gallery(
        card("weather", 0, 0, 3, 9), card("weather", 3, 0, 6, 9), card("weather", 9, 0, 3, 4), card("weather", 9, 4, 3, 5),
        card("weather", 0, 9, 6, 4), card("weather", 6, 9, 6, 4), card("weather", 0, 13, 2, 3), card("weather", 2, 13, 4, 3, n=2)))


@scene("37-home-weather-wide")
def _():
    return shot("37-home-weather-wide", gallery(card("weather", 0, 0, 12, 9), card("weather", 0, 9, 12, 5), card("weather", 0, 14, 8, 2)))


# ---- clock, analogue and digital
@scene("37-home-clock-sizes")
def _():
    return shot("37-home-clock-sizes", gallery(
        card("clock", 0, 0, 4, 4), card("clock", 4, 0, 6, 4), card("clock", 10, 0, 2, 3), card("clock", 0, 4, 12, 4), card("clock", 0, 8, 6, 8),
        card("clock", 6, 8, 3, 4), card("clock", 9, 8, 3, 8, n=2), card("clock", 6, 12, 3, 4, n=2)), theme={"clock": {"style": "analog"}})


@scene("37-home-clock-digital-sizes")
def _():
    return shot("37-home-clock-digital-sizes", gallery(
        card("clock", 0, 0, 4, 4), card("clock", 4, 0, 6, 4), card("clock", 10, 0, 2, 3), card("clock", 0, 4, 12, 4), card("clock", 0, 8, 6, 8),
        card("clock", 6, 8, 3, 4), card("clock", 9, 8, 3, 8, n=2), card("clock", 6, 12, 3, 4, n=2)), theme={"clock": {"style": "digital"}})


# ---- at home
@scene("37-home-at-home-sizes")
def _():
    return shot("37-home-at-home-sizes", gallery(
        card("homeassistant", 0, 0, 4, 5, title="At home"), card("homeassistant", 4, 0, 6, 5, title="At home"), card("homeassistant", 10, 0, 2, 5, title="At home"),
        card("homeassistant", 0, 5, 6, 4, title="At home"), card("homeassistant", 6, 5, 6, 8, title="At home"), card("homeassistant", 0, 9, 3, 3, title="At home"),
        card("homeassistant", 3, 9, 3, 7, title="At home"), card("homeassistant", 0, 12, 3, 4, title="At home", n=2), card("homeassistant", 6, 13, 6, 3, title="At home", n=2)))


# ---- news
@scene("37-home-news-sizes")
def _():
    return shot("37-home-news-sizes", gallery(
        card("news", 0, 0, 5, 9), card("news", 5, 0, 7, 6), card("news", 5, 6, 4, 3), card("news", 9, 6, 3, 3, n=2), card("news", 0, 9, 12, 5), card("news", 0, 14, 4, 2, n=2)))


# ---- Home as the owner arranged it: two halves with the dock under them; the shipped layout in the sixteen-row grid
def halves(dock_rows=7):
    return {"grid": dict(GRID), "widgets": [
        card("clock", 0, 0, 6, 4, id="clock"), card("homeassistant", 0, 4, 6, 5, id="home", title="At home"), card("weather", 6, 0, 6, 9, id="weather"),
        {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": dock_rows, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]}


@scene("37-home-halves")
def _():
    return shot("37-home-halves", halves())


def thirds():
    """The shipped arrangement, news back, as the editor would save it after it converts the three pixel rows to sixteen."""
    return {"grid": dict(GRID), "widgets": [
        card("clock", 0, 0, 4, 4, id="clock"), card("homeassistant", 0, 4, 4, 5, id="home", title="At home"), card("news", 4, 0, 5, 9, id="news"),
        card("weather", 9, 0, 3, 9, id="weather"), {"id": "dock", "type": "dock", "col": 0, "row": 9, "span": 12, "rows": 7, "tile": 120, "gap": 16, "padY": 40, "fade": 110}]}


@scene("37-home-thirds")
def _():
    return shot("37-home-thirds", thirds())


# ---- Settings > Home & look > Home widgets: the Home tab's first row opens the page
def widgets_page(*after, layout=None):
    steps = [("layout", 0, layout)] if layout else []
    steps += [("eval", 1500, "root.openSettings()"), ("keys", READY, k(*["down"] * 5)), ("keys", READY + 900, k("right", "up", "right", "down")),
              ("keys", READY + 1700, k("ok"))]
    if after:
        steps.append(("keys", READY + 2500, k(*after)))
    return steps


@scene("38-settings-home-widgets")
def _():
    return widgets_page(layout=halves()) + [("snap", READY + 3300, "38-settings-home-widgets")]


@scene("38-settings-home-widgets-order")
def _():
    return widgets_page("up", "right", layout=halves()) + [("snap", READY + 3500, "38-settings-home-widgets-order")]


# ---- the layout editor
EDIT = READY + 300


def editing(*after, layout=None, card="clock"):
    """Home with the editor open on `card`, then keys at EDIT + 900 ms and on."""
    steps = [("layout", 0, layout or halves())] + feeds()
    steps.append(("eval", EDIT, "root.editHome('%s')" % card))
    for i, keys in enumerate(after):
        steps.append(("keys", EDIT + 900 + 700 * i, keys))
    return steps, EDIT + 900 + 700 * len(after) + 700


@scene("38-home-edit")
def _():
    steps, at = editing()
    return steps + [("snap", at, "38-home-edit")]


@scene("38-home-edit-move")
def _():
    steps, at = editing(k("ok"), k("down"))                 # picked up and moved a cell down: the home card is under it, so it is turned down
    return steps + [("snap", at, "38-home-edit-move")]


@scene("38-home-edit-resize")
def _():
    steps, at = editing(k("down", "ok", "ok", "left"))      # the home card, picked up, sized, one column narrower
    return steps + [("snap", at, "38-home-edit-resize")]


@scene("38-home-edit-add")
def _():
    steps, at = editing(k("up", "left", "left", "ok"))      # up to the bar, left to Add, open the list
    return steps + [("snap", at, "38-home-edit-add")]


def free_thirds():
    """The thirds with the news card hidden: a hole in the middle, where the editor's cells show."""
    layout = thirds()
    for e in layout["widgets"]:
        if e["id"] == "news":
            e["hidden"] = True
    return layout


@scene("38-home-edit-free")
def _():
    steps, at = editing(layout=free_thirds())
    return steps + [("snap", at, "38-home-edit-free")]


@scene("38-home-edit-added")
def _():
    steps, at = editing(k("up", "ok"), k("down", "down", "down", "ok"), layout=free_thirds())     # the bar, the list, News, picked up where it fits
    return steps + [("snap", at, "38-home-edit-added")]


@scene("38-home-edit-sure")
def _():
    steps, at = editing(k("ok", "right", "ok", "ok"), k("back"), layout=free_thirds())          # moved a column and put down, then Back: it asks
    return steps + [("snap", at, "38-home-edit-sure")]
