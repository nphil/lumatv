import QtQuick 2.4
import TextFile 1.0
import "../Saved.js" as Saved
import "../kit"

// Current conditions and the days ahead, drawn for the room the card has (its content box, `width` x `height`):
//  stack    a narrow card (under 620): the mock's own boxes - h3 at 0, "now" row at 47 (icon 96, temperature 84 on a line-height-1 box,
//           condition under it), stats at 179, a rule at 220, then a row a day from 225 with the day, its icon, its low, a bar for how far
//           its low and high reach on the week's scale, and its high. The rows share what height is left, as many days as fit.
//  list     a wide card (620 to 1100) at least 240 high: "now" on the left with the place above it and the stats under it, the week as
//           rows on the right, each as wide as the room, so the bar is the middle of it and nothing is left empty.
//  columns  a wide, short card, or a very wide one: "now" on the left, a column a day on the right (day, icon, high, low, and a standing
//           bar when the card is tall enough).
// The reading comes from the native helper (Helper.topics.weather: fetched, saved, aged and dropped there; the place and unit are its
// settings, base.weather). Until the helper has answered, the document it saved last time is read from {{dataDir}}/cache/weather.json,
// so a cold start draws the last reading at once. The helper keeps seven days; `theme.weather.days` is how many the narrow card shows,
// the wide ones show the week.
Item {
    id: root
    property var theme: ({})
    property var entry: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6" })
    property bool isLight: false
    property var settings: ({})
    property bool focused: false
    // The muted ink of this card: the material's ink at 0.8. Its own second ink (0.57) read 2.6 to 2.9:1 over the picture; 3:1 is the floor.
    property color inkBase: mat.ink || "#F2EFF6"
    readonly property color soft: Qt.rgba(inkBase.r, inkBase.g, inkBase.b, 0.8)
    property bool inner: false
    property bool live: true            // set by the host; the helper decides when to fetch
    property double fetchedAt: 0        // when the reading on screen was fetched
    readonly property string iconDir: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/icons/"

    function activate() {}

    property int maxConfiguredDays: (theme.weather && theme.weather.days) || 5
    property int tempToken: (theme.type && theme.type.temp) || 84

    property string place: ""
    property string temp: ""
    property string cond: ""
    property string stats: ""
    property string nowKind: ""
    property var days: []

    // ---- the reading as numbers: the week's coolest low and warmest high are the ends of every bar
    function degrees(s) { var m = /-?[0-9]+(\.[0-9]+)?/.exec(String(s)); return m ? parseFloat(m[0]) : NaN }
    readonly property var week: days.map(function(d) { return { day: d.day, kind: d.kind, hi: d.hi, lo: d.lo, hiN: degrees(d.hi), loN: degrees(d.lo) } })
    readonly property real weekMin: { var m = Infinity; for (var i = 0; i < week.length; i++) if (week[i].loN < m) m = week[i].loN; return isFinite(m) ? m : 0 }
    readonly property real weekMax: { var m = -Infinity; for (var i = 0; i < week.length; i++) if (week[i].hiN > m) m = week[i].hiN; return isFinite(m) ? m : 1 }
    // "Feels like 57° · 53% humidity · 6 mph" as its three parts, for the wide card's column of stats
    readonly property var statParts: stats === "" ? [] : stats.split(" \u00b7 ")

    // ---- the size class
    readonly property bool wide: width >= 620
    readonly property string mode: !wide ? "stack" : (width < 1100 && height >= 240) ? "list" : "columns"
    readonly property int pad: 48                                       // between "now" and the week on a wide card

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

    // ---- "now": the place, the icon, the temperature, the condition. The mock's boxes on the narrow card; on a wide one the same parts
    // are sized to the room and set apart (the place at the top, "now" in the middle, the stats under it). A card under 130 high (two
    // rows) has the icon and the temperature level with each other and nothing else.
    readonly property bool empty: temp === "" && place === ""
    readonly property bool compact: height < 130
    readonly property int nowIcon: compact ? Math.max(40, Math.min(64, height - 8)) : mode === "stack" ? 96 : height >= 300 ? 112 : height >= 200 ? 96 : 72
    readonly property int nowTemp: compact ? Math.round(nowIcon * 0.875) : mode === "stack" ? tempToken : Math.round(nowIcon * tempToken / 96)
    readonly property int nowWidth: mode === "stack" ? width : mode === "list" ? Math.max(300, Math.min(400, Math.round(width * 0.4))) : Math.max(300, Math.min(440, Math.round(width * 0.3)))
    readonly property int showPlace: compact || height < 110 ? 0 : 1
    readonly property int condSize: nowIcon >= 96 ? 26 : 22
    readonly property int condH: nowIcon >= 96 ? 36 : 30
    readonly property bool showCond: !compact
    // the icon and the temperature with the condition under it, and the stats under that
    readonly property int coreH: compact ? nowIcon : Math.max(nowIcon, nowTemp + 4 + condH)
    readonly property int statLines: mode === "stack" || compact ? 0 : Math.max(0, Math.min(statParts.length, Math.floor((height - showPlace * 40 - coreH - 12) / 32)))
    readonly property int blockH: coreH + (statLines > 0 ? 12 + statLines * 32 : 0)
    readonly property int nowTop: mode === "stack" && !compact ? 47 : showPlace * 40 + Math.max(0, Math.round((height - showPlace * 40 - blockH) / 2))
    readonly property int tempTop: nowTop + (compact ? Math.round((nowIcon - nowTemp) / 2) : 0)

    Text {
        visible: root.showPlace === 1 && !root.empty
        x: 0; y: 0
        width: root.mode === "stack" ? parent.width : root.nowWidth
        text: root.place
        elide: Text.ElideRight
        color: root.soft
        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
        renderType: Text.NativeRendering
    }
    Image {
        x: 0; y: root.mode === "stack" && !root.compact ? 61 : root.nowTop
        width: root.nowIcon; height: root.nowIcon
        visible: root.nowKind !== ""
        source: root.nowKind ? root.iconDir + "wxbig-" + root.nowKind + ".png" : ""
        smooth: true
    }
    Text {
        id: tempText
        x: root.nowIcon + Math.round(root.nowIcon * 18 / 96)
        y: root.tempTop - Math.round(root.nowTemp * 0.183)
        text: root.temp
        color: root.mat.ink
        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: root.nowTemp
        font.letterSpacing: -root.nowTemp * 0.03
        renderType: Text.NativeRendering
    }
    Text {
        visible: root.showCond
        x: tempText.x
        y: root.nowTop + root.nowTemp + 4
        width: Math.max(0, root.nowWidth - x)
        text: root.cond
        elide: Text.ElideRight
        color: root.soft
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: root.condSize
        renderType: Text.NativeRendering
    }
    // the stats: one line under the narrow card's "now" row, one line a part under a wide card's
    Text {
        visible: root.mode === "stack" && !root.compact && root.height >= 200
        x: 0; y: 179
        width: parent.width
        text: root.stats
        elide: Text.ElideRight
        color: root.soft
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 20
        renderType: Text.NativeRendering
    }
    Repeater {
        model: root.statLines
        delegate: Text {
            x: 0
            y: root.nowTop + root.coreH + 12 + index * 32
            width: root.nowWidth
            text: root.statParts[index]
            elide: Text.ElideRight
            color: root.soft
            font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 22
            renderType: Text.NativeRendering
        }
    }
    // nothing read yet: the card says so, in its own words, and draws no reading
    Text {
        visible: root.empty
        anchors.centerIn: parent
        width: parent.width
        horizontalAlignment: Text.AlignHCenter
        text: "No weather reading yet"
        color: root.soft
        font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 24
        renderType: Text.NativeRendering
    }

    // ---- stack: the rows under the rule
    readonly property int rowsTop: 225
    readonly property int stackDays: Math.max(0, Math.min(maxConfiguredDays, week.length, Math.floor((height - rowsTop) / 41)))
    readonly property int stackRow: stackDays > 0 ? Math.max(41, Math.min(56, Math.floor((height - rowsTop) / stackDays))) : 41
    Rectangle {
        visible: root.mode === "stack" && root.stackDays > 0
        x: 0; y: 220
        width: parent.width; height: 1
        color: root.isLight ? "#1f17131f" : "#1affffff"
    }
    Repeater {
        model: root.mode === "stack" ? root.week.slice(0, root.stackDays) : []
        delegate: Item {
            x: 0
            y: root.rowsTop + index * root.stackRow
            width: root.width
            height: root.stackRow
            WeatherDayRow { anchors.fill: parent; day: modelData; compact: true; mat: root.mat; isLight: root.isLight; iconDir: root.iconDir; weekMin: root.weekMin; weekMax: root.weekMax }
        }
    }

    // ---- list: the week as rows on the right
    readonly property int listX: nowWidth + pad
    readonly property int listDays: Math.min(week.length, 7)
    readonly property int listRow: listDays > 0 ? Math.max(40, Math.min(58, Math.floor(height / listDays))) : 40
    Repeater {
        model: root.mode === "list" ? root.week.slice(0, root.listDays) : []
        delegate: Item {
            x: root.listX
            y: Math.round((root.height - root.listDays * root.listRow) / 2) + index * root.listRow
            width: root.width - root.listX
            height: root.listRow
            WeatherDayRow { anchors.fill: parent; day: modelData; mat: root.mat; isLight: root.isLight; iconDir: root.iconDir; weekMin: root.weekMin; weekMax: root.weekMax }
            Rectangle {
                visible: index > 0
                x: 0; y: 0; width: parent.width; height: 1
                color: root.isLight ? "#1f17131f" : "#1affffff"
            }
        }
    }

    // ---- columns: a column a day on the right
    readonly property int colsX: nowWidth + pad
    readonly property int colMin: height >= 240 ? 128 : 96
    readonly property int colCount: Math.max(0, Math.min(week.length, Math.floor((width - colsX) / colMin)))
    readonly property real colW: colCount > 0 ? (width - colsX) / colCount : 0
    Repeater {
        model: root.mode === "columns" ? root.week.slice(0, root.colCount) : []
        delegate: Item {
            x: root.colsX + Math.round(index * root.colW)
            y: 0
            width: Math.round(root.colW)
            height: root.height
            readonly property bool tall: root.height >= 240
            readonly property int icon: tall ? 48 : 36
            // day, icon, high, [bar], low, stacked and centred in the column; the bar takes what the rest leaves
            readonly property int head: 26 + 10 + icon + 8
            readonly property int foot: 30 + 4
            readonly property int barH: Math.max(0, height - head - foot - 30 - 16)
            Text {
                x: 0; y: 0; width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: modelData.day
                color: root.soft
                font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 22
                renderType: Text.NativeRendering
            }
            Image {
                x: Math.round((parent.width - parent.icon) / 2); y: 34
                width: parent.icon; height: parent.icon
                source: root.iconDir + "wxbig-" + modelData.kind + ".png"
                smooth: true
            }
            Text {
                x: 0; y: parent.head + 4; width: parent.width
                horizontalAlignment: Text.AlignHCenter
                text: modelData.hi
                color: root.mat.ink
                font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 26
                renderType: Text.NativeRendering
            }
            RangeBar {
                visible: parent.tall && parent.barH >= 40
                x: 0; y: parent.head + 4 + 34; width: parent.width; height: parent.barH
                vertical: true
                thickness: 8
                lo: modelData.loN; hi: modelData.hiN; min: root.weekMin; max: root.weekMax
            }
            Text {
                x: 0; width: parent.width
                y: parent.tall && parent.barH >= 40 ? parent.head + 4 + 34 + parent.barH + 6 : parent.head + 4 + 32
                horizontalAlignment: Text.AlignHCenter
                text: modelData.lo
                color: root.soft
                font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 24
                renderType: Text.NativeRendering
            }
        }
    }
}
