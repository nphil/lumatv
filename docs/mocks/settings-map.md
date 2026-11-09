# LG settings → LumaTV sections

Source: LG's Settings app for this C4 (`com.palm.app.settings`, copied to
`/data/home/lgtv/research/lgsettings/`). The item names below are the English
source strings in `main.js` (`Up("…")` calls and the `rV` panel map);
`res/en/GB/strings.json` only holds the British overrides (Colour, Programme).
Network, Family Settings and the LG account live in separate sub-apps and are
not in this bundle; they are marked *(sub-app)*.

LG's top level is Picture · Sound · General · Support, with roughly 130 items
below. LumaTV shows nine sections. Each section keeps the few items a person
changes from the sofa, worded plainly, and ends with a "More in LG settings"
row that opens LG's own screen (themed) for the rest.

## Picture

LumaTV rows: picture mode chooser (Mode tab), OLED pixel brightness,
Filmmaker ambient light, AI Picture Pro, aspect ratio, energy saving. Tabs:
Mode · Brightness · Colour · Advanced.

| LG item (path under Picture) | LumaTV |
|---|---|
| Select Mode: Vivid, Standard, Eco, Cinema, Sports, Game Optimizer, FILMMAKER MODE, Expert (Bright space, daytime), Expert (Dark space, night), Personalized Picture; HDR variants Cinema Home, HDR Effect | Mode tab, the chooser. LumaTV names: Filmmaker, Cinema, Standard, Vivid, Game Optimizer, Sports, Eco, Expert · day, Expert · night, Personalised |
| FILMMAKER MODE Ambient Light | Mode tab, switch "Filmmaker ambient light" |
| Aspect Ratio: 16:9, Original, 4:3, Vertical Zoom, 4-Way Zoom, Just Scan | Mode tab, value row "Aspect ratio" |
| Advanced Settings › Brightness: OLED Pixel Brightness, Adjust Contrast, Screen Brightness, Auto Dynamic Contrast, Dynamic Tone Mapping, Peak Brightness, Gamma(Adjust Brightness), Black Level, Video Range, Motion Eye Care, Expression Enhancer, Near Black Detail | Brightness tab (OLED pixel brightness also on the Mode tab) |
| Advanced Settings › Color: Color Depth, Tint, Color Gamut, Fine Tune (Color Adjustment: Select Color, Adjust Saturation, Adjust Tint, Adjust Luminance), White Balance (Color Temperature, Method, Point, Signal Level, Code Value) | Colour tab: colour depth, tint, colour temperature. White balance and Fine Tune → More in LG settings |
| Advanced Settings › Clarity: Adjust Sharpness, Super Resolution, Noise Reduction, MPEG Noise Reduction, Smooth Gradation, Real Cinema, TruMotion (De-Judder, De-Blur, OLED Motion, OLED Motion Pro), Precision Detail | Advanced tab |
| Advanced Settings: Apply to All Inputs, Reset | Advanced tab |
| General › AI Service: AI Picture Pro, AI Brightness Settings, AI Genre Selection | Mode tab "AI Picture Pro"; the other two on the Advanced tab |
| General › Energy Saving: Energy Saving Step (Auto, Off, Minimum, Medium, Maximum), Minimum Brightness, Screen Off | Mode tab value row "Energy saving" |
| 4:4:4 Pass Through, HDMI Deep Color, Dolby Vision PC (General › External Devices › HDMI Settings) | More in LG settings (also reachable from Inputs & devices) |

## Sound

LumaTV rows: sound out, receiver volume (read from the Denon), sound mode
(disabled while a receiver plays, with the reason), match screen and sound,
HDMI input audio format, TV speakers too. Tabs: Output · Mode · Advanced.

| LG item (path under Sound) | LumaTV |
|---|---|
| Sound Out: Use TV Speaker; Use Wired Speaker (HDMI(ARC) Device, Optical Out Device, Wired Headphones, Audio Out Device, USB Device); Use Wireless Speaker (Bluetooth Device, WiSA Speakers, LG WOWCAST); Use External Speakers with TV Speaker | "Sound out" value row (Denon AVR-S760H, eARC on HDMI 2); "TV speakers too" switch |
| Select Mode: AI Sound Pro, Standard, Cinema, Clear Voice Pro, Sports, Music, Game Optimizer | Mode tab; shown disabled while sound goes to the receiver |
| Advanced Settings: Balance, Equalizer, Installation Type (Stand, Wall-mounted), Automatic Volume Adjustment, TV Power Sound Volume, DTV Audio Setting, Select HDMI Input Audio Format, Digital Sound Output, eARC Support, LG Sound Sync, TV Sound Mode Share, Dolby Atmos, Match Screen and Sound | "Match screen and sound" slider, "HDMI input audio format" value row; eARC shown as state on the Sound out row. Rest → More in LG settings |
| Volume shown on screen | "Receiver volume": LumaTV's own, fed by the Denon (not an LG item) |
| General › AI Service: AI Sound Pro, AI Acoustic Tuning | Advanced tab |

## Inputs & devices

| LG item | LumaTV |
|---|---|
| Inputs (Home Dashboard / input list; names from SIMPLINK) | Input picker tiles; this section lists the same inputs with rename and hide |
| General › External Devices › HDMI Settings: SIMPLINK (HDMI-CEC), Auto Input Switch, Quick Media Switching, HDMI Deep Color, 4:4:4 Pass Through, Dolby Vision PC | HDMI-CEC switch, auto input switch, deep colour per input |
| General › External Devices: Connect Bluetooth Controller, Universal Control Settings, TV On With Mobile (Turn on via Wi-Fi), Delete Connection History; Support › Privacy & Terms › Camera, Microphone, Nearby Devices permissions | Bluetooth devices row; the rest → More in LG settings |
| General › Game Optimizer (own app) | Row that opens LG's Game Optimizer |

## Network

| LG item | LumaTV |
|---|---|
| Wi-Fi Connection, Wired Connection (Ethernet), network status *(sub-app)* | Network name and strength as the live line ("Nate's Home · Strong"); forget, choose network |
| Device Connection Settings, Screen Share, AirPlay and HomeKit *(sub-apps)* | More in LG settings |
| Support › IP Control Settings | More in LG settings |

## Ambient light

LumaTV's own feature (lights around the TV following the picture); LG has
no equivalent on the C4. LG's Lighting (Lighting Mode, Lighting Brightness,
Lighting Color) exists in the bundle for other models and is not shown.

## Home & look

| LG item | LumaTV |
|---|---|
| General › System › Home Settings: Home Promotion, Content Recommendation, Home Animation, Recent Input, Power On Screen | Replaced by LumaTV Home: wallpaper, dynamic tint, clock style, widgets, dock order, wallpaper slideshow |
| General › System › Additional Settings › Screen Saver Settings, Screen Saver Standby Time, Screen Saver Promotion | Screensaver rows; Standby Time is LumaTV's Screensaver after (1 min to 3 h, or Off) |
| General › Always Ready, Always-on Off Time | Standby look row |
| Standby Light, "Show LG logo when turning on" (`other.lgLogoDisplay`) | Boot look rows |

## Remote & pointer

| LG item | LumaTV |
|---|---|
| General › System › Additional Settings › Pointer Options: Pointer Speed (Slow, Normal, Fast), Pointer Size (Small, Medium, Large) | Segmented rows "Pointer speed", "Pointer size" |
| `general.remotePointer` (settingsservice; "Remote Pointer" string in Accessibility › Simple Mode) | Switch "Magic Remote pointer" |
| Hold Home opens LG's Home; number keys open dock tiles | LumaTV's own rows |
| Gear key held: LG's Settings app; gear tapped three times: LG's hidden support menu (`tvhotkey`, `mute-hidden-action`) | Neither opens. Segmented row "Hold gear" (Settings, the default, Notifications, Nothing; theme `remote.gearHold`); every tap is the quick panel. The menu's items: System › About this TV (model `factorymanager getFactoryOpt`, platform, software version), HDMI "Auto power sync" (`other.simplinkAutoPowerOn`) and "No signal image" (`general.noSignalScreenSaver`), Home & look › Standby "LG logo" (`other.lgLogoDisplay`), System › Support › Reset "AV reset", Channels "Auto Tuning" (`com.webos.app.channelsetting`), Privacy › Consent "Allow LG Remote Service" (off, owner-only) |
| Universal Control Settings, Learn Remote Control, Extend Pressing Time on Remote (Activate time for long press, Interval time for repeated actions), Settings Help | More in LG settings |

## AI access

Entirely LumaTV's: allow switch (Off / LAN only), paired assistants,
per-assistant capability levels, activity, lock out. No LG item maps here;
LG's "AI Service" items are picture and sound features and stay in those
sections.

## System

| LG item | LumaTV |
|---|---|
| General › System › Language (Menu Language, Keyboard Languages, Audio Language), Location (Broadcast Country, LG Services Country, Zip Code, Weather Location Setting), Time & Timer (Set Automatically, Time, Date, Timezone, Timers: Sleep Timer, Power On Timer, Power Off Timer, No Operation Auto Off) | Language, time zone, sleep timer rows |
| General › Energy Saving › Auto Power Off, No Signal Auto Off, Quick Start+ | Power rows |
| General › OLED Care: OLED Panel Care (Pixel Cleaning, Screen Move, Adjust Logo Brightness), Care Picture Settings, Care Recommendations | OLED care rows with panel hours (luna `panelcontroller/getPanelUsageTime`), then LumaTV's group "LumaTV panel care" (five switches, theme `oledcare`; `docs/research/oledcare-apps-result.md`) |
| General › Accessibility: Audio Guidance, Audio Description, Closed Caption, Text with High Contrast, Grayscale, Invert Colors, Sign Language Zoom, Screen Zoom, Focus with High Contrast, Hard of Hearing | Accessibility rows (switches), the deeper ones → More in LG settings |
| Support › Software Update (Auto Update, Check for Updates), TV Information (Software Version, Serial Number, Model Code, webOS TV Version, Total Power On Time, MAC addresses), Screen Self-diagnosis, Sound Self-diagnosis, Memory Optimizer, Quick Help, User Guide, LG Support Center | "About this TV" row; the rest → More in LG settings |
| Support › Privacy & Terms (User Agreements, Legal Notice, Open Source Licenses, Advertisement / Limit AD Tracking, Delete My Personal Information, App Permissions), Home/Store Mode, Live Plus, Set ID, Reset to Initial Settings | More in LG settings only |
| General › Channels (Channel Tuning & Settings, Channel Manager, Multi Channel Audio, HbbTV, Interactive Service, Copy Channels), Family Settings *(sub-app)*, LG Account *(sub-app)*, Simple Mode | More in LG settings only |
| LumaTV itself: version, on/off, LG Home escape, logs | LumaTV rows |

## Live lines on the rail

One short fact each, read from the owner's current value, and none where there is nothing to glance at (Nitin, 2026-10-25: minimal text):
Picture "Filmmaker" · Sound "Receiver" · Inputs & devices "HDMI 2" · Network "Nate's Home · Strong" (or "Wired") · Ambient light "On" or
"Off" · Remote & pointer "Pointer on" · AI access "On" or "Off" · System the sleep timer while one runs ("Off in 23 min"). Home & look and the
last row have no second line. A row's explanation is not here: it sits behind the row's info mark (docs/research/settings-ux-result.md).
