# Channel guide (2026-11-04)

LG's Live TV guide rebuilt as a kind of the Rust shell (`guide`): the channels and programmes LG's own guide service has, in LumaTV's
look, over the live picture. Nitin asked for it on 2026-11-03; it supersedes "no guide" (PLAN, 2026-10-23). The code is
`rustui/ui/src/guide/` (`model` is pure and tested on the host, `surface` and `layout` draw it, `host` joins it to the shell,
`fixture` is a make-believe set of channels for the host harness) and `rustui/ui/src/host/channels.rs`, the data layer it shares with the
Live TV banner.

## LG's guide today

`com.webos.app.livemenu` (native, 4.6 MB) has three tabs, TV Guide, Channels and Scheduler (`activateType` `tvguide`, `channels`,
`scheduler`). Live TV (`com.webos.app.livetv`) has no screen of its own: LG's `inputcommon` QML draws the banner over the tuner picture and
launches the guide. The Magic Remote of this set has no Guide or Info key: the guide is reached with List (held, it converts to TV Guide),
the on-screen remote and the banner's Guide button. The data come from `com.webos.service.iepg` (channels, programmes) and
`com.webos.service.utp.channel` (the channel the tuner was on).

## Data

Every shape below was read from the set with read-only `luna-send` on 2026-11-03 (a US antenna set with LG Channels); `channels.rs` has the
fields. 57 antenna channels have sparse listings (a handful of the first thirty have any), the 459 LG Channels have none: "no information"
is the normal answer, not a fault.

| Call | Use | State |
|---|---|---|
| `iepg/getChannelList` (`channelMode` Tuner, then IP) | the rows; 31 KB for the antenna, 150 KB for the internet | verified |
| `iepg/getEventList` (`localStartTime`, `localEndTime`, `channelId` list) | programmes of a few channels for a few hours; about 3.7 KB each, `requestField` is not honoured | verified |
| `utp.channel/getLastChannelId` (`INPUTSOURCE_RF`) | the channel the focus starts on; answers with no live pipeline | verified |
| `applicationManager/launch` `com.webos.app.livetv` `{id: channelChange, channelId, sender: livemenu, type: channels}` | tune, from LG's `DeepLinkHandler.qml` | **unverified**: it changes the channel |
| `applicationManager/closeByAppId` `com.webos.app.livemenu` | close LG's guide when the new one answers it | **unverified** |

The compositor's luna allowlist has the five reads and the whole of `applicationManager`; the rest of `iepg` and `utp` (favourites,
reservations, tuning) is not open.

## Where it opens

LG's guide is answered, not replaced: when `com.webos.app.livemenu` comes to the front (the `foreground` topic), the guide opens over the
app it came over and asks LG's to close, so the key chain, the banner's Guide button and the on-screen remote all land in it. LG's guide is
the fallback while the shell is not running. Which tab LG was asked for is not in the topic, so the Channels tab is answered as well (the
guide lists every channel with its number, name and logo); the Scheduler is its own app and stays LG's. `ctl shell show guide` opens it
for a check.

It closes on Back, on Home, when another app comes to the front, when the screen goes off, and after five minutes without a key or the
pointer (a still page of bright rows is what the panel should not hold). The quick panel can be opened over it.

## Keys

| Key | Does |
|---|---|
| Arrows | left and right to the neighbouring programme (a gap an hour at a time), up and down to the next channel at the same time; up from the first row is the chips, down comes back |
| CH+, CH- | a page up or down (a screen less one row); the compositor gives the guide these only while it is up |
| Wheel | one row a notch; never leaves the grid |
| OK | on a programme that is on the air, or on a stretch with no information: tunes. On any other: opens its sheet. On a chip: chooses it |
| Info | the sheet of the programme in focus (the Magic Remote has no Info key; for the programme on the air the strip under the grid says what the sheet says first) |
| Back | closes the sheet, then the guide |
| Pointer | hover is a cue and moves nothing; a click focuses a cell, a click on the focused cell is OK, a chip is chosen by one click |

The filters are chips over the grid: All, the antenna's (Cable or Satellite when the channels say so), Internet (until its list says it
has none), Favourites when any is set, and Now, which goes back to the programmes on the air. The chosen filter carries a check, so the
choice does not depend on colour. The hints are glyphs: the arrows, OK and Back.

The view moves only as far as the focus needs (a clearance of a hundredth of the span), never to centre it; the focus is always whole. A
cell wider than the screen is shown from its start. The time runs from six hours before the hour the guide opened to five days after.

## What it asks for, and when

The antenna's channels and the last channel at once; the internet's list (150 KB) only when a filter needs it, once. Programmes: the
rows on the screen and one each side, for the hours on the screen (more for the row in focus), after the focus has been still for 140 ms,
three channels and six hours a call, two calls in flight. An answer is kept; an empty one is "no information" and is not asked again; a
call that failed or was not answered within 12 s is left alone for 15 s. Listings older than ten minutes are asked for again when the guide
opens.

## Drawing

The grid is cached and moved by the compositor: the rows, the time and the ruler are scrolled on tracks of the flow engine, so a key
retargets a spring (260 ms, 120 ms when motion is reduced) and nothing is drawn by the frame. A row is four layers: the logo, the number
and name, one glyph run holding every cell of the stretch of time round the view (titles, dividers, how far the programme on the air has
gone, the line of now), and the lead, the title of the cell that crosses the left edge, which stays at the edge. A lead whose cell
changes while the time flies waits, hidden, until the time stops, so that it never sits on the titles flying under it. Only the rows on
the screen are held, with a row beside them and the rows the scroll is going to; a jump of more than two screens is a cut. The focus is a
lit plate lifted by the flow engine, one layer a move. The shell is woken by nothing when a spring stops, so the guide asks to be looked
at again while there is something to follow (every 8 ms while rows are still to be made, three a frame; every 50 ms while a scroll moves)
and not at all at rest. Text is glyph quads on whole pixels (no picture of text); colours, glass, radii and type come from the theme's
tokens, so both looks and every text size follow it live. At 200 % text the rows keep the number alone and the description leaves the strip.

Honest states: "Loading channels", "No channels", a service that does not answer says so, a channel with no listings says "No information"
in its row and its strip, and a channel switch that LG refuses says so and keeps the guide up.

## Checks

The model, the host and the surface have behaviour tests (`guide::model_tests`, `guide::surface_tests`, `guide::host`, `guide::layout`,
`host::channels`): what is asked and when, every key, the layer count with 459 channels, that a moving grid is looked at until it has
stopped and not after, the layout and contrast lint clean in both looks at 100 % and 200 %. `tests/render_rust_guide.py` makes the pictures
(the lint is read for each) and `tests/host_budget.py --surfaces guide` walks it: opened, a held arrow in time and in channels, CH+ and CH-
six pages down and up, a programme's sheet, the filters, a rest, closed. `tv-gate.sh rust` opens and closes the guide on the set.

Frame walk, host harness (the engine in-process, one refresh a frame, over a make-believe set of 57 antenna and 459 internet channels
with listings for four in five of the antenna's). The GPU time of a frame is the set's estimate from its layers, 1.5 ms and 0.085 ms a
layer; the work adds the time the process took to apply what the frame showed.

| | Frames | Most layers drawn in a frame | Work, worst | Work, 99th percentile | Over 8.3 ms |
|---|---|---|---|---|---|
| 120 Hz | 2374 | 52 (5.9 ms) | 6.0 ms | 5.3 ms | 0 |
| 60 Hz | 1206 | 52 (5.9 ms) | 5.9 ms | 5.3 ms | 0 |

The window with the most is the sheet (52 layers: the page, the sheet and its button over the rows); a held arrow in time draws at most 42
and in channels 39; a page of CH+ or CH- at most 42. The guide draws nothing by itself at rest; the 1 Hz frames of a rest window are Home's
own under the page (Home alone draws the same three small frames a second in the same harness).

## Not yet checked on the TV

- That the foreground topic reports `com.webos.app.livemenu` when it comes to the front, and that the guide opens and LG's closes without a
  visible flash of LG's. List pressed, List held and the banner's Guide button each open it. If the topic does not report it,
  `ctl shell show guide` still opens the guide, and a remote-button action for it (List to the guide, beside Quick and Settings) is the next
  entry point.
- That the channel switch (`launch` with `channelChange`) tunes, and that Live TV's banner is quiet afterwards (`type: channels`).
- The guide over a live picture: the page's dim and the glass of the sheet against video, both looks.
- The wheel (`wheelup`, `wheeldown`), CH+ and CH- while the guide is up, and that they are Live TV's again once it is closed.
- The real channel list: logos from `/mnt/lg/cmn_data/channel_logo/`, the internet's 150 KB answer, rows of "No information".
