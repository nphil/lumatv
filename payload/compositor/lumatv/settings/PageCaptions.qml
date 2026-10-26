import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// Closed Caption (LG: General › Accessibility › Closed Caption, and Subtitles): the caption switch, the analog and digital services,
// where the captions sit, and what they look like (LG's Digital Mode Option: style, size, font, colours, opacities, edge, window).
// The look rows answer only while the style is Custom, as in LG's own screen. Every row is LG's own key in the `caption` category
// (and `option.subtitleLanguageFirst` and `Second`), its values the TV's own list.
// NEXTGEN TV Mode waits for the ATSC 3.0 tuner (`system.supportATSC3tuner`), as LG's does.
PageList {
    id: page
    title: "Closed Caption"
    back: "System"
    tabs: ["Captions", "Text", "Background", "Subtitles"]
    readonly property bool custom: String(Sio.get("caption.captionStyle")) === "manual"

    // LG shows NEXTGEN TV Mode only on a TV with an ATSC 3.0 tuner
    Atsc3 { id: tuner }
    property alias atsc3: tuner.present

    function refresh() {
        Sio.read("caption", ["captionEnable", "captionAnalog", "captionDigital", "digitalPlusCaptionLanguage", "captionStyle", "captionSize", "captionFont",
                             "captionTextColor", "captionTextOpacity", "captionBgColor", "captionBgOpacity", "captionEdgeType", "captionEdgeColor",
                             "captionWindowColor", "captionWindowOpacity", "captionDigitalPosOption", "hardOfHearing", "aiCaptionEnable"])
        Sio.read("option", ["subtitleLanguageFirst", "subtitleLanguageSecond"])
    }
    Component.onCompleted: refresh()
    onOpened: refresh()

    // a look row: answers only while the style is Custom
    function look(row) {
        return R.over(row, { enabled: function() { return page.custom && row.enabled() }, detail: function() { return page.custom ? "" : "Set Style to Custom" } })
    }
    function pick(ds, tab, icon, label, names) { return look(C.choice(Sio, { ds: ds, tab: tab, icon: icon, label: label, names: names, page: page })) }

    rows: [
        R.toggle(Sio, { ds: "caption.captionEnable", tab: 0, icon: "type", label: "Closed Caption" }),
        C.choice(Sio, { ds: "caption.captionAnalog", tab: 0, icon: "tv", label: "Analog Mode", names: "captionAnalog", page: page }),
        C.choice(Sio, { ds: "caption.captionDigital", tab: 0, icon: "monitor", label: "Digital Mode", names: "captionDigital", page: page }),
        R.over(C.choice(Sio, { ds: "caption.digitalPlusCaptionLanguage", tab: 0, icon: "languages", label: "NEXTGEN TV Mode", names: "digitalPlusCaption", page: page }),
               { visible: function() { return page.atsc3 && C.shown(Sio, "caption.digitalPlusCaptionLanguage") } }),
        C.choice(Sio, { ds: "caption.captionDigitalPosOption", tab: 0, icon: "layout-grid", label: "Position Settings", names: "captionPosOption", page: page,
                        values: ["auto", "autoAi", "splitDown16_9", "splitUp16_9"] }),
        C.lgShown(Sio, "caption.captionStyle", R.seg(Sio, { ds: "caption.captionStyle", tab: 0, icon: "paintbrush", label: "Style",
                                                           options: ["Set by program", "Custom"], values: ["auto", "manual"] })),
        C.lgShown(Sio, "caption.hardOfHearing", R.toggle(Sio, { ds: "caption.hardOfHearing", tab: 0, icon: "volume-1", label: "Hard of Hearing" })),
        C.lgShown(Sio, "caption.aiCaptionEnable", R.toggle(Sio, { ds: "caption.aiCaptionEnable", tab: 0, icon: "sparkles", label: "AI Caption" })),

        look(C.lgShown(Sio, "caption.captionSize", R.seg(Sio, { ds: "caption.captionSize", tab: 1, icon: "type", label: "Size",
                                                               options: ["Small", "Standard", "Large"], values: ["small", "standard", "large"] }))),
        pick("caption.captionFont", 1, "type", "Font", "captionFont"),
        pick("caption.captionTextColor", 1, "palette", "Text Color", "captionColor"),
        look(C.lgShown(Sio, "caption.captionTextOpacity", R.seg(Sio, { ds: "caption.captionTextOpacity", tab: 1, icon: "blend", label: "Text Opacity",
                                                                       options: ["Flash", "Translucent", "Transparent", "Solid"], values: ["flash", "translucent", "transparent", "solid"] }))),
        pick("caption.captionEdgeType", 1, "square", "Edge Type", "captionEdgeType"),
        pick("caption.captionEdgeColor", 1, "palette", "Edge Color", "captionColor"),

        pick("caption.captionBgColor", 2, "palette", "BG Color", "captionColor"),
        look(C.lgShown(Sio, "caption.captionBgOpacity", R.seg(Sio, { ds: "caption.captionBgOpacity", tab: 2, icon: "blend", label: "BG Opacity",
                                                                      options: ["Flash", "Translucent", "Transparent", "Solid"], values: ["flash", "translucent", "transparent", "solid"] }))),
        pick("caption.captionWindowColor", 2, "palette", "Window Color", "captionColor"),
        look(C.lgShown(Sio, "caption.captionWindowOpacity", R.seg(Sio, { ds: "caption.captionWindowOpacity", tab: 2, icon: "blend", label: "Window Opacity",
                                                                         options: ["Translucent", "Transparent", "Solid"], values: ["translucent", "transparent", "solid"] }))),

        C.choice(Sio, { ds: "option.subtitleLanguageFirst", tab: 3, icon: "languages", label: "Subtitle First Choice", names: "digitalPlusCaption", page: page }),
        C.choice(Sio, { ds: "option.subtitleLanguageSecond", tab: 3, icon: "languages", label: "Subtitle Second Choice", names: "digitalPlusCaption", page: page })
    ]
}
