# Live TV channel banner

## What draws LG's banner

`inputcommon` (QML: `ChannelBanner/ZappingBanner.qml`, `BannerInterfaces.qml`) draws it into the Live TV card's own picture, over the video.
`com.webos.app.livetv`'s `appinfo.json` names `com.webos.app.inputcommon` as its main, and the one `inputcommon` process (CRIU-restored) hosts it. On the set
(`ctl windows`) the card is one window and one surface; its group's other member, `com.webos.app.livedmost`, lies under the video. The banner is not a layer
or a window, so it cannot be hidden by hiding one.

No restyle is mounted: `inputcommon.sh` and its patch were removed with the QML front-ends (8f6c04ab), `/proc/mounts` has no `inputcommon` entry, and the
banner is stock. Its look (`GlobalStyle.qml`, `zappingbanner_bg_left_*.png`): a flat bar, alpha 230, 156 high with 12 px corners and no shadow; flush to the
left or right edge, from y 562 (`mBannerTopMargin`) or at the top (`option.bannerPosition`: `lefttop`, `righttop`, `leftbottom`, `rightbottom`; the owner's
is unset, so low left); 582 to 960 wide, 1500 for an internet channel.

When it shows (`InputEventLayer.qml`, `BannerInterfaces.qml`): after a channel change by CH+, CH-, a number, Flashback or a favourite, and at a launch, for
five seconds; on OK, which toggles it with a Guide chip; on INFO as a detail card for thirty seconds. The set's `inputcommon` log of a launch
(2026-11-03, `/var/log/inputcommon`) has the order: `changeChannel` answered at 22:36:11.351, `channelBannerShow` at .548, the banner's view visible at .735, and
`eventFirstBannerClosed` at 16.803. Its data: `utp.broadcast/getCurrentChannel` (a subscription on the pipeline), `iepg/getChannelInfo`,
`getCurrentNextEventList`, `utp.channel/getLastChannelId`.

The "Guide" chip is a view of its own, not part of the bar (`DeeplinkView.qml`): a flat grey button drawn into the same surface 25 px under the bar's foot,
225 by 54, 27 in from the bar's left edge. On the set's captures of 2026-11-04 (`/data/home/tmp/lumatv-tv-20261104b/shots/`) it is x 27 to 251, y 743 to 796 at
a launch (`L-launch-b` to `L-launch-f`: up from the card's first second, gone by 6.5 s) and with OK's banner (`G-bg-ok-banner`), and in none of the seven
captures after a CH+ (`C-chup-a` to `g`). The owner's `option.bannerPosition` reads `none` (unset, so LG's default, low left): the chip does not depend on it, it
rides with the bar. The bar's rectangle (`y 556`, 168 high) ends 19 px above it, which is why the chip showed at every launch with the bar gone.

## Approach

LumaTV draws its own (`system/channel_feed.rs`, `banner.rs`, `banner_surface.rs`): one slim strip of the theme's glass, centred over the picture and resting on the
toasts' grid above the subtitles' band (bottom edge at y 889, from `lumatv-placement`'s `above_subtitles`). From the left: the channel's logo on a plate (the
letters of its call sign where it has no logo), its number and name, a rule, and what is on, the title over its time and a line of how much is left. At 100 % text
it is 800 by 114 px for a channel with a programme (the first banner was 1457 by 154 at x 64, y 870), 546 by 114 for "No information", which is said quietly
(secondary ink, no time, no line), and 1056 by 114 for long names; INFO grows it upward to 880 by 262 with what the programme is about, then what is next and its marks.
The glass is the theme's sheet, no more see-through than 1.5 % (`pane::reading_glass`; Lucent's `scrim.hidesMin` allows 3 %): at the reading alpha the first banner
had, 0.8, the print of an ad behind it read through its tiles. It follows the tuner with a subscription to `getLastChannelId` while Live TV is in front and
`banners.own` is on, asks for the channel's record and its programmes on each change, and shows within 700 ms; five seconds for a launch or a change, eight for
INFO. INFO is the UI's while Live TV is in front, so LG's INFO card does not open under it.

LG's banner is blanked on Live TV's picture (`compositor/.../mask.rs`, `POST /v1/mask`). The UI claims two rectangles (`system/lg_mask.rs`, placed by the
position setting): LG's bar and the Guide chip under it. The compositor then leaves the claimed rectangles undrawn, so the video's hole shows there, for 6.5 s
from a CH+ or CH- that reaches Live TV, from each channel change the tuner reports (a number, Flashback) and from a launch. The blanking starts before LG's
banner is drawn: LG draws it about 0.2 s after its tune is answered. The claim is released when Live TV leaves the front, `banners.own` goes off, the tuner
cannot be followed, or the UI is gone: the stock banner returns whenever the UI is not running. At the left the chip's rectangle is the chip and a 6 px margin;
at the right the bar's width is the channel's, so the rectangle runs from where the widest bar would put the chip to the screen's edge.

HDMI inputs have the same double. `inputcommon` hosts the input's card (`com.webos.app.hdmi2`: one window, one surface, a punch-through hole) and draws LG's launch pill into it
(`InputBanner/SimpleBanner.qml`: the bar's height, at `option.bannerPosition` like the channel banner, no Guide chip), so every switch showed LG's pill and this UI's.
On 2026-11-11 the switch was pushed to the UI at mono 12506.02 and `FR03_0_BA_STATE "launching"` came at 12506.63; LG hides the pill after 5 s. The UI claims the bar for
whichever input is in front with no keys and no chip and blanks it for the same 6.5 s from the switch; a switch from one input to another claims the new window. The OK and
INFO banners of an input stay LG's, and so does its no-signal screen except where it lies under the bar in those 6.5 s.

Not chosen: covering LG's banner (it is flush at x 0, this one is 64 px in, so the edge shows); `hotelMode/tvlinkOsdDisplay` (hides every banner but also
consumes the other keys and persists); taking OK and CH+/CH- and tuning here (breaks LG's popups and the keys' other duties); reviving the QML overlay
(removed with the other restyles).

Limits: the banner LG shows for OK stays LG's, with its Guide chip. During the 6.5 s after a channel key, anything else LG draws on the bar or at the chip's
place (a PIN prompt after tuning to a blocked channel, a no-signal notice) is blanked too. `banners.own` false gives LG's banner and everything else back.

## Renders

`tests/render_rust_banner.py` writes `build/renders/banner/` (not tracked): a channel with its logo, without one, with long names, with no programme and in INFO,
dark and light at 100 % and 200 % text, over an ad (white ground, black print, a phone number where the banner lands), a night street or any PNG
(`--backdrop`: the set's own captures). `--measure` reads each word's contrast on the finished picture (the median of its box against the pixel farthest from it)
and fails below 4.5 to 1; the 48 renders read from 5.5 to 6.6. The unit test `the_strip_hides_a_white_ad_and_its_black_print_...` holds every word to 4.5 over
white and over black. `--timeout` checks the banner is gone after its time.

`tests/host_budget.py --surfaces banner` walks a launch, a change and INFO on the host: at most 11 layers in a frame for the first two (2.4 ms by the gate's
estimate) and 18 for INFO (3.0 ms), against 8.3 ms at 120 Hz (16.7 ms at 60); the first banner had 14 and 15 (2.7 and 2.8 ms). 41 to 42 frames for each banner
(its fades) and none once it has gone. The set's own frame gate is not run.

## TV steps (not done: the owner may be watching)

1. With Live TV in front, run `luna-send -i -f luna://com.webos.service.utp.channel/getLastChannelId '{"sourceType":"INPUTSOURCE_RF","subscribe":true}'`
   and press CH+ once: a second answer naming the new channel within about a second means the banner follows the tuner. If none comes, the feed has
   to ask again on a timer (not built), and until then the bar LG draws is blanked on CH+ and CH- with nothing of this one in its place: `banners.own` false
   gives LG's banner back.
2. Install the build from `main` (`lumatv-ui` only: the compositor and its mask route are unchanged); name the binary hashes.
3. Launch Live TV from Home and capture at 1.5, 3 and 5 s: LG's grey "Guide" button (x 27 to 251, y 743 to 796) must not show and this strip must, centred with its
   foot at y 889. Press CH+, CH-, a number and INFO; capture each. LG's bar must not show, this one must, and INFO twice must end it. Leave it five seconds: it must
   be gone. OK pressed 7 s after a launch must show LG's banner with its Guide button. Check that the video shows where LG's bar would be, and that the claim is
   where LG puts the banner (`option.bannerPosition`).
4. Look at the strip over a bright ad and over a dark scene, dark and light, at 100 % text: whether anything of the picture shows under its glass, and its
   size and place against the subtitles.
5. Run the frame gate on Live TV through a few channel changes and an INFO (the compositor's plan and renderer changed).
6. Tune to a blocked or empty channel right after CH+ and check LG's prompt or notice is readable once the 6.5 s are over, and what shows during them.

## Checked on the TV by `luna-send` (read only, 2026-11-04) and not yet through the UI

- `utp.channel/getLastChannelId` answers `subscribed: true` with the channel id; `iepg/getChannelInfo {dataType: 0, channelId}` answers the channel's
  number, name and `imgUrl`, a local PNG; `iepg/getCurrentNextEventList {dataType: 0, channelId: [id]}` answers two programmes, the one on air and the next,
  with `startTime` in UTC and `localStartTime`; `settings/getSystemSettings {category: "option", keys: ["bannerPosition"]}` answers `{settings: {bannerPosition}}`.
- A channel's logo file can be missing (LG's own banner logs "Cannot open" for WSB-HD's): no plate is drawn then.

## Not verified on the TV

- `getLastChannelId` pushing on each tune.
- The `info` key being taken on Live TV (LG's INFO card not opening) and the compositor's luna bridge carrying the reads of the feed.
- A logo PNG under `/mnt/lg/cmn_data/channel_logo/` being readable by the UI process.
- The blanking on the set's GL path (frame gate, damage, no flash), its timing against LG's banner, and what LG draws on the bar that is not its banner.
- The Guide chip's blanking on the set (the launch captures with this build), and at the other `bannerPosition` values, where the chip's place is the bar's offset and
  not measured.
- The strip on the panel: its place against captions, and how much of a busy picture shows under its glass (4 levels in 255 under black print on white on the host).
