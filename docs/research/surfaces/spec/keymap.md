## Keys and what they open

The compositor's key chain (`KeyFilters/*.js`, 22 handlers from configd
`keyFilters`) decides which surface a key opens. LumaTV's `handleLumaHome` is
first in the chain. Power has no handler in the QML chain.

| Key | LG today | LumaTV |
|---|---|---|
| Home, release | `launchDefaultApp(home)`; from Home it closes the Home view | slot 1 launches LumaHome from any surface; replayed Home keys re-enter the chain, so the handler must be idempotent |
| Home, held 2.5 s | Screen Zoom (accessibility) | PLAN's 650 ms hold reaches LG's Home first; zoom moves to a Settings row |
| Back, release | closes the top overlay, alert, keyboard or spinner, else goes to the app; an app that ignores Back gets "Do you want to exit the app?" | untouched; claimed only while LumaHome is fullscreen and focused |
| Back, held 1 s (Exit) | closes the foreground app | untouched |
| Gear (Settings), short | `quicksettings` overlay, already LumaTV's by redirect | no key code needed |
| Gear, held 1 s | `com.palm.app.settings` (toggle) | launch redirect to LumaTV Settings |
| Gear, three times in 3 s | hidden AV-reset menu | Off: swallow |
| Input, short | `quickinputpicker` overlay | redirect through configd; no key code needed |
| Input, held 1 s | Home Hub (`homeconnect`) from `keyaction.inputHub` | repoint the config |
| 123 (Screen Remote), short | "More Actions" panel; held 1 s: audio description | redirect |
| Digits, held 1 s | Quick Access slot N; 0 opens the editor | LumaHome dock numbers while it is in front; digits untouched elsewhere |
| Mic | `voiceconductor` hold, then the Search app and the listening bar | untouched |
| Netflix, Prime Video | launch the app | untouched |
| Disney+, Sling | not installed: LG's auto-install path | remap or Off (Needs Nitin) |
| LG Channels, Movies, New | LG Channels app; Search with "movie"; Store category | remap or Off (Needs Nitin) |
| Guide, Channel list, Live TV | Guide overlay; channel list; Live TV | untouched (Live TV is rare) |
| Volume, Mute | `audioService`; the OSD is the compositor's | untouched; Mute held 2 s opens the accessibility panel |
| Colour keys | to the app; Yellow held 5 s shows a panel test pattern, Green seven times FreeSync info (HDMI), Blue seven times a panel-usage dump | Off for the held and repeated combinations |
| Menu, keyboard Windows | Home with `myapps` | route to the LumaHome apps grid |
| TV joystick press | Local Control Panel (`tvhotkey`) | untouched: the recovery path without a remote |
| Keyboard F2 F4 F5 F7, gamepad Menu and Settings | Input, Home, Settings, LG Channels; Menu and Settings buttons | untouched |
