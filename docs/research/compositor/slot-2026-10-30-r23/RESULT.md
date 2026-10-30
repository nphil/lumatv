# Round 23 (TV slot 2026-10-30)

Installed forge 89507fcb (helper sha256 ef23f5a3b3fec25e..., compositor and ui md5 equal the build), persistent, helper running, 6054 listening. Pop-ups off for the gates.

## Frames gates (panel_refresh 59.90 and 119.90 on every surface printed)

| Build | 60 Hz | 120 Hz |
|---|---|---|
| 89507fcb | home-dpad ok; quick dropped 3; picker dropped 3; toast burst_end 1998 ms; settings-scroll dropped 4; apps-grid dropped 4; home-widgets ok | quick 7; picker 6; toast burst_end 1998 ms; settings-scroll 47; apps-grid 3; home-dpad, home-widgets ok |
| 25cb7b06 (gate judges each motion switch on its own) | home-dpad burst_over20 1; quick dropped 1, burst_over20 2; picker ok; toast burst_end 255 ms; settings-scroll burst_over20 1; apps-grid ok; home-widgets dropped 2 | quick 7; picker 5; toast burst_end 247 ms; settings-scroll 41; apps-grid 3; home-widgets 3; home-dpad ok |

The set's theme has wallpaperMotion.enabled false and no weather key; `theme get` is unchanged after the runs. Full gate output in the `gate-frames-*` files.

## Video windows and the panel rate

Pop-up test events (cameras.testEvents on, pop-ups on, from 120 Hz, receiver muted):

| Camera | Stream | Panel during the pop-up |
|---|---|---|
| 128 | 640x480, 10 pictures/s | 120 Hz (osd_irq 121/s) |
| 105 | 1280x720, 25 declared | 120 Hz |
| 106, 108 | 20 declared | 120 Hz |
| 240 | 1920x1080, 25 declared | about 100 Hz from the first frame, still 100 Hz after the pop-up ended |

Camera 240: the compositor logged "the panel refreshes every 20.00 ms now (16.67 before)"; the helper logged "Home measures 99.9 Hz with a source connected to the main window", then "wrote 1 60 to fr_rate with Home in front at 99.9 Hz, but the output measures 99.9 Hz". There was no "set back to 120 Hz". Written by hand mid-playback: `1 60` gave 101, `0 60` 100, `1 50` 101 osd_irq per second. A 20 fps pop-up afterwards (camera 108) brought the panel back to 120 Hz (16.67 ms) by itself, as a 30/60 fps video does.

Verdict: fail. The firmware ignores fr_rate while a 25 fps 1080p stream is connected, and the helper's request does not restore 120 Hz. Pop-ups left off (popups.on false).

## End state

LumaTV persistent, Home in front, panel 120 Hz (osd_irq 121/s), backlight 80 (as found), receiver unmuted 0.52, cameras.testEvents off (helper.json as before the round).
