// WAM runs webOSUserScripts/userScript.js from an app's own directory in every document the app's view loads, before the
// page's own scripts: in the Content Store's loader page and in the cloud pages the loader sends the view to (upstream:
// webosose/wam, WebPageBase::setCustomUserScript, which also covers pages on origins outside the app's directory).
// tools/store_theme.py fills the two markers below (OPTIONS_MARKER and CSS_MARKER in it) and writes the result into the overlay's upper
// layer. They are not brand placeholders in double braces: tools/build-ipk.sh fills those from brand.json and fails the package on any
// it leaves.
//
// The script changes how the page looks and how long its transitions take, and nothing else: it makes no request, reads
// nothing from the page, writes none of its words and clicks nothing, and every step is guarded, so that a failure leaves
// LG's page as it is.
(function () {
    "use strict";
    try {
        var OPT = /*@@OPTIONS@@*/ null;
        var CSS = /*@@CSS@@*/ "";
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
        // Once per document. The script can run before <html> exists (WAM runs it as the document is created), so nothing below
        // needs the root: the marker is put on it as soon as it is there.
        if (window.__lumatvStore) return;
        Object.defineProperty(window, "__lumatvStore", { value: OPT.parts.join(",") });
        var whenRoot = function (fn) {
            if (document.documentElement) { fn(document.documentElement); return; }
            var wait = new MutationObserver(function () {
                if (document.documentElement) { wait.disconnect(); fn(document.documentElement); }
            });
            wait.observe(document, { childList: true });
        };
        whenRoot(function (root) { root.setAttribute("data-lumatv-store", OPT.parts.join(",")); });
        var has = function (part) { return OPT.parts.indexOf(part) >= 0; };

        // The look: one constructable stylesheet adopted by the document. It needs no <head>, so it is in place before the first
        // frame, and a page's content security policy, which governs <style> elements, does not see it. A <style> element is the
        // fallback on an engine without constructable sheets.
        if (has("look")) {
            var sheet = null;
            try {
                if (window.CSSStyleSheet && "adoptedStyleSheets" in document && CSSStyleSheet.prototype.replaceSync) {
                    sheet = new CSSStyleSheet();
                    sheet.replaceSync(CSS);
                    document.adoptedStyleSheets = Array.prototype.slice.call(document.adoptedStyleSheets).concat([sheet]);
                }
            } catch (e) {
                sheet = null;
            }
            if (!sheet) {
                var style = document.createElement("style");
                style.textContent = CSS;
                whenRoot(function (root) { (document.head || root).appendChild(style); });
            }
        }

        // The motion: no transition of the page runs longer than the cap (Lucent's card settle). A transition that is longer is
        // played faster, not cut, so it still ends, and `transitionend` still fires for whatever waits on it; a transition created
        // by script (Element.animate) is asked for the cap instead. An animation that repeats forever is left alone.
        if (has("motion")) {
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
        }

        // Lazy pictures (off unless asked for): an image made by script waits until it is near the screen. A picture the page
        // loads through a hidden <img> to learn when it is ready would never be ready, so this is a switch of its own and
        // is judged on the set before anyone leaves it on.
        if (has("lazy")) {
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
        }
    } catch (e) {
        /* nothing here may stop the store from opening */
    }
})();
