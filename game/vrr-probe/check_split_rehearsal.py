#!/usr/bin/env python3
"""Host qualification against an operator-supplied, hash-checked retail artifact; no test on the TV."""
import argparse
import itertools
from pathlib import Path

from paired_evidence import analyse
from split_rehearse import ORIGINAL, receipt, rehearse
from split_stock import CACHE, PULSE, TIMER, StockOwners, decode, load_image


def qualify(image, ops):
    jobs = [(hz,f,c) for hz in (64,80) for f in ('arm-cut','disconnect-cut') for c in range(1,6)]
    jobs += [(hz,f,1) for hz in (64,80) for f in ('none','deadline','mismatch','hang')]
    for hz,f,c in jobs:
        # The logical clock isolates the fault cases from host scheduling. No synthetic hardware times are emitted.
        run = rehearse(image,ops,hz,f,cut=c,now=itertools.count().__next__)
        calls,reads = run['owner_calls'], run['readbacks']
        names = [r['step'] for r in calls]
        assert names.count('select') == 1, names
        if f in ('hang','mismatch'):
            assert run['outcome'] == 'unrecovered' and names[-1] == 'select', run
            assert 'restored' not in [r['step'] for r in reads]
        else:
            assert run['outcome'] == ('rehearsed' if f == 'none' else 'aborted-restored'), run
            assert reads[-1]['step'] == 'restored'
            assert all(ORIGINAL[k] == v for k,v in reads[-1]['words'].items())
        if f == 'disconnect-cut' and c < 5:
            assert names[-3:] == ['reconcile','recover-disconnect','recover-timer'], names
        if f != 'none': assert run['reason']
        state = {p:ORIGINAL[k] for p,k in ((CACHE,'cache'),(PULSE,'pulse'),(TIMER,'timer'))}
        for call in calls:
            for _,a,v in call['stores']:
                if a == TIMER:
                    assert (state[CACHE],state[PULSE]) == (ORIGINAL['cache'],ORIGINAL['pulse']), call
                    assert v in (4000000, 5000000 if hz == 80 else 6250000), call
                if a in state: state[a] = v
        for fixtures in (False,True):
            evidence = analyse(receipt([run]), allow_synthetic=fixtures)
            assert evidence['verdict'] == 'inconclusive' and evidence['pairs'] == []
            assert {'no_trials','link_witness_missing'} <= set(evidence['reasons'])
    for bit in range(32):
        seed = ORIGINAL['pulse'] ^ (1 << bit)
        model = StockOwners(image,ops,{**ORIGINAL,'pulse':seed})
        writes = []
        model.pulse(0x50,lambda pc,a,x: writes.append(a))
        assert model.words()['pulse'] == ((seed & ~0x73) | 3)
        assert writes[0] == CACHE and writes[-1] == PULSE
        writes.clear()
        model.pulse(0x50,lambda pc,a,x: writes.append(a))
        assert not writes, 'same cached request must not repair hardware'
        model.pulse(0x20,lambda *_: None)
        assert model.words()['pulse'] == ((seed & ~0x73) | 1)
    print(f'PASS {len(jobs)} retail owner sequences; 32 full-word controls. No TV, output timing or live recovery evidence.')


if __name__ == '__main__':
    if not __debug__: raise SystemExit('checks require Python without -O')
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('image',type=Path)
    parser.add_argument('--objdump',default='xtensa-lx4son-linux-objdump')
    args = parser.parse_args()
    blob = load_image(args.image)
    qualify(blob,decode(blob,args.objdump))
