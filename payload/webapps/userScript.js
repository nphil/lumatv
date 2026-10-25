// WebAppMgr runs webOSUserScripts/userScript.js from an app's own directory in
// that app's page, before the app's own scripts. That is the whole injection
// point: the stylesheet beside it is linked in, and nothing about the app's
// bundle is touched.
//
// A <link> rather than an inline <style>: the file is on the same file:// origin,
// the engine caches its parsed rules, and editing it while iterating means
// reloading the app rather than re-staging anything.
(function () {
    "use strict";

    // The base sheet, then the palette generated from theme.json - so a
    // wallpaper or theme change re-tints the menus without touching this file.
    var HREFS = ["webOSUserScripts/{{shortId}}.css", "webOSUserScripts/theme.css"];

    function install() {
        if (document.head.querySelector('link[data-{{shortId}}]')) return;
        HREFS.forEach(function (href) {
            var link = document.createElement("link");
            link.rel = "stylesheet";
            link.href = href;
            link.setAttribute("data-{{shortId}}", "1");
            document.head.appendChild(link);
        });
    }

    if (document.head) {
        install();
    } else {
        // The script can run before <head> exists; this is cheaper and more
        // reliable than waiting for DOMContentLoaded, which some webOS apps
        // race by rendering from a synchronous bundle.
        var observer = new MutationObserver(function () {
            if (document.head) {
                observer.disconnect();
                install();
            }
        });
        observer.observe(document.documentElement, { childList: true });
    }
})();
