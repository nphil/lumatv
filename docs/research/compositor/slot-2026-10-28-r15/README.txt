Round 15, 2026-10-28, forge/main f67c511 (c1a2736), LumaTV's compositor live as sessions; the receiver muted through Home Assistant's MCP.
gate-full-gate-lines.txt, frames-60-full-gate.json, wlprobe-*: the full gate (--skip volume) and its evidence step; wlprobe-diff.txt against compositor/lg-wlprobe-2026-10-28.txt.
x1-reruns-gate-lines.txt: two more x1 runs. gate-120-gate-lines.txt, frames-120.json: `--only frames --refresh 120 --frames-quick`.
apps-gate/: the gate's own apps check (its app_running never matches SAM's list). apps-corrected-run/ (11 apps in one loop, app_running fixed, a 20 s window; the NVIDIA GeForce NOW web app leaves LG's
browser in front for the two apps after it), apps-plex-alone/, apps-appletv-alone/ (50 s window), apps-gfn-moonlight/, apps-plex-twice/, apps-youtube-plex/ (YouTube, Plex, YouTube, Plex): apps-result.txt
first, then each app's journal slice; apps-gate-lines-*.txt are the GATE lines of those runs (the exit status read of the last two repeats the first run's pid). plex-crash-log-r15.txt: Plex's own crash
reports of the round (3). img/: the DISPLAY capture 5 s into each run.
control/: the same launches under LG's compositor (SAM only). stray-browser.txt, browser-home.txt: Home launched with LG's browser in front, under LumaTV's compositor.
livetv/: three launches and leaves of Live TV with `lumatv transitions on` (summary.txt first). wall/: `wallpaper video` as shipped (placement refused), placement-variants*.txt: the same context with other
placements, probe-results.txt: tools/vplane_wall.py probe on every capture; img/wall*: the captures.
