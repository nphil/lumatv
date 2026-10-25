import QtQuick 2.4

// Current conditions and forecast rows on the mock's boxes: h3 at 0, "now" row at 47 (icon
// 96, temperature 84px on a line-height-1 box, condition under it), stats at 179, a rule at
// 220 and 41px rows from 225 with columns 70 | icon | 56 | 56. Data: zippopotam for lat/lon,
// Open-Meteo for conditions, unchanged from the previous widget layer.
Item {
    id: root
    property var theme: ({})
    property var entry: ({})
    property var mat: ({ ink: "#F2EFF6", ink2: "#A8F2EFF6" })
    property bool isLight: false
    property var settings: ({})
    property bool focused: false
    property bool inner: false
    property bool live: true
    property double fetchedAt: 0
    readonly property string iconDir: "file://{{dataDir}}/qml/WebOSCompositor/{{qmlModule}}/icons/"

    function activate() {}

    property string zip: (theme.weather && theme.weather.zip) || "30311"
    property string units: (theme.weather && theme.weather.units) || "fahrenheit"
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

    function kindFor(code, isDay) {
        if (code === 0) return isDay ? "sun" : "moon"
        if (code <= 2) return isDay ? "partly" : "partly-night"
        if (code === 3) return "cloud"
        if (code === 45 || code === 48) return "fog"
        if (code >= 95) return "storm"
        if ((code >= 71 && code <= 77) || code === 85 || code === 86) return "snow"
        if (code >= 61 || code >= 80) return "rain"
        if (code >= 51) return "drizzle"
        return "cloud"
    }
    function textFor(code) {
        if (code === 0) return "Clear"; if (code === 1) return "Mostly clear"; if (code === 2) return "Partly cloudy"; if (code === 3) return "Overcast"
        if (code === 45 || code === 48) return "Fog"; if (code >= 95) return "Thunderstorm"
        if ((code >= 71 && code <= 77) || code === 85 || code === 86) return "Snow"
        if (code >= 80) return "Showers"; if (code >= 61) return "Rain"; if (code >= 51) return "Drizzle"; return "Cloudy"
    }
    function dayName(iso) { var d = new Date(iso + "T12:00:00"); return ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][d.getDay()] }

    function fetchWeather() {
        var z = new XMLHttpRequest()
        z.onreadystatechange = function() {
            if (z.readyState !== 4) return
            var lat = 33.72, lon = -84.45, name = "Atlanta"
            try { var j = JSON.parse(z.responseText); lat = parseFloat(j.places[0].latitude); lon = parseFloat(j.places[0].longitude); name = j.places[0]["place name"] } catch (e) {}
            root.place = name
            var u = "https://api.open-meteo.com/v1/forecast?latitude=" + lat + "&longitude=" + lon +
                    "&current=temperature_2m,apparent_temperature,weather_code,is_day,relative_humidity_2m,wind_speed_10m" +
                    "&daily=weather_code,temperature_2m_max,temperature_2m_min&temperature_unit=" + root.units + "&wind_speed_unit=mph&timezone=auto&forecast_days=7"
            var x = new XMLHttpRequest()
            x.onreadystatechange = function() {
                if (x.readyState !== 4) return
                try {
                    var w = JSON.parse(x.responseText)
                    root.temp = Math.round(w.current.temperature_2m) + "\u00b0"
                    root.cond = textFor(w.current.weather_code)
                    root.stats = "Feels like " + Math.round(w.current.apparent_temperature) + "\u00b0 \u00b7 " + w.current.relative_humidity_2m + "% humidity \u00b7 " + Math.round(w.current.wind_speed_10m) + " mph"
                    root.nowKind = kindFor(w.current.weather_code, w.current.is_day === 1)
                    var rows = []
                    for (var i = 0; i < w.daily.time.length; i++)
                        rows.push({ day: i === 0 ? "Today" : dayName(w.daily.time[i]), kind: kindFor(w.daily.weather_code[i], true),
                                    hi: Math.round(w.daily.temperature_2m_max[i]) + "\u00b0", lo: Math.round(w.daily.temperature_2m_min[i]) + "\u00b0" })
                    root.days = rows
                } catch (e) { console.warn("[WeatherWidget] parse failed: " + e) }
            }
            x.open("GET", u); x.send()
        }
        z.open("GET", "https://api.zippopotam.us/us/" + zip); z.send()
    }
    // Fetches only while Home is on screen. Showing it again fetches at once if the reading is
    // older than the interval, so a TV left on an app does not poll unseen for hours.
    Timer { interval: 900000; running: root.live; repeat: true; triggeredOnStart: true
            onTriggered: if (Date.now() - root.fetchedAt >= interval - 5000) { root.fetchedAt = Date.now(); root.fetchWeather() } }

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
