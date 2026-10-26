.pragma library

// The pages that hang under a rail section of Settings (docs/research/settings-coverage.md): each is reached from a row of its
// parent, never from the rail, and Back returns to the page it was opened from. `parent` is the rail section whose rail row stays
// lit while the page is up (Settings.goTo uses it), `search: false` keeps a page that only means something when a row opened it
// (the chooser) out of Home's search. Settings.qml appends this list to its own.
var pages = [
    { id: "choice", title: "Choose", icon: "list-checks", page: "settings/PageChoice.qml", rail: false, search: false },
    { id: "captions", title: "Closed caption", icon: "type", page: "settings/PageCaptions.qml", rail: false, parent: "system" },
    { id: "audioguide", title: "Audio Guidance", icon: "audio-lines", page: "settings/PageAudioGuide.qml", rail: false, parent: "system" },
    { id: "audiooptions", title: "Audio options", icon: "headphones", page: "settings/PageAudioOptions.qml", rail: false, parent: "system" },
    { id: "clock", title: "Time & Date", icon: "clock", page: "settings/PageClock.qml", rail: false, parent: "system" },
    { id: "poweron", title: "Power On Timer", icon: "power", page: "settings/PagePowerOn.qml", rail: false, parent: "system" },
    { id: "btaudio", title: "Bluetooth Speakers", icon: "headphones", page: "settings/PageBtAudio.qml", rail: false, parent: "sound" },
    { id: "bthid", title: "Bluetooth Devices", icon: "gamepad-2", page: "settings/PageBtHid.qml", rail: false, parent: "inputs" },
    { id: "btdevice", title: "Bluetooth device", icon: "headphones", page: "settings/PageBtDevice.qml", rail: false, search: false },
    { id: "wifi", title: "Join Network", icon: "wifi", page: "settings/PageWifi.qml", rail: false, parent: "network" },
    { id: "ipsettings", title: "IP Settings", icon: "router", page: "settings/PageIp.qml", rail: false, parent: "network" },
    { id: "poweroff", title: "Power Off Timer", icon: "power", page: "settings/PagePowerOff.qml", rail: false, parent: "system" },
    { id: "hdmiaudio", title: "HDMI input audio", icon: "cable", page: "settings/PageHdmiAudio.qml", rail: false, parent: "sound" },
    { id: "equalizer", title: "Equalizer", icon: "sliders-vertical", page: "settings/PageEqualizer.qml", rail: false, parent: "sound" },
    { id: "language", title: "Language & location", icon: "languages", page: "settings/PageLanguage.qml", rail: false, parent: "system" },
    { id: "support", title: "Support", icon: "circle-help", page: "settings/PageSupport.qml", rail: false, parent: "system" },
    { id: "safety", title: "Safety", icon: "shield", page: "settings/PageSafety.qml", rail: false, parent: "system" },
    { id: "channels", title: "Channels", icon: "antenna", page: "settings/PageChannels.qml", rail: false, parent: "inputs" }
];
