#!/bin/sh
# tv-plxcore.sh [N]: up to N PlxNative launch and close passes like the apps gate (about 17 s each); the first pass that crashes leaves its core
# file (anonymous memory only), the thread names, the maps and the compositor's ring and journal in /media/developer/plxcore/ and ends the run.
# Needs /tmp/plxprlimit on the set (plxprlimit.c: zig cc -target arm-linux-musleabi -static -Os). core_pattern is put back at the end, whatever happened.
N=${1:-20}
D=/media/developer/plxcore
mkdir -p "$D"
old=$(cat /proc/sys/kernel/core_pattern)
trap 'echo "$old" > /proc/sys/kernel/core_pattern' EXIT INT TERM
echo "$D/core.%e.%p.%t" > /proc/sys/kernel/core_pattern
ls1() { timeout 10 luna-send -n 1 -f "luna://$1" "$2" 2>/dev/null; }
LD_D=/tmp/lumatv
ctl() { "$LD_D"/lib/ld-linux-aarch64.so.1 --library-path "$LD_D"/lib "$LD_D"/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg "$@" 2>/dev/null; }
i=1
while [ "$i" -le "$N" ]; do
    ls1 com.webos.applicationManager/launch '{"id":"com.beb.plxnative"}' > /dev/null
    sleep 11
    pid=$(pidof plxnative | cut -d' ' -f1)
    if [ -n "$pid" ]; then
        /tmp/plxprlimit "$pid"
        for t in /proc/"$pid"/task/*; do echo "$(basename "$t") $(cat "$t"/comm)"; done > "$D/threads.$i.txt"
        cp /proc/"$pid"/maps "$D/maps.$i.txt"
    fi
    ctl note "plxcore-close-$i" > /dev/null
    ls1 com.webos.applicationManager/closeByAppId '{"id":"com.beb.plxnative"}' > /dev/null
    sleep 6
    if ls "$D"/core.* > /dev/null 2>&1; then
        last=0; same=0
        while [ "$same" -lt 2 ]; do
            cur=$(ls -l "$D"/core.* | awk '{ s += $5 } END { print s }')
            if [ "$cur" = "$last" ]; then same=$((same + 1)); else same=0; fi
            last=$cur
            sleep 2
        done
        echo "pass $i: crashed, core kept"
        ctl ring 4000 > "$D/ring.$i.json"
        ctl journal > "$D/journal.$i.json"
        break
    fi
    echo "pass $i: clean (pid ${pid:-none})"
    rm -f "$D/threads.$i.txt" "$D/maps.$i.txt"
    i=$((i + 1))
done
ls -la "$D"
