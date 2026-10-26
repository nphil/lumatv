"""What the Home Assistant tests of the QML host share: the real plugin's topics fed to the fake helper, and the base class.

The topics are `tests/qmlhost/fixtures/ha-plugin.json`, written by `helper/app/tests/ha_plugin.rs` from the real plugin and helper
(that test fails when the file is not what they make now).
"""
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "qmlhost"))

import hafixture  # noqa: E402
from test_plugins_surfaces import Surfaces, publish  # noqa: E402

HUB = ("function hub() { return Qt.createQmlObject('import QtQuick 2.4\\nimport \"' + root.moduleDirUrl + 'kit\"\\n"
       "QtObject { property string st: PluginHub.haState; property var none: PluginHub.laneNone; property string words: PluginHub.haWords;"
       " property var cards: PluginHub.homeCards; function glance(c) { return PluginHub.glance(c) } }', root) }")
FOCUS = "root.widgetFocusId = 'home'; root.focusLayer = 'widgets';"


def feed(at=300, **kw):
    """The plugin's topics as the helper publishes them, at `at` ms."""
    return [arg for topic, doc in hafixture.topics(**kw) for arg in publish(at, topic, doc)]


class HaCase(Surfaces):
    MINE = Surfaces.MINE + ("PageHomeAssistant.qml", "PluginRows.js", "QuickPanel.qml", "PluginTiles.qml")

    def own_warnings(self, out, allow=()):
        # the weather card is another slice's: a change of it that has not landed whole says so through Widget.qml
        return super().own_warnings(out, allow + ("for 'weather'",))

    def plugin_posts(self, out):
        return [(path, body) for path, body in self.posts(out) if path.startswith("/v1/plugins/")]

    def settings_posts(self, out):
        return [body for path, body in self.posts(out) if path == "/v1/settings/home-assistant"]


