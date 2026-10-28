#!/bin/sh
# Plex's close under LumaTV's compositor, timed (round 15): YouTube 12 s then closed through SAM; Plex 12 s, `ctl note plex-close-N`, closed through SAM,
# and everything the close leaves copied at once. Up to $2 iterations (default 2), the second only when the first did not crash Plex.
# usage: sh plexclose-r15.sh OUTDIR [ITERATIONS] [lumatv|lg]     (LumaTV's compositor live, or LG's: no ctl, every iteration runs)
D=/tmp/lumatv; LIB=$D/lib; LD=$LIB/ld-linux-aarch64.so.1
O=${1:-/tmp/r15out/plexclose}; N=${2:-2}; MODE=${3:-lumatv}
YT=youtube.leanback.v4; PLEX=com.beb.plxnative; HOME_APP=com.webos.app.home
rm -rf $O; mkdir -p $O
ctl() { [ "$MODE" = lumatv ] && $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg "$@" 2>/dev/null; }
ls1() { timeout 10 luna-send -n 1 -f "luna://$1" "$2" 2>/dev/null; }
fg() { ls1 com.webos.applicationManager/getForegroundAppInfo '{}' | tr -d ' \n' | sed -n 's/.*"appId":"\([^"]*\)".*/\1/p'; }
running() { ls1 com.webos.applicationManager/running '{}' | tr -d ' \n' | grep -o '"id":"[^"]*"' | cut -d'"' -f4 | tr '\n' ' '; }
up() { cut -d' ' -f1 /proc/uptime; }
S=$O/steps.txt
say() { echo "$(up) $*" >> $S; }
close_app() { ls1 com.webos.applicationManager/closeByAppId "{\"id\":\"$1\"}"; }
gone() { w=0; while [ $w -lt 75 ]; do running | grep -q "$1" || return 0; sleep 0.2; w=$((w+1)); done; return 1; }
home() { ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.home"}' > /dev/null; }
# Whoever else has the TV: anything running that is not Home, or in front that is not what this script launched, ends the run with nothing touched.
others="$(running | sed 's/com.webos.app.home//')"
if [ -n "$(echo $others)" ]; then say "ABORT: running at the start: $others"; echo "ABORT: running at the start: $others"; exit 3; fi
touch $O/mark
tail -F /var/log/messages > $O/messages-follow.txt 2>/dev/null &
FOLLOW=$!
CR0=$(grep -c 'SIGNAL 11' /tmp/plxnative-crash.log 2>/dev/null || echo 0)
say "start: fg=$(fg) running=$(running) crash-log SIGNAL 11 count $CR0 follower $FOLLOW"
crashed=no
i=1
while [ $i -le $N ] && { [ $crashed = no ] || [ "$MODE" = lg ]; }; do
    home; sleep 3
    [ "$(fg)" = $HOME_APP ] || { say "ABORT: fg=$(fg) before iteration $i"; break; }
    # ---- YouTube, 12 s, closed through SAM
    ctl note yt-launch-$i > /dev/null
    t0=$(up); say "it$i yt launch call"
    ls1 com.webos.applicationManager/launch "{\"id\":\"$YT\"}" > /dev/null
    n=0; first=none
    while [ $n -lt 30 ]; do [ "$(fg)" = $YT ] && { first=$(awk -v a=$(up) -v b=$t0 'BEGIN{printf "%.2f", a-b}'); break; }; sleep 0.3; n=$((n+1)); done
    say "it$i yt in front after ${first}s"
    while [ "$(awk -v a=$(up) -v b=$t0 'BEGIN{print (a-b>=12)}')" = 0 ]; do sleep 0.2; done
    say "it$i yt: fg=$(fg), closing through SAM"
    ctl note yt-close-$i > /dev/null
    close_app $YT > $O/it$i-yt-close-answer.txt
    gone $YT && say "it$i yt gone" || say "it$i yt STILL RUNNING"
    home; sleep 3
    # ---- Plex, 12 s, `ctl note plex-close-N`, closed through SAM
    ctl note plex-launch-$i > /dev/null
    t0=$(up); say "it$i plex launch call"
    ls1 com.webos.applicationManager/launch "{\"id\":\"$PLEX\"}" > /dev/null
    n=0; first=none
    while [ $n -lt 30 ]; do [ "$(fg)" = $PLEX ] && { first=$(awk -v a=$(up) -v b=$t0 'BEGIN{printf "%.2f", a-b}'); break; }; sleep 0.3; n=$((n+1)); done
    say "it$i plex in front after ${first}s"
    while [ "$(awk -v a=$(up) -v b=$t0 'BEGIN{print (a-b>=11)}')" = 0 ]; do sleep 0.2; done
    PPID_=$(ls1 com.webos.applicationManager/running '{}' | tr -d ' \n' | sed -n 's/.*"id":"com.beb.plxnative","processid":"\([0-9]*\)".*/\1/p')
    rm -f $O/it$i-stall.txt.flag
    [ -n "$PPID_" ] && python3 $D/stall-snap.py $PPID_ $O/it$i-stall.txt &
    SAMPLER=$!
    while [ "$(awk -v a=$(up) -v b=$t0 'BEGIN{print (a-b>=12)}')" = 0 ]; do sleep 0.1; done
    say "it$i plex: fg=$(fg), pid ${PPID_:-?}, closing through SAM (sampler $SAMPLER)"
    ctl note plex-close-$i > /dev/null
    tc=$(up)
    touch $O/it$i-stall.txt.flag
    close_app $PLEX > $O/it$i-plex-close-answer.txt
    say "it$i plex closeByAppId answered"
    gone $PLEX && say "it$i plex gone after $(awk -v a=$(up) -v b=$tc 'BEGIN{printf "%.2f", a-b}') s" || say "it$i plex STILL RUNNING"
    # ---- at once: the logs, the ring, the compositor's side, whatever crash left behind
    ctl ring 700 > $O/it$i-ring-700.json
    ctl ring 4000 > $O/it$i-ring-4000.json
    ctl journal > $O/it$i-journal.json
    tail -n 400 /var/lib/lumatv/compositor/compositor.log > $O/it$i-compositor-log-tail.txt
    ctl windows > $O/it$i-windows.json
    { zcat /var/log/messages.3.gz /var/log/messages.2.gz /var/log/messages.1.gz /var/log/messages.0.gz 2>/dev/null; cat /var/log/messages; } | grep -a -E 'plxnative|SDL2|SDL_|APP_CLOSED|WebOS_|Unload|WAM |wayland|SIGSEGV|crash|fault' > $O/it$i-messages-filtered.txt
    cp /var/log/messages $O/it$i-messages-current.txt 2>/dev/null
    ls -la /var/log/messages* > $O/it$i-messages-files.txt
    tail -n 80 /tmp/plxnative-crash.log > $O/it$i-plxnative-crash-tail.txt 2>&1
    tail -n 60 /tmp/plxnative-events.log > $O/it$i-plxnative-events-tail.txt 2>&1
    dmesg | tail -n 60 > $O/it$i-dmesg-tail.txt 2>&1
    say "it$i copied (up $(up))"
    sleep 2
    CR1=$(grep -c 'SIGNAL 11' /tmp/plxnative-crash.log 2>/dev/null || echo 0)
    [ "$CR1" -gt "$CR0" ] && crashed=yes
    wait $SAMPLER 2>/dev/null
    say "it$i crash-log SIGNAL 11 count $CR1 (was $CR0) crashed=$crashed"
    CR0=$CR1
    home; sleep 3
    i=$((i+1))
done
sleep 2
kill $FOLLOW 2>/dev/null
find /var/spool/faultmanager /var/log/crashd /var/lib/lumatv/crash /tmp -maxdepth 3 -newer $O/mark -type f 2>/dev/null | grep -v "^$O" | grep -v '^/tmp/r15' | head -80 > $O/new-files.txt
for f in $(grep -i 'crash\|fault' $O/new-files.txt | head -12); do [ -f "$f" ] && [ "$(wc -c < $f)" -lt 300000 ] && { mkdir -p $O/crash-files; cp "$f" "$O/crash-files/$(echo $f | tr '/' '_')"; }; done
ls1 com.webos.applicationManager/closeByAppId "{\"id\":\"$YT\"}" > /dev/null; ls1 com.webos.applicationManager/closeByAppId "{\"id\":\"$PLEX\"}" > /dev/null
sleep 1; home; sleep 3
say "end: fg=$(fg) running=$(running) crashed=$crashed"
echo "plexclose done: crashed=$crashed; $(wc -l < $S) step lines"
