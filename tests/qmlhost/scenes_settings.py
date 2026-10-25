"""Scenes of SettingsSlice: every Settings page, one scene per tab (the first tab of a page that has a mock keeps the mock's name).

The page is opened from Home by the way the gear does, the rail taken to its section, the D-pad moved into the page and along the
tab bar, and the first row of the tab focused (a page shows its first stop focused, as it does when a person enters it).
`30-settings-ambient` is the one mock whose page only these scenes draw; the others add the tabs the mocks do not show.
"""
from scenes import scene, k, READY

RAIL = ["picture", "sound", "inputs", "network", "ambient", "look", "remote", "ai", "system"]
TABS = {
    "picture": ["Mode", "Brightness", "Colour", "Advanced"],
    "sound": ["Output", "Mode", "Advanced"],
    "inputs": ["Inputs", "HDMI", "Devices"],
    "network": ["Wi-Fi", "Wired", "Details"],
    "ambient": ["Light", "Layout", "Colour"],
    "look": ["Look", "Home", "Screensaver", "Standby"],
    "system": ["General", "Power", "OLED care", "Accessibility", "About"],
}


def page(section, tab=0, *after):
    """Steps that leave `section`'s tab `tab` on screen with its first row focused, then `after` (keys)."""
    steps = [("eval", 1500, "root.openSettings()")]
    if RAIL.index(section):
        steps.append(("keys", READY, k(*["down"] * RAIL.index(section))))
    keys = ["right"]
    if tab:
        keys += ["up"] + ["right"] * tab + ["down"]
    steps.append(("keys", READY + 900, k(*keys)))
    if after:
        steps.append(("keys", READY + 1700, k(*after)))
    return steps


def register(section, tab, name, *after, at=2600):
    @scene(name)
    def _():
        return page(section, tab, *after) + [("snap", READY + at, name)]


# the tabs the mocks do not show: <mock name>-<tab>
for section, mock in (("picture", "01-settings-picture"), ("sound", "02-settings-sound"), ("inputs", "30-settings-inputs"), ("network", "30-settings-network"),
                      ("look", "30-settings-home"), ("system", "30-settings-system")):
    for i, tab in enumerate(TABS[section]):
        if i:
            register(section, i, "%s-%s" % (mock, tab.lower().replace(" ", "-")))

# the ambient page: Light is the mock
register("ambient", 0, "30-settings-ambient")
register("ambient", 1, "30-settings-ambient-layout")
register("ambient", 2, "30-settings-ambient-colour")

# the screensaver tab with the art options showing (style Art), and the remote page entered (the mock draws its first row focused)
register("look", 2, "30-settings-home-screensaver-art", "right", "ok")


@scene("03-settings-remote-focused")
def _():
    return page("remote") + [("snap", READY + 2600, "03-settings-remote-focused")]


# the remote's app keys: the rows (the fourth stop below the pointer rows is the first key; the focus on the last one scrolls all four in),
# and the chooser opened for the Netflix key
@scene("03-settings-remote-appkeys")
def _():
    return page("remote", 0, *["down"] * 7) + [("snap", READY + 2600, "03-settings-remote-appkeys")]


@scene("03-settings-appkey-chooser")
def _():
    return page("remote", 0, *["down"] * 4, "ok") + [("snap", READY + 2800, "03-settings-appkey-chooser")]


@scene("03-settings-appkey-chosen")
def _():
    # the Prime Video key set to "Does nothing": the chooser is left and the row says Nothing
    return page("remote", 0, *["down"] * 5, "ok", "down", "ok") + [("snap", READY + 3000, "03-settings-appkey-chosen")]


# Glass depth (Home & look › Look): the row focused at the shipped step, and the panes at each of the other two
def depth(name, level):
    @scene(name)
    def _():
        return [("theme", 0, {"depth": {"level": level}})] + page("look", 0, *["down"] * 4) + [("snap", READY + 2600, name)]


depth("30-settings-home-glass-normal", "normal")
depth("30-settings-home-glass-flat", "flat")
depth("30-settings-home-glass-deep", "deep")
