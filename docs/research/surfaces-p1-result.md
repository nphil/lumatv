# Surfaces, phase 1 (LumaTV)

Key filter, focus rule, wallpaper tint, saved-first cards, token source and component kit,
startup warm-up, the frame gate, and the preload trim. Measured on the LG C4 (webOS 10.2.1),
2026-10-25. Plan: `docs/PLAN.md` (Rules every surface meets, Phases 1 and 8).

## Resume here

State at 2026-10-25 01:10 EDT. Milestone 1 of 5 is built, deployed and verified on the set;
the commit follows this note.

- Done: 1 (key filter, OK on release, focus rest).
- Next: 2 (wallpaper tint, saved-first weather and news), 3 (kit and tokens), 4 (warm-up and
  the frame gate), 5 (preload trim and the Home-return re-measure).
- The set: Active, LumaHome in front, dock order restored after test keys moved a tile (the
  list is `Apps, Netflix, PlxNative, Apple TV, GeForce NOW, Crunchyroll, ...`; a held OK picks a
  tile up and an OK release drops it, so test scripts must not leave one picked up).
- Tools used (not in the repo): a paramiko wrapper for the set, and `docs/research/perf/kb.py`
  (key injector with trace markers, copy it to `/tmp/kb.py` on the set).
- Deploy loop: `sh tools/build-ipk.sh`, push the ipk to `/tmp/lumatv.ipk`, `luna-send -n 4 -f
  luna://com.webos.appInstallService/dev/install '{"id":"org.nphil.lumatv","ipkUrl":"/tmp/lumatv.ipk","subscribe":true}'`,
  wait until the app directory matches the build (the install is asynchronous: an `apply` run
  one second after the call staged the previous files), then `lumatv apply` (13 s, restarts the
  compositor).

## 1. Key filter, OK on release, focus rest

What changed, in `payload/keyfilter/lumatv.js`, `payload/compositor/lumatv/` and the theme:

- The filter takes a key only while Home is in front, LumaHome is answering (a heartbeat from
  its 1 Hz timer, under 3 s old) and its handler takes the press. Any exception in the filter or
  the handler answers NextPolicy and logs one line per 5 s.
- Presses and releases are paired. A release goes where its press went, so an OK pressed on
  Home and released after an app has opened ends at LumaHome (which acts on it only while Home
  is up), and a release whose press went to an app is never taken. Repeats follow their press.
  Unit tests: `bun test tests/` (13 cases).
- OK acts on release. A press only starts the 700 ms hold timer (`motion.holdMs`); held, it picks
  a dock tile up. On any other layer a long press is an ordinary one (before, it did nothing).
  A held OK and a picked-up tile are cleared when Home is hidden or shown.
- Focus rest: the highlight (tint, edge, sheen, rim) follows every key press at once; lift,
  scale and shadow depth animate only after the focused item has rested `motion.restMs` (150 ms)
  and drop at once when it leaves. The name pill and the dim of unfocused cards follow the
  same rule; the dock's accent glow moves with the highlight. `FocusLift.qml` is the shared
  piece; `Glass.qml` takes `focusMix` (highlight) and `liftMix` (shadow depth) separately.

Checked on the set:

- Fault injection, key RIGHT on Home: with the normal filter no key reaches WebAppMgr; with a
  throw inside the filter, the key reaches it (`WaylandKeyboard::OnKey key:106` press and
  release) and one `[LumaKeys] the filter failed` line is logged; a second key within 5 s adds no
  line. A handler with no `try` at all also lets the key through on this firmware, so the catch
  is for the log, the pair record and the explicit answer, not a rescue from a swallowed key.
- OK: a press does nothing until the release; the release launches the tile; neither reaches
  the app that opens. Held 1.2 s on a tile, the tile is picked up and stays picked up after the
  release; Back puts it back; leaving Home while it is picked up and returning leaves the order
  and the dock untouched.
- Frames, `run.py` workloads `sweep` (held RIGHT 1.5 s, LEFT 1.5 s, UP, DOWN) and `dpad` (HA keys
  at 350 ms), 2 runs each, HEAD against this build:

  | | frames | GPU busy | compositor CPU | render p90 |
  |---|---|---|---|---|
  | sweep before | 224, 220 | 1760, 1775 ms | 805, 690 ms | 3.36, 2.95 ms |
  | sweep after | 150, 157 | 1195, 1271 ms | 584, 502 ms | 3.11, 2.78 ms |
  | dpad before | 324, 322 | 2597, 2589 ms | 984, 970 ms | 2.92, 2.91 ms |
  | dpad after | 243, 263 | 1955, 2110 ms | 731, 801 ms | 2.40, 2.66 ms |

  A sweep now draws one frame per key press. The harness's `dropped` count rose (6, 1 to 14, 6 on
  the sweep): it counts an animated frame that follows an idle gap, and the gap before the lift
  that starts 150 ms after the last key is one; the render time of those frames is 0.7-3.7 ms,
  so none was late to render. The gate in step 4 counts only frames inside a running animation.
