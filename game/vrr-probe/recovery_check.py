#!/usr/bin/env python3
"""Offline comparison only. No network, register access, restart or restore operations."""
import argparse
import hashlib
import json
import re
import sys
from pathlib import Path
from uuid import UUID

IMAGE = '3e13cfe557ad8037972c77f03bd4d8d226e2f58eee0127fd38a98ae23fa52ca8'
KERNEL = '5.4.268-329.ptl4tv.2'
SECTIONS = ['boot', 'image', 'cmdline', 'overrides', 'status', 'foreground', 'loader', 'end']
CLIPS = ('50', '60', '4k120')


def require(ok, reason):
    if not ok:
        raise ValueError(reason)


def snapshot(path):
    raw = Path(path).read_bytes()
    parts = re.split(r'^@@VRR (\w+)\n', raw.decode(), flags=re.M)
    require(not parts[0].strip() and parts[1::2] == SECTIONS, 'receipt incomplete, duplicated or out of order')
    s = dict(zip(SECTIONS, (x.strip() for x in parts[2::2])))
    require(s['boot'] == s['end'] == str(UUID(s['boot'])) and UUID(s['boot']).int, 'boot identity missing or changed during capture')
    require(s['image'].split() == [IMAGE, '/boot/Image.lz4-' + KERNEL], 'boot image mismatch')
    opts = s['cmdline'].split()
    require([x for x in opts if x.startswith('pqe.year=')] == ['pqe.year=24'] and not any(x.startswith('dispopts=') for x in opts), 'loader command line differs')
    require(s['overrides'] == 'none', 'override check missing or failed')
    st = s['status'].splitlines()
    require([x for x in st if x.startswith('fw_opt:')] == ['fw_opt:none, usr_opt:none, usr_num:0, max_num:3, year:24'], 'unexpected loader options')
    require(len([x for x in st if x.startswith('fwload handler:')]) == 1 and any(re.fullmatch(r'fwload handler:(?:0+|\(null\))', x) for x in st), 'firmware handler unknown or active')
    slots = [x for x in st if re.match(r'^\[\d+\]', x)]
    require(len(slots) == 3 and all(re.fullmatch(rf'\[{i}\] id:\d+, flag:\d+, fw:\s*0kb, alloc:\s*0kb, p:(?:0+|\(null\))', x) for i,x in enumerate(slots)), 'unexpected slot, allocation or override path')
    fg = json.loads(s['foreground'])
    require(fg.get('returnValue') is True and fg.get('appId') == 'com.webos.app.home', 'stock Home not reported in foreground')
    return s, hashlib.sha256(raw).hexdigest()


def fresh(log):
    # Only one normal transaction, no intervening resume or later replacement.
    starts = re.findall(r'load\(from:([^)]*)\):start\b', log)
    if starts != ['normal'] or re.search(r'load\(from:(?!normal\))|rpm \(from:|ret skip|fw_opt skip|backup open\(|de firmware is not working', log):
        return False
    events = re.findall(r'load\(from:normal\):(?:start|done \(ret:-?\d+\))|ioload fw\d+ (?:cfg:\d+(?: -> header)?|done\(ret:-?\d+\))', log)
    return events == ['load(from:normal):start', *[f'ioload fw{i} cfg:0 -> header' for i in (2, 1, 0)], 'ioload fw0 done(ret:0)', 'load(from:normal):done (ret:0)']


def observe(path):
    s, digest = snapshot(path)
    def answer(prompt):
        print(prompt, file=sys.stderr)
        return input().strip()
    notes = {'receipt_sha256': digest, 'boot': s['boot'], 'clips': {}}
    notes['cold_start'] = answer('Personally witnessed a maintenance-safe shutdown and physical cold start for THIS boot? [yes/no/unknown]')
    require(notes['cold_start'] in ('yes', 'no', 'unknown'), 'missing cold-start observation')
    for mode in CLIPS:
        clip = answer(f'{mode}: exact stock clip/scene identifier used?')
        seen = answer('Picture and sound correct, normal exit back to visible Home and resources released? [yes/no/unknown]')
        require(clip and seen in ('yes', 'no', 'unknown'), 'missing clip or observation')
        notes['clips'][mode] = [clip, seen]
    print(json.dumps(notes, indent=2))


def word(s, k):
    v = s[k]
    require(type(v) is int and 0 <= v <= 0xffffffff, 'invalid unsigned word: ' + k)
    return v


def compare(paths, states=None):
    (before, bd), (after, ad) = map(snapshot, paths[:2])
    require(before['boot'] != after['boot'], 'same boot ID; no stock restart established')
    print('PASS stock configuration and Home foreground before/after; boot changed')
    seen = []
    for s, digest, path in zip((before, after), (bd, ad), paths[2:]):
        n = json.loads(Path(path).read_text())
        require(n['receipt_sha256'] == digest and n['boot'] == s['boot'], 'operator observations belong to a different capture')
        if digest == ad:
            require(n['cold_start'] == 'yes', 'physical cold recovery not witnessed')
        require(set(n['clips']) == set(CLIPS), 'incomplete playback receipt')
        for clip in n['clips'].values():
            require(isinstance(clip, list) and len(clip) == 2 and isinstance(clip[0], str) and clip[0].strip() and clip[1] == 'yes', 'stock playback or visible handback not confirmed')
        seen.append(n['clips'])
    require(seen[0] == seen[1], 'before/after scenarios differ')
    print('PASS functional handback (operator observations, not scan timing)')
    require(not re.search(r'de firmware is not working|done\s*\(ret:(?!0\))-?\d+\)', after['loader']), 'loader reports a failure')
    print(('PASS' if fresh(after['loader']) else 'NOT ESTABLISHED') + ' fresh normal embedded-header selection for 2,1,0')
    if states:
        previous, current = [json.loads(Path(p).read_text()) for p in states]
        for row, snap, sha in ((previous, before, bd), (current, after, ad)):
            require(row['boot'] == snap['boot'] and row['receipt_sha256'] == sha, 'state belongs to another snapshot')
        require(previous['owner'] == current['owner'] and isinstance(previous['owner'], str) and previous['owner'].strip(), 'different logical owner')
        for k in ('cache', 'pulse', 'pixel', 'timer'):
            require(word(previous, k) == word(current, k), k + ' differs after recovery')
        print('PASS equality of supplied cache and hardware words separately (reader provenance not established by this tool)')
    else:
        print('NOT OBSERVED selector cache and hardware; Home is not their readback')
    print('NOT OBSERVED active MCU byte identity or independent timeout. STOP before timing changes.')
    return 2


def split(phase, original, observed=None):
    """Check a single observed phase, never authorize a subsequent transition."""
    b, a = original, observed
    require(str(UUID(b['boot'])) == b['boot'] and b['boot'] != str(UUID(int=0)), 'invalid baseline boot')
    require(isinstance(b['owner'], str) and b['owner'].strip(), 'missing owner')
    require(type(b['time_ns']) is int and b['time_ns'] >= 0 and b['phase'] == 'original', 'invalid baseline snapshot')
    require(word(b, 'cache') == 0x20 and word(b, 'pulse') & 0x73 == 1, 'baseline not in the qualified decoded configuration')
    require(word(b, 'pixel') & 0x3f == 1, 'baseline pixel selector not decoded MVI')
    n, d = word(b, 'timer_num'), word(b, 'timer_den')
    require(n and d and word(b, 'timer') == (400_000_000 // n) * d, 'original owner arguments do not reproduce the exact timer')
    # Refuse zero or wrapped baselines; only the bounded two-rate experiment is modeled.
    require(b['timer'] > 0, 'zero period')
    connected = phase.startswith('selected')
    timer = b['timer'] if phase == 'restored' else (5_000_000 if phase.endswith('80') else 6_250_000)
    expected = dict(cache=0x50 if connected else 0x20, pulse=((b['pulse'] & ~0x73) | 3) if connected else b['pulse'], pixel=word(b, 'pixel'), timer=timer)
    print('EXPECTED', ' '.join(f'{k}=0x{v:08x}' for k,v in expected.items()))
    if a is None:
        print('NO SAMPLE: predictions only; STOP')
        return 2
    require(a['boot'] == b['boot'] and a['owner'] == b['owner'], 'wrong boot or owner')
    require(type(a['time_ns']) is int and a['time_ns'] > b['time_ns'] and a['phase'] == phase, 'stale or wrong phase')
    for k,v in expected.items():
        require(word(a, k) == v, f'{k}: expected 0x{v:08x}, observed 0x{a[k]:08x}')
    print('PASS supplied words match this phase only; not proof of timing, provenance or live rollback')
    return 0


def main():
    p = argparse.ArgumentParser(description=__doc__); subs = p.add_subparsers(dest='op', required=True)
    for cmd in ('baseline', 'observe'):
        subs.add_parser(cmd).add_argument('receipt')
    q = subs.add_parser('compare')
    q.add_argument('paths', nargs=4, metavar='FILE')
    q.add_argument('--states', nargs=2, metavar=('BEFORE', 'AFTER'))
    q = subs.add_parser('split'); q.add_argument('phase', choices=[p + hz for p in ('disconnected', 'selected') for hz in ('64','80')] + ['restored']); q.add_argument('original'); q.add_argument('observed', nargs='?')
    args = p.parse_args()
    try:
        if args.op == 'compare': return compare(args.paths, args.states)
        if args.op == 'split': return split(args.phase, json.loads(Path(args.original).read_text()), json.loads(Path(args.observed).read_text()) if args.observed else None)
        if args.op == 'observe': observe(args.receipt)
        else:
            s, digest = snapshot(args.receipt); print(f'PASS stock inputs and Home foreground; boot={s["boot"]} receipt_sha256={digest}')
        return 0
    except (ValueError, OSError, KeyError, TypeError, AttributeError, EOFError) as err:
        print('FAIL / STOP:', err, file=sys.stderr); return 1


if __name__ == '__main__':
    sys.exit(main())
