# LG settings coverage: every item, and where LumaTV has it

Goal (Nitin, 2026-10-25): every item of LG's Settings app, even the rarely used ones, and LG's other settings menus, is covered by LumaTV's own Settings, so LG's Settings app (`com.palm.app.settings`, about 93 MB while open) never has to open. One emergency row, "Open LG Settings", stays in System.

## Resume here

Inventory done: 287 groups of LG items: 48 covered, 46 built, 110 to build, 83 cannot.

Built and pushed: Accessibility (Audio Guidance, Closed Caption with its Text, Background and Subtitles tabs, the display switches, Audio options), Time & power (clock, Power On and Power Off timers, auto off, Screen Off), Bluetooth speakers and controllers, Join Network and IP Settings, TV Name, IPv6, HDMI settings.

Left: Picture (Brightness, Colour, Clarity, Expert, White Balance, Fine Tune, zoom), Sound (Output, Advanced, per-input audio), Language and Location, Support (TV information, update, self-diagnosis, privacy, reset), Safety (lock PIN prompt, locks), Channels, and the one-off rows (Always Ready, extended press time, AI recommendations, OLED care extras). The "More in LG settings" rows stay on the pages not yet built.

Next: the build order below, one page per commit. Each built row flips its status here from `build` to `built`; a row that turns out not to be possible moves to `cannot` with the reason the TV gave.

## Sources

- LG's Settings bundle `research/lgsettings/main.js`, read with a JavaScript parser: the router table `rV` and the per-category panel maps give the menu tree, each panel's components give the items, the `BH(...)`/`RH(...)` reads and `changeSetting` writes give the keys, and its `qH` tables give the value names.
- The TV's own answers: `getSystemSettingDesc` for 834 keys of 13 categories on the owner's OLED65C4 (webOS 24, USA), read only, 2026-10-25. "TV answers" lists the values LG's own screen offers for the key (or its range and step). "hidden now" is `ui.visible: false` at the time of the read; some rows are state dependent (HDR-only rows are hidden while an SDR picture plays), the rest LG never shows on this set. "no description" means the TV has the key but no description of it, or no key at all.
- Quick Settings: the QML on the TV (`uiinv/qml/QuickSettings`, its `Constants.js` and controllers).
- Luna methods: the `service` and `method` pairs the bundle calls. `com.webos.surfacemanager`, the identity the compositor calls as, has the `all` permission group and may call any `com.webos.*`, `com.palm.*` or `com.lge.*` service (`client-permissions.d` and its role file on the TV); how each service answers to it has not been read.
- LG's names are the labels used in LumaTV. Current values are not recorded here; the secrets (the lock PIN, the Wi-Fi password, account data) are never read.

## Status

- `covered`: LumaTV has it today.
- `build`: to be built in the page named in the LumaTV column (`built` once it is, with the commit in the changelog).
- `cannot: why`: not possible or not wanted, with the reason. These are also the rows LG's own Settings still has to serve, through the emergency row.

## Build order

Most often used first: captions and Accessibility, Time & power (timers, auto off, clock), Bluetooth pairing, Network (join and IP: at the remote only), HDMI settings, Picture expert (Clarity, Brightness, Colour, white balance), Sound output and Advanced, Language and location, Support (TV information, software update, diagnosis, privacy, reset), Safety (PIN and locks), Channels, and the remaining one-off rows (Always Ready, extended press time, AI recommendation).

## Rules the new rows follow

- LG's category, key and value names only; a value the TV does not list is refused (the offline service stub refuses it the same way). Value lists and ranges come from the TV's description, not from a table of guesses.
- A row shows LG's name and its value. A second line carries state only; what a setting does goes behind the row's help.
- Reset, software update, the lock PIN, deleting personal information and network changes are done at the remote, behind the existing confirm question (danger style where it destroys). The PIN is typed into LG's own prompt (`com.webos.notification createPincodePrompt`); LumaTV acts on its yes or no and never sees the digits.
- Nothing here is added to the helper, the AI access tools or the MCP: an assistant can press the media keys and open apps, and can reach no Settings row.

## Picture

### Picture (top level)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Select Mode | picture.pictureMode | personalized, vivid, normal, eco, cinema, sports, game, filmMaker, expert1, expert2 | Picture › Mode (cards) | covered — HDR and Dolby Vision signals use the hdr*/dolbyHdr* names the TV lists for them; the cards write the SDR names only |
| Display Type | option.displayType | no description | - | cannot: the TV answers no such key (monitor and signage models) |
| Personalized Picture Wizard | luna app com.webos.app.picturewizard |  | - | cannot: LG's own image-survey app; the result lands in `picture.pictureMode` personalized |
| Aspect Ratio: 16:9, Original, 4:3 | aspectRatio.arcPerApp (per app_id) | no description | Picture › Mode | covered |
| Aspect Ratio: Vertical Zoom, 4-Way Zoom | aspectRatio.arcPerApp + aspectRatio.vertZoomVRatio, vertZoomVPosition, allDirZoomHRatio, allDirZoomVRatio, allDirZoomHPosition, allDirZoomVPosition | no description | Picture › Mode | build — two more values on the row, the zoom sliders under it while one is chosen |
| Just Scan | aspectRatio.justScan | on, off | Picture › Mode | build |

### Advanced Settings › Brightness

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| OLED Pixel Brightness | picture.backlight | 0 to 100 | Picture › Mode, Brightness | covered |
| Adjust Contrast | picture.contrast | 0 to 100 | Picture › Brightness | covered |
| Screen Brightness | picture.brightness | 0 to 100 | Picture › Brightness | covered |
| Auto Dynamic Contrast | picture.dynamicContrast | off, low, medium, high | Picture › Brightness | build |
| HGiG | picture.hgig (the TV has no description for it) | no description | Picture › Expert | build — shown only where the TV answers the key |
| Dynamic Tone Mapping | picture.hdrDynamicToneMapping, picture.DtmProfessional, picture.DtmMastering1k, DtmMastering4k, DtmMastering10k | on, off, HGIG; hidden now | Picture › Expert | build — LG shows it for HDR signals only |
| Expression Enhancer | picture.expressionEnhancer | off, detail, highlight; hidden now | Picture › Expert | build |
| Peak Brightness | picture.peakBrightness | off, low, medium, high | Picture › Brightness | build |
| Gamma (Adjust Brightness) | picture.gamma, picture.gammaLevel | low, medium, high1, high2 | Picture › Brightness | build |
| FILMMAKER MODE Ambient Light | picture.filmmakerAmbientLight | no description | Picture › Mode | covered — the C4 answers no such key; the row stays out |
| Near Black Detail | picture.nearBlackDetail | -3 to 3; hidden now | Picture › Brightness | build — LG shows it for HDR only |
| Video Range (Black Level) | picture.blackLevel | auto, low, high | Picture › Brightness | build |
| LED Local Dimming | picture.localDimming | off, low, medium, high; hidden now | - | cannot: edge-lit LCD only; the C4's panel is OLED, LG hides it |
| Motion Eye Care | picture.motionEyeCare | off, on | Picture › Brightness | build |

### Advanced Settings › Color

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Color Depth | picture.color | 0 to 100 | Picture › Colour | covered |
| Tint | picture.tint | -50 to 50 | Picture › Colour | covered |
| Color Gamut | picture.colorGamut | auto, native, dynamic, adobeRGB | Picture › Colour | build |
| Fine Tune: Color Adjustment | picture.dynamicColor | off, low, medium, high, user | Picture › Colour | build |
| Fine Tune: Select Color, Adjust Saturation, Adjust Tint (hue), Adjust Luminance | picture.colorManagementColorSystem, colorManagementSaturation*, colorManagementHue*, colorManagementLuminance* (Red, Green, Blue, Cyan, Magenta, Yellow) | red, green, blue, cyan, magenta, yellow | Fine Tune (child of Colour) | build |
| Fine Tune: Reset | luna settingsservice resetSystemSettings (the colorManagement* keys) |  | Fine Tune | build — asks first |
| White Balance: Color Temperature | picture.colorTemperature | -50 to 50 | Picture › Colour | build |
| White Balance: Method, Point, Signal Level, Code Value | picture.whiteBalanceMethod (2, 10, 22 point), whiteBalancePoint, whiteBalanceIre, whiteBalanceIre10pt, whiteBalanceCodeValue | 2, 10, 22 | White balance (child of Colour) | build — Code Value is a test-pattern drive LG hides on this set |
| White Balance: Luminance, Red, Green, Blue | picture.whiteBalanceLuminance, whiteBalanceRed/Green/Blue (22 values), whiteBalanceRed10pt/Green10pt/Blue10pt, whiteBalanceRedGain/GreenGain/BlueGain, whiteBalanceRedOffset/GreenOffset/BlueOffset, adjustingLuminance | 50 to 500 | White balance | build |
| White Balance: Reset | luna settingsservice resetSystemSettings (whiteBalance* keys) |  | White balance | build — asks first |

### Advanced Settings › Clarity

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Adjust Sharpness | picture.sharpness | 0 to 50 | Picture › Clarity | covered |
| Super Resolution | picture.superResolution | off, low, medium, high | Picture › Clarity | build |
| Noise Reduction | picture.noiseReduction | off, low, medium, high, auto | Picture › Clarity | build |
| MPEG Noise Reduction | picture.mpegNoiseReduction | off, low, medium, high, auto | Picture › Clarity | build |
| Smooth Gradation | picture.smoothGradation | off, low, medium, high | Picture › Clarity | build |
| Real Cinema | picture.realCinema | on, off | Picture › Clarity | build |
| TruMotion (Off, Natural, Smooth, Clear / Cinema Clear, User) | picture.truMotionMode | no description | Picture › Clarity | covered — Natural, Smooth, Clear, Off; Cinema Clear and User to build |
| TruMotion: De-Judder, De-Blur (User mode) | picture.truMotionJudder, picture.truMotionBlur | 0 to 10 | Picture › Clarity | build |
| OLED Motion Pro | picture.motionPro (on, off; LG's radio form picture.motionProOLED is hidden on this set) | on, off | Picture › Clarity | build |
| Precision Detail | picture.dolbyPrecisionDetail | on, off; hidden now | Picture › Clarity | build — Dolby Vision content only |

### Advanced Settings › other items

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Response Time | picture.responseTime | no description | - | cannot: the TV answers no such key (monitor models only) |
| Precision HDR Master Pro | picture.precisionHdrMasterPro (no description on this set) | no description | Picture › Expert | build — shown only where the TV answers the key |
| FILMMAKER MODE Auto Start | picture.filmMakerMode | on, off | Picture › Expert | build |
| Reduce Blue Light | picture.eyeComfortMode | on, off | Picture › Brightness | covered — LumaTV's row is named Eye comfort |
| DR Mode (Demand Response) | picture.demandResponse | on, off; hidden now | - | cannot: utility demand-response programme; LG hides it on this set |
| Apply to All Inputs | picture.applyToAllInput | picture, done | Picture › Expert | build — asks first |
| Reset (the picture mode's settings) | luna settingsservice resetSystemSettings (category picture, current mode) |  | Picture › Expert | build — asks first, danger style |

### General › AI Service (picture)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| AI Picture Pro | aiPicture.ai_Picture | on, off | Picture › Mode | covered |
| AI Brightness Settings | aiPicture.ai_Brightness | on, off | Picture › Expert | covered — named AI brightness |
| AI Genre Selection | aiPicture.ai_Genre | on, off | Picture › Expert | covered — named AI genre selection |

### Hidden: HDMI Signalling Override (a key sequence on Select Mode)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Colorimetry, EOTF, Mastering Color, Mastering Peak, MaxCLL, MaxFALL, Dolby Vision VSVDB version, Reset | other.colorimetry, eotf, masteringColor, masteringPeak, maxCLL, maxFALL, dolbyVSVDBVer | auto, bt709, bt601, bt2020, p3D65 | Picture › Expert | build — LG reaches it by a button sequence; LumaTV shows it as one row group for the signal in front |
| RGBW Dot Pattern (factory) | picture.RGBWDotPatternOnOff, RGBWDotPatternMax, RGBWDotPatternMin | on, off | - | cannot: factory panel test, LG hides it from owners |

## Sound

### Sound (top level)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Select Mode | sound.soundMode (dimension soundOutput) | aiSoundPlus, aiSound, standard, movie, news, sports, music, game | Sound › Mode | covered — AI Sound Pro, Standard, Cinema, Clear Voice Pro, Sports, Music, Game; the others (AI Sound, Bass Blast, Personalized) follow the TV's list |
| Personalized Sound Wizard | luna app com.webos.app.soundwizard |  | - | cannot: LG's own listening-test app |
| Sound Out (all choices below) | sound.soundOutput | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output | covered — TV speakers and receiver today; the full list to build |
| Use TV Speaker | sound.soundOutput tv_speaker | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output | build |
| Use Wired Speaker: Optical Out Device | sound.soundOutput external_optical | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output | build |
| Use Wired Speaker: HDMI (ARC) Device | sound.soundOutput external_arc | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output | covered |
| Use Wired Speaker: Audio Out Device (line out) | sound.soundOutput lineout | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output | build |
| Use Wired Speaker: Wired Headphones | sound.soundOutput headphone | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output | build |
| Use Wired Speaker: USB Device | sound.soundOutput usb_speaker; luna com.webos.service.audio setSoundOutput, com.webos.service.audio/tv getSoundCardsState | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output | build |
| Use Wireless Speaker: Bluetooth Device | sound.soundOutput bt_soundbar; luna com.webos.service.btaudiosrc getPairedDevices, getDeviceStatus, startDiscovery, connect, disconnect, removePairedDevice, onDeviceSelectionMenu | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output › Bluetooth Speakers | built |
| Bluetooth: Speaker Connection Settings (1 or 2 devices) | sound.deviceCntOfBtGeneral (singleConnection, multiConnection) | singleConnection, dualConnection | Sound › Output › Bluetooth Speakers | built |
| Bluetooth: Bluetooth Speaker Volume | luna com.webos.service.btaudiosrc getDeviceVolume |  | Sound › Output › Bluetooth Speakers | built — the volume keys move it; LG has no separate row on this set |
| Use Wireless Speaker: LG WOWCAST Device | sound.soundOutput wow_cast; luna com.webos.service.wowplay startDiscovery, p2pSearch, p2pDisconnect, removePairedDevice | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | - | cannot: LG WOWCAST dongle protocol (Wi-Fi Direct); the TV lists no paired device |
| Use Wireless Speaker: WiSA Speakers | sound.soundOutput wisa_speaker, sound.wisaSpeakerChannelType; luna com.webos.service.wisa searchSpeaker, setSpeakerType, deleteOneSpeaker, setLogVolumeTrim, soundTestOneSpeaker | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | - | cannot: needs the WiSA dongle the C4 does not have; LG hides it |
| Use Wireless Speaker: Mobile Device | sound.soundOutput mobile_phone; luna com.webos.service.soundshare getStreamOutStatus | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | - | cannot: LG ThinQ phone link; the TV answers no paired phone |
| Use External Speakers with TV Speaker | sound.soundOutput tv_speaker_external_arc, tv_external_speaker (optical), tv_speaker_headphone, tv_speaker_bluetooth, tv_speaker_wow_cast | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Sound › Output | covered — TV speakers too switch (ARC); optical and headphone twins to build |
| Bluetooth Device + TV Speaker: Playback Option, Audio Delay, Sound Balance, Speaker positions | sound.bluetoothMode, sound.avSyncBtSurround, luna com.webos.service.btaudiosrc/surround getVolumeBalancing, notifyEvent, com.webos.service.btaudiosrc setChannel | surroundMode, speakerMode | Bluetooth speakers | build |
| Bluetooth Surround Setup / Reset Settings | luna app com.webos.app.btsurroundautotuning; luna com.webos.service.btaudiosrc/surround notifyEvent |  | - | cannot: LG's surround auto-tuning app (microphone sweep) |
| Soundbar App | sound.blegattSoundbar; luna app com.webos.app.soundbar-setting | hidden now | - | cannot: LG soundbar's own app; the Denon is driven by the receiver page |

### Advanced Settings

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Dolby Atmos | sound.supportATMOS | on, off | Sound › Advanced | build |
| Balance | sound.audioBalance | -50 to 50 | Sound › Advanced | covered |
| Equalizer | sound.userEqualizer, sound.userEqualizerValues (per band) | on, off | Sound › Advanced | build — User sound mode only |
| Apply to All Inputs | sound.applyToAllInputSSM | sound, done | Sound › Advanced | build — asks first |
| Reset (the sound mode's settings) | luna settingsservice resetSystemSettings (category sound, current mode) |  | Sound › Advanced | build — asks first, danger style |
| Broadcasting on Auracast | sound.auracast; luna app com.webos.app.leaudio | on, off; hidden now | - | cannot: Auracast broadcast runs in LG's LE Audio app |
| Installation Type | sound.tvSetupConfiguration | stand, wall_mount | Sound › Advanced | build |
| Automatic Volume Adjustment | sound.autoVolume | on, off | Sound › Advanced | covered |
| DTV Audio Setting | sound.digitalAudioPriority | mpeg, dolbyDigital, dobyDigitalPlus, heAaC, auto | Sound › Advanced | build |
| Match Screen and Sound (AV Sync) | sound.avSync (on, off), sound.avSyncSpeaker, avSyncSpdif, avSyncWisa, avSyncBtSurround, sound.avSyncBypassInput | on, off | Sound › Output | covered — Bypass switch to build |
| Select HDMI Input Audio Format | sound.inputAudioFormatHDMI1..4 (bitstream, pcm), inputAudioFormatDP1, DP2, USBC1, USBC2 | bitstream, pcm | HDMI input audio (child of Sound › Devices) | build |
| TV Sound Mode Share | sound.soundAlive | on, off | Sound › Advanced | build |
| LG Sound Sync | sound.lgSoundSync | on, off | Sound › Advanced | build |
| Digital Sound Output | sound.soundOutputDigital | pcm, auto, passThrough | Sound › Output | covered |
| eARC Support | sound.eArcSupport | on, off; hidden now | Sound › Advanced | build |

### General › AI Service (sound)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| AI Sound Pro / AI Sound | sound.soundMode aiSoundPlus, aiSound | aiSoundPlus, aiSound, standard, movie, news, sports, music, game | Sound › Mode | covered |
| AI Acoustic Tuning | sound.magicSpaceSound; luna app com.webos.app.onetouchsoundtuning | off, standard, bass, treble | - | cannot: needs the Magic Remote's microphone sweep in LG's tuning app; the stored result's switch is to build as the AI acoustic tuning row |
| AI Acoustic Tuning Options | sound.magicSpaceSound | off, standard, bass, treble | Sound › Advanced | build |

### Other sound items LG places elsewhere

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| TV Power Sound Volume | general.tvOnSound | off, veryLow, low, medium, high, veryHigh | Sound › Advanced | build |
| Device Power Sound (Accessibility) | sound.tvPowerSound | on, off | System › Accessibility › Audio options | built |
| Clear Voice, Game TV Equalizer, Sliding Speaker, Bluetooth Speaker Mode | sound.clearVoice, sound.gameTVEqualizer, sound.slidingSpeaker, sound.btSpeakerMode | off, level1, level2, level3 | - | cannot: not in LG's menu on this model (no description visible or the hardware is absent) |

## General

### Accessibility

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Audio Guidance (on) | option.audioGuidance | on, off | System › Accessibility | covered |
| Audio Guidance: Speed, Volume, Pitch | option.audioGuidanceSpeed (verySlow..veryFast), option.audioGuidanceVolume (veryLow..veryHigh), option.audioGuidancePitch | veryFast, fast, medium, slow, verySlow | System › Accessibility › Audio Guidance | built |
| Audio Guidance: High-quality language pack | luna com.webos.service.tts installAdvancedLanguagePack, removeAdvancedLanguagePack, getAdvancedLanguagePackInstallStatus |  | - | cannot: a download of a voice pack through LG's server; no install progress can be shown offline |
| Audio Description | sound.audioDescription, sound.audioDescriptionVolume | on, off | System › Accessibility | built |
| Audio Description (ATSC 3.0) | sound.audioDescriptionAtsc30 | on, off | System › Accessibility | built |
| Video Description | sound.visualImpaired | on, off | System › Accessibility › Audio options | built |
| Audio Type, Additional Audio Selection | option.supplementaryAudio, option.additionalAudioSelection | off, audioDescription, spokenSubtitles, allType | System › Accessibility › Audio options | built |
| Amplify Dialogue (ATSC 3.0) | sound.dialogEnhancementAtsc30, sound.dialogEnhancementVolumeAtsc30 | on, off | System › Accessibility › Audio options | built |
| Learn Remote Control | luna app com.webos.app.remotecontrolguide |  | - | cannot: a spoken button tour in LG's own app |
| Extend Pressing Time on Remote: Activate time for long press, Interval time for repeated actions | option.remoteSlowInput, option.longPressInterval (basic, 1.5 .. 5), option.repeatInterval (basic, .5 .. 5) | on, off | Remote & pointer | build |
| Device Power Sound | sound.tvPowerSound | on, off | System › Accessibility › Audio options | built |
| Screen Zoom | option.screenMagnification, option.adjustments | on, off | System › Accessibility | built |
| Listen simultaneously with a hearing aid | sound.hearingAid (off, bluetooth, external_arc) and cochlear implant | off, bluetooth, external_arc | System › Accessibility › Audio options | built |
| Sign Language Zoom | other.autoSLZoomWithObjectDetection (manual zoom is a live overlay) | on, off | System › Accessibility › Audio options | built — the automatic switch only; the manual overlay is a player feature |
| Sign Language Guide: on, Speed, Size, Position, Type, Update | option.signLanguageGuidance, signLanguageGuidanceSpeed, signLanguageGuidanceSize, signLanguageGuidancePosition, signLanguageGuidanceAvatarType; luna com.webos.service.signlanguageavatar setSignLanguageGuidance, setAvatar, requestUpdate | on, off; hidden now | - | cannot: the avatar is a system overlay that LG draws; no setting is visible on this set |
| Closed Caption (on) | caption.captionEnable | on, off | System › Accessibility | covered |
| Closed Caption: Analog Mode | caption.captionAnalog | cc1, cc2, cc3, cc4, t1, t2, t3, t4 | System › Accessibility › Closed Caption | built |
| Closed Caption: Digital Mode (HD mode) | caption.captionDigital (off, svc1..svc6) | off, svc1, svc2, svc3, svc4, svc5, svc6 | System › Accessibility › Closed Caption | built |
| Closed Caption: Digital Mode Option (Style, Size, Font, Text Color, Text Opacity, BG Color, BG Opacity, Edge Type, Edge Color, Window Color, Window Opacity) | caption.captionStyle, captionSize, captionFont, captionTextColor, captionTextOpacity, captionBgColor, captionBgOpacity, captionEdgeType, captionEdgeColor, captionWindowColor, captionWindowOpacity | auto, manual | System › Accessibility › Closed Caption (Text, Background tabs) | built |
| Closed Caption: NEXTGEN TV Mode | caption.digitalPlusCaptionLanguage | eng, spa, fre | System › Accessibility › Closed Caption | built |
| Closed Caption: Position Settings | caption.captionDigitalPosOption, caption608PosOption, luna com.webos.service.utp.subtitle getCaptionPosition | auto, manual, autoAi, splitDown16_9, splitUp16_9 | System › Accessibility › Closed Caption | built — Set by Program, Automatic and the separated positions; Manual needs LG's live placement tool |
| AI Caption (beta) | caption.aiCaptionEnable; luna com.webos.service.nais getQrCodeGenerationUrl | on, off; hidden now | System › Accessibility › Closed Caption | built — the switch, which the TV hides on this set; the phone QR step is not shown |
| Hard of Hearing | caption.hardOfHearing | on, off | System › Accessibility › Closed Caption | built |
| Subtitles (on, first and second language) | caption.captionEnable, option.subtitleLanguageFirst, option.subtitleLanguageSecond | on, off | System › Accessibility › Closed Caption (Subtitles tab) | built |
| Closed Caption Settings (Japan, Philippines) | caption.japanCaptionEnable, japanSuperImposeEnable, phlCaptionEnable, phlSuperImposeEnable | off, jpn, eng | - | cannot: broadcast standards of other countries; LG hides them on this set |
| Text with High Contrast | option.highContrast | on, off | System › Accessibility | built |
| Focus with High Contrast | option.focusEmphasis | on, off | System › Accessibility | built |
| Grayscale, Invert Colors | option.adjustments, option.highContrast, option.screenMagnification |  | System › Accessibility | built — LG's two switches change the same group of keys together; LumaTV writes the same key sets |

### Always Ready (LG: General › Always Ready)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Always Ready (mode) | general.lifeOnScreenMode (off, lifeOnScreen, alwaysReady, allEnabled) | off, lifeOnScreen, alwaysReady, allEnabled | Home & look › Standby | build — LG's switch also asks "turn off the TV directly" |
| Always Ready: Energy saving, Notification, On-timer, Motion sensor | option.lifeOnScreenEnergySaving, lifeOnScreenNotification, lifeOnScreenOnTimer, lifeOnScreenUsingMotionSensor | auto, off | Home & look › Standby | build — the TV answers no description for some; shown where it does |
| Always Ready: Wallpaper | luna com.webos.service.iotclient getAccountInfoList; option.homeWallPaper | opt1, opt2, opt3 | - | cannot: LG's wallpaper catalogue is a cloud service; LumaTV has its own Wallpaper |

### AI Service

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| AI Recommendation: Who.Where.What? | option.broadcastInfoNoti | on, off; hidden now | Home & look › Home | build |
| AI Recommendation: Smart Tips | general.aiNudge | on, off | Home & look › Home | build |
| AI Recommendation: Smart Tips in Settings | general.aiSettingsNudge | on, off | Home & look › Home | build |
| Voice Recognition Settings, Help | luna app com.webos.app.aiplatformsupport |  | - | cannot: LG ThinQ AI voice set-up; the mic key is LumaTV's own voice search |

### Simple Mode

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Simple Mode (Normal, Simple), Picture Plus, Sound Plus, Remote Pointer, Daily Reminder Alert | option.seniorMode; luna com.webos.service.systemprofile enableProfile, disableProfile, com.webos.service.tvpower/power2 reboot; option.pqPlus, option.soundPlus, general.remotePointer, general.lifeAlarm | general, simple; hidden now | - | cannot: switching restarts the TV into LG's own simple Home and removes LumaTV; a person restarts it at the remote |

### Game Optimizer

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Game Optimizer (genre, black stabilizer, VRR / G-Sync / FreeSync, ALLM, ...) | luna app com.webos.app.gameoptimizer; other.gameMode, other.gameGenre, other.freesyncOLEDHDMI1..4, other.qmsVrr, other.gameOptimizationDP2, other.darkMode |  | Picture › Game | build — LG keeps these per HDMI input |

### Curvature, Lighting, Zero Connect, Shade, Battery (other LG models)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Curvature Setting (Button, Return, Transition Sound) | option.curvature, option.restoreCurve, general.bendableSound; luna com.webos.service.rollingscreen changeCurve |  | - | cannot: bendable OLED models only; this panel is flat |
| Lighting Setting, Lighting Settings | other.lightingEnable, lightingMode, lightingBrightness, lightingColorMode, lightingTnativeMode, lightingTnativeBrightness, lightingAutoBrightness, soundSyncMode*, aodPictureSyncMode | on, off | - | cannot: rear light of lifestyle models; LumaTV's own Ambient light replaces it |
| Zero Connect Box Settings | general.wavBoxPosition, general.wavUpperLedFlag; luna com.webos.service.wirelessavcontroller | none, front, side, bottom | - | cannot: wireless-box models only |
| Transparent Mode Switch Settings | general.tnativeShadescreen, general.tnativeSound | shadePoweroff, shadePoweroffon, shadeOff | - | cannot: transparent-OLED model only |
| Battery, Low Power Mode | other.lowPowerMode; luna com.webos.service.tvbattery | on, off | - | cannot: battery models only |
| Set ID | option.setId, enableSetIdMenu | 1 to 99 | - | cannot: signage and hotel installs only; LG hides it |
| Use Provided Remote Only | option.remoteSetting (no key on this set); luna com.webos.service.micomservice setSpecialRemoconMode | no description | - | cannot: hotel and signage installs only |

### OLED Care

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Care Picture Settings | other.oledCareMode (off, comfortCare, cinemaCare, kidsCare), picture.erpUserConfirm | off, comfortCare, cinemaCare, kidsCare | System › OLED care | build |
| OLED Panel Care: Pixel Cleaning | picture.scheduledPixelCleaning; luna com.webos.service.oledepl setPixelRefresherMode | on, off | System › OLED care | build — starts the run when the TV next goes off; asks first |
| OLED Panel Care: Screen Move | picture.screenShift | on, off | System › OLED care | covered |
| OLED Panel Care: Adjust Logo Brightness | picture.logoLuminanceAdjust (off, light, strong) | off, light, strong | System › OLED care | covered |
| OLED Panel Care: Care Recommendations | other.oledCareRecommendation | off, on | System › OLED care | build |
| Panel hours (TV Information › Total Power On Time) | luna com.webos.service.panelcontroller getPanelUsageTime |  | System › OLED care | covered |
| OLED Panel Settings | picture.oledPanelSettings | on, off | - | cannot: no screen of LG's uses it on this set |

### Usage Care (Family Care)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Usage Care | luna app com.webos.app.familycare |  | - | cannot: LG account family service; needs a signed-in LG account and LG's cloud |

### Channels

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Channel Tuning & Settings | luna app com.webos.app.channelsetting; luna com.webos.service.utp.channel/scan ... |  | Inputs › Channels | build — Auto Tuning and Manual Tuning as LumaTV rows; the scan runs in the TV's tuner service |
| Channel Manager | luna app com.webos.app.channeledit; luna com.webos.service.iepg getChannelList |  | Inputs › Channels | build — the list with hide and favourite; reorder stays to a person's remote |
| Dual Mono | channel.dualMono |  | Inputs › Channels | build |
| Multi Feed | channel.multiFeed |  | - | cannot: the TV answers no such key (broadcast-specific) |
| CI Information | luna com.webos.service.utp.channel/ci ... |  | - | cannot: needs a CAM card in the slot; none present |
| Save CAM Operator's Profile | channel.supportOperatorProfile | on, off; hidden now | - | cannot: CAM card only |
| Enter CI Password | lock.camPinCode | secret, not read | - | cannot: CAM card only |
| System Color (analog) | channel.analogSystemColor | multi, pal-m, pal-n, ntsc | Inputs › Channels | build |
| Channel Scrolling Options | channel.autoscanChannelGroup | primary_channel, all_channel | Inputs › Channels | build |
| Copy Channels (TV to USB, USB to TV) | luna com.webos.service.tvdataexchanger start |  | - | cannot: a USB transfer LG runs with its own progress and PIN prompt |
| HbbTV: Auto Start, Do Not Track, Distinctive ID, Reset ID, Delete Connection History, Multi Channel Audio, Third-Party Cookies | option.hbbTV, hbbTvDnt, hbbTvDeviceId, multiChannelAudio, thirdPartyCookie; luna com.webos.service.wampluggable.hbbtv resetDeviceId, resetDialConnection | on, off, onByUser, offByUser; hidden now | - | cannot: European broadcast standard; LG hides it in the USA |
| Interactive Service (Live TV, HDMI, ID reset) | option.interactive-service, interactive-service-hdmi, interactive-service-id; luna com.webos.service.cr resetAspectId | on, off, onByUser, offByUser | Inputs › Channels | build — LG's own ad-measurement switches |
| Signal Signing, Interactivity (DTVi), MHEG Guide, Data Service, Message, Hybridcast | channel.signingAtsc30, option.interactivity, option.mhegGuide, option.dataService, option.hybridCast | on, off | - | cannot: other countries' broadcast services; LG hides them |
| Remote Settings (channels on remote buttons) | luna app com.webos.app.channeledit target remoteSetting |  | - | cannot: LG's channel editor |
| Conditional Access module information | luna com.webos.service.nabs.cas |  | - | cannot: CAS card only |

### External Devices

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Connect Bluetooth Controller (keyboard, gamepad, mouse) | luna com.webos.service.bthidmanager requestDiscovery, subscribeDiscovery, requestConnect, getPairedDeviceLists, subscribeDevicesStatus, cancelDiscovery |  | Inputs › Devices › Bluetooth Devices | built |
| Bluetooth Controller Connection Status, Delete Device | luna com.webos.service.bthidmanager getPairedDeviceLists |  | Inputs › Devices › Bluetooth Devices | built |
| HDMI Settings: 4:4:4 Pass Through (per HDMI port) | other.444BypassHDMI1..4 (on, off), other.444BypassHDMINone | on, off; hidden now | Inputs › HDMI | built |
| HDMI Settings: Quick Media Switching | other.qmsVrr | off, on; hidden now | Inputs › HDMI | built |
| HDMI Settings: HDMI Deep Color | other.uhdDeepColor (off, 4k, 8k, auto; per gameInput) | off, 4k | Inputs › HDMI | built |
| HDMI Settings: SIMPLINK (HDMI-CEC) | other.simplinkEnable | on, off | Inputs › HDMI | covered |
| HDMI Settings: Dolby Vision PC | other.enableDolbyVisionPC | off, on | Inputs › HDMI | built |
| Input Compatibility Mode | other.inputCompatibilityMode | no description | - | cannot: the TV answers no such key |
| Auto Input Switch | other.autoInputSwitch | no description | - | cannot: the TV answers no such key (monitors only) |
| Universal Control Settings | luna com.webos.service.ics/blaster ...; option.irBlaster | 0 to 255 | - | cannot: the C4 has no IR blaster; LG hides it |
| TV On With Mobile: Turn on via Wi-Fi, via Bluetooth | network.wolwowlOnOff, network.bleAdvertisingOnOff | true, false | Network › Wi-Fi, Details | built — Wi-Fi switch exists in Network; Bluetooth one to build |
| Delete Connection History | luna com.webos.notification createToast (after clearing paired wireless devices) |  | Inputs › Devices | build — asks first |

### System › Language

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Menu Language | option.menuLanguage, localeInfo (locales.UI), luna com.palm.systemservice setPreferences |  | System › General | build — the TV restarts its menus on change; asks first |
| Audio Language (first, second) | sound.audioLanguageFirst, sound.audioLanguageSecond |  | System › General | build |
| Keyboard Languages | localeInfo.keyboards; luna com.palm.systemservice setPreferences |  | System › General | build |
| Teletext Languages (primary, secondary) | option.ttxLanguage1, option.ttxLanguage2 (the TV answers no such keys) | no description | - | cannot: teletext is not broadcast in the USA; LG hides it |

### System › Location

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Broadcast Country | option.country, time.timeZone; luna com.palm.systemservice setPreferences, com.webos.service.devicereset doReset | free text | - | cannot: changing it factory-resets the TV and deletes every channel; a person does that in LG's settings |
| LG Services Country | option.autoSmartServiceCountry; luna com.webos.service.sdx setCountrySettingByManual, setCountrySettingByAuto | on, off | System › General | build — asks first; the TV restarts |
| Zip Code | option.zipcode; luna com.webos.service.sdx send |  | System › General | build — digits on the on-screen keypad |
| Address, City, Weather Location | option.japanCitySelection, option.phlCitySelection; luna app com.webos.app.weatherlocation | free text; hidden now | - | cannot: Japan and Philippines only; the weather location is LumaTV's own widget setting |

### System › Time & Timers

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Set Automatically | time.autoClock (luna com.palm.systemservice setPreferences) | on, off | System › Time & power | built |
| Time, Date | luna com.palm.systemservice/time setSystemTime, getEffectiveBroadcastTime |  | System › Time & power › Time & Date | built — pickers; only while Set Automatically is off |
| Timezone, Custom Time Zone | time.timeZone, time.timeOffset, time.daylightSaving; luna com.palm.systemservice setPreferences | Offset | System › Time & power › Time & Date | built — the TV's own zone list for its country; Custom Time Zone (a UTC offset) is not offered |
| Sleep Timer | luna com.webos.service.tvReservationAgent/timer/sleep add, getRemainTime |  | System › Time & power | covered |
| Power On Timer: on, Time, Repeat, Input, Channel, Volume | time.onTimerEnable, onTimerHour, onTimerMinute, onTimerWeekday, onTimerSource, onTimerChannel, onTimerVolume | off, on | System › Time & power › Power On Timer | built — Live TV as the input needs a channel pick: the HDMI inputs only |
| Power Off Timer: on, Time, Repeat | time.offTimerEnable (offTimerFlag), offTimerHour, offTimerMinute, offTimerWeekday | off, on | System › Time & power › Power Off Timer | built |
| Auto Power Off after 4 Hours | time.autoOff2HourOnTimer | on, off | System › Time & power | built |

### Energy Saving

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Energy Saving Step | picture.energySaving (auto, off, min, med, max) | auto, off, min, med, max | Picture › Mode | covered |
| Minimum Brightness (Energy Saving) | picture.energySavingAutoMin | 0 to 10 | Picture › Mode | build |
| Screen Off (turn off the picture, keep the sound) | option.screenOff |  | System › Time & power | built |
| No Signal Auto Off (15 minutes) | time.autoOff15Min, time.noSignalAutoOff | on, off | System › Time & power | covered |
| No Operation Auto Off | time.noOperationAutoOff (off, 3, 6, 12, 24) | off, 3, 6, 12, 24; hidden now | System › Time & power | covered — the TV hides it on this set |
| Auto Power Off | time.automaticStandBy (0, 2, 4, 6, 8) | 0, 2, 4, 6, 8; hidden now | System › Time & power | built — the TV hides it on this set, so the row is left out |
| DPM Auto Off (No Signal Standby) | time.dpmsAutoOff | on, off; hidden now | - | cannot: display-power-management for PC inputs; LG hides it on this set |

### System › Additional Settings

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Quick Start+ | option.quickStartMode | on, off | System › Time & power | covered |
| Home Settings: Power On Screen | general.homeAutoLaunch, general.selectedInputApp, general.checkHomeAutoLaunchValue | on, off | Home & look › Home | build |
| Home Settings: Home Promotion, Content Recommendation | general.homePromotion, other.contentRecommendation | on, off | Home & look › Home | build |
| Home Settings: Wallpaper | option.homeWallPaper | opt1, opt2, opt3 | - | cannot: LumaTV Home has its own wallpaper (Home & look › Look) |
| Home Animation | general.homeEffect | on, off; hidden now | - | cannot: LG Home's motion; LumaTV Home has its own motion settings |
| Standby Light | option.standByLight | on, off | - | cannot: the C4 has no standby light; LG hides it |
| Screen Saver Settings: Screen Saver Promotion, Standby Time | general.screenSaverAd, option.screenSaverTimer (3, 10, 20, 30 min) | on, off | Home & look › Screensaver | build |
| Pointer Options: Tracking Speed, Pointer Size | option.pointerSpeed, option.pointerSize | slow, normal, fast | Remote & pointer | covered |
| Live Plus, Live Promotion | option.livePlus, option.livePromotion | on, off | Home & look › Home | build |
| Settings Help | option.helpOnSettings | on, off | - | cannot: LG's help box; LumaTV's own help sheet replaces it |
| Touch Lock | general.touchLock | on, off | - | cannot: panel buttons only; the C4 has none |
| Emergency Information | option.emergencyInformationAtsc30, emergencyInformationLanguageAtsc30 | on, off | System › Accessibility | build |
| Screen Off Time (standby screen) | option.screenOffTime | 5, 10, 30, 60, 120 | - | cannot: Always Ready hardware sensor models; LG hides it on this set |
| Motion Sensor Sensitivity | option.motionSensorSensitivity | low, medium, high | - | cannot: motion-sensor models only |
| View Transition Sound | general.tvOnSound | off, veryLow, low, medium, high, veryHigh | Sound › Advanced | build |
| Artistic Display Timer | option.artisticDisplayTimer | off, 10, 30, 60, 120; hidden now | - | cannot: lifestyle models; LG hides it |
| Adaptive Power Delivery | general.adaptivePowerDelivery (no key on this set) | no description | - | cannot: USB-C power models only |

### System › Signature TV Settings

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Always-on, Always-on Off Time | general.alwaysOn, alwaysOnDisableStartHour, alwaysOnDisableStartMinute, alwaysOnDisableEndHour, alwaysOnDisableEndMinute | on, off | - | cannot: SIGNATURE models only; the TV answers no such key |
| SIGNATURE Opening, Welcome Feature, Bluetooth Speaker Mode | general.tvOnScreen, welcomeFeature, blueToothSpeakerMode | effect, liveTV | - | cannot: SIGNATURE models only |

### System › Safety

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Lock control (on or off, needs the PIN) | lock.parentalControl, lock.systemPin; luna com.webos.notification createPincodePrompt | True, False | System › Safety | build — the PIN is typed at the remote; never read or written by a helper |
| TV Program Locks (TV Lock, Movie Lock, ratings) | lock.tvRatingGeneralAge, tvRatingGeneralDialog, tvRatingGeneralLanguage, tvRatingGeneralSex, tvRatingGeneralViolence, tvRatingChildrenAge, tvRatingChildrenFantasy, movieRating, tvRatingEnglish, tvRatingFrench | off, g, pg, 14, ma | System › Safety | build |
| Downloadable Rating | lock.downloadableRrt | free text; hidden now | - | cannot: a rating table broadcast by the station; empty on this set |
| Channel Locks | luna app com.webos.app.channeledit target channelLock |  | Inputs › Channels | build — with the channel list |
| Application Locks | lock.applockPerApp | True, False | System › Safety | build |
| Input Locks | lock.inputBlock | 0 to 4294967295 | System › Safety | build |
| Reset Password | lock.systemPin, lock.initialPinCode | secret, not read | System › Safety | build — asks for the old PIN at the remote |
| Rating locks (Brazil, Saudi, Singapore, Japan, Ziggo, VOO) | lock.ratingBR, ratingSA, parentalGuidanceSg, parentalGuidanceJp, ziggoRaiting, vooRating | off, 10+, 12+, 14+, 16+, 18+ | - | cannot: other countries' systems; LG hides them |

## Support

### Software Update

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Software Version | luna com.palm.systemservice/osInfo query (core_os_release), com.webos.service.update getVersionChecked |  | System › Support | build |
| Check for Updates | luna com.webos.service.update checkUpdate, getStatus, getContent, getProgress, startUpdate, com.webos.service.pkgUpdate statusAll |  | System › Support | build — a person confirms the install at the remote; never started by an assistant |
| Auto Update | support.automaticUpdate | on, off | System › Support | build — the TV's terms prompt (firstuse-overlay eula) comes first when turning it on |
| Revert to the previous software | luna com.webos.service.update revertSWImage |  | - | cannot: expert-mode downgrade LG gates behind its own key sequence; never offered |
| LED Driver information | luna com.webos.service.panelcontroller getCurrentLedFirmwareInfo, updateExternalChipImage |  | - | cannot: factory panel firmware; LG hides it from owners |

### User Guide, Help, Diagnosis

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| User Guide | luna app com.webos.app.tvuserguide |  | - | cannot: LG's web guide in its own browser app |
| Quick Help | luna app com.webos.app.self-diagnosis |  | - | cannot: LG's guided troubleshooter |
| Screen Self-diagnosis | luna app com.webos.app.customersupport target pictureTest |  | System › Support | build — LumaTV shows the same test patterns itself |
| Sound Self-diagnosis | luna app com.webos.app.customersupport target soundTest |  | System › Support | build — plays the TV's own test tone; the TV speaker must be the output |
| Memory Optimizer | luna com.webos.service.recent removeAllRecents, com.webos.notification createToast |  | System › Support | build — clears LG's recents and closes background apps; asks first |
| Home/Store Mode | option.storeMode (home, store) | home, store | - | cannot: a retail demo mode that locks the TV into a loop; never offered |
| Store Mode Settings | option.storeMode | home, store | - | cannot: retail demo mode |
| IP Control Settings | luna app com.webos.app.installation; ipcontrol powerOnDefaultMinor |  | - | cannot: LG's installer sequence for control systems; the helper's Home Assistant link is the supported path |
| TV Information: Device Name | network.deviceName |  | System › Support | covered — shown; renaming needs the on-screen keyboard (build) |
| TV Information: Model, Model Code, Serial Number | luna com.palm.systemservice/deviceInfo query (product_id, serial_number) |  | System › Support | build — read only |
| TV Information: webOS TV Version | luna com.palm.systemservice/osInfo query (core_os_release, core_os_release_codename) |  | System › Support | build — read only |
| TV Information: Total Power On Time | luna com.webos.service.panelcontroller getPanelUsageTime |  | System › OLED care | covered |
| TV Information: Available Internal Storage | luna com.webos.service.attachedstoragemanager getProperties (INTERNAL_STORAGE) |  | System › Support | build — read only |
| TV Information: MAC Address (Wired, Wireless) | luna com.webos.service.connectionmanager getinfo |  | System › Support | build — read only |
| TV Information: ESN, VSN, Widevine, Dolby Vision, Patent Information | luna com.webos.service.sm/crypto checkSeStore, com.webos.service.pqcontroller getInfo (dolbySWVersion), sm/deviceid |  | System › Support | build — read only; keys and IDs are shown as LG shows them |

### Privacy & Terms

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| User Agreements | luna app com.webos.app.firstuse-overlay target eula |  | - | cannot: legal text LG shows in its own sign-off flow; accepting is a person's act |
| Legal Notice, Open Source Licenses, Content Display Policy | LG's web pages and an in-app licence list |  | - | cannot: long legal documents opened in LG's reader |
| Do Not Sell My Personal Information | general.doNotSellMyPersonalInformation; luna com.webos.service.admanager updateDNS | on, off | System › Support | build — asks first |
| Advertisement: AD ID, Reset AD ID, Limit AD Tracking | luna com.webos.service.admanager resetIFA, com.webos.service.sm/deviceid getIDs, general.lmt, general.adCookie | on, off | System › Support | build |
| App Permissions: Camera, Microphone, Nearby Devices, File Access | luna com.webos.service.ppm queryPermissions |  | System › Support | build — read, and switch per app |
| Delete My Personal Information (viewing, voice, voice ID) | luna com.webos.service.sdx send; com.webos.service.accountmanager getLoginID |  | System › Support | build — asks first, danger style |
| CMP Settings (consent) | general.nativeConsentAd; luna com.webos.service.admanager launchCMPApp | hidden now | - | cannot: LG's consent screens in the cmp-client app |
| User Report | option.faultLogUpload | True, False; hidden now | System › Support | build — the consent text shows first |
| LG Support Center | LG web link |  | - | cannot: opens LG's support site in a browser |

### Reset

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Reset to Initial Settings: Reset All, Reset all except network settings | luna com.webos.service.devicereset doReset; luna com.webos.notification createPincodePrompt |  | System › Support | build — asks first twice, danger style, needs the PIN at the remote; never reachable by AI access |

### Hidden or retail-only

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Commercial and hotel mode (80 keys) | commercial.*, hotelMode.* |  | - | cannot: hotel and signage installs; the TV hides them |

## Other LG menus

### Network (LG opens its Network sub-app from General › Network: com.webos.app.firstuse-overlay target network)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Wi-Fi Connection: list of networks, Join (password, hidden network, WPS) | luna com.webos.service.wifi findnetworks, connect, getprofile, getprofilelist, deleteprofile |  | Network › Wi-Fi › Join Network | built — the list, a password on the keyboard, a hidden network; WPS and the saved-network list are not offered. Method names are webOS's, not run on the set |
| Wired Connection: connect, status | luna com.webos.service.connectionmanager getStatus, getinfo |  | Network › Wired | covered — read only; the cable has nothing to set |
| IP settings: Automatic or Manual (IP address, Subnet Mask, Gateway, DNS servers) | luna com.webos.service.connectionmanager setipv4, setdns |  | Network › Wi-Fi, Wired › IP Settings | built — method names are webOS's, not run on the set |
| Proxy | luna com.webos.service.connectionmanager |  | - | cannot: LG's menu has no proxy screen on this webOS; nothing to mirror |
| IPv6 | network.ipv6Enable | true, false | Network › Details | built |
| TV Name (Device Name) | network.deviceName |  | Network › Details | built — edited with the on-screen keyboard |
| Wake from the network (TV On With Mobile via Wi-Fi) | network.wolwowlOnOff | true, false | Network › Wi-Fi | covered |
| Bluetooth advertising (TV On With Mobile via Bluetooth) | network.bleAdvertisingOnOff | on, off | Network › Details | built |
| Allow mobile device access | network.allowMobileDeviceAccess | True, False | Network › Details | build |
| Screen Share, AirPlay and HomeKit, Device Connection Settings | luna app com.webos.app.connectionwizard; AirPlay and Miracast services |  | - | cannot: LG's pairing and phone-mirroring set-up, which needs the phone's side |
| LG Connect Apps, TV On with Mobile pairing | luna com.webos.service.iotclient |  | - | cannot: LG ThinQ cloud pairing |

### LG Account and family

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| LG Account: sign in, sign out, manage | luna com.webos.service.accountmanager getLoginID, getLoginUserData |  | - | cannot: an account sign-in is typed and confirmed by a person in LG's own flow; never exposed to AI access |
| Family Settings | luna app com.webos.app.familycare |  | - | cannot: LG account family service |

### Quick Settings (LG's own quick menu, QuickSettings QML on the TV)

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Picture Mode | picture.pictureMode | personalized, vivid, normal, eco, cinema, sports, game, filmMaker, expert1, expert2 | Quick panel (ShellHost) and Picture › Mode | covered |
| Sound Mode | sound.soundMode | aiSoundPlus, aiSound, standard, movie, news, sports, music, game | Quick panel and Sound › Mode | covered |
| Sound Out | sound.soundOutput | tv_speaker, tv_speakerbar, external_optical, external_arc, bt_soundbar, wisa_speaker, mobile_phone, lineout, builtin_soundbar, headphone, bt_audio, tv_external_speaker, tv_speaker_headphone, tv_speaker_bluetooth (+6) | Quick panel and Sound › Output | covered |
| Adjust Contrast, Adjust Sharpness, Color Depth, Tint, Black Level (Video Range), Color Temperature | picture.contrast, sharpness, color, tint, blackLevel, colorTemperature | 0 to 100 | Picture tabs | build — the quick panel stays short; the same rows are one step away |
| Brightness (OLED Pixel Brightness) and Battery Brightness | picture.backlight | 0 to 100 | Picture › Mode | covered |
| Energy Saving | picture.energySaving | auto, off, min, med, max | Picture › Mode | covered |
| Eye Care (Reduce Blue Light) | picture.eyeComfortMode | on, off | Picture › Brightness | covered |
| Care Picture Settings, OLED Panel Care | other.oledCareMode | off, comfortCare, cinemaCare, kidsCare | System › OLED care | build |
| Screen Off | option.screenOff |  | System › Time & power | built |
| Timer (Sleep Timer) | tvReservationAgent timer/sleep |  | Quick panel and System › Time & power | covered |
| Limit Setting, Volume Limit | hotelMode.maximumVolume, hotelMode.minimumVolume |  | - | cannot: hotel mode only; LG hides it |
| Game Optimizer | com.webos.app.gameoptimizer |  | Picture › Game | build |
| Multi View | luna com.webos.service.multiviewcontroller |  | - | cannot: LG's side-by-side player (not on this set's inputs) |
| Music Search (hum to search) | luna lg.thinqai.adapter |  | - | cannot: LG ThinQ cloud service |
| Network | connectionmanager |  | Quick panel (Wi-Fi tile) and Network | covered |
| Privacy & Terms, External Devices | the Settings rows above |  | System › Support, Inputs › Devices | build |
| Touch Lock | general.touchLock | on, off | - | cannot: panel buttons only; the C4 has none |
| Picture Plus, Sound Plus, Daily Reminder, Remote Pointer (Simple Mode tiles) | option.pqPlus, option.soundPlus, general.lifeAlarm, general.remotePointer | True, False; hidden now | Remote & pointer (pointer) | covered — the others only exist in Simple Mode, which is not offered |

### LG apps Settings opens

| LG item | Setting or luna method | TV answers | LumaTV | Status |
|---|---|---|---|---|
| Picture Wizard, Sound Wizard, One Touch Sound Tuning, Bluetooth Surround Tuning | com.webos.app.picturewizard, soundwizard, onetouchsoundtuning, btsurroundautotuning |  | - | cannot: listening and viewing tests in LG's own apps |
| Channel Edit, Channel Setting | com.webos.app.channeledit, com.webos.app.channelsetting |  | Inputs › Channels | build |
| Software Update app, Self-diagnosis, Customer Support, User Guide, Remote Control Guide | com.webos.app.softwareupdate, self-diagnosis, customersupport, tvuserguide, remotecontrolguide |  | System › Support | build — the parts that are settings; the guides stay LG's |
| Accessibility app, Accessibility overlay | com.webos.app.accessibility, com.webos.app.accessibility-overlay |  | System › Accessibility | build |
| Gallery of LG apps (Discovery, Browser, Weather Location, Installation, First Use) | com.webos.app.discovery, browser, weatherlocation, installation, firstuse-overlay |  | - | cannot: store, browser and installer flows |
