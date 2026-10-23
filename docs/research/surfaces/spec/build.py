import sys, re, os, collections, json
sys.path.insert(0, os.path.dirname(__file__))
import core
from core import ROWS, G, GROUPS, cell_ids
import g1_3, g5_9, g10_15, g15  # noqa
D = os.path.dirname(__file__)

def esc(s):
    return s.replace('|', '\\|')

def render_row(r):
    shows = f": {r['shows']}" if r['shows'] else ''
    name = f"**{r['name']}**{shows}"
    ans = f"**{r['ans']}**: {r['why']}" if r['why'] else f"**{r['ans']}**"
    cells = [name, cell_ids(r), r['seen'], r['plan'], ans, r['hook'], r['ph']]
    for c in cells:
        assert '|' not in c, (r['name'], c)
    return '| ' + ' | '.join(cells) + ' |'

def tables():
    out = []
    for gi, gname in G.items():
        rows = [r for r in ROWS if r['g'] == gi]
        out.append(f'## {gi}. {gname}\n')
        out.append('| Surface | LG id · kind · size | Seen | Plan | Answer | Hook | Ph |')
        out.append('|---|---|---|---|---|---|---|')
        for r in rows:
            out.append(render_row(r))
        out.append('')
    return '\n'.join(out)

def stats():
    ans = collections.Counter()
    dormant = 0
    seen = collections.Counter()
    for r in ROWS:
        ans[r['ans'].split(' ')[0]] += 1
        if '(dormant)' in r['ans']:
            dormant += 1
        seen[r['seen']] += 1
    return ans, dormant, seen

if __name__ == '__main__':
    ans, dormant, seen = stats()
    print(len(ROWS), 'rows', dict(ans), 'dormant', dormant, dict(seen))
    by = collections.Counter(r['g'] for r in ROWS)
    print({G[k]: v for k, v in sorted(by.items())})
    mx = max(len(render_row(r)) for r in ROWS)
    print('longest row', mx)

# ------------------------------------------------------------------ assembly
import datetime

def has_mock(plan):
    return bool(re.search(r'mocks? \d', plan)) and 'no mock' not in plan

NOT_MOCKED = [
    ('HDMI input banner', 'the pill on every input switch, plus OK and INFO banners; mock 20 is the channel banner'),
    ('Power-off logo', 'every power-off, about 3 s; mock 23 is the boot screen'),
    ('Always Ready and Gallery+', 'the standby screen, if Always Ready is on (unverified)'),
    ('Pointer right-click menu', 'Back and Home beside the pointer; mock 16 is the Home tile menu'),
    ('Voice listening bar', 'mic key held: "Speak now.", transcript, reply'),
    ('App launch prompts', 'update-required and not-available alerts when an app starts'),
    ('Quick panel popups and edit mode', 'slider, value picker, Move or Delete, add-tile list'),
    ('Settings sections without a mock', 'Inputs & devices, Network, Ambient light, Home & look, System'),
    ('Startup cover', 'first-app cover with "Loading..." at boot and wake'),
    ('Quick Access (number keys)', 'toast on launch and the "add to slot N?" alert'),
    ('Toast: advanced (icon and link)', 'icon toast whose click runs an action'),
]
RARE_NOT_MOCKED = [
    ('Second-screen pairing prompt', 'security prompt; Home Assistant pairs through it'),
    ('On-TV button panel', 'recovery path without a remote'),
    ('"More Actions" panel (on-screen remote)', 'Quit App, number pad, colour keys'),
    ('Game Dashboard', 'HDMI game mode replaces the quick panel'),
]

def totals():
    n = len(ROWS)
    ans_order = ['Replace', 'Restyle', 'Off', 'Keep']
    seen_order = ['daily', 'sometimes', 'rare', 'never']
    cross = {a: collections.Counter() for a in ans_order}
    dorm = collections.Counter()
    for r in ROWS:
        a = r['ans'].split(' ')[0]
        cross[a][r['seen']] += 1
        if '(dormant)' in r['ans']:
            dorm[r['seen']] += 1
    plan = collections.Counter(r['plan'].split(' ')[0] for r in ROWS)
    lines = [f'## Totals\n',
             f'{n} surfaces in 15 groups, one row each; a row may list several ids (160 app ids, 113 compositor view files and the system-UI modules are all covered). Rows by answer and how often they are seen:\n',
             '| Answer | daily | sometimes | rare | never | Total |', '|---|---|---|---|---|---|']
    for a in ans_order:
        t = sum(cross[a].values())
        extra = f' ({sum(1 for r in ROWS if r["ans"].startswith("Keep (dormant)"))} dormant)' if a == 'Keep' else ''
        lines.append(f'| {a} | ' + ' | '.join(str(cross[a][s]) for s in seen_order) + f' | {t}{extra} |')
    tot_seen = collections.Counter(r['seen'] for r in ROWS)
    lines.append('| **Total** | ' + ' | '.join(f'**{tot_seen[s]}**' for s in seen_order) + f' | **{n}** |')
    lines.append('')
    lines.append(f'Plan coverage today: Yes {plan["Yes"]}, Part {plan["Part"]}, Out {plan["Out"]}, No {plan["No"]}. '
                 f'Of the {tot_seen["daily"] + tot_seen["sometimes"]} daily and sometimes surfaces, '
                 f'{sum(1 for r in ROWS if r["seen"] in ("daily", "sometimes") and has_mock(r["plan"]))} have a mock.')
    lines.append('')
    return '\n'.join(lines)

def not_mocked():
    byname = {r['name']: r for r in ROWS}
    for name, _ in NOT_MOCKED + RARE_NOT_MOCKED:
        assert name in byname, name
    # every daily or sometimes row without a mock must be in the list, or be a host, third-party app or built
    listed = {n for n, _ in NOT_MOCKED}
    skip = {'Home', 'Fullscreen app host', 'HDMI input apps 1-4', 'Netflix', 'YouTube', 'Prime Video', 'PlxNative',
            'LumaTV app and frame', 'Magic Remote pointer image', 'Video planes'}
    for r in ROWS:
        if r['seen'] in ('daily', 'sometimes') and not has_mock(r['plan']) and r['name'] not in listed and r['name'] not in skip:
            raise SystemExit('unlisted unmocked row: ' + r['name'])
    for r in ROWS:
        if r['name'] in listed:
            assert r['seen'] in ('daily', 'sometimes'), r['name']
    lines = ['## Not yet mocked\n',
             f'Daily or sometimes surfaces with no screen in `mocks/`, most seen first ({len(NOT_MOCKED)}; Home is built and is the visual reference, the Apps tile has mock 12):\n']
    for i, (n, note) in enumerate(NOT_MOCKED, 1):
        r = byname[n]
        lines.append(f'{i}. **{n}** ({r["seen"]}): {note}.')
    lines.append('')
    lines.append('Rare, but next in line for what each one protects:\n')
    for i, (n, note) in enumerate(RARE_NOT_MOCKED, len(NOT_MOCKED) + 1):
        lines.append(f'{i}. **{n}**: {note}.')
    lines.append('')
    return '\n'.join(lines)

def assemble(resume):
    head = open(os.path.join(D, 'head.md')).read().replace('{{RESUME}}', resume.strip())
    parts = [head.rstrip() + '\n', tables(), open(os.path.join(D, 'keymap.md')).read(), open(os.path.join(D, 'noui.md')).read(),
             totals(), not_mocked(), open(os.path.join(D, 'facts.md')).read(), open(os.path.join(D, 'missing.md')).read(), open(os.path.join(D, 'needs.md')).read()]
    return '\n'.join(p.rstrip() + '\n' for p in parts)

RESUME = open(os.path.join(D, 'resume.md')).read() if os.path.exists(os.path.join(D, 'resume.md')) else 'pending'

if __name__ == '__main__' and len(sys.argv) > 1 and sys.argv[1] == 'write':
    out = assemble(RESUME)
    open('/data/home/lumaglass/docs/SURFACES.md', 'w').write(out)
    print('wrote', len(out), 'bytes,', out.count('\n'), 'lines')
