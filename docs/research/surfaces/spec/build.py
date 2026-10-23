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
