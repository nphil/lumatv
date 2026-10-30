#!/bin/sh
# Round 21: forge/main HEAD, target dirs warm from /tmp/lumatv-r20.
export PATH=/data/home/.local/bin:/data/home/.cargo/bin:$PATH
A=/tmp/lumatv-r21
B=/tmp/lumatv-r21
L=/tmp/r21-build
t0=$(date +%s)
ts() { echo "$(( $(date +%s) - t0 )) s"; }
: > $L/status21.txt
cd $B || exit 1
git fetch -q forge
true && echo "checkout $(git rev-parse --short HEAD) $(ts)" >> $L/status21.txt
echo "seed done $(ts)" >> $L/status21.txt
( cd helper && timeout 1500 ./build.sh > $L/helper21.log 2>&1; echo "helper rc=$? $(ts)" >> $L/status21.txt ) &
( timeout 1700 sh compositor/tools/build-tv.sh > $L/bundle21.log 2>&1; echo "bundle rc=$? $(ts)" >> $L/status21.txt ) &
wait
sh gfn/feeder/build.sh > $L/feeder21.log 2>&1; sh tools/build-ipk.sh > $L/ipk21.log 2>&1; echo "ipk rc=$? $(ts)" >> $L/status21.txt
echo "ALL DONE $(ts)" >> $L/status21.txt
