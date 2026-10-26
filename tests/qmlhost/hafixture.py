"""The Home Assistant plugin's topics as the real plugin and helper make them, for the QML host's scenes and tests.

`tests/qmlhost/fixtures/ha-plugin.json` is written by `helper/app/tests/ha_plugin.rs` (which fails when it is not what the plugin and
the helper make now): a house with a living room (two lamps and a scene), a doorbell that rang and a notification. Only the clock
is the host's: the times are made relative to now.
"""
import copy
import json
import os
import time

HERE = os.path.dirname(os.path.abspath(__file__))
with open(os.path.join(HERE, "fixtures", "ha-plugin.json")) as f:
    FIXTURE = json.load(f)

SECS = int(time.time())
ID = "home-assistant"


def topics(*, link="connected", plugin="connected", reason=None, cards=True, tiles=True, notes=True, ago=(1500, 5000)):
    """What the helper publishes while the plugin is `plugin` (connected|waiting|inert) and the lease is `link`
    (unconfigured|connecting|connected|offline|auth_failed): (topic, document) pairs."""
    t = copy.deepcopy(FIXTURE["topics"])
    roster = t["plugins"]
    mine = roster["plugins"][0]
    mine["state"] = plugin
    mine["registered"] = plugin == "connected"
    if reason:
        mine["reason"] = reason
    out = [("plugins", roster)]
    state = {"unconfigured": "Home Assistant is not linked yet", "connecting": "Connecting to Home Assistant", "connected": t["ha.link"]["detail"],
             "offline": "Cannot reach Home Assistant: connection refused", "auth_failed": "Home Assistant did not accept the access token"}[link]
    out.append(("ha.link", dict(t["ha.link"], state=link, detail=state, **({"address": None, "token": False} if link == "unconfigured" else {}))))
    if plugin == "connected":
        out.append(("plugin.%s.facts" % ID, {"facts": {"link": state, **({"summary": t["plugin.%s.facts" % ID]["facts"]["summary"]} if link == "connected" else {})}}))
        for item in t["plugin.%s.cards" % ID]["items"]:
            item["createdAt"] = SECS - 600
        for item, back in zip(t["plugin.%s.notifications" % ID]["items"], ago):
            item["receivedAt"] = SECS - back
        out.append(("plugin.%s.cards" % ID, t["plugin.%s.cards" % ID] if cards else {"items": []}))
        out.append(("plugin.%s.tiles" % ID, t["plugin.%s.tiles" % ID] if tiles else {"items": []}))
        out.append(("plugin.%s.notifications" % ID, t["plugin.%s.notifications" % ID] if notes else {"items": []}))
    return out
