.pragma library

// Screenshots and screen recording as the Settings page (PageCapture.qml) and the quick panel's two tiles (panels/QuickMore.qml) drive them
// (docs/research/capture-result.md). The answers of the helper's /v1/capture routes, reduced to what a surface shows, and the words for
// what happened. Nothing here calls anything.

// What GET /v1/capture said (`doc`), as the surface keeps it. `answered` is whether the helper answered at all; `enabled` is the switch
// (capture.enabled in the helper's config, off until it is turned on); `available` is whether the compositor answered; `recording` and
// `seconds` are the recording as it runs.
function state(doc) {
    var c = doc && doc.capture ? doc.capture : null
    if (!c || c.enabled !== true) return { answered: c !== null, enabled: false, available: false, recording: false, seconds: 0, count: 0, bytes: 0, max: 0, encoder: false, last: null, reason: "" }
    var rec = c.recorder || null
    var files = c.captures || {}
    return {
        answered: true,
        enabled: true,
        available: c.available === true,
        recording: c.recording === true,
        seconds: rec ? Math.floor(rec.seconds || 0) : 0,
        count: files.count || 0,
        bytes: files.bytes || 0,
        max: files.max || 0,
        encoder: !!(c.encoder && c.encoder.available),
        last: c.last || null,
        reason: c.reason || ""
    }
}

// The words for an answer to a command: `kind` is "shot", "start" or "stop"; `status` the HTTP status (0 when the helper did not answer);
// `st` the state read right after it (state()).
function said(kind, status, doc, st) {
    if (status >= 200 && status < 300 && doc && doc.ok === true) {
        if (kind === "shot") return "Screenshot saved"
        if (kind === "start") return "Recording"
        return "Recording saved"
    }
    var code = doc && doc.error ? String(doc.error.code || "") : why(kind, status, st)
    switch (code) {
    case "helper": return "The TV's helper did not answer"
    case "capture_disabled": return "Capture is off"
    case "no_indicator": return kind === "shot" ? "The shell is not on screen" : "The recording dot could not be shown"
    case "busy": return "Already recording"
    case "not_recording": return "Not recording"
    case "no_encoder": return "Recording is not set up on this TV"
    case "disk_full": return "Not enough space on the TV"
    case "compositor_unavailable": return "Needs LumaTV's own compositor"
    case "timeout": return "That took too long"
    }
    return kind === "shot" ? "The screenshot failed" : "The recording failed"
}

// The reason, when the body did not come: kit/Helper.qml reads one only on 200, 202 and 409, and the workstation host's network stack
// hands a 409 over as status 0 with no body (the set's Qt is not measured). Every other reason has a status of its own (the helper's
// capture.rs); the 409 group (no_indicator, busy, not_recording) is told from the state read after the command. "" is none of them.
function why(kind, status, st) {
    switch (status) {
    case 403: return "capture_disabled"
    case 501: return "no_encoder"
    case 503: return "compositor_unavailable"
    case 504: return "timeout"
    case 507: return "disk_full"
    case 0:
    case 409:
        if (!st || st.answered !== true) return "helper"
        if (!st.enabled) return "capture_disabled"
        if (kind === "stop") return "not_recording"
        return kind === "start" && st.recording ? "busy" : "no_indicator"
    }
    return ""
}

// "812 KB", "3.4 MB", "1.2 GB": what the files take.
function size(bytes) {
    var b = Number(bytes) || 0
    if (b < 1000 * 1000) return Math.max(0, Math.round(b / 1000)) + " KB"
    if (b < 1000 * 1000 * 1000) return (Math.round(b / 100000) / 10) + " MB"
    return (Math.round(b / 100000000) / 10) + " GB"
}

// "0:07", "12:30": a recording's length so far.
function clock(seconds) {
    var s = Math.max(0, Math.floor(Number(seconds) || 0))
    var m = Math.floor(s / 60)
    return m + ":" + (s % 60 < 10 ? "0" : "") + (s % 60)
}

// What the folder holds, as one value: "3 files, 5.2 MB".
function files(st) {
    if (!st || !st.enabled) return ""
    return st.count + (st.count === 1 ? " file, " : " files, ") + size(st.bytes)
}
