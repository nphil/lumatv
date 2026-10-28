"""Scenes of UnbuiltScreens: the launch screen and spinner after the mock, Quick Access, the quick panel's
popups and edit mode.

Each registers under the name of the mock it answers (`docs/mocks/shots/NN-name.png`), so a host render sits next to its design.
"""
import json

from scenes import scene, k, js, READY, APPS


@scene("15-spinner-word")
def _():
    # the ring alone for the first second, the word after it
    return [("eval", READY, "root.showSpinner('Loading')"), ("snap", READY + 4000, "15-spinner-word")]


@scene("15-long-press-ring-tile")
def _():
    # the first dock tile as Home lays it out (144, 689, 134 by 134): the ring and its label sit clear of the screen's edge
    return [("eval", READY, "root.showHold({x: 144, y: 689, width: 134, height: 134}, 'Apps', 0.62)"), ("snap", READY + 1500, "15-long-press-ring-tile")]


# Quick Access on, as the owner would have switched it in Settings > Remote & pointer
QA_ON = ("theme", 0, {"remote": {"quickAccess": {"on": True, "slots": {}}}})


@scene("31-quick-access-toast")
def _():
    # digit 2 on Home: the dock's second tile opens, the launch screen covers Home and the toast sits on it
    return [QA_ON, ("eval", READY, "root.key(Qt.Key_2, true, false, 0)"), ("eval", READY + 100, "root.key(Qt.Key_2, false, false, 0)"),
            ("snap", READY + 1500, "31-quick-access-toast")]


@scene("31-quick-access-assign")
def _():
    return [QA_ON, ("eval", READY, "root.accessAsk('4')"), ("snap", READY + 1200, "31-quick-access-assign")]


# ---- the notification view: LG's standard toast and alert in the Luma look, and the input popup with its port said
from scenes_notify import render, ALERT, INPUT  # noqa: E402  (loaded before this file: scenes_*.py in name order)

# What LG's own handlers send for a key that does nothing here ("not available") and for a message of three lines: no icon, no button.
STANDARD_TOAST = {"sourceId": "com.webos.app.home", "type": "standard", "timeout": 30, "iconPath": "", "readable": "", "userData": {"id": "scene"},
                  "message": "This function is not available."}
STANDARD_TOAST_LINES = dict(STANDARD_TOAST, message="Wi-Fi connection lost<br>The TV is trying to join Nate's Home again. Check that the router is on.")
# A plain question with no title and two buttons, the first focused: LG's alert for a setting that needs a confirmation.
STANDARD_ALERT = {"alertInfo": {"sourceId": "com.webos.app.home", "title": "", "message": "Turn off the TV now?", "readable": "", "showTimeout": False, "modal": False,
                                "timeout": 0, "timestamp": 21, "buttons": [{"label": "Cancel", "focus": True, "params": {}}, {"label": "Turn off", "params": {}}]}}


@scene("07-toast-standard", runner=True)
def _(out_dir, material):
    return render("07-toast-standard", out_dir, material, [("toast", 300, STANDARD_TOAST)])


@scene("07-toast-standard-lines", runner=True)
def _(out_dir, material):
    return render("07-toast-standard-lines", out_dir, material, [("toast", 300, STANDARD_TOAST_LINES)])


@scene("06-alert-standard", runner=True)
def _(out_dir, material):
    return render("06-alert-standard", out_dir, material, [("alert", 300, STANDARD_ALERT)])


@scene("18-device-connected-port", runner=True)
def _(out_dir, material):
    return render("18-device-connected-port", out_dir, material, [("input", 300, INPUT)])


# ---- the quick panel's popups and edit mode (mock 29), over Home as the gear opens it
from scenes import AI  # noqa: E402
from scenes_panels import again, AMBIENT_ROSTER, AMBIENT_TILES  # noqa: E402

OK_DOWN = "root.key(Qt.Key_Return, true, false, 0)"
OK_UP = "root.key(Qt.Key_Return, false, false, 0)"


def quick(*after, tiles=None):
    """The quick panel open on Home with the receiver and the plugin tiles published as the helper does; `after` are further steps."""
    theme = {"remote": {"quickPanelEdit": True}}              # popups and editing are behind a switch, off until the TV check
    if tiles is not None:
        theme["quickPanel"] = {"tiles": tiles}
    steps = [("theme", 0, theme)]
    steps += [("publish", 300, "ai", AI), ("publish", 300, "plugins", AMBIENT_ROSTER), ("publish", 300, "plugin.ambient.tiles", AMBIENT_TILES),
              again(READY - 800, controls=True), ("eval", READY - 500, "root.openLayer('quick')")]
    return steps + list(after)


def hold_ok(at):
    """OK held for a second: Home's hold timer (700 ms) fires between the two."""
    return [("eval", at, OK_DOWN), ("eval", at + 1100, OK_UP)]


@scene("29-qp-slider")
def _():
    tiles = ["picture", "receiver", "sleep", "wifi", "plugin.ambient.ambient", "oled", "ai"]
    return quick(("keys", READY + 800, k(*["down"] * 5)), ("keys", READY + 1500, "ok"), ("snap", READY + 2600, "29-qp-slider"), tiles=tiles)


@scene("29-qp-picker")
def _():
    return quick(("keys", READY + 800, "down down"), ("keys", READY + 1500, "ok"), ("snap", READY + 2600, "29-qp-picker"))


@scene("29-qp-edit")
def _():
    return quick(("keys", READY + 800, "down"), *hold_ok(READY + 1500), ("snap", READY + 3400, "29-qp-edit"))


@scene("29-qp-add")
def _():
    # hold picks the tile up, OK drops it, Down to "Add a tile" (after the six tiles), OK opens the list
    return quick(("keys", READY + 800, "down"), *hold_ok(READY + 1500), ("eval", READY + 3000, OK_DOWN), ("eval", READY + 3050, OK_UP),
                 ("keys", READY + 3500, k(*["down"] * 6)), ("keys", READY + 4300, "ok"), ("snap", READY + 5400, "29-qp-add"))
