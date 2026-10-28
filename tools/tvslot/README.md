# Round 19 TV slot

`r19_checks.py` runs the pending read-only and UI checks in one unattended slot, with LG's compositor live and nothing installed.

```
python3 tools/tvslot/r19_checks.py            # evidence in docs/research/round19/<date>/
python3 tools/tvslot/r19_checks.py --dry-run  # no TV: every command printed and syntax-checked
```

Expected duration: about 17 minutes. Planned key time and waits are 263 s, the gate's key cycles 88 s, plus about 45 captures, the
gate's capture transfers and analysis, and the 90 s the gate waits when the compositor is younger than that [INFERENCE: the
total is not measured]. `--only speed,settings,plugins,ble,banners` and `--skip` select checks; they always run in that order, the banner check
last because it puts an HDMI input in front.

## Before the slot (Main)

- The receiver is muted or as wanted: the script never touches it. Home in front, TV awake.
- For a PASS/FAIL on the patched banners, `lumatv inputcommon on` has been run; with it off the banner check is a baseline only
  (SKIPPED, LG's own look recorded). The script never switches it.
- `tv.py` is the connection helper (`$TV_PY` or `--tv-py`, default `/tmp/lt/tv.py`). The speed check uses `docs/research/perf/gate.py`
  with its own connection; Pillow is needed on this host.

## Checks

| Check | What it does | Reports |
|---|---|---|
| `speed` | `gate.py --surfaces quick,picker --input-key 241 --tag r19` over Home (the round 18 method; the Netflix and HDMI surfaces play audio or switch inputs and are left out) | each gate threshold PASS/FAIL, `speed/r19.json`, `speed/gate-stdout.txt` |
| `settings` | Opens Settings through the quick panel (OK only after the panel is seen on screen), walks System > About > Support > Privacy > App Permissions (four tabs) and Delete My Personal Information (opened, never confirmed), then Home & look > Home > Home Assistant (three tabs). Just Scan only when an HDMI app is in front at the start; no input is switched | a capture per page (`settings/*.png`, EYES: look at them), FAIL on a blank screen or a failed open, PASS/FAIL on "no value changed" (picture mode, Limit AD Tracking, Do Not Sell, theme file before and after) |
| `plugins` | Read-only GETs of the helper's loopback API (`/v1/state`, `/v1/plugins`, `plugin.<id>.cards`, `ha.link`) and one file read (`plugins/enabled.json`); no POST, no `lumatv-helper` command, nothing linked, enabled, installed or sent. Then captures of Home (the At-home lane) and the quick panel's lower tiles, with focus moves only (no OK) | helper answers, roster problem, owner's plugins and approved ids, first-party weather/news/clock present with a card each (a missing card with an empty source topic is EYES), ambient tile count, the Home Assistant plugin's state as the lane words it (PASS `Not switched on` or `Not linked`; EYES `Connected`; FAIL `Cannot run`, `Not running`, `Token refused`, `Offline`), no token or secret in `/v1/state` or `ha.link`; `plugins/plugins.json` (no address, no token) and the captures |
| `ble` | Baseline (adapter, ping, load), then two 15 s `le/startScan` runs, the second with `always`; skipped when a Bluetooth audio device is connected. Never port 6053, never ble-link | unknown 1 PASS/FAIL (first reply), unknown 2 (scanRecord filled, lengths, AD types), unknown 3 (repeats with the same RSSI, with and without `always`), leftover `luna-send`, ping during the scan; `ble/scan-shapes.json` holds counts only, no address, name or advertisement byte |
| `banners` | With `inputcommon` on: overlay mounted, 30 `.qml` files, `LumaStyle.qml`, LG dump set aside. Launches HDMI 2, 12 s capture burst on the set, classifies every frame (dark glass pill 44 px high at x 32, LG's grey pill, bare text, splash), the process pid and `"snapshot":true`; INFO key and card colour (`--no-info-key` skips it); closes the input | pill PASS/FAIL with first and last glass frame times, INFO card colour PASS/FAIL (glass or grey; corners, hairline and chips are EYES), the no-signal frames at 7 s and later (EYES), `banners/pill-montage.png`, `pill-frames.json`, key frames |

`--livetv` adds the Live TV channel strip and INFO card (plays audio: receiver muted first). The prompts ("Input Locked", the set-top box
question) are not reachable without locking an input and are reported SKIPPED.

Verdicts: PASS, FAIL, ERROR (the check itself broke), SKIPPED (reason given), EYES (the automatic part passed; the pictures need a look).
One check failing never stops the others. A moved guard value (failed units, `librdx` files, `aud_dsp_reset_count`) stops the slot after
that check. The exit status is 0 when no check says FAIL or ERROR and the end state holds.

## Start and end

`start-state.json`: power, foreground, picture mode, guard values, `inputcommon` and `inputs status`, theme file hash, the helper's
receiver read, whether LumaTV's own compositor runs. The script refuses to start (touching nothing) when the TV is not awake, a media
app is in front, or LumaTV's compositor is live.

End, in order: HDMI apps (and Live TV when started here) closed through SAM, Home in front, picture mode `filmMaker` (the one setting
written, only when it differs), `lumatv inputs status` running `[]`, the scratch dir `/tmp/r19` removed, then `power/powerOff` into Active
Standby (`--no-standby` leaves the TV on). `end-state.json` and the last line of `summary.txt`, `FINAL POWER STATE`, record the result.

## Evidence

`log.txt` first, `summary.txt`, `results.json`, `start-state.json`, `end-state.json`, then `speed/`, `settings/`, `ble/`, `banners/`.
The page calibration of the "panel seen" test (mean pixel change against two idle Home captures) is not measured on the set; when it
misjudges, the walk stops before any OK is pressed and reports FAIL.
