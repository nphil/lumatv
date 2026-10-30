#!/bin/sh
# run20.sh NAME ARGS...: one tv_gate run; stdout in /tmp/r20/gate/NAME.out; the gate dir copied to /tmp/r20/gate/NAME/
name=$1; shift
[ -e /tmp/r20/FAMILY ] && { echo "FAMILY, skipping $name" >> /tmp/r20/gate/chain.log; exit 1; }
mkdir -p /tmp/r20/gate
cd /tmp/r20-build
rm -rf /tmp/lumatv-tv-out/out/gate /tmp/lumatv-tv-out/gate.tgz
echo "start $(date +%T) $*" > /tmp/r20/gate/$name.out
python3 gate_r19w.py "$@" >> /tmp/r20/gate/$name.out 2>&1
echo "EXIT $? $(date +%T)" >> /tmp/r20/gate/$name.out
mkdir -p /tmp/r20/gate/$name; cp -r /tmp/lumatv-tv-out/out/gate/. /tmp/r20/gate/$name/ 2>/dev/null
echo "$(date +%T) $name done" >> /tmp/r20/gate/chain.log
