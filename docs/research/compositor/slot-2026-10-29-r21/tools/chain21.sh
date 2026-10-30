#!/bin/sh
# frames at 60 and 120 Hz, quick surface set, one after the other
cd /tmp/lumatv-r21
for spec in "frames60q --only frames --frames-quick" "frames120q --only frames --frames-quick --refresh 120"; do
  set -- $spec; name=$1; shift
  rm -rf /tmp/lumatv-tv-out/out/gate /tmp/lumatv-tv-out/gate.tgz
  echo "start $(date +%T) $*" > /tmp/r21/gate/$name.out
  python3 compositor/tools/tv_gate.py "$@" >> /tmp/r21/gate/$name.out 2>&1
  echo "EXIT $? $(date +%T)" >> /tmp/r21/gate/$name.out
  mkdir -p /tmp/r21/gate/$name; cp -r /tmp/lumatv-tv-out/out/gate/. /tmp/r21/gate/$name/ 2>/dev/null
  cp /tmp/lumatv-tv-out/frames*.json /tmp/r21/gate/$name/ 2>/dev/null
  echo "$(date +%T) $name done" >> /tmp/r21/gate/chain.log
done
