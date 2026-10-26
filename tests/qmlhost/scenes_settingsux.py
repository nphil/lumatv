"""Scenes of SettingsUX: the help behind a row's info mark (Settings, minimal text).

A row with a `help` carries a small info mark after its name; the focused one says how to open it (Hold OK), and OK held for
`motion.holdMs` opens the sheet under the row. The host's clock drives the hold: the press is sent through Home's own `key()`,
the sheet is grabbed 1.2 s later, the release follows.
"""
from scenes import scene, READY, AI, AUDIT
from scenes_settings import page

PRESS = "root.key(Qt.Key_Return, true, false, 0)"
RELEASE = "root.key(Qt.Key_Return, false, false, 0)"


def held(name, section, tab, *keys, at=2500, before=()):
    """The page on `section`'s `tab`, the focus taken along `keys`, then OK held until the help sheet is up."""
    @scene(name)
    def _():
        return list(before) + page(section, tab, *keys) + [("eval", READY + at, PRESS), ("snap", READY + at + 1300, name), ("eval", READY + at + 1500, RELEASE)]


def focused(name, section, tab, *keys, at=2600):
    @scene(name)
    def _():
        return page(section, tab, *keys) + [("snap", READY + at, name)]


# Picture, Mode tab: the strip, OLED pixel brightness, AI Picture Pro, then Aspect ratio (the rows with help show the mark; the focused one says how to open it)
focused("30-settings-help-mark", "picture", 0, "down", "down", "down")
held("30-settings-help", "picture", 0, "down", "down", "down")
# the AI access page: its rows are LuRows with help too, and the page answers the held OK itself (the focus goes down to Volume limit)
held("04-settings-ai-access-help", "ai", 0, "down", before=[("publish", 300, "ai", AI), ("publish", 300, "__audit", AUDIT)])
