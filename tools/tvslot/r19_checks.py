#!/usr/bin/env python3
"""Round 19: the pending read-only and UI checks, one unattended TV slot (LG's compositor live, nothing installed).

    python3 tools/tvslot/r19_checks.py [--only speed,settings,ble,banners] [--skip ...] [--livetv] [--dry-run]

Checks (each timed and isolated: one failing never stops the others; each leaves its evidence under OUT):
  speed     the quick panel and the input picker over Home, the round 18 method (docs/research/perf/gate.py)
  settings  Settings walk: App Permissions, Delete My Personal Information (opened, never confirmed), Home Assistant;
            Just Scan only while an HDMI input is in front at the start
  ble       docs/research/ble-result.md "Unknowns the TV slot answers" 1-3: read-only le/startScan, evidence redacted
  banners   the input banner restyle (pill, INFO card, no-signal line); never switches inputcommon, it reports what is on
  plugins   the helper's plugin broker (127.0.0.1:18731, plugin API level 1): GET routes only; who is on, first-party cards, the Home
            Assistant plugin's state, captures of Home's At-home lane and the quick panel's tiles. Never links, enables or installs.
  updateblock LG's system-update block, read only: the Homebrew Channel's blockUpdates, the flag file, /etc/hosts, the helper's topic
  layout    Home's layout editor, opened from Settings > Home & look > Home widgets > Edit on Home: capture the grid, move and size a
            card, Cancel and Discard; layout.json must have the same md5 afterwards (FAIL otherwise). Captures the Home widgets page.

The script never touches the receiver, never installs, never writes to the rootfs, never opens port 6053 and never
runs ble-link. The only writes to the TV are: kb.py in /tmp, a scratch dir /tmp/r19 (removed at the end), when the
picture mode is not filmMaker at the end that one setting, and the last step: power/powerOff into Active Standby (the owner's
rule for every TV session; --no-standby skips it). Needs: Pillow on this host (pixel checks), the connection
helper `tv.py` (default /tmp/lt/tv.py: `tv.py CMD [TIMEOUT]` and `tv.py put LOCAL REMOTE`).

Verdicts: PASS, FAIL, ERROR (the check itself broke), SKIPPED (with the reason), EYES (the automatic part passed; the
pictures still need a look). Exit status 0 when no check says FAIL or ERROR.
"""
import argparse
import base64
import datetime
import hashlib
import importlib.util
import io
import json
import os
import re
import shlex
import subprocess
import sys
import tempfile
import time
import traceback

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(os.path.dirname(HERE))
PERF = os.path.join(REPO, 'docs', 'research', 'perf')

APP = '/media/developer/apps/usr/palm/applications/org.nphil.lumatv'
TOOL = APP + '/tools/lumatv'
SAM = 'luna://com.webos.applicationManager'
BT = 'luna://com.webos.service.bluetooth2'
R = '/tmp/r19'                       # scratch directory on the set, removed at the end
KB = '/tmp/kb.py'
SPLASH_MD5 = '3dcd88495b38ef186f728b4c3d3d2a0c'   # the launch splash (HDMI glyph on grey), docs/research/lg-restyles-result.md round 11
IC_UPPER = '/var/lib/lumatv/inputcommon/upper'    # tools/inputcommon.sh: $STATE/inputcommon/upper, dataDir from brand.json
CENSUS_QML = [d for d in ('/data/home/lgtv/uiinv/inputcommon/qml', '/tmp/tvsrc/inputcommon/qml') if os.path.isdir(d)]
CHECK_ORDER = ['speed', 'settings', 'plugins', 'layout', 'updateblock', 'ble', 'banners']   # banners last: it puts an HDMI input in front
LAYOUT_FILE = '/var/lib/lumatv/theme/layout.json'    # tools/lumatv: $STATE/theme, dataDir from brand.json
MEDIA_APPS = re.compile(r'^(netflix|youtube|amazon|spotify|com\.webos\.app\.(lgchannels|music|discovery)|.*plex.*|.*disney.*|.*hulu.*)')
HDMI_APP = re.compile(r'^com\.webos\.app\.hdmi\d$')

# Where a panel or page is considered "seen": the mean absolute pixel change from the shot before must beat this many times the
# idle noise of two Home shots taken a second apart, plus a floor. [UNCALIBRATED: no panel capture of the live set was measured]
SEEN_FACTOR = 3.0
SEEN_FLOOR = 1.5


# --------------------------------------------------------------------------------------------------- the connection

class TvError(RuntimeError):
    pass


class Tv:
    """The set through tv.py: `tv.py CMD [TIMEOUT]` runs CMD in the set's shell and prints its output; `put` copies a file."""

    def __init__(self, tv_py):
        self.tv_py = tv_py

    def sh(self, cmd, timeout=60):
        last = None
        for attempt in range(3):
            try:
                p = subprocess.run([sys.executable, self.tv_py, cmd, str(int(timeout))], capture_output=True, text=True, timeout=timeout + 30)
            except subprocess.TimeoutExpired as e:
                last = e
                continue
            if p.returncode == 0:
                return p.stdout
            last = TvError((p.stderr or p.stdout)[-300:])
            time.sleep(2)
        raise TvError(f'tv.py failed 3 times: {last}')

    def put(self, local, remote):
        p = subprocess.run([sys.executable, self.tv_py, 'put', local, remote], capture_output=True, text=True, timeout=60)
        if p.returncode != 0 or 'ok' not in p.stdout:
            raise TvError(f'put {remote}: {(p.stderr or p.stdout)[-200:]}')


class DryTv:
    """Stands in for the set: records every command, syntax-checks it with `sh -n` (and its luna-send JSON), answers from canned
    replies shaped like the real ones so the parsing runs too."""

    def __init__(self):
        self.commands = []
        self.problems = []
        self.puts = []
        self.standby = False

    def sh(self, cmd, timeout=60):
        self.commands.append(cmd)
        print(f'[dry] tv> {cmd}')
        self.check(cmd)
        if 'power/powerOff' in cmd:
            self.standby = True
        if self.standby and 'getPowerState' in cmd:
            return '{"returnValue":true,"state":"Active Standby"}'
        return canned(cmd)

    def put(self, local, remote):
        self.puts.append((local, remote))
        print(f'[dry] put {local} -> {remote}')
        if not os.path.exists(local):
            self.problems.append(f'put: {local} is missing')

    def check(self, cmd):
        r = subprocess.run(['sh', '-n', '-c', cmd], capture_output=True, text=True)
        if r.returncode != 0:
            self.problems.append(f'sh -n: {r.stderr.strip()}: {cmd[:120]}')
        for m in re.finditer(r"luna-send [^']*?(?:-f )?(luna://\S+) '(\{.*?\})'(?=\s|$|;|\)|>|\|)", cmd):
            try:
                json.loads(m.group(2))
            except ValueError as e:
                self.problems.append(f'luna payload is not JSON ({e}): {m.group(2)[:100]}')
        for m in re.finditer(re.escape(KB) + r' ((?:[^;|&<>()]+))', cmd):
            bad = kb_problems(shlex.split(m.group(1)))
            if bad:
                self.problems.append(f'kb tokens {bad}: {m.group(1)[:100]}')
        for m in re.finditer(r"python3 -c '(.*?)' ", cmd + ' '):
            try:
                compile(m.group(1), 'remote', 'exec')
            except SyntaxError as e:
                self.problems.append(f'python3 -c does not compile: {e}')


KB_TOKEN = re.compile(r'^(@\S+|!\S+|s\d+(\.\d+)?|[A-Z]+[+-]|[A-Z]+:\d+|[A-Z]+x\d+(@\d+(\.\d+)?)?|[A-Z]+|\d+)$')


def kb_problems(tokens):
    return [t for t in tokens if not KB_TOKEN.match(t)]


def kb_seconds(tokens):
    """How long kb.py runs these tokens (its own run())."""
    total = 0.0
    for t in tokens:
        if t[0] in '@!' or t.endswith(('+', '-')):
            continue
        if t[0] == 's' and t[1:].replace('.', '').isdigit():
            total += float(t[1:])
        elif 'x' in t:
            count, _, gap = t.split('x', 1)[1].partition('@')
            total += int(count) * (0.05 + float(gap or 0.35))
        elif ':' in t:
            total += int(t.split(':')[1]) / 1000.0
        else:
            total += 0.05
    return total


# ----------------------------------------------------------------------------------------------------- image tools

def pil():
    try:
        from PIL import Image, ImageChops, ImageDraw, ImageStat
        return Image, ImageChops, ImageDraw, ImageStat
    except ImportError:
        return None


def load(path_or_bytes):
    P = pil()
    if P is None:
        return None
    src = io.BytesIO(path_or_bytes) if isinstance(path_or_bytes, (bytes, bytearray)) else path_or_bytes
    return P[0].open(src).convert('RGB')


def luma(px):
    return (px[0] + px[1] + px[2]) / 3.0


def mean_diff(a, b):
    """Mean absolute pixel change between two same-size images (0 when either is missing)."""
    P = pil()
    if P is None or a is None or b is None or a.size != b.size:
        return 0.0
    d = P[1].difference(a, b).convert('L')
    return P[3].Stat(d).mean[0]


def box_stats(img, x_max=420, y0=180, y1=440, floor=6):
    """Bounding box of the non-black pixels in the lower-left region where the banner sits, and the colour just inside its left edge."""
    P = pil()
    if P is None or img is None:
        return None
    region = img.crop((0, y0, x_max, y1)).convert('L').point(lambda v: 255 if v > floor else 0)
    bb = region.getbbox()
    if bb is None:
        return None
    left, top, right, bottom = bb[0], bb[1] + y0, bb[2] - 1, bb[3] + y0 - 1
    mid = (top + bottom) // 2
    inside = img.getpixel((min(left + 6, img.size[0] - 1), mid))
    return {'x0': left, 'y0': top, 'x1': right, 'y1': bottom, 'w': right - left + 1, 'h': bottom - top + 1, 'inside': list(inside), 'inside_luma': round(luma(inside), 1)}


def classify_pill(img):
    """glass (Lucent's dark pill: 44 px high at x 32), grey (LG's own pill or bar), bare (text on black at the edge, no box), none, other."""
    s = box_stats(img)
    if s is None:
        return 'none', None
    if 36 <= s['h'] <= 52 and 24 <= s['x0'] <= 40 and s['w'] >= 100 and s['inside_luma'] < 50:
        return 'glass', s
    if s['inside_luma'] >= 60 and (s['h'] >= 60 or s['x0'] < 8):
        return 'grey', s
    if s['x0'] < 8 and s['h'] < 40:
        return 'bare', s
    return 'other', s


INFO_POINTS = [(40, 290), (40, 366), (300, 368), (570, 366), (570, 290)]   # inside the card's padding (card 33-581 x 282-372 in the 960 capture)


def classify_info(img):
    """The INFO card's colour: glass (about #120E1A at 93 %), grey (LG's #575E66 at 90 %) or unclear. Corners, hairline and chips need eyes."""
    if img is None:
        return 'unclear', []
    lumas = sorted(luma(img.getpixel(p)) for p in INFO_POINTS)
    median = lumas[len(lumas) // 2]
    if median < 50 and lumas[-2] < 70:
        return 'glass', [round(v, 1) for v in lumas]
    if 62 <= median <= 110 and max(lumas) - min(lumas) < 25:
        return 'grey', [round(v, 1) for v in lumas]
    return 'unclear', [round(v, 1) for v in lumas]


def save_png(img, path):
    img.save(path, optimize=True)


def montage(items, path, cols=4, cell=(320, 180)):
    """items: [(caption, Image)]; one contact sheet."""
    P = pil()
    if P is None or not items:
        return None
    Image, _, ImageDraw, _ = P
    rows = -(-len(items) // cols)
    sheet = Image.new('RGB', (cols * cell[0], rows * (cell[1] + 14)), (30, 30, 30))
    d = ImageDraw.Draw(sheet)
    for i, (cap, im) in enumerate(items):
        x, y = (i % cols) * cell[0], (i // cols) * (cell[1] + 14)
        sheet.paste(im.resize(cell), (x, y + 14))
        d.text((x + 3, y + 1), cap, fill=(230, 230, 230))
    save_png(sheet, path)
    return path


# ------------------------------------------------------------------------------------------------------ BLE parsing

def split_json(text):
    """Every top-level JSON object in a luna-send stream (pretty-printed over lines or one a line)."""
    out, depth, start, in_str, esc = [], 0, None, False, False
    for i, ch in enumerate(text):
        if in_str:
            if esc:
                esc = False
            elif ch == '\\':
                esc = True
            elif ch == '"':
                in_str = False
            continue
        if ch == '"' and depth:
            in_str = True
        elif ch == '{':
            if depth == 0:
                start = i
            depth += 1
        elif ch == '}' and depth:
            depth -= 1
            if depth == 0:
                try:
                    out.append(json.loads(text[start:i + 1]))
                except ValueError:
                    pass
    return out


def record_bytes(v):
    if isinstance(v, list) and all(isinstance(x, int) and 0 <= x < 256 for x in v):
        return bytes(v)
    if isinstance(v, str):
        try:
            return bytes.fromhex(v)
        except ValueError:
            pass
        try:
            return base64.b64decode(v, validate=True)
        except ValueError:
            return None
    return None


def ad_structures(b):
    """[(type, data length)] of the length-prefixed AD structures of an advertisement; the last entry is ('truncated', 0) when it overruns."""
    out, i = [], 0
    while i < len(b):
        n = b[i]
        if n == 0:
            break
        if i + 1 + n > len(b):
            out.append(('truncated', 0))
            break
        out.append((b[i + 1], n - 1))
        i += n + 1
    return out


def scan_stats(objs):
    """What one scan's replies say about the three unknowns. No address or name leaves this function: devices are numbered."""
    first = objs[0] if objs else {}
    ids, devs, last_rssi = {}, [], {}
    repeats = same = 0
    lengths, over31, types, with_record, empty_record, named = [], 0, set(), 0, 0, 0
    singles = lists = 0
    for o in objs:
        batch = []
        if isinstance(o.get('device'), dict):
            singles += 1
            batch.append(('device', o['device']))
        if isinstance(o.get('devices'), list):
            lists += 1
            batch += [('devices', d) for d in o['devices'] if isinstance(d, dict)]
        for kind, d in batch:
            addr = str(d.get('address', ''))
            ids.setdefault(addr, len(ids) + 1)
            if kind == 'device':
                rssi = d.get('rssi')
                if addr in last_rssi:
                    repeats += 1
                    same += 1 if rssi == last_rssi[addr] else 0
                last_rssi[addr] = rssi
            if 'scanRecord' in d:
                b = record_bytes(d['scanRecord'])
                if b:
                    with_record += 1
                    lengths.append(len(b))
                    over31 += 1 if len(b) > 31 else 0
                    st = ad_structures(b)
                    types.update(t for t, _ in st)
                    named += 1 if any(t in (8, 9) for t, _ in st) else 0
                else:
                    empty_record += 1
    return {
        'replies': len(objs),
        'first_reply': {'keys': sorted(first.keys()), 'returnValue': first.get('returnValue'), 'subscribed': first.get('subscribed'),
                        'errorCode': first.get('errorCode'), 'errorText': first.get('errorText')},
        'device_replies': singles, 'list_replies': lists, 'addresses': len(ids),
        'scan_record': {'with_record': with_record, 'empty_or_unreadable': empty_record,
                        'length_min': min(lengths) if lengths else None, 'length_max': max(lengths) if lengths else None,
                        'records_over_31_bytes': over31, 'records_with_name': named,
                        'ad_types_seen': sorted(hex(t) if isinstance(t, int) else t for t in types)},
        'same_address_again': repeats, 'same_address_again_same_rssi': same,
    }


def dialog_like(img):
    """Whether the centre of the screen looks like the confirm dialog (an opaque flat sheet, 760 of 1920 wide, over a hidden page): one
    colour fills most of the central crop. [UNCALIBRATED: no capture of the dialog on the set]"""
    if img is None:
        return False
    crop = img.crop((300, 190, 660, 350))
    colours = crop.getcolors(maxcolors=crop.size[0] * crop.size[1])
    return bool(colours) and max(c for c, _ in colours) / float(crop.size[0] * crop.size[1]) > 0.5


def ha_words(owner_plugins, link):
    """What the Home Assistant plugin's chip says, as kit/PluginHub.qml works it out (haState, haWords): owner_plugins is `plugins` of
    GET /v1/plugins, link the value of the `ha.link` topic."""
    p = next((x for x in owner_plugins if x.get('id') == 'home-assistant'), None)
    if p is None:
        return 'Not switched on'
    if p.get('state') == 'inert':
        return 'Cannot run'
    if p.get('state') != 'connected':
        return 'Not running' if p.get('reason') else 'Starting'
    s = (link or {}).get('state')
    if s == 'auth_failed':
        return 'Token refused'
    if not s or s in ('off', 'unconfigured'):
        return 'Not linked'
    return {'offline': 'Offline', 'connecting': 'Connecting'}.get(s, 'Connected')


def banner_tree():
    """What the staged inputcommon overlay must hold, derived from tools/inputcommon_patch.py (never typed in here): `files`, the paths
    under LG's qml directory that the patch always writes (every entry of its FILES, the style sheet, LumaStyle.qml, its own screens);
    `count`, the number of .qml files the stage keeps, from running the real patch on the census copy of LG's tree when this host has
    one (the stage keeps every file the patch rewrote, among them the ones it finds reading a style value), else None, and then
    `at_least` is len(files)."""
    spec = importlib.util.spec_from_file_location('r19_inputcommon_patch', os.path.join(REPO, 'tools', 'inputcommon_patch.py'))
    m = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(m)
    files = [m.STYLE_FILE] + [rel for rel, _ in m.FILES] + [m.LUMA_STYLE_FILE] + [m.BG + n for n in m.NEW_FILES]
    count = None
    if CENSUS_QML:
        import shutil
        work = tempfile.mkdtemp(prefix='r19-ic-')
        try:
            shutil.copytree(CENSUS_QML[0], os.path.join(work, 'q'))
            pal = subprocess.run([sys.executable, os.path.join(REPO, 'tools', 'tokens.py'), os.path.join(REPO, 'payload', 'theme', 'theme.json'),
                                  os.path.join(work, 'none.json'), 'qml'], capture_output=True, text=True)
            if pal.returncode == 0:
                done = m.patch_tree(os.path.join(work, 'q'), json.loads(pal.stdout), os.path.join(REPO, 'payload', 'fonts'))
                count = sum(1 for f in done if f.endswith('.qml'))
        except (ValueError, OSError):
            count = None
        finally:
            shutil.rmtree(work, ignore_errors=True)
    return {'files': files, 'count': count, 'at_least': len(files)}


def ping_summary(text):
    m = re.search(r'min/avg/max\S*\s*=\s*([\d.]+)/([\d.]+)/([\d.]+)', text)
    loss = re.search(r'(\d+)% packet loss', text)
    return {'min_ms': float(m.group(1)), 'avg_ms': float(m.group(2)), 'max_ms': float(m.group(3)),
            'loss_pct': int(loss.group(1)) if loss else None} if m else None


# ------------------------------------------------------------------------------------------------ canned replies (dry run)

_PNG_CACHE = {}


def synth_png(kind):
    """A 960x540 capture of the named look, for the dry run's pixel checks."""
    P = pil()
    if P is None:
        return b''
    if kind in _PNG_CACHE:
        return _PNG_CACHE[kind]
    Image, _, ImageDraw, _ = P
    bg = (20, 49, 80) if kind in ('info', 'ui') else (0, 0, 0)
    im = Image.new('RGB', (960, 540), bg)
    d = ImageDraw.Draw(im)
    if kind == 'pill-glass':
        d.rounded_rectangle((32, 281, 191, 324), 22, fill=(17, 13, 24))
    elif kind == 'info':
        d.rounded_rectangle((33, 282, 581, 372), 12, fill=(20, 16, 28))
    b = io.BytesIO()
    im.save(b, 'PNG')
    _PNG_CACHE[kind] = b.getvalue()
    return _PNG_CACHE[kind]


def canned(cmd):
    if 'updateblock status' in cmd:
        return '{"ok":true,"updateBlock":{"flag":true,"hosts":{"snu.lge.com":true,"su.lge.com":true,"su-ssl.lge.com":true,"su-dev.lge.com":true},"live":true,"blocking":true,"state":"blocked"}}'
    if cmd.endswith(TOOL + ' status 2>&1'):
        return '{"ok":true,"version":"0.4.0","updateBlock":{"flag":true,"state":"blocked"}}'
    if 'org.webosbrew.hbchannel.service/getConfiguration' in cmd:
        return '{"returnValue":true,"root":true,"blockUpdates":true}'
    if 'webosbrew_block_updates' in cmd:
        return 'yes\n'
    if "grep -n 'lge.com' /etc/hosts" in cmd:
        return '3:127.0.0.1 snu.lge.com su-dev.lge.com su.lge.com su-ssl.lge.com\n4:::1 snu.lge.com su-dev.lge.com su.lge.com su-ssl.lge.com\n\n1\n'
    if 'getent hosts su.lge.com' in cmd:
        return '127.0.0.1       su.lge.com\n'
    if 'curl' in cmd and '/v1/state/updateblock' in cmd:
        return '{"value":{"flag":true,"live":true,"blocking":true,"state":"blocked"},"seq":2}'
    if 'base64.b64encode' in cmd:
        out = []
        for path in shlex.split(cmd.split("' ", 1)[1]) if "' " in cmd else []:
            kind = 'pill-glass' if re.search(r'pill-(0[3-9]|1)', path) else 'info' if 'info-1' in path else 'ui'
            out.append('@@' + path + '\n' + base64.b64encode(synth_png(kind)).decode())
        return '\n'.join(out) + '\n'
    if 'getForegroundAppInfo' in cmd:
        return '{"returnValue":true,"appId":"com.webos.app.home"}'
    if 'getPowerState' in cmd:
        return '{"returnValue":true,"state":"Active"}'
    if 'getSystemSettings' in cmd and 'pictureMode' in cmd:
        return '{"returnValue":true,"settings":{"pictureMode":"filmMaker"}}'
    if 'getSystemSettings' in cmd:
        return '{"returnValue":true,"settings":{"lmt":"off","doNotSellMyPersonalInformation":"off"}}'
    if 'systemctl --failed' in cmd:
        return 'guard failed=6 librdx=0 aud_reset=0\n'
    if 'inputs status' in cmd:
        return '{"ok":true,"inputs":{"ports":[],"running":[],"held":""}}'
    if 'inputcommon status' in cmd:
        return '{"ok":true,"qml":"/usr/palm/applications/com.webos.app.inputcommon/qml","staged":true,"mounted":true,"checkpoint":"aside","criuEnabled":true}'
    if 'theme get' in cmd:
        return 'd41d8cd98f00b204e9800998ecf8427e  -\n'
    if 'executeOneShot' in cmd and 'burst.sh' not in cmd:
        return '{"returnValue":true,"capturedWidth":960}'
    if '/running' in cmd:
        return '{"returnValue":true,"running":[{"id":"com.webos.app.hdmi2","processid":"7890"},{"id":"com.webos.app.home","processid":"6342"}]}'
    if '/proc/$p/cmdline' in cmd or 'cmdline' in cmd:
        return '/usr/bin/com.webos.app.inputcommon {"event":"launch","appId":"com.webos.app.hdmi2"}\n'
    if 'upper' in cmd and '*.qml' in cmd:
        return '%d\n' % (banner_tree()['count'] or banner_tree()['at_least'])
    if 'missing-files' in cmd:
        return 'none\n'
    if '/proc/mounts' in cmd:
        return '7\n'
    if 'ls -A /tmp/checkpoint' in cmd:
        return '.lumatv-aside.com.webos.app.inputcommon\nerror\n'
    if cmd.startswith('head -1') and '/times' in cmd:
        return 'launch-call-ms 460\n'
    if cmd.startswith('cat') and cmd.endswith('/times'):
        return 'launch-call-ms 460\n' + ''.join('%02d %d\n' % (i, 320 + 440 * (i - 1)) for i in range(1, 25))
    if 'md5.txt' in cmd and 'cat' in cmd:
        return ''.join('%s  ./pill-%02d.png\n' % (hashlib.md5(b'%d' % i).hexdigest(), i) for i in range(1, 25))
    if 'burst.sh' in cmd:
        return ''
    if '[d]one' in cmd or '/done' in cmd:
        return 'done\n'
    if 'le/startScan' in cmd and 'timeout 15' in cmd:
        return 'scanned\n'
    if cmd.startswith('head -c') and '/scan' in cmd:
        return dry_scan_text(cmd)
    if cmd.startswith('cat') and 'ping' in cmd:
        return '--- gw ping statistics ---\n10 packets transmitted, 10 received, 0% packet loss\nround-trip min/avg/max = 0.9/1.4/3.2 ms\n'
    if "ps | grep '[s]tartScan'" in cmd:
        return '0\n'
    if 'StartScan listener dropped' in cmd:
        return '1\n'
    if 'adapter/getStatus' in cmd:
        return '{"returnValue":true,"adapters":[{"powered":true,"discovering":false}]}'
    if 'device/getStatus' in cmd:
        return '{"returnValue":true,"devices":[{"name":"remote","connectedProfiles":["hid"]}]}'
    if 'le/getStatus' in cmd:
        return '{"returnValue":true,"advertising":true}'
    if 'top -b' in cmd:
        return 'CPU: 3% usr 1% sys 0% nic 95% idle\n'
    if 'curl' in cmd and '/v1/state/ha.link' in cmd:
        return '{"value":{"state":"off"},"seq":3}'
    if 'curl' in cmd and '/v1/state/plugin.ambient.tiles' in cmd:
        return '{"value":{"items":[{"id":"ambient-on"}]},"seq":1}'
    if 'curl' in cmd and '/v1/state/plugin.' in cmd and '.cards' in cmd:
        return '{"value":{"items":[{"id":"c1","revision":1,"card":{"size":"half"}}]},"seq":2}'
    if 'curl' in cmd and ('/v1/state/weather' in cmd or '/v1/state/news' in cmd):
        return '{"value":{"temp":61},"seq":1}'
    if 'curl' in cmd and '/v1/plugins' in cmd:
        return json.dumps({'plugins': [], 'firstParty': [{'id': i, 'name': i, 'version': '1', 'class': 'first-party', 'state': 'connected', 'registered': True, 'firstParty': True}
                                       for i in ('weather', 'news', 'clock', 'ambient')], 'problem': None})
    if 'curl' in cmd and cmd.rstrip().endswith('/v1/state'):
        return json.dumps({'boot': 'b', 'seq': 9, 'helper': {'version': '0.1', 'pid': 99, 'rssKb': 8000, 'threads': 6}, 'topics': {'weather': {}, 'ha.link': {}}})
    if 'layout.json' in cmd and 'md5sum' in cmd:
        return 'absent\n'
    if 'layout.json' in cmd and 'stat -c' in cmd:
        return '\n'
    if 'layout.json' in cmd and cmd.startswith('cat'):
        return 'ABSENT\n'
    if 'enabled.json' in cmd:
        return 'MISSING\n'
    if 'broker.sock' in cmd:
        return 'ls: /run/lumashow-plugins/broker.sock: No such file or directory\n'
    if '/v1/receiver' in cmd:
        return '{"value":{"mute":true,"volumeText":"53","input":"TV"}}'
    if 'lumatv-compositor' in cmd:
        return '0\n'
    if 'pidof surface-manager' in cmd:
        return '2549\n'
    if 'wc -l < /var/log/messages' in cmd:
        return '1000\n'
    return ''


def dry_scan_text(cmd):
    always = 'scan2' in cmd
    recs = ', '.join(str(v) for v in [2, 1, 6, 9, 5, 66, 111, 111, 100, 7, 255, 76, 0, 1, 2, 3, 4, 5])
    objs = ['{"subscribed":true,"returnValue":true,"adapterAddress":"00:11:22:33:44:55"}']
    for i in range(6 if always else 3):
        objs.append('{"returnValue":true,"device":{"address":"AA:BB:CC:00:00:0%d","rssi":-60,"scanRecord":[%s]}}' % (i % 2, recs))
    return '\n'.join(objs) + '\n'


# ---------------------------------------------------------------------------------------------------------- the runner

class Slot:
    def __init__(self, args, tv):
        self.args = args
        self.tv = tv
        self.dry = args.dry_run
        self.out = args.out
        os.makedirs(self.out, exist_ok=True)
        self.log_file = open(os.path.join(self.out, 'log.txt'), 'a')
        self.planned_s = 0.0
        self.results = []
        self.start = {}
        self.end = {}
        self.started_apps = set()
        self.scan_ran = False

    # ---- plumbing
    def log(self, msg):
        line = '%s %s' % (time.strftime('%H:%M:%S'), msg)
        print(line, flush=True)
        self.log_file.write(line + '\n')
        self.log_file.flush()

    def sleep(self, s):
        self.planned_s += s
        if not self.dry:
            time.sleep(s)

    def sh(self, cmd, timeout=60):
        return self.tv.sh(cmd, timeout)

    def luna(self, uri, payload='{}', timeout=30):
        return self.sh('luna-send -n 1 -f %s %s' % (uri, shlex.quote(payload)), timeout)

    @staticmethod
    def jl(text):
        m = re.search(r'\{.*\}', text, re.S)
        try:
            return json.loads(m.group(0)) if m else {}
        except ValueError:
            return {}

    def fg(self):
        return self.jl(self.luna(SAM + '/getForegroundAppInfo')).get('appId', '?')

    def power(self):
        return self.jl(self.luna('luna://com.webos.service.tvpower/power/getPowerState')).get('state', '?')

    def picture_mode(self):
        out = self.luna('luna://com.webos.settingsservice/getSystemSettings', '{"category":"picture","keys":["pictureMode"]}')
        m = re.search(r'"pictureMode"\s*:\s*"([^"]+)"', out)
        return m.group(1) if m else '?'

    def general_flags(self):
        out = self.luna('luna://com.webos.settingsservice/getSystemSettings', '{"category":"general","keys":["lmt","doNotSellMyPersonalInformation"]}')
        return self.jl(out).get('settings', {})

    def theme_md5(self):
        return self.sh("%s theme get | md5sum | cut -d' ' -f1" % TOOL, 30).strip()

    def guard(self):
        out = self.sh('echo "guard failed=$(systemctl --failed --no-legend 2>/dev/null | wc -l) librdx=$(ls /tmp/var/log/reports/librdx 2>/dev/null | wc -l) '
                      'aud_reset=$(cat /sys/module/aud/parameters/aud_dsp_reset_count 2>/dev/null)"', 30)
        m = re.search(r'failed=(\d+) librdx=(\d+) aud_reset=(\S*)', out)
        return {'failed': int(m.group(1)), 'librdx': int(m.group(2)), 'aud_reset': m.group(3)} if m else None

    def inputs(self):
        return self.jl(self.sh(TOOL + ' inputs status', 30)).get('inputs', {})

    def inputcommon(self):
        return self.jl(self.sh(TOOL + ' inputcommon status', 30))

    def running(self):
        return self.jl(self.luna(SAM + '/running')).get('running', [])

    def kb(self, tokens):
        toks = tokens.split()
        bad = kb_problems(toks)
        if bad:
            raise ValueError(f'kb tokens not understood: {bad}')
        self.planned_s += kb_seconds(toks)
        self.sh('python3 %s %s' % (KB, tokens), 40 + kb_seconds(toks))

    def launch(self, app_id):
        self.started_apps.add(app_id)
        return self.luna(SAM + '/launch', json.dumps({'id': app_id}))

    def close_app(self, app_id):
        return self.luna(SAM + '/closeByAppId', json.dumps({'id': app_id}))

    def shot_remote(self, path):
        cmd = ('rm -f %s; luna-send -n 1 luna://com.webos.service.capture/executeOneShot '
               '\'{"path":"%s","method":"DISPLAY","format":"PNG","width":960,"height":540}\'' % (path, path))
        for attempt in range(3):
            if '"returnValue":true' in self.sh(cmd, 30).replace(' ', ''):
                return True
            self.sleep(1.5)
        return False

    def fetch(self, paths, timeout=120):
        """{remote path: bytes}: the files base64-coded in one session (python3 is on the set, the gate's kb.py needs it)."""
        if not paths:
            return {}
        py = 'import sys,base64;[print("@@"+p+"\\n"+base64.b64encode(open(p,"rb").read()).decode()) for p in sys.argv[1:]]'
        out = self.sh("python3 -c '%s' %s" % (py, ' '.join(shlex.quote(p) for p in paths)), timeout)
        files, name, buf = {}, None, []
        for line in out.splitlines():
            if line.startswith('@@'):
                if name:
                    files[name] = base64.b64decode(''.join(buf))
                name, buf = line[2:], []
            elif name:
                buf.append(line.strip())
        if name:
            files[name] = base64.b64decode(''.join(buf))
        return files

    def shot(self, name, subdir, keep=True):
        """A 960x540 capture of the screen; kept as evidence under OUT/subdir unless keep is false. Returns the image (None when it failed)."""
        remote = '%s/%s.png' % (R, name)
        if not self.shot_remote(remote):
            self.log('shot %s: capture failed' % name)
            return None
        data = self.fetch([remote]).get(remote)
        img = load(data) if data else None
        if keep and img is not None:
            d = os.path.join(self.out, subdir)
            os.makedirs(d, exist_ok=True)
            save_png(img, os.path.join(d, name + '.png'))
        self.log('shot %s' % name)
        return img

    def home(self, wait=6):
        self.launch('com.webos.app.home')
        self.sleep(wait)
        return self.fg()

    # ---- states
    def state(self):
        rcv = self.jl(self.sh('curl -s -m 5 http://127.0.0.1:18731/v1/receiver', 15)).get('value', {})
        return {'time': time.strftime('%Y-%m-%dT%H:%M:%S'), 'power': self.power(), 'foreground': self.fg(), 'picture_mode': self.picture_mode(),
                'guard': self.guard(), 'inputcommon': self.inputcommon(), 'inputs': self.inputs(), 'theme_md5': self.theme_md5(),
                'general_flags': self.general_flags(), 'receiver_as_read_by_the_helper': rcv,
                'lumatv_compositor_running': int(self.sh("pgrep -f 'bin/[l]umatv-compositor' | wc -l", 15).strip() or 0),
                'running_apps': [a.get('id') for a in self.running()]}

    def save_state(self, name, st):
        with open(os.path.join(self.out, name), 'w') as f:
            json.dump(st, f, indent=1)

    # ---- one check
    def run_check(self, name, fn):
        t0 = time.time()
        res = {'check': name, 'verdict': 'ERROR', 'items': [], 'detail': ''}
        self.log('==== check %s: start' % name)
        try:
            res.update(fn())
        except Exception as e:                  # a broken check is a result, not the end of the slot
            res['verdict'] = 'ERROR'
            res['detail'] = '%s: %s' % (type(e).__name__, e)
            self.log(traceback.format_exc())
        res['seconds'] = round(time.time() - t0, 1)
        res['planned_wait_s'] = round(self.planned_s, 1)
        try:
            g = self.guard()
            res['guard_after'] = g
            if g and self.start.get('guard') and g != self.start['guard']:
                res['guard_moved'] = {'before': self.start['guard'], 'after': g}
        except Exception as e:
            res['guard_after_error'] = str(e)
        self.log('==== check %s: %s in %.0f s: %s' % (name, res['verdict'], res['seconds'], res['detail']))
        self.results.append(res)
        return res

    def item(self, res, name, verdict, detail='', **extra):
        res.append(dict({'name': name, 'verdict': verdict, 'detail': detail}, **extra))
        self.log('  %-7s %s %s' % (verdict, name, detail))

    # ================================================================================================= check: speed

    def check_speed(self):
        gate = os.path.join(PERF, 'gate.py')
        d = os.path.join(self.out, 'speed')
        os.makedirs(d, exist_ok=True)
        js = os.path.join(d, 'r19.json')
        cmd = [sys.executable, gate, '--surfaces', 'quick,picker', '--input-key', self.args.input_key, '--tag', 'r19', '--json', js]
        est = gate_estimate_s()
        self.log('speed: %s (the gate alone: about %.0f s of key cycles; it also waits until the compositor is 90 s old)' % (' '.join(cmd), est))
        if self.fg() != 'com.webos.app.home':
            self.home()
        if self.dry:
            self.planned_s += est
            return {'verdict': 'SKIPPED', 'detail': 'dry run: gate command printed, flags validated by gate.py --help', 'items': dry_gate_flags(gate, cmd)}
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=1800)
        with open(os.path.join(d, 'gate-stdout.txt'), 'w') as f:
            f.write(p.stdout + ('\n[stderr]\n' + p.stderr if p.stderr.strip() else ''))
        items = []
        for line in p.stdout.splitlines():
            m = re.match(r'^(PASS|FAIL)\s+(\S+)\s+(\S+)\s+(.*)$', line)
            if m:
                items.append({'name': '%s %s' % (m.group(2), m.group(3)), 'verdict': m.group(1), 'detail': m.group(4).strip()})
        v = re.search(r'^VERDICT (PASS|FAIL)', p.stdout, re.M)
        if not v:
            return {'verdict': 'ERROR', 'detail': 'the gate printed no VERDICT (exit %d): %s' % (p.returncode, (p.stderr or p.stdout)[-200:]), 'items': items}
        fails = [i['name'] for i in items if i['verdict'] == 'FAIL']
        return {'verdict': v.group(1), 'detail': 'FAIL: ' + ', '.join(fails) if fails else 'every threshold met', 'items': items, 'files': ['speed/r19.json', 'speed/gate-stdout.txt']}

    # ============================================================================================ check: settings walk

    def panel_seen(self, before, after, noise):
        d = mean_diff(before, after)
        return d > SEEN_FACTOR * noise + SEEN_FLOOR, d

    def open_quick(self, tag, sub):
        """The quick panel (the gear's tap), only taken as open when the screen changed: on a Home that did not open it, Down and OK would
        move and press Home's own focus. Returns (panel image, idle noise, why-not)."""
        a = self.shot(tag + '-home-a', sub, keep=False)
        self.sleep(1.2)
        b = self.shot(tag + '-home-b', sub)
        noise = mean_diff(a, b)
        seen = False
        panel = None
        for attempt in (1, 2):
            self.kb('MENU s1.6')
            panel = self.shot('%s-quick-%d' % (tag, attempt), sub)
            seen, d = self.panel_seen(b, panel, noise)
            self.log('quick panel seen=%s (change %.2f, idle noise %.2f)' % (seen, d, noise))
            if seen or self.dry:
                seen = True
                break
            self.kb('BACK s1.5')
        if not seen:
            return None, noise, 'the quick panel was not seen after two taps of the gear'
        return panel, noise, ''

    def open_settings(self, tag, sub):
        """The quick panel and its "All settings" button; OK is pressed only once the panel is seen."""
        panel, noise, why = self.open_quick(tag, sub)
        if panel is None:
            return None, why
        self.kb('DOWNx40@0.08 s0.6')
        self.shot(tag + '-quick-foot', sub)
        self.kb('OK s2.5')
        st = self.shot(tag + '-settings', sub)
        seen, d = self.panel_seen(panel, st, noise)
        self.log('settings seen=%s (change %.2f)' % (seen, d))
        if not seen and not self.dry:
            return None, 'Settings was not seen after OK on "All settings"'
        return st, ''

    # ============================================================================================ check: plugins
    # Read-only: GET routes of the helper's loopback API and one file read. No POST (rescan, actions, the lease refresh), no
    # `lumatv-helper` command (plugin-enable, ha-link and the rest are the owner's), nothing linked, enabled, installed or sent.

    def api(self, path):
        """GET a route of the helper on the set; (parsed JSON or None, raw text)."""
        text = self.sh('curl -s -m 8 http://127.0.0.1:18731%s' % path, 20)
        try:
            return json.loads(text), text
        except ValueError:
            return None, text

    def check_plugins(self):
        sub = 'plugins'
        items = []
        d = os.path.join(self.out, sub)
        os.makedirs(d, exist_ok=True)
        evidence = {}
        if self.fg() != 'com.webos.app.home':
            self.home()
        st, st_text = self.api('/v1/state')
        helper = (st or {}).get('helper')
        if helper:
            self.item(items, 'helper answers on 127.0.0.1:18731', 'PASS', 'version %s, pid %s, rss %s kB, %d topics' % (
                helper.get('version'), helper.get('pid'), helper.get('rssKb'), len((st.get('topics') or {}))))
            evidence['helper'] = {k: helper.get(k) for k in ('version', 'pid', 'rssKb', 'threads')}
            evidence['topic_names'] = sorted((st.get('topics') or {}).keys())
        else:
            self.item(items, 'helper answers on 127.0.0.1:18731', 'FAIL', 'no JSON with a helper object: %r' % st_text[:120])
        leak = re.search(r'"(token|access_token|password|secret)"\s*:\s*"[^"]+"', st_text)
        self.item(items, 'no token or secret in /v1/state', 'FAIL' if leak else 'PASS',
                  ('a string value under "%s"' % leak.group(1)) if leak else 'no key named token, access_token, password or secret holds a string')
        plug, plug_text = self.api('/v1/plugins')
        if not isinstance(plug, dict) or 'plugins' not in plug:
            self.item(items, 'GET /v1/plugins', 'FAIL', 'no plugins list: %r' % plug_text[:120])
            plug = {'plugins': [], 'firstParty': [], 'problem': None}
        owners, first = plug.get('plugins') or [], plug.get('firstParty') or []
        view = lambda l: [{k: p.get(k) for k in ('id', 'name', 'version', 'class', 'state', 'registered', 'reason')} for p in l]
        evidence['plugins'] = view(owners)
        evidence['first_party'] = view(first)
        evidence['problem'] = plug.get('problem')
        self.item(items, 'owner\'s plugins (switched on)', 'INFO', ', '.join('%s %s' % (p.get('id'), p.get('state')) for p in owners) or 'none')
        self.item(items, 'plugin roster problem', 'PASS' if not plug.get('problem') else 'FAIL', str(plug.get('problem') or 'none'))
        roster = self.sh('cat /var/lib/lumatv/plugins/enabled.json 2>/dev/null || echo MISSING', 15).strip()
        approved = [e.get('id') for e in (self.jl(roster).get('plugins') or [])] if roster != 'MISSING' else None
        evidence['approved_in_enabled_json'] = approved
        self.item(items, 'approved plugins (enabled.json)', 'INFO', 'no file (nothing approved)' if approved is None else ', '.join(map(str, approved)) or 'none')
        sock = self.sh('ls -l /run/lumashow-plugins/broker.sock 2>&1 | cut -c1-80', 15).strip()
        self.item(items, 'broker socket', 'INFO', 'exists only while an owner plugin is on: ' + (sock or 'no answer'))
        # first-party plugins deliver cards
        firsts = {p.get('id'): p for p in first}
        absent = [i for i in ('weather', 'news', 'clock') if i not in firsts or firsts[i].get('state') == 'inert']
        self.item(items, 'first-party plugins weather, news, clock', 'FAIL' if absent else 'PASS',
                  ('missing or inert: %s' % absent) if absent else 'listed: %s' % ', '.join('%s %s' % (i, firsts[i].get('state')) for i in ('weather', 'news', 'clock')))
        cards = {}
        for pid, source in (('weather', 'weather'), ('news', 'news'), ('clock', None)):
            doc, _ = self.api('/v1/state/plugin.%s.cards' % pid)
            its = ((doc or {}).get('value') or {}).get('items') or []
            cards[pid] = {'cards': len(its), 'ids': [c.get('id') for c in its], 'sizes': [(c.get('card') or {}).get('size') for c in its]}
            if its:
                self.item(items, 'first-party card: %s' % pid, 'PASS', '%d card(s)' % len(its))
            elif source and not (self.api('/v1/state/%s' % source)[0] or {}).get('value'):
                self.item(items, 'first-party card: %s' % pid, 'EYES', 'no card, and the %s topic has no data either (no fetch yet: not the plugin\'s fault)' % source)
            else:
                self.item(items, 'first-party card: %s' % pid, 'FAIL', 'no card although its source has data' if source else 'the clock plugin published no card')
        evidence['first_party_cards'] = cards
        amb, _ = self.api('/v1/state/plugin.ambient.tiles')
        evidence['ambient_tiles'] = len((((amb or {}).get('value')) or {}).get('items') or [])
        self.item(items, 'first-party tiles: ambient', 'INFO', '%d tile(s) for the quick panel' % evidence['ambient_tiles'])
        # the Home Assistant plugin: its state, never its address or token
        link, link_text = self.api('/v1/state/ha.link')
        lv = ((link or {}).get('value')) or {}
        words = ha_words(owners, lv)
        evidence['home_assistant'] = {'words': words, 'link_state': lv.get('state'), 'link_keys': sorted(lv.keys()),
                                      'plugin': next((view([p])[0] for p in owners if p.get('id') == 'home-assistant'), None)}
        bad_token = {k: v for k, v in lv.items() if 'token' in k.lower() and not isinstance(v, (bool, type(None)))}
        self.item(items, 'ha.link carries no token', 'FAIL' if bad_token else 'PASS', ('keys with a value: %s' % sorted(bad_token)) if bad_token else 'keys %s' % sorted(lv.keys()))
        if words in ('Not switched on', 'Not linked'):
            self.item(items, 'Home Assistant plugin state', 'PASS', words + ' (expected until Nitin links it)')
        elif words in ('Connected', 'Connecting'):
            self.item(items, 'Home Assistant plugin state', 'EYES', words + ': somebody has linked it since; not expected tonight')
        else:
            self.item(items, 'Home Assistant plugin state', 'FAIL', words + ' (reason %s)' % ((evidence['home_assistant']['plugin'] or {}).get('reason')))
        with open(os.path.join(d, 'plugins.json'), 'w') as fh:
            json.dump(evidence, fh, indent=1)
        # what the TV draws: Home's At-home lane, and the quick panel's plugin tiles (focus moves only, no OK)
        lane = self.shot('home-lane', sub)
        self.kb('DOWN s0.8')
        self.shot('home-lane-down', sub)
        self.kb('DOWN s0.8')
        self.shot('home-lane-down2', sub)
        self.kb('UPx2@0.4 s0.8')
        self.item(items, 'Home At-home lane', 'EYES', 'the chip says "Home Assistant / %s" (or the plugins\' cards); three captures of Home, focus moved with Down and back' % words, file=sub + '/home-lane.png')
        panel, noise, why = self.open_quick('q', sub)
        if panel is None:
            self.item(items, 'quick panel plugin tiles', 'FAIL', why)
        else:
            self.kb('DOWNx8@0.25 s0.6')
            self.shot('quick-mid', sub)
            self.kb('DOWNx6@0.25 s0.6')
            self.shot('quick-low', sub)
            self.kb('BACK s1.2')
            self.item(items, 'quick panel plugin tiles', 'EYES', 'tiles of plugins sit between the built-in ones and AI access (ambient: %d); look at quick-mid and quick-low' % evidence['ambient_tiles'],
                      file=sub + '/quick-mid.png')
        self.kb('BACK s1.0')
        bad = [i for i in items if i['verdict'] in ('FAIL', 'ERROR')]
        eyes = [i for i in items if i['verdict'] == 'EYES']
        return {'verdict': 'FAIL' if bad else ('EYES' if eyes else 'PASS'), 'items': items, 'files': ['plugins/plugins.json'],
                'detail': ('failed: ' + ', '.join(i['name'] for i in bad)) if bad else 'helper, roster, first-party cards and the Home Assistant state read; captures need a look (Home Assistant: %s)' % words}

    def close_settings(self, sub, tag):
        self.kb('BACKx10@0.4 s1.0')
        self.shot(tag + '-closed', sub, keep=False)

    def walk(self, sub, tag, steps, items, prev):
        """steps: [(item name, kb tokens, shot name, what to look for)]. Keys only (OK only where a step says it opens a page)."""
        for name, toks, shot, look in steps:
            self.kb(toks)
            img = self.shot(shot, sub)
            if img is None:
                self.item(items, name, 'ERROR', 'no capture')
                continue
            st = box_stats(img, x_max=960, y0=0, y1=540, floor=10)
            blank = st is None
            same = prev is not None and mean_diff(prev, img) < 0.05
            prev = img
            verdict = 'FAIL' if blank else 'EYES'
            detail = 'blank screen' if blank else ('unchanged from the step before; ' if same else '') + look
            self.item(items, name, verdict, detail, file='%s/%s.png' % (sub, shot))
        return prev

    def check_settings(self):
        sub = 'settings'
        items, notes = [], []
        pre = {'picture': self.picture_mode(), 'flags': self.general_flags(), 'theme': self.theme_md5()}
        start_fg = self.start.get('foreground', '')
        if self.fg() != 'com.webos.app.home':
            self.home()
        # 1. System > About > Support > Privacy > App Permissions, Delete My Personal Information
        st, why = self.open_settings('w1', sub)
        if st is None:
            self.item(items, 'open Settings', 'FAIL', why)
            self.kb('BACKx4@0.4 s1.0')
        else:
            prev = self.walk(sub, 'w1', W1_STEPS, items, st)
            self.close_settings(sub, 'w1')
        # 2. Home & look > Home tab > Home Assistant
        if self.fg() != 'com.webos.app.home':
            self.home()
        st, why = self.open_settings('w2', sub)
        if st is None:
            self.item(items, 'open Settings (2)', 'FAIL', why)
            self.kb('BACKx4@0.4 s1.0')
        else:
            self.walk(sub, 'w2', W2_STEPS, items, st)
            self.close_settings(sub, 'w2')
        # 3. Just Scan: only while an HDMI input was in front at the start; no input is ever switched here
        if HDMI_APP.match(start_fg) and self.fg() == start_fg:
            st, why = self.open_settings('w3', sub)
            if st is None:
                self.item(items, 'Just Scan: open Settings', 'FAIL', why)
                self.kb('BACKx4@0.4 s1.0')
            else:
                self.walk(sub, 'w3', W3_STEPS, items, st)
                self.close_settings(sub, 'w3')
        else:
            self.item(items, 'Just Scan', 'SKIPPED', 'no HDMI input in front at the start (foreground %s); no input is switched for it' % (start_fg or '?'))
        post = {'picture': self.picture_mode(), 'flags': self.general_flags(), 'theme': self.theme_md5()}
        same = pre == post
        self.item(items, 'no value changed by the walk', 'PASS' if same else 'FAIL',
                  'picture mode, Limit AD Tracking, Do Not Sell and the theme file are as before' if same else 'before %s, after %s' % (pre, post))
        bad = [i for i in items if i['verdict'] in ('FAIL', 'ERROR')]
        looked = [i for i in items if i['verdict'] == 'EYES']
        return {'verdict': 'FAIL' if bad else 'EYES', 'items': items,
                'detail': ('%d step(s) failed: %s' % (len(bad), ', '.join(i['name'] for i in bad))) if bad else '%d pages captured, none changed a value; look at the images' % len(looked)}

    # ========================================================================================= check: LG system-update block
    # Read only: `lumatv updateblock status` and `lumatv status`, the Homebrew Channel's getConfiguration, the flag file, /etc/hosts, the helper's
    # `updateblock` topic. Nothing is written, nothing is pressed; the setting is changed only in Settings, by a person.

    def check_updateblock(self):
        items = []
        d = os.path.join(self.out, 'updateblock')
        os.makedirs(d, exist_ok=True)
        ev = {}
        names = ('snu.lge.com', 'su.lge.com', 'su-ssl.lge.com', 'su-dev.lge.com')
        # (a) the switch's state: the channel's own setting, the flag file behind it, and what lumatv and the helper say
        ub = self.jl(self.sh(TOOL + ' updateblock status 2>&1', 30))
        u = ub.get('updateBlock') or {}
        ev['lumatv_updateblock'] = u
        self.item(items, 'lumatv updateblock status answers', 'PASS' if u else 'FAIL', json.dumps(u)[:200] if u else 'no updateBlock in the reply: %r' % str(ub)[:160])
        st_raw = self.sh(TOOL + ' status 2>&1', 60)
        st = self.jl(st_raw)
        self.item(items, 'lumatv status carries updateBlock, no shell error', 'PASS' if st.get('updateBlock') and 'not found' not in st_raw else 'FAIL',
                  'state %s' % (st.get('updateBlock') or {}).get('state') if st.get('updateBlock') else 'missing from `lumatv status`' + (' ("not found" in the output)' if 'not found' in st_raw else ''))
        hbc = self.jl(self.luna('luna://org.webosbrew.hbchannel.service/getConfiguration', '{}'))
        ev['hbchannel_getConfiguration'] = {k: hbc.get(k) for k in ('returnValue', 'root', 'blockUpdates', 'errorText')}
        if isinstance(hbc.get('blockUpdates'), bool):
            self.item(items, "Homebrew Channel's blockUpdates (the switch)", 'PASS' if hbc['blockUpdates'] else 'FAIL',
                      'blockUpdates %s: %s' % (str(hbc['blockUpdates']).lower(), 'the switch reads ON' if hbc['blockUpdates'] else 'the switch reads OFF: LG\'s update can reach this TV'))
        else:
            self.item(items, "Homebrew Channel's blockUpdates (the switch)", 'INFO', 'luna-send got no setting from the channel (%s); Settings asks as the compositor, which may differ' % (hbc.get('errorText') or 'no answer'))
        flag = self.sh('[ -e /var/luna/preferences/webosbrew_block_updates ] && echo yes || echo no', 15).strip()
        self.item(items, 'flag file /var/luna/preferences/webosbrew_block_updates', 'PASS' if flag == 'yes' else 'FAIL', flag)
        # (b) the hosts block itself, as the TV resolves it now
        hosts = self.sh("grep -n 'lge.com' /etc/hosts 2>&1; echo; grep -c ' /etc/hosts ' /proc/mounts", 15)
        ev['etc_hosts_lge_lines'] = hosts.strip().splitlines()
        with open(os.path.join(d, 'etc-hosts-lge.txt'), 'w') as fh:
            fh.write(hosts)
        mapped = [n for n in names if re.search(r'^\s*\d+:\s*(127\.0\.0\.1|::1)\s[^#]*\b%s\b' % re.escape(n), hosts, re.M)]
        self.item(items, '/etc/hosts maps LG\'s four update servers to loopback', 'PASS' if len(mapped) == 4 else 'FAIL',
                  'mapped: %s%s' % (', '.join(mapped) or 'none', '' if len(mapped) == 4 else '; missing: ' + ', '.join(n for n in names if n not in mapped)))
        binds = hosts.strip().splitlines()[-1].strip() if hosts.strip() else ''
        self.item(items, '/etc/hosts bind mounts', 'INFO', '%s in /proc/mounts (more than one stacks, as Starfish\'s once hid the block; the mapping above is what is read through the top one)' % binds)
        name = 'hosts-block'
        res = self.sh('getent hosts su.lge.com 2>/dev/null | head -n1; ping -c1 -W1 su.lge.com 2>&1 | head -n1', 15).strip()
        ev['resolution'] = res.splitlines()[:2]
        self.item(items, 'su.lge.com resolves to this TV', 'PASS' if re.search(r'127\.0\.0\.1|::1', res) else 'FAIL', res[:160] or 'no answer')
        # the helper's topic, read the same way as the page does
        doc, _ = self.api('/v1/state/updateblock')
        topic = (doc or {}).get('value') or {}
        ev['helper_topic'] = topic
        agree = bool(topic) and topic.get('state') == u.get('state')
        self.item(items, "helper's updateblock topic agrees with lumatv", 'PASS' if agree else 'FAIL',
                  'both say %s' % topic.get('state') if agree else 'helper %s, lumatv %s' % (topic.get('state'), u.get('state')))
        state = u.get('state') or topic.get('state')
        self.item(items, 'updates are blocked now', 'PASS' if state == 'blocked' else 'FAIL',
                  {'blocked': 'flag on and all four names mapped', 'armed': 'flag on, hosts not mapped yet (applies at the next restart)',
                   'hosts-only': 'flag off but still mapped until the next restart', 'open': 'neither: LG\'s update can reach this TV'}.get(state, 'state %r' % state))
        with open(os.path.join(d, 'updateblock.json'), 'w') as fh:
            json.dump(ev, fh, indent=1)
        bad = [i['name'] for i in items if i['verdict'] in ('FAIL', 'ERROR')]
        return {'verdict': 'FAIL' if bad else 'PASS', 'items': items, 'files': ['updateblock/updateblock.json', 'updateblock/etc-hosts-lge.txt'],
                'detail': ('failed: ' + ', '.join(bad)) if bad else 'blocked: flag on, four names mapped, helper and lumatv agree'}

    # ================================================================================================= check: BLE scans

    # ============================================================================================= check: layout editor
    # Through Settings > Home & look > Home tab > Home widgets > "Edit on Home" (the same editor as holding OK on a card, but no
    # guess about which card Home has focused). Nothing is saved: the editor is left through Cancel and Discard, and the owner's layout.json
    # must be byte for byte as before (md5; absent stays absent). OK is pressed only on rows this plan names, and the dialog's keys
    # only once the screen looks like the dialog.

    def layout_file(self):
        """(md5 or 'absent', mtime or '', content text) of the owner's layout.json, read straight from the file (`lumatv layout get`
        would stage the theme when it is missing)."""
        f = LAYOUT_FILE
        md5 = self.sh('if [ -f %s ]; then md5sum %s | cut -d" " -f1; else echo absent; fi' % (f, f), 20).strip()
        mtime = self.sh('stat -c %%Y %s 2>/dev/null; true' % f, 15).strip()
        text = self.sh('cat %s 2>/dev/null || echo ABSENT' % f, 15)
        return md5, mtime, text

    def check_layout(self):
        sub = 'layout'
        items = []
        before = self.layout_file()
        doc = self.jl(before[2]) if before[0] != 'absent' else {}
        customised = any(v for v in doc.values()) if isinstance(doc, dict) else False
        self.item(items, 'layout.json before', 'INFO', 'md5 %s, mtime %s, %s' % (before[0], before[1] or '-', 'customised' if customised else 'not customised (the shipped layout)'))
        if self.fg() != 'com.webos.app.home':
            self.home()
        home_noise = None
        st, why = self.open_settings('l1', sub)
        if st is None:
            self.item(items, 'open Settings', 'FAIL', why)
            self.kb('BACKx4@0.4 s1.0')
            return self.layout_result(items, before, 'could not open Settings')
        # Settings > Home & look > Home tab > Home widgets (first row of the tab), captured with nothing changed
        self.kb('UPx12@0.1 DOWNx5@0.3 s0.5')
        self.kb('RIGHT s0.8')
        self.kb('UPx15@0.1 RIGHTx6@0.4 LEFTx2@0.5 s0.8')
        self.kb('DOWN s0.6 OK s2.0')
        page = self.shot('home-widgets', sub)
        self.kb('UPx15@0.1 RIGHT s1.2')
        self.shot('home-widgets-order', sub)
        self.kb('LEFT s0.8')
        self.item(items, 'Settings > Home & look > Home widgets', 'EYES', 'the page on its first tab (a row per card, Off/Small/Half/Wide) and its second tab (the order); nothing was changed',
                  file=sub + '/home-widgets.png')
        # the "Edit on Home" row: the last row, or the one before it while Reset to default is enabled (a customised layout)
        self.kb('DOWN s0.5')
        self.kb('DOWNx30@0.08 s0.6')
        if customised:
            self.kb('UP s0.6')
        before_open = self.shot('edit-row-focus', sub)
        self.kb('OK s2.2')
        grid = self.shot('edit-grid', sub)
        noise = mean_diff(self.shot('edit-grid-b', sub, keep=False), grid)
        opened = self.dry or (before_open is not None and grid is not None and mean_diff(before_open, grid) > SEEN_FACTOR * noise + SEEN_FLOOR)
        if not opened:
            self.item(items, 'layout editor opens', 'FAIL', 'the screen did not change after OK on "Edit on Home"; no editor keys were sent')
            self.kb('BACKx4@0.4 s1.0')
            return self.layout_result(items, before, 'the editor did not open')
        self.item(items, 'layout editor opens', 'EYES', 'the grid over Home with the bar (Add, Remove, Undo, Cancel, Save); look at edit-grid.png', file=sub + '/edit-grid.png')
        thr = max(0.4, SEEN_FACTOR * noise)
        # pick a card up, move it one cell, turn to the size, size it one step, put it down
        self.kb('OK s0.7')
        prev = self.shot('edit-pickup', sub)
        moved = None
        for key in ('RIGHT', 'LEFT', 'DOWN', 'UP'):
            self.kb('%s s0.8' % key)
            now = self.shot('edit-move-%s' % key.lower(), sub)
            if self.dry or mean_diff(prev, now) > thr:
                moved = key
                break
            prev = now
        self.item(items, 'pick up and move one cell', 'EYES' if moved else 'INFO', ('moved %s: compare edit-pickup and edit-move-%s' % (moved.lower(), moved.lower())) if moved else
                  'no direction changed the picture (the card may fill its row and column, or the move was refused)')
        self.kb('OK s0.7')
        prev = self.shot('edit-size-mode', sub)
        sized = None
        for key in ('RIGHT', 'LEFT', 'DOWN', 'UP'):
            self.kb('%s s0.8' % key)
            now = self.shot('edit-size-%s' % key.lower(), sub)
            if self.dry or mean_diff(prev, now) > thr:
                sized = key
                break
            prev = now
        self.item(items, 'resize one step', 'EYES' if sized else 'INFO', ('sized with %s: compare edit-size-mode and edit-size-%s' % (sized.lower(), sized.lower())) if sized else
                  'no direction changed the picture (the size may be at its limit, or refused)')
        self.kb('OK s0.8')
        after = self.shot('edit-changed', sub)
        # Cancel: Back with nothing picked up asks "Save changes?" when something changed (Keep editing, Save, Discard, the last one danger)
        self.kb('BACK s1.0')
        dlg = self.shot('edit-cancel', sub)
        homeish = mean_diff(dlg, after) < thr
        if dialog_like(dlg) and (self.dry or not homeish):
            self.kb('RIGHTx5@0.25 s0.6')          # the dialog stops at its last answer: Discard
            self.shot('edit-discard-focus', sub)
            self.kb('OK s1.8')
            self.item(items, 'Cancel and confirm Discard', 'EYES', 'the question was recognised and Discard (the last answer) was chosen; see edit-cancel.png and edit-discard-focus.png')
        elif moved or sized:
            self.item(items, 'Cancel and confirm Discard', 'FAIL', 'the screen after Back is not recognisably the question; no key sent. The editor may still be open with unsaved changes: Back, then Discard, at the remote')
            return self.layout_result(items, before, 'the discard question was not recognised')
        else:
            self.item(items, 'Cancel and confirm Discard', 'INFO', 'nothing changed, so Back left the editor without a question (edit-cancel.png)')
        left = self.shot('edit-left', sub)
        self.sleep(1.0)
        return self.layout_result(items, before, '')

    def layout_result(self, items, before, stopped):
        after = self.layout_file()
        same = after[0] == before[0]
        self.item(items, 'layout.json unchanged', 'PASS' if same else 'FAIL', ('md5 %s before and after (mtime %s then %s)' % (before[0], before[1] or '-', after[1] or '-')) if same
                  else 'md5 %s before, %s after: THE LAYOUT WAS WRITTEN' % (before[0], after[0]))
        bad = [i for i in items if i['verdict'] in ('FAIL', 'ERROR')]
        eyes = [i for i in items if i['verdict'] == 'EYES']
        return {'verdict': 'FAIL' if bad else ('EYES' if eyes else 'PASS'), 'items': items,
                'detail': ('failed: ' + ', '.join(i['name'] for i in bad)) if bad else 'editor opened, a card moved and sized, discarded; layout.json unchanged; look at the captures'}

    def check_ble(self):
        items = []
        self.sh('mkdir -p %s' % R, 15)
        adapter = self.jl(self.luna(BT + '/adapter/getStatus'))
        powered = any(a.get('powered') for a in adapter.get('adapters', []))
        devs = self.jl(self.luna(BT + '/device/getStatus')).get('devices', [])
        audio = [d for d in devs if any('a2dp' in str(p).lower() for p in d.get('connectedProfiles', []))]
        self.item(items, 'baseline: adapter', 'PASS' if powered else 'FAIL', 'powered' if powered else 'the adapter is not powered (reply keys %s)' % sorted(adapter.keys()))
        if audio:
            return {'verdict': 'SKIPPED', 'items': items, 'detail': 'a Bluetooth audio device is connected (%d): no scan while audio may play' % len(audio)}
        if not powered:
            return {'verdict': 'FAIL', 'items': items, 'detail': 'the adapter is not powered'}
        le = self.jl(self.luna(BT + '/le/getStatus'))
        self.log('le/getStatus keys: %s' % sorted(le.keys()))
        gw = "GW=$(ip route 2>/dev/null | awk '/default/{print $3; exit}'); "
        self.sh(gw + 'ping -c 10 -W 2 "$GW" > %s/ping-base.txt 2>&1; top -b -n 1 | head -5 > %s/top-base.txt' % (R, R), 40)
        base_ping = ping_summary(self.sh('cat %s/ping-base.txt' % R, 15))
        load_base = self.sh('cat %s/top-base.txt' % R, 15).strip().splitlines()[:2]
        self.log('baseline ping %s, load %s' % (base_ping, load_base))
        runs = {}
        for tag, payload in (('scan1', '{"subscribe":true}'), ('scan2', '{"subscribe":true,"always":true}')):
            if runs:
                self.sleep(20)          # the helper's own rule: never more than half the time on air
            mark = int(self.sh('wc -l < /var/log/messages', 15).strip() or 0)
            self.scan_ran = True
            self.sh(gw + "timeout 15 luna-send -n 1000000 %s/le/startScan '%s' > %s/%s.txt 2>&1 & ping -c 14 -W 2 \"$GW\" > %s/%s-ping.txt 2>&1; wait; echo scanned"
                    % (BT, payload, R, tag, R, tag), 60)
            self.sleep(15)
            text = self.sh('head -c 4000000 %s/%s.txt' % (R, tag), 60)
            ping = ping_summary(self.sh('cat %s/%s-ping.txt' % (R, tag), 15))
            left = self.sh("ps | grep '[s]tartScan' | wc -l", 15).strip()
            dropped = self.sh("tail -n +%d /var/log/messages | grep -a -c 'StartScan listener dropped'" % (mark + 1), 30).strip()
            stats = scan_stats(split_json(text))
            stats.update({'raw_bytes': len(text), 'ping_during': ping, 'luna_send_left_over': left, 'listener_dropped_lines': dropped})
            runs[tag] = stats
            self.log('%s: %s replies, %s addresses, ping %s, left over %s, dropped lines %s' % (tag, stats['replies'], stats['addresses'], ping, left, dropped))
        s1, s2 = runs['scan1'], runs['scan2']
        f = s1['first_reply']
        if f['returnValue'] is True or f['subscribed'] is True:
            self.item(items, 'unknown 1: luna-send may call le/startScan', 'PASS', 'first reply %s' % f)
        else:
            self.item(items, 'unknown 1: luna-send may call le/startScan', 'FAIL', 'first reply %s' % f)
        sr = s1['scan_record']
        if s1['addresses'] == 0:
            self.item(items, 'unknown 2: scanRecord', 'EYES', 'no device was heard in 15 s (a phone advertising or a beacon in range is needed); nothing to judge', scan_record=sr)
        elif sr['with_record'] > 0:
            self.item(items, 'unknown 2: scanRecord', 'PASS', 'filled for %d reply entries, %s-%s bytes, %d over 31 bytes (advertisement plus scan response when over 31) [INFERENCE], AD types %s, name in %d'
                      % (sr['with_record'], sr['length_min'], sr['length_max'], sr['records_over_31_bytes'], sr['ad_types_seen'], sr['records_with_name']), scan_record=sr)
        else:
            self.item(items, 'unknown 2: scanRecord', 'FAIL', 'devices heard (%d) but no scanRecord with bytes: ble_scan::advertisement rebuilds name and manufacturer data only' % s1['addresses'], scan_record=sr)
        self.item(items, 'unknown 3: same advertisement, same RSSI', 'EYES',
                  'without always: %d replies for a device heard again, %d of them with the same RSSI; with always: %d and %d. Replies %d vs %d. %s'
                  % (s1['same_address_again'], s1['same_address_again_same_rssi'], s2['same_address_again'], s2['same_address_again_same_rssi'], s1['replies'], s2['replies'],
                     '--always adds repeats' if s2['replies'] > s1['replies'] else 'always changes nothing seen'))
        clean = all(r['luna_send_left_over'] == '0' for r in runs.values())
        self.item(items, 'no luna-send left over', 'PASS' if clean else 'FAIL', 'left over: %s' % [r['luna_send_left_over'] for r in runs.values()])
        self.item(items, 'cost (unknown 5, partial)', 'EYES', 'ping round trip base %s, scan %s, always %s; load before %s' % (base_ping, s1['ping_during'], s2['ping_during'], load_base))
        d = os.path.join(self.out, 'ble')
        os.makedirs(d, exist_ok=True)
        with open(os.path.join(d, 'scan-shapes.json'), 'w') as fh:
            json.dump({'note': 'counts and shapes only: no address, name or advertisement byte of any device was written',
                       'baseline_ping': base_ping, 'runs': runs}, fh, indent=1)
        bad = [i['name'] for i in items if i['verdict'] in ('FAIL', 'ERROR')]
        return {'verdict': 'FAIL' if bad else 'EYES', 'items': items, 'detail': ('failed: ' + ', '.join(bad)) if bad else 'unknowns 1-3 answered in ble/scan-shapes.json (unknown 3 and the cost read by eye)',
                'files': ['ble/scan-shapes.json']}

    # ============================================================================================= check: input banners

    def burst(self, name, app_id, ms, sub):
        """Launch app_id and capture frames for ms from the launch call's return, on the set; returns {frame name: (ms, Image, md5)}."""
        rd = '%s/%s' % (R, name)
        self.launch_noted(app_id)
        self.sh('nohup sh %s/burst.sh %s %s %s %d >/dev/null 2>&1 &' % (R, rd, name, app_id, ms), 15)
        self.started_apps.add(app_id)
        self.sleep(2.5)
        proc = self.process_of(app_id)
        self.sleep(ms / 1000.0 - 2.0)
        for _ in range(12):
            if 'done' in self.sh('cat %s/done 2>/dev/null' % rd, 15):
                break
            self.sleep(1.5)
        times = {}
        for line in self.sh('cat %s/times' % rd, 15).splitlines():
            m = re.match(r'^(\d+) (\d+)$', line.strip())
            if m:
                times[int(m.group(1))] = int(m.group(2))
        launch_ms = re.search(r'launch-call-ms (\d+)', self.sh('head -1 %s/times' % rd, 15))
        md5s = {}
        for line in self.sh('cat %s/md5.txt' % rd, 15).splitlines():
            m = re.match(r'^([0-9a-f]{32})\s+\.?/?(\S+)$', line.strip())
            if m:
                md5s[m.group(2)] = m.group(1)
        paths = ['%s/%s-%02d.png' % (rd, name, n) for n in sorted(times)]
        data = self.fetch(paths, 240)
        frames = {}
        for n in sorted(times):
            p = '%s/%s-%02d.png' % (rd, name, n)
            fname = '%s-%02d' % (name, n)
            if p in data:
                frames[fname] = (times[n], load(data[p]), md5s.get(fname + '.png', hashlib.md5(data[p]).hexdigest()))
        return frames, proc, int(launch_ms.group(1)) if launch_ms else None

    def launch_noted(self, app_id):
        self.log('launch %s (the burst script launches it)' % app_id)

    def process_of(self, app_id):
        pid = None
        for a in self.running():
            if a.get('id') == app_id:
                pid = str(a.get('processid') or a.get('pid') or '')
        cmdline = self.sh("p=%s; tr '\\0' ' ' < /proc/$p/cmdline 2>/dev/null | cut -c1-300" % shlex.quote(pid or '0'), 15).strip() if pid else ''
        return {'pid': pid, 'snapshot_flag': '"snapshot":true' in cmdline.replace(' ', ''), 'cmdline': cmdline}

    def check_banners(self):
        sub = 'banners'
        items = []
        app = 'com.webos.app.hdmi%s' % self.args.hdmi
        ic = self.inputcommon()
        on = bool(ic.get('mounted')) and bool(ic.get('staged'))
        self.item(items, 'inputcommon state', 'INFO', 'on (staged and mounted) - the PASS/FAIL below judges the patched banners' if on
                  else 'OFF: nothing below can judge the patched banners; the burst is LG\'s own look (baseline). Main switches `lumatv inputcommon on` first.', status=ic)
        if self.fg() != 'com.webos.app.home':
            self.home()
        self.sh('mkdir -p %s' % R, 15)
        self.tv.put(os.path.join(HERE, 'burst.sh'), R + '/burst.sh')
        if on:
            mounts = self.sh("grep -c inputcommon /proc/mounts", 15).strip()
            want = banner_tree()
            count = self.sh("find %s -name '*.qml' 2>/dev/null | wc -l" % IC_UPPER, 15).strip()
            absent = self.sh('echo missing-files; for f in %s; do [ -f %s/$f ] || echo "$f"; done' % (' '.join(shlex.quote(f) for f in want['files']), IC_UPPER), 15).replace('missing-files', '').split()
            absent = [a for a in absent if a != 'none']
            aside = self.sh('ls -A /tmp/checkpoint 2>/dev/null', 15)
            ok_aside = '.lumatv-aside.com.webos.app.inputcommon' in aside and not re.search(r'(^|\n)com\.webos\.app\.inputcommon(\n|$)', aside)
            self.item(items, 'overlay mounted', 'PASS' if mounts.isdigit() and int(mounts) > 0 else 'FAIL', '%s inputcommon lines in /proc/mounts' % mounts)
            if want['count'] is not None:
                self.item(items, 'patched tree: %d .qml files' % want['count'], 'PASS' if count == str(want['count']) else 'FAIL',
                          '%s files in %s; %d expected from running tools/inputcommon_patch.py on the census copy of LG\'s tree' % (count, IC_UPPER, want['count']))
            else:
                self.item(items, 'patched tree: at least %d .qml files' % want['at_least'], 'PASS' if count.isdigit() and int(count) >= want['at_least'] else 'FAIL',
                          '%s files in %s; no census copy of LG\'s tree on this host, so only the lower bound from tools/inputcommon_patch.py' % (count, IC_UPPER))
            self.item(items, 'patch files present (incl. LumaStyle.qml)', 'PASS' if not absent else 'FAIL',
                      '%d of %d from tools/inputcommon_patch.py%s' % (len(want['files']) - len(absent), len(want['files']), ('; missing: ' + ', '.join(absent)) if absent else ''))
            self.item(items, 'LG dump set aside', 'PASS' if ok_aside else 'FAIL', aside.strip().replace('\n', ' ')[:200])
        # the pill and the no-signal sheet: one burst of HDMI N
        frames, proc, launch_ms = self.burst('pill', app, 12000, sub)
        self.item(items, 'input process', 'INFO', 'pid %s, "snapshot":true %s (7890 without the flag is the 04:04 dump; 2513/2517 with it LG\'s boot instances)' % (proc['pid'], proc['snapshot_flag']), process=proc)
        kinds = {}
        for fname, (ms, img, md5) in frames.items():
            k, st = ('splash', None) if md5 == SPLASH_MD5 else classify_pill(img)
            kinds[fname] = {'ms': ms, 'kind': k, 'box': st, 'md5': md5}
        glass = [(v['ms'], f) for f, v in kinds.items() if v['kind'] == 'glass']
        counts = {}
        for v in kinds.values():
            counts[v['kind']] = counts.get(v['kind'], 0) + 1
        if not on:
            self.item(items, 'pill (baseline, switch off)', 'INFO', 'frame kinds %s; glass frames %d' % (counts, len(glass)))
        elif glass:
            self.item(items, 'pill: the dark glass pill shows', 'PASS', '%d frames, %d ms to %d ms after the launch call (44 px high at x 32 expected); other frames %s' % (len(glass), min(glass)[0], max(glass)[0], counts))
        else:
            self.item(items, 'pill: the dark glass pill shows', 'FAIL', 'no frame shows it; frame kinds %s (grey = LG\'s pill, bare = text with no box: the style fell back)' % counts)
        late = sorted((v['ms'], f) for f, v in kinds.items() if v['ms'] >= 7000)
        self.item(items, 'no-signal sheet / key-help line (at 7 s and later)', 'EYES', 'frames %s: dim ink words in Manrope on black expected; the key-help line (red bar and words) is INFO when not drawn'
                  % [f for _, f in late[:3]])
        # evidence: the frames that matter, a contact sheet, the times and the md5s
        d = os.path.join(self.out, sub)
        os.makedirs(d, exist_ok=True)
        keep = []
        if glass:
            order = sorted(glass)
            keep += [order[0][1], order[len(order) // 2][1], order[-1][1]]
        ordered = sorted(kinds, key=lambda f: kinds[f]['ms'])
        if ordered:
            keep += [ordered[0], ordered[-1]] + [f for _, f in late[:1]]
        keep = list(dict.fromkeys(keep))
        for f in keep:
            save_png(frames[f][1], os.path.join(d, f + '.png'))
        montage([('%s %dms %s' % (f, kinds[f]['ms'], kinds[f]['kind']), frames[f][1]) for f in ordered[:: max(1, len(ordered) // 12)]], os.path.join(d, 'pill-montage.png'))
        with open(os.path.join(d, 'pill-frames.json'), 'w') as fh:
            json.dump({'launch_call_ms': launch_ms, 'process': proc, 'frames': kinds}, fh, indent=1)
        # the INFO card, over the HDMI input
        if self.args.no_info_key:
            self.item(items, 'INFO card', 'SKIPPED', '--no-info-key')
        else:
            self.info_card(items, sub, app, on)
        if self.args.livetv:
            self.live_tv(items, sub)
        else:
            self.item(items, 'Live TV channel strip and card', 'SKIPPED', 'tuning a channel plays audio; pass --livetv with the receiver muted')
        self.item(items, 'prompts (Input Locked, set-top box question)', 'SKIPPED', 'not reachable without locking an input; the host proof stands')
        # leave: close the input, wait, and check nothing runs
        self.close_inputs(items)
        bad = [i for i in items if i['verdict'] in ('FAIL', 'ERROR')]
        eyes = [i for i in items if i['verdict'] == 'EYES']
        verdict = 'FAIL' if bad else ('EYES' if eyes or not on else 'PASS')
        if not on and not bad:
            verdict = 'SKIPPED'
        return {'verdict': verdict, 'items': items, 'files': ['banners/pill-montage.png', 'banners/pill-frames.json'],
                'detail': ('failed: ' + ', '.join(i['name'] for i in bad)) if bad else ('inputcommon is off: baseline only, nothing judged' if not on else 'automatic parts passed; the pictures list what to look at')}

    def info_card(self, items, sub, app, on):
        pre = self.shot('pre-info', sub, keep=False)
        self.kb('358 s1.3')
        a = self.shot('info-1', sub)
        self.sleep(1.0)
        self.shot('info-2', sub)
        self.kb('BACK s1.0')
        kind, lumas = classify_info(a)
        moved = mean_diff(pre, a) > 1.0 if pre is not None and a is not None else False
        if not moved and not self.dry:
            self.item(items, 'INFO card', 'EYES', 'the screen did not change after the INFO key (the key may have gone to the source over CEC); look at info-1.png', file=sub + '/info-1.png')
            return
        if not on:
            self.item(items, 'INFO card (baseline, switch off)', 'INFO', 'colour %s, samples %s' % (kind, lumas), file=sub + '/info-1.png')
        elif kind == 'glass':
            self.item(items, 'INFO card colour: dark glass', 'EYES', 'samples %s: dark. Still by eye: 34 px corners, hairline, "HDMI 2" dim over the device name, chips as light-wash tiles, about 550 px wide' % lumas, file=sub + '/info-1.png')
        elif kind == 'grey':
            self.item(items, 'INFO card colour: dark glass', 'FAIL', 'samples %s: LG\'s grey sheet (#575E66-like)' % lumas, file=sub + '/info-1.png')
        else:
            self.item(items, 'INFO card colour: dark glass', 'EYES', 'samples %s: not clearly dark or grey (the picture behind shows through, or the source drew its own overlay); look at info-1.png' % lumas, file=sub + '/info-1.png')

    def live_tv(self, items, sub):
        self.launch('com.webos.app.livetv')
        self.sleep(6)
        self.shot('channel', sub)
        self.kb('OK s1.2')
        self.shot('channel-ok', sub)
        self.kb('RIGHT s1.0')
        self.shot('channel-options', sub)
        self.kb('358 s1.3')
        self.shot('channel-info', sub)
        self.kb('BACK s1.0')
        self.close_app('com.webos.app.livetv')
        self.sleep(2)
        self.item(items, 'Live TV channel strip and INFO card', 'EYES', 'dark glass card, 34 px corners, hairline, number/station/programme in Manrope, violet progress fill, about 880 px wide', file=sub + '/channel-info.png')

    def close_inputs(self, items):
        for a in sorted(set(self.started_apps)):
            if HDMI_APP.match(a) or a in ('com.webos.app.livetv',):
                self.close_app(a)
        self.sleep(3)
        run = self.inputs().get('running')
        self.item(items, 'inputs closed', 'PASS' if run == [] else 'FAIL', 'inputs status running %s' % run)

    # ================================================================================================ the whole slot

    def finish(self):
        """The end state: HDMI apps closed through SAM, Home in front, picture mode filmMaker, scratch removed, nothing left running."""
        notes = []
        try:
            for a in self.running():
                if HDMI_APP.match(a.get('id', '')) or a.get('id') == 'com.webos.app.livetv' and 'com.webos.app.livetv' in self.started_apps:
                    self.close_app(a['id'])
                    notes.append('closed ' + a['id'])
            self.sleep(2)
            fg = self.home()
            if self.picture_mode() != 'filmMaker':
                self.luna('luna://com.webos.settingsservice/setSystemSettings', '{"category":"picture","settings":{"pictureMode":"filmMaker"}}')
                notes.append('picture mode set back to filmMaker')
                self.sleep(1)
            if self.scan_ran:
                self.sh("pkill -f 'bluetooth2/le/startScan' 2>/dev/null; echo ok", 15)
            self.sh('rm -rf %s' % R, 20)
            self.end = self.state()
            self.end['notes'] = notes
            ok = {'home_in_front': fg == 'com.webos.app.home' and self.end['foreground'] == 'com.webos.app.home',
                  'picture_mode_filmMaker': self.end['picture_mode'] == 'filmMaker',
                  'inputs_running_empty': self.end['inputs'].get('running') == [],
                  'inputcommon_as_found': (self.end['inputcommon'].get('mounted'), self.end['inputcommon'].get('staged')) == (self.start['inputcommon'].get('mounted'), self.start['inputcommon'].get('staged')),
                  'guard_as_found': self.end['guard'] == self.start['guard']}
            # last of all: Active Standby, as round 8 did (power/powerOff with the remote's reason; SSH survives it)
            if not self.args.no_standby:
                self.luna('luna://com.webos.service.tvpower/power/powerOff', '{"reason":"remoteKey"}')
                for _ in range(23):
                    self.sleep(2)
                    if 'standby' in self.power().lower():
                        break
                self.end['final_power'] = self.power()
                ok['active_standby'] = 'standby' in self.end['final_power'].lower()
            else:
                self.end['final_power'] = self.power()
            self.log('final power state: %s' % self.end['final_power'])
            self.save_state('end-state.json', self.end)
            return ok
        except Exception as e:
            self.log('end-state restore failed: %s' % e)
            self.log(traceback.format_exc())
            return {'error': str(e)}

    def run(self):
        self.log('round 19 checks%s, evidence in %s' % (' (DRY RUN)' if self.dry else '', self.out))
        # an awake TV only: a set already in standby is left alone, and the slot refuses (the UI checks need the screen)
        self.start = self.state()
        self.save_state('start-state.json', self.start)
        self.log('start state: %s' % json.dumps({k: self.start[k] for k in ('power', 'foreground', 'picture_mode', 'guard', 'running_apps')}))
        refuse = None
        if self.start['power'] not in ('Active', 'Screen Saver') and not self.dry:
            refuse = 'power state is %r, not an awake TV; nothing was touched' % self.start['power']
        if self.start['lumatv_compositor_running'] and not self.args.allow_lumatv_compositor:
            refuse = "LumaTV's own compositor is running; these checks are for LG's compositor (--allow-lumatv-compositor to override)"
        if MEDIA_APPS.match(self.start['foreground']) and not self.args.allow_foreground:
            refuse = 'a media app is in front (%s): audio may be playing (--allow-foreground to override)' % self.start['foreground']
        if refuse:
            self.log('REFUSED: ' + refuse)
            return self.report([], {}, refuse)
        self.sh('mkdir -p %s' % R, 15)
        self.tv.put(os.path.join(PERF, 'kb.py'), KB)
        chosen = [c for c in CHECK_ORDER if (not self.args.only or c in self.args.only) and c not in self.args.skip]
        fns = {'speed': self.check_speed, 'settings': self.check_settings, 'plugins': self.check_plugins, 'layout': self.check_layout, 'updateblock': self.check_updateblock, 'ble': self.check_ble, 'banners': self.check_banners}
        aborted = None
        try:
            for name in chosen:
                if aborted:
                    self.results.append({'check': name, 'verdict': 'SKIPPED', 'detail': aborted, 'items': [], 'seconds': 0})
                    continue
                res = self.run_check(name, fns[name])
                if res.get('guard_moved'):
                    aborted = 'a guard value moved during %s: %s' % (name, res['guard_moved'])
                    res['verdict'] = 'FAIL' if res['verdict'] in ('PASS', 'EYES') else res['verdict']
                    res['detail'] += ' [GUARD MOVED: remaining checks skipped]'
                    self.log('ABORT: ' + aborted)
        finally:
            end = self.finish()
        return self.report(chosen, end, None)

    def report(self, chosen, end, refused):
        lines = ['round 19 checks, %s' % time.strftime('%Y-%m-%d %H:%M:%S')]
        if refused:
            lines.append('REFUSED: ' + refused)
            lines.append('FINAL POWER STATE: %s (nothing was touched)' % self.start.get('power', '?'))
        for r in self.results:
            lines.append('%-9s %-8s %5.0f s  %s' % (r['check'], r['verdict'], r.get('seconds', 0), r.get('detail', '')))
            for i in r.get('items', []):
                lines.append('    %-8s %s%s' % (i['verdict'], i['name'], (': ' + i['detail']) if i.get('detail') else ''))
        if end:
            lines.append('end state: ' + json.dumps(end))
            lines.append('FINAL POWER STATE: %s' % self.end.get('final_power', '?'))
        text = '\n'.join(lines)
        with open(os.path.join(self.out, 'summary.txt'), 'w') as f:
            f.write(text + '\n')
        with open(os.path.join(self.out, 'results.json'), 'w') as f:
            json.dump({'refused': refused, 'results': self.results, 'end_state_checks': end}, f, indent=1, default=str)
        print(text)
        failed = [r for r in self.results if r['verdict'] in ('FAIL', 'ERROR')]
        return 2 if refused else (1 if failed or (end and not end.get('error') and not all(end.values())) or end.get('error') else 0)


# ------------------------------------------------------------------------------------------------------- walk plans
# Keys only. OK appears where a step opens a page (Support, Delete My Personal Information, App Permissions, Home Assistant) and
# nowhere else: every other step is Up/Down/Left/Right/Back. Clamps make the paths independent of where the focus was left:
# Up on the tab bar does nothing, Right on the last tab stays, Down on the last row stays, and the rail's Up stops at the top.
# The rail has 9 sections then LG's own Settings row (OK there would launch LG's app): never more Downs than the section's index.

W1_STEPS = [
    ('rail: System', 'UPx12@0.1 DOWNx8@0.3 s0.5', 'system-rail', 'the rail on System'),
    ('System page', 'RIGHT s0.8', 'system-general', 'System, General tab'),
    ('System > About', 'UPx15@0.1 RIGHTx6@0.4 s0.8', 'system-about', 'About tab: the TV name, LumaTV, a Support row'),
    ('Support row focused', 'DOWNx10@0.2 s0.5', 'system-about-support-focus', 'the focus ring on the Support row (the last row)'),
    ('Support page', 'OK s1.5', 'support-open', 'Support opens on TV info'),
    ('Support > Privacy tab', 'UPx15@0.1 RIGHTx6@0.4 LEFTx2@0.5 s0.8', 'support-privacy', 'Privacy tab: Do Not Sell, Limit AD Tracking, AD ID, App Permissions, Delete My Personal Information'),
    ('Delete row focused', 'DOWNx15@0.15 s0.6', 'privacy-bottom-focus', 'the focus ring on Delete My Personal Information (the last row)'),
    ('Delete My Personal Information (opened only)', 'OK s1.8', 'delete-data', 'two danger rows, a reason line when LG\'s terms or the network are missing; nothing is pressed'),
    ('App Permissions row focused', 'BACK s1.2 DOWNx15@0.15 UP s0.6', 'privacy-permissions-focus', 'back on Privacy with the focus ring on App Permissions'),
    ('App Permissions: Camera', 'OK s1.8', 'permissions-camera', 'four tabs, the Camera list'),
    ('App Permissions: Microphone', 'UPx15@0.1 RIGHT s1.2', 'permissions-mic', 'Microphone tab'),
    ('App Permissions: Nearby Devices', 'RIGHT s1.2', 'permissions-nearby', 'Nearby Devices tab'),
    ('App Permissions: Access File', 'RIGHT s1.2', 'permissions-files', 'Access File tab (empty when the TV hides it)'),
]
W2_STEPS = [
    ('rail: Home & look', 'UPx12@0.1 DOWNx5@0.3 s0.5', 'look-rail', 'the rail on Home & look'),
    ('Home & look page', 'RIGHT s0.8', 'look-look', 'Look tab'),
    ('Home tab', 'UPx15@0.1 RIGHTx6@0.4 LEFTx2@0.5 s0.8', 'look-home-tab', 'Home tab: Home widgets, 12-hour clock, Home Assistant, Greeting ...'),
    ('Home Assistant row focused', 'DOWNx3@0.3 s0.5', 'look-home-ha-focus', 'the focus ring on Home Assistant (third row)'),
    ('Home Assistant page', 'OK s2.0', 'home-assistant', 'Connection tab: address, whether a token is kept (never the token)'),
    ('Home Assistant: second tab', 'UPx15@0.1 RIGHT s1.5', 'home-assistant-tab2', 'the plugin\'s second tab (Room)'),
    ('Home Assistant: third tab', 'RIGHT s1.5', 'home-assistant-tab3', 'the third tab (Doorbell)'),
]
W3_STEPS = [
    ('rail: Picture', 'UPx12@0.1 s0.6 RIGHT s1.2', 'picture-mode-top', 'Picture > Mode, top'),
    ('Picture > Mode: Just Scan', 'DOWNx30@0.12 s0.8', 'picture-mode-bottom', 'the rows near the bottom: Aspect ratio and Just Scan show the input\'s value, not "No input open"'),
]


# ------------------------------------------------------------------------------------------------------------ helpers

def load_gate():
    spec = importlib.util.spec_from_file_location('r19_gate', os.path.join(PERF, 'gate.py'))
    mod = importlib.util.module_from_spec(spec)
    sys.path.insert(0, PERF)
    try:
        spec.loader.exec_module(mod)
    finally:
        sys.path.remove(PERF)
    return mod


def gate_estimate_s(surfaces=('quick', 'picker'), cycles=None):
    """Seconds of key cycles the gate runs for these surfaces: 3 measured plus the soak cycles, each cycle as gate.py plans it."""
    g = load_gate()
    cycles = g.SOAK_DEFAULT_CYCLES if cycles is None else cycles
    return sum((g.MEASURE_CYCLES + cycles) * g.cycle_seconds(s) for s in g.find(list(surfaces), values={'INPUT': '241', 'HDMI': '2'}))


def dry_gate_flags(gate, cmd):
    r = subprocess.run([sys.executable, gate, '--help'], capture_output=True, text=True)
    flags = [a for a in cmd if a.startswith('--')]
    missing = [f for f in flags if f not in r.stdout]
    g = load_gate()
    names = [n for n in ('quick', 'picker') if n in [e['name'] for e in g.SURFACES]]
    return [{'name': 'gate.py --help knows %s' % ' '.join(flags), 'verdict': 'PASS' if not missing and r.returncode == 0 else 'FAIL', 'detail': 'missing %s' % missing if missing else ''},
            {'name': 'surfaces quick,picker exist', 'verdict': 'PASS' if names == ['quick', 'picker'] else 'FAIL', 'detail': ''}]


def walk_seconds():
    return sum(kb_seconds(t.split()) for steps in (W1_STEPS, W2_STEPS, W3_STEPS) for _, t, _, _ in steps)


def parse_args(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.split('\n\n')[0], formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument('--only', type=lambda s: s.split(','), default=None, help='checks to run: ' + ','.join(CHECK_ORDER))
    ap.add_argument('--skip', type=lambda s: s.split(','), default=[], help='checks to leave out')
    ap.add_argument('--out', help='evidence directory (default docs/research/round19/<date>)')
    ap.add_argument('--tv-py', default=os.environ.get('TV_PY', '/tmp/lt/tv.py'), help='the connection helper (default $TV_PY or /tmp/lt/tv.py)')
    ap.add_argument('--input-key', default='241', help="the remote's Input key evdev code for the picker (default 241)")
    ap.add_argument('--hdmi', default='2', help='the HDMI input of the banner check (default 2, the one rounds 17 and 18 used)')
    ap.add_argument('--livetv', action='store_true', help='also tune Live TV for its channel strip and INFO card (plays audio: mute the receiver first)')
    ap.add_argument('--no-info-key', action='store_true', help='do not send the INFO key over HDMI (it reaches the source over CEC)')
    ap.add_argument('--allow-foreground', action='store_true', help='run even when a media app is in front at the start')
    ap.add_argument('--allow-lumatv-compositor', action='store_true', help="run even when LumaTV's own compositor is live")
    ap.add_argument('--dry-run', action='store_true', help='no TV: print and syntax-check every command against canned replies')
    ap.add_argument('--no-standby', action='store_true', help='leave the TV on at the end (default: Active Standby, the owner\'s rule for every TV session)')
    a = ap.parse_args(argv)
    for c in (a.only or []) + a.skip:
        if c not in CHECK_ORDER:
            ap.error('unknown check %r' % c)
    if not a.out:
        a.out = os.path.join(tempfile.mkdtemp(prefix='r19-dry-') if a.dry_run else os.path.join(REPO, 'docs', 'research', 'round19'),
                             '' if a.dry_run else datetime.date.today().isoformat())
    return a


def main(argv=None):
    a = parse_args(argv)
    tv = DryTv() if a.dry_run else Tv(a.tv_py)
    slot = Slot(a, tv)
    rc = slot.run()
    if a.dry_run:
        print('\n[dry] %d commands, %d uploads; planned waits and key time %.0f s + gate %.0f s' % (len(tv.commands), len(tv.puts), slot.planned_s, gate_estimate_s()))
        print('[dry] walk key time alone: %.0f s' % walk_seconds())
        if tv.problems:
            print('[dry] PROBLEMS:')
            for p in tv.problems:
                print('  -', p)
            return 3
        print('[dry] every command passed sh -n, every luna payload parsed, every kb token was valid, python snippets compiled')
        return 0
    return rc


if __name__ == '__main__':
    sys.exit(main())
