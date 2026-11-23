# Task manager: Back hold, running apps, stats, system actions

A Back hold opens the task manager from anywhere: Home, an app, a game, a hung app. It is the app switcher (`switcher.rs`, `POST /v1/switcher`, topics `apps` and `switcher`; the identifiers keep that name) grown into a recent-apps screen with the running apps as cards, a stats strip, and a System menu. A second door stays: a second press of Home (`remote.appSwitcher`).

Three layers, each able to work when the one above it cannot:

| Layer | Process | Draws | Works while |
|---|---|---|---|
| Full screen | the Rust UI client (`rustui/ui`, `panels/switcher*.rs`) | everything: cards' chrome, stats, System sheet, confirms | the UI answers |
| Basic screen | the compositor (`lumatv-compositor`) | pictures, plain text and rows, from the palette the UI last sent | the UI is hung or gone |
| Rescue | `lumatv-sm` (`compositor/sm`) | nothing; acts on a long Back hold, logs, toasts | the compositor is hung |

## Back hold

- A Back press is withheld from the app. Released before `BACK_HOLD` (600 ms), it is replayed to the app as a tap (press then release, in the call that sees the release). Held to 600 ms, the task manager opens and the app sees nothing of that press: not the press, not the repeats, not the release.
- While the task manager is open, Back goes straight through to it (it closes, or goes back one step; no delay). Over the on-screen keyboard, over Home and over an app that does not answer, Back is withheld like any other time: a tap reaches whoever has the key at the release, a hold opens the task manager.
- Back held 1 s used to close the foreground app (`SystemRequest::Exit`, LG's long-press parity). It is gone; the task manager's Quit is the way.
- A single Back on Home does nothing, and is swallowed: no key reaches the Home stub, no app manager call. `remote.recents` (Back from the top of Home opens the switcher) is retired with its setting, code, docs and tests.
- 600 ms sits under LG's own 1 s hold and well above a tap; it is a chosen value, to be confirmed with held keys on the set. The Home escape stays at 650 ms and is unaffected.

## Contract: compositor to UI (topics)

`apps`: array, most recently in front first, possibly empty. Every card app the app manager lists as running, minus Home, LumaTV's own apps and non-card windows, whether or not it has a window:

```
{ "id": "com.example.app", "title": "Example",
  "picture": "live" | "thumb" | "video" | "none",
  "w": 1920, "h": 1080,            // the picture's size; 0, 0 when there is none
  "state": "" | "quitting" | "stuck",
  "fg": false,                     // in front when the task manager opened
  "hung": false,                   // the process is stopped, or gave no sign of life
  "pid": 1234 }                    // 0 when unknown
```

`live`: the window's own buffer, drawn by the compositor in the card's rectangle. `thumb`: the last frame the compositor kept when the app left the front (the app released its buffer on backgrounding). Both are drawn by the compositor in the rectangle the UI places; the UI treats them alike and draws chrome over them. `video`: on the video plane, nothing to draw: the UI shows its icon tile. `none`: no picture yet: the same tile.

`switcher`: `{ "open": bool, "from": id | "", "n": seq, "mode": "ui" | "fallback" }`. `mode` is `fallback` while the compositor draws the basic screen itself; the UI draws nothing of the task manager then, and stays in `fallback` until the task manager closes.

`stats`: pushed only while the task manager is open: first within 500 ms of opening, then once a second. Never pushed or sampled while it is closed.

```
{ "n": seq,
  "cpu": [pct, ...],                       // per core, 0..100, core order (3 on the set)
  "mem": { "totalKb": N, "availKb": N, "cachedKb": N },
  "gpu": pct | null, "tempC": N | null, "mhz": N | null,
  "refresher": bool }                      // the panel's pixel refresher runs: Restart TV is refused
```

## Contract: UI to compositor (`POST /v1/switcher`, as today: `{"op": ...}`)

- `open`, `close`, `activate {id}`, `quitall`: as today. `open` also succeeds with no app running (the stats and System stay reachable).
- `quit {id, force?}`: a clean quit as today (window `close`, app manager `closeByAppId`, `reason: windowPolicy`). A `hung` app is killed at once (a stopped process never acts on SIGTERM). `force: true` kills an app that is `stuck` (did not quit in 4 s).
- `layout`: as today (`cards`, `scrim`, `ms`, `reduced`), plus `palette`: `{"panel": [r,g,b], "text": [r,g,b], "muted": [r,g,b], "accent": [r,g,b], "danger": [r,g,b]}` (0 to 255), the theme's tokens for the basic screen. The compositor keeps the last one; a Rose Pine default stands until it arrives.
- `system {action}`: `restart-lumatv`, `lg-home`, `reboot`. The UI asks for a confirm first (initial focus on the safe choice; Back cancels); the compositor does not. The reply to the POST carries the result: `{ok: true}` when the action was taken, `{ok: false, error: "refresher" | "busy" | ...}` when it was refused (`reboot` while the panel's pixel refresher runs: checked when the POST arrives, when the action runs and just before the bridge line is sent). The compositor closes the open apps first (a hung one is killed at once, the others get 4 s), then sends the bridge line; with no effect after 20 s it shows the notice "<action> did not start."
- `beat {n}`: acknowledges `stats.n`. Armed only for a UI whose `layout` carried a `palette` (an older UI never acks and is not judged by it). No ack for 2.5 s while open, no `layout` within 600 ms of the first `switcher` topic (which waits up to 120 ms for the app manager's answer), or the UI going away, puts up the basic screen. A UI that is connected but silent is also restarted (`shell_rust_restart`, as `ctl shell restart`) so that its last frame no longer covers the basic screen; the basic screen then stays until the task manager closes. It is the theme's panel colour at 0.96 alpha over the dimmed apps and takes every key but Home, Volume and Mute; each Back press while the task manager is open is a heartbeat to `lumatv-sm`. Idle: 45 s without a key closes the task manager, in both modes (a still picture is not left on an OLED).

## System actions

Each runs from `lumatv-sm`, the one process that knows how to end and start LumaTV; the compositor sends it a bridge line (`{"kind": ...}`, as `fallback` today).

- **LG Home** (`fallback`, existing): the escape, no failure counted; LG's compositor and Home come back, LumaTV stays off for the boot.
- **Restart LumaTV** (`restart`): the compositor quits every app cleanly and waits up to 4 s, then sm runs the installer's own `lumatv restart` detached from its unit (`systemd-run`), which also brings Home back. No failure is counted.
- **Restart TV** (`reboot`): refused while the pixel refresher runs (`/mnt/lg/cmn_data/pnwash/state`, first word `2`; helper `GET /v1/panel/refresher`); the compositor checks for the UI's sake, sm checks again and is the only place `reboot` is run. A plain `reboot` returns powered on; the luna `machineReboot` call drops to standby and is not used.
- `{"kind": "taskmgr", "open": bool}`: the compositor tells sm when the task manager opens and closes.

## Rescue (sm)

sm already reads the remotes' evdev nodes itself (Home held 10 s escapes), pings the compositor every 500 ms and falls back to LG Home after two missed beats (1.5 to 2 s; a stopped compositor ignores SIGTERM and is killed after a 1 s reap, so about 3 s to LG Home), 6 s of frames never shown, or 45 s with no window; each writes a crash bundle. A compositor frozen outright is therefore already recovered and logged. Added, additive, nothing about those policies changes:

- Back (codes 158 and 412) held 3 s with no `taskmgr` line (open or closed) since 0.25 s before the press, armed only once the compositor has answered pings for 10 s: sm restarts LumaTV (the `restart` flow, no failure counted) with a crash bundle that says why. Held 10 s on: LG Home. The compositor therefore sends `{"kind": "taskmgr", "open": false}` for every Back it consumes at press (the task manager closing on Back, the keyboard stepping back), or a held Back there would restart LumaTV.
- A toast through LG's notification service after LG Home returns, naming what happened.
- No timer, no poll while no Back is down; `RLIMIT_CORE` 0 (`no_core_dumps()` exists).
- Not done: feedback by sm drawing (DRM master after the compositor is killed): untested and risky. A reboot is not offered by a hold.
- sm's self-tests run on the host (`lumatv-sm json tests: ok`: the Back state machine on a fake clock, the refresher guard, the bridge lines, the toast). Still to run on the TV: SIGSTOP of the compositor (owner's go-ahead first), Back held 3 s, Back held 10 s with `tools/lumatv` made non-executable, `luna-send createToast` as root from the detached shell, Back codes 158 versus 412 on the real remote. `lumatv restart` can refuse (refresher) or wait (an active recovery or the mutation lock, up to 180 s); sm learns of that only from the tool's log, and the 10 s escape is the backstop.

## Stats

Sampled by the compositor once a second while open, nothing while closed; sources, all read in under 0.1 ms together: `/proc/stat` (per core, saturating deltas), `/proc/meminfo` (`MemAvailable`), `/sys/kernel/debug/mali0/dvfs_utilization` (`busy_time`, `idle_time`), `/proc/lg/pm/temperature` (°C) and `/proc/lg/pm/frequency` (kHz), the pnwash state. `ctl switcher` reports `stats.samples` and `stats.us` (time spent sampling) so the cost, and zero while hidden, can be read.

## Pictures

Apps release their buffer when they leave the front, so the windows alone cannot make the list or its pictures. The list is the app manager's `running`, refreshed at open and by life events. When an app leaves the front (`State::freeze_app`'s call sites, before its picture is released) the compositor keeps a 16:9 thumbnail on the GPU (at most 12, about 1 MB each, least recently used out; dropped with the app). A card's picture is the window's live buffer when it has one, else the thumbnail, else none.

## LumaGame hook

A game's picture is on the video plane and cannot be sampled, and a game should end its session cleanly. A small hook, per app, over a unix socket the app creates at `/tmp/<app id>/hook.sock` (JSON lines; absent means the app has no hook):

- `{"op": "quit", "grace_ms": 3000}`: end the session cleanly (save, release the pad and the video plane) and exit. Reply `{"ok": true}`. After the grace the compositor falls back to `closeByAppId`, then to a kill.
- `{"op": "snapshot", "path": "/tmp/<app id>/last.rgba", "w": 640, "h": 360}`: write the last frame as raw RGBA at that size within 300 ms. Reply `{"ok": true}`. Asked when the app leaves the front and when the task manager opens.

The compositor does not call the hook yet. Until it does, a quit is the clean window close, then `closeByAppId`, then a kill, and a game's card shows its icon tile (its picture is on the video plane). LumaGame can build to the interface above.

## Checks

Host: unit tests for the Back hold (tap replay, hold, repeats, keyboard, Home), the list reconcile, hung and quit paths, the System ops, the stats sampler; UI scene screenshots at 100 % and 200 % text, dark and light. TV: the frame gate at 120 Hz with the task manager opened and closed over Home and an app, a SIGSTOPped app, stats cost while open and while closed, `video-check`.

## Status (2026-11-23)

Built and tested on the host; not yet installed on the TV and not yet pushed: the Back hold (600 ms; a shorter press is replayed whole at release), the running-app list (windows with a surface, plus app-manager apps), cards with a live, a kept (`thumb`) or no picture, stats only while open, the System sheet (Restart LumaTV, LG Home, Restart TV, each with a confirm; Restart TV refused while the refresher runs), the compositor's basic screen with the heartbeat to `lumatv-sm`, and the rescue in `lumatv-sm` (Back held 3 s with no heartbeat: restart plus a crash bundle; held 10 s: LG Home). `remote.recents` is retired everywhere and a single Back on Home does nothing. The old 1 s Back-hold exit is gone.

Host results: compositor lib 559 tests and the `back_to_home`, `ime_own`, `keys`, `overlay_keys`, `switcher` and `page_alpha` suites pass, clippy clean; UI 1876 tests pass, clippy clean.

Focus look: raised glass, no outline and no underline (Lucent, 2026-11-23). The card in focus is the larger, unveiled picture under the focus wash, the sheen and the lit rim; the page mark under the cards is short and dim. The pictures are the compositor's, so they do not rise or cast a shadow from the UI.

Video holes: `tests/page_alpha.rs` pins that a client's video window clears every layer below it inside its rectangle (and nothing outside it); the compositor needed no change for that.

Open, on the TV: the install order (UI by live swap; compositor and `lumatv-sm` together, staged for standby; `lumatv-sm` never alone), the hung-app test, the frame gate at 120 Hz, stats cost open and closed, the 600 ms threshold with held keys, `video-check`. A SIGSTOP test of the compositor itself needs the owner's go-ahead. The compositor does not call the LumaGame hook yet.
