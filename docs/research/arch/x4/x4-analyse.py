#!/usr/bin/env python3
"""X4: census of luna traffic around LG's compositor from an ls-monitor -j capture.
Usage: x4-analyse.py all.jsonl [label]
Counts one copy per message (transport TX), resolves unique names where the capture shows both names, and prints
markdown tables: calls into the compositor's bus names, calls the compositor makes, the tvpower handshake,
video placement, and overall bus volume."""
import json, sys, collections

SM_PREFIX = 'com.webos.surfacemanager'
SM_EXTRA = {'com.webos.service.sysuicompmgr', 'com.webos.service.systemUIManager', 'com.webos.service.recent',
            'com.webos.coverUI', 'com.webos.keyfilter.magicnumber'}

def is_sm(name):
    return bool(name) and (name.startswith(SM_PREFIX) or name in SM_EXTRA)

def short(payload, n=90):
    s = json.dumps(payload, separators=(',', ':'), ensure_ascii=False) if not isinstance(payload, str) else payload
    return s if len(s) <= n else s[:n] + '…'

def main():
    path = sys.argv[1]; label = sys.argv[2] if len(sys.argv) > 2 else path
    msgs = []; bad = 0
    with open(path, errors='replace') as f:
        for line in f:
            line = line.strip()
            if not line.startswith('{'): continue
            try: msgs.append(json.loads(line))
            except json.JSONDecodeError: bad += 1
    tx = [m for m in msgs if m.get('transport') == 'TX']
    if not tx: tx = msgs
    t0 = min(m.get('timeStart', 0) for m in tx) if tx else 0
    t1 = max(m.get('timeStart', 0) for m in tx) if tx else 0
    dur = max(t1 - t0, 1e-9)
    # unique name -> service name
    uname = {}
    for m in msgs:
        for k in ('sender', 'destination'):
            u = m.get(k + 'UniqueName'); n = m.get(k)
            if u and n and not n.startswith(('com.webos.lunasend',)) and n != u: uname.setdefault(u, n)
    def name(m, k):
        n = m.get(k) or ''
        return uname.get(n, n)
    types = collections.Counter(m.get('type') for m in tx)
    print('## X4 census: %s' % label)
    print('\n%d messages (%d TX copies counted, %d unparsable lines), %.1f s, %.0f messages/s; types: %s\n' % (len(msgs), len(tx), bad, dur, len(tx) / dur, dict(types)))

    calls = [m for m in tx if m.get('type') == 'call']
    into = collections.Counter(); into_sample = {}
    outof = collections.Counter(); outof_sample = {}
    for m in calls:
        s, d = name(m, 'sender'), name(m, 'destination')
        meth = (m.get('methodCategory') or '/').rstrip('/') + '/' + (m.get('method') or '?')
        if is_sm(d):
            k = (s, d, meth); into[k] += 1; into_sample.setdefault(k, short(m.get('payload', {})))
        if is_sm(s):
            k = (d, meth); outof[k] += 1; outof_sample.setdefault(k, short(m.get('payload', {})))
    print('### Calls into the compositor\'s bus names (%d calls, %d distinct caller/method pairs)\n' % (sum(into.values()), len(into)))
    print('| Caller | Bus name | Method | Calls | First payload |\n|---|---|---|---|---|')
    for (s, d, meth), c in sorted(into.items(), key=lambda kv: (-kv[1], kv[0])):
        print('| `%s` | `%s` | `%s` | %d | `%s` |' % (s, d, meth, c, into_sample[(s, d, meth)].replace('|', '\\|')))
    print('\n### Calls the compositor makes (%d calls, %d distinct)\n' % (sum(outof.values()), len(outof)))
    print('| Destination | Method | Calls | First payload |\n|---|---|---|---|')
    for (d, meth), c in sorted(outof.items(), key=lambda kv: (-kv[1], kv[0])):
        print('| `%s` | `%s` | %d | `%s` |' % (d, meth, c, outof_sample[(d, meth)].replace('|', '\\|')))

    # tvpower handshake and video placement, in order
    print('\n### tvpower / videooutput traffic touching the compositor\n')
    print('| t (s) | Type | From | To | Method | Payload |\n|---|---|---|---|---|---|')
    shown = 0
    for m in sorted(tx, key=lambda m: m.get('timeStart', 0)):
        s, d = name(m, 'sender'), name(m, 'destination')
        meth = (m.get('methodCategory') or '/').rstrip('/') + '/' + (m.get('method') or '?')
        if (('tvpower' in s or 'tvpower' in d) and (is_sm(s) or is_sm(d))) or ('videooutput' in d and 'setDisplayWindow' in meth) or ('videooutput' in d and is_sm(s)):
            print('| %.2f | %s | `%s` | `%s` | `%s` | `%s` |' % (m.get('timeStart', 0) - t0, m.get('type'), s, d, meth, short(m.get('payload', {}), 120).replace('|', '\\|')))
            shown += 1
            if shown > 80: print('| … | | | | | |'); break

    # subscriptions: calls with subscribe:true into the compositor, and returns streamed out
    subs = collections.Counter()
    for m in calls:
        p = m.get('payload')
        if isinstance(p, dict) and p.get('subscribe') and is_sm(name(m, 'destination')):
            subs[(name(m, 'sender'), name(m, 'destination'), m.get('method'))] += 1
    if subs:
        print('\n### Subscriptions opened on the compositor during the window\n')
        for (s, d, meth), c in subs.most_common(): print('- `%s` -> `%s/%s` x%d' % (s, d, meth, c))

    # bus volume context
    pairs = collections.Counter((name(m, 'sender'), name(m, 'destination')) for m in tx)
    print('\n### Busiest sender -> destination pairs on the whole bus (context)\n')
    for (s, d), c in pairs.most_common(15): print('- `%s` -> `%s`: %d (%.1f/s)' % (s, d, c, c / dur))

if __name__ == '__main__':
    main()
