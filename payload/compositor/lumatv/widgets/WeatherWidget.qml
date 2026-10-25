import QtQuick 2.4
import TextFile 1.0
import "../Saved.js" as Saved
import "../kit"

// Current conditions and forecast rows on the mock's boxes: h3 at 0, "now" row at 47 (icon
// 96, temperature 84px on a line-height-1 box, condition under it), stats at 179, a rule at
// 220 and 41px rows from 225 with columns 70 | icon | 56 | 56. The reading comes from the native
// helper (Helper.topics.weather: fetched, saved, aged and dropped there; the place and unit are its
// settings, base.weather). Until the helper has answered, the document it saved last time is read
// from {{dataDir}}/cache/weather.json, so a cold start draws the last reading at once.
Item {
    id: root
    property var theme: ({})
    property var entry: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6" })
    property bool isLight: false
    property var settings: ({})
    property bool focused: false
    property bool inner: false
    property bool live: true            // set by the host; the helper decides when to fetch
    property double fetchedAt: 0        // when the reading on screen was fetched
    readonly property string iconDir: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/icons/"

    function activate() {}

    property int maxConfiguredDays: (theme.weather && theme.weather.days) || 5
    property int tempSize: (theme.type && theme.type.temp) || 84

    property string place: ""
    property string temp: ""
    property string cond: ""
    property string stats: ""
    property string nowKind: ""
    property var days: []
    property int rowsTop: 225
    property int maxDays: Math.max(0, Math.min(maxConfiguredDays, Math.floor((height - rowsTop) / 41)))

    // The strings of a reading are the helper's `display`; a reading older than the one on screen,
    // or past its six hours, is not drawn.
    function take(doc) {
        if (!Saved.weatherUsable(doc, Date.now()) || doc.fetchedAt <= fetchedAt) return
        var d = doc.display
        place = d.place; temp = d.temp; cond = d.cond; stats = d.stats; nowKind = d.nowKind; days = d.days
        fetchedAt = doc.fetchedAt
    }
    TextFile { id: savedFile; name: "file://{{dataDir}}/cache/weather.json" }
    readonly property var topic: Helper.topics.weather || null
    onTopicChanged: if (topic) take(topic.doc)
    Component.onCompleted: {
        take(Saved.parse(savedFile.exists ? savedFile.read() : ""))
        if (topic) take(topic.doc)
    }


    Text {
        x: 0; y: 0
        text: root.place
        color: root.mat.ink2
        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
        renderType: Text.NativeRendering
    }

    Image {
        x: 0; y: 61
        width: 96; height: 96
        visible: root.nowKind !== ""
        source: root.nowKind ? root.iconDir + "wxbig-" + root.nowKind + ".png" : ""
    }
    Text {
        id: tempText
        x: 114
        y: 47 - Math.round(root.tempSize * 0.183)
        text: root.temp
        color: root.mat.ink
        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: root.tempSize
        font.letterSpacing: -root.tempSize * 0.03
        renderType: Text.NativeRendering
    }
    Text {
        x: 114; y: 135
        width: parent.width - 114
        text: root.cond
        elide: Text.ElideRight
        color: root.mat.ink2
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 26
        renderType: Text.NativeRendering
    }
    Text {
        x: 0; y: 179
        width: parent.width
        text: root.stats
        elide: Text.ElideRight
        color: root.mat.ink2
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
        renderType: Text.NativeRendering
    }

    Rectangle {
        visible: root.maxDays > 0 && root.days.length > 0
        x: 0; y: 220
        width: parent.width; height: 1
        color: root.isLight ? "#1f17131f" : "#1affffff"
    }

    Repeater {
        model: root.days.slice(0, root.maxDays)
        delegate: Item {
            x: 0
            y: root.rowsTop + index * 41
            width: root.width
            height: 41
            Text {
                x: 0; y: Math.round((41 - implicitHeight) / 2)
                text: modelData.day
                color: root.mat.ink
                font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 23
                renderType: Text.NativeRendering
            }
            Image {
                x: 70; y: 8
                width: 26; height: 26
                source: root.iconDir + "wx-" + modelData.kind + (root.isLight ? "_l" : "_d") + ".png"
            }
            Text {
                x: parent.width - 112; width: 56
                y: Math.round((41 - implicitHeight) / 2)
                horizontalAlignment: Text.AlignRight
                text: modelData.hi
                color: root.mat.ink
                font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 23
                renderType: Text.NativeRendering
            }
            Text {
                x: parent.width - 56; width: 56
                y: Math.round((41 - implicitHeight) / 2)
                horizontalAlignment: Text.AlignRight
                text: modelData.lo
                color: root.mat.ink
                font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 23
                renderType: Text.NativeRendering
            }
        }
    }
}
