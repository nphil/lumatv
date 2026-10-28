// WAM runs webOSUserScripts/userScript.js from an app's own directory in every document the app's view loads, before the
// page's own scripts: in the Content Store's loader page and in the cloud pages the loader sends the view to (upstream:
// webosose/wam, WebPageBase::setCustomUserScript, which also covers pages on origins outside the app's directory).
// tools/store_theme.py fills the three markers below (OPTIONS_MARKER, CSS_MARKER and FONT_MARKER in it), cuts the parts that are not asked
// for out of the text (a block between a part marker and its end marker stays only when one of the parts it names is on) and writes the
// result into the overlay's upper layer. The markers are not brand placeholders in double braces: tools/build-ipk.sh fills those from
// brand.json and fails the package on any it leaves.
//
// The script changes how the page looks and how long its transitions take, and nothing else: it makes no request (the probe part, a
// diagnostic that is never on unless asked for, posts what it saw to a port of the set itself), reads nothing from the page but
// computed styles, writes none of its words and clicks nothing, and every step is guarded, so that a failure leaves LG's page as it is.
(function () {
    "use strict";
    try {
        var OPT = /*@@OPTIONS@@*/ null;
        var CSS = /*@@CSS@@*/ "";
        var FONT = /*@@FONT@@*/ "";
        if (!OPT) return;

        // The top document, and frames on the app's own origins (its files and the cloud page's domain); a trailer or an ad
        // frame in the page is not the store's.
        if (window.top !== window) {
            var own = location.protocol === "file:";
            for (var h = 0; !own && h < OPT.frameHosts.length; h++) {
                var host = OPT.frameHosts[h];
                own = location.hostname === host || location.hostname.slice(-(host.length + 1)) === "." + host;
            }
            if (!own) return;
        }
        // Once per document. The script runs before <html> exists (WAM runs it as the document is created), so nothing below needs the
        // root: the marker is put on it as soon as it is there.
        if (window.__lumatvStore) return;
        Object.defineProperty(window, "__lumatvStore", { value: OPT.parts.join(",") });
        function whenRoot(fn) {
            if (document.documentElement) { fn(document.documentElement); return; }
            var wait = new MutationObserver(function () {
                if (document.documentElement) { wait.disconnect(); fn(document.documentElement); }
            });
            wait.observe(document, { childList: true });
        }
        whenRoot(function (root) { root.setAttribute("data-lumatv-store", OPT.parts.join(",")); });

        // One constructable stylesheet adopted by the document: it needs no <head>, so it is in place before the first frame, and a page's
        // content security policy, which governs <style> elements, does not see it. A <style> element is the fallback.
        function adopt(text) {
            try {
                if (window.CSSStyleSheet && "adoptedStyleSheets" in document && CSSStyleSheet.prototype.replaceSync) {
                    var sheet = new CSSStyleSheet();
                    sheet.replaceSync(text);
                    document.adoptedStyleSheets = Array.prototype.slice.call(document.adoptedStyleSheets).concat([sheet]);
                    return;
                }
            } catch (e) { /* the <style> element below */ }
            var style = document.createElement("style");
            style.textContent = text;
            whenRoot(function (root) { (document.head || root).appendChild(style); });
        }

        //@@part look canvas probe
        // Computed colours are "rgb(r, g, b)" or "rgba(r, g, b, a)" (an alpha in percent only after a slash, in newer engines).
        function rgba(s) {
            var m = String(s).match(/[\d.]+/g);
            if (!m || m.length < 3) return null;
            var a = m.length > 3 ? +m[3] : 1;
            if (m.length > 3 && /%\s*\)\s*$/.test(s)) a /= 100;
            return [+m[0], +m[1], +m[2], a];
        }
        function lum(c) {
            function ch(v) { v /= 255; return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4); }
            return 0.2126 * ch(c[0]) + 0.7152 * ch(c[1]) + 0.0722 * ch(c[2]);
        }
        //@@end

        var sheetHead = "";
        //@@part look
        // ---- the face. Sandstone, LG's component set, asks for `font-family: Sandstone` in 132 declarations of the Settings build alone, so
        // the face is put behind that name, one @font-face per weight the page declares (a single face for a range of weights loses to the
        // page's own face at each exact weight: measured), and no selector is needed at all. A character the face lacks falls to the page's
        // own face of the same name. One blob URL serves every rule, so the font is decoded once, and only when a weight is used. It is made
        // at once and not when the page is idle: a face that arrives after the page's words have been laid out makes the browser lay them
        // out again (50 ms for 2,200 elements on the workstation, measured), and the cloud page may be rendered by the server.
        function decode(b64) {
            var s = atob(b64), n = s.length, a = new Uint8Array(n), i;
            for (i = 0; i < n; i++) a[i] = s.charCodeAt(i);
            return a;
        }
        function faces() {
            if (!FONT) return "";
            var url, out = "", i, j, f;
            try { url = URL.createObjectURL(new Blob([decode(FONT)], { type: "font/woff2" })); } catch (e) { url = "data:font/woff2;base64," + FONT; }
            for (i = 0; i < OPT.faces.length; i++) {
                f = OPT.faces[i];
                for (j = 0; j < f[1].length; j++) {
                    out += '@font-face{font-family:"' + f[0] + '";font-weight:' + f[1][j] + ";font-display:block;src:url(" + url +
                        ') format("woff2");unicode-range:' + OPT.range + "}";
                }
            }
            return out;
        }
        sheetHead = faces();

        // ---- the focus. LG draws a light plate under a focused item and its words in dark on it; which element draws it cannot be read from the
        // set, so it is found by what it computes: a light, opaque background (the element itself, its ::before or its ::after) that
        // carries words, in the focused element or below it. That plate becomes Lucent's glass and its words ink, together; a plate that is
        // not found stays LG's, with LG's words. Marks are attributes (store.css) and are cleared when the focus leaves.
        var marks = [], focusEl = null, timer = 0, DELAYS = [60, 140, 260], found = false, watcher = null, watched = 0, queued = false, born = 0;
        function mark(el, attr, value) {
            el.setAttribute(attr, value === undefined ? "" : value);
            marks.push([el, attr]);
        }
        function clearMarks() {
            for (var i = 0; i < marks.length; i++) {
                try {
                    if (marks[i][1].charAt(0) === "-") marks[i][0].style.removeProperty(marks[i][1]);
                    else marks[i][0].removeAttribute(marks[i][1]);
                } catch (e) { /* gone */ }
            }
            marks.length = 0;
            window.__lumatvFocus = null;
        }
        // The item that loses the focus: its marks come off at once, so that LG's colours are what is computed, except on a plate LG hides by
        // fading it out (a running transition of its opacity) while its colour stays light: that would be LG's light plate fading under the
        // dark ink of the moment before. Its marks stay for the fade (our `transition: none` ends it at once), 300 ms at most, and go early
        // if the item is focused again. A plate LG hides at once, or one that is light all the time (a panel), is not held.
        var retired = [];
        function drop(entry) {
            clearTimeout(entry[2]);
            try { entry[0].removeAttribute(entry[1]); } catch (e) { /* gone */ }
            var at = retired.indexOf(entry);
            if (at >= 0) retired.splice(at, 1);
        }
        function releaseNear(f) {
            for (var i = retired.length - 1; i >= 0; i--) {
                if (retired[i][0] === f || f.contains(retired[i][0]) || retired[i][0].contains(f)) drop(retired[i]);
            }
        }
        function fading(list, el, pseudo) {
            for (var i = 0; i < list.length; i++) {
                if ((list[i].transitionProperty === "opacity" || list[i].transitionProperty === "visibility") && list[i].effect && list[i].effect.target === el &&
                    (list[i].effect.pseudoElement || "") === pseudo) return true;
            }
            return false;
        }
        function retireMarks() {
            var plates = [], list, i, m, pseudo, cs, c, entry;
            for (i = 0; i < marks.length; i++) if (/^data-lu-p[sba]$/.test(marks[i][1])) plates.push(marks[i]);
            clearMarks();
            if (!plates.length) return;
            try { list = document.getAnimations(); } catch (e) { return; }
            for (i = 0; i < plates.length; i++) {
                m = plates[i];
                pseudo = m[1] === "data-lu-ps" ? "" : (m[1] === "data-lu-pb" ? "::before" : "::after");
                try {
                    if (!fading(list, m[0], pseudo)) continue;
                    cs = getComputedStyle(m[0], pseudo || null);
                    c = rgba(cs.backgroundColor);
                    if (c && c[3] >= 0.6 && lum(c) >= OPT.plateLum) {
                        m[0].setAttribute(m[1], "");
                        entry = [m[0], m[1], 0];
                        entry[2] = setTimeout(drop.bind(null, entry), 300);
                        retired.push(entry);
                    }
                } catch (e) { /* gone */ }
            }
        }
        function family(f, fa) {
            // the focused element and what is below it (five levels, sixty elements) that is big enough to be a plate under it: the style of
            // the small ones (a label, an icon) is never read
            var all = [f], depth = [0], out = [f], i = 0, el, d, k, r;
            while (i < all.length && all.length < 60) {
                el = all[i]; d = depth[i++];
                if (d >= 5) continue;
                for (k = el.firstElementChild; k && all.length < 60; k = k.nextElementSibling) {
                    all.push(k); depth.push(d + 1);
                    r = k.getBoundingClientRect();
                    if (r.width * r.height >= 0.08 * fa) out.push(k);
                }
            }
            return out;
        }
        function headings(f) {
            // what the running transitions of the focused item are heading to: a plate whose colour is still fading in is judged by where it
            // ends and not by the colour it has at this frame
            var out = [], list, i, a, el, kf;
            try { list = document.getAnimations(); } catch (e) { return out; }
            for (i = 0; i < list.length && out.length < 30; i++) {
                a = list[i];
                if (a.transitionProperty !== "background-color" || !a.effect) continue;
                el = a.effect.target;
                if (!el || !f.contains(el)) continue;
                kf = a.effect.getKeyframes();
                if (kf.length) out.push([el, a.effect.pseudoElement || "", kf[kf.length - 1].backgroundColor]);
            }
            return out;
        }
        function wordsOf(f) {
            // the boxes of the words of the focused element, as their centres: what a plate is for
            var out = [], seen = [], walker = document.createTreeWalker(f, NodeFilter.SHOW_TEXT, null), n, e, r;
            while ((n = walker.nextNode()) && out.length < 40) {
                if (!/\S/.test(n.nodeValue)) continue;
                e = n.parentElement;
                if (!e || seen.indexOf(e) >= 0) continue;
                seen.push(e);
                r = e.getBoundingClientRect();
                out.push({ el: e, cx: (r.left + r.right) / 2, cy: (r.top + r.bottom) / 2 });
            }
            return out;
        }
        function carries(rect, words) {
            for (var i = 0; i < words.length; i++) {
                if (words[i].cx >= rect.left && words[i].cx <= rect.right && words[i].cy >= rect.top && words[i].cy <= rect.bottom) return true;
            }
            return false;
        }
        var PSEUDO = { ps: "", pb: "::before", pa: "::after" };
        function plateOf(el, kind, fa, page, words, heading) {
            var cs, c, w, h, rect, i;
            if (kind === "ps") {
                cs = getComputedStyle(el);
            } else {
                cs = getComputedStyle(el, PSEUDO[kind]);
                if (cs.content === "none" || cs.content === "normal") return null;
            }
            c = rgba(cs.backgroundColor);
            for (i = 0; i < heading.length; i++) if (heading[i][0] === el && heading[i][1] === PSEUDO[kind]) c = rgba(heading[i][2]) || c;
            if (!c || c[3] < 0.6 || lum(c) < OPT.plateLum) return null;
            rect = el.getBoundingClientRect();
            w = rect.width; h = rect.height;
            if (kind !== "ps") {
                w = parseFloat(cs.width); h = parseFloat(cs.height);
                if (!(w > 0 && h > 0)) { w = rect.width; h = rect.height; }
            }
            if (w * h < 0.12 * fa || w * h > 0.7 * page) return null;
            if (!carries(rect, words)) return null;            // a plate has words on it; the white tile behind an icon has none
            return { el: el, kind: kind, area: w * h, rect: rect };
        }
        function detect(f, fa, words) {
            var els = family(f, fa), kinds = ["ps", "pb", "pa"], heading = headings(f), found = [], page = innerWidth * innerHeight, big = 0, out = [], i, k, p;
            for (i = 0; i < els.length; i++) {
                for (k = 0; k < 3; k++) { p = plateOf(els[i], kinds[k], fa, page, words, heading); if (p) found.push(p); }
            }
            for (i = 0; i < found.length; i++) if (found[i].area > big) big = found[i].area;
            for (i = 0; i < found.length; i++) if (found[i].area >= 0.9 * big) out.push(found[i]);
            return out;
        }
        function recolour(f, plates, words) {
            // the words of the focused element that lie on a plate, in the tier LG gave them
            var plated = [], hosts = [], i, j, e, cs, bg, op, solid, tier, c, a, other, count = 0;
            for (i = 0; i < plates.length; i++) plated.push(plates[i].el);
            for (i = 0; i < words.length; i++) {
                for (j = 0; j < plates.length; j++) if (carries(plates[j].rect, [words[i]])) { hosts.push(words[i].el); break; }
            }
            for (i = 0; i < hosts.length; i++) {
                op = 1; solid = []; other = false;
                for (e = hosts[i]; e; e = e.parentElement) {
                    cs = getComputedStyle(e);
                    a = parseFloat(cs.opacity);
                    if (a < 1) { op *= a; solid.push(e); }
                    bg = rgba(cs.backgroundColor);
                    if (bg && bg[3] >= 0.5) { other = plated.indexOf(e) < 0; break; }   // the surface the words are on: the plate, or another
                    if (e === f) break;
                }
                if (other) continue;                      // words on a surface of their own (a badge) keep their colour
                c = rgba(getComputedStyle(hosts[i]).color);
                a = (c ? c[3] : 1) * op;
                tier = a >= 0.85 ? 1 : a >= 0.6 ? 2 : 3;
                mark(hosts[i], "data-lu-ink", String(tier));
                count++;
                for (j = 0; j < solid.length; j++) mark(solid[j], "data-lu-solid");
            }
            mark(f, "data-lu-root");
            return count;
        }
        function contrast(a, b) {
            var x = lum(a), y = lum(b);
            return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
        }
        function mix(ink, a, bg) {
            return [ink[0] * a + bg[0] * (1 - a), ink[1] * a + bg[1] * (1 - a), ink[2] * a + bg[2] * (1 - a)];
        }
        // What is behind an element: the first ancestor whose background is mostly opaque, else the page's.
        function behind(el) {
            var e, c;
            for (e = el.parentElement; e; e = e.parentElement) {
                c = rgba(getComputedStyle(e).backgroundColor);
                if (c && c[3] >= 0.5) return c;
            }
            c = rgba(getComputedStyle(document.documentElement).backgroundColor);
            return c && c[3] >= 0.5 ? c : [0, 0, 0, 1];
        }
        // The ring is the ink over the plate (or, with no plate, over what is around the item) and must be OPT.ring.floor:1 against both sides.
        // The sheet's is derived for LG's black; on a lighter surface (a popup's panel) the alpha goes up until it holds.
        function ringAlpha(plated, around, base) {
            var R = OPT.ring, a, c;
            for (a = base; a < 1; a = Math.round((a + 0.1) * 100) / 100) {
                c = mix(R.ink, a, plated ? R.plate : around);
                if ((!plated || contrast(c, R.plate) >= R.floor) && contrast(c, around) >= R.floor) return a;
            }
            return 1;
        }
        function ringColour(a) {
            return "rgba(" + OPT.ring.ink.join(", ") + ", " + a + ")";
        }
        function markVar(el, name, value) {
            el.style.setProperty(name, value);
            marks.push([el, name]);
        }
        function interactive(f, fa) {
            if (fa > 0.5 * innerWidth * innerHeight) return false;
            return /^(A|BUTTON|INPUT|SELECT|TEXTAREA|SUMMARY)$/.test(f.tagName) || f.hasAttribute("tabindex") || f.hasAttribute("role");
        }
        var ATTR = { ps: "data-lu-ps", pb: "data-lu-pb", pa: "data-lu-pa" };
        // One look at the focused item: the plate and its words when there is one, else the ring alone for an item that can be acted on.
        function apply(f) {
            releaseNear(f);
            clearMarks();
            found = false;
            var fr = f.getBoundingClientRect(), fa = fr.width * fr.height, plates = null, words, count = 0, i, ra;
            try {
                if (fa > 0 && fa <= 0.7 * innerWidth * innerHeight) {
                    words = wordsOf(f);
                    plates = words.length ? detect(f, fa, words) : null;
                }
                if (plates && plates.length) {
                    for (i = 0; i < plates.length; i++) mark(plates[i].el, ATTR[plates[i].kind]);
                    ra = ringAlpha(true, behind(plates[0].el), OPT.ring.base);
                    if (ra !== OPT.ring.base) for (i = 0; i < plates.length; i++) markVar(plates[i].el, "--st-ring", ringColour(ra));
                    count = recolour(f, plates, words);
                    found = true;
                    window.__lumatvFocus = { plates: plates.map(function (p) { return p.kind + ":" + p.el.tagName.toLowerCase() + ":" + Math.round(p.area); }), words: count,
                        ms: Math.round(performance.now() - born) };
                    if (watcher) watcher.takeRecords();                  // what our own marks changed is not news
                    return true;
                }
                if (interactive(f, fa)) {
                    mark(f, "data-lu-fr");
                    ra = ringAlpha(false, behind(f), OPT.ring.baseOut);
                    if (ra !== OPT.ring.baseOut) markVar(f, "--st-ring-out", ringColour(ra));
                }
                if (watcher) watcher.takeRecords();
            } catch (e) { clearMarks(); }
            return false;
        }
        function unwatch() {
            if (watcher) { watcher.disconnect(); watcher = null; }
        }
        // A plate that is not there at the focus is on its way: LG's class change (a mutation, seen before the frame is drawn), its colour
        // transition (its start, an event) or, failing both, a few looks at 60, 140 and 260 ms.
        function again(f) {
            if (document.activeElement !== f || found) { unwatch(); return; }
            if (apply(f)) unwatch();
        }
        function wait(f, n) {
            if (document.activeElement !== f || found) return;
            again(f);
            if (!found && n < DELAYS.length) timer = setTimeout(function () { wait(f, n + 1); }, DELAYS[n]);
        }
        document.addEventListener("focusin", function (e) {
            var f = e.target;
            if (!f || f.nodeType !== 1 || f === document.body || f === document.documentElement) return;
            clearTimeout(timer);
            unwatch();
            focusEl = f;
            born = performance.now();
            if (apply(f)) return;
            if (window.MutationObserver) {
                watched = 0;
                watcher = new MutationObserver(function () { if (++watched > 8) unwatch(); else again(f); });
                watcher.observe(f, { attributes: true, subtree: true, attributeFilter: ["class", "style"] });
            }
            timer = setTimeout(function () { wait(f, 1); }, DELAYS[0]);
        }, true);
        document.addEventListener("transitionrun", function (e) {
            if (!focusEl || found || queued || !e.target || e.target.nodeType !== 1 || !focusEl.contains(e.target)) return;
            queued = true;
            Promise.resolve().then(function () { queued = false; again(focusEl); });
        }, true);
        document.addEventListener("focusout", function (e) {
            if (e.target !== focusEl) return;
            clearTimeout(timer);
            unwatch();
            retireMarks();
            found = false;
            focusEl = null;
        }, true);
        //@@end look

        //@@part canvas
        // ---- the canvas (opt-in). LG paints some containers near-black itself (the header's band is one); every large near-black band is marked,
        // at load and after the page's changes settle, and store.css paints it the palette's canvas like the page behind it.
        var bareRuns = 0, bareTimer = 0;
        function bare() {
            var list = document.querySelectorAll("body div, body header, body nav, body section, body main, body aside"), vw = innerWidth, vh = innerHeight, i, el, r, c;
            for (i = 0; i < list.length && i < 1500; i++) {
                el = list[i];
                if (el.hasAttribute("data-lu-bare")) continue;
                r = el.getBoundingClientRect();
                if (r.width < vw * 0.6 || r.height < 24 || (r.height > vh * 0.6 && r.width < vw * 0.95)) continue;
                c = rgba(getComputedStyle(el).backgroundColor);
                if (c && c[3] >= 0.9 && lum(c) < 0.01) el.setAttribute("data-lu-bare", "");
            }
        }
        function bareSoon() {
            clearTimeout(bareTimer);
            if (bareRuns >= 12) return;
            bareTimer = setTimeout(function () { bareRuns++; try { bare(); } catch (e) { /* the band stays LG's */ } }, 500);
        }
        whenRoot(function (root) { new MutationObserver(bareSoon).observe(root, { childList: true, subtree: true }); });
        window.addEventListener("load", bareSoon);
        //@@end canvas

        if (sheetHead || CSS) adopt(sheetHead + CSS);

        //@@part motion
        // ---- the motion: no transition of the page runs longer than the cap (Lucent's card settle). A transition that is longer is
        // played faster, not cut, so it still ends, and `transitionend` still fires for whatever waits on it; a transition created
        // by script (Element.animate) is asked for the cap instead. An animation that repeats forever is left alone.
        var cap = OPT.capMs;
        document.addEventListener("transitionrun", function (event) {
            try {
                var running = event.target.getAnimations({ subtree: !!event.pseudoElement });
                for (var i = 0; i < running.length; i++) {
                    var a = running[i];
                    if (!(a instanceof CSSTransition) || a.transitionProperty !== event.propertyName) continue;
                    var d = a.effect.getComputedTiming().duration;
                    if (typeof d === "number" && d > cap) a.updatePlaybackRate(d / cap);
                }
            } catch (e) { /* a transition that cannot be sped up runs as LG wrote it */ }
        }, true);
        var animate = Element.prototype.animate;
        if (typeof animate === "function") {
            Element.prototype.animate = function (keyframes, options) {
                try {
                    if (typeof options === "number" && options > cap) {
                        options = cap;
                    } else if (options && typeof options === "object" && typeof options.duration === "number" &&
                               options.duration > cap && isFinite(options.iterations === undefined ? 1 : options.iterations)) {
                        options = Object.assign({}, options, { duration: cap });
                    }
                } catch (e) { /* the animation runs as asked */ }
                return animate.call(this, keyframes, options);
            };
        }
        //@@end motion

        //@@part lazy
        // ---- lazy pictures (off unless asked for): an image made by script waits until it is near the screen. A picture the page
        // loads through a hidden <img> to learn when it is ready would never be ready, so this is a switch of its own and
        // is judged on the set before anyone leaves it on.
        var create = Document.prototype.createElement;
        Document.prototype.createElement = function (name, options) {
            var element = create.call(this, name, options);
            try {
                if (typeof name === "string" && name.toLowerCase() === "img") {
                    element.loading = "lazy";
                    element.decoding = "async";
                }
            } catch (e) { /* the image loads as the page asked */ }
            return element;
        };
        //@@end lazy

        //@@part probe
        // ---- the probe (diagnostic, never on unless `lumatv store probe` asked for it). The set gives no way to read the store's markup (WAM's
        // inspector does not listen on this firmware), so the page tells it: what the page is made of (class names, structure, computed
        // styles of what matters, none of its words: text becomes its length), which families and variables it uses, and for each focused
        // item what the plate-finder saw. Everything is posted to tools/store_probe.py on a loopback port of the set, which writes files.
        (function () {
            var P = OPT.probe;
            if (!P || !P.url) return;
            var KEEP = { "class": 1, id: 1, role: 1, tabindex: 1, type: 1, "aria-selected": 1, "aria-hidden": 1, "aria-disabled": 1, "aria-expanded": 1,
                "aria-current": 1, "aria-pressed": 1, "aria-checked": 1, "aria-haspopup": 1 };
            var URLA = { src: 1, href: 1, srcset: 1, poster: 1, "data-src": 1 };
            var VARS = ["--sand-focus-bg-color", "--sand-focus-bg-color-rgb", "--sand-component-focus-text-color-rgb", "--sand-text-color-rgb",
                "--sand-text-sub-color", "--sand-component-text-color-rgb", "--sand-bg-color", "--sand-selected-color-rgb", "--sand-spinner-color",
                "--sand-overlay-bg-color-rgb", "--sand-shadow-color-rgb", "--sand-toggle-on-bg-color"];
            var sent = 0, lastFocus = 0, doms = 0, lastHref = "";
            function trimUrl(u) {
                u = String(u || "");
                if (/^data:/i.test(u)) return "data:";
                if (/^blob:/i.test(u)) return "blob:";
                try { var x = new URL(u, location.href); return x.protocol + "//" + x.host + x.pathname.slice(-90); } catch (e) { return u.slice(0, 40); }
            }
            function maskCss(s) {
                return String(s).replace(/url\(([^)]*)\)/g, function (m, u) { return "url(" + trimUrl(u.replace(/^["']|["']$/g, "")) + ")"; });
            }
            function beacon(kind, body) {
                try { new Image().src = P.url.replace(/\/p$/, "/b") + "?k=" + encodeURIComponent(kind) + "&d=" + encodeURIComponent(body.slice(0, 1500)); } catch (e) { /* none */ }
            }
            function post(kind, data) {
                var body;
                if (sent >= P.max) return;
                sent++;
                try {
                    body = JSON.stringify({ kind: kind, ms: Math.round(performance.now()), page: trimUrl(location.href), top: window.top === window, data: data });
                } catch (e) { return; }
                try {
                    fetch(P.url, { method: "POST", mode: "cors", headers: { "Content-Type": "text/plain" }, body: body })
                        .then(function (r) { return r.text(); })
                        .then(function (t) { console.info("lumatv-probe " + kind + " " + body.length + " " + t); })
                        .catch(function (e) { console.info("lumatv-probe " + kind + " not sent: " + e); beacon(kind, body); });
                } catch (e) { beacon(kind, body); }
            }
            function attrs(el) {
                var o = {}, i, a, n;
                for (i = 0; i < el.attributes.length; i++) {
                    a = el.attributes[i]; n = a.name;
                    if (KEEP[n] || /^data-lu-/.test(n)) o[n] = String(a.value).slice(0, 220);
                    else if (URLA[n]) o[n] = trimUrl(a.value);
                    else if (n === "style") o[n] = maskCss(a.value).slice(0, 300);
                    else o[n] = "#" + String(a.value).length;     // any other attribute: its name and the length of its value
                }
                return o;
            }
            function textLen(el) {
                var n = 0, k;
                for (k = el.firstChild; k; k = k.nextSibling) if (k.nodeType === 3) n += k.nodeValue.trim().length;
                return n;
            }
            function sty(cs, pcs) {
                var o = {}, v;
                v = cs.backgroundColor; if (v !== "rgba(0, 0, 0, 0)") o.bg = v;
                if (cs.backgroundImage !== "none") o.bi = maskCss(cs.backgroundImage).slice(0, 120);
                if (!pcs || cs.color !== pcs.color) o.co = cs.color;
                if (!pcs || cs.fontFamily !== pcs.fontFamily) o.ff = cs.fontFamily.slice(0, 80);
                if (!pcs || cs.fontWeight !== pcs.fontWeight) o.fw = cs.fontWeight;
                if (!pcs || cs.fontSize !== pcs.fontSize) o.fs = cs.fontSize;
                if (cs.opacity !== "1") o.op = cs.opacity;
                if (cs.boxShadow !== "none") o.bs = cs.boxShadow.slice(0, 100);
                if (cs.outlineStyle !== "none" && cs.outlineWidth !== "0px") o.ol = cs.outlineWidth + " " + cs.outlineColor;
                if (cs.transform !== "none") o.tf = cs.transform;
                if (cs.transitionDuration !== "0s") o.tr = cs.transitionProperty.slice(0, 50) + " " + cs.transitionDuration;
                if (cs.willChange !== "auto") o.wc = cs.willChange;
                if (cs.borderRadius !== "0px") o.br = cs.borderRadius;
                if (cs.position !== "static") o.po = cs.position;
                if (cs.overflowX !== "visible") o.ov = cs.overflowX;
                if (cs.cursor === "pointer") o.cu = 1;
                return o;
            }
            function pseudo(el, which) {
                var cs = getComputedStyle(el, "::" + which), o;
                if (cs.content === "none" || cs.content === "normal") return null;
                o = sty(cs, null);
                o.ct = cs.content.length; o.w = cs.width; o.h = cs.height; o.po = cs.position;
                return o;
            }
            function rec(el, i, p, full) {
                var cs = getComputedStyle(el), r = el.getBoundingClientRect(), o;
                o = { i: i, p: p, t: el.tagName.toLowerCase(), c: (typeof el.className === "string" ? el.className : (el.getAttribute("class") || "")).slice(0, 220),
                    a: attrs(el), r: [Math.round(r.left), Math.round(r.top), Math.round(r.width), Math.round(r.height)], x: textLen(el), s: sty(cs, el.parentElement ? getComputedStyle(el.parentElement) : null) };
                if (full) { o.b = pseudo(el, "before"); o.f = pseudo(el, "after"); }
                if (o.x > 0) { o.s.co = cs.color; o.s.fs = cs.fontSize; o.s.fw = cs.fontWeight; o.s.ff = cs.fontFamily.slice(0, 80); }    // a word's own look, inherited or not
                return o;
            }
            function serialize(root, budget, full, done) {
                var nodes = [], q = [[root, -1]], qi = 0;
                function slice() {
                    var t0 = performance.now(), e;
                    while (qi < q.length && nodes.length < budget && performance.now() - t0 < 14) {
                        e = q[qi++];
                        var i = nodes.length, k;
                        if (!/^(SCRIPT|STYLE|NOSCRIPT)$/.test(e[0].tagName)) {
                            nodes.push(rec(e[0], i, e[1], full));
                            for (k = e[0].firstElementChild; k; k = k.nextElementSibling) q.push([k, i]);
                        } else {
                            nodes.push({ i: i, p: e[1], t: e[0].tagName.toLowerCase() });
                        }
                    }
                    if (qi < q.length && nodes.length < budget) setTimeout(slice, 0);
                    else done(nodes, qi < q.length);
                }
                slice();
            }
            function env() {
                var out = { ua: navigator.userAgent, vp: [innerWidth, innerHeight, devicePixelRatio], origin: location.origin, ready: document.readyState,
                    parts: window.__lumatvStore, fonts: [], sheets: [], vars: {}, csp: [], families: {} }, i, f, s, cs, els, root = document.documentElement;
                try { document.fonts.forEach(function (ff) { out.fonts.push(ff.family + "|" + ff.weight + "|" + ff.style + "|" + ff.status); }); } catch (e) { /* none */ }
                for (i = 0; i < document.styleSheets.length && i < 40; i++) {
                    s = document.styleSheets[i];
                    try { out.sheets.push({ href: s.href ? trimUrl(s.href) : "inline", rules: s.cssRules.length }); } catch (e) { out.sheets.push({ href: s.href ? trimUrl(s.href) : "inline", rules: "blocked" }); }
                }
                els = [root, document.body, document.body && document.body.firstElementChild];
                for (f = 0; f < els.length; f++) {
                    if (!els[f]) continue;
                    cs = getComputedStyle(els[f]);
                    out.vars[f] = {};
                    for (i = 0; i < VARS.length; i++) { var v = cs.getPropertyValue(VARS[i]); if (v) out.vars[f][VARS[i]] = v.trim(); }
                }
                Array.prototype.forEach.call(document.querySelectorAll('meta[http-equiv]'), function (m) { out.csp.push(String(m.getAttribute("content")).slice(0, 300)); });
                els = document.body ? document.body.querySelectorAll("*") : [];
                for (i = 0; i < els.length && i < 3000; i++) {
                    if (textLen(els[i]) > 0) { var fam = getComputedStyle(els[i]).fontFamily.slice(0, 80); out.families[fam] = (out.families[fam] || 0) + 1; }
                }
                return out;
            }
            function focusSnap(f) {
                var chain = [], sub = [], q = [f], qi = 0, e, k, d = {};
                for (e = f; e && e.nodeType === 1 && chain.length < 8; e = e.parentElement) chain.push(rec(e, chain.length, chain.length + 1, true));
                while (qi < q.length && sub.length < 60) {
                    e = q[qi++];
                    sub.push(rec(e, sub.length, -1, true));
                    for (k = e.firstElementChild; k; k = k.nextElementSibling) q.push(k);
                }
                post("focus", { found: window.__lumatvFocus || null, chain: chain, sub: sub });
            }
            function dom(reason) {
                if (doms >= 6 || !document.body) return;
                doms++;
                serialize(document.body, 3500, true, function (nodes, truncated) { post("dom", { reason: reason, nodes: nodes, truncated: truncated }); });
            }
            document.addEventListener("focusin", function (e) {
                var f = e.target, now = Date.now();
                if (!f || f.nodeType !== 1 || now - lastFocus < 700) return;
                lastFocus = now;
                setTimeout(function () { try { focusSnap(document.activeElement || f); } catch (err) { post("error", String(err)); } }, 400);
            }, true);
            // How the page marks the item the remote moved to, when it is not by the browser's own focus (an item that is only given a class): the
            // classes that change within 700 ms of a key press, which key it was by kind (an arrow, enter, back: never a letter) and the elements.
            var keyAt = 0, keyKind = "", changes = [], flushed = 0, flushTimer = 0;
            function describe(el) {
                var c = (typeof el.className === "string" ? el.className : (el.getAttribute("class") || "")).split(/\s+/).filter(Boolean).slice(0, 3).join(".");
                return el.tagName.toLowerCase() + (c ? "." + c : "");
            }
            function names(s) { return String(s || "").split(/\s+/).filter(Boolean); }
            document.addEventListener("keydown", function (e) {
                var k = e.keyCode;
                keyAt = Date.now();
                keyKind = k >= 37 && k <= 40 ? "arrow" : k === 13 ? "enter" : (k === 461 || k === 8 || k === 27) ? "back" : "other";
                clearTimeout(flushTimer);
                flushTimer = setTimeout(function () {
                    if (!changes.length || flushed >= 12) return;
                    flushed++;
                    post("classes", { key: keyKind, changes: changes.splice(0, 60) });
                }, 800);
            }, true);
            if (window.MutationObserver) {
                new MutationObserver(function (list) {
                    var i, m, was, now, add, rem;
                    if (Date.now() - keyAt > 700) return;
                    for (i = 0; i < list.length && changes.length < 60; i++) {
                        m = list[i];
                        was = names(m.oldValue); now = names(m.target.getAttribute("class"));
                        add = now.filter(function (n) { return was.indexOf(n) < 0; }); rem = was.filter(function (n) { return now.indexOf(n) < 0; });
                        if (add.length || rem.length) changes.push({ el: describe(m.target), add: add.slice(0, 6), rem: rem.slice(0, 6) });
                    }
                }).observe(document, { attributes: true, subtree: true, attributeFilter: ["class"], attributeOldValue: true });
            }
            document.addEventListener("securitypolicyviolation", function (e) {
                post("csp", { directive: e.violatedDirective, blocked: trimUrl(e.blockedURI), disposition: e.disposition });
            }, true);
            window.addEventListener("error", function (e) { post("error", String(e.message).slice(0, 200) + " @" + trimUrl(e.filename) + ":" + e.lineno); }, true);
            window.addEventListener("load", function () {
                setTimeout(function () {
                    try { post("env", env()); } catch (err) { post("error", "env: " + err); }
                    try { dom("load"); } catch (err) { post("error", "dom: " + err); }
                    lastHref = location.href;
                }, 1500);
                // a page change (the store moves between its pages without a load): look again when it has settled
                setInterval(function () {
                    if (location.href !== lastHref) { lastHref = location.href; setTimeout(function () { try { dom("navigated"); } catch (err) { post("error", "dom: " + err); } }, 2500); }
                }, 1000);
            });
            post("hello", { parts: window.__lumatvStore, ready: document.readyState });
        })();
        //@@end probe
    } catch (e) {
        /* nothing here may stop the store from opening */
    }
})();
