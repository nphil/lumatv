import QtQuick 2.4
import "../kit"
import "Rows.js" as R
import "Choice.js" as C
import "."

// USB Device (LG: Sound › Sound Out › Use Wired Speaker › USB Device): the sound cards the TV sees on its USB ports, as `audio/tv getSoundCardsState`
// lists them (`soundCards`: number, name, playbackState "connected" while one plays, selected). OK on a card is what LG's list does: `audio setSoundOutput
// {soundOutput: "usb_speaker", cardNumber}`. The Sound page shows the way here only where the TV's config `profile.list` has "usbSoundOutput", as LG shows
// the row.
PageList {
    id: page
    title: "USB Device"
    back: "Sound"
    backId: "sound"

    property var cards: []                     // [{number, name, playing}]
    LunaCall {
        id: luna
        onAnswered: {
            if (tag !== "cards" || !doc || doc.returnValue === false || !doc.soundCards) return
            var out = []
            for (var i = 0; i < doc.soundCards.length; i++) out.push({ number: doc.soundCards[i].number, name: String(doc.soundCards[i].name), playing: doc.soundCards[i].playbackState === "connected" })
            page.cards = out
        }
    }
    Timer { id: reread; interval: 600; onTriggered: luna.call("cards", "luna://com.webos.service.audio/tv", "getSoundCardsState", {}) }

    function refresh() { luna.call("cards", "luna://com.webos.service.audio/tv", "getSoundCardsState", {}) }
    onOpened: refresh()

    function useCard(c) {
        luna.act("luna://com.webos.service.audio", "setSoundOutput", { soundOutput: "usb_speaker", cardNumber: c.number })
        reread.restart()
    }
    function cardRow(c) {
        return { kind: "value", id: "card:" + c.number, icon: "usb", label: c.name, value: c.playing ? "In use" : "", chevron: false, act: function() { page.useCard(c) } }
    }
    function build() {
        var out = [R.head("Device List")]
        if (!cards.length) out.push(R.note("No USB sound device is connected."))
        for (var i = 0; i < cards.length; i++) out.push(cardRow(cards[i]))
        return out
    }
    rows: build()
}
