#!/usr/bin/env python3
"""Run the retail split owners in private host memory. No live mode, device I/O, delay or panel timing estimates."""
import argparse
import json
from pathlib import Path
import subprocess
import sys
import time

from paired_evidence import FORMAT
from recovery_check import split_words
from split_stock import IMAGE_SHA, StockOwners, decode, load_image

ORIGINAL = dict(cache=0x20, pulse=0xa5c5b009, pixel=0x81, timer=4_000_000, timer_num=100, timer_den=1)
ARM_NS = 1_000_000_000 // 60   # Two nominal 120 Hz periods, a host abort budget, NOT a hardware guarantee.
RUN_NS = 100_000_000
FAULTS = ('none','arm-cut','disconnect-cut','deadline','mismatch','hang')


class Stop(Exception):
    pass


def rehearse(image, ops, hz, fault='none', *, cut=1, now=time.monotonic_ns):
    if hz not in (64,80) or fault not in FAULTS or not 1 <= cut <= 5:
        raise ValueError('unsupported host scenario')
    emu = StockOwners(image, ops, ORIGINAL)
    original = split_words('restored', ORIGINAL)
    selected = split_words('selected'+str(hz), ORIGINAL)
    disconnected = split_words('disconnected'+str(hz), ORIGINAL)
    start = now(); deadline = start + RUN_NS
    attempts, reads, reasons = [], [], []
    faulted = False
    unreadable = False
    mismatch = False

    def guard():
        t = now()
        if t >= deadline: raise Stop('deadline expired')
        return t - start

    def call(name, owner, *args):
        nonlocal faulted
        row = {'step':name,'owner':owner,'args':[0,*args],'start_ns':guard(),'stores':[]}
        attempts.append(row)
        def on_store(pc, address, value):
            nonlocal faulted
            row['stores'].append([pc,address,value])
            if (not faulted and len(row['stores']) == cut and
                (name,fault) in (('select','arm-cut'), ('disconnect','disconnect-cut'))):
                faulted = True
                raise Stop('interrupted stock store sequence')
            guard()
        if owner == 'pulse': emu.pulse(*args, on_store)
        else: emu.period(*args, on_store)
        row['end_ns'] = guard()

    def readback(label):
        guard()
        if unreadable: raise Stop('owner and readback unavailable')
        words = emu.words()
        if mismatch: words['pulse'] ^= 1 << 31  # Unknown change outside the owned field.
        reads.append({'step':label, 'ns':guard(), 'words':words})
        return words

    def check(label, expected):
        words = readback(label)
        wrong = [key for key in expected if words[key] != expected[key]]
        if wrong: raise Stop(label + ': mismatched ' + ','.join(wrong))

    def restore():
        w = readback('recovery-before')
        if (w['pixel'] != original['pixel'] or w['cache'] not in (0x20,0x50) or
            w['pulse'] not in (original['pulse'],selected['pulse']) or w['timer'] not in (original['timer'],selected['timer'])):
            raise Stop('unknown state; no corrective write')
        base = {**original,'timer':w['timer']}
        if w['cache'] == 0x20 and w['pulse'] == selected['pulse']:
            # A repeated original request is a cache hit: establish cache/hardware agreement first.
            call('reconcile','pulse',0x50)
            check('reconciled',{**selected,'timer':w['timer']})
        if w['cache'] != 0x20 or w['pulse'] != original['pulse']:
            call('recover-disconnect','pulse',0x20)
        check('proved-disconnected',base)
        if w['timer'] != original['timer']:
            call('recover-timer','timer',ORIGINAL['timer_num'],ORIGINAL['timer_den'])
        check('restored', original)

    try:
        check('before', original)
        call('set-disconnected-timer','timer',hz*100,100)
        check('disconnected', disconnected)
        deadline = min(deadline, now() + ARM_NS)
        call('select','pulse',0x50)
        if fault == 'deadline': deadline = now()  # Exercise the same deadline check without sleeping.
        unreadable, mismatch = fault == 'hang', fault == 'mismatch'
        check('selected', selected)
        call('disconnect','pulse',0x20)
        check('disconnected-again', disconnected)
        deadline = start + RUN_NS
        call('restore-timer','timer',ORIGINAL['timer_num'],ORIGINAL['timer_den'])
        check('restored', original)
        outcome = 'rehearsed'
    except Stop as error:
        reasons.append(str(error))
        # One bounded cleanup pass. A missing observer or unknown field cannot be repaired by guessing.
        deadline = now() + ARM_NS
        try:
            restore()
            outcome = 'aborted-restored'
        except Stop as rollback_error:
            reasons.append(str(rollback_error)); outcome = 'unrecovered'
    return {'outcome':outcome,'reason':reasons,'fault':fault,'store_cut':cut,'hz':hz,'elapsed_host_ns':now()-start,
            'owner_calls':attempts,'readbacks':reads}


def receipt(runs):
    # A held-picture wiring rehearsal contains no +11/+13 paired trials. Never invent phase, IDs or link events.
    return {'format':FORMAT,'version':1,'clock':{'calibrated':False},'witness':{'resolves_repeats':False},'trials':[],
            'inventory':{'channels':{},'unavailable':['selected-frame id','decoder completion','release','link frame start']},
            'split_rehearsal':{'scope':'host-memory-only','image_sha256':IMAGE_SHA,'original_assumed':ORIGINAL,
                              'arm_budget_host_ns':ARM_NS,'independent_disconnect':False,'runs':runs}}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('image',type=Path,help='copied retail boot file or extracted image 0')
    p.add_argument('--objdump',default='xtensa-lx4son-linux-objdump',help='installed Xtensa disassembler')
    p.add_argument('--rate',type=int,choices=(64,80),default=80)
    p.add_argument('--fault',choices=FAULTS,default='none')
    args = p.parse_args()
    try:
        image = load_image(args.image); ops = decode(image,args.objdump)
        run = rehearse(image,ops,args.rate,args.fault)
    except (OSError, ValueError, subprocess.SubprocessError, KeyError) as exc:
        p.exit(1, f'STOP: {exc}\n')
    print(json.dumps(receipt([run]),indent=2))
    print(f"HOST ONLY: {run['outcome']}; zero observations of the TV",file=sys.stderr)
    return 0 if run['outcome'] == 'rehearsed' else 2


if __name__ == '__main__':
    sys.exit(main())
