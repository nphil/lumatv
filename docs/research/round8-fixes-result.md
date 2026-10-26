# Round 8 fixes (RoundFixes, 2026-10-26)

## Resume here

All four items are committed. Left for TvRound10, on the set: the frame bars of the quick panel and the input picker (the gate's
first frame <= 50 ms, burst end <= 180 ms, no interval over 20 ms, p90 < 3 ms). In order:

1. `python3 docs/research/perf/gate.py --surfaces quick,picker --input-key 241` (Home in front, receiver muted as in round 8).
2. The picker's close burst (375 ms, 19-20 animated frames, 1 gap, in all three cycles of `round8/tv-2026-10-25/gate1.json`) has no
   cause that the QML explains: the sheet is hidden 150 ms after Back and nothing of the picker animates beyond it. If it is still
   there after this change, the `full` captures of the first three cycles (`gate-r8a-picker-m0`) say which window or client draws
   during the 350 ms (candidates: LG's own input-picker view closing, Home's dock tile lift after its focus rest).
3. The quick panel's one first frame of 125 ms (cycle 2 of 3 opens; the other opens were 25-42 ms) is a stall before the first frame;
   no code path of the panel is different between cycles. Look at that cycle's capture.
4. `panels/AppsGrid.qml` start-up TypeError (round 9, `page.rail[parent.at].sub`, Hidden row): the row is guarded for a rail that is not
   computed yet; the log of the next start must not have it.

## What was wrong and what changed

1. Widget editor, OK on Save in "Save changes?" (`012eaf2`): in `LumaHome.confirmAnswered` the line that hands an `edit-` answer to the
   editor had been left as the body of the uninstall condition, so the answer was never delivered. Fixed; `tests/test_widgets.py` has the
   case (Back, Right, OK writes layout.json and leaves the editor).
2. Apps library (`34bfc16`): the rail's rows were 80 px slots for rows that are 88 px tall and can grow when a detail wraps; the three-title
   details ("GeForce NOW, Punktfunk, Moonlight") wrapped into the next row. The slots now follow the row's height and the details are counts
   ("4 apps"). The bar's clock was a `QtObject` read once at build; it is a page property read again every time the library is shown
   (`Timer.triggeredOnStart`). Test: `test_shell_host.py`.
3. Settings (`34bfc16`): the four `4:4:4 Pass Through` rows carried their port as the second line, which "Not available now" replaces
   when the row is dimmed; the port is in the label now ("4:4:4 Pass Through (HDMI 1)"). Bluetooth Devices listed the service's four
   slots; the empty ones (`isPaired: false`, address `N/A`, name "Not Paired") are skipped, and the offline fake answers in the TV's own
   shape (an array of four slots) so the test would catch it. Time & Date's value was the page's build time for the same reason as the
   clock above. The "Vision" group label scrolling off was not touched (not in the assignment's list of fixes; it is SettingsCoverage's).
4. Quick panel and input picker (commit below):
   - Slide 150 -> 110 ms in both directions (`PanelMotion`). On the set the first frame came 41-42 ms after the gear tap or Back and the burst ended
     one frame after the slide: 41 + 150 + 17 = 208 ms (measured 192-208, 242 once). 41 + 110 + 17 = 168 ms.
   - Nothing that rebinds the rows is asked for while the panel slides: the quick panel's picture-mode, input and sleep-timer readings and
     the picker's input and sound-output readings start 170 ms after the panel is told to open (after the slide plus 60 ms, so the
     frame they cause is outside the 50 ms the burst rule chains over). The picker follows the input in front if no key has moved its choice.
   - The receiver tile keeps two lines for its second line, so its answer ("Not answering" to the reading) no longer changes the
     row's height and moves the rows below mid-slide (the helper's reading ages out between openings, so every opening of the gate
     had that change).
   - The picker no longer starts its focus-rest timer on Back (the lift of the focused tile was restarted on the way out).
   Offline timing (software GL) cannot confirm the frame bars; the tests check the structure: the slide is over 200 ms after opening and
   no reading is asked for before the slide ends, the receiver's row height is the same before and after its answer.
