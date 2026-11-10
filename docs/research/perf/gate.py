#!/usr/bin/env python3
"""Per-surface frame gate: the PLAN's "Rules every surface meets", measured on the set.

    gate.py [--surfaces a,b] [--cycles N] [--soak] [--json OUT] [--tag T]

For each surface in SURFACES the gate injects its open and close keys with /tmp/kb.py (exact
timing, markers at the instant of the key), captures with capture.sh, cuts the trace into one
window per marker with analyse.py and checks every window against the thresholds below:

  p90 animated-frame render      < 3.0 ms, over every frame of the surface's cycles pooled
  animated frames over 20 ms     none inside a running animation under LumaTV's compositor; under
                                 LG's, at most LG_COMPOSITOR_DROP_SHARE of them (`dropped` counts those, and
                                 `burst_over20` the intervals inside the burst a key starts); the
                                 first animated frame after an idle gap is late by construction.
                                 Under LumaTV's compositor both are read off its ring: an interval
                                 after a frame with a motion running after it (a still screen is none)
  first frame after the key      <= 50 ms
  burst end after the key        <= 180 ms (opens and closes)
  compositor RSS over N cycles   flat, see rss_growth()
  /var/log/messages              no line matching LOG_FAIL_PATTERNS
  trace complete                 every window starts inside the trace: no CPU's ring overran first (16 MB each, capture.sh)

A surface whose key is not consumed (no frame in its window) fails. The marker sits at the edge
the surface acts on: the press by default, the release for `act: 'release'` (LG's Quick Settings
opens on the release of the settings key, so a marker at the press would count the 50 ms the
injected tap is held). `still` switches Home's own motion off for the surface (the wallpaper's 10 Hz tick and the weather
card's sprites: a frame of either could otherwise be taken for the response, and a card that draws every refresh keeps every burst of frames
from ending; the user's settings are put back). `delivery` is for
a surface a luna call opens (toast, alert): its first frame is judged from the moment the shell was
handed the notification, because the spawn, the hub and the service before that are not the
compositor's: LumaTV's compositor marks the trace itself (`Q:NAME-delivered`, when the service's push
reaches it), under LG's it is the return of luna-send (kb.py's `Q:NAME-sent`). Every window of a
surface the compositor shows a panel for on a key (`Q:NAME-show`) says in `show_ms` when it was asked.
The run starts with 3
open/close cycles per surface in `full` captures (scheduler and GPU events, for root-causing a
miss); then --cycles (default 10) cycles in `basic` captures (frame events only), whose 1 s
sampler gives the compositor RSS. --soak is the release run: 500 cycles, split into traces of at
most SOAK_TRACE_MAX so the ring never overflows. Exit status 0 only when every check passes.

Home must be on screen when the gate starts. The set needs python3 for kb.py; kb.py and
capture.sh are copied to /tmp on the set at the start, so nothing else has to be installed.
The gate also measures the panel (the display's vsync interrupts, `DE2VSYNC`) before it starts and stops when it does not run at --refresh: the TV
re-times it to 100 Hz after a camera popup or a video window, and a run at the wrong rate judges every window by the wrong refresh. Under LumaTV's
compositor it also stops when the bundle's `ctl` in /tmp/lumatv does not answer, when the app switcher is up after its wake, and, with --refresh other
than 60, when the compositor's refresh is not held at it (see unfit()).

Screens walked with the D-pad and apps launched from Home (`settings-scroll`, `apps-grid`, `home-widgets`, `switcher`, `keyboard`,
`transitions-netflix`, `transitions-settings`; named with --surfaces, under LumaTV's compositor, Home at rest in front) run on the owner's own Home
layout, which they never ask to be reset: they reach Settings through the quick panel, a dock tile by its number key (its place read off the dock's own
list) and Home's top by keys that clamp. They put Home's focus in a known place before every cycle (`prep`, a window of its own), are timed from the key
that enters the screen and hold the whole walk in the open window: the same bars as every surface, the burst-end check off, and the key-to-first-frame
check off for a launch, whose first frame is the app's resume. A surface that needs a setting for its run (`switches`: the app switcher, the number
keys, smooth transitions) sets it, reads it back at the end whatever happened and fails if it is not the owner's again;
`transitions-*` also read the compositor's counters (`ctl transitions`) around the measured cycles. Errors that another app's own QML logs (LG
Search's) are not counted against a surface; at 120 Hz the shell's `[FrameMotion]` lines are read from the same log and printed as `slides:`.
tv_gate.py runs them in its frames check: all of them by default, the ones that start no app (and change a setting only when the number keys are off or Quick Access on) with --frames-quick.

Adding a surface is one entry in SURFACES (see the field list there).
"""
import argparse
import contextlib
import json
import math
import os
import re
import shlex
import signal
import sys
import time

import kwait
import ringsum

HERE = os.path.dirname(os.path.abspath(__file__))
# The tool is not on the set's PATH.
TOOL = '/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv'
# Home's own motion, by theme path: the wallpaper's tick and the weather card's sprites. The gate's `still` surfaces switch each off that is on, and put each back
# as the owner had it (one that is already off is left alone, whatever the other is: a Home with its wallpaper off still draws the weather every refresh).
MOTION_SWITCHES = ('wallpaperMotion.enabled', 'weather.animated')

# What puts a film or an input in front of the surfaces over playback. The set's commands: luna-send on the set, kb.py as the gate copied it.
SAM = 'luna://com.webos.applicationManager'
NETFLIX_FILM = '{"id":"netflix","params":{"contentTarget":"m=https://www.netflix.com/watch/80141336&source_type=4"}}'
NETFLIX_SETUP = [f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"netflix\"}}'", 'sleep 3',      # a fresh start: the deep link plays only into a launch
                 f"luna-send -n 1 {SAM}/launch '{NETFLIX_FILM}'", 'sleep 12',
                 'python3 /tmp/kb.py OK', 'sleep 25']                                           # ENTER plays it; 25 s later it is in the middle of the film
NETFLIX_TEARDOWN = [f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"netflix\"}}'", 'sleep 2',
                    f"luna-send -n 1 {SAM}/launch '{{\"id\":\"com.webos.app.home\"}}'", 'sleep 6']
NETFLIX_FULL_BUDGET_S = 5.0     # seconds of one full capture over the film (the ring holds about 7 s there)
HDMI_SETUP = [f"luna-send -n 1 {SAM}/launch '{{\"id\":\"com.webos.app.hdmi{{HDMI}}\"}}'", 'sleep 8']
# LG's Content Store, cold, with its first page up (the loader, then LG's cloud page, the shelves' pictures): the page the theme
# (`lumatv store on`) restyles. A fresh start each run, so the page's network phases are part of what the setup waits out.
STORE_ID = 'com.webos.app.discovery'
STORE_SETUP = [f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"{STORE_ID}\"}}'", 'sleep 3',
               f"luna-send -n 1 {SAM}/launch '{{\"id\":\"{STORE_ID}\"}}'", 'sleep 20']
STORE_TEARDOWN = [f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"{STORE_ID}\"}}'", 'sleep 2',
                  f"luna-send -n 1 {SAM}/launch '{{\"id\":\"com.webos.app.home\"}}'", 'sleep 6']
# LG's Search app (`com.webos.app.voice`, resident, an overlay over Home), cold, with its keyboard hidden by Back so that the keys
# move focus: what `lumatv voice on` restyles. The first launch after a close is dropped by the app inside 300 ms, hence the sleep.
VOICE_ID = 'com.webos.app.voice'
VOICE_SETUP = [f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"{VOICE_ID}\"}}'", 'sleep 3',
               f"luna-send -n 1 {SAM}/launch '{{\"id\":\"{VOICE_ID}\"}}'", 'sleep 6', 'python3 /tmp/kb.py BACK', 'sleep 1.5']
VOICE_TEARDOWN = [f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"{VOICE_ID}\"}}'", 'sleep 2',
                  f"luna-send -n 1 {SAM}/launch '{{\"id\":\"com.webos.app.home\"}}'", 'sleep 4']
HDMI_TEARDOWN = [f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"com.webos.app.hdmi{{HDMI}}\"}}'",
                 f"luna-send -n 1 {SAM}/launch '{{\"id\":\"com.webos.app.home\"}}'", 'sleep 6']

# The surfaces that walk a screen (settings-scroll, apps-grid, home-widgets, switcher, keyboard-*) and the ones that launch an app (transitions-*).
# None of them counts on where the owner put Home's cards, status bar items or dock tiles: the layout is the owner's and nothing here asks for a reset.
# - Home's top (`HOME_TOP`, home-widgets): Up eight times leaves the dock and the cards and stops on the status bar (Up does nothing there), Left six
#   times is its first item.
# - LumaTV's Settings (`QUICK_PANEL_FOOT`): the gear's tap is the compositor's, which shows LumaTV's quick panel over whatever Home has focused; Down
#   held down its list ends on its "All settings" button (Down does nothing there), and OK on it opens Settings with its rail focused. The panel has
#   about fifteen rows, so twenty Downs end on the button whatever the owner arranged.
# - A dock tile (`{DIGIT}` in `open`, with `tile: {'app': id}`): Home's number keys open the dock's tile of that number (1 to 9, 0 the tenth) from any
#   focus, as OK on it does (Dock.openTile: the tile's launch origin, the launch screen, SAM's launch; the Apps tile opens the library). The number
#   comes from the dock's own list (the set's launch points, not hidden ones), so a tile the owner moved is found where it is. It needs the number keys
#   on and Quick Access off (the surface switches them for its run and puts them back, `number-keys` and `quick-access`).
KB = '/tmp/kb.py'
HOME_TOP = 'UPx8@0.1 LEFTx6@0.06'
QUICK_PANEL_FOOT = 'MENU s1.3 DOWNx20@0.03 s0.2'
DOCK_NUMBERS = 10       # the number keys reach the dock's first ten tiles
HOME_ID = 'com.webos.app.home'


def launch_cmd(app):
    return f"luna-send -n 1 {SAM}/launch '{{\"id\":\"{app}\"}}'"


def close_cmd(app):
    return f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"{app}\"}}'"


# An app that is running and not in front, so that a launch from its tile is a warm one (the transition is what is measured, not the app's start),
# and Home in front again: the app is started, given time to draw its first page, and Home is launched over it.
def app_warm(app, start_s):
    return [close_cmd(app), 'sleep 3', launch_cmd(app), f'sleep {start_s}', launch_cmd(HOME_ID), 'sleep 5']


def app_gone(*apps):
    return [close_cmd(a) for a in apps] + ['sleep 2', launch_cmd(HOME_ID), 'sleep 5']


# Apps with a card of their own for the switcher: the film app, LG's Content Store and LG's browser, one after the other, Home last. A card
# needs an appId and a window that has been in front; LG's Settings is an overlay window SAM never names, so it has none.
SWITCHER_APPS = [('netflix', 10), ('com.webos.app.discovery', 8), ('com.webos.app.browser', 6)]
SWITCHER_SETUP = [close_cmd(a) for a, _ in SWITCHER_APPS] + ['sleep 3'] + \
                 [c for a, s in SWITCHER_APPS for c in (launch_cmd(a), f'sleep {s}', launch_cmd(HOME_ID), 'sleep 3')]
SWITCHER_TEARDOWN = app_gone(*[a for a, _ in SWITCHER_APPS])

# What the surfaces switch for a run (a surface's `switches`: {name: value}). The gate reads the value first, sets the one the surface needs and
# puts the one it read back at the end, whatever happened: `read(sh)` the current value, `write(sh, value)` the new one, `wait_s` how long the set
# takes to act on it (Home reads theme.json every 2 s, the compositor once a second).


def _tool_json(sh, args):
    rc, out, err = sh(f'{TOOL} {args}')
    if rc != 0:
        raise RuntimeError(f'lumatv {args} failed: {err.strip() or out.strip()}')
    try:
        return json.loads(out)
    except ValueError:
        raise RuntimeError(f'lumatv {args} answered {out.strip()[:80]!r}')


def _theme_set(sh, patch):
    rc, _, err = sh(TOOL + ' theme set ' + shlex.quote(json.dumps(patch)))
    if rc != 0:
        raise RuntimeError(f'lumatv theme set failed: {err.strip()}')


def _read_app_switcher(sh):
    doc = _tool_json(sh, 'theme get')
    return ((doc.get('theme') or {}).get('remote') or {}).get('appSwitcher') is True     # off unless the owner's file says on


def _read_transitions(sh):
    return bool(_tool_json(sh, 'transitions status')['transitions']['on'])



def _write_flag(sh, args):
    rc, out, err = sh(f'{TOOL} {args}')
    if rc != 0:
        raise RuntimeError(f'lumatv {args} failed: {err.strip() or out.strip()}')


def _read_theme_remote(sh, key, default):
    return ((_tool_json(sh, 'theme get').get('theme') or {}).get('remote') or {}).get(key, default)


def _read_number_keys(sh):
    return _read_theme_remote(sh, 'numberKeys', True) is not False                 # on unless the owner's file says off


def _read_quick_access(sh):
    q = _read_theme_remote(sh, 'quickAccess', None)
    return isinstance(q, dict) and q.get('on') is True                             # off unless the owner's file says on


SWITCHES = {
    'app-switcher': {'read': _read_app_switcher, 'wait_s': 3,
                     'write': lambda sh, on: _theme_set(sh, {'remote': {'appSwitcher': bool(on)}})},
    'number-keys': {'read': _read_number_keys, 'wait_s': 3,
                    'write': lambda sh, on: _theme_set(sh, {'remote': {'numberKeys': bool(on)}})},
    'quick-access': {'read': _read_quick_access, 'wait_s': 3,              # the deep merge keeps the owner's own number choices
                     'write': lambda sh, on: _theme_set(sh, {'remote': {'quickAccess': {'on': bool(on)}}})},
    'transitions': {'read': _read_transitions, 'wait_s': 3,
                    'write': lambda sh, on: _write_flag(sh, 'transitions ' + ('on' if on else 'off'))},
}
NUMBER_KEYS = {'number-keys': True, 'quick-access': False}      # what a surface that presses a dock tile's number key switches (see QUICK_PANEL_FOOT's note)

# The compositor's control tool, from the bundle (LOADGEN's loader and library path). /tmp/lumatv/bin and lib are gone after a reboot until the
# bundle is deployed again: then neither the ring below nor `gate.sh refresh` can be had (unfit()).
CTL = '/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg'
# The compositor's own account of its transitions (`ctl transitions`, docs/research/transitions-result.md): the counters a surface that launches an app
# reads before and after its measured cycles.
CTL_TRANSITIONS = CTL + ' transitions'
# The compositor's trace ring (`ctl ring N`, src/ring.rs): a line a frame drawn (`D`: render time, owed pixels, UI draws and layers), a line a flip (`L`: the
# draw's cost to the GPU's finish, what the pacer made of it). The gate keeps the ring of every cycle it measures (<name>.ring.json beside the trace) and
# prints its numbers (ringsum.py); the ring (768 KiB of text) holds about 50 s of the busiest frames at 60 Hz and 25 s at 120.
CTL_RING = CTL + ' ring'
RING_LINES = 8000

# A page of LumaTV's Settings from its rail: Right enters it, Down held (the remote's own repeat: a move at the press and one every 100 ms from 300 ms on, 6 in 800 ms) and Up held walk its list, Back
# is the rail again. Left and Right inside a page change a slider or a segment, so neither is pressed there.
SETTINGS_PAGE = 'RIGHT s0.1 DOWN:800 s0.15 UP:800 s0.1 BACK s0.15'
# LG's Search app, cold, with its keyboard up: the keyboard comes with it (VOICE_SETUP hides it with Back, which these surfaces do not).
KEYBOARD_SETUP = VOICE_SETUP[:4]
# The D-pad across the keys: along the top row and back, two rows down and up, then held along the row and down and up the rows.
KEYBOARD_WALK = 'RIGHTx6@0.12 DOWNx2@0.12 LEFTx6@0.12 UPx2@0.12 RIGHT:900 s0.2 LEFT:900 s0.2 DOWN:600 s0.2 UP:600'

# ------------------------------------------------------------------ surfaces
#
# name          unique; also the marker prefix: windows are "<name>-open" and "<name>-close"
# open, close   kb.py token strings. open starts with the token "@<name>-open" and close with
#               "@<name>-close", each placed immediately before the key it times. A close with
#               no key only ends the open window. With `act: 'release'` open is the marker and
#               one key token, and the gate presses the key 50 ms before the marker and releases
#               it at the marker; what follows the key in open (a walk of the screen it opened)
#               runs from the release on, inside the same window.
# settle_s      seconds waited after open and after close before the next step
# close_settle_s  seconds waited after the close when that is not settle_s (a full capture's ring holds about FULL_RING_S: a cycle
#               longer than FULL_CAPTURE_MAX_S loses its first markers, and validate() says so)
# transition    True: the close key is a measured response too (checked like the open); False
#               for workloads on a surface that stays up (Home idle, D-pad, sweep)
# setup         shell commands run on the set once before the surface, teardown after it
#               (also on failure)
# reopen        shell commands run before every cycle, outside the measured windows, for a
#               surface the close key tears down for good (reopen_wait_s waits for it to build)
# prep          kb.py tokens run at the start of every cycle under the marker "@<name>-prep", which ends the window of the
#               cycle before: the keys that put Home's focus where the surface starts (HOME_TOP, QUICK_PANEL_FOOT)
# tile          {'app': id}: `{DIGIT}` in `open` is the number key of that app's dock tile, read off the set's launch points when
#               the surface starts (resolve_tile); an app with no tile among the first ten fails the surface before a key is pressed
# switches      {name: value} of SWITCHES: set for the surface, read first and put back at the end whatever happened
# ctl_transitions  True: the compositor's own transition counters (`ctl transitions`) are read around the measured cycles; at least
#               one launch and one leave a cycle must show, and no frame over 20 ms apart while one ran
# app_retries   the captures of a surface whose burst held an app's own drawing that are measured again (default APP_RETRIES);
#               0 for a surface that is the app's drawing (a launch, the keyboard LG's process draws)
# soak          False when cycling it says nothing about memory (Home idle)
# act           'press' (default) or 'release': the edge of the key the surface acts on
# still         True (default): Home's own motion (the wallpaper's tick, the weather card's sprites) is off while the surface is measured
# delivery      True: a luna call opens it; the first frame is timed from the compositor's delivery marker, else luna-send's return
# default       False: part of the gate only when named with --surfaces
# full_budget_s seconds one full capture may hold (default FULL_TRACE_BUDGET_S): less over playback, where the ring fills faster
# thresholds    per-surface overrides of THRESHOLDS; None switches a check off
# inputs_free   verify that teardown released every input decoder port and input app
SURFACES = [
    {'name': 'home-idle', 'still': False,
     'open': '@home-idle-open s3', 'close': '@home-idle-close', 'settle_s': 3,
     'transition': False, 'soak': False,
     'thresholds': {'first_frame_ms': None, 'burst_end_ms': None, 'burst_over20': None}},
    {'name': 'home-dpad',
     'open': '@home-dpad-open RIGHTx8@0.35 LEFTx8@0.35 UP s0.5 DOWN', 'close': '@home-dpad-close', 'settle_s': 1,
     'transition': False,
     'thresholds': {'burst_end_ms': None}},
    # held keys with the remote's own repeat cadence: first repeat after 300 ms, then every 100 ms
    {'name': 'home-sweep',
     'open': '@home-sweep-open RIGHT:1500 s0.4 LEFT:1500 s0.4 UP s0.6 DOWN', 'close': '@home-sweep-close', 'settle_s': 1,
     'transition': False,
     'thresholds': {'burst_end_ms': None}},
    {'name': 'quick-settings', 'act': 'release',
     'open': '@quick-settings-open MENU', 'close': '@quick-settings-close BACK', 'settle_s': 2.5},
    # LumaTV's quick panel (the gear's tap) and input picker (the Input key), which replace LG's Quick Settings and picker on the gear and Input
    # keys (the second key-filter entry, payload/keyfilter/lumatv.js). The gear is acted on at its release: LG swallows its press and replays a
    # short press when the key comes up. The Input key is acted on at its press, LG's and ours alike, so the picker surfaces are timed from the
    # press (round 11 timed them from the release and saw only the slide's tail). Run on request (`--surfaces quick,picker`) with Home in front;
    # `quick-settings` above is the same gear tap, so it now measures the same panel. The Input key is evdev 241 (`Key_webOS_InputSource`,
    # round 8); pass it as `--input-key 241` (`python3 /tmp/kb.py --learn 15` on the set reads another remote's).
    # The surfaces over playback have the film or the input in front first (`setup`), outside the measured windows: `-netflix` is the 4K Dolby
    # Vision test film (HANDOFF: mute the receiver first and restore it after), `-hdmi` is HDMI input N (`--hdmi N`, default 3, the game console).
    # The panel slides in 110 ms and the gate's burst end is 180 ms, so the slide is what the burst-end check measures.
    # Over the film the trace takes about 79k events/s (round 11: 832993 written in 10.5 s, the first cycle's markers lost with 172k
    # events of CPU 0's ring), so the `-netflix` surfaces get one cycle per full capture.
    {'name': 'quick', 'default': False, 'act': 'release',
     'open': '@quick-open MENU', 'close': '@quick-close BACK', 'settle_s': 1.5},
    {'name': 'picker', 'default': False,
     'open': '@picker-open {INPUT}', 'close': '@picker-close BACK', 'settle_s': 1.5},
    {'name': 'quick-netflix', 'default': False, 'act': 'release', 'setup': NETFLIX_SETUP, 'teardown': NETFLIX_TEARDOWN,
     'open': '@quick-netflix-open MENU', 'close': '@quick-netflix-close BACK', 'settle_s': 1.5, 'full_budget_s': NETFLIX_FULL_BUDGET_S},
    {'name': 'picker-netflix', 'default': False, 'setup': NETFLIX_SETUP, 'teardown': NETFLIX_TEARDOWN,
     'open': '@picker-netflix-open {INPUT}', 'close': '@picker-netflix-close BACK', 'settle_s': 1.5, 'full_budget_s': NETFLIX_FULL_BUDGET_S},
    # LG's own Quick Settings over the same film, the panel switched off for the run: the baseline for the first frame over Dolby Vision
    # playback, where the display's OSD path is off until something draws (round 11: its interrupt resumed 1171 ms after the gear tap).
    {'name': 'lgqs-netflix', 'default': False, 'act': 'release',
     'setup': [TOOL + " theme set '{\"remote\":{\"quickPanel\":false}}'", 'sleep 3'] + NETFLIX_SETUP,
     'teardown': NETFLIX_TEARDOWN + [TOOL + " theme set '{\"remote\":{\"quickPanel\":true}}'"],
     'open': '@lgqs-netflix-open MENU', 'close': '@lgqs-netflix-close BACK', 'settle_s': 2.5, 'full_budget_s': NETFLIX_FULL_BUDGET_S},
    {'name': 'quick-hdmi', 'default': False, 'act': 'release', 'setup': HDMI_SETUP, 'teardown': HDMI_TEARDOWN,
     'open': '@quick-hdmi-open MENU', 'close': '@quick-hdmi-close BACK', 'settle_s': 1.5, 'inputs_free': True},
    {'name': 'picker-hdmi', 'default': False, 'setup': HDMI_SETUP, 'teardown': HDMI_TEARDOWN,
     'open': '@picker-hdmi-open {INPUT}', 'close': '@picker-hdmi-close BACK', 'settle_s': 1.5, 'inputs_free': True},
    # The library behind the Apps tile, on request (`--surfaces apps`, with LumaTV's Home in front and at rest, its Apps tile focused:
    # OK on LG's own Home would launch what it has focused). OK, decided on its release, opens it and BACK closes it. Search, the
    # drawer and the voice bar sit behind longer key paths (the bar's icons, the library's search button) that a surface here
    # cannot spell out yet.
    {'name': 'apps', 'default': False, 'act': 'release',
     'open': '@apps-open OK', 'close': '@apps-close BACK', 'settle_s': 2.5},
    # LG's Content Store scrolled with the D-pad (`--surfaces store-scroll`, with the store's home page up, the first shelf focused): five
    # moves down and five back at the remote's own pace, the way `home-dpad` walks Home. The compositor draws a web card by what the page
    # commits, so a page that animates a long transition or paints a shadow per tile shows as frames over 20 ms in these windows and as
    # a late first frame; run it with `lumatv store off` and again with `store on` and compare the two (the store's open time is the
    # other number, docs/research/store-result.md). The page is LG's and its focus rule is not ours, so the thresholds that judge a
    # LumaTV surface (a burst that ends within 180 ms) are not asked of it; the first frame after each key is.
    {'name': 'store-scroll', 'default': False, 'setup': STORE_SETUP, 'teardown': STORE_TEARDOWN,
     'open': '@store-scroll-open DOWNx5@0.35 UPx5@0.35', 'close': '@store-scroll-close', 'settle_s': 1,
     'transition': False,
     'thresholds': {'burst_end_ms': None}},
    # LG's Search app over Home, restyled by `lumatv voice on` (`--surfaces voice-focus`): the keyboard is hidden, then focus walks the field,
    # the keyword rows and the genre cards and the top row's buttons and back, the same cycle in either look. The app is LG's native Qt
    # client, so the first frame after a key is what it commits; the gate's burst-end check is not asked of it.
    {'name': 'voice-focus', 'default': False, 'setup': VOICE_SETUP, 'teardown': VOICE_TEARDOWN,
     'open': '@voice-focus-open DOWNx3@0.4 UPx3@0.4 RIGHTx2@0.4 LEFTx2@0.4', 'close': '@voice-focus-close', 'settle_s': 1,
     'transition': False, 'soak': False,
     'thresholds': {'burst_end_ms': None}},
    # The screens of LumaTV's shell walked with the D-pad, one cycle a surface (`--surfaces settings-scroll,apps-grid,home-widgets,switcher,
    # transitions-netflix,transitions-settings,keyboard`, LumaTV's compositor, Home at rest in front), on the owner's own Home layout:
    # none counts on where a card, a status bar item or a dock tile is (see HOME_TOP, QUICK_PANEL_FOOT and the dock's number keys above). Each
    # ends on Home. The window of a walk is the key that enters the screen (first frame within 50 ms), the walk and the way out, so the frames over
    # 20 ms inside a running animation and the p90 of the render cost are those of everything the walk moves; a walk's burst never ends at a point
    # of its own, so the burst-end check is off as for `home-dpad`.
    # A walk presses no OK inside a screen and no key that changes a value: Settings is entered by the quick panel's "All settings" and moved with
    # Up and Down, which on a row of a page only move focus (Left and Right there would change a slider, so Back leaves a page), the library's OK
    # launches an app and is not pressed, the switcher's Down and OK quit an app and are not pressed.
    # `settings-scroll`: LumaTV's Settings from the quick panel's "All settings" (timed from the press of OK on it: the panel acts on the press, and
    # Settings, up at once at 120 Hz with the panel cut under it, is drawn before the release; timed from the release the first frame was the next
    # key's, 0.6 s on, on 2026-11-05), the Picture page's long list down and up with a held key, the rail down through every section to System (each
    # Down switches the page beside it), that page's list, the rail back up to Picture, Back. Settings' rail stays where it was left, so the walk ends
    # on Picture, where the next open starts.
    {'name': 'settings-scroll', 'default': False, 'transition': False,
     'prep': QUICK_PANEL_FOOT,
     'open': '@settings-scroll-open OK s0.6 ' + SETTINGS_PAGE + ' DOWNx8@0.06 ' + SETTINGS_PAGE + ' UPx9@0.06 BACK',
     'close': '@settings-scroll-close', 'settle_s': 0.6, 'close_settle_s': 0.5,
     'thresholds': {'burst_end_ms': None}},
    # `apps-grid`: the library from the Apps tile's number key (the tile is wherever the dock's list has it), across its first row, down the grid
    # with a held key and up, out to the rail, down and up its sections (the grid is rebuilt for each), back in, Back. Nothing is launched.
    {'name': 'apps-grid', 'default': False, 'transition': False,
     'switches': NUMBER_KEYS, 'tile': {'app': 'com.webos.app.discovery'},
     'open': '@apps-grid-open {DIGIT} s0.7 RIGHTx4@0.12 DOWN:600 s0.15 UP:600 s0.15 LEFTx5@0.12 DOWNx3@0.12 UPx3@0.12 RIGHT BACK',
     'close': '@apps-grid-close', 'settle_s': 0.7, 'close_settle_s': 0.5,
     'thresholds': {'burst_end_ms': None}},
    # `home-widgets`: every row of Home, from the status bar's four items down through the clock, the At-home card, the news and the weather
    # to the dock, along the dock's row and down its rows with held keys, and back up to the status bar. No OK, no Back: nothing opens.
    {'name': 'home-widgets', 'default': False, 'transition': False,
     'prep': HOME_TOP,
     'open': '@home-widgets-open RIGHTx3@0.12 LEFTx3@0.12 DOWN s0.15 DOWN s0.15 RIGHT s0.15 RIGHT s0.15 DOWN s0.15 LEFT:900 s0.2 RIGHT:900 s0.2 '
             'DOWNx2@0.15 UPx2@0.15 UPx6@0.12',
     'close': '@home-widgets-close', 'settle_s': 0.6, 'close_settle_s': 0.5,
     'thresholds': {'burst_end_ms': None}},
    # `switcher`: the app switcher (Settings > Remote & pointer > App switcher, switched on for the run with `lumatv theme set` and put back) opened
    # by a press of Home on Home with apps running (setup starts three), walked with Left and Right (taps, then held), closed with Back. The compositor
    # marks its own opening `Q:switcher-show`, so each open window says when the shell was asked. Pictures move in 200 ms (app-switcher-result.md).
    {'name': 'switcher', 'default': False, 'act': 'release', 'transition': False,
     'switches': {'app-switcher': True}, 'setup': SWITCHER_SETUP, 'teardown': SWITCHER_TEARDOWN,
     'open': '@switcher-open HOME s0.9 RIGHTx2@0.2 LEFTx2@0.2 RIGHT:700 s0.2 LEFT:700 s0.2 BACK',
     'close': '@switcher-close', 'settle_s': 0.8, 'close_settle_s': 0.5,
     'thresholds': {'burst_end_ms': None}},
    # `transitions-netflix`, `transitions-settings`: an app launched and Home pressed in it, with smooth transitions on for the run (`lumatv
    # transitions on`, put back). The app is running already (setup), so the launch is a resume and the compositor's grow out of the tile (220 ms)
    # and shrink into it (180 ms) are what the windows hold; the key-to-first-frame and burst-end checks measure the app's resume and SAM's round
    # trip, not the compositor, and are off; the frames over 20 ms apart, the render cost, and the compositor's own counters (`ctl transitions`: a
    # launch and a leave per cycle, none of its frames late) are what is asked. Netflix is launched from its tile by its number key (the tile is
    # wherever the dock's list has it). LG's Settings has no tile: it is the last row of LumaTV's Settings rail ("More in LG settings", reached
    # through the quick panel as in `settings-scroll`), a launch with no tile known, which grows from the screen's middle; Back leaves it.
    {'name': 'transitions-netflix', 'default': False,
     'switches': dict(NUMBER_KEYS, transitions=True), 'setup': app_warm('netflix', 12), 'teardown': app_gone('netflix'),
     'tile': {'app': 'netflix'},
     'open': '@transitions-netflix-open {DIGIT}', 'close': '@transitions-netflix-close HOME', 'settle_s': 3, 'close_settle_s': 2,
     'ctl_transitions': True, 'app_retries': 0,
     'thresholds': {'first_frame_ms': None, 'burst_end_ms': None}},
    {'name': 'transitions-settings', 'default': False, 'act': 'release',
     'switches': {'transitions': True}, 'setup': app_warm('com.palm.app.settings', 6), 'teardown': app_gone('com.palm.app.settings'),
     'prep': QUICK_PANEL_FOOT + ' OK s1.0 DOWNx12@0.04 s0.3',
     'open': '@transitions-settings-open OK', 'close': '@transitions-settings-close BACK', 'settle_s': 3, 'close_settle_s': 2,
     'ctl_transitions': True, 'app_retries': 0,
     'thresholds': {'first_frame_ms': None, 'burst_end_ms': None}},
    # Search opens the stock LG keyboard. Walk without typing; its frames belong to LG's process.
    {'name': 'keyboard', 'default': False, 'setup': KEYBOARD_SETUP, 'teardown': VOICE_TEARDOWN,
     'open': '@keyboard-open ' + KEYBOARD_WALK, 'close': '@keyboard-close', 'settle_s': 0.5, 'close_settle_s': 0.5,
     'transition': False, 'app_retries': 0, 'thresholds': {'burst_end_ms': None}},
    # The gallery is built by the theme flag, not by a key (Home polls theme.json within 2 s), and
    # BACK turns the flag off again. Each cycle sets it, waits for the build, then times the first
    # D-pad move inside the built gallery (open) and BACK (close).
    # The toast and the alert (`--surfaces toast,alert`). The toast is LumaTV's, raised through the compositor's control socket
    # (`!toast` in kb.py: `ctl shell show toast`, the shell's own feed path; LG's createToast stalled the set for 20 minutes in round 26),
    # and nothing closes it but its own timer (8 s); the alert is LG's createAlert, closed by OK. The stock alert is not expected to pass.
    # A cycle of the toast is 7.8 s. With 8 s on each side of the close it was 16.3 s, longer than a full capture's ring
    # holds: LG's toast on 2026-10-25 and LumaTV's on 2026-10-27 each found 2 windows of 3, the two of LumaTV's without any `Q:` marker
    # and with the display's OSD interrupt "back" 281 and 582 ms into them (the oldest events of CPU 0's ring were gone).
    # The toast slides in on the layer beat (220 ms): that slide is the burst, not a drop, so its end is held to 300 ms rather than the
    # 180 ms of an opening (rounds 21 to 23 measured 241-255 ms).
    {'name': 'toast', 'default': False, 'delivery': True,
     'open': '@toast-open !toast', 'close': '@toast-close', 'settle_s': 7, 'close_settle_s': 0.5, 'transition': False,
     'thresholds': {'burst_end_ms': 300.0}},
    # The format badge (`!format`: `ctl shell show toast format:...`) slides in on the same layer beat and leaves by its own 5 s timer,
    # so the close marker falls as it leaves and the close window holds that exit.
    {'name': 'format', 'default': False, 'delivery': True,
     'open': '@format-open !format', 'close': '@format-close', 'settle_s': 5, 'close_settle_s': 0.8, 'transition': False,
     'thresholds': {'burst_end_ms': 300.0}},
    {'name': 'alert', 'default': False, 'delivery': True,
     'open': '@alert-open !alert', 'close': '@alert-close OK', 'settle_s': 2.5},
]

SURFACE_DEFAULTS = {'setup': [], 'teardown': [], 'reopen': [], 'reopen_wait_s': 0, 'settle_s': 1.0, 'close_settle_s': None,
                    'transition': True, 'soak': True, 'default': True, 'thresholds': {},
                    'act': 'press', 'still': True, 'delivery': False, 'full_budget_s': None, 'inputs_free': False,
                    'prep': '', 'switches': {}, 'tile': None, 'ctl_transitions': False, 'app_retries': None}

# The PLAN gate. An override in a surface may set any of these to None to turn the check off.
THRESHOLDS = {
    'p90_render_ms': 3.0,     # p90 of the animated-frame render time, all cycles pooled
    'p90_window_ms': 4.0,     # the same for one cycle's window: a single bad cycle is not averaged away
    'dropped': 0,             # animated frames over 20 ms that follow another animated frame
    'burst_over20': 0,        # frame intervals over 20 ms inside the burst that starts at the key
    'first_frame_ms': 50.0,   # marker to first presented frame
    'burst_end_ms': 180.0,    # marker to the last frame of that burst: "fully open"
}

# Under LG's compositor the floor is not ours to fix: a trivial client over idle Home takes two
# vblanks for 17 % of its frames (X1, PLAN.md), so there `dropped` is held to that share of the
# animated frames instead of to zero. LumaTV's own compositor is held to zero.
LG_COMPOSITOR_DROP_SHARE = 0.17

MEASURE_CYCLES = 3
SOAK_DEFAULT_CYCLES = 10
SOAK_RELEASE_CYCLES = 500
SOAK_TRACE_MAX = 60       # cycles per basic trace in a soak
# The full trace's 8 MB per-CPU ring held about 13 s at the 44k events/s measured over
# Quick Settings; a longer capture loses its first markers. A full capture is planned to stay
# under this many seconds, so a long surface gets one cycle per capture.
FULL_TRACE_BUDGET_S = 11.0
# What the ring holds, and the longest one cycle may take: a cycle is the least a full capture is cut to, so one longer than this loses its
# first markers whatever the plan does (the stock toast's 16.3 s did, validate() refuses it now).
FULL_RING_S = 13.0
FULL_CAPTURE_MAX_S = 12.0
GAP_MS = 17.5             # analyse.py's gap listing threshold, as run.py uses (a 60 Hz refresh and a twentieth); scaled by --refresh
# The refresh the frames are presented at (--refresh): 120 under the compositor's refresh test mode (`ctl refresh 120`), where a
# dropped frame is an interval over 10 ms and the gap listing starts at 8.75; the first-frame and burst-end limits are wall time and stay.
REFRESH_HZ = 60.0
# A capture whose burst held an app's own drawing (analyse.py `burst_app_jobs`: a film's player redrawing its controls at 60 Hz) measures
# the app as well as the surface; it is kept under `discarded` and measured again, at most this many more times.
APP_RETRIES = 2
# Camera pop-ups, plugin notes and toasts take the keys and re-time the panel (a pop-up's video window put the apps grid 1450 ms late), so a
# capture one landed in does not measure the surface. The gate snoozes every camera's alerts for the run (CameraHold: HOLD_MINUTES, renewed
# as it goes, so a gate that is killed leaves the snooze for that long at most) and puts it back as it found it. That does not hold a doorbell
# ring or a toast, so a capture with a pop-up, a note or a toast in it is `discarded` and measured again, at most DISTURBED_RETRIES more
# times; QUIET_WAIT_S is how long one already up at the start is waited out.
HOLD_MINUTES = 30
HOLD_LABEL = 'Frame gate'
HELPER_API = 'http://127.0.0.1:18731/v1'
DISTURBED_RETRIES = 2
QUIET_WAIT_S = 20
# Home builds and warms its surfaces for about a minute after the compositor starts (round 16: idle measured 40 s after a restart
# read p90 12.88 ms, 0.46 later); the gate waits until the compositor is this old.
COMPOSITOR_SETTLE_S = 90

# RSS rule: the first RSS_SKIP_CYCLES cycles warm the compositor and are ignored. After them the
# resident set may grow by RSS_PER_100_CYCLES_MB per 100 cycles, plus RSS_NOISE_MB for the
# sampler's own noise (the RSS wobbles by about 0.8 MB around a Quick Settings open in the
# recorded .sys files, more than the 0.08 MB 10 cycles are allowed to leak). A shrink passes;
# a missing sample (NaN) fails.
RSS_SKIP_CYCLES = 2
RSS_PER_100_CYCLES_MB = 1.0
RSS_NOISE_MB = 1.0
PAGE_KB = 4

# A line of /var/log/messages that matches any of these fails the surface. The QML ones count only from our processes (LOG_OWN_PROGRAM_RE): LG's
# apps that the gate drives (Search, Settings, the Content Store) log their own QML warnings there (LG Search's `SearchView.qml:294:13: Unable to
# assign [undefined] to QQuickItem*`), and a surface is not failed for another app's code.
LOG_QML_PATTERNS = [r'TypeError', r'ReferenceError', r'is not a type', r'Binding loop', r'Unable to assign', r'Cannot assign', r'ShaderEffectSource']
LOG_ANY_PROCESS_PATTERNS = [r'\[LumaKeys\].*failed', r'\[LumaHome\].*(invalid|unreadable|failed)', r'crash', r'segfault']
LOG_FAIL_PATTERNS = LOG_QML_PATTERNS + LOG_ANY_PROCESS_PATTERNS
# The program of a PmLog line (`<time> [<monotonic>] user.info <program> [] <context> ...`): LumaTV's shell client (`org.nphil.lumatv.shell`, the
# runner's name for its app id) and its other apps, LG's compositor (which hosts LumaHome under it), the runner and the kernel. A line whose
# program cannot be read is counted, as before.
LOG_PROGRAM_RE = re.compile(r'\buser\.\w+\s+(\S+)')
LOG_OWN_PROGRAM_RE = re.compile(r'^(org\.nphil\.lumatv|lumatv|surface-manager$|qml-runner$|kernel$)')
# Known stock-LG noise that the patterns would otherwise flag on every run: Quick Settings' own
# response callback throws when the foreground-app array changes (surfaces-p1-result.md); Netflix
# reports the instance the `-netflix` setup closed (closeByAppId) as a crash report of a user kill;
# LG's toast model reads `userData.id` of the audio service's "Dolby Vision / Dolby Atmos" toast,
# which has none, 20-30 ms after it is shown at the film's start (round 12, all three setups); the
# same line also reports it as a bare "Type error" (round 16, both film surfaces).
LOG_IGNORE_PATTERNS = [r'Controllers/ScreenOffInterfaces/Interfaces\.qml:52',
                       r'\[CrashReporter\] Previous CrashReport.*REASON_USER_KILL',
                       r"SystemUI/Notification/Services\.qml:513: TypeError: (Cannot read property 'id' of undefined|Type error)$"]
LOG_FAIL_RE = re.compile('|'.join(LOG_FAIL_PATTERNS))
LOG_ANY_PROCESS_RE = re.compile('|'.join(LOG_ANY_PROCESS_PATTERNS))
LOG_IGNORE_RE = re.compile('|'.join(LOG_IGNORE_PATTERNS))
# What the shell's FrameMotion logs at the end of each 120 Hz slide (`[FrameMotion] 17 steps in 150 ms for 110`): the frames a slide got and the
# time it took, against the time it was asked for. QML's console.info goes to PmLog, so this is read from the same log as the errors.
LOG_SLIDE_RE = re.compile(r'\[FrameMotion\] (\d+) steps in (\d+) ms for (\d+)')
LOG_FILE = '/var/log/messages'
LOG_TS = re.compile(r'\[\s*(\d+\.\d+)\s*\]')
LOG_REPORT_MAX = 20

CAPTURE = '/tmp/perf/capture.sh'
SAMPLER = '/tmp/perf/sample.py'
# The synthetic background load (compositor/crates/lumatv-compositor/src/bin/lumatv-loadgen.rs, in the deployed bundle): run through the bundle's own
# loader, a camera-like decode, Home Assistant websocket chatter, a BLE scan stand-in and a low-priority download to /tmp.
LOADGEN = '/tmp/lumatv/lib/ld-linux-aarch64.so.1 --library-path /tmp/lumatv/lib /tmp/lumatv/bin/lumatv-loadgen'
LOAD_KINDS = ('decode', 'ws', 'ble', 'download')
LOAD_LOG = '/tmp/lumatv/out/gate/loadgen.log'


# ------------------------------------------------------- the surfaces table

def surface(entry):
    """An entry with the defaults filled in."""
    s = dict(SURFACE_DEFAULTS)
    s.update(entry)
    s['thresholds'] = dict(THRESHOLDS, **entry.get('thresholds', {}))
    return s


def validate(entries):
    """Raise ValueError on a table the harness would misread."""
    seen = set()
    for e in entries:
        s = surface(e)
        n = s['name']
        if n in seen:
            raise ValueError(f'duplicate surface {n}')
        seen.add(n)
        for field in ('open', 'close'):
            toks = s.get(field, '').split()
            if not toks or toks[0] != f'@{n}-{field}':
                raise ValueError(f'{n}: {field} must start with the marker token @{n}-{field}')
        unknown = set(s['thresholds']) - set(THRESHOLDS)
        if unknown:
            raise ValueError(f'{n}: unknown thresholds {sorted(unknown)}')
        if s['act'] not in ('press', 'release'):
            raise ValueError(f"{n}: act is 'press' or 'release'")
        if s['act'] == 'release':
            opens = s['open'].split()
            if len(opens) < 2 or opens[1][0] in '@!s' or opens[1][-1] in '+-:' or 'x' in opens[1]:
                raise ValueError(f'{n}: act release needs open to be the marker and one plain key token, then what follows it')
        preps = s['prep'].split()
        if any(t[0] in '@!' for t in preps):
            raise ValueError(f'{n}: prep is keys; its marker @{n}-prep is put before it')
        if bool(s['tile']) != ('{DIGIT}' in s['open'].split()):
            raise ValueError(f'{n}: tile and the {{DIGIT}} token of open go together')
        unknown = set(s['switches']) - set(SWITCHES)
        if unknown:
            raise ValueError(f'{n}: unknown switches {sorted(unknown)}')
        if s['delivery'] and '!' not in s['open']:
            raise ValueError(f'{n}: delivery needs a luna call (!NAME) in open')
        if cycle_seconds(s) > FULL_CAPTURE_MAX_S:
            raise ValueError(f'{n}: one cycle takes {cycle_seconds(s):.1f} s and a full capture of it overflows the trace ring (about '
                             f'{FULL_RING_S:g} s) and loses its first markers: shorten the settles ({FULL_CAPTURE_MAX_S:g} s at most)')
    return True


# What the surfaces take from the command line: the Input key's evdev code ({INPUT}) and the HDMI input of the surfaces over an input ({HDMI}).
PLACEHOLDERS = {'INPUT': 'an evdev code: find the remote\'s Input key with `python3 /tmp/kb.py --learn 15` on the set and pass --input-key CODE',
                'HDMI': 'an input number: pass --hdmi N'}


def fill(s, values):
    """The surface with its {NAME} placeholders replaced from `values`; ValueError, naming what to pass, for one that has no value."""
    def sub(text):
        for name in PLACEHOLDERS:
            if '{%s}' % name in text:
                if values.get(name) in (None, ''):
                    raise ValueError(f"surface {s['name']} needs {PLACEHOLDERS[name]}")
                text = text.replace('{%s}' % name, str(values[name]))
        return text
    out = dict(s)
    for field in ('open', 'close'):
        out[field] = sub(s[field])
    for field in ('setup', 'teardown', 'reopen'):
        out[field] = [sub(c) for c in s[field]]
    return out


def find(names, entries=None, values=None):
    table = {e['name']: e for e in (SURFACES if entries is None else entries)}
    out = []
    for n in names:
        if entries is None and n in ('overlay-netflix', 'overlay-hdmi'):
            raise ValueError(f'{n}: native overlay cards require plugin-broker records; the control socket has no card injection command')
        if n not in table:
            raise ValueError(f'unknown surface {n!r}; known: {", ".join(table)}')
        out.append(fill(surface(table[n]), values or {}))
    return out


# ------------------------------------------------------------ cycle commands

def token_seconds(token):
    """How long kb.py takes to run one token (mirrors kb.py's run())."""
    if token[0] in '@!':
        return 0.0
    if token[0] == 's' and token[1:].replace('.', '').isdigit():
        return float(token[1:])
    if token.endswith('+') or token.endswith('-'):
        return 0.0
    if 'x' in token:
        _, rest = token.split('x', 1)
        count, _, gap = rest.partition('@')
        return int(count) * (0.05 + float(gap or 0.35))
    if ':' in token:
        return int(token.split(':')[1]) / 1000.0
    return 0.05


def settle_token(seconds):
    return f's{seconds:g}' if seconds else None


def open_tokens(s):
    """kb.py tokens that open a surface. A surface that acts on the release gets its marker at the
    release: the key goes down 50 ms earlier, the marker is written, the key goes up."""
    toks = s['open'].split()
    if s['act'] == 'release':
        marker, key, *rest = toks
        return [key + '+', 's0.05', marker, key + '-'] + rest
    return toks


def cycle_tokens(s):
    """kb.py tokens for one open/close cycle."""
    toks = ([f"@{s['name']}-prep"] + s['prep'].split()) if s['prep'] else []
    toks += open_tokens(s)
    toks.append(settle_token(s['settle_s']))
    toks += s['close'].split()
    toks.append(settle_token(s['settle_s'] if s['close_settle_s'] is None else s['close_settle_s']))
    return [t for t in toks if t]


def cycle_seconds(s):
    """Wall time of one cycle on the set, with a margin for each kb.py start."""
    return sum(token_seconds(t) for t in cycle_tokens(s)) + s['reopen_wait_s'] + 0.3


def cycle_command(s, n):
    """Shell command that runs n cycles on the set. One kb.py process when nothing has to run
    between cycles, so the timing across cycles is exact too."""
    toks = cycle_tokens(s)
    if not s['reopen']:
        return f'python3 {KB} ' + ' '.join(shlex.quote(t) for t in toks * n)
    one = '; '.join(s['reopen'] + ([f'sleep {s["reopen_wait_s"]:g}'] if s['reopen_wait_s'] else []) +
                    [f'python3 {KB} ' + ' '.join(shlex.quote(t) for t in toks)])
    return '; '.join([one] * n)


def split_cycles(n, max_per):
    """n cycles in as few traces as hold at most max_per each, sizes as even as possible."""
    if n <= 0:
        return []
    k = -(-n // max_per)
    q, r = divmod(n, k)
    return [q + 1] * r + [q] * (k - r)


def plan_full(s, cycles):
    """Cycles per full capture: as many as fit the surface's budget (FULL_TRACE_BUDGET_S unless it says), at least one."""
    per = max(1, int((s.get('full_budget_s') or FULL_TRACE_BUDGET_S) // cycle_seconds(s)))
    return split_cycles(cycles, per)


# ------------------------------------------------------------------ parsing

MARK = re.compile(r'^\s*.*?-\d+\s+\[\d{3}\]\s+\S+\s+([\d.]+):\s+tracing_mark_write:\s?(.*)$')


def parse_markers(text):
    """[(time, text)] of the trace_marker writes in a trace dump."""
    out = []
    for line in text.splitlines():
        if 'tracing_mark_write' not in line:
            continue
        m = MARK.match(line)
        if m:
            out.append((float(m.group(1)), m.group(2).strip()))
    return out


def cycle_ends(markers, name):
    """End time of each cycle of a surface: the next cycle's open marker, the last one's the
    trace's stop marker (or the last marker when the stop is lost)."""
    opens = [t for t, txt in markers if txt == f'P:{name}-open']
    stops = [t for t, txt in markers if txt.startswith('stop ')]
    tail = stops[-1] if stops else (markers[-1][0] if markers else None)
    if not opens or tail is None:
        return []
    return opens[1:] + [tail]


def parse_rss(text):
    """[(uptime, pages)] from a capture.sh .sys sampler file; lines without an rss= field (the
    compositor was gone for that sample) are skipped."""
    out = []
    for line in text.splitlines():
        head, _, _ = line.partition('|')
        m = re.search(r'\brss=(\d+)', line)
        try:
            t = float(head)
        except ValueError:
            continue
        if m:
            out.append((t, int(m.group(1))))
    return out


def rss_at(series, t):
    """RSS of the last sample at or before t (series sorted by time), else None."""
    best = None
    for ts, pages in series:
        if ts > t:
            break
        best = pages
    return best


def rss_growth(series, ends):
    """Compositor RSS growth after the first RSS_SKIP_CYCLES cycles.

    series: [(uptime, pages)] over all chunks; ends: each cycle's end time. Returns the first
    sample, the RSS at the end of cycle RSS_SKIP_CYCLES (base) and of the last cycle, the growth
    and the allowed growth in MB, and ok. Growth is NaN, and not ok, when either sample is
    missing or there are fewer than RSS_SKIP_CYCLES + 1 cycles.
    """
    n = len(ends)
    series = sorted(series)
    out = {'cycles': n, 'growth_mb': math.nan, 'allowed_mb': None, 'base_pages': None, 'last_pages': None,
           'first_pages': None, 'ok': False}
    if n <= RSS_SKIP_CYCLES or not series:
        return out
    out['first_pages'] = series[0][1]
    out['base_pages'] = rss_at(series, ends[RSS_SKIP_CYCLES - 1])
    out['last_pages'] = rss_at(series, ends[-1])
    out['allowed_mb'] = round(RSS_PER_100_CYCLES_MB * (n - RSS_SKIP_CYCLES) / 100.0 + RSS_NOISE_MB, 3)
    if out['base_pages'] is None or out['last_pages'] is None:
        return out
    out['growth_mb'] = round((out['last_pages'] - out['base_pages']) * PAGE_KB / 1024.0, 3)
    out['ok'] = out['growth_mb'] <= out['allowed_mb']
    return out


# ---------------------------------------------------------------------- log

def lines_since(text, uptime):
    """Lines of a log whose bracketed monotonic timestamp is at or after uptime. A line without
    one belongs to the entry above it. Returns (lines, filtered): filtered is False when no line
    had a timestamp, and then every line is returned."""
    lines = text.splitlines()
    if not any(LOG_TS.search(ln) for ln in lines):
        return lines, False
    out, keep = [], False
    for ln in lines:
        m = LOG_TS.search(ln)
        if m:
            keep = float(m.group(1)) >= uptime
        if keep:
            out.append(ln)
    return out, True


def foreign_qml(line):
    """True for a QML error that another app's process logged (LG Search's, LG Settings'): a line whose program is not LumaTV's, LG's compositor's or
    the runner's, and which only the QML patterns match (a crash or a LumaTV line from any process still counts)."""
    m = LOG_PROGRAM_RE.search(line)
    return bool(m) and not LOG_OWN_PROGRAM_RE.match(m.group(1)) and not LOG_ANY_PROCESS_RE.search(line)


def log_failures(lines):
    """(failing lines, ignored count): lines matching LOG_FAIL_PATTERNS, less the known stock noise and other apps' own QML errors."""
    bad, ignored = [], 0
    for ln in lines:
        if LOG_FAIL_RE.search(ln):
            if LOG_IGNORE_RE.search(ln) or foreign_qml(ln):
                ignored += 1
            else:
                bad.append(ln)
    return bad, ignored


def slides_in(lines):
    """[(steps, ms, asked_ms)] of the `[FrameMotion]` lines in `lines`: what each slide of the 120 Hz mode got, in the order they were logged."""
    return [tuple(int(x) for x in m.groups()) for m in map(LOG_SLIDE_RE.search, lines) if m]


# ----------------------------------------------------------------- checks

def check(name, ok, value, limit, detail=''):
    return {'name': name, 'ok': bool(ok), 'value': value, 'limit': limit, 'detail': detail}


def p90(values):
    """The 90th percentile by analyse.py's rule (index 0.9 * (n - 1)), None for no values."""
    vals = sorted(values)
    return vals[int(0.9 * (len(vals) - 1))] if vals else None


def _worst(values):
    vals = [v for v in values if v is not None]
    return max(vals) if vals else None


PANEL_TOLERANCE = 0.08


def panel_check(report, asked_hz):
    """`panel_refresh`: the panel ran at the refresh that was asked for in every window (analyse.py `panel_hz`, from the vblanks). None for a report
    without vblanks. A window on another refresh is judged by its own (a refresh and a fifth) so that it does not count thousands of drops, and
    this check fails it as what it is: not a measurement at that refresh. Round 22: a camera popup, a hardware video window, made the TV re-time the
    panel from 120 Hz to 100 and it stayed so for the rest of the run."""
    hz = [w['panel_hz'] for w in report['windows'] if w.get('panel_hz') is not None]
    if not hz:
        return None
    off = [h for h in hz if abs(h - asked_hz) > PANEL_TOLERANCE * asked_hz]
    worst = max(hz, key=lambda h: abs(h - asked_hz))
    return check('panel_refresh', not off, worst, f'{asked_hz:g} Hz +-{PANEL_TOLERANCE * 100:g} %',
                 (f'{len(off)} of {len(hz)} windows on another refresh: the TV re-timed the panel (the compositor log has `the panel refreshes every ... now`); '
                  'the counts of these windows are by that refresh, and say nothing of the asked one') if off else '')


def render_limits(th):
    """The render-time limits under LumaTV's compositor. analyse.py reads the render time off the compositor's own loop thread there (the `loop` marker of the
    trace; the supervisor `pidof surface-manager` names runs 0.5 ms a frame whatever is drawn), and that is one stage of a frame that is two deep: it may take
    half a refresh (8.3 ms at 60 Hz, 4.2 at 120), where the 3 ms of the Qt clients' GUI thread was set for 60."""
    half = 500.0 / REFRESH_HZ
    return {k: None if th[k] is None else max(th[k], half * (4.0 / 3.0 if k == 'p90_window_ms' else 1.0)) for k in ('p90_render_ms', 'p90_window_ms')}


def evaluate_windows(s, report, cycles, lg_compositor=False, lumatv=False):
    """Checks for one surface from an analyse.py report that holds `cycles` cycles (windows of
    several captures concatenated)."""
    th = dict(s['thresholds'], **render_limits(s['thresholds'])) if lumatv else s['thresholds']
    opens = [w for w in report['windows'] if w['name'] == f"{s['name']}-open"]
    closes = [w for w in report['windows'] if w['name'] == f"{s['name']}-close"]
    keyed = opens + (closes if s['transition'] else [])
    # A window whose start the capture's ring lost has no first frame, burst or OSD interrupt to read (analyse.py `trace_from_ms`): it still counts as a
    # window, `trace_complete` names it, and every check that reads the trace leaves it out. The compositor's own ring (`ring_dropped`) is not the trace's.
    cut = [w for w in keyed if w.get('trace_from_ms') is not None]
    read = [w for w in keyed if w.get('trace_from_ms') is None]
    expected = cycles * (2 if s['transition'] else 1)
    # A surface whose first frame is not checked has no key to answer (Home idle): an empty burst there is not a dead key.
    dead = [w['name'] for w in read if w['first_frame_ms'] is None and th['first_frame_ms'] is not None]
    unmarked = [w['name'] for w in opens if s['delivery'] and w.get('sent_ms') is None and w.get('delivered_ms') is None] if dead else []
    dead_note = ('no frame after the key in ' + ', '.join(dead)) if dead else ''
    if unmarked:
        dead_note += (f"; {len(unmarked)} of them have neither luna-send's return nor the compositor's delivery marker in the trace "
                      '(the ring lost its oldest events, or the call never returned)')
    out = [check('windows', len(keyed) == expected and not dead, f'{len(keyed)}/{expected}', f'== {expected}',
                 dead_note or ('marker lost: trace overflow or key not delivered' if len(keyed) != expected else ''))]
    panel = panel_check(report, REFRESH_HZ)
    if panel:
        out.append(panel)
    if keyed:
        late = max(max(w['trace_from_ms'] for w in cut), 0.0) if cut else 0.0
        out.append(check('trace_complete', not cut, len(cut), '== 0',
                         (f"{len(cut)} of {len(keyed)} windows began before every CPU's events were in the trace (the capture's ring overran: its dump header counts the "
                          f"events lost); their first frame, burst and OSD interrupt are not read; the trace is whole {late:g} ms after the key at worst") if cut else ''))

    if th['first_frame_ms'] is not None:
        v = _worst([w['first_frame_ms'] for w in read]) if not dead else None
        # a key that found the display's OSD path off waits for it, whatever the surface does (analyse.py `osd_wake_ms`, read where the trace is whole)
        off = [w['osd_wake_ms'] for w in read if w.get('osd_wake_ms') is not None]
        # when the compositor asked the shell for the panel the key opens (its own marker, `Q:<name>-show`): the rest is the shell's
        asked = [w['show_ms'] for w in keyed if w.get('show_ms') is not None]
        notes = ([f'the OSD was off at {len(off)} key(s), on after {max(off):g} ms at worst'] if off else []) + \
                ([f'the compositor asked the shell {max(asked):g} ms after the key at worst'] if asked else [])
        out.append(check('first_frame_ms', v is not None and v <= th['first_frame_ms'], v, f"<= {th['first_frame_ms']:g}", '; '.join(notes)))
    if th['burst_end_ms'] is not None:
        ends = [w['burst_end_ms'] for w in read]
        v = None if any(e is None for e in ends) or not ends else max(ends)
        out.append(check('burst_end_ms', v is not None and v <= th['burst_end_ms'], v, f"<= {th['burst_end_ms']:g}",
                         ('no burst at the key' if read else 'no window with its start in the trace') if v is None else ''))
    if th['p90_render_ms'] is not None or th['p90_window_ms'] is not None:
        pooled, per_window = [], []
        for w in read:
            samples = w.get('burst_anim_samples') if s['transition'] else None
            if not samples:
                samples = w.get('anim_render_samples') or []
            pooled += samples
            p = w['burst_anim_p90_ms'] if s['transition'] else None
            if p is None and w['anim_render_ms']:
                p = w['anim_render_ms']['p90']
            per_window.append(p)
        v = p90(pooled)
        if v is None:   # a report without per-frame samples: the worst window's p90
            v = _worst(per_window)
        if th['p90_render_ms'] is not None:
            out.append(check('p90_render_ms', v is None or v < th['p90_render_ms'], v, f"< {th['p90_render_ms']:g}",
                             f'{len(pooled)} frames' if pooled else ('no animated frames' if v is None else '')))
        if th['p90_window_ms'] is not None:
            vw = _worst(per_window)
            out.append(check('p90_window_ms', vw is None or vw < th['p90_window_ms'], vw, f"< {th['p90_window_ms']:g}",
                             'worst single window'))
    if th['dropped'] is not None:
        v = sum(w.get('dropped_in_motion', w['dropped']) for w in read)
        frames = sum(w.get('anim_frames') or 0 for w in read)
        if lg_compositor and th['dropped'] == 0:
            share = v / frames if frames else 0.0
            out.append(check('dropped_share', share <= LG_COMPOSITOR_DROP_SHARE, round(share, 3), f'<= {LG_COMPOSITOR_DROP_SHARE}',
                             f"{v} of {frames} animated frames, LG's compositor floor"))
        elif keyed and all(w.get('ring_dropped') is not None for w in keyed):
            # LumaTV's compositor says itself which of its frames had a motion running after them: the count is its own. The trace's
            # `dropped_in_motion` is not quoted beside it: its chain reads a frame the compositor draws ahead of its refresh as no frame of a
            # motion (session 11's home-widgets: 5 refreshes missed in the ring, 0 read in the trace)
            ring = sum(w['ring_dropped'] for w in keyed)
            out.append(check('dropped', ring <= th['dropped'], ring, f"<= {th['dropped']}",
                             f"the compositor's own count over {sum(w['ring_judged'] for w in keyed)} flips after a frame of a motion"))
        else:
            out.append(check('dropped', v <= th['dropped'], v, f"<= {th['dropped']}"))
    if th['burst_over20'] is not None:
        v = sum(w['burst_over20'] for w in read)
        if read and all(w.get('ring_burst_over20') is not None for w in read):
            # an interval after a frame with no motion after it is a still screen until the next key, not a refresh missed
            ring = sum(w['ring_burst_over20'] for w in read)
            out.append(check('burst_over20', ring <= th['burst_over20'], ring, f"<= {th['burst_over20']}",
                             f"after a frame of a motion, by the compositor's flags; {v} with the still screens between keys"))
        else:
            out.append(check('burst_over20', v <= th['burst_over20'], v, f"<= {th['burst_over20']}"))
    return out


def rss_check(g):
    if math.isnan(g['growth_mb']):
        return check('rss_growth_mb', False, None, f"<= {g['allowed_mb']}" if g['allowed_mb'] is not None else '',
                     f"no RSS sample ({g['cycles']} cycles)")
    return check('rss_growth_mb', g['ok'], g['growth_mb'], f"<= {g['allowed_mb']:g}",
                 f"{g['base_pages']} -> {g['last_pages']} pages over {g['cycles'] - RSS_SKIP_CYCLES} cycles")


def log_check(bad, ignored):
    return check('log_errors', not bad, len(bad), '== 0', f'{ignored} known stock lines ignored' if ignored else '')


def inputs_free_check(rig):
    try:
        rc, out, err = rig.sh(TOOL + ' inputs status')
        if rc != 0:
            raise RuntimeError(err.strip() or f'inputs status exited {rc}')
        status = json.loads(out)
        inputs = status['inputs']
        ports = inputs['ports']
        free = (status.get('ok') is True and isinstance(ports, list) and bool(ports)
                and all(p.get('open') is False for p in ports) and inputs['running'] == [])
        return check('inputs_free', free, inputs, 'all ports closed; no input app running')
    except Exception as e:
        return check('inputs_free', False, None, 'all ports closed; no input app running', str(e))


def set_switches(rig, wanted, saved):
    """Sets what a surface switches (SWITCHES) and notes in `saved`, as (name, value read first), each one before it is written, so that a
    failure half way still has the ones touched to put back. A switch that already has the value is left alone."""
    for name, want in wanted.items():
        sw = SWITCHES[name]
        was = sw['read'](rig.sh)
        saved.append((name, was))
        if was != want:
            sw['write'](rig.sh, want)
            if sw['wait_s']:
                time.sleep(sw['wait_s'])


def restore_switches(rig, saved):
    """Puts each switch back to the value it was read with, last set first, and reads it again; the problems found, '' for none."""
    problems = []
    for name, was in reversed(saved):
        sw = SWITCHES[name]
        try:
            if sw['read'](rig.sh) != was:
                sw['write'](rig.sh, was)
                if sw['wait_s']:
                    time.sleep(sw['wait_s'])
            now = sw['read'](rig.sh)
            if now != was:
                problems.append(f'{name} is {now!r}, was {was!r}')
        except Exception as e:
            problems.append(f'{name}: {e}')
    return '; '.join(problems)


def tile_digit(points, app):
    """(the evdev code of the number key that opens `app`'s dock tile, its place in the dock), None when it has none among the first ten. The dock
    lists the launch points that are not hidden, in the order the service gives them (Dock.qml refresh()); Home's number keys open the tile of that
    number: 1 to 9 the first nine, 0 the tenth (LumaHome.tileForKey). The keys' evdev codes are KEY_1 = 2 to KEY_9 = 10 and KEY_0 = 11."""
    shown = [p for p in points if not p.get('hidden')]
    for i, p in enumerate(shown[:DOCK_NUMBERS]):
        if p.get('id') == app:
            return (i + 2), i + 1
    return None


def resolve_tile(rig, s):
    """The surface with the number key of its app's dock tile in place of `{DIGIT}`, and what it did ('' for a surface with no tile). RuntimeError,
    before any key is pressed, when the dock's list cannot be read or has no tile of the app among the first ten."""
    if not s['tile']:
        return s, ''
    app = s['tile']['app']
    rc, out, err = rig.sh("luna-send -n 1 luna://com.webos.service.homelaunchpoints/listLaunchPoints '{}'")
    if rc != 0:
        raise RuntimeError(f'the dock\'s launch points could not be read: {err.strip()}')
    try:
        points = json.loads(out)['launchPoints']
    except (ValueError, KeyError, TypeError):
        raise RuntimeError(f'the launch points answered {out.strip()[:80]!r}')
    found = tile_digit(points, app)
    if found is None:
        raise RuntimeError(f"{app} has no tile among the dock's first {DOCK_NUMBERS}, which the number keys open: nothing was pressed")
    code, number = found
    return dict(s, open=s['open'].replace('{DIGIT}', str(code))), f"{app}: the dock's tile {number}, number key (evdev {code})"


def parse_transitions(out):
    """The counters of `ctl transitions`' answer: launches, leaves, skipped, lateGaps, maxGapMs; ValueError when it is not one."""
    doc = json.loads(out[out.index('{'):])
    stats = doc['stats']
    return {k: stats[k] for k in ('launches', 'leaves', 'skipped', 'lateGaps', 'maxGapMs')} | {'enabled': doc['enabled']}


def transitions_checks(before, after, cycles):
    """What the compositor says of the transitions that ran between two readings: at least one launch and one leave a cycle (a cycle that
    shows none measured a cut), and none of their frames late (over 20 ms after the one before)."""
    ran = min(after['launches'] - before['launches'], after['leaves'] - before['leaves'])
    late = after['lateGaps'] - before['lateGaps']
    return [check('transitions_ran', ran >= cycles, ran, f'>= {cycles}',
                  f"{after['launches'] - before['launches']} launches, {after['leaves'] - before['leaves']} leaves, "
                  f"{after['skipped'] - before['skipped']} changes of the front left alone; "
                  + ('transitions are on' if after['enabled'] else 'the compositor says transitions are OFF')),
            check('transitions_late_gaps', late == 0, late, '== 0', f"worst gap so far {after['maxGapMs']} ms")]


def verdict(results):
    return 'PASS' if results and all(c['ok'] for r in results for c in r['checks']) else 'FAIL'


def fmt_value(v):
    if v is None:
        return 'none'
    if isinstance(v, float):
        return f'{v:.2f}'
    return str(v)


def format_lines(results):
    lines = []
    for r in results:
        for c in r['checks']:
            lines.append(f"{'PASS' if c['ok'] else 'FAIL'}  {r['name']:<15} {c['name']:<15} {fmt_value(c['value']):>10}  "
                         f"({c['limit']})" + (f"  {c['detail']}" if c['detail'] else ''))
        for ln in r.get('log_lines', [])[:LOG_REPORT_MAX]:
            lines.append(f"      log: {ln}")
        if len(r.get('log_lines', [])) > LOG_REPORT_MAX:
            lines.append(f"      log: ... {len(r['log_lines']) - LOG_REPORT_MAX} more")
        if r.get('slides'):
            lines.append(f"      slides: {slides_summary(r['slides'])}")
        for d in r.get('discarded', []):
            lines.append(f"      discarded: {d['trace']} ({d.get('cycles', '?')} cycle(s)): " + '; '.join(d.get('why') or []))
    return '\n'.join(lines)


def slides_summary(slides):
    """`N slide(s), fewest steps S: 17 steps in 150 ms for 110; ...`: what the 120 Hz mode logged, the distinct slides in the order they came (six
    at most). A slide of D ms at 120 Hz has about D / 8.33 steps; at 60, half that."""
    seen = []
    for steps, ms, asked in slides:
        item = f'{steps} steps in {ms} ms for {asked}'
        if item not in seen:
            seen.append(item)
    return f'{len(slides)} slide(s), fewest steps {min(s[0] for s in slides)}: ' + '; '.join(seen[:6])


def discard_summary(results):
    """The captures measured again, and why: a line each is in format_lines; this is the run's total."""
    gone = [d for r in results for d in r.get('discarded', [])]
    dist = [d for d in gone if d.get('disturbed')]
    left = sum(1 for r in results for c in r['checks'] if c['name'] == 'undisturbed')
    text = (f"DISCARDED {len(gone)} capture(s), {sum(d.get('cycles') or 0 for d in gone)} cycle(s), measured again: {len(dist)} disturbed "
            f"(a pop-up, note or toast), {len(gone) - len(dist)} with an app's own drawing")
    return text + (f"; {left} surface(s) still disturbed after {DISTURBED_RETRIES} retries" if left else '')


def format_results(results):
    return format_lines(results) + f'\nVERDICT {verdict(results)}'


def jsonable(v):
    """v with NaN as null, so the --json file stays valid JSON."""
    if isinstance(v, float) and math.isnan(v):
        return None
    if isinstance(v, dict):
        return {k: jsonable(x) for k, x in v.items()}
    if isinstance(v, (list, tuple)):
        return [jsonable(x) for x in v]
    return v


WINDOW_KEYS = ('name', 'dur_s', 'frames', 'fps', 'over20', 'worst_ms', 'anim_frames', 'dropped', 'dropped_in_motion', 'anim_render_ms',
               'first_frame_ms', 'burst_end_ms', 'burst_over20', 'burst_anim_p90_ms', 'sent_ms', 'delivered_ms', 'show_ms', 'osd_wake_ms',
               'burst_first_render_ms', 'burst_anim_samples', 'burst_app_jobs', 'irq_counts', 'gaps', 'panel_hz', 'drop_ms', 'ring_judged', 'ring_dropped',
               'ring_burst_over20', 'trace_from_ms')


def app_drawn(report, name):
    """The windows of surface `name` whose burst held an app's own drawing."""
    return [{'name': w['name'], 'burst_end_ms': w.get('burst_end_ms'), 'burst_app_jobs': w['burst_app_jobs']} for w in report['windows']
            if w['name'] in (f'{name}-open', f'{name}-close') and w.get('burst_app_jobs')]


def device_open_in(listing, device):
    """Whether an `ls -l /proc/PID/fd` listing has `device` open."""
    return any(line.rstrip().endswith(f' -> {device}') for line in listing.splitlines())


# ------------------------------------------------------------------- the set

class Rig:
    """The set, through run.py's Set: shell, capture and trace transfer."""

    def __init__(self):
        import run  # paramiko; only when there is a set to talk to
        self.run = run
        self.set = run.Set()
        self.hold = None

    @property
    def sm_pid(self):
        return self.set.sm_pid

    def sh(self, cmd, timeout=120):
        return self.set.sh(cmd, timeout=timeout)

    def push_tools(self):
        self.sh('mkdir -p /tmp/perf')
        for local, remote, mode in ((os.path.join(HERE, 'kb.py'), KB, '755'), (os.path.join(HERE, 'capture.sh'), CAPTURE, '755'),
                                    (os.path.join(HERE, 'sample.py'), SAMPLER, '644')):
            with open(local, 'rb') as f:
                data = f.read()
            stdin, out, _ = self.set.cli.exec_command(f'cat > {remote} && chmod {mode} {remote}')
            stdin.write(data)
            stdin.channel.shutdown_write()
            if out.channel.recv_exit_status() != 0:
                raise RuntimeError(f'could not copy {local} to the set')

    def load_start(self, kinds, seconds):
        """Starts the background load for `seconds` (the gate's whole run is the limit it must outlast); False when it did not start."""
        self.sh(f'mkdir -p {os.path.dirname(LOAD_LOG)}')
        self.sh(f'({LOADGEN} run {int(seconds)} --only {",".join(kinds)} --download-mib 48 > {LOAD_LOG} 2>&1 &)')
        # In a shell of its own: this one's command line has no `lumatv-loadgen run` for pgrep to find (the start's has).
        rc, out, err = self.sh('sleep 1; pgrep -f "lumatv-loadge[n] run"')
        return rc == 0 and bool(out.split())

    def load_stop(self):
        """Ends the background load and returns the generator's summary line (what each workload did), '' when it printed none."""
        self.sh('pkill -TERM -f "lumatv-loadge[n] run"; sleep 2; pkill -KILL -f "lumatv-loadge[n] run"; true')
        rc, out, err = self.sh(f'tail -n 1 {LOAD_LOG}')
        return out.strip() if rc == 0 else ''

    def motion_state(self):
        """Home's own motion switches as the owner's theme.json has them, by theme path (MOTION_SWITCHES): True, False, or None when the file does not
        say (the shipped default, on). `theme get` answers {"ok": true, "theme": {...}}."""
        rc, out, _ = self.sh(f'{TOOL} theme get')
        try:
            doc = json.loads(out)
            theme = doc.get('theme', doc)
            return {path: (theme.get(path.split('.')[0]) or {}).get(path.split('.')[1]) for path in MOTION_SWITCHES}
        except (ValueError, AttributeError):
            return {path: None for path in MOTION_SWITCHES}

    def transitions_stats(self):
        """The compositor's transition counters now (parse_transitions)."""
        rc, out, err = self.sh(CTL_TRANSITIONS)
        if rc != 0:
            raise RuntimeError(f'ctl transitions failed: {err.strip() or out.strip()}')
        try:
            return parse_transitions(out)
        except (ValueError, KeyError, TypeError):
            raise RuntimeError(f'ctl transitions answered {out.strip()[:80]!r}')

    def keys_read(self):
        """(device, read): the device kb.py writes the gate's keys to, and whether LumaTV's compositor has it open; `read` is None where there is no
        such compositor or device to ask. A device lginput2 made after the compositor started was never opened: after the set's first wake the keys
        went nowhere, the saver came up and the gate timed its sweep (2026-11-04: 69 to 819 ms to the first frame, against 1 to 10 on 2026-10-30)."""
        rc, out, _ = self.sh(f'python3 {KB} --device')
        device = out.strip()
        if rc != 0 or not device.startswith('/dev/input/'):
            return None, None
        rc, out, _ = self.sh("pids=$(pgrep -f 'bin/[l]umatv-compositor --'); "
                             "if [ -z \"$pids\" ]; then echo NO_COMPOSITOR; else for p in $pids; do ls -l /proc/$p/fd 2>/dev/null; done; fi")
        if 'NO_COMPOSITOR' in out:
            return device, None
        return device, device_open_in(out, device)

    def panel_hz(self, seconds=2):
        """The refresh the panel runs at now, Hz, from the display's vsync interrupts over `seconds` (`DE2VSYNC` in /proc/interrupts, one a refresh, all on
        CPU 0); None where the set lists none. `ctl refresh` says what the compositor asked for, not what the TV runs: it re-times the panel to 100 Hz
        while a camera popup or a video window is up and keeps it there (2026-11-04: 99.9 Hz through a 23 minute run asked at 120)."""
        rc, out, _ = self.sh("n() { awk '/DE2VSYNC/ { for (i = 2; i <= 5; i++) if ($i ~ /^[0-9]+$/) s += $i; print s + 0 }' /proc/interrupts; }; "
                             f'a=$(n); sleep {seconds}; echo $a $(n)')
        try:
            before, after = (int(x) for x in out.split())
        except ValueError:
            return None
        return (after - before) / seconds

    def save_ring(self, name):
        """The compositor's trace ring as it stands (a cycle's own: taken before the next one), into <name>.ring.json beside the trace. The path, or None
        under LG's compositor (it has no ring) or when the set answered no ring."""
        if self.lg_compositor():
            return None
        rc, out, _ = self.sh(f'{CTL_RING} {RING_LINES}', timeout=60)
        if rc != 0 or '"lines"' not in out:
            return None
        os.makedirs(self.run.LOCAL, exist_ok=True)
        path = f'{self.run.LOCAL}/{name}.ring.json'
        with open(path, 'w') as f:
            f.write(out)
        return path

    def lumatv_state(self):
        """{'refresh': ..., 'switcher': ...}: the compositor's `ctl refresh` and `ctl switcher` answers, each None when the bundle's ctl gave no JSON;
        None under LG's compositor, which has neither."""
        if self.lg_compositor():
            return None
        state = {}
        for word in ('refresh', 'switcher'):
            rc, out, _ = self.sh(f'{CTL} {word}', timeout=30)
            try:
                state[word] = json.loads(out[out.index('{'):]) if rc == 0 else None
            except ValueError:
                state[word] = None
        return state

    def set_motion(self, changes):
        """Switches Home's own motion by theme path ({'weather.animated': False}); Home reads the file every 2 s, so wait 3."""
        patch = {}
        for path, on in changes.items():
            group, key = path.split('.')
            patch.setdefault(group, {})[key] = bool(on)
        self.sh(TOOL + ' theme set ' + shlex.quote(json.dumps(patch)))
        time.sleep(3)

    def remember_compositor(self, pid):
        self.set.sm_pid = pid

    def lg_compositor(self):
        """True while LG's surface-manager draws; False when LumaTV's compositor is the live one."""
        rc, out, _ = self.sh('pgrep -f bin/lumatv-compositor')
        return not (rc == 0 and out.split())

    def compositor_pid(self):
        rc, out, _ = self.sh('pidof surface-manager')
        return int(out.split()[0]) if rc == 0 and out.split() else None

    def compositor_age_s(self):
        """Seconds since the compositor started, None when it cannot be read."""
        rc, out, _ = self.sh("p=$(pidof surface-manager | cut -d' ' -f1); "
                             "echo $(cut -d' ' -f1 /proc/uptime) $(cut -d' ' -f22 /proc/$p/stat) $(getconf CLK_TCK)")
        try:
            up, start, hz = (float(x) for x in out.split())
            return up - start / hz
        except ValueError:
            return None

    def wake(self):
        """Home's screensaver and idle dim take the first key, which wakes them and does nothing else: the first gear tap of a run after
        5 minutes of quiet opened no panel (round 16). One Back, eaten by the saver or harmless on a Home with nothing open, first. With the
        owner's "Recent apps on Back" (theme `remote.recents`) on and an app running, Back from the top of Home opens the app switcher, which
        then takes every key of the run; a second Back closes it."""
        self.sh(f'python3 {KB} BACK')
        time.sleep(2)
        state = self.lumatv_state()
        if state and (state.get('switcher') or {}).get('open'):
            self.sh(f'python3 {KB} BACK')
            time.sleep(1)

    def capture(self, name, mode):
        self.set.capture(name, mode)
        time.sleep(1)

    def stop(self, name):
        """Stop the capture; the local (trace, sys) paths."""
        self.set.stop(name)
        return f'{self.run.LOCAL}/{name}.trace', f'{self.run.LOCAL}/{name}.sys'

    def analyse(self, name):
        return self.run.analyse(name, self.set.sm_pid, GAP_MS * 60.0 / REFRESH_HZ, REFRESH_HZ)

    def helper(self, method, path, body=None):
        """(HTTP status, JSON answer) of the helper's loopback API from the set; (0, {}) when it did not answer. Writes carry X-Luma-Client."""
        cmd = f"curl -s -m 10 -w '\\n%{{http_code}}' -X {method} -H 'X-Luma-Client: gate' -H 'Content-Type: application/json' "
        if body is not None:
            cmd += f'-d {shlex.quote(json.dumps(body))} '
        rc, out, _ = self.sh(cmd + shlex.quote(HELPER_API + path), timeout=30)
        text, _, code = out.rpartition('\n')
        try:
            return int(code), (json.loads(text) if text.strip() else {})
        except ValueError:
            return 0, {}

    def hold_start(self):
        """Snoozes every camera's alerts for the run; the line to print. A set whose helper has no snooze route (older than 2026-11-08) is measured
        without, with the captures still checked for a pop-up."""
        self.hold = CameraHold(self.helper)
        try:
            return self.hold.start()
        except Exception as e:
            self.hold = None
            return f'cameras NOT snoozed for the run ({type(e).__name__}: {e}); pop-ups are only detected'

    def hold_renew(self):
        if self.hold:
            self.hold.renew()

    def hold_stop(self):
        hold, self.hold = self.hold, None
        return hold.stop() if hold else 'camera snooze: none was set'

    def shell_noise(self, toasts=True):
        """Why the shell shows a pop-up or a toast now ([] when it shows none, or when there is no shell to ask: LG's compositor); `toasts` False
        for a surface whose own cycle raises a toast."""
        rc, out, _ = self.sh(f'{CTL} shell', timeout=30)
        try:
            return ui_noise(json.loads(out[out.index('{'):]).get('ui') or {}, toasts) if rc == 0 else []
        except ValueError:
            return []

    def feed_mark(self):
        """Where the helper's event feed is now, for feed_since; None when it does not answer."""
        status, doc = self.helper('GET', '/events?timeout=0')
        return {'boot': doc.get('boot'), 'seq': doc.get('seq')} if status == 200 and doc.get('seq') is not None else None

    def feed_since(self, mark):
        """The pop-ups and notes the helper published since feed_mark (feed_disturbances); none when there was no mark."""
        if mark is None:
            return []
        status, doc = self.helper('GET', f"/events?after={mark['seq']}&boot={mark['boot']}&timeout=0")
        if status != 200:
            return [f'the helper feed did not answer (HTTP {status})']
        if doc.get('reset'):
            return ['the helper restarted or its event ring overran: what came up is unknown']
        return feed_disturbances(doc.get('events') or [])


def ui_noise(ui, toasts=True):
    """What the shell's state (`ctl shell`'s `ui`) says is on screen to disturb a capture: a camera pop-up, a toast."""
    why = []
    popup = ui.get('popup') or {}
    if popup.get('up'):
        why.append(f"a camera pop-up is up (camera {popup.get('camera')}, {popup.get('kind')})")
    if toasts and ui.get('toasts'):
        why.append('a toast is up')
    return why


def feed_disturbances(events):
    """One line for each camera pop-up and plugin note in the helper's events (/v1/events). A pop-up the helper holds is never published, and a
    pop-up's later updates carry the same id."""
    why, seen = [], set()
    for e in events:
        topic, data = e.get('topic') or '', e.get('data')
        if topic == 'cameras.popup' and isinstance(data, dict) and data:
            if (data.get('camera'), data.get('id')) not in seen:
                seen.add((data.get('camera'), data.get('id')))
                why.append(f"camera pop-up: camera {data.get('camera')}, {data.get('kind')}")
        elif topic.startswith('plugin.') and topic.endswith('.notifications') and e.get('kind') == 'posted' and isinstance(data, dict):
            title = (data.get('post') or {}).get('title') or data.get('channel')
            why.append(f"note from {topic.split('.')[1]}: {title}")
    return why


def settle_quiet(rig, toasts=True):
    """Waits up to QUIET_WAIT_S for a pop-up or toast already up to go; what is still up then."""
    end = time.monotonic() + QUIET_WAIT_S
    while True:
        noise = rig.shell_noise(toasts)
        if not noise or time.monotonic() >= end:
            return noise
        time.sleep(1)


@contextlib.contextmanager
def signals_ignored():
    """No Ctrl-C or kill while the run's settings are being put back."""
    sigs = (signal.SIGINT, signal.SIGTERM, signal.SIGHUP)
    try:
        old = {s: signal.signal(s, signal.SIG_IGN) for s in sigs}
    except ValueError:      # not the main thread
        yield
        return
    try:
        yield
    finally:
        for s, handler in old.items():
            signal.signal(s, handler)


class CameraHold:
    """A snooze of every camera's alert (the helper's `/v1/dnd/snooze`, kind `camera`, no source) for the run, put back as it was found. It does
    not touch Do Not Disturb, whose turning on empties the owner's missed list, nor the snoozes of single cameras. An owner's snooze of every
    camera is what ours replaces: it is posted again, with its own end, by stop(). One left by an earlier gate that died (the same label) is not
    the owner's: stop() ends it. `call(method, path, body)` is Rig.helper."""

    def __init__(self, call, clock=time.time):
        self.call, self.clock = call, clock
        self.before = self.prev = None
        self.posted = 0.0

    @staticmethod
    def whole(snooze):
        return snooze.get('kind') == 'camera' and not snooze.get('source')

    def _state(self):
        status, doc = self.call('GET', '/dnd')
        if status != 200 or not isinstance(doc.get('snoozes'), list):
            raise RuntimeError(f'GET /v1/dnd answered {status}')
        return doc

    def _post(self, body):
        status, doc = self.call('POST', '/dnd/snooze', body)
        if status != 200:
            raise RuntimeError(f"POST /v1/dnd/snooze answered {status}: {str(doc.get('error') or doc)[:100]}")

    def _hold(self):
        self._post({'kind': 'camera', 'label': HOLD_LABEL, 'minutes': HOLD_MINUTES})
        self.posted = self.clock()

    def start(self):
        self.before = self._state()
        self.prev = next((s for s in self.before['snoozes'] if self.whole(s) and s.get('label') != HOLD_LABEL), None)
        self._hold()
        return (f'cameras snoozed for the run (every camera, {HOLD_MINUTES} min, renewed); '
                + ("the owner's snooze of every camera is put back after" if self.prev else 'no snooze of every camera was on'))

    def renew(self):
        """Posts the snooze again once half of it has gone; the same scope, so it replaces itself."""
        if self.posted and self.clock() - self.posted > HOLD_MINUTES * 30:
            self._hold()

    @staticmethod
    def _key(state, now_ms):
        return sorted((s.get('kind'), s.get('source') or '', s.get('label') or '', s.get('until')) for s in state['snoozes']
                      if s.get('label') != HOLD_LABEL and (s.get('until') or 0) > now_ms)

    def stop(self):
        """Ends our snooze and posts the owner's again; the line to print, which says whether the state is as it was found."""
        try:
            for s in self._state()['snoozes']:
                if self.whole(s) and s.get('label') == HOLD_LABEL:
                    status, _ = self.call('DELETE', f"/dnd/snooze/{s['id']}")
                    if status != 200:
                        raise RuntimeError(f"DELETE of the snooze {s['id']} answered {status}")
            now_ms = int(self.clock() * 1000)
            if self.prev and (self.prev.get('until') or 0) > now_ms:
                self._post({'kind': 'camera', 'label': self.prev.get('label') or '', 'until': self.prev['until']})
            after = self._state()
        except Exception as e:
            return f'FAIL camera snooze NOT put back ({type(e).__name__}: {e}): end the "{HOLD_LABEL}" snooze in /v1/dnd'
        same = (self._key(after, now_ms) == self._key(self.before, now_ms)
                and all(after.get(k) == self.before.get(k) for k in ('on', 'until', 'letDoorbell')))
        return 'camera snooze put back as found' + ('' if same else '; FAIL the snoozes or Do Not Disturb differ from before the run: ' + json.dumps(
            {'before': self._key(self.before, now_ms), 'after': self._key(after, now_ms)}))


class LogWatch:
    """What /var/log/messages gained since mark(). The file rotates about every minute: when the inode changed or the file shrank, the file and
    the ones it rotated to (not the compressed ones) are read, oldest first, and cut by the bracketed monotonic timestamp against /proc/uptime at the
    mark, so that what was logged before the rotation is not lost with it."""

    def __init__(self, rig):
        self.rig = rig

    def mark(self):
        _, out, _ = self.rig.sh(f"stat -c '%s %i' {LOG_FILE} 2>/dev/null; cut -d' ' -f1 /proc/uptime")
        rows = out.split('\n')
        try:
            size, inode = rows[0].split()
            return {'size': int(size), 'inode': inode, 'uptime': float(rows[1])}
        except (ValueError, IndexError):
            return {'size': 0, 'inode': None, 'uptime': 0.0}

    def since(self, mark):
        """(lines, filtered): new lines; filtered False when they could not be cut by time."""
        _, out, _ = self.rig.sh(f"stat -c '%s %i' {LOG_FILE} 2>/dev/null")
        try:
            size, inode = out.split()
            size = int(size)
        except ValueError:
            return [], True
        if inode == mark['inode'] and size >= mark['size']:
            _, text, _ = self.rig.sh(f"tail -c +{mark['size'] + 1} {LOG_FILE}")
            return text.splitlines(), True
        _, text, _ = self.rig.sh(f"for f in $(ls -tr {LOG_FILE} {LOG_FILE}.* 2>/dev/null); do case $f in *.gz|*.xz|*.bz2) ;; *) cat $f ;; esac; done")
        return lines_since(text, mark['uptime'])


def read(path):
    with open(path, errors='replace') as f:
        return f.read()


def attach_ring_motion(report, ring_path):
    """Gives each window of an analysis the compositor's own count of the refreshes its motions missed (`ring_dropped` of `ring_judged` flips, from the
    ring of the capture's cycle: ringsum.motion_drops), and of the intervals of its mark's burst that a motion missed (`ring_burst_over20`:
    ringsum.burst_drops). The trace can only guess which frames were animated (a key's response frame, a frame the idle tick drew, the last frame
    of a motion and the first of the next key all look the same, and a frame the compositor draws ahead of its refresh follows no commit), the
    compositor knows. A window the ring does not reach gets none."""
    try:
        flips = ringsum.flips(ringsum.load(ring_path))
    except (OSError, ValueError, KeyError, TypeError):
        return
    for w in report['windows']:
        drop_ms = w.get('drop_ms') or analyse_drop_ms()
        got = ringsum.motion_drops(flips, w['start'], w['start'] + w['dur_s'], drop_ms)
        if got is not None:
            w['ring_judged'], w['ring_dropped'] = got
        over = ringsum.burst_drops(flips, w['start'], drop_ms)
        if over is not None:
            w['ring_burst_over20'] = over


def analyse_drop_ms():
    """The drop threshold of the refresh that was asked for (a refresh and a fifth)."""
    return round(1.2 * 1000.0 / REFRESH_HZ, 2)


def merge_reports(reports):
    """One report whose windows are those of several captures, in order."""
    out = dict(reports[0])
    out['windows'] = [w for r in reports for w in r['windows']]
    out['trace'] = [r['trace'] for r in reports]
    return out


FRAME_EVENT = re.compile(r'^\s*.*?-\d+\s+\[\d{3}\]\s+\S+\s+([\d.]+):\s+drm_vblank_event_delivered:')


def frame_times(text):
    """Presentation time of every frame in a trace dump, sorted."""
    return sorted(float(m.group(1)) for m in map(FRAME_EVENT.match, text.splitlines()) if m)


def apply_delivery(report, text, name, window_s=2.0):
    """For a surface a luna call opens: its `<name>-open` windows' first_frame_ms, burst_end_ms and burst_over20 are measured from the
    moment the shell was handed the notification instead of from the marker at the call's spawn: the compositor's own
    `Q:<name>-delivered` (the service's push reached it) when it wrote one, else `Q:<name>-sent` (luna-send returned, which is all LG's
    compositor offers). sent_ms and delivered_ms (spawn to each) are recorded. A window with neither marker after it gets no first
    frame. Mutates the report's windows."""
    import analyse
    marks = parse_markers(text)
    opens = [t for t, txt in marks if txt == f'P:{name}-open']
    sents = [t for t, txt in marks if txt == f'Q:{name}-sent']
    delivered = [t for t, txt in marks if txt == f'Q:{name}-delivered']
    frames = frame_times(text)
    windows = [w for w in report['windows'] if w['name'] == f'{name}-open']
    for w, t0 in zip(windows, opens):
        nxt = next((t for t in opens if t > t0), None)

        def first_in_window(times):
            return next((t for t in times if t >= t0 and (nxt is None or t < nxt)), None)
        sent, got = first_in_window(sents), first_in_window(delivered)
        w['sent_ms'] = round((sent - t0) * 1000.0, 2) if sent is not None else None
        w['delivered_ms'] = round((got - t0) * 1000.0, 2) if got is not None else None
        start = got if got is not None else sent
        if start is None:
            w['first_frame_ms'] = w['burst_end_ms'] = None
            continue
        later = [f for f in frames if start <= f < start + window_s and (nxt is None or f < nxt)]
        b = analyse.burst(later, start)
        w['first_frame_ms'], w['burst_end_ms'], w['burst_over20'] = b['first_frame_ms'], b['burst_end_ms'], b['over20']
    return report


def annotate_shows(report, text, name):
    """For each window of surface `name` (`<name>-open`, `<name>-close`): show_ms, the time from its marker to the compositor's own
    `Q:<name>-show` (the instant it asked the shell to show the panel the key opens), None when it wrote none inside the window (a
    key that goes straight to the client, a compositor that is not LumaTV's). The first frame minus this is the shell's part.
    Mutates the report's windows."""
    marks = parse_markers(text)
    phases = [(t, txt[2:]) for t, txt in marks if txt.startswith('P:')]
    shows = [t for t, txt in marks if txt == f'Q:{name}-show']
    for kind in ('open', 'close'):
        windows = [w for w in report['windows'] if w['name'] == f'{name}-{kind}']
        spans = [(t, phases[i + 1][0] if i + 1 < len(phases) else None) for i, (t, nm) in enumerate(phases) if nm == f'{name}-{kind}']
        for w, (t0, t1) in zip(windows, spans):
            at = next((t for t in shows if t >= t0 and (t1 is None or t < t1)), None)
            w['show_ms'] = round((at - t0) * 1000.0, 2) if at is not None else None
    return report


def run_surface(rig, s, tag, soak_cycles, log):
    base = f'gate-{tag}-{s["name"]}'
    res = {'name': s['name'], 'checks': [], 'traces': [], 'windows': [], 'log_lines': [], 'log_ignored': 0,
           'log_unfiltered': False, 'rss': None, 'discarded': [], 'slides': [], 'rings': []}
    checks = res['checks']
    pid0 = rig.compositor_pid()
    saved = []

    def scan(mark):
        lines, filtered = log.since(mark)
        bad, ignored = log_failures(lines)
        res['slides'] += slides_in(lines)
        res['log_lines'] += bad
        res['log_ignored'] += ignored
        res['log_unfiltered'] = res['log_unfiltered'] or not filtered

    try:
        mark = log.mark()
        s, how = resolve_tile(rig, s)
        if how:
            checks.append(check('launch_path', True, None, '', how))
        set_switches(rig, s['switches'], saved)
        for cmd in s['setup']:
            rc, _, err = rig.sh(cmd)
            if rc != 0:
                raise RuntimeError(f'setup {cmd!r} failed: {err.strip()}')

        reports = []
        retries = APP_RETRIES if s['app_retries'] is None else s['app_retries']
        before = rig.transitions_stats() if s['ctl_transitions'] else None
        for i, n in enumerate(plan_full(s, MEASURE_CYCLES)):
            app_left, quiet_left, attempt = retries, DISTURBED_RETRIES, 0
            while True:
                name = f'{base}-m{i}' + (f'-r{attempt}' if attempt else '')
                res['traces'].append(name)
                rig.hold_renew()
                toasts = '!toast' not in str(s['open'])         # the toast surface raises its own
                start_noise = settle_quiet(rig, toasts)
                feed = rig.feed_mark()
                rig.capture(name, 'full')
                trace = ring = None
                try:
                    rc, _, err = rig.sh(cycle_command(s, n), timeout=3600)
                    ring = rig.save_ring(name)
                    # a pop-up or note the helper published while the keys ran, or one still on screen at either end
                    disturbed = ([f'at the start: {w}' for w in start_noise] + rig.feed_since(feed) + [f'at the end: {w}' for w in rig.shell_noise(toasts)])
                finally:
                    trace, _sysfile = rig.stop(name)
                if ring:
                    try:
                        res['rings'].append(ringsum.line(name, ringsum.summarize(ringsum.load(ring))))
                    except (OSError, ValueError, KeyError, TypeError) as e:
                        res['rings'].append(f'{name}: the ring could not be read ({e})')
                if rc != 0:
                    checks.append(check('keys', False, rc, '== 0', f'kb.py exited {rc}: {err.strip()}'))
                one = rig.analyse(name)
                if ring:
                    attach_ring_motion(one, ring)
                text = read(trace)
                annotate_shows(one, text, s['name'])
                if s['delivery']:
                    apply_delivery(one, text, s['name'])
                drawn = app_drawn(one, s['name'])
                if disturbed and quiet_left:
                    quiet_left -= 1
                elif drawn and not disturbed and app_left:
                    app_left -= 1
                else:
                    break
                res['discarded'].append({'trace': name, 'cycles': n, 'disturbed': bool(disturbed), 'windows': drawn,
                                         'why': disturbed or ["an app's own drawing in the burst"]})
                attempt += 1
            if disturbed:       # the retries were disturbed too: the last capture is kept and the surface fails on that, not on its numbers
                checks.append(check('undisturbed', False, len(disturbed), '== 0', f'{attempt + 1} captures disturbed, the last kept: ' + '; '.join(disturbed)))
            reports.append(one)
        report = merge_reports(reports)
        lg = rig.lg_compositor()
        checks += evaluate_windows(s, report, MEASURE_CYCLES, lg, lumatv=lg is False)
        if before is not None:
            res['transitions'] = {'before': before, 'after': rig.transitions_stats()}
            checks += transitions_checks(before, res['transitions']['after'], MEASURE_CYCLES)
        res['windows'] = [{k: w.get(k) for k in WINDOW_KEYS} for w in report['windows']
                          if w['name'] in (f"{s['name']}-open", f"{s['name']}-close")]
        scan(mark)

        if s['soak'] and soak_cycles:
            series, ends = [], []
            for i, n in enumerate(split_cycles(soak_cycles, SOAK_TRACE_MAX)):
                name = f'{base}-s{i}'
                res['traces'].append(name)
                mark = log.mark()
                rig.capture(name, 'basic')
                try:
                    rc, _, err = rig.sh(cycle_command(s, n), timeout=3600)
                finally:
                    trace, sysf = rig.stop(name)
                if rc != 0:
                    checks.append(check('keys', False, rc, '== 0', f'kb.py exited {rc}: {err.strip()}'))
                series += parse_rss(read(sysf))
                ends += cycle_ends(parse_markers(read(trace)), s['name'])
                scan(mark)
            res['rss'] = rss_growth(series, ends)
            if len(ends) != soak_cycles:
                checks.append(check('soak_cycles', False, len(ends), f'== {soak_cycles}', 'markers lost in the soak traces'))
            checks.append(rss_check(res['rss']))
    except Exception as e:  # a surface that cannot be measured fails; the next one still runs
        checks.append(check('run', False, None, 'no error', f'{type(e).__name__}: {e}'))
    finally:
        for cmd in s['teardown']:
            try:
                rc, _, err = rig.sh(cmd)
                if rc != 0:
                    raise RuntimeError(f'exited {rc}: {err.strip()}')
            except Exception as e:
                checks.append(check('teardown', False, None, 'no error', f'{cmd!r}: {e}'))
        if s['inputs_free']:
            checks.append(inputs_free_check(rig))
        if saved:
            problems = restore_switches(rig, saved)
            checks.append(check('switches_restored', not problems, ', '.join(n for n, _ in saved), 'as found', problems))
    checks.append(log_check(res['log_lines'], res['log_ignored']))
    if res['log_unfiltered']:
        checks[-1]['detail'] = (checks[-1]['detail'] + '; ' if checks[-1]['detail'] else '') + 'rotated log had no timestamps, whole file scanned'
    pid1 = rig.compositor_pid()
    checks.append(check('compositor_pid', pid0 is not None and pid0 == pid1, pid1, f'== {pid0}',
                        'the compositor restarted during the run' if pid0 != pid1 else ''))
    if pid1 is not None:
        rig.remember_compositor(pid1)
    return res


def kwait_lines(results, per_surface=12):
    """The kernel waits behind each late frame the surfaces' full captures found (`kwait.py`, through analyse.py): one line a surface with
    the counts by class, and a line for each late frame, the ones it could say most about first. A surface with no late frame has none."""
    out = []
    for r in results:
        late = [(w['name'], g) for w in r.get('windows', []) for g in (w.get('gaps') or []) if g.get('kwait')]
        if not late:
            continue
        out.append(f"{r['name']}: " + kwait.summary_line(kwait.summarize([g['kwait'] for _, g in late])))
        late.sort(key=lambda x: (not x[1]['kwait']['kernel'], -x[1]['kwait']['kernel_ms']))
        for name, g in late[:per_surface]:
            out.append(f"    {name} +{g['at']} s: {kwait.describe(g['kwait'])}")
        if len(late) > per_surface:
            out.append(f'    ... {len(late) - per_surface} more late frames')
    return '\n'.join(out)


def unfit(state, hz):
    """Why LumaTV's compositor is in no state to be measured, None when it is or when there is nothing to ask (`state` None: LG's compositor). `state` is
    Rig.lumatv_state()'s; `hz` the refresh the gate was asked for. On 2026-11-07 two runs failed six surfaces of seven on states like these: /tmp/lumatv
    without the bundle after a reboot (no ring, and `gate.sh refresh 120` had failed, so the automatic policy moved the panel between 60 and 120), then
    the wake's Back opening the app switcher over Netflix, which took Home's keys (a first frame 352 ms after the key: the clock's next second)."""
    if state is None:
        return None
    refresh, switcher = state.get('refresh'), state.get('switcher')
    if refresh is None or switcher is None:
        return ("the compositor's ctl from /tmp/lumatv did not answer (bin and lib are gone after a reboot until the bundle is deployed again): the ring "
                'cannot be read, so dropped frames would be judged from the trace alone, and `gate.sh refresh` cannot have set the refresh; deploy the bundle, '
                'or link /tmp/lumatv/bin and lib to the installed one, and start again')
    if switcher.get('open'):
        apps = ', '.join(map(str, switcher.get('apps') or []))
        return (f'the app switcher is up (over {apps or "no app"}) after the wake closed it: it takes the keys, and the surfaces would walk its cards; '
                'close the switcher and start again')
    if hz != 60 and (refresh.get('mode') != 'manual' or refresh.get('hz') != hz):
        return (f"the compositor's refresh is {refresh.get('mode')} at {refresh.get('hz')} Hz and the gate was asked for {hz:g}: the automatic policy moves "
                f'between 60 and 120 with what is on screen; `gate.sh refresh {hz:g}` first')
    return None


def run_gate(rig, surfaces, soak_cycles, tag, log=None, load=None, load_seconds=3600):
    """measure_surfaces with the cameras' alerts snoozed for the run and put back as found when it ends, however it ends."""
    try:
        print(rig.hold_start(), flush=True)
        return measure_surfaces(rig, surfaces, soak_cycles, tag, log, load, load_seconds)
    finally:
        with signals_ignored():
            print(rig.hold_stop(), flush=True)


def measure_surfaces(rig, surfaces, soak_cycles, tag, log=None, load=None, load_seconds=3600):
    rig.push_tools()
    log = log or LogWatch(rig)
    results = []
    age = rig.compositor_age_s()
    if age is not None and age < COMPOSITOR_SETTLE_S:
        time.sleep(COMPOSITOR_SETTLE_S - age)
    rig.wake()
    device, read = rig.keys_read()
    if read is False:
        raise RuntimeError(f'the compositor has not opened {device}, where the gate injects its keys: they would go nowhere and the gate would time the saver; '
                           'run a compositor that opens devices made after its start (the changelog of 2026-11-04), or restart it')
    why = unfit(rig.lumatv_state(), REFRESH_HZ)
    if why:
        raise RuntimeError(why)
    hz = rig.panel_hz()
    if hz is not None and abs(hz - REFRESH_HZ) > PANEL_TOLERANCE * REFRESH_HZ:
        raise RuntimeError(f'the panel runs at {hz:.1f} Hz and the gate was asked for {REFRESH_HZ:g}: every window would be judged by the wrong refresh. '
                           'The TV re-times it (to 100 Hz after a camera popup or a video window) and only a 30 or 60 fps video, played for a few seconds, brings the '
                           'rest back (docs/research/vwin-refresh-result.md); then `gate.sh refresh <hz>`, check DE2VSYNC/s, and start again')
    loaded = False
    if load:
        loaded = rig.load_start(load, load_seconds)
        print(f"background load {','.join(load)}: {'started' if loaded else 'DID NOT START (the bundle has no bin/lumatv-loadgen?); measuring without it'}", flush=True)
    original = rig.motion_state()         # per switch; None: the file says nothing, the shipped default (on) applies
    owner = {path: value is not False for path, value in original.items()}
    now = dict(owner)
    try:
        for s in surfaces:
            want = owner if not s['still'] else {path: False for path in owner}
            changes = {path: on for path, on in want.items() if on != now[path]}
            if changes:
                rig.set_motion(changes)
                now.update(changes)
            r = run_surface(rig, s, tag, soak_cycles, log)
            print(format_lines([r]), flush=True)
            for ring_line in r.get('rings', []):
                print(ring_line, flush=True)
            late = kwait_lines([r])
            if late:
                print(late, flush=True)
            results.append(r)
    finally:
        if loaded:
            print(f'background load ended: {rig.load_stop() or "no summary"}', flush=True)
        back = {path: on for path, on in owner.items() if on != now[path]}
        if back:
            rig.set_motion(back)
    return results


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0],
                                 epilog='Surfaces: ' + ', '.join(e['name'] for e in SURFACES))
    ap.add_argument('--surfaces', help='comma-separated names (default: all)')
    ap.add_argument('--cycles', type=int, help=f'open/close cycles in the RSS soak (default {SOAK_DEFAULT_CYCLES}, '
                    f'{SOAK_RELEASE_CYCLES} with --soak; at least {RSS_SKIP_CYCLES + 1})')
    ap.add_argument('--soak', action='store_true', help=f'release run: {SOAK_RELEASE_CYCLES} cycles, traces of at most {SOAK_TRACE_MAX}')
    ap.add_argument('--json', help='write every number to this file')
    ap.add_argument('--input-key', help='the evdev code of the remote\'s Input key, for the picker surfaces (see kb.py --learn)')
    ap.add_argument('--hdmi', default='3', help='the HDMI input the -hdmi surfaces put in front (default 3, the game console)')
    ap.add_argument('--tag', default=time.strftime('%m%d-%H%M'), help='prefix of the trace names in /tmp/perf')
    ap.add_argument('--load', metavar='KINDS', help=f'measure under synthetic background load, a comma-separated subset of {",".join(LOAD_KINDS)} (see LOADGEN): the gate '
                    'starts it after the compositor has settled and ends it with the run; the numbers are the same, the thresholds are not changed')
    ap.add_argument('--load-seconds', type=int, default=3600, help='the longest the load runs if the gate does not end it (default 3600)')
    ap.add_argument('--refresh', type=float, default=60.0, help='the refresh the output runs at, Hz (default 60): 120 for the compositor\'s refresh test mode, where an interval over 10 ms is a dropped frame')
    a = ap.parse_args(argv)
    cycles = a.cycles if a.cycles is not None else (SOAK_RELEASE_CYCLES if a.soak else SOAK_DEFAULT_CYCLES)
    if cycles <= RSS_SKIP_CYCLES:
        ap.error(f'--cycles must be at least {RSS_SKIP_CYCLES + 1}: the first {RSS_SKIP_CYCLES} are warm-up')
    validate(SURFACES)
    try:
        chosen = find(a.surfaces.split(',') if a.surfaces else [e['name'] for e in SURFACES if surface(e)['default']],
                      values={'INPUT': a.input_key, 'HDMI': a.hdmi})
    except ValueError as e:
        ap.error(str(e))
    tag = re.sub(r'[^\w.-]', '_', a.tag)
    load = [k for k in a.load.split(',') if k] if a.load else None
    if load and set(load) - set(LOAD_KINDS):
        ap.error(f'--load: unknown workload(s) {",".join(sorted(set(load) - set(LOAD_KINDS)))}; the workloads are {",".join(LOAD_KINDS)}')
    global REFRESH_HZ
    REFRESH_HZ = a.refresh
    import analyse
    analyse.set_refresh(a.refresh)
    for sig in (signal.SIGTERM, signal.SIGHUP):      # a kill unwinds run_gate's finally like Ctrl-C does, so the camera snooze is put back
        signal.signal(sig, lambda signum, _frame: sys.exit(128 + signum))
    results = run_gate(Rig(), chosen, cycles, tag, load=load, load_seconds=a.load_seconds)
    v = verdict(results)
    print(discard_summary(results))
    print(f'VERDICT {v}')
    if a.json:
        with open(a.json, 'w') as f:
            json.dump(jsonable({'tag': tag, 'verdict': v, 'measure_cycles': MEASURE_CYCLES, 'soak_cycles': cycles, 'refresh_hz': REFRESH_HZ,
                                'thresholds': THRESHOLDS, 'surfaces': results}), f, indent=1, default=str)
    return 0 if v == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
