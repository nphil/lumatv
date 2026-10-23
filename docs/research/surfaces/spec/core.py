"""Row model for SURFACES.md (scaffold; removed when the catalogue is final)."""
import json, glob, os, re
SNAP = '/data/home/lgtv/uiinv'
RAW = '/data/home/lumaglass/docs/research/surfaces'
GROUPS = ["Home & launching", "Inputs & devices", "Volume & sound", "Notifications & prompts",
          "Settings & wizards", "Live TV", "Search, voice & keyboard", "Power, boot & screensaver",
          "Game", "Casting & sharing", "Ads, promos & tracking", "Accounts & first run",
          "Service/factory", "Accessibility", "Third-party/protected"]
G = {i + 1: g for i, g in enumerate(GROUPS)}

A = {}
for f in sorted(glob.glob(f'{RAW}/*.json')):
    try:
        d = json.load(open(f))
    except Exception:
        continue
    for r in d.get('rows', []):
        A.setdefault(r['id'], r)
LA = {a['id']: a for a in json.load(open(f'{SNAP}/listApps.json'))['apps']}

def kb(i):
    p = f'{SNAP}/apps/{i}/size_kb'
    if os.path.exists(p):
        return int(open(p).read().strip())
    r = A.get(i)
    if r:
        m = re.match(r'(\d+) KB', r.get('size', ''))
        if m:
            return int(m.group(1))
    a = LA.get(i)
    if a and a.get('appsize'):
        return int(a['appsize']) // 1024
    return None

def fs(k):
    if k is None:
        return 'n/a'
    return f'{k / 1024:.1f} MB' if k >= 1000 else f'{k} KB'

# sizes measured on the set (du -sk) or read from the snapshot where the dir is missing
SIZE = {'com.webos.app.adapp': 4615, 'com.webos.app.browser': 12160, 'com.webos.chromecast': 16,
        'com.webos.app.discovery': 631, 'com.webos.app.membership': 5119,
        'com.beb.plxnative': 36296, 'com.amazoff.patcher': 8076, 'com.limelight.webos': 4284,
        'io.unom.punktfunk.client-webos': 25544, 'org.nphil.lumaglass': 5040,
        'org.nphil.lumaglass.frame': 340, 'org.webosbrew.appupdateblocker': 128,
        'org.webosbrew.bridge-64to32': 37524, 'org.webosbrew.hbchannel': 2536,
        'org.webosbrew.hyperhdr.loader': 4352, 'org.webosbrew.inputhook': 52,
        'org.webosbrew.safeupdate': 664, 'com.webos.chromecast-settings': 16,
        'com.webos.app.lgchannels': 383, 'com.webos.app.voiceagent': 7667}
KIND = {'com.webos.app.voiceagent': 'system-qml', 'com.webos.app.browser': 'native', 'youtube.leanback.v4': 'native',
        'com.webos.app.hdmioptical': 'qml', 'com.lge.app.viewster': 'web'}

def kind(i):
    if i in KIND:
        return KIND[i]
    a = LA.get(i)
    r = A.get(i)
    t = (a or {}).get('type') or (r or {}).get('tech') or '?'
    return {'native_builtin': 'native'}.get(t, t)

def I(i):
    if i.startswith('`') or ' ' in i:
        return i
    k = SIZE[i] if i in SIZE else kb(i)
    return f'`{i}` · {kind(i)} · {fs(k)}'

def S(*names):
    return '`' + '`, `'.join(names) + '` · service'

def F(path, kind_, size):
    return f'`{path}` · {kind_} · {size}'

ROWS = []
def row(g, name, shows, ids, seen, plan, ans, why, hook, ph):
    if isinstance(ids, str):
        ids = [ids]
    ROWS.append(dict(g=g, name=name, shows=shows, ids=ids, seen=seen, plan=plan, ans=ans, why=why,
                     hook=hook, ph=str(ph)))

def cell_ids(r):
    out = []
    for x in r['ids']:
        out.append(I(x))
    return '; '.join(out)
