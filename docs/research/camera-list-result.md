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
