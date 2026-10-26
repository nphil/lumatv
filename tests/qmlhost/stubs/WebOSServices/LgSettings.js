// Which settingsservice categories and keys exist, and what the ones LumaTV's Settings writes accept, taken from
// LG's own Settings app for this C4 (research/lgsettings/main.js: the `category`/`keys` lists its getSystemSettings
// calls send, and the value tables behind its row labels). The stub service refuses a write to a key outside `keys`
// or to a value outside `values` or `ranges`, as the real one does, so a page that writes a made-up key fails its test.
.pragma library
.import "LgDesc.js" as Desc

var keys = {
    "aiPicture": ["ai_Brightness", "ai_Genre", "ai_Picture", "isAiPictureActing"],
    "aspectRatio": ["allDirZoomHPosition", "allDirZoomHRatio", "allDirZoomVPosition", "allDirZoomVRatio", "arcMenuBySystem", "arcPerApp", "justScan", "vertZoomVPosition", "vertZoomVRatio"],
    "caption": ["aiCaptionEnable", "caption608PosOption", "captionAnalog", "captionBgColor", "captionBgOpacity", "captionDigital", "captionDigitalPosOption", "captionEdgeColor", "captionEdgeType", "captionEnable", "captionFont", "captionSize", "captionStyle", "captionTextColor", "captionTextOpacity", "captionWindowColor", "captionWindowOpacity", "digitalPlusCaptionLanguage", "hardOfHearing", "japanCaptionEnable", "japanSuperImposeEnable", "naisDevicePaired", "phlCaptionEnable", "phlSuperImposeEnable"],
    "channel": ["analogSystemColor", "autoscanChannelGroup", "dualMono", "multiFeed", "signingAtsc30", "supportOperatorProfile"],
    "dimensionInfo": ["_3dStatus", "colorSystem", "dynamicRange", "gameInput", "input"],
    "general": ["SCA3SystemCountry", "adCookie", "aiChannel", "aiNudge", "aiSettingsNudge", "alwaysOn", "alwaysOnDisableEndHour", "alwaysOnDisableEndMinute", "alwaysOnDisableStartHour", "alwaysOnDisableStartMinute", "avmsd", "bendableSound", "blueToothSpeakerMode", "doNotSellMyPersonalInformation", "homeAutoLaunch", "homeEffect", "homePromotion", "lastInputApp", "lifeAlarm", "lifeOnScreenMode", "lmt", "nativeConsentAd", "noSignalScreenSaver", "personalRecommend", "remotePointer", "screenSaverAd", "screenSaverEnabled", "selectedInputApp", "tnativeShadescreen", "tnativeSound", "touchLock", "tvOnScreen", "tvOnSound", "wavBoxPosition", "wavUpperLedFlag", "welcomeFeature"],
    "hotelMode": ["auxSettingsEnabled", "enableHotelMode", "enableIrRemote", "enableLimitedInputSrcChange", "enableLimitedMode", "enablePowerOnDefault", "enablePowerOnDefaultVolume", "enableProgramChange", "enableSetIdMenu", "enableStartVolume", "enableSystemProviderMode", "enterStationMenu", "keyManagement", "maximumVolume", "minimumVolume", "powerOnDefaultInput", "swUpdateEnable", "swUpdateNotice"],
    "lock": ["applockPerApp", "camPinCode", "downloadableRrt", "inputBlock", "movieRating", "parentalControl", "parentalGuidance", "parentalGuidanceJp", "ratingBR", "ratingSA", "systemPin", "tvRatingChildrenAge", "tvRatingChildrenFantasy", "tvRatingEnglish", "tvRatingFrench", "tvRatingGeneralAge", "tvRatingGeneralDialog", "tvRatingGeneralLanguage", "tvRatingGeneralSex", "tvRatingGeneralViolence", "vooRating", "ziggoRaiting"],
    "monitor": ["mapping_monitor_opt"],
    "network": ["bleAdvertisingOnOff", "deviceName", "ipv6Enable", "wifiConnection", "wiredConnection", "wolwowlOnOff"],
    "option": ["_3dModeEstreamer", "additionalAudioSelection", "addressInfo", "adjustments", "artisticDisplayTimer", "audioGuidance", "audioGuidancePitch", "audioGuidanceSpeed", "audioGuidanceVolume", "autoSmartServiceCountry", "broadcastInfoNoti", "cameraResourcePermission", "cicNumber", "country", "countryRegion", "curvature", "dataService", "demoMode", "eStreamerPosition", "emergencyAlert", "emergencyInformationAtsc30", "emergencyInformationLanguageAtsc30", "enabling3dSettingsMenu", "faultLogUpload", "filePermission", "fileapiResourcePermission", "focusEmphasis", "hbbTV", "hbbTvDeviceId", "hbbTvDnt", "hddEcoMode", "helpOnSettings", "highContrast", "homeWallPaper", "hybridCast", "ibb", "interactive-service", "interactive-service-hdmi", "interactive-service-id", "interactivity", "japanCitySelection", "lineView", "livePlus", "livePromotion", "localeCountryGroup", "logoLight", "longPressInterval", "menuTransparency", "mhegGuide", "motionSensorSensitivity", "multiChannelAudio", "nearbyDevicePermission", "personalizedImgList", "personalizedString", "phlCitySelection", "pointerSize", "pointerSpeed", "pqPlus", "quickStartMode", "remoteSetting", "remoteSlowInput", "repeatInterval", "restoreCurve", "screenMagnification", "screenOffTime", "screenSaverTimer", "seniorMode", "setId", "signLanguageGuidance", "signLanguageGuidanceAvatarType", "signLanguageGuidancePosition", "signLanguageGuidanceSize", "signLanguageGuidanceSpeed", "smartServiceCountryCode3", "soundPlus", "standByLight", "storeLogo", "storeMode", "storeMode2", "subtitleLanguageFirst", "subtitleLanguageSecond", "supplementaryAudio", "teletextLanguageFirst", "teletextLanguageSecond", "thirdPartyCookie", "usageCare", "vsn", "zipcode"],
    "other": ["444BypassHDMI1", "444BypassHDMI2", "444BypassHDMI3", "444BypassHDMI4", "444BypassHDMINone", "adaptivePowerDelivery", "aodPictureSyncMode", "autoInputSwitch", "autoSLZoomWithObjectDetection", "colorimetry", "contentRecommendation", "dolbyVSVDBVer", "enableDolbyVisionPC", "enableLighting", "eotf", "gameMode", "inputCompatibilityMode", "lightingAutoBrightness", "lightingBrightness", "lightingColorMode", "lightingEnable", "lightingMode", "lightingTnativeBrightness", "lightingTnativeMode", "lowPowerMode", "masterLuminanceLevel", "masteringColor", "masteringPeak", "maxCLL", "maxFALL", "oledCareMode", "oledCareRecommendation", "othersColorMode", "qmsVrr", "simplinkEnable", "soundSyncModeColor", "soundSyncModeDisplayMode", "soundSyncModeFrequency", "soundSyncModeStaticColor", "staticModeColor", "staticModeColor1", "staticModeColor2", "staticModeColor3", "staticModeColor4", "uhdDeepColor", "whiteColorMode"],
    "picture": ["DtmMastering10k", "DtmMastering1k", "DtmMastering4k", "DtmProfessional", "RGBWDotPatternMax", "RGBWDotPatternMin", "RGBWDotPatternOnOff", "ScreenOff", "adjustingLuminance", "adjustingLuminance10pt", "applyToAllInput", "backlight", "blackLevel", "brightness", "brightnessActive", "clarityActive", "color", "colorActive", "colorGamut", "colorManagementColorSystem", "colorManagementHueBlue", "colorManagementHueCyan", "colorManagementHueGreen", "colorManagementHueMagenta", "colorManagementHueRed", "colorManagementHueYellow", "colorManagementLuminanceBlue", "colorManagementLuminanceCyan", "colorManagementLuminanceGreen", "colorManagementLuminanceMagenta", "colorManagementLuminanceRed", "colorManagementLuminanceYellow", "colorManagementSaturationBlue", "colorManagementSaturationCyan", "colorManagementSaturationGreen", "colorManagementSaturationMagenta", "colorManagementSaturationRed", "colorManagementSaturationYellow", "colorTemperature", "contrast", "demandResponse", "displayType", "dolbyPrecisionDetail", "dynamicColor", "dynamicContrast", "energySaving", "energySavingAutoMin", "energySavingModified", "erpUserConfirm", "expressionEnhancer", "eyeComfortMode", "filmMakerMode", "filmmakerAmbientLight", "gamma", "gammaLevel", "hdrDynamicToneMapping", "hgig", "inputListForCopyCms", "inputListForCopyPsm", "inputListForCopyWb", "isfMode", "keyListForCopyCms", "keyListForCopyPsm", "keyListForCopyWb", "localDimming", "logoLuminanceAdjust", "motionEyeCare", "motionPro", "motionProOLED", "mpegNoiseReduction", "nearBlackDetail", "noiseReduction", "oledPanelSettings", "peakBrightness", "pictureMode", "pictureSettingModified", "pictureTest", "precisionHdrMasterPro", "psmListForExpert", "realCinema", "responseTime", "scheduledPixelCleaning", "screenShift", "sharpness", "smoothGradation", "superResolution", "tint", "truMotionBlur", "truMotionJudder", "truMotionMode", "wb10PointsGammaValue", "wb22PointsGammaValue", "whiteBalanceActive", "whiteBalanceApplyAllInputs", "whiteBalanceBlue", "whiteBalanceBlue10pt", "whiteBalanceBlueGain", "whiteBalanceBlueOffset", "whiteBalanceCodeValue", "whiteBalanceCodeValue10pt", "whiteBalanceColorTemperature", "whiteBalanceGreen", "whiteBalanceGreen10pt", "whiteBalanceGreenGain", "whiteBalanceGreenOffset", "whiteBalanceIre", "whiteBalanceIre10pt", "whiteBalanceLuminance", "whiteBalanceMethod", "whiteBalancePoint", "whiteBalanceRed", "whiteBalanceRed10pt", "whiteBalanceRedGain", "whiteBalanceRedOffset", "zoomRatio", "zoomVPosition"],
    "sound": ["_3dSoundZooming", "aiSound", "audioBalance", "audioDescription", "audioDescriptionAtsc30", "audioDescriptionVolume", "auracast", "autoVolume", "avSync", "avSyncBtSurround", "avSyncBypassInput", "avSyncSpdif", "avSyncSpeaker", "avSyncWisa", "blegattSoundbar", "bluetoothMode", "clearVoice", "deviceCntOfBtGeneral", "dialogEnhancementAtsc30", "dialogEnhancementVolumeAtsc30", "digitalAudioPriority", "eArcSupport", "hearingAid", "inputAudioFormatDP1", "inputAudioFormatDP2", "inputAudioFormatHDMI1", "inputAudioFormatHDMI2", "inputAudioFormatHDMI3", "inputAudioFormatHDMI4", "inputAudioFormatUSBC1", "inputAudioFormatUSBC2", "lgSoundSync", "magicSoundModified", "magicSoundTime", "magicSpaceSound", "slidingSpeaker", "slidingSpeakerMelody", "smartSoundMode", "soundAlive", "soundEffect", "soundMode", "soundOutput", "soundOutputDigital", "soundTest", "startupSound", "supportATMOS", "tvPowerSound", "tvSetupConfiguration", "userEqualizer", "userEqualizerValues", "virtualSurround", "visualImpaired", "volumeControl", "wisaSpeakerChannelType"],
    "support": ["automaticUpdate"],
    "time": ["autoOff15Min", "autoOff2HourOnTimer", "automaticStandBy", "dpmsAutoOff", "noOperationAutoOff", "noSignalAutoOff", "offTimerEnable", "offTimerHour", "offTimerMinute", "offTimerWeekday", "onTimerAppId", "onTimerChannel", "onTimerEnable", "onTimerHour", "onTimerMinute", "onTimerVolume", "onTimerWeekday", "timeZone", "warmTimerEnable", "warmTimerWeekday1Enable", "warmTimerWeekday2Enable", "warmTimerWeekday3Enable", "warmTimerWeekday4Enable", "warmTimerWeekday5Enable", "warmTimerWeekday6Enable", "warmTimerWeekday7Enable", "warmTimerWeekday8Enable", "warmTimerWeekend1Enable", "warmTimerWeekend2Enable", "warmTimerWeekend3Enable", "warmTimerWeekend4Enable", "warmTimerWeekend5Enable", "warmTimerWeekend6Enable", "warmTimerWeekend7Enable", "warmTimerWeekend8Enable"],
    "timer": ["noMenuAutoOff15Min"],
    "voiceframework": ["thinQtvTTSBlock"],
    "": ["localeInfo"]
};

// A value table is the whole set LG's own labels name; on/off keys are listed in `onoff`, wolwowlOnOff is true/false.
var values = {
    "picture.pictureMode": ["vivid", "hdrVivid", "normal", "eco", "hdrEco", "cinema", "hdrStandard", "hdrCinema", "sports", "game", "photo", "hdrGame", "hdrCinemaBright", "hdrEffect", "hdrExternal", "dolbyHdrVivid", "dolbyHdrCinemaBright", "dolbyHdrCinema", "dolbyHdrDarkAmazon", "dolbyHdrGame", "dolbyHdrStandard", "expert1", "expert2", "filmMaker", "hdrFilmMaker", "personalized", "hdrPersonalized", "dolbyHdrPersonalized"],
    "picture.energySaving": ["auto", "off", "min", "med", "max", "screen_off"],
    "picture.truMotionMode": ["off", "smooth", "clear", "user", "cinemaClear", "natural"],
    "picture.blackLevel": ["low", "medium", "high", "auto", "limited", "full"],
    "picture.logoLuminanceAdjust": ["off", "low", "high", "light", "strong"],
    "sound.soundMode": ["aiSoundPlus", "aiSound", "aiSound_soundbar", "standard", "news", "music", "movie", "sports", "game", "pagode", "sertaWego", "brazilianPunk", "game_rts", "game_fps", "dtsVirtualX", "custom", "bassBlast", "personalized", "precision"],
    "sound.soundOutput": ["tv_speaker", "external_arc", "external_optical", "bt_soundbar", "wow_cast", "mobile_phone", "lineout", "headphone", "usb_speaker", "tv_speaker_bluetooth", "tv_speaker_wow_cast", "tv_external_speaker", "tv_speaker_external_arc", "tv_speaker_headphone", "wisa_speaker"],
    "sound.soundOutputDigital": ["auto", "pcm", "passThrough"],
    "option.pointerSpeed": ["slow", "normal", "fast"],
    "option.pointerSize": ["small", "medium", "large"],
    "aspectRatio.arcPerApp": ["21x9", "16x9", "4x3", "14x9", "just_scan", "original", "full_wide", "zoom", "zoom2", "cinema_zoom", "vertZoom", "allDirZoom", "32x9", "32x12", "twinZoom"],
    "time.noOperationAutoOff": ["off", "3", "6", "12", "24"],
    "other.oledCareMode": ["off", "comfortCare", "cinemaCare", "kidsCare"],
    "option.logoLight": ["off", "low", "medium", "high"],
    "option.screenSaverTimer": ["3", "10", "20", "30"]
};

var onoff = ["picture.eyeComfortMode", "picture.screenShift", "picture.filmMakerMode", "picture.filmmakerAmbientLight", "aiPicture.ai_Picture", "aiPicture.ai_Genre",
             "aiPicture.ai_Brightness", "general.remotePointer", "option.quickStartMode", "other.simplinkEnable", "option.audioGuidance", "option.highContrast",
             "time.autoOff15Min", "time.noSignalAutoOff", "caption.captionEnable", "sound.autoVolume", "sound.avSync"];
var truefalse = ["network.wolwowlOnOff"];

// Numbers: [min, max]. LG's slider limits are in the service's own descriptions; these are the outer bounds the pages stay inside.
var ranges = {"picture.backlight": [0, 100], "picture.contrast": [0, 100], "picture.brightness": [0, 100], "picture.color": [0, 100], "picture.tint": [-50, 50],
              "picture.sharpness": [0, 50], "sound.audioBalance": [-50, 50],
              "sound.avSyncSpdif": [0, 400], "sound.avSyncSpeaker": [0, 400], "sound.avSyncWisa": [0, 400], "sound.avSyncBtSurround": [0, 400]};

// The screens LG's Settings app opens from launch parameters ({target: name}): the keys of its router table (`rV` in main.js).
// An unknown name lands on its Picture page, so only a row that names one of these goes where its text says.
var targets = [
    "picture", "sound", "general", "support", "AddOptions", "Brightness", "Color", "ColorManagement", "WhiteBalance", "Clarity",
    "SoundOut", "WiredSpeaker", "WirelessSpeaker", "Bluetooth", "Wowcast", "WiSASoundSync", "MultiSpeaker", "BluetoothSurround",
    "WowcastOrchestra", "UsbDevice", "AdditionalSound", "InputAudioFormat", "AvSync", "LifeOnScreenMode", "AiService",
    "AiRecommendation", "MagicSpaceSound", "LightingSetting", "SeniorSetting", "channel", "System", "AdditionalGeneral",
    "HomeSettings", "ScreenSaverSettings", "PointerOptions", "EmergencyInformation", "MotionSensor",
    "ArtisticDisplayStartTimePanel", "DpmsAutoOffPanel", "accessibility", "parentalControls", "ApplicationLocks", "InputLocks",
    "RatingLocks", "ContentsLocks", "AudioGuidance", "HighQualityTTS", "SignLanguageZoom", "DialogEnhancementAtsc",
    "ClosedCaption", "AiClosedCaptionQR", "JPClosedCaption", "PHClosedCaption", "Subtitles", "TVInformation", "HbbTV", "Message",
    "CasModuleInfo", "CasModuleInfoFHD", "CopyChannels", "InteractiveService", "InteractiveServiceHdmi", "OledScreenSaver",
    "ExternalDevice", "ExternalDeviceSettings", "MobileTVOn", "EnergySaving", "EnergySavingSetting", "PolicyNotice",
    "SignatureTVSetting", "AlwaysOn", "SoftwareUpdate", "DateTime", "DeepColor", "Timer", "TimerPowerOn", "TimerPowerOff",
    "Location", "ServiceCountry", "Language", "LegalNotice", "AspectRatioSettings", "TvInstallationTypeItemPanel",
    "DtvAudioSettingItemPanel", "DigitalSoundOutItemPanel", "AudioTypeItemPanel", "AdditionalAudioSelectionItemPanel",
    "MultiOutputItemPanel", "RemoteSlowInput", "MultiFeedItemPanel", "SystemColorItemPanel", "ChannelScrollingOptionItemPanel",
    "OledCare", "CarePictureSettings", "PixelCleaning", "LogoLuminance", "ScreenOffTime", "ZeroConnectBoxSettings",
    "ZeroConnectBoxLocation", "WirelessBoxSettings", "LightingSettings", "ShadeSettings", "Battery", "CurvatureSetting",
    "CurvatureButtonSetting", "FirstCurvatureSetting", "SecondCurvatureSetting", "UserCurvatureSetting", "HomeStoreMode",
    "MemoryOptimizer", "ConnectBluetooth", "ResponseTime", "DeletePersonalInfo", "DeletePersonalViewDetails",
    "DeletePersonalVoiceDetails", "Advertising", "Camera", "MicroPhone", "NearbyDevice"
];

// The TV's own description of a key (LgDesc.js) is the table for every key that has no hand-kept one above: a value LG's screen
// does not list, or a number outside the key's range, is refused.
function domain(ds) {
    if (values[ds]) return { kind: "enum", list: values[ds] };
    if (onoff.indexOf(ds) >= 0) return { kind: "enum", list: ["on", "off"] };
    if (truefalse.indexOf(ds) >= 0) return { kind: "enum", list: ["true", "false"] };
    if (ranges[ds]) return { kind: "range", min: ranges[ds][0], max: ranges[ds][1] };
    var t = Desc.d[ds];
    if (!t || t[t.length - 1] !== 1) return null;                // a key the TV does not check takes any value
    if (t[0] === "E" && t[1].length + t[2].length > 0) return { kind: "enum", list: t[1].concat(t[2]) };
    if (t[0] === "R") return { kind: "range", min: t[1], max: t[2] };
    return null;
}

// null when the write is accepted, else the reason the service would give. A key that holds a list (the equalizer's five levels) is
// checked level by level against the key's own range.
function refuse(category, key, value) {
    var ds = category + "." + key;
    if (!keys[category] && !Desc.d[ds]) return "unknown category " + category;
    if ((keys[category] || []).indexOf(key) < 0 && !Desc.d[ds]) return "unknown key " + ds;
    var d = domain(ds);
    if (!d) return null;
    if (Array.isArray(value)) {
        for (var i = 0; i < value.length; i++) {
            var why = refuse(category, key, value[i]);
            if (why) return why;
        }
        return null;
    }
    var s = String(value);
    if (d.kind === "enum") return d.list.indexOf(s) >= 0 ? null : "value " + s + " is not one of " + d.list.join("|") + " for " + ds;
    var n = Number(s);
    return isNaN(n) || n < d.min || n > d.max ? "value " + s + " is outside " + d.min + ".." + d.max + " for " + ds : null;
}
