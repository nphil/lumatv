#!/usr/bin/env python3
"""A stand-in for LG's settings service (`luna://com.webos.service.settings`) and the other luna services Settings reads, for the Rust
shell's host runs (`tests/test_rust_settings.py`, `tests/render_rust_settings.py`). It answers as the set answers (OLED65C4, webOS 24; see
`tests/qmlhost/stubs/WebOSServices/FakeSettings.js` for the QML host's twin): a read gives the keys it has in `settings` and names the
rest in `errorKey` with `returnValue: false`; a per-app key is answered only with `app_id`; a description comes as `results`; a write
changes the value and is kept in `writes`. `answers` maps the end of another luna uri to its payload (or a function of the call).
"""
import copy
import json

VALUES = {
    "picture": {"pictureMode": "filmMaker", "backlight": "80", "contrast": "85", "brightness": "50", "color": "50", "tint": "0",
                "sharpness": "10", "energySaving": "off", "energySavingAutoMin": "3", "screenShift": "on", "logoLuminanceAdjust": "strong",
                "truMotionMode": "off", "blackLevel": "auto", "filmMakerMode": "on", "eyeComfortMode": "off", "dynamicContrast": "off",
                "peakBrightness": "off", "gamma": "high2", "motionEyeCare": "off", "colorTemperature": "0", "colorGamut": "auto",
                "dynamicColor": "off", "superResolution": "off", "noiseReduction": "off", "mpegNoiseReduction": "off",
                "smoothGradation": "off", "realCinema": "on", "motionPro": "off", "applyToAllInput": "picture",
                "scheduledPixelCleaning": "off"},
    "aiPicture": {"ai_Picture": "off", "ai_Genre": "off", "ai_Brightness": "on"},
    "aspectRatio": {"arcPerApp": "original", "justScan": "on"},
    "sound": {"soundMode": "standard", "soundOutput": "external_arc", "audioBalance": "0", "soundOutputDigital": "passThrough",
              "avSync": "on", "avSyncSpdif": "0", "avSyncSpeaker": "0", "avSyncWisa": "0", "avSyncBtSurround": "0", "avSyncBypassInput": "off",
              "eArcSupport": "on", "autoVolume": "off", "supportATMOS": "on", "userEqualizer": "off", "tvSetupConfiguration": "stand",
              "magicSpaceSound": "off", "applyToAllInputSSM": "sound", "digitalAudioPriority": "auto"},
    "general": {"remotePointer": "on", "lifeOnScreenMode": "off", "homeAutoLaunch": "on", "homePromotion": "off", "aiNudge": "off",
                "aiSettingsNudge": "off", "screenSaverAd": "off", "tvOnSound": "off", "doNotSellMyPersonalInformation": "off", "lmt": "off"},
    "network": {"deviceName": "Living Room TV", "wolwowlOnOff": "true"},
    "option": {"pointerSpeed": "normal", "pointerSize": "medium", "quickStartMode": "on", "audioGuidance": "off", "highContrast": "off",
               "focusEmphasis": "off", "screenMagnification": "off", "adjustments": {"blackAndWhite": "off", "colorInversion": "off"},
               "country": "USA", "localeCountryGroup": "US", "screenSaverTimer": "10", "livePlus": "off", "livePromotion": "off",
               "lifeOnScreenEnergySaving": "auto", "lifeOnScreenNotification": "true", "lifeOnScreenUsingMotionSensor": "true",
               "broadcastInfoNoti": "off", "screenOff": "off"},
    "other": {"simplinkEnable": "on", "lgLogoDisplay": "on", "lightingEnable": "on", "oledCareMode": "off", "oledCareRecommendation": "off",
              "contentRecommendation": "off"},
    "time": {"noOperationAutoOff": "3", "autoOff15Min": "on", "automaticStandBy": "0", "timeZone": {"ZoneID": "America/New_York", "City": "New York"},
             "onTimerEnable": "off", "offTimerEnable": "off"},
    "caption": {"captionEnable": "off"},
    "": {"localeInfo": {"keyboards": ["en"], "locales": {"UI": "en-US", "FMT": "en-US", "STT": "en-US"}, "clock": "locale", "timezone": ""},
         "eulaStatus": {"generalTermsAllowed": True, "voiceAllowed": False}},
}

# what the TV's description says of a key: `ui` (active, visible) and `values` (a range or the list it offers)
DESCS = {
    "picture.backlight": {"values": {"range": {"min": 0, "max": 100, "interval": 1}}},
    "picture.contrast": {"values": {"range": {"min": 0, "max": 100, "interval": 1}}},
    "picture.brightness": {"values": {"range": {"min": 0, "max": 100, "interval": 1}}},
    "picture.energySavingAutoMin": {"values": {"range": {"min": 0, "max": 10, "interval": 1}}},
    "picture.gamma": {"values": {"arrayExt": [{"value": "low"}, {"value": "medium"}, {"value": "high1"}, {"value": "high2"}]}},
    "picture.dynamicContrast": {"values": {"arrayExt": [{"value": "off"}, {"value": "low"}, {"value": "medium"}, {"value": "high"}]}},
    "picture.peakBrightness": {"values": {"arrayExt": [{"value": "off"}, {"value": "low"}, {"value": "medium"}, {"value": "high"}]}},
    "picture.blackLevel": {"values": {"arrayExt": [{"value": "auto"}, {"value": "low"}, {"value": "high"}]}},
    "picture.hdrDynamicToneMapping": {"ui": {"visible": False}},
    "picture.nearBlackDetail": {"ui": {"visible": False}},
    "sound.soundOutput": {"values": {"arrayExt": [{"value": v} for v in ("tv_speaker", "external_arc", "external_optical", "lineout", "headphone",
                                                                         "tv_speaker_external_arc", "tv_external_speaker", "tv_speaker_headphone")]}},
    "sound.avSyncSpdif": {"values": {"range": {"min": 0, "max": 400, "interval": 5}}},
    "sound.audioBalance": {"values": {"range": {"min": -10, "max": 10, "interval": 1}}},
    "sound.supportATMOS": {"ui": {"active": False}},
    "option.screenSaverTimer": {"values": {"arrayExt": [{"value": "3"}, {"value": "10"}, {"value": "20"}, {"value": "30"}]}},
    "other.oledCareMode": {"values": {"arrayExt": [{"value": "off"}, {"value": "comfortCare"}, {"value": "cinemaCare"}, {"value": "kidsCare"}]}},
    "time.automaticStandBy": {"values": {"arrayExt": [{"value": "0"}, {"value": "2"}, {"value": "4"}, {"value": "6"}, {"value": "8"}]}},
}

PER_APP = {"picture": ["truMotionMode"], "aspectRatio": ["arcPerApp", "justScan"]}
DIMENSIONS = {"picture": {"pictureMode": "filmMaker", "dynamicRange": "sdr", "input": "hdmi2"}}


class LgService:
    def __init__(self, values=None, descs=None, answers=None, drop=()):
        self.values = copy.deepcopy(values if values is not None else VALUES)
        self.descs = copy.deepcopy(descs if descs is not None else DESCS)
        self.answers = dict(answers or {})
        self.drop = set(drop)          # "category.key" names the TV does not have
        self.writes = []               # (category, settings, app_id, dimension)
        self.calls = []

    def respond(self, doc):
        """The payload for a luna call of the client, or None when this service does not answer it."""
        uri, p = doc.get('uri', ''), doc.get('payload') or {}
        self.calls.append((uri, p))
        if uri.endswith('/getSystemSettings'):
            cat, keys, app = p.get('category', ''), p.get('keys', []), p.get('app_id')
            have = self.values.get(cat, {})
            settings = {k: have[k] for k in keys if k in have and f"{cat}.{k}" not in self.drop and (k not in PER_APP.get(cat, []) or app)}
            missing = [k for k in keys if k not in settings]
            out = {'returnValue': not missing, 'category': cat, 'settings': settings}
            if missing:
                out['errorKey'] = missing
            if cat in DIMENSIONS and settings:
                out['dimension'] = DIMENSIONS[cat]
            return out
        if uri.endswith('/getSystemSettingDesc'):
            cat, keys = p.get('category', ''), p.get('keys', [])
            results = [dict(category=cat, key=k, **self.descs[f"{cat}.{k}"]) for k in keys if f"{cat}.{k}" in self.descs]
            return {'returnValue': True, 'results': results}
        if uri.endswith('/setSystemSettings'):
            cat = p.get('category', '')
            self.writes.append((cat, p.get('settings', {}), p.get('app_id'), p.get('dimension')))
            self.values.setdefault(cat, {}).update(p.get('settings', {}))
            return {'returnValue': True}
        for tail, answer in self.answers.items():
            if uri.endswith(tail):
                return answer(doc) if callable(answer) else answer
        return None

    def written(self, ds):
        """The last value written for "category.key", or None."""
        cat, _, key = ds.rpartition('.')
        for c, settings, _, _ in reversed(self.writes):
            if c == cat and key in settings:
                return settings[key]
        return None
