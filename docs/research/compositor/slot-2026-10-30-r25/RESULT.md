# Round 25 (TV slot 2026-10-30)

Installed forge 91589d0f (contains ea61f3df), persistent, helper restarted. Receiver muted during the checks, restored to 0.52 unmuted.

| Check | Result |
|---|---|
| Camera 240 pop-up with Home in front | pass: `vwin: claim camera-popup granted a texture (no window: texture-only)`, claim `slot texture`; osd_irq 121 before, during and after; no `the panel refreshes every 20.00 ms now`; helper: first picture 177 ms, 26.5 pictures a second |
| Pop-up over YouTube (SUB0) | pass: claim `slot sub`, `SUB0` 1280x720 beside YouTube's `MAIN` 3840x2160 at 60, still playing and front; osd_irq 121 to 124 |
| Card replacement, camera 103 (person, then doorbell 4 s later) | pass: one claim held across both cards, `vwin: claim camera-popup released` once when the second card ended; helper: `live for pop-up 4 ... ended with no picture shown`, then `live for pop-up 5` played 16.6 s at 16 a second |
| Full view over YouTube, Back | pass: `cameras-full` granted on `MAIN`, osd_irq 121; Back returns YouTube to the front with no claim. The video does not resume (YouTube is back at its home page, as in round 22) |

Pop-ups left on (`popups.on` true); `cameras.testEvents` off (`helper.json` restored). Not done: Home to the front while a card is up over YouTube.

End state: LumaTV persistent, Home in front, panel 120 Hz (osd_irq 121 to 122), YouTube closed, receiver 0.52 unmuted.
