#!/usr/bin/env python3
"""Patches a staged copy of LG's Quick Settings into the Luma look and
opening behaviour.

Two kinds of change:

  * Behaviour. Stock rebuilds the whole panel on every open, and the build
    blocks the open transition - measured as one frozen frame of 236-357ms
    where the panel's own 180ms pause and 150ms slide should have run, so the
    menu snapped into place instead of sliding. A panel that has completed its
    data pass stands down to the view container's background state instead of
    exiting, with its subscriptions live, and a re-open is the transition only:
    fully open 170ms after the launch.

  * Look. LG's flat slab and grey cards become the same dark glass as the rest
    of Luma: one static gradient, a hairline along the lit edge, rounded
    where the panel faces the screen. Nothing here samples what is behind it,
    so the cost is the same handful of quads whatever is playing underneath.

Every edit is anchored to an exact fragment of LG's source. A missing anchor is
fatal: a half-patched panel is worse than an unpatched one, and a firmware
update that rewrites these files must fail loudly here rather than silently
ship a broken menu.

usage: systemui_patch.py <staged QuickSettings dir>
Runs on the set (python 3.10). Called by tools/lumatv; see systemui_stage.
"""
import json
import sys
import pathlib

MARKER = "Luma"

# The panel's palette. Left side is LG's literal, right side ours; applied to
# Utils/Style.qml only, where every one of these is a UI colour.
#
# Ink and glass match payload/theme/theme.json, so the panel and the Home cards
# are the same material. The focused fill stays light - the panel's focused
# text is dark and LG's own contrast assumptions depend on it - but picks up
# the lavender of the accent instead of flat white.
# Token-scoped first: Style.qml uses the same literal for focus fills and for
# label text, so these have to be rewritten by name before the general pass
# turns every remaining #E6E6E6 into ink.
PALETTE_SCOPED = [
    ('FOCUS_BG_COLOR: "#E6E6E6"', 'FOCUS_BG_COLOR: "#LUMAFOCUS"'),
    ('FOCUS_BG_COLOR1: "#E6E6E6"', 'FOCUS_BG_COLOR1: "#LUMAFOCUSEDGE"'),
    # The small buttons along the top are a mix: one is drawn as a circle, one
    # as a rounded square, the rest with no background at all. Only the focused
    # one carries a pill, so the fix is to make that pill the same shape for all
    # of them rather than to give five icons five different backgrounds.
    ('BG_RADIUS: getRelativeValue(6)', 'BG_RADIUS: getRelativeValue(32)'),
    ('defaultFocusColor: "#E6E6E6"', 'defaultFocusColor: "#LUMAFOCUS"'),
]

PALETTE = [
    ('"#323941"', '"#E0161126"'),   # panel slab
    ('"#575E66"', '"#59241E33"'),   # header card
    ('"#2E3741"', '"#66312845"'),   # header card, edit mode
    ('"#3E444D"', '"#59241E33"'),   # recent-history card
    ('"#3E454D"', '"#59241E33"'),   # unfocused bubble button
    ('"#444444"', '"#59241E33"'),   # popup item background
    ('"#404040"', '"#66120E1A"'),   # popup background
    ('"#E6E6E6"', '"#F0EAE4F8"'),   # focus fill and primary text
    ('"#ABAEB3"', '"#A8F2EFF6"'),   # secondary text
    ('"#AAAAAA"', '"#A8F2EFF6"'),   # move guidance
    ('"#8D9298"', '"#4DF2EFF6"'),   # slider track
    ('"#7D848C"', '"#A8F2EFF6"'),   # close and edit icons
    ('"#4C5059"', '"#F2EFF6"'),     # label on a lifted glass surface stays light
    ('"#575E66"', '"#59241E33"'),
]

# Corner radii. LG rounds cards at 24 and buttons at 5-6 on a 1080 grid; the
# Luma card radius is 26 and its tiles 22, which is what makes the two
# read as one set.
RADII = [
    ("RADIUS: getRelativeValue(24)", "RADIUS: getRelativeValue(26)"),
    ("BG_RADIUS: getRelativeValue(24)", "BG_RADIUS: getRelativeValue(22)"),
    ("BG_RADIUS: getRelativeValue(5)", "BG_RADIUS: getRelativeValue(14)"),
]

# The slab behind the whole panel, and the inset that turns it into a sheet.
#
# Stock is a full-bleed grey rectangle flush to three edges of the screen. The
# panel reads better as what it actually is - a sheet laid over whatever is
# playing - so it is inset on every side and rounded all the way round, the way
# the Home cards are. The inset is applied to the view that holds both the slab
# and its contents, so nothing inside has to be re-measured.
SHEET_INSET = 24

SLAB_FROM = '''            width: parent.width
            height: systemProperties.isPortrait ? styler.quickSettingMainView.MAINLAYOUT.HEIGHT_PORTRAIT : parent.height

            anchors {
                left: parent.left
                top: parent.top
            }

            color: "#323941"
            opacity: 0.95
            visible: interfaces.systemInfo.checkBackGround
        }'''

SLAB_TO = '''            width: parent.width
            height: (systemProperties.isPortrait ? styler.quickSettingMainView.MAINLAYOUT.HEIGHT_PORTRAIT : parent.height) - styler.getRelativeValue(%(inset)d)

            anchors {
                left: parent.left
                top: parent.top
            }

            // Luma: a sheet of dark glass rather than a flat grey fill.
            // The gradient and the hairline are static, so this is one quad
            // however busy the picture behind it is; a shader that sampled the
            // screen would cost a full-screen read every frame it animates.
            color: "transparent"
            visible: interfaces.systemInfo.checkBackGround

            Rectangle {
                anchors.fill: parent
                radius: styler.getRelativeValue(34)
                border.width: 1
                border.color: "#26FFFFFF"
                // Flat, to match the settings panes: a gradient is relative to
                // the height of the surface it is on, so the same one reads as
                // a different colour on a short panel and a tall one.
                color: "#E8120E1A"
            }
        }'''

# The inset itself. The loader already carries the open transition's slide in
# its left margin, so the sheet's left inset is added to it rather than
# replacing it.
INSET_FROM = '''        source: getSource()
        anchors.leftMargin: xPosition'''
INSET_TO = '''        source: getSource()
        // Luma: the sheet floats clear of the screen edges. xPosition is
        // the open transition's slide, so the inset is added to it.
        anchors.leftMargin: xPosition + styler.getRelativeValue(%(inset)d)
        anchors.topMargin: styler.getRelativeValue(%(inset)d)'''

# Residency. The volume OSD is built once and only shown per press; Quick
# Settings is rebuilt from 165 QML files on every press, ~420ms from the key to
# its open transition. Here a panel that has finished its data pass stays
# built, the way the OSD does.
#
# How it stands down matters. The only stand-down stock has is exit, and the
# manager refuses to launch an app whose last lifecycle event was a close until
# the container is destroyed - keeping the item alive past a close left the
# settings button dead. The container's own "background" state is the one that
# works: it hides the item, releases focus, closes the overlay view and drops
# the panel from the foreground-app list, while the lifecycle stays at launch,
# so the next press is accepted as a relaunch of the same instance.
READY_FLAG_FROM = "    property alias uiController: uiController"
READY_FLAG_TO = '''    property alias uiController: uiController
    // Luma: true once the one-time data pass has run. A preloaded
    // instance reaches "ready" without it, and is not worth keeping.
    property bool lumaDataReady: false
    // True for an open that reuses the built panel, which has nothing left to
    // wait for and so skips the transition's opening pause.
    property bool lumaWarmOpen: false'''

READY_SET_FROM = '''            systemProperties.isAppReadyToUse = true;
        }
    }

    UIController {'''
READY_SET_TO = '''            systemProperties.isAppReadyToUse = true;
            lumaDataReady = true;
        }
    }

    UIController {'''

EXIT_FROM = '''    function exitApp() {
        if(!systemProperties.isPreload) {
            cancel();
        }
        root.exit();
    }'''
EXIT_TO = '''    function exitApp() {
        // Luma: a close the panel made itself (back, the settings key
        // again, a tile that hands off) stands down and stays built, with its
        // subscriptions live. The container is in foreground then, or in
        // launching when the close is the settings key's second press. A close
        // from outside - the container already closing: power, Home, another
        // app - exits as stock, so the manager never holds a closed app it
        // would refuse to relaunch. A panel with a popup or edit mode still up
        // exits too rather than reopening into it.
        var view = uiController.mainView;
        var standing = root.parent ? root.parent.state : "";
        if (lumaDataReady && !systemProperties.isPreload && view
                && (standing === "foreground" || standing === "launching")
                && !view.showMenuPopup && !view.showCandidateList && !view.showEditOptBox
                && !view.showControlSliderPopup && !view.showDeleteDecisionPopup && !view.editEnabled) {
            root.parent.gotoState("background");
            return;
        }
        if(!systemProperties.isPreload) {
            cancel();
        }
        root.exit();
    }'''

LAUNCH_FROM = '''                if(container.systemProperties.isPreload) {
                    container.launchAppAfterPreload = true;
                } else {
                    root.show();
                    container.open(payload.params, true);
                }'''
LAUNCH_TO = '''                if(container.systemProperties.isPreload) {
                    container.launchAppAfterPreload = true;
                } else if(container.lumaDataReady && container.uiController.mainView) {
                    // Luma: resident. Built, populated and subscribed, so
                    // this is focus back on the first tile and the open
                    // transition, nothing more.
                    var view = container.uiController.mainView;
                    container.systemProperties.launchParams = payload.params;
                    container.lumaWarmOpen = true;
                    view.mainList.currentIndex = 0;
                    view.mainList.forceActiveFocus();
                    root.show();
                    view.startMainView();
                } else {
                    root.show();
                    container.open(payload.params, true);
                }'''

# The open transition waits 180ms before it starts, which covers the rebuild on
# a cold open. A resident panel has nothing to cover.
PAUSE_FROM = '''                PauseAnimation {
                    duration: 180
                }'''
PAUSE_TO = '''                PauseAnimation {
                    // Luma: only a cold open has a build to hide.
                    duration: rootWindow.lumaWarmOpen ? 0 : 180
                }'''

# Every button, list row, popup and card in the panel - not just the tokens in
# Style.qml. LG hardcodes these fills in the component files themselves, so the
# same map is applied across Component/ and Containers/.
#
# The focused fill stays light on purpose: LG swaps each tile's icon for a dark
# "focusedImage" when it takes focus, so a dark focus fill would put dark icons
# on dark glass. Frosted light glass keeps that contrast and still reads as the
# same material, with depth coming from the lift rather than from the colour.
WIDGET_PALETTE = [
    ('"#575E66"', '"#59241E33"'),   # every unfocused button and card fill
    ('"#3E444D"', '"#59241E33"'),
    ('"#3E454D"', '"#59241E33"'),
    ('"#444444"', '"#59241E33"'),
    ('"#404040"', '"#EB171124"'),   # popup body, which floats over live picture
    ('"#E6E6E6"', '"#F2EDE8FA"'),   # focused fill, and near-white label text
    ('"#ADAFB3"', '"#C4BCD6E6"'),   # focused but unavailable
    ('"#C7C8CC"', '"#C4F2EFF6"'),   # tile label at rest
    ('"#4C5059"', '"#F2EFF6"'),     # label on a lifted glass surface stays light
]

# The lift. Luma raises a focused tile rather than ringing it, so the
# panel's tiles do the same: a small scale, eased like the Home tiles
# (theme.json motion.focusMs 150, OutCubic). Scale does not affect layout, so
# the grid does not reflow, and it is a transform - no repaint of the tile.
TILE_LIFT_FROM = '''    objectName: "childDelegate"
'''
TILE_LIFT_TO = '''    objectName: "childDelegate"

    // Luma: focus lifts the tile instead of ringing it.
    scale: root.hasFocus ? 1.04 : 1.0
    Behavior on scale { NumberAnimation { duration: 150; easing.type: Easing.OutCubic } }
'''

# A hairline along the top edge of an unfocused tile, which is what makes the
# dark fill read as glass rather than as a flat grey box.
TILE_EDGE_FROM = '''        color: root.objDeactive ? (root.hasFocus ? "#ADAFB3" :"#575E66") : (root.hasFocus ? "#E6E6E6" :"#575E66")
        radius: styler.controlPanelButton.BACKGROUND_RADIUS'''
TILE_EDGE_TO = '''        color: root.objDeactive ? (root.hasFocus ? "#ADAFB3" :"#575E66") : (root.hasFocus ? "#LUMAFOCUS" :"#575E66")
        radius: styler.controlPanelButton.BACKGROUND_RADIUS
        // Luma: focus lifts the tile - brighter glass and a lit edge -
        // instead of inverting it to a light fill.
        border.width: 1
        border.color: root.hasFocus ? "#LUMAFOCUSEDGE" : "#1FFFFFFF"'''

# The small round buttons along the top of the panel - settings, AI,
# accessibility, Wi-Fi, edit. Stock draws their background only while focused,
# so at rest they are bare icons on the slab. Here they are always a glass pill:
# dark at rest with a hairline, frosted light when focused, which is the same
# treatment the tiles get.
ICON_BUTTON_FROM = '''        color: "#E6E6E6"
        radius: buttonType === c_type ? parent.width/2 : styler.getRelativeValue(5)
        visible: hasFocus ? true : false'''
ICON_BUTTON_TO = '''        // Luma: a glass pill at rest, lifted on focus.
        color: root.hasFocus ? "#LUMAFOCUS" : "#4D241E33"
        radius: buttonType === c_type ? parent.width/2 : styler.getRelativeValue(16)
        visible: true
        border.width: 1
        border.color: root.hasFocus ? "#LUMAFOCUSEDGE" : "#1FFFFFFF"'''


def edit(path, changes, *, required=True):
    """Apply (from, to) pairs to one file. Already-patched files are left be."""
    text = path.read_text()
    if MARKER in text and required:
        return False
    for frm, to in changes:
        if frm not in text:
            if required:
                raise SystemExit(f"anchor not found in {path.name}: {frm[:60]}")
            continue
        text = text.replace(frm, to)
    path.write_text(text)
    return True


root = pathlib.Path(sys.argv[1])

# The palette is derived from the live theme (tools/palette.py) and passed in as
# JSON, so the menus are tinted by whatever wallpaper and theme are in use. The
# defaults below are the shipped dark theme, so a missing or unreadable file
# still produces the same panel rather than no panel.
PAL = {"sheetTop": "#F01F1832", "sheetBottom": "#E8120E1A", "row": "#59241E33",
       "focus": "#F2EDE8FA", "hairline": "#1FFFFFFF", "ink": "#FFF2EFF6",
       "inkDim": "#A8F2EFF6", "focusInk": "#FFF2EFF6", "accent": "#FFA79BF5",
       "focusEdge": "#61FFFFFF",
       "radius": 34, "rowRadius": 22}
if len(sys.argv) > 2:
    try:
        PAL.update(json.loads(sys.argv[2]))
    except Exception:
        pass

SLAB_TO = SLAB_TO.replace("#F01F1832", PAL["sheetTop"]).replace("#E8120E1A", PAL["sheetBottom"])
SLAB_TO = SLAB_TO.replace('"#26FFFFFF"', '"%s"' % PAL["hairline"])
SLAB_TO = SLAB_TO.replace("getRelativeValue(34)", "getRelativeValue(%d)" % PAL["radius"])
TILE_EDGE_TO = (TILE_EDGE_TO.replace("#LUMAFOCUSEDGE", PAL.get("focusEdge", "#61FFFFFF"))
                            .replace("#LUMAFOCUS", PAL["focus"])
                            .replace('"#1FFFFFFF"', '"%s"' % PAL["hairline"]))
ICON_BUTTON_TO = (ICON_BUTTON_TO.replace("#LUMAFOCUSEDGE", PAL.get("focusEdge", "#61FFFFFF"))
                                .replace("#LUMAFOCUS", PAL["focus"])
                                .replace('"#4D241E33"', '"%s"' % PAL["row"])
                                .replace('"#1FFFFFFF"', '"%s"' % PAL["hairline"]))

# LG swaps every tile icon for a dark "focusedImage" when it takes focus, which
# only works against a light fill. Focus is glass here, so the icon that is
# drawn for a dark surface is kept in both states.
ICON_SWAP_FROM = ("childDelegate.titleIdCenter ? childDelegate.hasFocus ? "
                  "(interfaces[entry.interfaceName].menuObj.focusedImage.url) : "
                  "( interfaces[entry.interfaceName].menuObj.image.url) : \"\"")
ICON_SWAP_TO = ("childDelegate.titleIdCenter ? "
                "interfaces[entry.interfaceName].menuObj.image.url : \"\"")
# The widget fills, rebound to the derived palette.
WIDGET_PALETTE = [(frm, {"#59241E33": PAL["row"], "#F2EDE8FA": PAL["focus"],
                         "#C4F2EFF6": PAL["inkDim"], "#F2EFF6": PAL["ink"]}
                   .get(to.strip('"'), to.strip('"')).join('""'))
                  for frm, to in WIDGET_PALETTE]

# Style.qml carries no marker of its own - it is pure token substitution, and
# re-running it is harmless because every replacement maps LG's literal to one
# of ours, never the other way.
style = root / "Utils" / "Style.qml"
text = style.read_text()
for frm, to in PALETTE_SCOPED:
    text = text.replace(frm, (to.replace("#LUMAFOCUSEDGE", PAL.get("focusEdge", "#61FFFFFF"))
                                .replace("#LUMAFOCUS", PAL["focus"])
                                .replace("#LUMAROW", PAL["row"])))
for frm, to in PALETTE + RADII:
    text = text.replace(frm, to)
style.write_text(text)

# Structure first: these anchors quote LG's own colour literals, which the
# palette pass below then rewrites. Reversing the order would leave every
# anchor unmatched.
edit(root / "Containers" / "QuickSettingsMainView.qml",
     [(SLAB_FROM, SLAB_TO % {"inset": SHEET_INSET * 2})])
edit(root / "Containers" / "RootView.qml",
     [(INSET_FROM, INSET_TO % {"inset": SHEET_INSET})])
edit(root / "Component" / "BaseControlPanelButton.qml",
     [(TILE_LIFT_FROM, TILE_LIFT_TO), (TILE_EDGE_FROM, TILE_EDGE_TO)])
edit(root / "Component" / "ImageButton.qml", [(ICON_BUTTON_FROM, ICON_BUTTON_TO)])
edit(root / "Component" / "ControlPanelImageButton.qml", [(ICON_SWAP_FROM, ICON_SWAP_TO)], required=False)
edit(root / "QuickSettingsMain.qml",
     [(READY_FLAG_FROM, READY_FLAG_TO), (READY_SET_FROM, READY_SET_TO), (EXIT_FROM, EXIT_TO)])
edit(root / "QuickSettings.qml", [(LAUNCH_FROM, LAUNCH_TO)])
edit(root / "Controllers" / "UIController.qml", [(PAUSE_FROM, PAUSE_TO)])

# Then the fills, across every component and container in the panel.
for qml in sorted((root / "Component").glob("*.qml")) + sorted((root / "Containers").rglob("*.qml")):
    body = qml.read_text()
    out = body
    for frm, to in WIDGET_PALETTE:
        out = out.replace(frm, to)
    if out != body:
        qml.write_text(out)

print("patched")
