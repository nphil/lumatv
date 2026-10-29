#!/usr/bin/env python3
"""Writes assets/glyphs/: the remote-key and action glyphs LumaTV's hints are drawn with, as SVG files, and their index.

    python3 generators/gen-glyphs.py [--out assets/glyphs]

Lucent, "Hints are glyphs, not sentences": a hint to press a key is the key's glyph, never the words "OK" or "Back". The glyphs mirror the
LG Magic Remote (MR24GN/GA, the C4's), which has no lettering: OK is the scroll wheel pressed in the navigation ring, the arrows are the
ring with that direction's chevron, Back is the curved return arrow, Home the house, the voice key the microphone, the colour keys small
bars. Every file is toolkit-neutral: a 24-unit grid, stroke and fill `currentColor` (the colour keys carry the palette tone they take in
`data-tone`), the same stroke family as Lucide (1.9, round caps and joins). The parts that are context (the ring around the wheel) are drawn
at half opacity so the pressed part reads first. Lucide shapes (ISC) come from docs/mocks/icons.js, so these match the shell's icons.
"""
import argparse
import json
import os
import re

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), ".."))
SRC = os.path.join(ROOT, "docs", "mocks", "icons.js")
STROKE = 1.9

RING = '<circle cx="12" cy="12" r="9.5" opacity=".5"/>'


def chevron(turn):
    g = '<path d="m10 8 4 4-4 4"/>'
    return RING + (g if turn == 0 else '<g transform="rotate(%d 12 12)">%s</g>' % (turn, g))


def bar(tone):
    return '<rect x="4" y="9" width="16" height="6" rx="3" fill="currentColor" stroke="none"/>'


# name -> (kind, accessible name, body or Lucide icon name, extra attributes)
KEYS = [
    ("key-ok", "OK", None, RING + '<circle cx="12" cy="12" r="4.2" fill="currentColor" stroke="none"/>'),
    ("key-ok-hold", "Hold OK", None, '<circle cx="12" cy="12" r="6.2" opacity=".5"/><circle cx="12" cy="12" r="2.8" fill="currentColor" stroke="none"/>'
                                      '<path d="M12 2.5a9.5 9.5 0 1 1-9.5 9.5"/>'),
    ("key-up", "Up", None, chevron(270)),
    ("key-down", "Down", None, chevron(90)),
    ("key-left", "Left", None, chevron(180)),
    ("key-right", "Right", None, chevron(0)),
    ("key-back", "Back", "undo-2", None),
    ("key-home", "Home", "house", None),
    ("key-menu", "Menu", None, '<path d="M4 7h16"/><path d="M4 12h16"/><path d="M4 17h16"/>'),
    ("key-mic", "Voice key", "mic", None),
]
TONES = [("key-red", "Red", "danger"), ("key-green", "Green", "positive"), ("key-yellow", "Yellow", "warning"), ("key-blue", "Blue", "info")]
ACTIONS = [
    ("expand", "expand", "maximize-2", None),
    ("collapse", "collapse", None, '<path d="m14 10 7-7"/><path d="M20 10h-6V4"/><path d="m3 21 7-7"/><path d="M4 14h6v6"/>'),
    ("close", "close", "x", None),
    ("open", "open", "play", None),
    ("confirm", "confirm", "check", None),
    ("options", "options", "ellipsis", None),
    ("move", "move", None, '<path d="M12 2v20"/><path d="m15 19-3 3-3-3"/><path d="m19 9 3 3-3 3"/><path d="M2 12h20"/><path d="m5 9-3 3 3 3"/><path d="m9 5 3-3 3 3"/>'),
    ("edit", "edit", None, '<path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/>'),
    ("hold", "hold", "hourglass", None),
]


def lucide():
    with open(SRC, encoding="utf-8") as f:
        text = f.read()
    return json.loads(re.search(r"const I=(\{.*?\});\s*function svg", text, re.S).group(1))


def svg(body, title, extra=""):
    return ('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" '
            'stroke-width="%s" stroke-linecap="round" stroke-linejoin="round"%s role="img" aria-label="%s"><title>%s</title>%s</svg>\n'
            % (STROKE, extra, title, title, body))


def build():
    icons = lucide()
    files, index = {}, {"glyphs": {}}

    def add(name, kind, title, body, lucide_name=None, tone=None):
        if body is None:
            body = icons[lucide_name]
        files[name + ".svg"] = svg(body, title, ' data-tone="%s"' % tone if tone else "")
        entry = {"file": name + ".svg", "kind": kind, "name": title}
        if tone:
            entry["tone"] = tone
        index["glyphs"][name] = entry

    for name, title, lname, body in KEYS:
        add(name, "key", title, body, lname)
    for name, title, tone in TONES:
        add(name, "key", title, bar(tone), None, tone)
    for name, title, lname, body in ACTIONS:
        add(name, "action", title, body, lname)
    index["_"] = ("Hints are glyphs, not sentences (Lucent). Each glyph has one accessible name (`name`). The Magic Remote has no OK or Back lettering, "
                  "so no glyph carries a word. A hint is the key glyphs, then what they do (an action glyph), then a label only where no glyph can "
                  "say it (a name the data supplies). Holding is the key glyph followed by `hold`, except OK, which has `key-ok-hold`.")
    index["compose"] = {
        "size": "1.45 times the type size of the text role beside the hint (label 26 and caption 24 on the TV, so 37 and 35 px at 100 %); it follows the role at 200 % text",
        "name": "one accessible name per hint: the key names joined with 'and', then what it does, then the label ('OK, expand'); a surface may give its own where the generic words lose the meaning ('OK allows')",
        "ink": "keys in the reading ink, what they do in the second ink; the colour keys in the palette tone named in `tone`",
    }
    return files, index


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out", default=os.path.join(ROOT, "assets", "glyphs"))
    a = ap.parse_args()
    files, index = build()
    os.makedirs(a.out, exist_ok=True)
    for n, text in files.items():
        with open(os.path.join(a.out, n), "w", encoding="utf-8") as f:
            f.write(text)
    with open(os.path.join(a.out, "index.json"), "w", encoding="utf-8") as f:
        json.dump(index, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print("%d glyphs in %s" % (len(files), a.out))


if __name__ == "__main__":
    main()
