import QtQuick 2.4
import TextFile 1.0
import "../Saved.js" as Saved

// Current conditions and forecast rows on the mock's boxes: h3 at 0, "now" row at 47 (icon
// 96, temperature 84px on a line-height-1 box, condition under it), stats at 179, a rule at
// 220 and 41px rows from 225 with columns 70 | icon | 56 | 56. Data: zippopotam for lat/lon,
// Open-Meteo for conditions. The last reading is saved to {{dataDir}}/cache/weather.json after
// every fetch and drawn from there at once on a cold start (Saved.js).
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
    property double fetchedAt: 0        // when the reading on screen was fetched, also when it came from disk
    property int failures: 0
    property double lastAttempt: 0
    property bool fetching: false
    property int fetchGen: 0
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

    // The strings of a reading, as Saved.weatherDoc computes them for the document on disk.
    function show(doc) {
        var d = doc.display
        place = d.place; temp = d.temp; cond = d.cond; stats = d.stats; nowKind = d.nowKind; days = d.days
        fetchedAt = doc.fetchedAt
    }

    // ---- saved first. A newer reading on disk than the one on screen (the native helper's)
    // replaces it. Refresh is due 15 min after the reading, retries after 2, 10 and then every
    // 30 minutes (Saved.js). A place the zip lookup cannot resolve is an error, never another
    // place's weather: the saved reading stays up and the lookup is tried again.
    TextFile { id: savedFile; name: "file://{{dataDir}}/cache/weather.json" }
    function loadSaved() {
        var doc = Saved.parse(savedFile.exists ? savedFile.read() : "")
        if (!Saved.weatherUsable(doc, zip, units, Date.now()) || doc.fetchedAt <= fetchedAt) return
        show(doc)
    }
    function fetchWeather() {
        if (fetching) return
        fetching = true
        lastAttempt = Date.now()
        var gen = ++fetchGen, askedZip = zip, askedUnits = units
        watchdog.restart()
        var z = new XMLHttpRequest()
        z.onreadystatechange = function() {
            if (z.readyState !== 4 || gen !== root.fetchGen) return
            var geo = null
            try {
                var p = JSON.parse(z.responseText).places[0]
                geo = { place: p["place name"], lat: parseFloat(p.latitude), lon: parseFloat(p.longitude) }
                if (!geo.place || isNaN(geo.lat) || isNaN(geo.lon)) geo = null
            } catch (e) { geo = null }
            if (!geo) { root.fail("place lookup"); return }
            var u = "https://api.open-meteo.com/v1/forecast?latitude=" + geo.lat + "&longitude=" + geo.lon +
                    "&current=temperature_2m,apparent_temperature,weather_code,is_day,relative_humidity_2m,wind_speed_10m" +
                    "&daily=weather_code,temperature_2m_max,temperature_2m_min&temperature_unit=" + askedUnits + "&wind_speed_unit=mph&timezone=auto&forecast_days=7"
            var x = new XMLHttpRequest()
            x.onreadystatechange = function() {
                if (x.readyState !== 4 || gen !== root.fetchGen) return
                var doc = null
                try { doc = Saved.weatherDoc(askedZip, askedUnits, geo, JSON.parse(x.responseText), Date.now()) }
                catch (e) { doc = null }
                if (!doc) { root.fail("forecast"); return }
                root.finish(doc)
            }
            x.open("GET", u); x.send()
        }
        z.open("GET", "https://api.zippopotam.us/us/" + askedZip); z.send()
    }
    function finish(doc) {
        watchdog.stop(); fetching = false
        if (doc.zip !== zip || doc.units !== units) { refresh(); return }   // reconfigured meanwhile
        show(doc)
        failures = 0
        savedFile.write(JSON.stringify(doc))
        schedule()
    }
    function fail(what) {
        watchdog.stop(); fetching = false
        failures++
        console.warn("[WeatherWidget] " + what + " failed; retry in " + Math.round(Saved.retryDelay(failures) / 60000) + " min")
        schedule()
    }
    // A request that never answers must not hold the card back from its next try.
    Timer { id: watchdog; interval: 30000; onTriggered: { if (root.fetching) { root.fetchGen++; root.fail("request") } } }
    Timer { id: due; repeat: false; onTriggered: root.refresh() }
    function schedule() {
        due.stop()
        if (!live) return
        due.interval = Math.max(1000, Math.min(Saved.nextAttempt(fetchedAt, failures, lastAttempt) - Date.now(), 3600000))
        due.restart()
    }
    function refresh() {
        if (!live || fetching) return
        if (Saved.nextAttempt(fetchedAt, failures, lastAttempt) > Date.now() + 1000) { schedule(); return }
        fetchWeather()
    }
    // Fetches only while Home is on screen; showing it again looks at the disk (the helper may
    // have fetched) and fetches if the reading is due, so a TV left on an app does not poll
    // unseen for hours.
    onLiveChanged: { if (live) { loadSaved(); refresh() } else { due.stop() } }
    onZipChanged: reconfigured()
    onUnitsChanged: reconfigured()
    function reconfigured() { fetchedAt = 0; failures = 0; loadSaved(); refresh() }
    Component.onCompleted: loadSaved()

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
