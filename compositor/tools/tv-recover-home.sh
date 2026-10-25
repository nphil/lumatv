#!/bin/sh
# Brings the user interface back after the compositor was replaced: the KMS maintenance window, the end of a trial
# cutover, lumatv-sm's fallback and the forward switch to LumaTV's compositor all end here. Runs ON THE SET
# (busybox sh); installed as /tmp/lumatv/recover-home.sh and /var/lib/lumatv/compositor/bin/recover-home.sh.
#
# What goes wrong without it (first KMS window, 2026-10-25): webapp-mgr.service is Restart=on-failure and PartOf
# surface-manager.service (the oneshot wrapper, not the daemon unit); when the compositor goes away WebAppMgr
# restarts into no socket until systemd's start limit leaves it "failed", and the web Home stub (a WebAppMgr app)
# cannot launch: black panel. The fix is the sequence tools/lumatv's relaunch_home uses after a compositor
# restart: reset the unit, start it, wait until listRunningApps answers, launch Home once, wait for it. Under
# LG's compositor "it" is pixels on the panel; under LumaTV's the Home app is the transparent stub (LumaHome is
# LG's compositor's QML), so there the wait is for Home in listRunningApps.
#
#   recover-home.sh [LOG]   wait for surface-manager-daemon to be active, recover WebAppMgr, launch Home,
#                           sweep the librdx crash reports (HANDOFF "Standby"); lines appended to LOG if given
WAM=webapp-mgr.service
SM=surface-manager-daemon.service
LOG=${1:-}
CAP=/tmp/lumatv-recover.png

say() { echo "recover: $*"; [ -n "$LOG" ] && echo "$(date '+%H:%M:%S') recover: $*" >> "$LOG"; }
ls_ok() { grep -q '"returnValue"[[:space:]]*:[[:space:]]*true'; }
ours() { grep -q ' /usr/bin/surface-manager ' /proc/self/mountinfo && pgrep -f 'bin/lumatv-compositor' > /dev/null; }
home_running() { timeout 10 luna-send -n 1 luna://com.palm.webappmanager/listRunningApps '{}' 2>/dev/null | grep -q '"com.webos.app.home"'; }

# One recovery at a time: the supervisor's fallback and the trial script's revert can both ask for it, and a
# second Home launch while the first is still waiting for pixels leaves the panel black (tools/lumatv).
LOCK=/tmp/lumatv-recover.lock
if ! mkdir $LOCK 2>/dev/null; then
    say "another recovery is running; waiting for it"
    n=0; while [ -d $LOCK ] && [ $n -lt 120 ]; do sleep 1; n=$((n+1)); done
    [ -d $LOCK ] && rm -rf $LOCK
    exit 0
fi
trap 'rm -rf $LOCK' EXIT INT TERM
n=0
while [ "$(systemctl is-active $SM)" != "active" ] && [ $n -lt 40 ]; do sleep 1; n=$((n+1)); done
say "$SM $(systemctl is-active $SM) after ${n}s; exe $(readlink /proc/$(pgrep -x surface-manager | head -1)/exe 2>/dev/null)"
n=0
while [ ! -S /tmp/xdg/wayland-0 ] && [ $n -lt 20 ]; do sleep 1; n=$((n+1)); done
[ -S /tmp/xdg/wayland-0 ] || say "no /tmp/xdg/wayland-0 after ${n}s"

st=$(systemctl is-active $WAM)
if [ "$st" != "active" ]; then
    systemctl reset-failed $WAM 2>/dev/null
    systemctl start $WAM
    say "$WAM was $st; started: $(systemctl is-active $WAM)"
fi
n=0
while [ $n -lt 30 ]; do
    if timeout 10 luna-send -n 1 luna://com.palm.webappmanager/listRunningApps '{}' 2>/dev/null | ls_ok; then break; fi
    sleep 1; n=$((n+1))
done
say "WebAppMgr answering after ${n}s ($(systemctl is-active $WAM))"
sleep 1
timeout 10 luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}' >/dev/null 2>&1
n=0
if ours; then
    while [ $n -lt 30 ]; do
        home_running && break
        sleep 1; n=$((n+1))
    done
    if [ $n -lt 30 ]; then say "Home running under LumaTV's compositor after ${n}s"; else say "Home not in listRunningApps after 30 s"; fi
else
    while [ $n -lt 30 ]; do
        b=$(timeout 10 luna-send -n 1 luna://com.webos.service.capture/executeOneShot "{\"path\":\"$CAP\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":320,\"height\":180}" 2>/dev/null | tr ',' '\n' | grep writtenBytes | tr -dc '0-9')
        [ -n "$b" ] && [ "$b" -gt 4096 ] && break
        sleep 1; n=$((n+1))
    done
    if [ $n -lt 30 ]; then say "Home on the panel after ${n}s (capture $b bytes)"; else say "panel still blank after 30 s (capture ${b:-none} bytes)"; fi
fi
rm -f $CAP
# The compositor restart makes com.webos.app.inputcommon crash on teardown; its report would make faultmanager
# reboot the set at the next power-off. Pathname expansion handles the raw bytes in the names.
sleep 10
rm -f /tmp/var/log/reports/librdx/* 2>/dev/null
say "librdx reports swept; remaining $(ls /tmp/var/log/reports/librdx 2>/dev/null | wc -l)"
