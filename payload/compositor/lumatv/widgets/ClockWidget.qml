import QtQuick 2.4

// Clock card in two styles (theme.clock.style or the layout entry's settings.style):
//  "digital": greeting, time, date on the mock's CSS boxes (h3 at 0, a 92px line-height-1
//             box at y=43, date at y=143; Manrope's 1.366em ascent+descent puts the glyph
//             box 0.183em above a line-height-1 box, applied straight to the Text y).
//  "analog":  a face drawn once into a Canvas, three hands that are pure rotation
//             transforms (the second hand ticks with a short overshoot), and the current
//             weekday, date and week strip to the right.
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
    property int timeSize: (theme.type && theme.type.clock) || 92
    property color inkC: mat.ink || "#F2EFF6"
    property color ink2C: mat.ink2 || "#A8F2EFF6"
    property color ink3C: mat.ink3 || "#6BF2EFF6"
    property color secondC: (theme.clock && theme.clock.secondHand) || "#ff3b5c"

    // ------------------------------------------------------------------ digital
    Item {
        anchors.fill: parent
        visible: !root.analog
        Text {
            id: greetText
            visible: root.showGreeting
            x: 0; y: 0
            color: root.mat.ink2
            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 24
            renderType: Text.NativeRendering
        }
        Text {
            id: timeText
            x: 0
            y: (root.showGreeting ? 43 : 0) - Math.round(root.timeSize * 0.183)
            color: root.mat.ink
            font.family: "Manrope"; font.weight: Font.DemiBold
            font.pixelSize: root.timeSize
            font.letterSpacing: -root.timeSize * 0.03
            renderType: Text.NativeRendering
        }
        Text {
            id: ampmText
            visible: root.twelveHour
            x: timeText.x + Math.round(timeText.implicitWidth) + 10
            anchors.baseline: timeText.baseline
            color: root.mat.ink2
            font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 32
            renderType: Text.NativeRendering
        }
        Text {
            id: dateText
            x: 0
            y: (root.showGreeting ? 43 : 0) + root.timeSize + 8
            color: root.mat.ink2
            font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 26
            renderType: Text.NativeRendering
        }
    }

    // ------------------------------------------------------------------ analog
    Item {
        id: analogView
        anchors.fill: parent
        visible: root.analog
        property int dia: Math.min(root.height, 166)
        property real cx: dia / 2
        property real cy: dia / 2

        Canvas {
            // static face: glass disc, four bold markers, eight dots; repainted only on material change
            id: face
            width: analogView.dia; height: analogView.dia
            onPaint: {
                var ctx = getContext("2d"), r = width / 2
                ctx.reset()
                ctx.beginPath(); ctx.arc(r, r, r - 1, 0, Math.PI * 2)
                ctx.fillStyle = root.isLight ? "rgba(255,255,255,0.5)" : "rgba(255,255,255,0.07)"; ctx.fill()
                ctx.lineWidth = 1; ctx.strokeStyle = root.isLight ? "rgba(255,255,255,0.95)" : "rgba(255,255,255,0.16)"; ctx.stroke()
                ctx.lineCap = "round"
                for (var i = 0; i < 12; i++) {
                    var a = i * Math.PI / 6, major = i % 3 === 0
                    if (major) {
                        ctx.beginPath()
                        ctx.moveTo(r + Math.cos(a) * (r - 12), r + Math.sin(a) * (r - 12))
                        ctx.lineTo(r + Math.cos(a) * (r - 24), r + Math.sin(a) * (r - 24))
                        ctx.lineWidth = 4; ctx.strokeStyle = root.inkC; ctx.stroke()
                    } else {
                        ctx.beginPath(); ctx.arc(r + Math.cos(a) * (r - 18), r + Math.sin(a) * (r - 18), 2.5, 0, Math.PI * 2)
                        ctx.fillStyle = root.ink3C; ctx.fill()
                    }
                }
            }
            Connections { target: root; onMatChanged: face.requestPaint(); onIsLightChanged: face.requestPaint() }
        }
        Rectangle {
            // hour hand
            x: analogView.cx - 4; y: analogView.cy - 44
            width: 8; height: 56; radius: 4
            color: root.inkC
            antialiasing: true
            transform: Rotation { origin.x: 4; origin.y: 44; angle: root.hourAngle }
        }
        Rectangle {
            // minute hand
            x: analogView.cx - 3; y: analogView.cy - 62
            width: 6; height: 74; radius: 3
            color: root.inkC
            antialiasing: true
            transform: Rotation { origin.x: 3; origin.y: 62; angle: root.minuteAngle }
        }
        Rectangle {
            // second hand: ticks with a short overshoot, transform only
            x: analogView.cx - 1; y: analogView.cy - 66
            width: 2; height: 84; radius: 1
            color: root.secondC
            antialiasing: true
            transform: Rotation {
                origin.x: 1; origin.y: 66
                angle: root.secondAngle
                Behavior on angle { NumberAnimation { duration: 160; easing.type: Easing.OutBack } }
            }
        }
        Rectangle {
            x: analogView.cx - 6; y: analogView.cy - 6
            width: 12; height: 12; radius: 6
            color: root.secondC
            antialiasing: true
            Rectangle { x: 4; y: 4; width: 4; height: 4; radius: 2; color: root.isLight ? "#ffffff" : "#120e1a" }
        }

        // right block: weekday, date, current week strip with today in a filled disc
        Item {
            id: cal
            x: analogView.dia + 36
            y: 0
            width: root.width - x
            height: root.height
            property string weekday: ""
            property string dateLine: ""
            property var week: []          // [{ n, initial, today }]
            property int cell: Math.floor(Math.min(48, width / 7))
            Text {
                x: 0; y: -2
                text: cal.weekday
                color: root.inkC
                font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 32
                font.letterSpacing: -0.3
                renderType: Text.NativeRendering
            }
            Text {
                x: 0; y: 42
                text: cal.dateLine
                color: root.ink2C
                font.family: "Manrope"; font.weight: Font.Medium; font.pixelSize: 22
                renderType: Text.NativeRendering
            }
            Repeater {
                model: cal.week
                delegate: Item {
                    x: index * cal.cell
                    y: cal.height - 70
                    width: cal.cell; height: 70
                    Text {
                        width: parent.width
                        y: 0
                        horizontalAlignment: Text.AlignHCenter
                        text: modelData.initial
                        color: modelData.today ? root.inkC : root.ink3C
                        font.family: "Manrope"; font.weight: Font.DemiBold; font.pixelSize: 14
                        renderType: Text.NativeRendering
                    }
                    Rectangle {
                        visible: modelData.today
                        x: Math.round((parent.width - 36) / 2); y: 26
                        width: 36; height: 36; radius: 18
                        color: root.inkC
                        antialiasing: true
                    }
                    Text {
                        width: parent.width
                        y: 26 + Math.round((36 - implicitHeight) / 2)
                        horizontalAlignment: Text.AlignHCenter
                        text: modelData.n
                        color: modelData.today ? (root.isLight ? "#ffffff" : "#120e1a") : root.ink2C
                        font.family: "Manrope"; font.weight: modelData.today ? Font.Bold : Font.Medium; font.pixelSize: 20
                        renderType: Text.NativeRendering
                    }
                }
            }
        }
    }

    property real hourAngle: 0
    property real minuteAngle: 0
    property real secondAngle: 0
    property int calMonthKey: -1

    function pad(n) { return (n < 10 ? "0" : "") + n }
    function greetingFor(h) {
        return h < 5 ? "Good night" : h < 12 ? "Good morning" : h < 17 ? "Good afternoon" : h < 22 ? "Good evening" : "Good night"
    }
    function buildCalendar(d) {
        var start = new Date(d.getFullYear(), d.getMonth(), d.getDate() - d.getDay())
        var out = [], initials = ["S", "M", "T", "W", "T", "F", "S"]
        for (var i = 0; i < 7; i++) {
            var day = new Date(start.getFullYear(), start.getMonth(), start.getDate() + i)
            out.push({ n: day.getDate(), initial: initials[i], today: i === d.getDay() })
        }
        cal.week = out
        cal.weekday = Qt.formatDate(d, "dddd")
        cal.dateLine = Qt.formatDate(d, "MMMM d, yyyy")
        calMonthKey = d.getFullYear() * 12 + d.getMonth() + d.getDate() * 1000
    }
    function tick() {
        var d = new Date()
        var h = d.getHours(), m = d.getMinutes(), s = d.getSeconds()
        if (root.analog) {
            root.hourAngle = ((h % 12) + m / 60) * 30
            root.minuteAngle = (m + s / 60) * 6
            // keep the second hand monotonic so the tick never spins backwards at :00
            var target = s * 6
            var base = Math.floor(root.secondAngle / 360) * 360
            var next = base + target
            if (next < root.secondAngle - 180) next += 360
            root.secondAngle = next
            var key = d.getFullYear() * 12 + d.getMonth() + d.getDate() * 1000
            if (key !== root.calMonthKey) buildCalendar(d)
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
    onAnalogChanged: { calMonthKey = -1; tick() }
    // Stopped while Home is hidden; starting again ticks at once, so the hands never show a stale time.
    Timer { interval: 1000; running: root.live; repeat: true; triggeredOnStart: true; onTriggered: root.tick() }
}
