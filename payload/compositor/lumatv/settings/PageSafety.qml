import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "Names.js" as N
import "."

// Safety (LG: General › System › Safety): which locks are on. This page only reads. LG changes a lock after its PIN prompt, and the setting
// itself takes the new value from anyone who writes it, so a page that wrote `lock.parentalControl` or a rating would open the lock without
// the PIN. Changing a lock stays in LG's own screen (the last row). The PIN (`lock.systemPin`) is never read. Keys, as LG's rows read them:
// `lock.parentalControl` ("true" or "false"), the TV ratings (`tvRatingGeneralAge`, Dialog, Language, Sex, Violence and the children's two; "off"
// is no lock), `lock.movieRating`, `lock.applockPerApp` and `lock.inputBlock` (a bit mask of the inputs, 0 for none).
PageList {
    id: page
    title: "Safety"
    back: "System"

    readonly property var tvRatings: ["tvRatingGeneralAge", "tvRatingGeneralDialog", "tvRatingGeneralLanguage", "tvRatingGeneralSex", "tvRatingGeneralViolence",
                                      "tvRatingChildrenAge", "tvRatingChildrenFantasy"]
    function refresh() {
        Sio.read("lock", ["parentalControl", "movieRating", "applockPerApp", "inputBlock"])
        Sio.read("lock", tvRatings)
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    function flag(ds) { return !Sio.has(ds) ? "" : String(Sio.get(ds)) === "true" ? "On" : "Off" }
    function ratingsOn() {
        var any = false, seen = false
        for (var i = 0; i < tvRatings.length; i++) {
            var ds = "lock." + tvRatings[i]
            if (!Sio.has(ds)) continue
            seen = true
            if (String(Sio.get(ds)) !== "off") any = true
        }
        return seen ? (any ? "On" : "Off") : ""
    }
    function row(icon, label, ds, value, help) {
        return { kind: "value", id: label, icon: icon, label: label, chevron: false, help: help,
                 visible: function() { return !Sio.missing(ds) || ds === "lock.tvRatings" },
                 detail: function() { return ds === "lock.tvRatings" || Sio.has(ds) ? "" : "TV not answering" },
                 value: function() { var v = value(); return v === "" ? "\u2013" : v } }
    }

    rows: [
        row("lock", "Lock System", "lock.parentalControl", function() { return page.flag("lock.parentalControl") },
            "On: the locks below ask for the PIN."),
        row("tv", "TV Program Locks", "lock.tvRatings", function() { return page.ratingsOn() },
            "Blocks programs above the ratings set, until the PIN is typed."),
        row("film", "Movie Lock", "lock.movieRating", function() { return Sio.has("lock.movieRating") ? N.name("movieRating", Sio.get("lock.movieRating")) : "" },
            "The highest movie rating that plays without the PIN."),
        row("layout-grid", "Application Locks", "lock.applockPerApp", function() { return page.flag("lock.applockPerApp") },
            "Apps that ask for the PIN to open."),
        row("cable", "Input Locks", "lock.inputBlock", function() { return !Sio.has("lock.inputBlock") ? "" : Sio.num("lock.inputBlock", 0) > 0 ? "On" : "Off" },
            "Inputs that ask for the PIN to open."),
        R.lg(page, { id: "changelocks", target: "parentalControls", label: "Change locks", help: "A lock changes only with the PIN, in LG's screen." })
    ]
}
