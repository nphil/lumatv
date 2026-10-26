# Round 8: the TV slot script

## Resume here

`docs/research/round8/tv_round8.py` is written and tested offline (`tests/test_round8.py`, 43 tests against a simulated set); it has not
run on the set. It needs Main's go for the set and Nitin's yes for the test plugin.

```
python3 docs/research/round8/tv_round8.py --plan                 # the steps; touches nothing
python3 docs/research/round8/tv_round8.py --go --approve-test-plugin
python3 docs/research/round8/tv_round8.py --go --skip x4,logo    # any of plugin,x4,maliit,logo
```

Other flags: `--no-standby`, `--keep-logo-off`, `--plex-keys "OK s4|OK s5"` (kb.py tokens, stages split by `|`), `--plex-seconds 20`,
`--out DIR`. About 20 minutes; paramiko and the homelab's Home Assistant token as in `tv_next_slot.py`. Evidence goes to
`docs/research/round8/<date>-<time>/` (`summary.txt` first); the summary lines are PASS, FAIL, INFO or SKIP and exit status 1 means a FAIL or a
PROBLEM (something that could not be put back, printed last).

Before the slot: the set's package must be the current tree, applied (`lumatv apply`): LumaHome live, the helper with the plugin broker and
`/v1/device`, `payload/systemui/MasterBrand` with the `logo-events` record. The script checks the first and the last and says so; a
helper without the broker shows as "the helper switched it on" failing.

## What the script does

Preflight (read-only apart from waking the set): it answers and is Active, nothing of another slot runs (`ls-monitor`, X1/X2 clients, the
compositor supervisor, `slot.sh`, `kb.py`, the card app, plugin scripts; the ambient probe's lock or `vt_devmode`), Home is in front. A set it
woke is put back to sleep if the run is refused. `/tmp/round8.lock` on the set keeps two runs apart. Then the receiver is muted through Home
Assistant and read back from the helper's direct reading (Home Assistant's entity is stale after an unmute, so it is only the fallback); if
that cannot be shown, nothing that plays or cycles power runs.

| Step | What it does | PASS means |
|---|---|---|
| plugin | `00-home` baseline. Test plugin (below) on. Lane screenshot, then UP/OK/OK from the dock's first tile (Home's focus path into the At-home lane) to run the chip's action. A note posted, toast screenshot. Quick panel by the door `quick` (screenshot, tile action through `POST /v1/plugins/round8/actions/ping`, screenshot). Settings by the door `settings`, UP x10 then DOWN x4 to the fifth rail section (Ambient light), screenshot, the helper's `base.ambient` and `/v1/ambient`. Drawer by the door `drawer`, screenshot. Each surface has the compositor's log since it was asked for, checked with `gate.py`'s failure patterns | each surface differs from Home at rest by more than its floor (3 %, 10 % for Settings) and is not one flat colour; the plugin's card, tiles and both notes are helper topics; the plugin was told of both actions; no failure line in any log window |
| maliit | `find`, `server.conf`, `ls -R`, the process's maps, strings of `libplugin-global.so`; every QML/JS/qmldir/json file under a path with `maliit` tarred and copied to `maliit/tree/` with md5sums | the tree is copied; with no QML on disk it is an INFO that the keys are compiled in (a rebuilt resource is Nitin's decision, `lg-restyles-result.md`), not a FAIL |
| x4 | Each segment under its own `ls-monitor -j -e` (the repo's `x4-monitor.sh`, its capture moved to `/tmp/round8/x4`), gzipped, with `x4-analyse.py`'s census beside it. Picker: `showInputPicker`, screenshot, `systemUIManager/closeByAppId`. PlxNative: launch, the keys, up to three stages, playback held, closed, Home. Standby/wake: `powerOff`, Active Standby, 20 s, wake. No PIN, no Netflix | both picker calls in the capture and Home after; PlxNative in front, a `setDisplayWindow`/video window by it, closed and Home seen before the capture ends; the standby rounds `request power off`, `prepare power off`, `prepare power on` pushed to the compositor's subscription |
| logo | `lumatv logo on` first if it was off (before any power-off of the run; put back at the end), then the standby cycle's `logo-events` lines | `power-off logo N launch` and `power-off answered tvpowerd` are new lines; INFO gives the gap and the `[Logo]` lines. It reuses the x4 standby cycle; alone it runs its own |

End, always, in this order: monitor stopped, test plugin off and the roster put back byte for byte, PlxNative closed, Home in front and seen, PlxNative
not in SAM's running list, logo switch back, the receiver unmuted (not if Home or the quiet is not confirmed: that is a PROBLEM and it
stays muted), `/tmp/round8` removed, Active Standby. Picture Mode is read before and after, never set.

## The test plugin (Principle 0)

Switching a plugin on in the helper's roster is installing software the TV runs, so it is the owner's act: without `--approve-test-plugin` the
step draws only what the helper's own plugins give (the ambient tile) and the lane and drawer show their empty state. With it, the script adds
`round8` to `<data>/plugins/enabled.json` (the file as it was is kept and put back; a roster that did not exist is removed again), starts
`plugin/plugin.py` as plugin user 20000+n (the first one the roster does not use) with `os.setuid`, and takes both away at the end. The plugin has
cards, tiles and notify permissions only, no network, no leases; it speaks the real wire (every message it sends parses as the plugin-api crate's
own types, checked once with a throwaway Rust program, and `tests/test_round8.py` runs it against a stand-in broker).

The approval is the SHA-256 of the manifest's consent document, rebuilt in Python (`consent_document`, permissions in the library's enum order,
not the author's) and pinned in the test to what LumaShow's crate gives: `8bb95c0e...97d5`. If `plugin.json` changes, recompute it with
`plugin_api::Manifest::parse(bytes)?.consent_document()` and update `RUST_CONSENT` and `RUST_APPROVAL` in the test.

## Not known until the first run

- **PlxNative's way to playback.** Nothing recorded says what its first screen offers. The default keys are OK, wait, OK, wait, OK (three
  stages, each checked for the video window); `after-keys-N.png` shows what each opened, `--plex-keys` changes them. A FAIL here is the keys,
  not the capture. Playback is judged by the live `getForegroundVideoWindow` naming the app or by a placement call from it in the capture.
- **The lane's focus path** (UP, OK, OK from the dock's first tile) is read from `LumaHome.qml`'s key code; `01b-lane-focus.png` shows where the focus went.
- **The door**: a layer named in `/tmp/lumatv-layer` is picked up by a 2 s poll (3.2 s waited). The `settings` door has to be emptied to close (Back
  leaves the door's own state); the script does it. A reboot clears the file.
- **Screenshot floors** (3 % and 10 %) are chosen, not measured; the numbers are in each line so the first run calibrates them.
- Toasts for plugin notes, the lane's look and `01-lane.png` against the mock are by eye (INFO lines give the pixel share).
- The compositor's own sender side during standby (pass 2 saw none of its pushes after a restart) is counted and reported, not asserted.

## Files

`docs/research/round8/{tv_round8.py, plugin/plugin.py, plugin/plugin.json}`, `tests/test_round8.py`.

CHANGELOG line for the main agent: *Round 8's TV slot script (`docs/research/round8/tv_round8.py`): plugin drawing (lane, quick-panel tiles, the ambient page, the drawer) through a test plugin on the helper's plugin API and Home's development door, the X4 remainder (input picker, PlxNative playback, standby and wake), the maliit tree copied read-only, and the power-off logo record, with the receiver muted before anything plays and put back after Home is; tested against a simulated set.*
