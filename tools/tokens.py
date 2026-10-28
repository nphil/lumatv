#!/usr/bin/env python3
"""Resolves the one token source, theme.json, for everything that is not the compositor's own QML.

usage: tokens.py <shipped theme.json> <theme.json> qml|css|js

The compositor reads theme.json directly (LumaHome merges the user's file over the shipped one
and hands the result to kit/Lu.qml). This tool applies the same merge and resolves the tokens for
the consumers that cannot:

  css  generated web CSS (--lu-* custom properties, named as in Lucent's tokens.css, plus the
       Sandstone variables the settings web apps read). Loaded after payload/webapps/lumatv.css,
       which carries no colour of its own.
  qml  JSON of the resolved settings-sheet palette, read by tools/systemui_patch.py to recolour
       Quick Settings.
  js   Tokens.js, a .pragma library for QML in other processes to import by file URL
       (import "file:///var/lib/lumatv/theme/Tokens.js" as Lu): both materials, the type roles,
       radii, spacing and motion, resolved the same way.

The menus are always drawn on the dark material, over a wallpaper or over live picture, so their
sheet is derived from it (see sheet below) rather than taken from the Home glass, which is sheer.
Runs on the set (python 3.10) and on the workstation. Called by tools/lumatv.
"""
import json
import os
import sys


def merge(base, over):
    """Objects merge key by key, everything else is replaced: LumaHome.mergeDeep."""
    if over is None:
        return base
    if not isinstance(over, dict) or not isinstance(base, dict):
        return over
    out = dict(base)
    for k, v in over.items():
        out[k] = merge(base.get(k), v) if k in base else v
    return out


def parse(colour):
    """#AARRGGBB or #RRGGBB -> (r, g, b, a) with a in 0..1. QML's order."""
    c = (colour or "").lstrip("#")
    if len(c) == 8:
        a, r, g, b = (int(c[i:i + 2], 16) for i in (0, 2, 4, 6))
        return r, g, b, a / 255.0
    if len(c) == 6:
        r, g, b = (int(c[i:i + 2], 16) for i in (0, 2, 4))
        return r, g, b, 1.0
    raise ValueError("not a colour: %r" % (colour,))


def argb(r, g, b, a):
    return "#%02X%02X%02X%02X" % (max(0, min(255, int(round(a * 255)))), r, g, b)


def hexrgb(r, g, b):
    return "#%02X%02X%02X" % (r, g, b)


def rgba(value):
    r, g, b, a = parse(value) if isinstance(value, str) else value
    return "rgba(%d, %d, %d, %.3f)" % (r, g, b, a)


def triple(value):
    r, g, b, _ = parse(value) if isinstance(value, str) else value
    return "%d, %d, %d" % (r, g, b)


def luminance(r, g, b):
    def channel(v):
        v /= 255.0
        return v / 12.92 if v <= 0.03928 else ((v + 0.055) / 1.055) ** 2.4
    return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b)


def contrast(a, b):
    la, lb = luminance(*a[:3]), luminance(*b[:3])
    return (max(la, lb) + 0.05) / (min(la, lb) + 0.05)


def toward(rgb, target, amount):
    return tuple(int(round(v + (t - v) * amount)) for v, t in zip(rgb, target))


def load(path):
    with open(path) as f:
        return json.load(f)


def resolve(shipped_path, user_path):
    theme = load(shipped_path)
    try:
        theme = merge(theme, load(user_path))
    except (OSError, ValueError):
        pass        # no user file, or one that does not parse: the shipped tokens stand

    dark, light = theme["dark"], theme["light"]
    radii = theme["radii"]

    # The accent: a fixed #rrggbb, or the wallpaper's tint as LumaHome last saved it beside
    # theme.json (tint.json), or the palette's. "tile" (the shipped mode) means per tile on
    # Home, which a menu has no equivalent of, so a menu takes the palette's.
    accent = dark["accent"]
    mode = theme.get("accent")
    if isinstance(mode, str) and mode.startswith("#"):
        accent = mode
    elif mode == "wallpaper":
        try:
            tint = load(os.path.join(os.path.dirname(os.path.abspath(user_path)), "tint.json"))
            if isinstance(tint.get("accent"), str):
                accent = tint["accent"]
        except (OSError, ValueError):
            pass
    ar, ag, ab, _ = parse(accent)

    # Text on the accent: the palette's accentInk and both inks, whichever reads best.
    ink_dark, ink_light = parse(dark["ink"]), parse(light["ink"])
    candidates = [parse(dark["accentInk"]), ink_dark, ink_light]
    accent_ink = max(candidates, key=lambda c: contrast((ar, ag, ab), c))

    # A menu sheet is not a Home card: it can be over a bright film as easily as over the
    # wallpaper, and it carries small text. So the theme supplies the hue and the sheet supplies
    # its own contrast: it is darkened (or lightened, for a light ink) until body text clears 7:1,
    # AAA for normal text and the right target for text read across a room.
    gr, gg, gb, _ = parse(dark["glass"])
    ir, ig, ib, _ = parse(dark["ink"])
    er, eg, eb, ea = parse(dark["glassEdge"])
    sheet = (gr, gg, gb)
    ink_rgb = (ir, ig, ib)
    target = (0, 0, 0) if luminance(*ink_rgb) > 0.5 else (255, 255, 255)
    for _ in range(20):
        if contrast(sheet, ink_rgb) >= 7.0:
            break
        sheet = toward(sheet, target, 0.12)
    # The sheet is nearly opaque on purpose: sheer glass works on Home because the compositor
    # drew the wallpaper behind it, while a settings sheet can be over motion.
    sheet_light = luminance(*sheet) >= 0.5
    top = toward(sheet, (0, 0, 0), 0.06) if sheet_light else tuple(int(round(v + (255 - v) * 0.06)) for v in sheet)
    row_target = (0, 0, 0) if sheet_light else (255, 255, 255)

    menu = {
        "sheetTop": argb(*top, 0.93),
        "sheetBottom": argb(*sheet, 0.93),
        # Rows, buttons and cards inside a sheet: sheer, because the sheet is behind them.
        "row": argb(*toward(sheet, row_target, 0.10), 0.35),
        # Focus and selection are the same glass, lifted: a thin wash and a lit edge, never a solid
        # light fill, so the hue stays and text stays the same ink.
        "focus": argb(255, 255, 255, 0.07) if not sheet_light else argb(0, 0, 0, 0.06),
        "focusEdge": argb(255, 255, 255, 0.28) if not sheet_light else argb(0, 0, 0, 0.22),
        "hairline": argb(er, eg, eb, min(0.22, max(0.10, ea))) if not sheet_light else argb(0, 0, 0, 0.12),
        "ink": argb(ir, ig, ib, 1.0),
        "inkDim": argb(ir, ig, ib, 0.66),
        "focusInk": argb(ir, ig, ib, 1.0),
        "accent": argb(ar, ag, ab, 1.0),
        "accentInk": argb(*accent_ink[:3], 1.0),
        "danger": argb(*parse(dark["danger"])),
        "radius": radii["sheet"],
        "rowRadius": radii["row"],
        "scrim": argb(gr, gg, gb, 0.42),
    }
    return theme, menu, (ar, ag, ab), accent_ink


# Lucent's names for the colour roles, in the order of its tokens.css.
ROLES = [("canvas", "canvas"), ("sheet", "sheet"), ("scrim", "scrim"), ("scrim-hide", "scrimHide"), ("glass", "glass"),
         ("glass-raised", "glassFocus"), ("edge", "glassEdge"), ("edge-raised", "glassFocusEdge"),
         ("ink", "ink"), ("ink-2", "ink2"), ("ink-3", "ink3"), ("focus", "focus"), ("focus-ink", "focusInk"),
         ("track-off", "trackOff"), ("positive", "positive"), ("warning", "warning"), ("danger", "danger"),
         ("info", "info"), ("live", "live")]
WEIGHTS = {"regular": 400, "medium": 500, "semibold": 600, "bold": 700}
FONT_FACES = [(400, "Regular"), (500, "Medium"), (600, "SemiBold"), (700, "Bold")]


def css(theme, menu, accent, accent_ink):
    dark = theme["dark"]
    out = [":root {"]
    for name, role in ROLES:
        out.append("  --lu-%s: %s;" % (name, rgba(dark[role])))
    out.append("  --lu-accent: %s;" % hexrgb(*accent))
    out.append("  --lu-accent-ink: %s;" % hexrgb(*accent_ink[:3]))
    out.append("  --lu-font: %s, system-ui, sans-serif;" % theme["font"]["family"])
    for name in ("sheet", "card", "tile", "row", "control", "pill"):
        out.append("  --lu-radius-%s: %dpx;" % (name, theme["radii"][name]))
    for i, v in enumerate(theme["space"], 1):
        out.append("  --lu-space-%d: %dpx;" % (i, v))
    out.append("  --lu-target: %dpx;" % theme["target"])
    for name, weight in WEIGHTS.items():
        out.append("  --lu-weight-%s: %d;" % (name, weight))
    for role in ("display", "title", "body", "label", "caption"):
        t = theme["type"][role]
        out.append("  --lu-type-%s: %dpx;" % (role, t["size"]))
        out.append("  --lu-type-%s-weight: %d;" % (role, WEIGHTS[t["weight"]]))
        out.append("  --lu-type-%s-line: %s;" % (role, t["line"]))
        out.append("  --lu-type-%s-tracking: %sem;" % (role, t["tracking"]))
    out.append("  --lu-hover-wash: %s;" % rgba(parse(dark["ink"])[:3] + (theme["states"]["hoverWash"],)))
    # Resolved for the settings sheets (the contrast-guarded derivation above)
    out.append("  --lu-sheet-top: %s;" % rgba(menu["sheetTop"]))
    out.append("  --lu-sheet-bottom: %s;" % rgba(menu["sheetBottom"]))
    out.append("  --lu-row: %s;" % rgba(menu["row"]))
    out.append("  --lu-focus-wash: %s;" % rgba(menu["focus"]))
    out.append("  --lu-focus-edge: %s;" % rgba(menu["focusEdge"]))
    out.append("  --lu-hairline: %s;" % rgba(menu["hairline"]))
    out.append("  --lu-ink-dim: %s;" % rgba(menu["inkDim"]))
    out.append("  --lu-menu-scrim: %s;" % rgba(menu["scrim"]))
    out.append("}")
    # Sandstone, the component set the settings apps are made of, is themed through its own
    # variables, and every component reads them. Values are RGB triples where Sandstone composes
    # them with its own alphas.
    track = parse(dark["trackOff"])
    out.append("""
:root,
[class*="ThemeDecorator_theme"] {
  --sand-bg-color: transparent;
  --sand-fullscreen-bg-color: transparent;
  --sand-component-bg-color: transparent;
  --sand-overlay-bg-color-rgb: %(glass)s;
  --sand-alert-overlay-bg-color-rgb: %(glass)s;
  --sand-overlay-outline-color: %(hairline)s;
  --sand-text-color-rgb: %(ink)s;
  --sand-text-sub-color: %(ink_dim)s;
  --sand-component-text-color-rgb: %(ink)s;
  --sand-component-text-sub-color-rgb: %(ink)s;
  --sand-focus-bg-color: %(focus_bg)s;
  --sand-focus-bg-color-rgb: %(focus_rgb)s;
  --sand-component-focus-text-color-rgb: %(focus_ink)s;
  --sand-selected-color-rgb: %(accent_rgb)s;
  --sand-toggle-on-bg-color: %(accent_hex)s;
  --sand-toggle-on-color: %(ink_hex)s;
  --sand-toggle-off-bg-color: %(track)s;
  --sand-checkbox-color: %(accent_hex)s;
  --sand-spinner-color: %(accent_hex)s;
  --sand-progress-color-rgb: %(accent_rgb)s;
  --sand-progress-bg-color-rgb: %(track_rgb)s;
  --sand-progress-bg-color-alpha: %(track_alpha).3f;
  --sand-shadow-color-rgb: 0, 0, 0;
}""" % {
        "glass": triple(dark["glass"]), "hairline": rgba(menu["hairline"]), "ink": triple(dark["ink"]),
        "ink_dim": rgba(menu["inkDim"]), "focus_bg": rgba(dark["focus"]), "focus_rgb": triple(dark["focus"]),
        "focus_ink": triple(dark["focusInk"]), "accent_rgb": "%d, %d, %d" % accent, "accent_hex": hexrgb(*accent),
        "ink_hex": hexrgb(*parse(dark["ink"])[:3]), "track": rgba(track),
        "track_rgb": "%d, %d, %d" % track[:3], "track_alpha": track[3]})
    return "\n".join(out) + "\n"


# QML's easing names in CSS: the focus and card motion of the web apps follows the theme's.
EASING = {"OutCubic": "cubic-bezier(0.215, 0.61, 0.355, 1)", "InOutCubic": "cubic-bezier(0.645, 0.045, 0.355, 1)",
          "OutQuad": "cubic-bezier(0.25, 0.46, 0.45, 0.94)", "InOutQuad": "cubic-bezier(0.455, 0.03, 0.515, 0.955)",
          "OutQuart": "cubic-bezier(0.165, 0.84, 0.44, 1)", "OutExpo": "cubic-bezier(0.19, 1, 0.22, 1)", "Linear": "linear"}


def light_skin(theme):
    """The light material's accent and the ink that reads best on it, both as (r, g, b): the owner's fixed accent or the
    palette's (the wallpaper tints Home, not a web page)."""
    dark, light = theme["dark"], theme["light"]
    mode = theme.get("accent")
    accent = parse(mode)[:3] if isinstance(mode, str) and mode.startswith("#") else parse(light["accent"])[:3]
    ink = max([parse(light["accentInk"]), parse(dark["ink"]), parse(light["ink"])], key=lambda c: contrast(accent, c))
    return accent, ink


def web(theme, accent, accent_ink):
    """LumaTV's own web apps (the frame app behind the Home mini apps): Lucent's tokens as CSS for both materials. An app
    takes the material by `<html data-theme="dark|light">` (dark when there is none), finds the four Manrope faces beside
    the stylesheet and uses only `var(--lu-*)`. The names are `css`'s where they mean the same (`css` is the dark-only skin
    of LG's Enact apps). The accent is the wallpaper's or the owner's fixed one in the dark material and the owner's fixed
    one or the palette's in the light one."""
    dark, light = theme["dark"], theme["light"]
    light_accent, light_ink = light_skin(theme)
    states = theme["states"]

    def material(selector, scheme, m, acc, ink, focus_wash):
        ink_rgb = parse(m["ink"])[:3]
        out = ["%s {" % selector, "  color-scheme: %s;" % scheme]
        out += ["  --lu-%s: %s;" % (name, rgba(m[role])) for name, role in ROLES]
        out += ["  --lu-accent: %s;" % hexrgb(*acc), "  --lu-accent-ink: %s;" % hexrgb(*ink[:3]),
                "  --lu-hover-wash: %s;" % rgba(ink_rgb + (states["hoverWash"],)),
                "  --lu-selected-wash: %s;" % rgba(ink_rgb + (states["selectedWash"],)),
                "  --lu-focus-wash: %s;" % rgba(ink_rgb + (focus_wash,))]
        return out

    # what both materials share: the typeface, shape, space, type and motion
    shared = ["  --lu-font: %s, system-ui, sans-serif;" % theme["font"]["family"]]
    shared += ["  --lu-radius-%s: %dpx;" % (name, theme["radii"][name]) for name in ("sheet", "card", "tile", "row", "control", "pill")]
    shared += ["  --lu-space-%d: %dpx;" % (i, v) for i, v in enumerate(theme["space"], 1)]
    shared += ["  --lu-target: %dpx;" % theme["target"], "  --lu-row-height: %dpx;" % theme["row"]]
    shared += ["  --lu-weight-%s: %d;" % (name, weight) for name, weight in WEIGHTS.items()]
    for role in ("display", "title", "body", "label", "caption"):
        t = theme["type"][role]
        shared += ["  --lu-type-%s: %dpx;" % (role, t["size"]), "  --lu-type-%s-weight: %d;" % (role, WEIGHTS[t["weight"]]),
                   "  --lu-type-%s-line: %s;" % (role, t["line"]), "  --lu-type-%s-tracking: %sem;" % (role, t["tracking"])]
    m = theme["motion"]
    shared += ["  --lu-focus-ms: %dms;" % m["focusMs"], "  --lu-card-ms: %dms;" % m["cardMs"], "  --lu-layer-ms: %dms;" % m["layerMs"],
               "  --lu-press-ms: %dms;" % m["pressMs"], "  --lu-label-ms: %dms;" % m["labelMs"], "  --lu-rest-ms: %dms;" % m["restMs"],
               "  --lu-ease-focus: %s;" % EASING.get(m["focusEasing"], "ease-out"), "  --lu-ease-card: %s;" % EASING.get(m["cardEasing"], "ease-out")]

    out = ["/* Generated by tools/tokens.py from theme.json; do not edit. LumaTV's web apps: Lucent's tokens, both materials. */"]
    for weight, face in FONT_FACES:
        out.append('@font-face { font-family: "%s"; font-weight: %d; font-display: block; src: url("%s-%s.ttf") format("truetype"); }'
                   % (theme["font"]["family"], weight, theme["font"]["family"], face))
    out.append("")
    out += material(':root, [data-theme="dark"]', "dark", dark, accent, accent_ink, states["focusWash"][0]) + shared + ["}", ""]
    out += material('[data-theme="light"]', "light", light, light_accent, light_ink, states["focusWash"][1]) + ["}"]
    return "\n".join(out) + "\n"


def js(theme, accent, accent_ink):
    """Tokens.js: both materials as strings QML takes as colours, and everything structural."""
    t = {
        "font": theme["font"]["family"],
        "dark": theme["dark"], "light": theme["light"],
        "accent": hexrgb(*accent), "accentInk": hexrgb(*accent_ink[:3]),
        "type": theme["type"], "radii": theme["radii"], "space": theme["space"],
        "target": theme["target"], "row": theme["row"], "focus": theme["focus"],
        "states": theme["states"], "motion": theme["motion"],
    }
    return (".pragma library\n\n// Generated by tools/tokens.py from theme.json; do not edit. Colours are QML #AARRGGBB.\n"
            "var tokens = %s\n" % json.dumps(t, indent=2, sort_keys=True))


def main():
    if len(sys.argv) != 4 or sys.argv[3] not in ("qml", "css", "js", "web"):
        sys.exit("usage: tokens.py <shipped theme.json> <theme.json> qml|css|js|web")
    theme, menu, accent, accent_ink = resolve(sys.argv[1], sys.argv[2])
    kind = sys.argv[3]
    if kind == "qml":
        print(json.dumps(menu))
    elif kind == "css":
        sys.stdout.write(css(theme, menu, accent, accent_ink))
    elif kind == "web":
        sys.stdout.write(web(theme, accent, accent_ink))
    else:
        sys.stdout.write(js(theme, accent, accent_ink))


if __name__ == "__main__":
    main()
