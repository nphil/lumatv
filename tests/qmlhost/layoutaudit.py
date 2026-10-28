"""Finds what reads as sloppy on a rendered surface, from its item tree and its picture: words cut short, words
outside or crammed against the pill, card or row they sit in, words cut by a clip or the window edge, two runs of
words on top of each other, words drawn inside a scaled item (a focus that moves them sideways), a run of words not
centred in its pill or not level with the icon beside it, and words that do not read on what is behind them.
`audit(root, width, height, image, checks)` walks the tree after the surface has settled and returns one finding per
fault; qmlhost.py writes them with `--audit MS:PATH[:MODE]` and scenes.py `--audit` fails a scene that has any.

A "box" is the pill, card, row, tile or pane a run of words belongs to: a Rectangle with a radius and a fill or an
edge, an item whose own-sized child is such a Rectangle (the kit's items draw their shape that way; a child inset by
up to 8 px on every side still counts, the segmented control's pill is one), or an item carrying a Glass quad (a
pane). The painted rectangle of a Text is its contentWidth by contentHeight at its alignment inside the item; the
glyph rectangle drops the leading Qt adds under the last line for `lineHeight` above 1 (measured: the extra goes
below the glyphs); the ink rectangle runs from the first line's cap top to the last line's descent, which is what the
eye sees take room.

Thresholds, in px of the 1920x1080 surface: a word may leave its box, a clip or the window by at most 1, keeps 8
(space.2) from the box's sides and 4 from its top and bottom, is centred in a box up to 160 tall when its centre is
within 2.5 of the box's, and is level with the icon before it when their centres are within 3. Contrast follows
Lucent (LANGUAGE.md section 3): the background is the median luminance of the ink rectangle in the finished picture,
the ink the pixel farthest from it; 4.5:1 for text under 48 px and 3:1 from 48 px up (the TV's "large", twice the
Show's 24 units) and for a one-glyph mark (an initial, a badge digit) for ink at .9 alpha or more; fainter text (ink2, a
dimmed row) is muted by design and needs 3:1.

`checks` names the kinds to report (default all): the 200 % text pass asks for "overlap" alone, the one universal
rule that holds at any text size on a screen without a text-size setting.
"""
import json

from PyQt5 import QtCore, QtGui, QtQml

TOL = 1.0            # px a run of words may leave its box, clip or window by
PAD_X = 8.0          # px of air from the box's left and right (Lucent space.2)
PAD_Y = 4.0          # px from the box's top and bottom
CENTRE = 2.5         # px a run's centre may sit from its box's
ICON_LEVEL = 3.0     # px an icon's centre may sit from the words' beside it
CENTRED_MAX_H = 160  # boxes taller than this hold a layout, not one run of words (a pane, a drawer)
ICON_MAX = 72        # an icon-like item (a glyph, a glyph tile, a dot) is at most this big
MIN_ALPHA = 0.05
READING_ALPHA = 0.9  # ink at least this opaque is reading text; fainter is muted by design
LARGE_PX = 48        # text from this size up may read at 3:1
CONTRAST = {"text": 4.5, "large": 3.0, "muted": 3.0}
KINDS = ("elided", "overflow", "clipped", "scaled", "outside", "tight", "overlap", "off-centre", "icon-level", "contrast")

DESCENDERS = set("gjpqy,;()[]{}@_\u00e7\u00b5")   # glyphs that reach under the baseline (the underscore is drawn under it)
HIGH_MARKS = set("`")                            # marks drawn above the capitals' height: the ink rectangle starts at the line's top for them

_LINEAR = [((c / 255.0) / 12.92 if c / 255.0 <= 0.04045 else (((c / 255.0) + 0.055) / 1.055) ** 2.4) for c in range(256)]


def type_name(item):
    name = item.metaObject().className()
    for cut in ("_QMLTYPE_", "_QML_"):
        name = name.split(cut)[0]
    return name.replace("QQuick", "")


def inherits(item, cls):
    return item.inherits(cls)


def rect_of(item):
    return item.mapRectToScene(QtCore.QRectF(0, 0, item.width(), item.height()))


def colour_alpha(v):
    if isinstance(v, QtGui.QColor):
        return v.alphaF()
    return 0.0


def painted_rectangle(item):
    """A Rectangle that draws a shape: a radius and a fill or an edge someone can see."""
    if not inherits(item, "QQuickRectangle") or not item.isVisible():
        return False
    radius = item.property("radius") or 0
    if radius <= 0:
        return False
    if colour_alpha(item.property("color")) >= MIN_ALPHA:
        return True
    width = QtQml.QQmlProperty(item, "border.width").read() or 0
    return width > 0 and colour_alpha(QtQml.QQmlProperty(item, "border.color").read()) >= MIN_ALPHA


def box_rect(item):
    """The scene rectangle of the box `item` is, or None when it is not one."""
    if painted_rectangle(item):
        return rect_of(item)
    mine = rect_of(item)
    if mine.width() <= 0 or mine.height() <= 0:
        return None
    best = None
    for child in item.childItems():
        if type_name(child) == "Glass" and child.isVisible():
            return mine
        if painted_rectangle(child):
            r = rect_of(child)
            inset = (r.left() - mine.left(), r.top() - mine.top(), mine.right() - r.right(), mine.bottom() - r.bottom())
            if all(-TOL <= i <= 8 + TOL for i in inset) and (best is None or sum(inset) < best[0]):
                best = (sum(inset), r)          # the shape that fits the item best: a pill, not the shadow drawn under it
    return best[1] if best else None


def text_rects(item):
    """(painted, glyph, ink) rectangles of a Text in scene coordinates (see the module note)."""
    cw = float(item.property("contentWidth") or 0)
    ch = float(item.property("contentHeight") or 0)
    w, h = item.width(), item.height()
    ha = int(item.property("effectiveHorizontalAlignment") or 1)
    va = int(item.property("verticalAlignment") or 32)
    x = (w - cw) if ha == 2 else (w - cw) / 2 if ha == 4 else 0
    y = (h - ch) if va == 64 else (h - ch) / 2 if va == 128 else 0
    glyph_h = ch
    mode = int(item.property("lineHeightMode") or 0)
    line = float(item.property("lineHeight") or 1)
    lines = int(item.property("lineCount") or 1)
    if mode == 0 and line > 1 and lines >= 1:
        natural = ch / (lines * line)
        glyph_h = ch - natural * (line - 1)
    metrics = QtGui.QFontMetricsF(item.property("font"))
    cap_top = metrics.ascent() - metrics.capHeight()
    if any(c in HIGH_MARKS for c in (item.property("text") or "")):
        cap_top = 0.0
    # the ink ends at the last baseline unless a glyph hangs under it (numerals and capitals do not)
    text = item.property("text") or ""
    bottom = glyph_h if any(c in DESCENDERS for c in text) else glyph_h - metrics.descent()
    painted = item.mapRectToScene(QtCore.QRectF(x, y, cw, ch))
    glyph = item.mapRectToScene(QtCore.QRectF(x, y, cw, glyph_h))
    ink = item.mapRectToScene(QtCore.QRectF(x, y + cap_top, cw, max(1.0, bottom - cap_top)))
    return painted, glyph, ink


def is_icon(item):
    return type_name(item) in ("LuIcon", "LuTile") or (inherits(item, "QQuickImage") and item.property("source") and item.property("source").toString())


def short(item):
    name = item.objectName() or type_name(item)
    text = item.property("text") if inherits(item, "QQuickText") else None
    if text:
        name += "[%s]" % (text if len(text) <= 40 else text[:37] + "\u2026")
    return name


def path_of(item, stop):
    parts = []
    while item is not None and item is not stop:
        parts.append(short(item))
        item = item.parentItem()
    parts.reverse()
    return "/".join(parts[-5:])


def luminances(image, rect):
    """The relative luminance of every pixel of `rect` in `image` (sRGB, WCAG), or [] when the rect is off the picture."""
    r = rect.toAlignedRect().intersected(image.rect())
    if r.width() <= 0 or r.height() <= 0:
        return []
    crop = image.copy(r).convertToFormat(QtGui.QImage.Format_RGB888)
    stride = crop.bytesPerLine()
    data = crop.constBits().asstring(stride * crop.height())
    out = []
    w3 = crop.width() * 3
    for row in range(crop.height()):
        line = data[row * stride:row * stride + w3]
        for i in range(0, w3, 3):
            out.append(0.2126 * _LINEAR[line[i]] + 0.7152 * _LINEAR[line[i + 1]] + 0.0722 * _LINEAR[line[i + 2]])
    return out


def contrast_in(image, rect):
    """(ratio, background, ink): the background is the median luminance of the box, the ink the pixel farthest from it."""
    ls = sorted(luminances(image, rect))
    if not ls:
        return None
    bg = ls[len(ls) // 2]
    ink = ls[0] if bg - ls[0] >= ls[-1] - bg else ls[-1]
    hi, lo = max(bg, ink), min(bg, ink)
    return (hi + 0.05) / (lo + 0.05), bg, ink


def audit(root, width, height, image=None, checks=None):
    checks = set(KINDS if checks is None else checks)
    window = QtCore.QRectF(0, 0, width, height)
    findings = []
    texts = []          # (item, ink rect, box item): two runs overlap when their ink does, not their boxes
    boxes = {}          # box item -> {"rect", "texts": [rects], "icons": [rects]}

    owners = []         # the item each finding is about, in step with `findings`

    def add(kind, item, detail, rect=None):
        if kind not in checks:
            return
        f = {"kind": kind, "item": path_of(item, root), "detail": detail}
        if rect is not None:
            f["rect"] = [round(rect.x()), round(rect.y()), round(rect.width()), round(rect.height())]
        findings.append(f)
        owners.append(item)

    def occlude(rect):
        """A pane or an opaque slab painted over words drawn earlier hides them: they are not on the screen."""
        hidden = {id(t[0]) for t in texts if rect.contains(t[1])}
        if not hidden:
            return
        texts[:] = [t for t in texts if id(t[0]) not in hidden]
        keep = [i for i, o in enumerate(owners) if id(o) not in hidden]
        findings[:] = [findings[i] for i in keep]
        owners[:] = [owners[i] for i in keep]
        for info in boxes.values():
            info["texts"] = [r for r in info["texts"] if not rect.contains(r)]

    def note_in_box(box, key, rect):
        if box is None:
            return
        boxes[box][key].append(rect)

    def covers(item, opacity):
        """A veil over the whole window (a layer's scrim): what was drawn before it is the page behind, not the screen."""
        if not inherits(item, "QQuickRectangle"):
            return False
        r = rect_of(item)
        if r.width() * r.height() < 0.95 * width * height:
            return False
        return colour_alpha(item.property("color")) * opacity >= 0.3

    def walk(item, opacity, clips, scaled, box):
        if not item.isVisible():
            return
        opacity *= item.opacity()
        if opacity < MIN_ALPHA:
            return
        if covers(item, opacity):
            del texts[:]
            boxes.clear()
            del findings[:]
            del owners[:]
            return
        if item.isVisible():
            r = rect_of(item)
            if r.width() * r.height() >= 10000 and (any(type_name(c) == "Glass" and c.isVisible() for c in item.childItems())
                                                   or (inherits(item, "QQuickRectangle") and colour_alpha(item.property("color")) * opacity >= 0.8)):
                occlude(r)
        if item.scale() != 1 and item is not root:
            scaled = item
        if item.clip():
            clips = clips + [rect_of(item)]
        icon = is_icon(item)
        own_box = box_rect(item)
        if own_box is not None and own_box.width() <= ICON_MAX and own_box.height() <= ICON_MAX:
            icon = True                         # a glyph tile, a dot: a small shape beside words
        if icon:
            note_in_box(box, "icons", rect_of(item))
        if own_box is not None:
            boxes[item] = {"rect": own_box, "texts": [], "icons": []}
            box = item
        elif icon:
            return
        if inherits(item, "QQuickText") and (item.property("text") or "") != "":
            painted, glyph, ink = text_rects(item)
            if painted.width() <= 0:
                return
            for clip in clips + [window]:
                if not glyph.intersects(clip):
                    return                          # wholly hidden by the clip: not drawn, nothing to judge
            if item.property("truncated"):
                add("elided", item, "cut short with an ellipsis", painted)
            own = rect_of(item)
            if painted.width() > own.width() + TOL:
                add("overflow", item, "draws %.0f px wide in an item %.0f wide" % (painted.width(), own.width()), painted)
            for clip in clips + [window]:
                if (ink.left() < clip.left() - TOL or ink.right() > clip.right() + TOL
                        or ink.top() < clip.top() - TOL or ink.bottom() > clip.bottom() + TOL):
                    add("clipped", item, "cut by a clip or the window edge at %s" % rect_str(clip), ink)
                    break
            if scaled is not None:
                add("scaled", item, "drawn inside %s scaled %.3f: focus moves the words" % (short(scaled), scaled.scale()), glyph)
            if box is not None:
                b = boxes[box]["rect"]
                left, right = ink.left() - b.left(), b.right() - ink.right()
                top, bottom = ink.top() - b.top(), b.bottom() - ink.bottom()
                if min(left, right) < -TOL or min(top, bottom) < -TOL:
                    add("outside", item, "leaves its box %s by %.0f px" % (short(box), -min(left, right, top, bottom)), ink)
                elif min(left, right) < PAD_X - 0.5 or min(top, bottom) < PAD_Y - 0.5:
                    add("tight", item, "only %.0f/%.0f px beside and %.0f/%.0f px above and below it in %s" % (left, right, top, bottom, short(box)), ink)
            if image is not None and "contrast" in checks:
                alpha = colour_alpha(item.property("color")) * opacity
                size = float(item.property("font").pixelSize())
                mark = len((item.property("text") or "").strip()) <= 1          # one glyph (an initial, a badge digit) is a mark, not reading
                need = CONTRAST["muted"] if alpha < READING_ALPHA else CONTRAST["large"] if size >= LARGE_PX or mark else CONTRAST["text"]
                measured = contrast_in(image, ink)
                if measured is not None and measured[0] < need:
                    add("contrast", item, "reads at %.1f:1 on its background (needs %.1f; ink alpha %.2f, %.0f px)" % (measured[0], need, alpha, size), ink)
            texts.append((item, ink, box))
            note_in_box(box, "texts", glyph)
            return
        for child in sorted(item.childItems(), key=lambda c: c.z()):      # paint order
            walk(child, opacity, clips, scaled, box)

    walk(root, 1.0, [], None, None)

    for i in range(len(texts)):
        a, ra, _ = texts[i]
        for j in range(i + 1, len(texts)):
            b, rb, _ = texts[j]
            inter = ra.intersected(rb)
            if inter.width() > 1 and inter.height() > 1:
                add("overlap", a, "over %s" % path_of(b, root), inter)

    for box, info in boxes.items():
        b = info["rect"]
        if not info["texts"] or b.height() > CENTRED_MAX_H:
            continue
        union = info["texts"][0]
        for r in info["texts"][1:]:
            union = union.united(r)
        if abs(union.center().y() - b.center().y()) > CENTRE:
            add("off-centre", box, "words sit %.1f px %s of the middle of %s" % (abs(union.center().y() - b.center().y()),
                "below" if union.center().y() > b.center().y() else "above", rect_str(b)), union)
        for icon in info["icons"]:
            leading = icon.right() <= union.left() + TOL
            if leading and icon.width() <= ICON_MAX and icon.height() <= ICON_MAX and abs(icon.center().y() - union.center().y()) > ICON_LEVEL:
                add("icon-level", box, "icon centre %.1f px from the words' centre" % abs(icon.center().y() - union.center().y()), icon)
    return findings


def rect_str(r):
    return "%.0f,%.0f %.0fx%.0f" % (r.x(), r.y(), r.width(), r.height())


def write(path, findings):
    with open(path, "w") as f:
        json.dump(findings, f, indent=1)


def lines(findings, tag="layout"):
    return ["[%s] %s: %s (%s)" % (tag, f["kind"], f["item"], f["detail"]) for f in findings]
