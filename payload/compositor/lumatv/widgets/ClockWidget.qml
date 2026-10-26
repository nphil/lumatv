import QtQuick 2.4

// Clock card in two styles (theme.clock.style or the layout entry's settings.style):
//  "digital": greeting, time, date on the mock's CSS boxes (h3 at 0, a 92px line-height-1 box at y=43, date at y=143; Manrope's 1.366em
//             ascent+descent puts the glyph box 0.183em above a line-height-1 box, applied straight to the Text y).
//  "analog":  a face drawn once into a Canvas, three hands that are pure rotation transforms (the second hand ticks with a short
//             overshoot), and the weekday, date and week strip beside it.
// Both are drawn for the room the card has (its content box, `width` x `height`), by `plan()`, and nothing is clipped or left empty:
//  tiny     under 120 high or 260 wide: the time (or the dial) and, if there is room, the date
//  regular  the mock's own: the dial or the time with the weekday, the date and the week strip at the bottom, as much as fits
//  zoned    700 or wider and under 230 high: the dial or the time, the weekday and date, and the week strip spread across the card
//  column   under 560 wide and 300 or more high: everything stacked and centred, the week strip under it
//  month    560 wide and 230 high or more: the dial or the time down the left, the month to its right (a second and a third month when
//           the room allows)
// Text is whole-pixel native text; the second hand moves on Home's shared tick and nothing else here runs per frame.
Item {
    id: root
    property var theme: ({})
    property var entry: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6", ink3: "#6BF2EFF6" })
    property bool isLight: false
    property var settings: ({})
    property bool focused: false
    property bool inner: false
    property bool live: true

    function activate() {}

    property string style: settings.style || (theme.clock && theme.clock.style) || "digital"
    property bool analog: style === "analog"
    property bool twelveHour: (theme.clock && theme.clock.twelveHour) !== false
    property bool showGreeting: (theme.clock && theme.clock.greeting) !== false
    property int timeToken: (theme.type && theme.type.clock) || 92
    property color inkC: mat.ink || "#F2EFF6"
    property color ink2C: mat.ink2 || "#A8F2EFF6"
    property color ink3C: mat.ink3 || "#6BF2EFF6"
    property color secondC: (theme.clock && theme.clock.secondHand) || "#ff3b5c"
    readonly property color onInk: isLight ? "#ffffff" : "#120e1a"

    // ------------------------------------------------------------------ the plan
    // Width of "12:45" and of its AM/PM at a time size (Manrope SemiBold: digits 0.62 em, the colon 0.28, the tracking -0.03 em, the
    // AM/PM 0.35 em of the time's with a 10 px gap).
    function timeW(ts, ampm) { return Math.round(ts * 2.63 + (ampm ? ts * 0.5 + 10 : 0)) }
    // The largest time size, from the token down, whose width fits `maxW` and whose height with `extra` under it fits `maxH`.
    function fitTime(token, maxW, maxH, ampm, extra) {
        var sizes = [token, 80, 68, 56, 48, 40]
        for (var i = 0; i < sizes.length; i++) if (sizes[i] <= token && timeW(sizes[i], ampm) <= maxW && sizes[i] + extra <= maxH) return sizes[i]
        return 40
    }
    function clampInt(v, a, b) { return Math.max(a, Math.min(b, Math.round(v))) }

    // { mode, dia, ts, greet, dateOn, face: {x, y}, info: {x, y, w} | null, strip: {x, y, w, cell} | null, months: {x, y, cell, count, w} | null }
    function plan(w, h, analog, greeting, token, ampm) {
        var p = { mode: "regular", dia: 0, ts: 0, greet: false, dateOn: false, face: { x: 0, y: 0 }, info: null, strip: null, months: null }
        var GAP = 36
        if (w < 260 || h < 120) {
            p.mode = "tiny"
            if (analog) {
                p.dia = clampInt(Math.min(h, w, 120), 40, 120)
                p.face = { x: 0, y: Math.round((h - p.dia) / 2) }
                if (w - p.dia >= 150 && h >= 64) p.info = { x: p.dia + 24, y: Math.round((h - 64) / 2), w: w - p.dia - 24 }
                else p.face.x = Math.round((w - p.dia) / 2)
            } else {
                p.ts = fitTime(Math.min(token, 64), w, h, ampm, 0)
                p.dateOn = h - p.ts >= 34
                p.face = { x: 0, y: Math.round((h - p.ts - (p.dateOn ? 34 : 0)) / 2) }
            }
            return p
        }
        if (w < 560 && h >= 300) {
            p.mode = "column"
            if (analog) {
                p.dia = clampInt(Math.min(200, h - 166, w), 96, 200)
                var ch = p.dia + 16 + 64 + 16 + 70
                var y0 = Math.round((h - ch) / 2)
                p.face = { x: Math.round((w - p.dia) / 2), y: y0 }
                p.info = { x: 0, y: y0 + p.dia + 16, w: w, center: true }
                var cellC = clampInt(Math.min(56, w / 7), 28, 56)
                p.strip = { x: Math.round((w - 7 * cellC) / 2), y: y0 + p.dia + 16 + 64 + 16, w: 7 * cellC, cell: cellC }
            } else {
                p.ts = fitTime(token, w, h, ampm, 0)
                p.greet = greeting
                p.dateOn = true
                var fh = (p.greet ? 43 : 0) + p.ts + 8 + 34
                var cellD = clampInt(Math.min(56, w / 7), 28, 56)
                var chd = fh + 24 + 70
                var y1 = Math.round((h - chd) / 2)
                p.face = { x: 0, y: y1, center: true, w: w }
                p.strip = { x: Math.round((w - 7 * cellD) / 2), y: y1 + fh + 24, w: 7 * cellD, cell: cellD }
            }
            return p
        }
        if (w >= 560 && h >= 230) {
            p.mode = "month"
            var colW, y2
            if (analog) {
                p.dia = clampInt(Math.min(h - 84, 220, w * 0.32), 96, 220)
                colW = Math.max(p.dia, 190)
                y2 = Math.round((h - (p.dia + 80)) / 2)
                p.face = { x: Math.round((colW - p.dia) / 2), y: y2 }
                p.info = { x: 0, y: y2 + p.dia + 14, w: colW, center: true }
            } else {
                p.greet = greeting
                p.ts = fitTime(token, Math.round(w * 0.45), h, ampm, 0)
                p.dateOn = true
                colW = timeW(p.ts, ampm)
                var bh = (p.greet ? 43 : 0) + p.ts + 8 + 34
                p.face = { x: 0, y: Math.round((h - bh) / 2) }
            }
            var rx = colW + 48, rw = w - rx
            var cell = clampInt(Math.min(64, rw / 7, (h - 44) / 7), 24, 64)
            var count = clampInt(Math.floor((rw + 48) / (7 * cell + 48)), 1, 3)
            var mw = count * 7 * cell + (count - 1) * 48
            p.months = { x: rx + Math.round((rw - mw) / 2), y: 0, cell: cell, count: count, w: mw }
            return p
        }
        if (w >= 700) {
            p.mode = "zoned"
            var faceW, infoW = 0, stripCell
            if (analog) {
                p.dia = clampInt(Math.min(h, 166), 96, 166)
                faceW = p.dia
                infoW = 230
            } else {
                p.greet = greeting && h >= 150
                var spread = w >= 1100                   // room for the weekday and the date between the time and the week
                p.dateOn = !spread
                p.ts = fitTime(token, Math.round(w * 0.5), h - (p.greet ? 43 : 0) - (p.dateOn ? 34 : 0), ampm, 0)
                faceW = timeW(p.ts, ampm)
                if (spread) infoW = 230
            }
            stripCell = clampInt(Math.min(64, (w - faceW - infoW - 2 * GAP) / 7), 36, 64)
            var stripW = 7 * stripCell
            var spare = w - faceW - infoW - stripW
            if (spare < 2 * 24 && stripCell > 36) { stripCell = 36; stripW = 252; spare = w - faceW - infoW - stripW }
            var gap = Math.max(24, Math.round(spare / (infoW > 0 ? 2 : 1)))
            if (analog) {
                p.face = { x: 0, y: Math.round((h - p.dia) / 2) }
                p.info = { x: faceW + gap, y: Math.round((h - 70) / 2), w: infoW }
                p.strip = { x: faceW + gap + infoW + gap, y: Math.round((h - 70) / 2), w: stripW, cell: stripCell }
            } else {
                var fh2 = (p.greet ? 43 : 0) + p.ts + (p.dateOn ? 8 + 34 : 0)
                p.face = { x: 0, y: Math.round((h - fh2) / 2) }
                if (infoW > 0) p.info = { x: faceW + gap, y: Math.round((h - 70) / 2), w: infoW }
                p.strip = { x: faceW + gap + infoW + (infoW > 0 ? gap : 0), y: Math.round((h - 70) / 2), w: stripW, cell: stripCell }
            }
            return p
        }
        // regular: the mock's boxes
        if (analog) {
            p.dia = clampInt(Math.min(h, 166), 96, 166)
            var sx = p.dia + GAP, sw = w - sx
            var cellR = clampInt(Math.min(56, sw / 7), 0, 56)
            var k = Math.max(0.72, Math.min(1.17, cellR / 48))
            var showStrip = cellR >= 30 && h - 70 * k >= 80
            p.face = { x: 0, y: Math.round((h - p.dia) / 2) }
            p.info = { x: sx, y: showStrip ? 0 : Math.round((h - 70) / 2), w: sw }
            if (showStrip) p.strip = { x: sx, y: Math.round(h - 70 * k), w: 7 * cellR, cell: cellR }
        } else {
            p.greet = greeting && h >= 150
            p.dateOn = h >= (p.greet ? 43 : 0) + 52 + 41
            p.ts = fitTime(token, w, h + 10, ampm, (p.greet ? 43 : 0) + (p.dateOn ? 41 : 0))
            p.dateOn = p.dateOn && h + 10 >= (p.greet ? 43 : 0) + p.ts + 41
            p.face = { x: 0, y: 0 }
        }
        return p
    }
    readonly property var geo: plan(width, height, analog, showGreeting, timeToken, twelveHour)

    // ------------------------------------------------------------------ digital
    Item {
        id: digitalFace
        visible: !root.analog
        x: root.geo.face.x; y: root.geo.face.y
        width: root.geo.face.w || root.width; height: root.height
        readonly property int timeTop: root.geo.greet ? 43 : 0
        readonly property bool centred: root.geo.face.center === true
        Text {
            id: greetText
            visible: root.geo.greet
            x: 0; y: 0
            width: parent.width
            horizontalAlignment: digitalFace.centred ? Text.AlignHCenter : Text.AlignLeft
            color: root.mat.ink2
            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
            renderType: Text.NativeRendering
        }
        Text {
            id: timeText
            x: digitalFace.centred ? Math.round((parent.width - (implicitWidth + (root.twelveHour ? ampmText.implicitWidth + 10 : 0))) / 2) : 0
            y: digitalFace.timeTop - Math.round(root.geo.ts * 0.183)
            color: root.mat.ink
            font.family: "Manrope"; font.weight: Font.DemiBold
            font.pixelSize: root.geo.ts
            font.letterSpacing: -root.geo.ts * 0.03
            renderType: Text.NativeRendering
        }
        Text {
            id: ampmText
            visible: root.twelveHour
            x: timeText.x + Math.round(timeText.implicitWidth) + 10
            anchors.baseline: timeText.baseline
            color: root.mat.ink2
            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: Math.round(root.geo.ts * 0.35)
            renderType: Text.NativeRendering
        }
        Text {
            id: dateText
            visible: root.geo.dateOn
            x: 0
            y: digitalFace.timeTop + root.geo.ts + 8
            width: parent.width
            horizontalAlignment: digitalFace.centred ? Text.AlignHCenter : Text.AlignLeft
            color: root.mat.ink2
            font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: root.geo.ts >= 80 ? 26 : 24
            renderType: Text.NativeRendering
        }
    }

    // ------------------------------------------------------------------ analog
    Item {
        id: analogFace
        visible: root.analog
        x: root.geo.face.x; y: root.geo.face.y
        width: root.geo.dia; height: root.geo.dia
        readonly property real k: root.geo.dia / 166
        readonly property real cx: width / 2
        readonly property real cy: height / 2

        Canvas {
            // static face: glass disc, four bold markers, eight dots; repainted only on material change or a new size
            id: face
            width: analogFace.width; height: analogFace.height
            onWidthChanged: requestPaint()
            onPaint: {
                var ctx = getContext("2d"), r = width / 2, k = analogFace.k
                ctx.reset()
                ctx.beginPath(); ctx.arc(r, r, r - 1, 0, Math.PI * 2)
                ctx.fillStyle = root.isLight ? "rgba(255,255,255,0.5)" : "rgba(255,255,255,0.07)"; ctx.fill()
                ctx.lineWidth = 1; ctx.strokeStyle = root.isLight ? "rgba(255,255,255,0.95)" : "rgba(255,255,255,0.16)"; ctx.stroke()
                ctx.lineCap = "round"
                for (var i = 0; i < 12; i++) {
                    var a = i * Math.PI / 6, major = i % 3 === 0
                    if (major) {
                        ctx.beginPath()
                        ctx.moveTo(r + Math.cos(a) * (r - 12 * k), r + Math.sin(a) * (r - 12 * k))
                        ctx.lineTo(r + Math.cos(a) * (r - 24 * k), r + Math.sin(a) * (r - 24 * k))
                        ctx.lineWidth = 4 * k; ctx.strokeStyle = root.inkC; ctx.stroke()
                    } else {
                        ctx.beginPath(); ctx.arc(r + Math.cos(a) * (r - 18 * k), r + Math.sin(a) * (r - 18 * k), 2.5 * k, 0, Math.PI * 2)
                        ctx.fillStyle = root.ink3C; ctx.fill()
                    }
                }
            }
            Connections { target: root; onMatChanged: face.requestPaint(); onIsLightChanged: face.requestPaint() }
        }
        Rectangle {
            // hour hand
            x: analogFace.cx - 4 * analogFace.k; y: analogFace.cy - 44 * analogFace.k
            width: 8 * analogFace.k; height: 56 * analogFace.k; radius: width / 2
            color: root.inkC
            antialiasing: true
            transform: Rotation { origin.x: 4 * analogFace.k; origin.y: 44 * analogFace.k; angle: root.hourAngle }
        }
        Rectangle {
            // minute hand
            x: analogFace.cx - 3 * analogFace.k; y: analogFace.cy - 62 * analogFace.k
            width: 6 * analogFace.k; height: 74 * analogFace.k; radius: width / 2
            color: root.inkC
            antialiasing: true
            transform: Rotation { origin.x: 3 * analogFace.k; origin.y: 62 * analogFace.k; angle: root.minuteAngle }
        }
        Rectangle {
            id: secondHand
            // second hand: steps once a second on Home's shared tick, transform only
            x: analogFace.cx - width / 2; y: analogFace.cy - 66 * analogFace.k
            width: Math.max(2, 2 * analogFace.k); height: 84 * analogFace.k; radius: width / 2
            color: root.secondC
            antialiasing: true
            transform: Rotation {
                origin.x: secondHand.width / 2; origin.y: 66 * analogFace.k
                angle: root.secondAngle
            }
        }
        Rectangle {
            x: analogFace.cx - 6 * analogFace.k; y: analogFace.cy - 6 * analogFace.k
            width: 12 * analogFace.k; height: width; radius: width / 2
            color: root.secondC
            antialiasing: true
            Rectangle { x: parent.width / 3; y: parent.width / 3; width: parent.width / 3; height: width; radius: width / 2; color: root.onInk }
        }
    }

    // the weekday and the date beside (or under) the dial
    Item {
        id: info
        visible: root.geo.info !== null
        x: root.geo.info ? root.geo.info.x : 0
        y: root.geo.info ? root.geo.info.y : 0
        width: root.geo.info ? root.geo.info.w : 0
        height: 70
        readonly property bool centred: !!(root.geo.info && root.geo.info.center)
        readonly property int wdSize: root.geo.mode === "tiny" ? 24 : 32
        Text {
            x: 0; y: info.wdSize === 24 ? 0 : -2
            width: parent.width
            horizontalAlignment: info.centred ? Text.AlignHCenter : Text.AlignLeft
            text: root.weekday
            color: root.inkC
            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: info.wdSize
            font.letterSpacing: -0.3
            renderType: Text.NativeRendering
        }
        Text {
            x: 0; y: info.wdSize === 24 ? 34 : 42
            width: parent.width
            horizontalAlignment: info.centred ? Text.AlignHCenter : Text.AlignLeft
            text: root.dateLine
            color: root.ink2C
            font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: info.wdSize === 24 ? 20 : 22
            renderType: Text.NativeRendering
        }
    }

    // the current week: initials over the days, today in a filled disc
    Item {
        id: strip
        visible: root.geo.strip !== null
        x: root.geo.strip ? root.geo.strip.x : 0
        y: root.geo.strip ? root.geo.strip.y : 0
        width: root.geo.strip ? root.geo.strip.w : 0
        height: 70 * k
        readonly property int cell: root.geo.strip ? root.geo.strip.cell : 48
        readonly property real k: Math.max(0.72, Math.min(1.17, cell / 48))
        Repeater {
            model: root.geo.strip ? root.week : []
            delegate: Item {
                x: index * strip.cell
                y: 0
                width: strip.cell; height: 70 * strip.k
                Text {
                    width: parent.width
                    y: 0
                    horizontalAlignment: Text.AlignHCenter
                    text: modelData.initial
                    color: modelData.today ? root.inkC : root.ink2C
                    font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: Math.round(14 * strip.k)
                    renderType: Text.NativeRendering
                }
                Rectangle {
                    visible: modelData.today
                    x: Math.round((parent.width - width) / 2); y: Math.round(26 * strip.k)
                    width: Math.round(36 * strip.k); height: width; radius: width / 2
                    color: root.inkC
                    antialiasing: true
                }
                Text {
                    width: parent.width
                    y: Math.round(26 * strip.k) + Math.round((Math.round(36 * strip.k) - implicitHeight) / 2)
                    horizontalAlignment: Text.AlignHCenter
                    text: modelData.n
                    color: modelData.today ? root.onInk : root.ink2C
                    font.family: "Manrope"; font.weight: modelData.today ? Font.Bold : Font.Medium; font.pixelSize: Math.round(20 * strip.k)
                    renderType: Text.NativeRendering
                }
            }
        }
    }

    // the month, and the next ones when the card is wide enough
    Item {
        id: monthsBlock
        visible: root.geo.months !== null
        x: root.geo.months ? root.geo.months.x : 0
        y: 0
        width: root.geo.months ? root.geo.months.w : 0
        height: root.height
        readonly property int cell: root.geo.months ? root.geo.months.cell : 40
        readonly property int fs: Math.round(Math.max(15, Math.min(26, cell * 0.42)))
        Repeater {
            model: root.geo.months ? root.months.slice(0, root.geo.months.count) : []
            delegate: Item {
                id: month
                readonly property var m: modelData
                x: index * (7 * monthsBlock.cell + 48)
                y: 0
                width: 7 * monthsBlock.cell; height: root.height
                readonly property int head: 44
                Text {
                    x: 0; y: 0
                    width: parent.width
                    text: month.m.title
                    color: root.inkC
                    font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 26
                    renderType: Text.NativeRendering
                }
                Repeater {
                    model: ["S", "M", "T", "W", "T", "F", "S"]
                    delegate: Text {
                        x: index * monthsBlock.cell; y: month.head
                        width: monthsBlock.cell
                        horizontalAlignment: Text.AlignHCenter
                        text: modelData
                        color: root.ink2C
                        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: Math.max(14, monthsBlock.fs - 4)
                        renderType: Text.NativeRendering
                    }
                }
                Repeater {
                    model: month.m.cells
                    delegate: Item {
                        visible: modelData.n > 0
                        x: (index % 7) * monthsBlock.cell
                        y: month.head + Math.round(monthsBlock.cell * 0.8) + Math.floor(index / 7) * monthsBlock.cell
                        width: monthsBlock.cell; height: monthsBlock.cell
                        Rectangle {
                            visible: modelData.today
                            anchors.centerIn: parent
                            width: Math.min(parent.width, parent.height) - 6; height: width; radius: width / 2
                            color: root.inkC
                            antialiasing: true
                        }
                        Text {
                            anchors.centerIn: parent
                            text: modelData.n
                            color: modelData.today ? root.onInk : root.ink2C
                            font.family: "Manrope"; font.weight: modelData.today ? Font.Bold : Font.Medium; font.pixelSize: monthsBlock.fs
                            renderType: Text.NativeRendering
                        }
                    }
                }
            }
        }
    }

    property real hourAngle: 0
    property real minuteAngle: 0
    property real secondAngle: 0
    property int calMonthKey: -1
    property int frameTick: 0
    property int lastSecond: -1
    property string weekday: ""
    property string dateLine: ""
    property var week: []          // [{ n, initial, today }]
    property var months: []        // [{ title, cells: [{ n, today }], rows }] this month and the two after it

    function pad(n) { return (n < 10 ? "0" : "") + n }
    function greetingFor(h) {
        return h < 5 ? "Good night" : h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : h < 22 ? "Good evening" : "Good night"
    }
    function monthOf(d, offset) {
        var first = new Date(d.getFullYear(), d.getMonth() + offset, 1)
        var days = new Date(first.getFullYear(), first.getMonth() + 1, 0).getDate()
        var cells = [], i
        for (i = 0; i < first.getDay(); i++) cells.push({ n: 0, today: false })
        for (i = 1; i <= days; i++) cells.push({ n: i, today: offset === 0 && i === d.getDate() })
        while (cells.length % 7) cells.push({ n: 0, today: false })
        return { title: Qt.formatDate(first, "MMMM yyyy"), cells: cells, rows: cells.length / 7 }
    }
    function buildCalendar(d) {
        var start = new Date(d.getFullYear(), d.getMonth(), d.getDate() - d.getDay())
        var out = [], initials = ["S", "M", "T", "W", "T", "F", "S"]
        for (var i = 0; i < 7; i++) {
            var day = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i)
            out.push({ n: day.getDate(), initial: initials[i], today: i === d.getDay() })
        }
        week = out
        weekday = Qt.formatDate(d, "dddd")
        dateLine = Qt.formatDate(d, "MMMM d, yyyy")
        months = [monthOf(d, 0), monthOf(d, 1), monthOf(d, 2)]
        calMonthKey = d.getFullYear() * 12 + d.getMonth() + d.getDate() * 1000
    }
    function tick() {
        var d = new Date()
        var h = d.getHours(), m = d.getMinutes(), s = d.getSeconds()
        if (s === root.lastSecond) return
        root.lastSecond = s
        var key = d.getFullYear() * 12 + d.getMonth() + d.getDate() * 1000
        if (key !== root.calMonthKey) buildCalendar(d)
        if (root.analog) {
            root.hourAngle = ((h % 12) + m / 60) * 30
            root.minuteAngle = (m + s / 60) * 6
            // keep the second hand monotonic so the tick never spins backwards at :00
            var target = s * 6
            var base = Math.floor(root.secondAngle / 360) * 360
            var next = base + target
            if (next < root.secondAngle - 180) next += 360
            root.secondAngle = next
        } else {
            if (root.twelveHour) {
                var hh = h % 12; if (hh === 0) hh = 12
                timeText.text = hh + ":" + pad(m)
                ampmText.text = h < 12 ? "AM" : "PM"
            } else {
                timeText.text = pad(h) + ":" + pad(m)
            }
            greetText.text = greetingFor(h)
            dateText.text = Qt.formatDate(d, "dddd, MMMM d")
        }
    }
    onAnalogChanged: { lastSecond = -1; tick() }
    onTwelveHourChanged: { lastSecond = -1; tick() }
    // Driven by Home's shared tick, which stops while Home is hidden; showing Home again
    // ticks at once, so the hands never show a stale time.
    onFrameTickChanged: tick()
    onLiveChanged: if (live) { lastSecond = -1; tick() }
    Component.onCompleted: tick()
}
