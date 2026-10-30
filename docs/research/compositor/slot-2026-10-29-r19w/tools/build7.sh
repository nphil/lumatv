#!/bin/sh
# Round 19W: forge/main HEAD (PageAlpha's AR24 default, HomeParity's, DisplayLab's) in the fresh worktree: warm target dirs from /tmp/lumatv-r19, then helper, bundle and ipk.
export PATH=/data/home/.local/bin:/data/home/.cargo/bin:$PATH
A=/tmp/lumatv-r19
B=/tmp/lumatv-r19w
L=/tmp/r19w-build
t0=$(date +%s)
ts() { echo "$(( $(date +%s) - t0 )) s"; }
: > $L/status7.txt
cd $B || exit 1
git fetch -q forge
git checkout -q --detach forge/main && echo "checkout $(git rev-parse --short HEAD) $(ts)" >> $L/status7.txt
for d in ambient/capture/build jpeg/build vplane/build gfx64/build helper/target helper/dist compositor/target rustui/target rustui/build rustui/app/stub/build gfn/build gfn/feeder/build gfn/engine/target build; do
  [ -d "$A/$d" ] && { mkdir -p "$B/$d" && cp -a "$A/$d/." "$B/$d/" & }
done
wait
echo "seed done $(ts)" >> $L/status7.txt
( cd helper && timeout 1500 ./build.sh > $L/helper7.log 2>&1; echo "helper rc=$? $(ts)" >> $L/status7.txt ) &
( timeout 1700 sh compositor/tools/build-tv.sh > $L/bundle7.log 2>&1; echo "bundle rc=$? $(ts)" >> $L/status7.txt ) &
wait
sh tools/build-ipk.sh > $L/ipk7.log 2>&1; echo "ipk rc=$? $(ts)" >> $L/status7.txt
echo "ALL DONE $(ts)" >> $L/status7.txt
