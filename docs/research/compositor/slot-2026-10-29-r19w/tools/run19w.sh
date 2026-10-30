#!/bin/sh
# run19w.sh NAME ARGS...: one tv_gate run, its stdout in /tmp/r19w-gate/NAME.out with an EXIT line at the end.
# A file /tmp/r19w-gate/PAUSE holds the run back at its start (between two steps of the chain) until it is removed.
name=$1; shift
while [ -e /tmp/r19w-gate/PAUSE ]; do sleep 5; done
mkdir -p /tmp/r19w-gate
cd /tmp/r19w-build
rm -rf /tmp/lumatv-tv-out/out/gate /tmp/lumatv-tv-out/gate.tgz
echo "start $(date +%T) $*" > /tmp/r19w-gate/$name.out
python3 gate_r19w.py "$@" >> /tmp/r19w-gate/$name.out 2>&1
echo "EXIT $? $(date +%T)" >> /tmp/r19w-gate/$name.out
