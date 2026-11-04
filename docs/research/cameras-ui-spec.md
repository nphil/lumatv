# Cameras on the TV: UI spec for the Rust shell

The behaviour, numbers and words of the Cameras screens: the pop-up, the Cameras app (grid, full view, timeline strip, events and clips sheet) and the Settings page.
Toolkit-neutral; the screens are the Rust shell's. Sources: `scrypted-cameras-design.md` 3.3 to 3.7 (the contracts), the mocks in `docs/mocks/cameras-scrypted/`
(`index.html?v=<view>&theme=light&text=200`, `node render.mjs` renders them and checks overflow, overlap, the safe area and contrast; kept pictures are `*.jpg`),
the glyphs in `assets/glyphs/`, Lucent v2 (TV profile: safe area 96 and 54, target 64, row 80). No QML was written for this; nothing here has run on the TV.

## Rules for every screen

- **No sentence of instruction.** Keys are glyphs: the key glyphs in the reading ink, then what they do in the second ink, 1.45 times the type size of the text beside them,
  never the words OK or Back (`assets/glyphs/index.json`). Labels only name things: a camera, a time, a control.
- **Honest states, in words that carry no address or token:** `unlinked` "Scrypted isn't linked", `connecting` "Connecting to Scrypted", `refused` "Scrypted refused this TV",
  `untrusted` "Can't verify Scrypted", `offline` "Can't reach Scrypted", linked with no camera shared "No cameras shared". A picture is never invented: a tile that is not fresh keeps
  its last picture, dimmed, with its age.
- **Times** follow `theme.clock.twelveHour` (default on): "2:31 PM"; today has no day word, then "Yesterday", then "Oct 27". Ago: under 60 s "Just now", under 90 min "N min ago",
  under 24 h "N h ago", then "Yesterday", "N days ago". Behind live: "Now", "42 s ago", "3 h 25 min ago". Clip length "0:30". A bridge clock a little ahead never shows a negative age
  (`now` is the TV clock plus `now_skew_ms`, which is bridge minus TV).
- **What was seen:** the kind's word ("Person", "Vehicle", "Animal", "Package", "Doorbell rang", "Motion"; any other kind "Event"), then " · " and the label when the recorder
  named it and the label is not the class again ("Animal · Domestic Cat"; a class name used as a label reads as words, `dog_cat` is "Dog cat"). Glyphs: `user`, `car`,
  `paw-print`, `package`, `bell-ring`, `activity`.
- **Chrome over the picture** is one group on a reading surface (the glass at `readingAlpha`, no blur over video). It fades out after 2.5 s without a key (180 ms) and in over 120 ms;
  the first key only reveals it; Back works while it is hidden; it never fades while nothing plays (starting, failed, paused, the sheet up, a toast showing). Reduced motion: one linear fade of 120 ms at most.
- **Motion** is time-based (`p = clamp((now - start) / duration)`), no frame counters. The motions that exist: the chrome fade, the sheet's travel (16 px, 220 ms in, 180 ms out), the grid's
  row scroll (300 ms, out-cubic, interruptible). Focus changes land at once; nothing animates per picture.
- **One button size a screen** (target 64). Checked at 100 % and 200 % text: no overlap, no overflow, inside the safe area.

## Entry points

The pop-up's OK; a quick-panel tile "Cameras" (value: the camera count, or the state's words); Settings, Home & look, Cameras. The app opens over Home or over any app and Back returns to where
it was in one frame: the layer is built at warm-up and shown, nothing is built or compiled in the frame a key waits for. The last picture of every camera (at most 16, about 150 KB each) is
kept in memory, so the grid paints them in its first frame. What draws a picture (its shader or pipeline) is warmed once with a stand-in at warm-up, so the first picture is not the frame that builds it.

## Data the screens read

| Source | Fields used |
|---|---|
| topic `cameras` | `state`, `reason`, `now_skew_ms`; per camera `id`, `name`, `room`, `online`, `live`, `recordings`, `events`, `clips`, `since`, `native`, `prebuffered`; already ordered and filtered by the `show` setting |
| topic `cameras.popup` | `id`, `event`, `camera`, `name`, `kind`, `label`, `at`, `untilMs`, `seq`, `thumbnail` (a picture of the moment is there), `shownAt` (when the helper published it), `overApps` (the owner's "Over playing apps", enforced by the shell), `test`; an event's story (`new`, `update`, `end`) has one id, which is also its picture's: an `update` re-publishes the same `id` with new words and `seq` and is never a new pop-up |
| topic `cameras.live` | `camera`, `mode` (`live`, `playback`, `clip`), `state` (`asking`, `starting`, `playing`, `failed`, `stopped`), `reason`, `codec`, `width`, `height`, `fps`, `positionMs` and `startedAt` (both `null` in live), `rate`, `paused` (a recording held: the last picture stays, nothing is dialled) |
| topic `cameras.events` | the last 30 pushed events, newest first, with `phase`; one card per event `id`, from its newest `new` or `update` entry; `end` entries make no card; `score` may be absent and is not read |
| `GET /v1/cameras/{id}/frame?w=&h=` | 200 picture, else 404 `no_picture` (none yet, or too old), 503 `not_connected` or 429 `busy`; a still is never older than 10 s |
| `.../recordings`, `.../events`, `.../clips` | ranges, `unknown`, ticks, clips (3.4); `.../thumbnail`, `.../clip-thumbnail`, `.../events/{event}/thumbnail` for pictures |
| `POST /v1/cameras/live`, `/playback`, `/live/stop`, `/live/pause`, `/popup/clear` | a refusal is HTTP 409 `{error: {code, message}}` and leaves the running session alone; its words (table under "Words of a refusal") are shown for 3 s |
| `GET` and `POST /v1/settings/scrypted-cameras` | the page of 3.5; choices at `/v1/plugins/scrypted-cameras/choices/{cameras or kinds}`; forget at `POST /v1/plugins/scrypted-cameras/actions/forget` |
| facts `plugin.scrypted-cameras.link`, `.device`, `.verifier` | the connection in words; the TV's name for the bridge's Devices group; the SHA-256 of the TV's own token as 64 hexadecimal digits (a hash, never the token, an address or the pin); the last two exist while a link does |

## The pop-up

The card of class `camera`: 560 wide, placed by the overlay layout, never below 810. Head: the kind's glyph (36), the camera's name (body, semibold) and what was seen with its age (label, second
ink). Picture 520 by 292, giving height back (never under 120) where the type is large. Foot: the state at the left, the hint glyphs at the right: OK, `expand`; Back, `close`.

- **Picture:** the event's own picture (`/events/{event}/thumbnail`) at once, then the camera's stills, the next ask 250 ms after the last arrived (4 a second) for as long as the card is up.
  State: `Live` with the live dot once a still is on screen; `Event picture` while only the event's is; `Connecting` before any; `No picture` after two refused still asks with none shown.
- **Policy:** the helper decides which events pop up (their kind is chosen for that camera, a new object, the cooldown, not older than 30 s); the shell decides when the card shows. Due when its `id` differs from the one shown and `untilMs` is ahead;
  it waits (looking again every 500 ms) while the host is busy (a PIN, an alert, a panel, the keyboard, the AI prompt) and, with `overApps` false, while an app is in front, and shows when free if still in date (a card meant for Home that an app
  interrupts shows on Home if the person comes back within its time); one that is up when the host turns busy, or when an app comes to the front with `overApps` false, steps aside, is not dismissed and returns with the time it has left. An update changes the words of
  the card up and re-reads `untilMs`. It ends at `untilMs` and tells the helper (`popup/clear`). While the viewer is up there is no card: the full view turns to the event's camera, the grid does nothing.
- **Keys:** only OK and Back, and only 600 ms after it arrived; the arrows stay the app's. OK clears the card (told to the helper) and opens the full view of its camera; Back clears it.
- **Over an app** it is stills only; nothing of the app's playback is touched. A doorbell stays half as long again, is never held back by the cooldown and tells the overlay system it showed
  (`popupShown(kind)`), which folds the Home Assistant plugin's record of the same ring (within 20 s) into the card.

## The grid

Over a scrim that hides the page (`scrimHide`). Header at y 54: "Cameras" (title), the count ("8 cameras", label, second ink), and at the right two buttons, `Events` (`list-video`) and `Settings` (`settings`).
Under it, when tiles exist and the state is not `ready`, one banner line with the state's words. Hints at the foot, right: OK `expand`, Back `close`.

- **Tiles:** columns by count, 1 to 4 across (1, 2, 2, 2, 3, 3, then 4); at most two rows show, so at most 8 stills are asked for (the bridge's limit); a tile is as wide as its columns allow and no taller
  than its rows allow with 44 px of name under it, 16:9, an even size, gap 24, the block centred in the room (x 96 to 1824, y 150 to 966 at 100 % text; the header's height at larger type moves `y`).
  Picture cover with the tile radius; the name only (no room). Focus: a lit edge of 3 px and scale 1.03, the name in the full ink.
- **Picture pacing:** each shown tile asks `/frame` at its own pixel size, the next ask after the last arrived plus the interval of the `video.grid` choice: Light 333 ms, Balanced 200 ms (default),
  Smooth 125 ms. Tiles scrolled away or a closed grid ask nothing; their last pictures stay.
- **Tile words:** nothing while the picture is fresh (under 12 s) and the link is `ready`; `Connecting` before the first; `No picture` after two refused asks; a camera with `online: false`
  "Offline" (with " · 3 min ago" past a minute); any other state of the link or a picture older than 12 s dims the tile and gives its age.
- **Scroll:** more than 8 cameras scroll by whole rows, only as far as the focus needs, 300 ms; the foot's left says the range left below ("9 to 12", `chevron-down`).
- **Keys:** Left and Right move along a row without wrapping, Up and Down by rows; Down from the last tile of a longer row above a short last row lands on the last tile; Up from the top row moves to the header's
  buttons (Left and Right between them, Down back to the tiles). OK opens the focused camera's full view, `Events` opens the sheet for every camera, `Settings` opens Settings, Home & look, Cameras. Back closes.
- **Empty states:** no tiles draws the state's words (title) and one primary button `Settings`; a refused or untrusted link the same.

## The full view

The camera on the whole screen, fitted. The picture is a hole (the layer is transparent over the video plane) while `cameras.live` plays this camera in the mode asked; before that, and always
when an app is in front, it is stills (1280 by 720, the next ask 125 ms after the last). Over an app live video is offered only when `theme.cameras.liveOverApps` is true (check C12); the strip, the sheet and
playback are then not offered. Over Home the live stream is asked on opening and at every switch (`POST live {camera}`; the answer is the `cameras.live` topic).

- **Name pill**, top left: `video`, the name, the state (the live dot and "Live", "Starting", "Recorded", "Can't play"), "2 of 8". **Info pill**, top right: `3840×2160 · H.265 · 15 fps` from the stream (`native` until it plays); a member the stream does not give is left out, and with none the pill is not drawn.
  A stream that is not prebuffered says "Opens in a few seconds" while it starts. While starting and failing and when stills only, a stills backdrop shows (4 a second); a prebuffered start is black until it plays.
- **Hints**, bottom right, only when the strip and the sheet are closed: Left and Right `video` (when there is more than one camera), Up `history` (when the camera records and live video is on), Down `list-video` (when it has
  events or clips and live video is on), OK `layout-grid` (more than one camera), Back `close`.
- **Keys with the chrome up:** Left and Right the previous and next camera (wrapping); Up the strip; Down the sheet; OK the grid; Back, in order: a playback goes live, else the view closes. With the chrome hidden a
  key other than Back only reveals it. A new camera or a close tears the session down off the UI path (`live/stop`).
- **Failure:** `state: failed` shows a note over a dimmed backdrop: "Can't play this camera", the reason's words, a `Try again` button (OK, which asks again at once: a stream address lives 25 to 60 s, so a retry always asks for a new one), Back closes.
  A recording that ends (`stopped`, "End of recording") asks live again by itself. A stream the bridge ended because the device was removed shows the same note and the grid's state turns to `refused`.
  **A refused start** (`POST live` answered 409 for the camera shown) is the same note, over the stills, not a toast: a camera Scrypted says is offline reads "This camera is offline", "Looking again every few seconds.",
  and any other refusal its words from the table below. A refusal that passes (`offline`, `busy`, `timeout`, `unreachable`, `not_ready`, `not_now`, and `upstream_error` twice at most) is asked again by itself after 3 s, 6 s and
  12 s and then every 30 s, eight times; a stream that drops after it played is asked again the same way ("The stream went quiet. Trying again soon."). A camera that never played is never asked again by itself
  (a battery doorbell would wake for each ask), and neither is a refusal that is the same next time (`not_supported`, `camera_not_found`, `live_off`). A session another camera held when the start was refused is stopped.
- **The bars** (`theme.cameras.fillBars`, on): where the picture is not 16:9 (a doorbell's 3:4, a 4:3 camera) and the bars would be 16 px or more, they are a blurred, dimmed copy of the camera's picture
  (`docs/research/cameras-fill-result.md`). While the video plays in its window the layer is a hole there and the wash is two bars beside it, made from a 32 by 18 picture of the camera (the helper decodes the bridge's
  320 px still at one eighth) asked for every 2 s, and only with a granted, placed video window (`vwin`): on the plane path nothing of where the video is is known and the bars stay black. A still shows whole over
  a wash of itself, made from it at most once a second. A grid tile whose picture is not 16:9 within 4 % shows the whole picture over a wash of itself, with the tile's round corners, instead of its middle.

### Words of a refusal

A refusal of a `POST` (the bridge's `error.code`, passed through by the helper) is a toast of one line for 3 s over the screen it came from; the picture goes on as it was (a refused start of the camera shown is a note, see Failure). A code this table lacks shows the helper's own words.

| Code | Words |
|---|---|
| `offline` (409) | The camera is offline |
| `not_supported` (501) | No stream fits this TV |
| `no_recording` | No recording at that time |
| `camera_not_found` (404) | Scrypted no longer shares this camera |
| `busy` (429) | Scrypted is busy |
| `timeout` | Scrypted didn't answer |
| `upstream_error` (502) | Scrypted couldn't start it |
| `event_not_found` (404, a picture) | no words: the card has no picture and says nothing about it |

## The strip

Up opens it over the picture: a glass panel from the safe edges, three rows of focus, top to bottom.

1. **Top row** (right): `span` (15 min, 1 h, 6 h, 24 h; default 1 h) and `rate` (1×, 2×, 4×); Left and Right between them, OK cycles the value. Left of it the time of the cursor (clock, 28 semibold) and how far behind live.
   Up from here closes the strip.
2. **The bar:** recorded ranges as accent bars, `unknown` ranges as the same bar dimmed to 30 %, only a confirmed gap (inside what neither names, between `since` and now) hatched; event ticks as dots above the bar; the axis
   under it (the window's clock times); the knob. Left and Right scrub, OK commits the cursor (when it is not where the picture is) or toggles pause in a playback, Down to the controls.
3. **Controls:** `Live` (selected while live), `−10 s`, `Pause` or `Play`, `+10 s`, then at the right `‹ Event` and `Event ›` (the nearest tick before or after the cursor, 3 s of lead-in). Down from here opens the sheet.

- **Scrub:** one press moves the cursor a sixtieth of the span (never under 5 s), ×3 after 6 repeats, ×10 after 14, ×30 after 28 (a repeat is the same key within 350 ms); never past now or before `since`.
  The window moves only when the cursor leaves it, by half a span, never into the future. The cursor commits on the key's release, or 450 ms after the last press where releases are not delivered: `POST playback {camera, at}`; a cursor in a confirmed gap
  shows "No recording at that time" (3 s) and asks nothing. While the request is out the picture area shows the cursor's thumbnail.
  **A refusal** from live (HTTP 409 on `POST playback`) shows its words as the toast and live goes on as it was. The bridge keeps one playback session a camera and ends the old one as it takes the next call, so **a refusal while a recording plays**
  (a seek, the next clip) leaves nothing playing: `cameras.live` says `failed` with the words, the shell shows them as the toast and asks live again by itself, the cursor staying where it was with `Live` selected. A recording's stream that ends while the next is asked for is
  not an end of recording; the end of one is `stopped`, "End of recording".
- **Bubble** while the cursor is away from the picture: `/thumbnail?at=&w=256&h=144` asked 250 ms after the cursor settled, the last picture kept until the next arrives; under it the time and, on an event, its kind word.
- **Data:** `/recordings` and `/events` for the window on opening, on a span change, when the window moves and every 30 s while open; a failed load shows an empty bar. Rate and Pause: Play re-dials at the held time (the helper does it);
  a rate change in a playback re-dials at the playhead. The playhead is `positionMs` (4 updates a second), live is the right edge.
- **Fade:** the strip fades with the chrome in a live view and closes with it; in a playback it stays while paused and comes back with the next key.

## The events and clips sheet

Down opens a lower pane over the picture (full view) or over the grid (`Events`, every camera). Title (the camera's name, or "Recent events"), tabs `Events` and `Clips` (the camera's own; none in the recent list), a row of cards, 352 by 198 pictures with
the kind's glyph at the corner and, for a clip, its length; under the picture the title (two lines at most) and "2:31 PM · 6 min ago". Four cards show; a `chevron-right` says there are more; Left and Right move, Up to the tabs, Down returns,
OK plays (a clip: `POST playback {camera, clip}`; an event with a recording: `{camera, at}` three seconds before; an event without one only turns to the camera), Back closes. Events come from `/events` for the last 24 h (up to 30, shown newest first),
clips from `/clips`, the recent list from `cameras.events`. Pictures: `/thumbnail?at=`, `/clip-thumbnail?clip=` (the clip's id is percent-encoded), `/events/{event}/thumbnail`; a card loads its picture only when it shows; a picture that is gone or never came (`event_not_found`, `has_thumbnail` false, an event with no thumbnail) leaves the card as the kind's glyph on a quiet tile, never a blank or a made-up picture.

## The Settings page

Home & look, Cameras; tabs `Scrypted`, `Cameras`, `Pop-ups`, `Notify for`, `Video`, drawn from the schema of 3.5 (`PluginRows` rules: bool is a switch, integer a slider, a list of choices rows with a check, the order picked shown as a number).

- **Scrypted:** the connection (chip with the state's words, "Connected · 8 cameras") with its help behind the info mark ("Linked from a shell on the TV with `lumatv-helper scrypted-link`"); while a link exists, `Device name` (the TV's name) and `Verifier` (read-only fields `link.device` and `link.verifier` of the schema, between the connection and Forget; its 64 digits in groups of 8, tabular, second ink, wrapping by group so that none is cut at 200 % text), whose help says where they go ("Type this name and verifier into the bridge's Devices in Scrypted. The verifier is a hash; the token never leaves the TV.");
  and `Forget link` (a second OK, "Sure?", six seconds, as the Home Assistant page). With no link the chip says "Not linked" and there are no other rows. `refused` shows the same rows with the chip in the danger tone: the TV is not (or no longer) in the bridge's Devices, and the name and verifier are what to add again.
- **Notify for:** one row per camera with the kinds as its value ("Person, Doorbell", "Off"), opening a page with a switch per kind (Person, Vehicle, Animal, Package, Doorbell, Motion); defaults person and doorbell. The page is rebuilt when `revision` changes.
- **Video:** `Live quality` Full, Medium, Low and `Grid` Light, Balanced, Smooth as two-to-three option segments.

## Test vectors

Grid, room (96, 150, 1728, 816), gap 24, name 44: 1 camera 1×1 at 1372×772 (x 274); 2: 2×1 at 852×480 (x 96, y 296); 3 and 4: 2×2 at 624×352 (x 324); 5 and 6: 3×2 at 560×316 (y 186); 7 and 8: 4×2 at 414×232 (y 270);
12: 4 across, 3 rows, two shown.

Scrub step by span for 0, 6, 14 and 28 repeats: 15 min 15, 45, 150, 450 s; 1 h 60, 180, 600, 1800 s; 6 h 360, 1080, 3600, 10800 s; 24 h 1440, 4320, 14400, 43200 s.

Axis at 2:37 PM, 12-hour: 15 min "2:24 PM", "2:27 PM", "2:30 PM", "2:33 PM", "2:36 PM"; 1 h "1:40 PM" to "2:30 PM" every 10 min; 6 h "9:00 AM" to "2:00 PM" every hour;
24 h "4:00 PM", "8:00 PM", "12:00 AM", "4:00 AM", "8:00 AM", "12:00 PM". Ticks fall on the local clock.

Gaps: with `ranges` [now-50 min, now-10 min] and [now-8 min, now], `unknown` [now-10 min, now-8 min] in a 1 h window ending now, the only gap is the first 10 minutes; with `since` = now-50 min there is none; the part of a window after now is no gap.
Update: a pop-up shown with id 41 (person), then a record with id 41 (animal, label "Domestic Cat", later `untilMs`): the card up reads "Animal · Domestic Cat", is not shown again (no second `popupShown`), and ends at the later time.
Jump to event: from 2:20 PM with events at 1:58, 2:12 and 2:31 PM, next is 2:31, previous 2:12; standing on 2:12 (within 1 s) next is 2:31 and previous 1:58.

## Not built

Every screen above. Until they exist the old QML surface (`panels/Cameras.qml`, `kit/CameraFrame.qml`) draws the pop-up and a viewer of at most six tiles on the Scrypted topics: it gates live on `cameras.state` being
`ready` and, over an app, on `theme.cameras.liveOverApps`, reads the pop-up's `at` in milliseconds and leaves out a label that only repeats the kind. It still draws text hints, which the no-sentence rule removes.
