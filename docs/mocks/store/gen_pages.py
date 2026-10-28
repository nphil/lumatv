#!/usr/bin/env python3
"""Writes the stand-in Content Store pages: LG's page as the TV showed it (docs/research/store/off-*.jpg), in several ways of drawing the
focus plate, so that the theme is proven against what it cannot read: the store's markup.

    python3 gen_pages.py OUTDIR [--dialects pseudo,child,...]      writes OUTDIR/<page>-<dialect>.html, one self-contained file each

The look is measured from the TV captures: black canvas and black header, the 1080p type of Sandstone (the page's `font-family: Sandstone`,
declared here from a system face as LG declares it from local()), a caption row under each poster (app icon, name, category, price) whose focus
plate is rgb(230, 230, 230) with words in rgb(75, 76, 80), the download button beside it and outside the plate, a large light Install plate,
grey-blue secondary buttons, a tab strip with a white underline, four icon buttons in the header. The class names are Sandstone's compiled
ones in every dialect but `opaque`, whose names mean nothing, because the theme must not depend on them.

Dialects: how LG's focus plate might be built, none of which can be read from the set:
  pseudo   the plate is the focused item's ::before, faded in by opacity
  child    the plate is a child of the focused item, its colour transitioned from transparent
  self     the plate is the focused item's own background
  tile     the focused element is the whole tile and the plate is the caption below its poster
  opaque   as child, with class names that mean nothing
  late     as child, but the plate is switched by a class the page's own focus handler adds after the focus event (React-style), not by :focus
Each page also carries what the plate-finder must not take for a plate: a white tile behind every app icon, a translucent price.
"""
import hashlib
import os
import sys

DIALECTS = ("pseudo", "child", "self", "tile", "opaque", "late")
BGD = ("child", "opaque", "late")       # the dialects whose plate is a child element of the focused item
PAGES = ("home", "category", "detail", "search", "popup")
# not a picture of anything: the weight of a full store page (ten shelves of fourteen tiles, about 2,300 elements) for bench.mjs, as it is
# in the markup (`heavy`) and as an app renders it, 300 ms after the document (`heavy-spa`: the store's cloud page is rendered by script)
HEAVY = "heavy"
SPA = "heavy-spa"

# component part -> (component, part): Sandstone's compiled name is enact_sandstone_<Component>_<Component>_<part>__<hash>
SAND = {
    "root": ("ThemeDecorator", "root"), "header": ("Header", "header"), "title": ("Header", "title"), "subtitle": ("Header", "subtitle"),
    "slot": ("Header", "slotAfter"), "tabs": ("TabGroup", "tabGroup"), "tab": ("TabGroup", "tab"), "tabsel": ("TabGroup", "selected"),
    "heading": ("Heading", "heading"), "item": ("Item", "item"), "bg": ("Item", "bg"), "tile": ("ImageItem", "imageItem"),
    "poster": ("ImageItem", "image"), "cap": ("ImageItem", "captions"), "icon": ("ImageItem", "imageIcon"), "name": ("ImageItem", "caption"),
    "meta": ("ImageItem", "label"), "btn": ("Button", "button"), "btnbg": ("Button", "bg"), "btnc": ("Button", "client"),
    "ibtn": ("Button", "iconOnly"), "body": ("BodyText", "bodyText"), "input": ("Input", "inputArea"), "popup": ("Popup", "popup"),
    "scrim": ("FloatingLayer", "scrim"), "spinner": ("Spinner", "spinner"),
}

ICONS = {
    "search": '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>',
    "more": '<circle cx="12" cy="5" r="1.4"/><circle cx="12" cy="12" r="1.4"/><circle cx="12" cy="19" r="1.4"/>',
    "x": '<path d="M6 6l12 12M18 6L6 18"/>',
    "download": '<path d="M12 4v11"/><path d="m7 11 5 5 5-5"/><path d="M5 20h14"/>',
    "left": '<path d="m15 5-7 7 7 7"/>',
    "right": '<path d="m9 5 7 7-7 7"/>',
    "back": '<path d="M19 12H5"/><path d="m11 6-6 6 6 6"/>',
}

APPS = [("DAZN", "Entertainment", "Free", 200, "Undisputed"), ("FITE is now Tr", "Entertainment", "Free", 18, "Fight Night"),
        ("STAGE+ Stre", "Entertainment", "Free", 48, "Classical"), ("Marquee TV -", "Entertainment", "Free", 330, "Giselle"),
        ("LG Radio+", "Entertainment", "Free", 262, "Trending"), ("+100k Songs", "Entertainment", "Free", 330, "Karaoke"),
        ("LG Gaming Po", "Game", "Free", 270, "Playtime"), ("YouTube", "Entertainment", "Free", 0, "Watch"),
        ("Just Dance N", "Game", "Free", 30, "Dance"), ("LG Fitness", "Life", "Free", 340, "Move"),
        ("Xfinity Stream", "Entertainment", "Free", 250, "Stream"), ("Fandango at", "Entertainment", "Free", 20, "Movies")]


def make_names(dialect):
    def cn(key):
        if dialect == "opaque":
            return "q" + hashlib.md5(key.encode()).hexdigest()[:5]
        comp, part = SAND[key]
        return "enact_sandstone_%s_%s_%s__%s" % (comp, comp, part, hashlib.md5(key.encode()).hexdigest()[:5])
    return cn


def css(dialect, cn):
    c = {k: "." + cn(k) for k in SAND}
    plate = "rgb(230, 230, 230)"
    words = "rgb(75, 76, 80)"
    base = """
@font-face { font-family: Sandstone; font-weight: 300; src: local("Liberation Sans"); }
@font-face { font-family: Sandstone; font-weight: 400; src: local("Liberation Sans"); }
@font-face { font-family: Sandstone; font-weight: 600; src: local("Liberation Sans Bold"), local("Liberation Sans"); }
@font-face { font-family: Sandstone; font-weight: 700; src: local("Liberation Sans Bold"), local("Liberation Sans"); }
:root { --ts: 1; }
* { box-sizing: border-box; }
html, body { margin: 0; background: #000; }
body { width: 1920px; height: 1080px; overflow: hidden; position: relative; color: rgb(236, 236, 236); }
%(root)s { font-family: Sandstone; font-size: calc(24px * var(--ts)); font-weight: 400; line-height: 1.25; position: absolute; inset: 0; overflow: clip; }
.page { position: absolute; inset: 0; }
svg.i { width: 36px; height: 36px; fill: none; stroke: currentColor; stroke-width: 2.2; stroke-linecap: round; stroke-linejoin: round; flex: none; }

/* the header: a black band of its own, as on the TV */
%(header)s { position: absolute; left: 0; right: 0; top: 0; height: 232px; background: rgb(0, 0, 0); padding: 36px 96px 0 80px; }
.hd { display: flex; align-items: center; height: 96px; }
%(title)s { font-size: calc(54px * var(--ts)); font-weight: 300; margin: 0; line-height: 1.1; }
%(subtitle)s { font-size: calc(26px * var(--ts)); color: rgba(236, 236, 236, .8); }
.hd .grow { flex: 1; }
%(slot)s { display: flex; gap: 22px; align-items: center; }
.avatar { width: 40px; height: 40px; border-radius: 50%%; background: rgb(115, 97, 219); display: flex; align-items: center; justify-content: center; font-size: 22px; font-weight: 600; color: #fff; }
%(tabs)s { position: absolute; left: 80px; right: 0; bottom: 0; height: 78px; display: flex; border-bottom: 2px solid rgb(46, 46, 46); }
%(tab)s { flex: 0 0 auto; width: 340px; display: flex; align-items: center; justify-content: center; font-size: calc(28px * var(--ts)); position: relative; border-radius: 12px; }
%(tabsel)s::after { content: ""; position: absolute; left: 0; right: 0; bottom: 0; height: 4px; background: #fff; }
[tabindex]:focus { outline: 0; }

/* banners and shelves */
.banners { position: absolute; left: 100px; top: 280px; display: flex; gap: 20px; }
.banners img { height: 250px; display: block; object-fit: cover; }
.banners .b1 { width: 966px; } .banners .b2 { width: 340px; } .banners .b3 { width: 340px; }
.arrow { position: absolute; top: 380px; width: 40px; height: 48px; color: #fff; }
.shelves { position: absolute; left: 100px; right: 0; top: 584px; display: flex; flex-direction: column; gap: 24px; }
%(heading)s { font-size: calc(28px * var(--ts)); font-weight: 600; margin: 0 0 18px; }
.row { display: flex; gap: 20px; overflow: clip; }
%(tile)s { flex: 0 0 416px; width: 416px; position: relative; }
%(poster)s { display: block; width: 416px; height: 233px; object-fit: cover; border: 1px solid rgba(255, 255, 255, .25); }
.cwrap { position: relative; display: flex; align-items: center; }
%(cap)s { position: relative; display: flex; align-items: center; gap: 16px; width: 344px; min-height: 126px; padding: 14px 16px 14px 12px; border-radius: 12px; margin-top: 8px; }
%(icon)s { width: 96px; height: 96px; flex: none; background: #fff; border-radius: 8px; display: flex; align-items: center; justify-content: center; position: relative; }
%(icon)s img { width: 96px; height: 96px; display: block; }
.txt { position: relative; display: flex; flex-direction: column; min-width: 0; }
%(name)s { font-size: calc(30px * var(--ts)); font-weight: 600; color: rgb(236, 236, 236); line-height: 1.15; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
%(meta)s { font-size: calc(24px * var(--ts)); color: rgba(236, 236, 236, .88); line-height: 1.2; }
.price { opacity: .75; }
.dl { position: relative; width: 56px; height: 56px; margin-left: 16px; border: 0; background: transparent; color: rgb(236, 236, 236); border-radius: 12px; display: flex; align-items: center; justify-content: center; }
.dl svg { width: 40px; height: 40px; }

/* buttons: LG's secondary plate is grey-blue with light words; its focus plate is the light one */
%(btn)s { position: relative; display: inline-flex; align-items: center; justify-content: center; min-width: 120px; min-height: 66px; padding: 0 28px; font-size: calc(28px * var(--ts)); font-weight: 600; color: rgb(242, 242, 242); background: transparent; border: 0; border-radius: 12px; font-family: inherit; }
%(btnbg)s { position: absolute; inset: 0; border-radius: 12px; }
%(btnc)s { position: relative; display: inline-flex; align-items: center; gap: 12px; }
%(ibtn)s { min-width: 66px; padding: 0; border-radius: 14px; color: rgb(236, 236, 236); }
%(body)s { margin: 0; font-size: calc(26px * var(--ts)); line-height: 1.4; color: rgba(236, 236, 236, .9); }
%(input)s { position: relative; display: flex; align-items: center; gap: 18px; height: 84px; padding: 0 28px; border-radius: 12px; border: 2px solid rgba(255, 255, 255, .5); font-size: calc(32px * var(--ts)); background: transparent; }
%(item)s { position: relative; display: flex; align-items: center; gap: 24px; min-height: 96px; padding: 12px 24px; border-radius: 12px; width: 100%%; background: transparent; border: 0; color: inherit; font: inherit; text-align: left; }
%(item)s .rn { position: relative; font-size: calc(30px * var(--ts)); font-weight: 600; color: rgb(236, 236, 236); }
%(spinner)s { position: absolute; left: 928px; top: 520px; width: 64px; height: 64px; border-radius: 50%%; background: rgba(120, 120, 120, .85); }
%(scrim)s { position: absolute; inset: 0; background: rgba(0, 0, 0, .55); }
%(popup)s { position: absolute; left: 560px; top: 330px; width: 800px; min-height: 330px; padding: 44px 48px; background: rgb(95, 107, 122); border-radius: 14px; display: flex; flex-direction: column; gap: 22px; }
%(popup)s h2 { margin: 0; font-size: calc(36px * var(--ts)); font-weight: 600; }
.actions { display: flex; gap: 22px; margin-top: auto; }
.grid { position: absolute; left: 100px; top: 272px; display: grid; grid-template-columns: repeat(4, 416px); gap: 36px 20px; }
.detail { position: absolute; left: 80px; right: 96px; top: 250px; display: grid; grid-template-columns: 330px minmax(0, 1fr); gap: 24px 60px; }
.dicon { width: 330px; height: 256px; background: rgb(12, 12, 14); display: flex; align-items: center; justify-content: center; }
.dicon img { width: 120px; height: 120px; }
.install { width: 330px; margin-top: 8px; }
.fine { font-size: calc(22px * var(--ts)); color: rgba(236, 236, 236, .8); line-height: 1.7; margin-top: 14px; }
.dright h3 { margin: 0 0 8px; font-size: calc(24px * var(--ts)); font-style: italic; font-weight: 400; border-bottom: 2px solid rgb(70, 70, 70); padding-bottom: 6px; }
.shots { display: flex; gap: 20px; margin-top: 14px; }
.shots img { width: 300px; height: 169px; object-fit: cover; }
.query { position: absolute; left: 80px; right: 96px; top: 250px; }
.results { position: absolute; left: 80px; right: 96px; top: 380px; display: grid; grid-template-columns: 1fr 1fr; gap: 12px 40px; }
.rimg { position: relative; width: 84px; height: 84px; background: #fff; border-radius: 8px; }
""" % c
    if dialect == "pseudo":
        plates = """
%(item)s::before, %(btn)s::before, %(tab)s::before { content: ""; position: absolute; inset: 0; border-radius: 12px; background: %(plate)s; opacity: 0; transition: opacity .2s ease-in-out; }
%(cap)s::before { content: ""; position: absolute; inset: 0; border-radius: 12px; background: %(plate)s; opacity: 0; transition: opacity .2s ease-in-out; }
%(btn)s::before { background: rgb(125, 132, 140); opacity: 1; }
%(btn)s:focus::before { background: %(plate)s; }
%(ibtn)s::before { opacity: 0; }
%(ibtn)s:focus::before { opacity: 1; }
%(item)s:focus::before, %(tab)s:focus::before { opacity: 1; }
%(tile)s:focus %(cap)s::before { opacity: 0; }
.capf:focus::before { opacity: 1; }
""" % dict(c, plate=plate)
    elif dialect in BGD:
        plates = """
.bg { position: absolute; inset: 0; border-radius: 12px; background-color: transparent; transition: background-color .2s ease-in-out; }
%(btnbg)s { background-color: rgb(125, 132, 140); transition: background-color .2s ease-in-out; }
%(item)s:focus .bg, %(tab)s:focus .bg, .capf:focus .bg { background-color: %(plate)s; }
%(btn)s:focus %(btnbg)s { background-color: %(plate)s; }
%(ibtn)s %(btnbg)s { background-color: transparent; }
%(ibtn)s:focus %(btnbg)s { background-color: %(plate)s; }
""" % dict(c, plate=plate)
    elif dialect == "self":
        plates = """
%(btn)s { background: rgb(125, 132, 140); }
%(ibtn)s { background: transparent; }
%(item)s:focus, %(tab)s:focus, .capf:focus, %(btn)s:focus { background: %(plate)s; }
""" % dict(c, plate=plate)
    else:   # tile
        plates = """
%(cap)s { transition: background-color .2s; }
%(btn)s { background: rgb(125, 132, 140); }
%(ibtn)s { background: transparent; }
%(tile)s:focus %(cap)s, %(item)s:focus, %(tab)s:focus, %(btn)s:focus { background: %(plate)s; }
""" % dict(c, plate=plate)
    focus_words = """
%(item)s:focus, %(tab)s:focus, %(btn)s:focus, .capf:focus, %(tile)s:focus %(cap)s { color: %(words)s; }
.capf:focus %(name)s, %(tile)s:focus %(name)s, %(item)s:focus .rn { color: %(words)s; }
.capf:focus %(meta)s, %(tile)s:focus %(meta)s { color: rgba(75, 76, 80, .85); }
%(btn)s:focus %(btnc)s { color: %(words)s; }
""" % dict(c, words=words)
    if dialect == "late":
        plates, focus_words = plates.replace(":focus", ".on"), focus_words.replace(":focus", ".on")
    return base + plates + focus_words


def svg(name, cn=None):
    return '<svg class="i" viewBox="0 0 24 24">%s</svg>' % ICONS[name]


def item_open(dialect, cn, cls_extra="", tag="div"):
    """the focusable caption row (or, for `tile`, the plain row under the poster)"""
    focus = "" if dialect == "tile" else ' tabindex="0"'
    capf = "" if dialect == "tile" else " capf"
    bg = '<div class="bg"></div>' if dialect in BGD else ""
    return '<%s class="%s%s"%s>%s' % (tag, cn("cap"), capf, focus, bg), "</%s>" % tag


def tile(i, app, dialect, cn):
    name, cat, price, hue, label = app
    o, c = item_open(dialect, cn)
    tile_focus = ' tabindex="0"' if dialect == "tile" else ""
    return ('<div class="%s tile-%d"%s><img class="%s" data-art="thumb:%d:%s" alt="">' % (cn("tile"), i, tile_focus, cn("poster"), hue, label) +
            '<div class="cwrap">' + o + '<div class="%s"><img data-art="icon:%d:%s" alt=""></div><div class="txt"><span class="%s">%s</span><span class="%s">%s</span><span class="%s price">%s</span></div>' % (
                cn("icon"), hue, name, cn("name"), name, cn("meta"), cat, cn("meta"), price) + c +
            '<button class="dl" tabindex="0" aria-label="download">%s</button></div></div>' % svg("download"))


def button(label, cn, dialect, extra="", icon=None):
    bg = '<span class="%s"></span>' % cn("btnbg") if dialect in BGD else ""
    inner = (svg(icon) if icon else "") + label
    return '<button class="%s %s" tabindex="0">%s<span class="%s">%s</span></button>' % (cn("btn"), extra, bg, cn("btnc"), inner)


def ibutton(icon, cn, dialect, label):
    bg = '<span class="%s"></span>' % cn("btnbg") if dialect in BGD else ""
    return '<button class="%s %s" tabindex="0" aria-label="%s">%s<span class="%s">%s</span></button>' % (cn("btn"), cn("ibtn"), label, bg, cn("btnc"), svg(icon))


def header(title, cn, dialect, tabs=None, back=False, sub=None):
    left = (ibutton("back", cn, dialect, "back") if back else "")
    t = '<div><h1 class="%s">%s</h1>%s</div>' % (cn("title"), title, ('<div class="%s">%s</div>' % (cn("subtitle"), sub)) if sub else "")
    slot = '<div class="%s">%s<button class="%s %s" tabindex="0" aria-label="account"><span class="%s"><span class="avatar">N</span></span></button>%s%s</div>' % (
        cn("slot"), ibutton("search", cn, dialect, "search"), cn("btn"), cn("ibtn"), cn("btnc"), ibutton("more", cn, dialect, "more"), ibutton("x", cn, dialect, "close"))
    out = '<header class="%s"><div class="hd">%s%s<div class="grow"></div>%s</div>' % (cn("header"), left, t, slot)
    if tabs:
        out += '<nav class="%s" role="tablist">' % cn("tabs")
        for n in tabs[0]:
            bg = '<div class="bg"></div>' if dialect in BGD else ""
            out += '<div class="%s%s" role="tab" tabindex="0">%s%s</div>' % (cn("tab"), (" " + cn("tabsel")) if n == tabs[1] else "", bg, n)
        out += "</nav>"
    return out + "</header>"


# what the page's own code does in the `late` dialect: the class that shows the plate is added after the focus event, as a component library's
# state change would, and taken off at the blur
LATE = ("<script>document.addEventListener('focusin', function (e) { var t = e.target; setTimeout(function () { t.classList.add('on'); }, 0); });"
        "document.addEventListener('focusout', function (e) { e.target.classList.remove('on'); });</script>")
TABS = ["Featured", "Entertainment", "Sports", "Music", "Game", "News &amp; Info", "Lifestyle"]


def page(name, dialect, cn):
    focus = {"home": ".tile-1 .capf, .tile-1" if dialect != "tile" else ".tile-1", "category": ".tile-5 .capf" if dialect != "tile" else ".tile-5",
             "detail": ".install", "search": ".results .rowa", "popup": ".popup-ok", HEAVY: ".tile-3 .capf" if dialect != "tile" else ".tile-3",
             SPA: ".tile-3 .capf" if dialect != "tile" else ".tile-3"}[name]
    if dialect != "tile" and name in ("home", "category"):
        focus = focus.split(",")[0]
    out = ['<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Stand-in: LG Content Store - %s (%s)</title>' % (name, dialect),
           "<style>%s</style>" % css(dialect, cn), ('<script src="art.js"></script>' if name not in (HEAVY, SPA) else "") + (LATE if dialect == "late" else "") + "</head>",
           '<body data-focus="%s"><div class="%s"><div class="page">' % (focus, cn("root"))]
    if name == "home":
        out.append(header("Apps", cn, dialect, tabs=(TABS, "Featured")))
        out.append('<div class="banners"><img class="b1" data-art="banner:190:Watch with TV" alt=""><img class="b2" data-art="card:18:Sling" alt=""><img class="b3" data-art="card:150:Tenzing" alt=""></div>')
        out.append('<div class="arrow" style="left:40px">%s</div><div class="arrow" style="left:1830px">%s</div>' % (svg("left"), svg("right")))
        out.append('<div class="shelves"><section><h2 class="%s">Global Sports &amp; Art</h2><div class="row">%s</div></section><section><h2 class="%s">Entertainment Picks</h2></section></div>' % (
            cn("heading"), "".join(tile(i, a, dialect, cn) for i, a in enumerate(APPS[:4])), cn("heading")))
    elif name in (HEAVY, SPA):
        out.append(header("Apps", cn, dialect, tabs=(TABS, "Featured")))
        shelves = "".join('<section><h2 class="%s">Shelf %d</h2><div class="row">%s</div></section>' % (
            cn("heading"), shelf + 1, "".join(tile(i if shelf == 0 else 1000 + shelf * 20 + i, APPS[i % len(APPS)], dialect, cn) for i in range(14))) for shelf in range(10))
        if name == HEAVY:
            out.append('<div class="shelves">%s</div>' % shelves)
        else:
            out.append('<div id="mount"></div><template id="t"><div class="shelves">%s</div></template>'
                       '<script>setTimeout(function () { document.getElementById("mount").appendChild(document.getElementById("t").content.cloneNode(true)); }, 300);</script>' % shelves)
    elif name == "category":
        out.append(header("Game", cn, dialect, tabs=(TABS, "Game")))
        out.append('<div class="grid">%s</div>' % "".join(tile(i, a, dialect, cn) for i, a in enumerate(APPS[:8])))
    elif name == "detail":
        out.append(header("Fandango at Home", cn, dialect, back=True, sub="Entertainment"))
        out.append('<div class="detail"><div><div class="dicon"><img data-art="icon:20:Fandango" alt=""></div>%s<div class="fine">Category &nbsp; Entertainment<br>Rating &nbsp; 16+<br>Size : 24.53MB<br>Price : Free (In-app purchase)</div></div>' % button("Install", cn, dialect, "install"))
        out.append('<div class="dright"><h3>Details</h3><p class="%s">When you use the app, there are items available for purchase. Watch the latest movies and TV shows with Fandango at Home. Enjoy thousands of FREE movies, rent and buy new releases, and watch your favorites in 4K UHD.</p>' % cn("body"))
        out.append('<p class="%s">Updated : 08/31/2026<br>Version : 4.3.74<br>Seller : Fandango</p><h3>Screenshots</h3><div class="shots"><img data-art="thumb:210:One" alt=""><img data-art="thumb:230:Two" alt=""><img data-art="thumb:250:Three" alt=""></div>%s</div></div>' % (cn("body"), button("more", cn, dialect, "morebtn")))
    elif name == "search":
        out.append(header("Search", cn, dialect))
        out.append('<div class="query"><div class="%s" role="search">%s<span>stream</span></div></div><div class="results">' % (cn("input"), svg("search")))
        for i, a in enumerate(APPS[:6]):
            bg = '<span class="bg"></span>' if dialect in BGD else ""
            out.append('<div class="%s %s" tabindex="0">%s<div class="rimg"><img data-art="icon:%d:%s" alt="" style="width:84px;height:84px"></div><span class="rn"><span>%s</span><br><span class="%s">%s &middot; %s</span></span></div>' % (
                cn("item"), "rowa" if i == 1 else "", bg, a[3], a[0], a[0], cn("meta"), a[1], a[2]))
        out.append("</div>")
    else:
        out.append(header("Fandango at Home", cn, dialect, back=True, sub="Entertainment"))
        out.append('<div class="detail"><div><div class="dicon"><img data-art="icon:20:Fandango" alt=""></div>%s</div></div>' % button("Install", cn, dialect, "install"))
        out.append('<div class="%s"></div><div class="%s popup" role="dialog"><h2>Install Fandango at Home?</h2><p class="%s">The app is free and uses 25 MB. It will appear with your other apps when it is installed.</p><div class="actions">%s%s</div></div>' % (
            cn("scrim"), cn("popup"), cn("body"), button("Install", cn, dialect, "popup-ok"), button("Cancel", cn, dialect)))
    out.append("</div></div></body></html>")
    return "".join(out)


def main(argv):
    if len(argv) < 2:
        print(__doc__.split("\n\n")[0], file=sys.stderr)
        return 2
    outdir = argv[1]
    dialects = DIALECTS
    if "--dialects" in argv:
        dialects = tuple(argv[argv.index("--dialects") + 1].split(","))
    os.makedirs(outdir, exist_ok=True)
    here = os.path.dirname(os.path.abspath(__file__))
    for name in ("art.js",):
        with open(os.path.join(here, name), "rb") as src, open(os.path.join(outdir, name), "wb") as dst:
            dst.write(src.read())
    for d in dialects:
        cn = make_names(d)
        for p in PAGES + (HEAVY, SPA):
            with open(os.path.join(outdir, "%s-%s.html" % (p, d)), "w", encoding="utf-8") as f:
                f.write(page(p, d, cn))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
