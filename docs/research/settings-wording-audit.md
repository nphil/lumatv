# Settings wording audit

Every row of every LumaTV Settings page (the Rust kind in `rustui/ui/src/settings/`, the QML pages in `payload/compositor/lumatv/settings/`, the
quick panel) was read for one question: from the words alone, what does ON mean? The rules that came out of it are in
`rust-settings-result.md` ("Wording rules"). The helper's Privacy menu supplies the titles of the blockers and consents, so those are
changed in `helper/privacy/src/` and show on every toolkit.

The QML Settings had no Privacy page of its own: its privacy rows are the Support > Privacy tab, covered below. The Privacy page with the
Blockers is the Rust one.

## Changed rows

ON means what the third column says; the setting underneath is untouched in every row (the privacy rows have a test that ON in the row sends
`enabled: true` and that the row shows the helper's own `on`).

| Page | Old label | New label | What ON means | Why |
| --- | --- | --- | --- | --- |
| Privacy > Blocking | Content recognition & advertising services | Block content recognition (ACR) and ads | LG's content recognition, ad overlays, sports promotions and banners are stopped | a bare service name: on could mean the service runs |
| Privacy > Blocking | Viewing information, ads & promotions | Keep LG's viewing and ad settings off | LumaTV holds LG's 16 viewing-data and ad settings off | said nothing about who holds what |
| Privacy > Blocking | Telemetry & diagnostic uploaders | Block telemetry uploads | LG's log, crash and usage uploads are stopped | bare service name; LG jargon |
| Privacy > Blocking | LG push promotions | Block LG push promotions | push notices and nudges are stopped | bare service name |
| Privacy > Blocking | Block ad & tracker hosts | unchanged | ad and tracker names are blocked for every app | already a protection verb |
| Privacy > Blocking | Prime Video ads (Amazoff) | Prime Video ads, a status: Allowed / Blocked / Ready / Partly blocked / Not running / Not available, with a chevron | status, not a switch | it restarts Prime Video and switches the old AmazOff app off, so it is one plain question, not a switch (see Questions) |
| Privacy > Blocking | Held settings, "16 of 16" | LG privacy settings kept off, "16 of 16 kept off" | a link | the number had no unit; the row had no subject |
| LG privacy settings kept off (was Held settings) | a list of LG's own key names ("Ad cookie", "Live plus content recognition") with On / Off | a note ("LumaTV switches these back off if LG turns them on."), three groups (Viewing data, Ads, Tips and suggestions), each setting named for what LG would do ("Collect my viewing history", "Personalised ads") and a state in words: Kept off / On now (LG turned it on; it is switched back) / Off / On (when nothing is held) / Not on this TV | a status list | the screen explained nothing and mixed the key names of LG's settings service with words |
| Privacy tab names | Blockers / Consent / Ads & data | Blocking / LG may collect / Ads & data | | a header says what the group does; "Consent" names an act, not a topic |
| Privacy (rail) | "3 of 6 on" | "3 of 6 blocking" | | on of what |
| Privacy > LG may collect | Screen content recognition | Allow content recognition (ACR) | LG may recognise what the TV shows | consent rows say what is allowed |
| Privacy > LG may collect | Screen recognition (master consent) / (GDPR consent) | Allow content recognition (main consent) / (EU consent) | the same, as LG's other agreement | |
| Privacy > LG may collect | Ads based on viewing | Allow ads based on what I watch | | |
| Privacy > LG may collect | Personalised advertising (and "(secondary)") | Allow personalised ads (and "(second consent)") | | |
| Privacy > LG may collect | Advertising cookies | Allow advertising cookies | | |
| Privacy > LG may collect | Sharing data with other companies | Allow sharing data with other companies | | |
| Privacy > LG may collect | Additional usage data | Allow extra usage data | | |
| Privacy > LG may collect | Remote diagnostics uploads | Allow remote diagnostics uploads | | |
| Privacy > LG may collect | Voice recordings (and "(secondary)") | Allow voice recordings (and "(second consent)") | | |
| Privacy > LG may collect | Sharing viewing data with data partners | Allow sharing viewing data with data partners | | |
| Privacy > LG may collect | Who.Where.What? | Allow Who.Where.What? suggestions | | |
| Privacy > LG may collect | Marketing messages | Allow marketing messages | | |
| Privacy > LG may collect | generalTermsAllowed, networkAllowed, firstUseAllowed, allAllowed (the raw key, dimmed) | Accept LG's general terms / network service terms / first-use terms / all of LG's terms | the agreement is accepted | the key was shown as the label; any other unknown consent now shows its key in words ("Allow some new consent") |
| Privacy > LG may collect (also Support > Privacy) | Live Plus | Allow content recognition (Live Plus) | LG recognises what the TV shows for recommendations and ads | |
| Privacy > Ads & data (also Support > Privacy) | Live Promotion; detail "Turn Live Plus on" | Allow custom ads while watching (Live Promotion); detail "Allow content recognition first" | LG shows custom ads and services while watching | moved to Ads & data; the detail names the row it waits for |
| Privacy > Ads & data (also Support > Privacy) | Do Not Sell My Personal Information | Ask LG not to sell my data | LG's ad service has been told not to sell this TV's data | "Do Not" next to a switch: a double negative when read as "on = do not" |
| Privacy > Ads & data (also Support > Privacy) | Limit AD Tracking | Limit ad tracking | tracking is limited | case only |
| Home & look > Home | Home Promotion | Show promotions in LG's Home | LG's own Home menu shows sponsored ads | a bare name; the help said "shows or hides" |
| Home & look > Home | Content Recommendation | Show content recommendations in LG's Home | LG's Home shows recommendations | |
| Home & look > Home | Who.Where.What? | Show Who.Where.What? suggestions | LG suggests people, places and things in Live TV | |
| Home & look > Home | Smart Tips | Show feature tips on Home | LG's feature tips appear on Home | |
| Home & look > Home | Smart Tips in Settings | Show setting tips in LG's Settings | LG's tips appear in its Settings screens | |
| Home & look > Screensaver | Screen Saver Promotion | Show ads on LG's screen saver | apps may put ads on LG's screen saver | |
| Language & location | LG Services Country | Set LG services country automatically | the TV finds the country itself | "Country" with a switch: on could mean any country |
| Network > Wi-Fi | TV On With Mobile | Wake from a phone (Bluetooth) | a phone can turn the TV on over Bluetooth | same words as "Wake from the network" |
| Sound > Output | Delay bypass | Skip the sound delay | the delay is skipped | |
| Sound > Output | TV speakers too | Also play on TV speakers | the TV's speakers play beside the receiver | |
| Picture > Mode | Just Scan | Show the whole picture (Just Scan) | the picture is shown whole, not cropped | LG's name says nothing about on |
| System > Time & power / OLED care | Pixel Cleaning | Clean pixels when the TV is turned off | a cleaning runs at the next power off | |
| System > OLED care | Care Recommendations | Suggest picture care settings | LG suggests a care setting | |
| System > OLED care | Screen shift | Shift the picture slightly | the picture moves a pixel now and then | |
| System > OLED care | LumaTV screens / Shift apps | Protect LumaTV screens / Shift still apps | LumaTV's own screens shift and dim / an app that sits still is moved | a verb, like the rows beside them |
| Support > Update | Auto Update | Install updates automatically | the TV installs updates by itself | |
| Support > Update | Block LG system updates, a switch | LG system updates, a status: Blocked / Allowed, with a chevron | status | allowing LG's update can remove root and LumaTV, so it is a question and not a switch; blocking asks nothing |
| AI access | Allow AI assistants: Off / LAN only | Allow AI assistants: Blocked / Home network only | | "LAN" is jargon; "Off" next to "Allow" is a double negative |
| AI access (rail and quick panel) | On / Off | Allowed / Blocked | | a status in words |
| AI access (quick panel detail) | LAN only | Home network only | | |
| Quick panel (Add a tile) | Screen off, sound on: button "Turn off" | button "Screen off" | | "Turn off" what |
| Cameras (plugin page) | Event pop-ups / Over playing apps | Show event pop-ups / Show pop-ups over playing apps | pop-ups are shown | |

## Rows checked and kept

ON already means the thing the label names, so no change: Audio Guidance, Video Description, NEXTGEN TV Description, Amplify Dialogue,
Sign Language Zoom, Device Power Sound, Closed Caption, Emergency alert, Text with High Contrast, Focus with High Contrast, Grayscale,
Invert Colors, Screen Zoom, Magic Remote pointer, Number keys open dock tiles, Hold the gear key for notifications, Quick panel on gear,
Quick Access, Extend Pressing Time on Remote, Receiver power, Always Ready (and its notifications and motion sensor), Dynamic tint, Wallpaper
motion, Smooth transitions, 12-hour clock, Greeting, Artist and title, Weather and home fact, LumaTV saver, Input banner, HDMI-CEC (SIMPLINK),
Quick Media Switching, HDMI Deep Color, 4:4:4 Pass Through, Dolby Vision PC, Wake from the network, IPv6, Screen recording, Eye comfort, AI Picture
Pro, Real Cinema, Precision Detail, OLED Motion Pro, Filmmaker Auto Start, AI genre selection, AI brightness, Filmmaker ambient light, Dolby
Atmos, Automatic volume, No Signal Auto Off, Auto Power Off after 4 Hours, Quick Start+, Open apps without updating, Block ad & tracker hosts, Allow AI assistants, Skip black bars (Ambient light), the Home Assistant and camera plugin switches ("Tell me
when it rings", the camera kinds). The rows with several answers (Dim over time On / Off, Dim logos and subtitles Off / Light / Strong,
Screensaver Clock / Art / Off, the Auto / Always / Hidden of the input choices) are value chips with plain options already.

Not changed here: "Home Assistant's notifications" (the Home Assistant plugin's own schema, `LumaShow/plugins/home-assistant`).

## Questions (confirmation dialogs)

A switch never opens one. The ones left are for what cannot be undone, restarts something, or gives LG's update a way in, and none of them
sits on a switch.

| Where | Old | New |
| --- | --- | --- |
| Privacy: any blocker or consent | "Turn on {title}?" and the helper's technical sentence, then Turn on / Cancel | none: the switch flips, the helper is asked to prepare and apply in one go, and a toast says what was done ("Content recognition and ads blocked", "Personalised ads no longer allowed; also changed: ..." when the agreement is shared) |
| Privacy: Prime Video ads | "Turn on Prime Video ads (Amazoff)?" with a body about a per-TV CA, goproxy, rootfs and DRM, and a step for the owner ("Close Prime and stop the old AmazOff service/autostart before enabling") | status row; "Block Prime Video ads?" / "Prime Video restarts, and the old AmazOff app, if it is installed, is switched off." / Cancel, Block ads. The helper closes Prime Video, stops the old app's programs, moves its start-up script aside and removes the certificate-check override it left in Prime's configuration (`helper/privacy/src/legacy.rs`); nothing is left for the owner |
| Privacy / Support: Do Not Sell | "Stop the sale of this TV's data?" | none; toast "LG asked not to sell this TV's data" |
| Privacy / Support: Live Plus | "Turn Live Plus on?" | none; toast |
| System: Pixel cleaning | "Clean the pixels?" | none; toast says it runs at the next power off and takes about an hour |
| Home & look: Always Ready off | "Turn Always Ready off?" | none; toast "Always Ready off: in standby the TV goes dark" |
| Language: services country | "Find the country automatically?" | none; toast |
| Support > App Permissions: turn an app on | "Allow {camera} access?" Allow / Block | none; toast "{app}: camera access allowed" |
| Support: Block LG system updates | switch, a double question to come off | status row; blocking asks nothing (toast), allowing asks twice ("Allow LG system updates?" then "Really allow LG system updates?", Allow updates) |
| Privacy: Reset AD ID, Delete connection history, resets, Delete My Personal Information, Lock out all AI, Remove assistant / profile / device, Forget link | question with a danger answer | unchanged: they are buttons, not switches |
| AI access: turning access on, a wider level, a higher limit, resuming | "Not available here" notice (the phone's approval is not connected) | unchanged: it refuses and says why; nothing is sent |

The QML pages got only the removal of the question on the rows above (Support, Language, System, Look, App Permissions); there is no toast
there. Their wording matches the Rust pages.
