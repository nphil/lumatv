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
                                 first animated frame after an idle gap is late by construction
  first frame after the key      <= 50 ms
  burst end after the key        <= 180 ms (opens and closes)
  compositor RSS over N cycles   flat, see rss_growth()
  /var/log/messages              no line matching LOG_FAIL_PATTERNS

A surface whose key is not consumed (no frame in its window) fails. The marker sits at the edge
the surface acts on: the press by default, the release for `act: 'release'` (LG's Quick Settings
opens on the release of the settings key, so a marker at the press would count the 50 ms the
injected tap is held). `still` switches the wallpaper's 10 Hz tick off for the surface (a tick
frame could otherwise be taken for the response; the user's setting is put back). `delivery` is for
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

Adding a surface is one entry in SURFACES (see the field list there).
"""
import argparse
import json
import math
import os
import re
import shlex
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
# The tool is not on the set's PATH.
TOOL = '/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv'

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
HDMI_TEARDOWN = [f"luna-send -n 1 {SAM}/closeByAppId '{{\"id\":\"com.webos.app.hdmi{{HDMI}}\"}}'",
                 f"luna-send -n 1 {SAM}/launch '{{\"id\":\"com.webos.app.home\"}}'", 'sleep 6']

# ------------------------------------------------------------------ surfaces
#
# name          unique; also the marker prefix: windows are "<name>-open" and "<name>-close"
# open, close   kb.py token strings. open starts with the token "@<name>-open" and close with
#               "@<name>-close", each placed immediately before the key it times. A close with
#               no key only ends the open window. With `act: 'release'` open is the marker and
#               one key token, and the gate presses the key 50 ms before the marker and releases
#               it at the marker.
# settle_s      seconds waited after open and after close before the next step
# close_settle_s  seconds waited after the close when that is not settle_s (a full capture's ring holds about FULL_RING_S: a cycle
#               longer than FULL_CAPTURE_MAX_S loses its first markers, and validate() says so)
# transition    True: the close key is a measured response too (checked like the open); False
#               for workloads on a surface that stays up (Home idle, D-pad, sweep)
# setup         shell commands run on the set once before the surface, teardown after it
#               (also on failure)
# reopen        shell commands run before every cycle, outside the measured windows, for a
#               surface the close key tears down for good (reopen_wait_s waits for it to build)
# soak          False when cycling it says nothing about memory (Home idle)
# act           'press' (default) or 'release': the edge of the key the surface acts on
# still         True (default): the wallpaper's tick is off while the surface is measured
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
    # The gallery is built by the theme flag, not by a key (Home polls theme.json within 2 s), and
    # BACK turns the flag off again. Each cycle sets it, waits for the build, then times the first
    # D-pad move inside the built gallery (open) and BACK (close).
    # LG's own toast and alert, for the baseline the rebuilt ones (PLAN phase 2) must beat, and LumaTV's own under its compositor (the
    # shell client's Notify.qml, `compositor-surfaces-result.md`). A luna call opens them (`!toast`, `!alert` in kb.py) and nothing closes
    # the toast but its own timer (about 5 s); the alert is closed by OK. Run on request (`--surfaces toast,alert`): the stock ones are not
    # expected to pass. A cycle of the toast is 7.8 s. With 8 s on each side of the close it was 16.3 s, longer than a full capture's ring
    # holds: LG's toast on 2026-10-25 and LumaTV's on 2026-10-27 each found 2 windows of 3, the two of LumaTV's without any `Q:` marker
    # and with the display's OSD interrupt "back" 281 and 582 ms into them (the oldest events of CPU 0's ring were gone).
    {'name': 'toast', 'default': False, 'delivery': True,
     'open': '@toast-open !toast', 'close': '@toast-close', 'settle_s': 7, 'close_settle_s': 0.5, 'transition': False},
    {'name': 'alert', 'default': False, 'delivery': True,
     'open': '@alert-open !alert', 'close': '@alert-close OK', 'settle_s': 2.5},
    {'name': 'kit-gallery',
     'setup': [TOOL + " theme set '{\"kitGallery\":true}'", 'sleep 3'],
     'teardown': [TOOL + " theme set '{\"kitGallery\":false}'"],
     'reopen': [TOOL + " theme set '{\"kitGallery\":true}'"], 'reopen_wait_s': 3,
     'open': '@kit-gallery-open DOWN', 'close': '@kit-gallery-close BACK', 'settle_s': 1.5},
]

SURFACE_DEFAULTS = {'setup': [], 'teardown': [], 'reopen': [], 'reopen_wait_s': 0, 'settle_s': 1.0, 'close_settle_s': None,
                    'transition': True, 'soak': True, 'default': True, 'thresholds': {},
                    'act': 'press', 'still': True, 'delivery': False, 'full_budget_s': None, 'inputs_free': False}

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
GAP_MS = 17.5             # analyse.py's gap listing threshold, as run.py uses
# A capture whose burst held an app's own drawing (analyse.py `burst_app_jobs`: a film's player redrawing its controls at 60 Hz) measures
# the app as well as the surface; it is kept under `discarded` and measured again, at most this many more times.
APP_RETRIES = 2
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

# A line of /var/log/messages that matches any of these fails the surface.
LOG_FAIL_PATTERNS = [
    r'TypeError', r'ReferenceError', r'is not a type', r'Binding loop', r'Unable to assign',
    r'Cannot assign', r'\[LumaKeys\].*failed', r'\[LumaHome\].*(invalid|unreadable|failed)',
    r'crash', r'segfault', r'ShaderEffectSource',
]
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
LOG_IGNORE_RE = re.compile('|'.join(LOG_IGNORE_PATTERNS))
LOG_FILE = '/var/log/messages'
LOG_TS = re.compile(r'\[\s*(\d+\.\d+)\s*\]')
LOG_REPORT_MAX = 20

KB = '/tmp/kb.py'
CAPTURE = '/tmp/perf/capture.sh'
SAMPLER = '/tmp/perf/sample.py'


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
            if len(opens) != 2 or opens[1][0] in '@!s' or opens[1][-1] in '+-:' or 'x' in opens[1]:
                raise ValueError(f'{n}: act release needs open to be the marker and one plain key token')
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
        marker, key = toks
        return [key + '+', 's0.05', marker, key + '-']
    return toks


def cycle_tokens(s):
    """kb.py tokens for one open/close cycle."""
    toks = open_tokens(s)
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


def log_failures(lines):
    """(failing lines, ignored count): lines matching LOG_FAIL_PATTERNS, less the known stock noise."""
    bad, ignored = [], 0
    for ln in lines:
        if LOG_FAIL_RE.search(ln):
            if LOG_IGNORE_RE.search(ln):
                ignored += 1
            else:
                bad.append(ln)
    return bad, ignored


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


def evaluate_windows(s, report, cycles, lg_compositor=False):
    """Checks for one surface from an analyse.py report that holds `cycles` cycles (windows of
    several captures concatenated)."""
    th = s['thresholds']
    opens = [w for w in report['windows'] if w['name'] == f"{s['name']}-open"]
    closes = [w for w in report['windows'] if w['name'] == f"{s['name']}-close"]
    keyed = opens + (closes if s['transition'] else [])
    expected = cycles * (2 if s['transition'] else 1)
    # A surface whose first frame is not checked has no key to answer (Home idle): an empty burst there is not a dead key.
    dead = [w['name'] for w in keyed if w['first_frame_ms'] is None and th['first_frame_ms'] is not None]
    unmarked = [w['name'] for w in opens if s['delivery'] and w.get('sent_ms') is None and w.get('delivered_ms') is None] if dead else []
    dead_note = ('no frame after the key in ' + ', '.join(dead)) if dead else ''
    if unmarked:
        dead_note += (f"; {len(unmarked)} of them have neither luna-send's return nor the compositor's delivery marker in the trace "
                      '(the ring lost its oldest events, or the call never returned)')
    out = [check('windows', len(keyed) == expected and not dead, f'{len(keyed)}/{expected}', f'== {expected}',
                 dead_note or ('marker lost: trace overflow or key not delivered' if len(keyed) != expected else ''))]

    if th['first_frame_ms'] is not None:
        v = _worst([w['first_frame_ms'] for w in keyed]) if not dead else None
        # a key that found the display's OSD path off waits for it, whatever the surface does (analyse.py `osd_wake_ms`)
        off = [w['osd_wake_ms'] for w in keyed if w.get('osd_wake_ms') is not None]
        # when the compositor asked the shell for the panel the key opens (its own marker, `Q:<name>-show`): the rest is the shell's
        asked = [w['show_ms'] for w in keyed if w.get('show_ms') is not None]
        notes = ([f'the OSD was off at {len(off)} key(s), on after {max(off):g} ms at worst'] if off else []) + \
                ([f'the compositor asked the shell {max(asked):g} ms after the key at worst'] if asked else [])
        out.append(check('first_frame_ms', v is not None and v <= th['first_frame_ms'], v, f"<= {th['first_frame_ms']:g}", '; '.join(notes)))
    if th['burst_end_ms'] is not None:
        ends = [w['burst_end_ms'] for w in keyed]
        v = None if any(e is None for e in ends) or not ends else max(ends)
        out.append(check('burst_end_ms', v is not None and v <= th['burst_end_ms'], v, f"<= {th['burst_end_ms']:g}",
                         'no burst at the key' if v is None else ''))
    if th['p90_render_ms'] is not None or th['p90_window_ms'] is not None:
        pooled, per_window = [], []
        for w in keyed:
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
        v = sum(w.get('dropped_in_motion', w['dropped']) for w in keyed)
        frames = sum(w.get('anim_frames') or 0 for w in keyed)
        if lg_compositor and th['dropped'] == 0:
            share = v / frames if frames else 0.0
            out.append(check('dropped_share', share <= LG_COMPOSITOR_DROP_SHARE, round(share, 3), f'<= {LG_COMPOSITOR_DROP_SHARE}',
                             f"{v} of {frames} animated frames, LG's compositor floor"))
        else:
            out.append(check('dropped', v <= th['dropped'], v, f"<= {th['dropped']}"))
    if th['burst_over20'] is not None:
        v = sum(w['burst_over20'] for w in keyed)
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
    return '\n'.join(lines)


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
               'burst_first_render_ms', 'burst_anim_samples', 'burst_app_jobs', 'irq_counts', 'gaps')


def app_drawn(report, name):
    """The windows of surface `name` whose burst held an app's own drawing."""
    return [{'name': w['name'], 'burst_end_ms': w.get('burst_end_ms'), 'burst_app_jobs': w['burst_app_jobs']} for w in report['windows']
            if w['name'] in (f'{name}-open', f'{name}-close') and w.get('burst_app_jobs')]


# ------------------------------------------------------------------- the set

class Rig:
    """The set, through run.py's Set: shell, capture and trace transfer."""

    def __init__(self):
        import run  # paramiko; only when there is a set to talk to
        self.run = run
        self.set = run.Set()

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

    def motion_enabled(self):
        """The wallpaper motion switch as the owner's theme.json has it: True, False, or None when the file does not say."""
        rc, out, _ = self.sh(f'{TOOL} theme get')
        try:
            return json.loads(out).get('wallpaperMotion', {}).get('enabled')
        except (ValueError, AttributeError):
            return None

    def set_motion(self, enabled):
        """Switch the wallpaper's 10 Hz tick; Home reads the file every 2 s, so wait 3."""
        self.sh(TOOL + ' theme set ' + shlex.quote(json.dumps({'wallpaperMotion': {'enabled': bool(enabled)}})))
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
        5 minutes of quiet opened no panel (round 16). One Back, eaten by the saver or harmless on a Home with nothing open, first."""
        self.sh(f'python3 {KB} BACK')
        time.sleep(2)

    def capture(self, name, mode):
        self.set.capture(name, mode)
        time.sleep(1)

    def stop(self, name):
        """Stop the capture; the local (trace, sys) paths."""
        self.set.stop(name)
        return f'{self.run.LOCAL}/{name}.trace', f'{self.run.LOCAL}/{name}.sys'

    def analyse(self, name):
        return self.run.analyse(name, self.set.sm_pid, GAP_MS)


class LogWatch:
    """What /var/log/messages gained since mark(). The file rotates within minutes: when the
    inode changed or the file shrank, the whole file is read and cut by the bracketed monotonic
    timestamp against /proc/uptime at the mark."""

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
        _, text, _ = self.rig.sh(f'cat {LOG_FILE}')
        return lines_since(text, mark['uptime'])


def read(path):
    with open(path, errors='replace') as f:
        return f.read()


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
           'log_unfiltered': False, 'rss': None, 'discarded': []}
    checks = res['checks']
    pid0 = rig.compositor_pid()

    def scan(mark):
        lines, filtered = log.since(mark)
        bad, ignored = log_failures(lines)
        res['log_lines'] += bad
        res['log_ignored'] += ignored
        res['log_unfiltered'] = res['log_unfiltered'] or not filtered

    try:
        mark = log.mark()
        for cmd in s['setup']:
            rc, _, err = rig.sh(cmd)
            if rc != 0:
                raise RuntimeError(f'setup {cmd!r} failed: {err.strip()}')

        reports = []
        for i, n in enumerate(plan_full(s, MEASURE_CYCLES)):
            for attempt in range(APP_RETRIES + 1):
                name = f'{base}-m{i}' + (f'-r{attempt}' if attempt else '')
                res['traces'].append(name)
                rig.capture(name, 'full')
                trace = None
                try:
                    rc, _, err = rig.sh(cycle_command(s, n), timeout=3600)
                finally:
                    trace, _sysfile = rig.stop(name)
                if rc != 0:
                    checks.append(check('keys', False, rc, '== 0', f'kb.py exited {rc}: {err.strip()}'))
                one = rig.analyse(name)
                text = read(trace)
                annotate_shows(one, text, s['name'])
                if s['delivery']:
                    apply_delivery(one, text, s['name'])
                drawn = app_drawn(one, s['name'])
                if not drawn or attempt == APP_RETRIES:
                    break
                res['discarded'].append({'trace': name, 'windows': drawn})
            reports.append(one)
        report = merge_reports(reports)
        checks += evaluate_windows(s, report, MEASURE_CYCLES, rig.lg_compositor())
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
    checks.append(log_check(res['log_lines'], res['log_ignored']))
    if res['log_unfiltered']:
        checks[-1]['detail'] = (checks[-1]['detail'] + '; ' if checks[-1]['detail'] else '') + 'rotated log had no timestamps, whole file scanned'
    pid1 = rig.compositor_pid()
    checks.append(check('compositor_pid', pid0 is not None and pid0 == pid1, pid1, f'== {pid0}',
                        'the compositor restarted during the run' if pid0 != pid1 else ''))
    if pid1 is not None:
        rig.remember_compositor(pid1)
    return res


def run_gate(rig, surfaces, soak_cycles, tag, log=None):
    rig.push_tools()
    log = log or LogWatch(rig)
    results = []
    age = rig.compositor_age_s()
    if age is not None and age < COMPOSITOR_SETTLE_S:
        time.sleep(COMPOSITOR_SETTLE_S - age)
    rig.wake()
    original = rig.motion_enabled()       # None: the file says nothing, the shipped default (on) applies
    now = original is not False
    try:
        for s in surfaces:
            want = (original is not False) if not s['still'] else False
            if want != now:
                rig.set_motion(want)
                now = want
            r = run_surface(rig, s, tag, soak_cycles, log)
            print(format_lines([r]), flush=True)
            results.append(r)
    finally:
        if now != (original is not False):
            rig.set_motion(original is not False)
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
    results = run_gate(Rig(), chosen, cycles, tag)
    v = verdict(results)
    print(f'VERDICT {v}')
    if a.json:
        with open(a.json, 'w') as f:
            json.dump(jsonable({'tag': tag, 'verdict': v, 'measure_cycles': MEASURE_CYCLES, 'soak_cycles': cycles,
                                'thresholds': THRESHOLDS, 'surfaces': results}), f, indent=1, default=str)
    return 0 if v == 'PASS' else 1


if __name__ == '__main__':
    sys.exit(main())
