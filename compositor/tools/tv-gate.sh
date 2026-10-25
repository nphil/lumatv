#!/bin/sh
# The C4 gate, executed ON THE SET as /tmp/lumatv/gate.sh while LumaTV's compositor is live (cutover.sh enable).
# Each check prints one "GATE <name>: PASS|FAIL <detail>" line; files under /tmp/lumatv/out/gate/. Driven from
# the workstation by tools/tv_gate.py, which mutes the receiver, wakes the set and orders the checks.
#
#   gate.sh live      LumaTV is the compositor: mounted, 12 names registered, no fallback, WAM up, Home (the web
#                     stub) mapped and on the panel
#   gate.sh x1 SECS   the X1 probe client (64-bit, LG's blob) for SECS: no frame gap over 20 ms; pacer late 0 lost 0
#   gate.sh netflix   the Dolby Vision test film by deep link: card mapped, video window exported, Picture Mode
#                     dolbyHdrCinema while playing, filmMaker back on Home
#   gate.sh hdmi      HDMI 2 (the receiver) as the foreground app, then Home
#   gate.sh standby   power off (remoteKey); the driver wakes the set; `gate.sh woke` then checks the rounds
#   gate.sh woke      after the wake: PreparePowerOff and PreparePowerOn acked, no fallback, Home on the panel
#   gate.sh escape    Home held 1 s through the compositor's own routing: the supervisor falls back to LG's,
#                     records the escape, LG's Home comes back
STATE=/var/lib/lumatv/compositor
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
BLOB=$LIB/libmali-bifrost-g52-g24p0-wayland-gbm.so
OUT=$D/out/gate
mkdir -p $OUT
ctl() { $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg "$@" 2>/dev/null; }
ls1() { timeout 10 luna-send -n 1 -f "luna://$1" "$2" 2>/dev/null; }
cap() { timeout 10 luna-send -n 1 luna://com.webos.service.capture/executeOneShot "{\"path\":\"$1\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}" 2>/dev/null | tr ',' '\n' | grep writtenBytes | tr -dc '0-9'; }
is_mounted() { grep -q " /usr/bin/surface-manager " /proc/self/mountinfo; }
pass() { echo "GATE $1: PASS $2"; }
fail() { echo "GATE $1: FAIL $2"; }
picture_mode() { ls1 com.webos.settingsservice/getSystemSettings '{"category":"picture","keys":["pictureMode"]}' | tr -d ' \n' | sed -n 's/.*"pictureMode":"\([^"]*\)".*/\1/p'; }
home() { ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.home"}' > /dev/null; }
fg() { ls1 com.webos.applicationManager/getForegroundAppInfo '{}' | tr -d ' \n' | sed -n 's/.*"appId":"\([^"]*\)".*/\1/p'; }
# With the Home stub mounted (tools/lumatv homestub) Home is a transparent page by design and LumaHome lives in
# LG's compositor's QML, so a Home on the panel under LumaTV's compositor is black: the panel test then passes on
# the mapped window alone and the detail line carries home-stub 1.
stub_mounted() { grep -c ' /usr/palm/applications/com.webos.app.home ' /proc/self/mountinfo; }
home_panel_ok() { [ "$(stub_mounted)" -ge 1 ] || [ "${1:-0}" -gt 4096 ]; }

case "${1:-help}" in
live)
    m=$(is_mounted && echo yes || echo no)
    regs=$(grep -c '^\[.*\] registered com\.' $STATE/sm.log 2>/dev/null)
    fb=$(grep -c 'fallback:' $STATE/sm.log 2>/dev/null)
    wam=$(systemctl is-active webapp-mgr)
    ctl windows > $OUT/live-windows.json
    homewin=$(grep -c '"com.webos.app.home"' $OUT/live-windows.json)
    b=$(cap $OUT/live.png)
    detail="mounted $m registered $regs fallbacks $fb wam $wam home-window $homewin home-stub $(stub_mounted) panel ${b:-0} bytes fg $(fg)"
    if [ "$m" = yes ] && [ "$regs" -ge 12 ] && [ "$fb" = 0 ] && [ "$wam" = active ] && [ "$homewin" -ge 1 ] && home_panel_ok "$b"; then pass live "$detail"; else fail live "$detail"; fi
    ;;
x1)
    secs=${2:-60}
    before=$(ctl status | sed -n 's/.*\(flips [0-9]* late [0-9]* lost [0-9]*\).*/\1/p')
    XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0 X1_SECONDS=$secs X1_WINDOW_TYPE=_WEBOS_WINDOW_TYPE_OVERLAY \
        $LD --library-path $LIB --preload $LIB/x1heapshim.so $D/bin/x1client $BLOB > $OUT/x1.log 2>&1
    res=$(grep RESULT $OUT/x1.log)
    after=$(ctl status | sed -n 's/.*\(flips [0-9]* late [0-9]* lost [0-9]*\).*/\1/p')
    over=$(echo "$res" | sed -n 's/.*20-33.5 \([0-9]*\), >33.5 \([0-9]*\).*/\1+\2/p')
    over=$((${over:-1+1}))
    fps=$(echo "$res" | sed -n 's/.* = \([0-9.]*\) fps.*/\1/p')
    late_before=$(echo "$before" | awk '{print $4+$6}'); late_after=$(echo "$after" | awk '{print $4+$6}')
    detail="$res | pacer before: $before after: $after"
    echo "$detail" > $OUT/x1-result.txt
    if [ -n "$res" ] && [ "$over" = 0 ] && [ "${late_before:-0}" = "${late_after:-1}" ]; then pass x1 "$fps fps, no gap over 20 ms, pacer late/lost unchanged"; else fail x1 "$detail"; fi
    ;;
netflix)
    ls1 com.webos.applicationManager/launch '{"id":"netflix","params":{"contentTarget":"m=https://www.netflix.com/watch/80141336&source_type=4"}}' > /dev/null
    sleep 12
    ctl remote 28 tap > /dev/null
    sleep 25
    ctl windows > $OUT/netflix-windows.json
    ctl journal > $OUT/netflix-journal.json
    card=$(grep -c '"netflix"' $OUT/netflix-windows.json)
    video=$(grep -o '"video":\[[^]]*\]' $OUT/netflix-windows.json | head -c 300)
    placements=$(grep -c 'setDisplayWindow' $OUT/netflix-journal.json)
    pm=$(picture_mode)
    b=$(cap $OUT/netflix.png)
    ls1 com.webos.applicationManager/closeByAppId '{"id":"netflix"}' > /dev/null
    sleep 2; home; sleep 6
    pm2=$(picture_mode)
    detail="netflix windows $card video $video placements $placements pictureMode playing $pm after $pm2 fg $(fg) panel ${b:-0} bytes"
    echo "$detail" > $OUT/netflix-result.txt
    if [ "$card" -ge 1 ] && [ "$pm" = dolbyHdrCinema ] && [ "$pm2" = filmMaker ]; then pass netflix "$detail"; else fail netflix "$detail"; fi
    ;;
hdmi)
    ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.hdmi2"}' > /dev/null
    sleep 10
    ctl windows > $OUT/hdmi-windows.json
    card=$(grep -c '"com.webos.app.hdmi2"' $OUT/hdmi-windows.json)
    f=$(fg)
    b=$(cap $OUT/hdmi.png)
    home; sleep 6
    detail="hdmi2 windows $card fg while open $f now $(fg) panel ${b:-0} bytes"
    echo "$detail" > $OUT/hdmi-result.txt
    if [ "$card" -ge 1 ] && [ "$f" = com.webos.app.hdmi2 ]; then pass hdmi "$detail"; else fail hdmi "$detail"; fi
    ;;
standby)
    ctl journal > $OUT/standby-journal-before.json
    cat /proc/uptime > $OUT/standby-uptime-before
    ls1 com.webos.service.tvpower/power2/powerOff '{"reason":"remoteKey"}' > $OUT/standby-poweroff.txt
    echo "GATE standby: requested ($(head -c 120 $OUT/standby-poweroff.txt))"
    ;;
woke)
    sleep 8
    ctl journal > $OUT/woke-journal.json
    ctl status > $OUT/woke-status.json
    off=$(grep -c 'PreparePowerOff' $OUT/woke-journal.json); on=$(grep -c 'PreparePowerOn' $OUT/woke-journal.json)
    fb=$(grep -c 'fallback:' $STATE/sm.log 2>/dev/null)
    st=$(ls1 com.webos.service.tvpower/power/getPowerState '{}' | tr -d ' \n' | sed -n 's/.*"state":"\([^"]*\)".*/\1/p' | tr 'A-Z' 'a-z')
    home; sleep 8
    b=$(cap $OUT/woke.png)
    detail="rounds: PreparePowerOff $off PreparePowerOn $on; fallbacks $fb; state $st; uptime before $(cut -d' ' -f1 $OUT/standby-uptime-before) now $(cut -d' ' -f1 /proc/uptime); mounted $(is_mounted && echo yes || echo no); panel ${b:-0} bytes"
    echo "$detail" > $OUT/woke-result.txt
    if [ "$off" -ge 1 ] && [ "$on" -ge 1 ] && [ "$fb" = 0 ] && [ "$st" = active ] && is_mounted && home_panel_ok "$b"; then pass standby "$detail"; else fail standby "$detail"; fi
    ;;
escape)
    ctl remote 172 press > /dev/null
    sleep 1
    ctl remote 172 release > /dev/null 2>&1
    u=0; while is_mounted && [ $u -lt 20 ]; do sleep 0.5; u=$((u+1)); done
    sleep 2
    fb=$(grep 'fallback:' $STATE/sm.log | tail -1)
    esc=$([ -f $STATE/escaped ] && echo yes || echo no)
    exe=$(readlink /proc/$(pgrep -x surface-manager | head -1)/exe 2>/dev/null)
    # recover-home.sh (started by the supervisor) brings Home back; wait for it.
    n=0; while [ -d /tmp/lumatv-recover.lock ] && [ $n -lt 120 ]; do sleep 1; n=$((n+1)); done
    sleep 2
    b=$(cap $OUT/escape.png)
    detail="unmounted after $((u/2))s; $fb; escaped $esc; exe $exe; recovery ${n}s; panel ${b:-0} bytes; wam $(systemctl is-active webapp-mgr)"
    echo "$detail" > $OUT/escape-result.txt
    if ! is_mounted && [ "$esc" = yes ] && echo "$fb" | grep -q 'hold Home' && [ "${b:-0}" -gt 4096 ]; then pass escape "$detail"; else fail escape "$detail"; fi
    ;;
*) sed -n 2,17p $0 ;;
esac
