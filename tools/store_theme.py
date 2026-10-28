#!/usr/bin/env python3
"""Builds the Content Store theme's script: the file WAM runs in LG's com.webos.app.discovery.

usage: store_theme.py build <payload/store> <shipped theme.json> <theme.json> <out file> [--parts look,motion] [--probe-port N]
       store_theme.py parts <text>

The store is a thin loader that sends the view to LG's cloud page, so a stylesheet beside the page cannot reach it (a page on an
https origin cannot load a file:// stylesheet or font). The theme is one script instead, webOSUserScripts/userScript.js in the app's
directory, which WAM runs in every document the view loads: payload/store/userScript.js is the code, store.css the rules it sets
attributes for, Manrope-store.woff2 the one face, and this builder cuts out the parts that are not asked for, fills in Lucent's tokens
from the live theme.json (the dark material only: LG's page and its pictures are made for black) and writes the result. Runs on the set
(python 3.10) and on the workstation; called by tools/store.sh.

Parts, named on the command line and kept in the switch's flag file:
  look    Manrope behind Sandstone's name (and LG Smart UI's), Lucent's glass and ring on the plate LG draws under a focused item, ink on
          its words; a page element is never matched by a class name
  motion  no transition of the page runs longer than the theme's card settle
  canvas  the palette's canvas in place of LG's black, with LG's own black bands painted the same (opt-in: not seen on the set)
  lazy    images made by script wait until they are near the screen (opt-in: see userScript.js)
  probe   diagnostic: the page posts its structure and what the focus found to tools/store_probe.py (needs --probe-port; never persisted)

`build` prints one JSON object. `parts` prints the canonical list of a request ("" is the default) or exits 2.
"""
import base64
import json
import os
import re
import sys

import tokens

PARTS = ("look", "canvas", "motion", "lazy", "probe")
DEFAULT_PARTS = ("look", "motion")
# The cloud page's domain (docs/research/store-result.md: https://<env>.app.lgwebostv.com/discovery_2024/discovery.html); frames of
# it, and of the app's own files, are the store's.
FRAME_HOSTS = ("lgwebostv.com",)
# The families the face is put behind, with the weights each is declared at by Sandstone (LG's Settings build: Sandstone 300/400/600/700,
# MuseoSans 100-900) and by LG's own system faces. A rule is a few bytes, so a family the page does not use costs nothing.
FACES = (("Sandstone", (300, 400, 600, 700)), ("MuseoSans", (100, 300, 400, 500, 700, 900)),
         ("LG Smart UI", (300, 400, 600, 700)), ("LG Display", (400, 700)))
# The characters the face has (generators/gen-store-font.py cuts the same set); any other falls to the page's own face of the family.
RANGE = "U+0020-007E,U+00A0-00FF,U+2013-2014,U+2018-201E,U+2022,U+2026,U+2039-203A,U+20AC,U+2122,U+2190-2193,U+2212"
# A plate is light when its relative luminance is at least this: LG's is rgb(230, 230, 230), 0.79; its unfocused buttons (125, 132, 140), 0.23.
PLATE_LUM = 0.35
# Lucent's contrast.focus and contrast.body.
RING_FLOOR = 3.3
TEXT_FLOOR = 4.5
RING_PX = 2
# The three tiers of ink a plate's words take, by the strength LG gave them: the alpha of the ink.
INK_TIERS = (1.0, 0.80, 0.66)
# What payload/store/userScript.js has in the three places this builder fills. Not a double-brace placeholder: tools/build-ipk.sh fills
# the brand's in every staged file, and fails the package on any it finds left, in this file as much as in the template.
OPTIONS_MARKER = '/*@@OPTIONS@@*/ null'
CSS_MARKER = '/*@@CSS@@*/ ""'
FONT_MARKER = '/*@@FONT@@*/ ""'
PROBE_MAX = 150
PART_START = re.compile(r"^\s*(?://|/\*)@@part ([a-z ]+?)\s*(?:\*/)?\s*$")
PART_END = re.compile(r"^\s*(?://|/\*)@@end(?: [a-z ]+?)?\s*(?:\*/)?\s*$")


def parse_parts(text):
    """The canonical tuple of the parts a request names (commas or spaces); the default for an empty one. ValueError on an unknown part."""
    names = [p for p in (text or "").replace(",", " ").split() if p]
    if not names:
        return DEFAULT_PARTS
    for n in names:
        if n not in PARTS:
            raise ValueError("unknown part %r (parts: %s)" % (n, ", ".join(PARTS)))
    return tuple(p for p in PARTS if p in names)


def cut(text, parts):
    """The text with every block of a part that is not on taken out, and the markers of the blocks that stay. A block starts at a line
    `//@@part a b` (or the same in a CSS comment) and ends at `//@@end`; it stays when any part it names is on."""
    out, keep, inside = [], True, False
    for line in text.split("\n"):
        m = PART_START.match(line)
        if m:
            if inside:
                raise ValueError("a part block inside a part block: " + line.strip())
            inside, keep = True, any(n in parts for n in m.group(1).split())
            continue
        if PART_END.match(line):
            if not inside:
                raise ValueError("a part end without a start: " + line.strip())
            inside, keep = False, True
            continue
        if keep:
            out.append(line)
    if inside:
        raise ValueError("a part block is not closed")
    return "\n".join(out)


def composite(fg, alpha, bg):
    return tuple(int(round(f * alpha + b * (1.0 - alpha))) for f, b in zip(fg[:3], bg[:3]))


def alpha_for(ink, over, floor):
    """The smallest alpha, in steps of 2 %, at which the ink laid over `over` has the contrast `floor` against it."""
    for pct in range(20, 101, 2):
        a = pct / 100.0
        if tokens.contrast(composite(ink, a, over), over) >= floor:
            return a
    return 1.0


def ring_alpha(ink, outside, plate):
    """The smallest alpha, in steps of 2 %, at which the ring (the ink over the plate, drawn inside the plate's edge) is RING_FLOOR:1
    against what is outside the plate and against the plate itself."""
    for pct in range(20, 101, 2):
        a = pct / 100.0
        ring = composite(ink, a, plate)
        if tokens.contrast(ring, outside) >= RING_FLOOR and tokens.contrast(ring, plate) >= RING_FLOOR:
            return a
    return 1.0


def ring_options(theme):
    """What the script needs to keep the ring at RING_FLOOR:1 on a surface that is not LG's black: the ink, the glass, the floor and the
    alphas the sheet's tokens were derived at (for black). The script raises the alpha where the page around the plate is lighter."""
    dark = theme["dark"]
    ink, plate = tokens.parse(dark["ink"])[:3], tokens.parse(dark["glassFocus"])[:3]
    black = (0, 0, 0)
    return {"ink": list(ink), "plate": list(plate), "floor": RING_FLOOR, "base": ring_alpha(ink, black, plate), "baseOut": alpha_for(ink, black, RING_FLOOR)}


def tokens_css(theme, parts):
    """Lucent's values as custom properties on the root: the plate (the focus glass, opaque so that the words' contrast does not depend on
    what is behind it), the ring, the ink in three tiers, the row radius and the canvas. Raises ValueError when the palette would break a floor."""
    dark = theme["dark"]
    ink = tokens.parse(dark["ink"])[:3]
    plate = tokens.parse(dark["glassFocus"])[:3]
    canvas = tokens.parse(dark["canvas"])[:3]
    black = (0, 0, 0)
    for alpha in INK_TIERS:
        text = composite(ink, alpha, plate)
        if tokens.contrast(text, plate) < TEXT_FLOOR:
            raise ValueError("the palette's ink at %.2f is %.2f:1 on the plate, under %.1f" % (alpha, tokens.contrast(text, plate), TEXT_FLOOR))
    ring, ring_out = ring_options(theme)["base"], ring_options(theme)["baseOut"]
    rows = [
        "--st-plate: %s;" % tokens.hexrgb(*plate),
        "--st-ring: %s;" % tokens.rgba(ink + (ring,)),
        "--st-ring-out: %s;" % tokens.rgba(ink + (ring_out,)),
        "--st-ink: %s;" % tokens.hexrgb(*ink),
        "--st-ink-2: %s;" % tokens.rgba(ink + (INK_TIERS[1],)),
        "--st-ink-3: %s;" % tokens.rgba(ink + (INK_TIERS[2],)),
        "--st-radius: %dpx;" % theme["radii"]["row"],
        "--st-canvas: %s;" % tokens.hexrgb(*canvas),
    ]
    return ":root { " + " ".join(rows) + " }\n"


def build_css(payload, theme, parts):
    with open(os.path.join(payload, "store.css"), encoding="utf-8") as f:
        rules = cut(f.read(), parts)
    # the comment at the top of store.css explains the sheet in the repository; the script carries only what the page needs
    rules = re.sub(r"/\*.*?\*/", "", rules, flags=re.S)
    rules = re.sub(r"\n\s*\n+", "\n", rules).strip()
    if not rules:
        return ""
    return tokens_css(theme, parts) + rules + "\n"


def build_script(payload, shipped, user, parts, probe_port=None):
    theme, _menu, _accent, _accent_ink = tokens.resolve(shipped, user)
    if "probe" in parts and not probe_port:
        raise ValueError("the probe part needs the port of the receiver (--probe-port)")
    options = {"parts": list(parts), "capMs": int(theme["motion"]["cardMs"]), "frameHosts": list(FRAME_HOSTS),
               "faces": [[name, list(weights)] for name, weights in FACES], "range": RANGE, "plateLum": PLATE_LUM, "ring": ring_options(theme)}
    if "probe" in parts:
        options["probe"] = {"url": "http://127.0.0.1:%d/p" % int(probe_port), "max": PROBE_MAX}
    css = build_css(payload, theme, parts) if ("look" in parts or "canvas" in parts) else ""
    font = ""
    if "look" in parts:
        with open(os.path.join(payload, "Manrope-store.woff2"), "rb") as f:
            font = base64.b64encode(f.read()).decode("ascii")
    with open(os.path.join(payload, "userScript.js"), encoding="utf-8") as f:
        text = f.read()
    for marker in (OPTIONS_MARKER, CSS_MARKER, FONT_MARKER):
        if text.count(marker) != 1:
            raise ValueError("payload/store/userScript.js lost the marker %s" % marker)
    text = cut(text, parts)
    # str.replace inserts the text as it is, whatever backslashes and dollar signs the CSS carries.
    text = text.replace(OPTIONS_MARKER, json.dumps(options)).replace(CSS_MARKER, json.dumps(css)).replace(FONT_MARKER, json.dumps(font))
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
    parts_text, probe_port = "", None
    rest = argv[6:]
    while rest:
        flag = rest.pop(0)
        if flag == "--parts" and rest:
            parts_text = rest.pop(0)
        elif flag == "--probe-port" and rest:
            probe_port = rest.pop(0)
        else:
            print("unknown argument %r" % flag, file=sys.stderr)
            return 2
    try:
        parts = parse_parts(parts_text)
    except ValueError as e:
        print(e, file=sys.stderr)
        return 2
    try:
        text, options = build_script(payload, shipped, user, parts, probe_port)
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
