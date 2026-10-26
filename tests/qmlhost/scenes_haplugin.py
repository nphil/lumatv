"""Scenes of the Home Assistant plugin on the TV: its cards in the At-home lane, its tiles in the quick panel, its notifications in
the drawer, its page in Settings, and what the lane says while it has nothing to show.

Everything the plugin and the helper publish is the fixture the real ones made (`tests/qmlhost/fixtures/ha-plugin.json`, written by
`helper/app/tests/ha_plugin.rs`, which fails when it is not what they make now): a house with a living room (two lamps and a scene),
a doorbell that rang and a notification. Only the clock is the scene's: the times are made relative to now.
"""
import copy

from hafixture import FIXTURE, ID, SECS, topics
from scenes import AI, READY, scene


def publish(at=300, **kw):
    return [("publish", at, topic, doc) for topic, doc in topics(**kw)]


FOCUS_LANE = ("eval", READY - 600, "root.widgetFocusId = 'home'; root.focusLayer = 'widgets'")


@scene("36-home-ha")
def _():
    return publish() + [("snap", READY + 800, "36-home-ha")]


@scene("36-home-ha-focus")
def _():
    # OK enters the chips: the first is the ring (a full-width card that runs), the second the room's
    return publish() + [FOCUS_LANE, ("keys", READY, "ok down"), ("snap", READY + 1400, "36-home-ha-focus")]


@scene("36-home-ha-refused")
def _():
    return publish(link="auth_failed", cards=False, tiles=False, notes=False) + [("snap", READY + 800, "36-home-ha-refused")]


@scene("36-home-ha-offline")
def _():
    return publish(link="offline", cards=False, tiles=False, notes=False) + [("snap", READY + 800, "36-home-ha-offline")]


@scene("36-home-ha-unlinked")
def _():
    return publish(link="unconfigured", cards=False, tiles=False, notes=False) + [FOCUS_LANE, ("snap", READY + 1400, "36-home-ha-unlinked")]


@scene("10-quick-panel-ha")
def _():
    from scenes_panels import again
    return [("publish", 300, "ai", AI)] + publish() + [again(READY - 800, controls=True), ("eval", READY - 500, "root.openLayer('quick')"),
                                                      ("snap", READY + 1500, "10-quick-panel-ha")]


@scene("13-notifications-ha")
def _():
    return publish() + [("eval", READY - 900, "root.openLayer('drawer')"), ("snap", READY + 1200, "13-notifications-ha")]



def page(*keys, link="connected", plugin="connected", reason=None):
    """Home's lane, OK on it, then `keys` inside the page: Settings at the Home Assistant page."""
    steps = publish(link=link, plugin=plugin, reason=reason, cards=False, tiles=False, notes=False) if plugin != "connected" or link != "connected" \
        else publish(cards=False, tiles=False, notes=False)
    steps += [FOCUS_LANE, ("keys", READY, "ok")]
    if keys:
        steps.append(("keys", READY + 2200, " ".join(keys)))
    return steps


TABS = ["connection", "room", "doorbell", "presence", "alerts"]
for i, tab in enumerate(TABS):
    @scene("30-settings-home-assistant" if i == 0 else "30-settings-home-assistant-%s" % tab)
    def _(i=i, tab=tab):
        name = "30-settings-home-assistant" if i == 0 else "30-settings-home-assistant-%s" % tab
        # the page opens with the first row focused; Up reaches the tab bar, Right goes along it, Down comes back into the tab
        keys = ["up"] + ["right"] * i + ["down"] if i else []
        return page(*keys) + [("snap", READY + 3800 if i else READY + 2600, name)]


@scene("30-settings-home-assistant-unlinked")
def _():
    return page(link="unconfigured") + [("snap", READY + 2400, "30-settings-home-assistant-unlinked")]


@scene("30-settings-home-assistant-refused")
def _():
    return page(link="auth_failed") + [("snap", READY + 2400, "30-settings-home-assistant-refused")]


@scene("30-settings-home-assistant-off")
def _():
    # not switched on: no plugin in the roster at all
    return [("publish", 300, "plugins", {"plugins": [], "firstParty": copy.deepcopy(FIXTURE["topics"]["plugins"]["firstParty"]), "problem": None}),
            FOCUS_LANE, ("keys", READY, "ok"), ("snap", READY + 2400, "30-settings-home-assistant-off")]
