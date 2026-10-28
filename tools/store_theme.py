#!/usr/bin/env python3
"""Builds the Content Store theme's script: the file WAM runs in LG's com.webos.app.discovery.

usage: store_theme.py build <payload/store> <shipped theme.json> <theme.json> <out file> [--parts look,motion] [--material auto|dark|light]
       store_theme.py parts <text>

The store is a thin loader that sends the view to LG's cloud page, so a stylesheet beside the page cannot reach it (a page on an
https origin cannot load a file:// stylesheet or font). The theme is one script instead, webOSUserScripts/userScript.js in the app's
directory, which WAM runs in every document the view loads: the generated file holds the stylesheet (Lucent's tokens for the material
chosen, the store's own derived values, payload/store/store.css) and the one font face as data, and payload/store/userScript.js is the
code that adopts them. Runs on the set (python 3.10) and on the workstation; called by tools/store.sh.

Parts, named on the command line and kept in the switch's flag file:
  look    the stylesheet: canvas, ink, type, shape, the focus rule
  motion  no transition of the page runs longer than the theme's card settle
  lazy    images made by script wait until they are near the screen (off unless named: see userScript.js)

`build` prints one JSON object. `parts` prints the canonical list of a request ("" is the default) or exits 2.
"""
import base64
import json
import os
import sys

import tokens

PARTS = ("look", "motion", "lazy")
DEFAULT_PARTS = ("look", "motion")
# The cloud page's domain (docs/research/store-result.md: https://<env>.app.lgwebostv.com/discovery_2024/discovery.html); frames of
# it, and of the app's own files, are the store's.
FRAME_HOSTS = ("lgwebostv.com",)
# Lucent's contrast.focus is 3:1; the ring is made at least this against both what is outside it and the focused tile's fill.
RING_FLOOR = 3.3
RING_PX = 3
UNDERLINE_PX = 4
GLOW = {"dark": 0.10, "light": 0.14}
# languages/lucent/tokens.json shadow.rest
SHEET_SHADOW = {"dark": "0 24px 60px rgba(0, 0, 0, 0.28)", "light": "0 24px 60px rgba(0, 0, 0, 0.18)"}


def parse_parts(text):
    """The canonical tuple of the parts a request names (commas or spaces); the default for an empty one. ValueError on an unknown part."""
    names = [p for p in (text or "").replace(",", " ").split() if p]
    if not names:
        return DEFAULT_PARTS
    for n in names:
        if n not in PARTS:
            raise ValueError("unknown part %r (parts: %s)" % (n, ", ".join(PARTS)))
    return tuple(p for p in PARTS if p in names)


def composite(fg, alpha, bg):
    return tuple(int(round(f * alpha + b * (1.0 - alpha))) for f, b in zip(fg[:3], bg[:3]))


def ring_alpha(ink, canvas, raised):
    """The smallest alpha, in steps of 2 %, at which the ring (the ink over the focused tile's raised glass, which is what is under an
    outline drawn inside the tile's edge) is RING_FLOOR:1 against the canvas around the tile and against the tile's own fill."""
    fill = composite(raised[:3], raised[3], canvas)
    for pct in range(30, 101, 2):
        a = pct / 100.0
        ring = composite(ink, a, fill)
        if tokens.contrast(ring, canvas) >= RING_FLOOR and tokens.contrast(ring, fill) >= RING_FLOOR:
            return a
    return 1.0


def pick_material(theme, user_path, wanted):
    """dark or light: the one asked for; else the theme's own `material` when it names one; else what Home last decided from the
    wallpaper (tint.json, written beside theme.json by LumaHome); else dark."""
    if wanted in ("dark", "light"):
        return wanted
    if theme.get("material") in ("dark", "light"):
        return theme["material"]
    try:
        tint = tokens.load(os.path.join(os.path.dirname(os.path.abspath(user_path)), "tint.json"))
        if isinstance(tint.get("light"), bool):
            return "light" if tint["light"] else "dark"
    except (OSError, ValueError):
        pass
    return "dark"


def extras(theme, accent):
    """The store's own values, derived from the tokens, for both materials, in the form web() writes its own: a block per material,
    which the page takes by `<html data-theme>`. Everything store.css reads that the shared tokens do not name."""
    dark, light = theme["dark"], theme["light"]
    light_accent, _ = tokens.light_skin(theme)
    focus = theme["focus"]
    shared = ["  --st-ring-w: %dpx;" % RING_PX, "  --st-underline: %dpx;" % UNDERLINE_PX,
              "  --st-scale-row: %s;" % focus["rowScale"], "  --st-scale-card: %s;" % focus["cardScale"],
              "  --st-scale-button: %s;" % focus["buttonScale"], "  --st-scale-press: %s;" % focus["pressScale"],
              "  --st-lift-card: %dpx;" % focus["cardLift"]]

    def block(selector, scheme, m, acc):
        ink = tokens.parse(m["ink"])[:3]
        canvas = tokens.parse(m["canvas"])[:3]
        a = ring_alpha(ink, canvas, tokens.parse(m["glassFocus"]))
        return ["%s {" % selector,
                "  --st-ink-rgb: %s;" % tokens.triple(m["ink"]),
                "  --st-accent-rgb: %d, %d, %d;" % tuple(acc),
                "  --st-focus-fill-rgb: %s;" % tokens.triple(m["glassFocus"]),
                "  --st-sheet-rgb: %s;" % tokens.triple(m["sheet"]),
                "  --st-sheet-solid: %s;" % tokens.hexrgb(*composite(tokens.parse(m["sheet"]), tokens.parse(m["sheet"])[3], canvas)),
                "  --st-glow: %s;" % tokens.rgba(tuple(acc) + (GLOW[scheme],)),
                "  --st-ring: %s;" % tokens.rgba(ink + (a,)),
                "  --st-shadow-sheet: %s;" % SHEET_SHADOW[scheme]]

    return (block(':root, [data-theme="dark"]', "dark", dark, accent) + shared + ["}", ""] +
            block('[data-theme="light"]', "light", light, light_accent) + ["}", ""])


def build_css(payload, theme, accent, accent_ink, material="dark"):
    web = tokens.web(theme, accent, accent_ink)
    # web() names four static faces beside the stylesheet; a page on the cloud origin cannot load them, so they are replaced by the
    # store's one face, carried as data.
    lines = [l for l in web.splitlines() if not l.startswith("@font-face")]
    with open(os.path.join(payload, "Manrope-store.woff2"), "rb") as f:
        face = base64.b64encode(f.read()).decode("ascii")
    with open(os.path.join(payload, "store.css"), encoding="utf-8") as f:
        sheet = f.read()
    if material == "light":
        # store-light.css comes first: its rule has no specificity and store.css's colours for the secondary tiers must win over it
        with open(os.path.join(payload, "store-light.css"), encoding="utf-8") as f:
            sheet = f.read() + "\n" + sheet
    family = theme["font"]["family"]
    out = ["/* Generated by tools/store_theme.py from theme.json and payload/store; do not edit. */",
           '@font-face { font-family: "%s"; font-weight: 400 700; font-display: block; src: url(data:font/woff2;base64,%s) format("woff2"); }' % (family, face),
           "\n".join(lines), "\n".join(extras(theme, accent)), sheet]
    css = "\n".join(out)
    # The material is chosen here, not by an attribute on <html> that a page could lose or that does not exist yet when the script runs:
    # the light blocks come after the dark ones and say :root, so they win where they name the same property.
    if material == "light":
        css = css.replace('[data-theme="light"] {', ":root {")
    return css


def build_script(payload, shipped, user, parts, wanted_material="auto"):
    theme, _menu, accent, accent_ink = tokens.resolve(shipped, user)
    material = pick_material(theme, user, wanted_material)
    options = {"parts": list(parts), "material": material, "capMs": int(theme["motion"]["cardMs"]), "frameHosts": list(FRAME_HOSTS)}
    css = build_css(payload, theme, accent, accent_ink, material) if "look" in parts else ""
    with open(os.path.join(payload, "userScript.js"), encoding="utf-8") as f:
        text = f.read()
    for marker in ('/*{{OPTIONS}}*/ null', '/*{{CSS}}*/ ""'):
        if text.count(marker) != 1:
            raise ValueError("payload/store/userScript.js lost the marker %s" % marker)
    # str.replace inserts the text as it is, whatever backslashes and dollar signs the CSS carries.
    text = text.replace('/*{{OPTIONS}}*/ null', json.dumps(options)).replace('/*{{CSS}}*/ ""', json.dumps(css))
    return text, options


def main(argv):
    if len(argv) >= 3 and argv[1] == "parts" and len(argv) == 3:
        try:
            print(",".join(parse_parts(argv[2])))
        except ValueError as e:
            print(e, file=sys.stderr)
            return 2
        return 0
    if len(argv) < 6 or argv[1] != "build":
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    payload, shipped, user, out = argv[2:6]
    parts_text, material = "", "auto"
    rest = argv[6:]
    while rest:
        flag = rest.pop(0)
        if flag == "--parts" and rest:
            parts_text = rest.pop(0)
        elif flag == "--material" and rest:
            material = rest.pop(0)
        else:
            print("unknown argument %r" % flag, file=sys.stderr)
            return 2
    if material not in ("auto", "dark", "light"):
        print("material is auto, dark or light", file=sys.stderr)
        return 2
    try:
        parts = parse_parts(parts_text)
    except ValueError as e:
        print(e, file=sys.stderr)
        return 2
    try:
        text, options = build_script(payload, shipped, user, parts, material)
    except (OSError, ValueError, KeyError) as e:
        print(json.dumps({"ok": False, "error": str(e)}))
        return 1
    tmp = "%s.tmp.%d" % (out, os.getpid())
    with open(tmp, "w", encoding="utf-8") as f:
        f.write(text)
    os.replace(tmp, out)
    options.update({"ok": True, "bytes": len(text.encode("utf-8"))})
    print(json.dumps(options))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
