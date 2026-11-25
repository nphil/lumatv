#!/usr/bin/env python3
"""Where the shell's and the engine's heap goes, on the host: runs `lumatv-ui --host` built with `--features heap-audit` through the `heap` walk of
`tests/host_budget.py` (Home at rest with its widgets, Settings opened and closed, the apps grid opened and closed, twice, then ten seconds of
rest) and names the owner of every live block from the stack it was made on. Host numbers: the set's allocator, sizes and texture memory differ.

    cargo build --release -p lumatv-ui --features heap-audit --bin lumatv-ui     # in rustui/
    python3 tests/heap_audit.py --bin rustui/target/release/lumatv-ui [--keep DIR] [--top 14] [--stacks LABEL]

For each rest it prints the live bytes by owner (the nearest frame in this repository's own code, by file, and the categories `CATEGORIES` names), glibc's
arena and the free part of it, and the process's resident anonymous memory. `--stacks LABEL` prints the largest stacks of one rest in full."""
import argparse
import collections
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import host_budget as hb  # noqa: E402

#: (category, regex over the whole stack's files and functions): the first that matches names the live block's category
CATEGORIES = [
    ('kept source pixels', r'kept\.rs'),
    ('glyph/text atlas', r'atlas|glyph|gridfit|text_pictures|skrifa|fontdue'),
    ('wallpaper', r'wallpaper|backdrop'),
    ('scene layers and shm files', r'lumashow-ui/src/(scene|writer|group|handle)\.rs|shm\.rs|ImageFile|image_file'),
    ('weather and maps', r'weather|wx'),
    ('cameras', r'cameras?'),
    ('settings pages', r'settings'),
    ('apps grid', r'home/apps\.rs|dock\.rs'),
    ('json and strings', r'serde_json|Value'),
    ('engine (render crate)', r'rustui/render/'),
    ('shell (ui crate)', r'rustui/ui/'),
]
#: the frames of each stack that are named
FRAMES = 18
PROJECT = ('/rustui/', '/compositor/', '/placement/', '/saver/', '/lumaguide/', '/LumaShow/')


def maps_of(path):
    out = []
    for line in open(path):
        m = re.match(r'([0-9a-f]+)-([0-9a-f]+) \S+ ([0-9a-f]+) \S+ \S+\s*(.*)$', line)
        if m:
            out.append((int(m.group(1), 16), int(m.group(2), 16), int(m.group(3), 16), m.group(4)))
    return out


def load_base(maps, binary):
    real = os.path.realpath(binary)
    for start, _, off, name in maps:
        if name and os.path.realpath(name) == real and off == 0:
            return start
    raise SystemExit('the binary is not in the maps')


def symbolize(binary, base, addrs):
    """{addr: [(function, file:line), ...]} innermost first, by addr2line (the frames are return addresses: one byte back is the call)."""
    uniq = sorted(addrs)
    if not uniq:
        return {}
    args = ['addr2line', '-e', binary, '-f', '-C', '-i', '-a'] + ['%x' % (a - base - 1) for a in uniq if a >= base]
    out = subprocess.run(args, capture_output=True, text=True, check=True).stdout.splitlines()
    table, cur, i = {}, None, 0
    while i < len(out):
        line = out[i]
        if line.startswith('0x'):
            cur = int(line, 16) + base + 1
            table[cur] = []
            i += 1
            continue
        table[cur].append((line, out[i + 1] if i + 1 < len(out) else '?'))
        i += 2
    return table


def parse(path):
    meta, sites = {}, []
    for line in open(path):
        if line.startswith('site '):
            f = line.split()
            sites.append((int(f[1]), int(f[2]), int(f[3]), int(f[4]), int(f[5]), [int(x, 16) for x in f[6:]]))
        elif line.startswith('# '):
            f = line[2:].split()
            meta[f[0].rstrip(':')] = f[1:]
    return meta, sites


def owner_of(frames, table):
    """The nearest frame in this repository's own code, as `file:function`, and the whole stack's text."""
    near, text = None, []
    for a in frames:
        for func, loc in table.get(a, []):
            text.append(func + ' ' + loc)
            if near is None and any(p in loc for p in PROJECT):
                near = '%s %s' % (re.sub(r'^.*?/(rustui|compositor|placement|saver|lumaguide|LumaShow)/', r'\1/', loc.split(':')[0]), func.split('<')[0].split('::')[-1])
    return near or '(std/other)', ' | '.join(text)


def mb(n):
    return '%7.2f' % (n / 1048576)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--bin', default=os.environ.get('LUMATV_UI_BIN') or os.path.join(hb.ROOT, 'rustui', 'target', 'release', 'lumatv-ui'))
    ap.add_argument('--keep', help='keep the raw dumps in this folder')
    ap.add_argument('--top', type=int, default=14)
    ap.add_argument('--refresh', type=int, default=120)
    ap.add_argument('--stacks', help='print the largest stacks of this rest (its label as in the table)')
    ap.add_argument('--owner', help='with --stacks: only the stacks whose nearest repository frame contains this text')
    ap.add_argument('--by-peak', action='store_true', help='with --stacks: largest by the most each held at once, not by what is live')
    ap.add_argument('--diff', nargs=2, metavar=('FROM', 'TO'), help='the owners whose live bytes differ between two rests, and the stacks of the largest growth')
    ap.add_argument('--cycles', type=int, default=2, help='how many times Settings and the apps grid are opened and closed')
    ap.add_argument('--from-dumps', help='analyse the dumps in this folder without running')
    o = ap.parse_args()
    binary = os.path.abspath(o.bin)
    dumps = o.from_dumps or o.keep or tempfile.mkdtemp(prefix='heapaudit-')
    if not o.from_dumps:
        os.makedirs(dumps, exist_ok=True)
        os.environ['LUMATV_HEAP_AUDIT'] = dumps
        os.environ['LUMATV_HEAP_CYCLES'] = str(o.cycles)
        hb.run_walk('heap', refresh=o.refresh, binary=binary)
    try:
        labels = sorted(f[:-5] for f in os.listdir(dumps) if f.endswith('.heap'))
        if not labels:
            raise SystemExit('no dump: is the binary built with --features heap-audit?')
        data = {l: parse(os.path.join(dumps, l + '.heap')) for l in labels}
        base = load_base(maps_of(os.path.join(dumps, labels[0] + '.maps')), binary)
        # the first frames of a stack name its owner; addr2line on every address of every stack takes minutes
        for l in labels:
            data[l] = (data[l][0], [s[:5] + (s[5][:FRAMES],) for s in data[l][1]])
        addrs = {a for _, sites in data.values() for s in sites for a in s[5]}
        cached = os.path.join(dumps, 'symbols.json')
        known = {int(k): v for k, v in json.load(open(cached)).items()} if os.path.exists(cached) else {}
        fresh = symbolize(binary, base, addrs - known.keys())
        known.update(fresh)
        json.dump(known, open(cached, 'w'))
        table = known
        cache = {}
        categories = {}
        rows = {}
        for l in labels:
            meta, sites = data[l]
            by_owner, by_cat = collections.Counter(), collections.Counter()
            for live, count, _, _, _, frames in sites:
                key = tuple(frames)
                if key not in cache:
                    cache[key] = owner_of(frames, table)
                    categories[key] = next((c for c, rx in CATEGORIES if re.search(rx, cache[key][1], re.I)), 'other')
                by_owner[cache[key][0]] += live
                by_cat[categories[key]] += live
            rows[l] = (meta, by_owner, by_cat, sum(s[0] for s in sites), sites)
        print('HOST heap audit (lumatv-ui --host, shell and engine in one process; live bytes from the counting allocator, MiB)')
        print('%-26s %9s %9s %9s %9s %9s %9s %9s' % ('rest', 'live', 'arena', 'free', 'RssAnon', 'gpu(cen)', 'shm len', 'shm files'))
        for l in labels:
            meta, _, _, live, _ = rows[l]
            ml = meta.get('mallinfo2', [])
            g = lambda k: int(ml[ml.index(k) + 1]) if k in ml else 0  # noqa: E731
            shm = meta.get('shm_files', ['0', '0', '0'])
            print('%-26s %s %s %s %s %s %s %9s' % (l, mb(live), mb(g('arena')), mb(g('free')), mb(int(meta.get('RssAnon', ['0'])[0]) * 1024), mb(int(meta.get('gpu_bytes', ['0'])[0])), mb(int(shm[1])), shm[0]))
        census = rows[labels[0]][0].get('gpu_census')
        if census:
            print('\n%s: %s' % (labels[0], ' '.join(census)))
        print('\nby category (first matching rule over the whole stack):')
        cats = [c for c, _ in CATEGORIES] + ['other']
        print('%-26s' % 'category' + ''.join('%8s' % l[:2] for l in labels))
        for c in cats:
            vals = [rows[l][2].get(c, 0) for l in labels]
            if any(vals):
                print('%-26s' % c + ''.join('%8.2f' % (v / 1048576) for v in vals))
        for l in labels:
            if l.split('-', 1)[1] in ('home-idle', 'home-after-apps', 'home-idle-10s', 'home-after-settings'):
                print('\n%s: top owners (nearest repo frame, MiB)' % l)
                for near, n in rows[l][1].most_common(o.top):
                    print('  %s  %s' % (mb(n), near))
        peaks = collections.Counter()
        for live, count, total, tcount, peak, frames in rows[labels[-1]][4]:
            peaks[cache[tuple(frames)][0]] += peak
        print('\nthe most each owner held at once over the run, MiB (a no-trim heap keeps its high-water mark):')
        for near, n in peaks.most_common(o.top):
            print('  %s  %s' % (mb(n), near))
        if o.diff:
            a, b = ([l for l in labels if l.startswith(x) or l.endswith(x)][0] for x in o.diff)
            owners = rows[b][1].copy()
            owners.subtract(rows[a][1])
            print('\nlive bytes %s -> %s by owner, MiB:' % (a, b))
            for near, n in sorted(owners.items(), key=lambda kv: -abs(kv[1]))[:o.top]:
                if n:
                    print('  %+6.2f  %s' % (n / 1048576, near))
            was = {}
            for live, count, _, _, _, frames in rows[a][4]:
                was[tuple(frames)] = was.get(tuple(frames), 0) + live
            grown = collections.Counter()
            for live, count, _, _, _, frames in rows[b][4]:
                grown[tuple(frames)] += live
            grown.subtract(was)
            for key, n in grown.most_common(o.top):
                print('\n  %+.3f MiB' % (n / 1048576))
                for addr in key[:12]:
                    for func, loc in table.get(addr, [])[:2]:
                        print('    %s  %s' % (func[:70], loc.replace(os.getcwd(), '')[-60:]))
        if o.stacks:
            sel = [l for l in labels if l.endswith(o.stacks) or l == o.stacks]
            for l in sel[:1]:
                print('\n%s: largest stacks' % l)
                chosen = [x for x in sorted(rows[l][4], key=(lambda x: x[4]) if o.by_peak else (lambda x: x[0]), reverse=True) if not o.owner or o.owner in cache[tuple(x[5])][0]]
                for live, count, total, tcount, peak, frames in chosen[:o.top]:
                    print('  %s MiB in %d blocks (peak %s MiB, %d allocated over the run):' % (mb(live), count, mb(peak), tcount))
                    for a in frames[:12]:
                        for func, loc in table.get(a, [])[:2]:
                            print('      %s  %s' % (func[:90], loc.replace(hb.ROOT, '')[:90]))
    finally:
        if not o.from_dumps and not o.keep:
            shutil.rmtree(dumps, ignore_errors=True)


if __name__ == '__main__':
    main()
