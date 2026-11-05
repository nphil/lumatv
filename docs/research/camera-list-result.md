# Camera list on Home's status bar

## Resume here

**Host-tested, not yet on the set.** A camera button sits beside the bell. OK opens a sheet in the notification drawer's place that grows out of the button: one row for every camera with a small live picture and its name. Up and Down walk the rows (a half row of the next shows under the last whole one), OK opens the camera in the Cameras viewer on the row's picture, Back closes the sheet into the button, a click does the same. The model is `panels/camlist.rs`, the surface `panels/camlist_surface.rs`, both wired as the panel `Which::Cameras` (one panel at a time, 45 s idle close, closed by an app coming to the front).

**Pictures are stills, not video.** Rows ask `GET /v1/cameras/{id}/frame?w=160&h=90` (the helper decodes the camera's 320 px stream at half size: a 160 by 90 BMP, painted as it is). Why no video, from the decoder budget:

- The set has two hardware video windows (MAIN and SUB0); Home's pipeline holds MAIN. A window over Home retimes the panel from 1080p25 to 100 Hz (`vwin-refresh-result.md`).
- The compositor's CPU-decoded textures are `MAX_TEXTURES` 2 (H.264 up to 1280 by 720 at 30), and the helper plays one live session at a time.
- The bridge serves at most 8 still streams (`scrypted/stills.rs`), and lets one go 3 s after its last ask.

The list therefore uses no decoder and no plane, and at most 8 still streams (7 are asked for: five rows in view and one beside them). The focused row is asked again 125 ms after each picture with `fps=8`, the other rows in view after 333 ms with `fps=4`, the row beside the view after 1 s. While the focus rests on the camera button (350 ms, for at most 10 s) the first four rows are asked for at 1 s, so the sheet opens on pictures. Nothing is asked after the sheet closes, and an answer still on its way is dropped undecoded.

## Measured on the host

`tests/host_budget.py --surfaces camera-popup` (120 Hz, 8 cameras, stills from `tests/camera_popup_host.py`): 760 frames, most layers in a frame 37 (4.6 ms of the 8.3 ms), work at most 5.2 ms, 99th percentile 4.5 ms, no frame over 80 layers. Growing out of the button 28 layers (4.0 ms), walking down past the fifth row 34 (5.2 ms work), a held Up 37 (4.7 ms), closing 28, opening again 33. The Home walk's status bar window went from 36 layers to 27 (the buttons are flat glass now).

First fresh picture of each row from the opening (`camera_popup_host.py first-picture`, a helper that takes 150 ms to start a camera's stream and 20 ms after, host load about 30): opened straight after the focus reached the button 421, 526, 672, 881, 1071 and 1263 ms for rows 0 to 5; opened after the focus rested 1.2 s on the button 0 ms for rows 0 to 3 and 449 and 490 ms for rows 4 and 5. The focused row was asked 4 to 5.5 times a second, rows in view 2.1 to 2.8, no ask after Back, no refusal for a ninth stream.

## Known limit

The shell's helper client makes one request at a time (`host/http.rs`), so a row's first picture waits for the rows asked before it (the climb from 421 to 1263 ms), and a camera that is not prebuffered and takes seconds to give its first picture holds every row behind it. The resting prefetch hides this when the focus rests on the button. A pool of two to four request threads in that client would flatten it; it is shared with every screen, so it is not part of this change.

## TV check

Nothing here is accepted until it is checked on the set. From Home with cameras linked:

1. Right from Search onto the camera button, rest 1 s, OK: the sheet grows out of the button and the first rows show pictures at once. Count the time to a picture on rows 5 and 6.
2. Down past the fifth row and back with a held arrow: the column moves by the focus's spring, the pictures keep coming, no row jumps.
3. OK on a row: the viewer opens on that camera. Back returns to Home with the focus on the camera button.
4. Back in the list: it closes into the button; `ctl shell` reports `panels.cameraList.asking` empty, and the helper's open streams fall to none within 3 s.
5. The frame gate with the list open and scrolling, and `ctl refresh`: the panel stays at 120 Hz.
6. Settings, Home & look, Layout of pop-ups, Status bar: turn Full width on and move the buttons left, middle and right. The glass runs edge to edge of the cards' grid; the camera list, the notification drawer, Quick Settings and Who's watching still grow out of their own buttons and sit under them.

## Where the bar sits

The pill is centred along the top by default and is an item of the Layout of pop-ups editor (`layout.overlays.statusbar`: an anchor and an offset along the bar's row; `placement/` zone `bar`, one row high, so it only moves along it). Home reads it from its theme each time it lays the bar out. Every popup of the bar grows out of the frame of its button as that layout gives it when the popup opens, and its body sits under that button: the camera list, the notification drawer and Quick Settings are 560 wide with their middle under the button's middle and their top just under the pill (`quick_layout::dropdown`), kept 40 px from the screen's sides (against a side where the button is nearer to it than half the sheet) and 56 px from the foot, shorter where the room under the pill runs out; their columns scroll in what is left (the camera list shows five rows and a half, scrolling). Quick Settings' PIN pad at a large text size needs the sheet's whole height and keeps the top margin then. Who's watching is a full page that grows out of the profile chip. Nothing is estimated per anchor.

Host check: `tests/camera_popup_host.py pill` renders the pill at left, centre and right and the popups opened from it (`pill-*.png`); the shell test `every_popup_of_the_bar_grows_out_of_its_buttons_laid_out_frame_wherever_the_pill_sits_and_its_body_stays_on_the_screen` moves the pill (left, centre, right and a custom offset) and asserts each popup's origin is its button's centre, its body's middle is under the button's (or against a margin of the screen), the body starts under the pill and is whole on the screen, and that the camera list still scrolls.

Full width is a separate switch of the same editor (`layout.overlays.statusbar.full`, a boolean beside the anchor and offsets, read each time; the registry's `statusbar` item is the only one that `stretch`es). With it on the glass is the whole width of the bar's zone, which is the cards' grid's width (the safe area's left edge to its right), at the pill's height, and the buttons stay grouped in their box where the anchor and offset put them inside the glass: left, middle, right or moved. `Registry::glass_of` is the glass and `rect_of` the box (`Home::status_pill` and `status_group`); the buttons' frames, which every popup, the focus and the D-pad read, come from the box and so do not depend on the glass. In the editor the switch is the first button of the panel when the status bar is chosen, the stand-in is the glass with the buttons drawn over it, and the keys and the pointer move the box. The host test checks the glass's edges against the grid's and the dock's at 100 % and 200 % text for every place, the shell test runs the popup check with the glass spread as well, and `tests/camera_popup_host.py pill-full` renders it (`pill-full-*.png`).
