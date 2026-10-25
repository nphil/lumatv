#!/bin/sh
# C4 cutover, executed ON THE SET as /tmp/lumatv/cutover.sh. LumaTV's compositor becomes the live one with LG's
# compositor kept installed: the supervisor (lumatv-sm) is bind-mounted over /usr/bin/surface-manager and the unit
# restarted; on any failure the supervisor itself execs LG's binary at that path and brings LG's Home back. The
# early boot hook (tools/lumatv compositor-boot, tools/compositor.sh) places the mount at boot while the enabled
# flag exists, so only `enable` (and a running trial) sets it: a reboot during a test session comes up on LG's.
#
#   cutover.sh install     copy the bundle to /var/lib/lumatv/compositor, record the fingerprint, reset failures
#                          (not enabled)
#   cutover.sh probe       ask ls-hubd for the real bus names from the real path with LG's compositor running: tells
#                          whether the role allows them to this file (exit 0) or not (exit 2); ~2 s, nothing restarts
#   cutover.sh trial SECS  mount, restart the unit, watch for SECS seconds, restore LG's (default 120); the flag
#                          exists only while the trial runs
#   cutover.sh enable      flag, mount, restart the unit and leave LumaTV's compositor live (also after reboots)
#   cutover.sh disable     unmount, restart LG's compositor, bring Home back; the enabled flag is removed
#   cutover.sh status      what is live now
#   cutover.sh revert      unmount and restart LG's compositor (the flag stays)
STATE=/var/lib/lumatv/compositor
D=/tmp/lumatv
LG=/usr/bin/surface-manager

is_mounted() { grep -q " $LG " /proc/self/mountinfo; }

revert() {
    # When the supervisor already fell back (unmounted and execd LG's binary, recovery under way) a second restart
    # would only take LG's compositor down again; then only the recovery is joined.
    if is_mounted; then
        umount "$LG" || umount -l "$LG"
        systemctl restart surface-manager-daemon
    fi
    echo "revert: $(systemctl is-active surface-manager-daemon); mounted: $(is_mounted && echo yes || echo no); exe: $(readlink /proc/$(pgrep -x surface-manager | head -1)/exe 2>/dev/null)"
    # A recovery still running from the forward switch would make this one a no-op (recover-home.sh joins a
    # running one and exits); it is bounded, so wait for it first.
    n=0; while [ -d /tmp/lumatv-recover.lock ] && [ $n -lt 150 ]; do sleep 1; n=$((n+1)); done
    sh $STATE/bin/recover-home.sh $STATE/sm.log
}

case "${1:-help}" in
install)
    mkdir -p $STATE/bin $STATE/lib
    cp $D/bin/lumatv-compositor $D/bin/lumatv-sm $STATE/bin/
    cp $D/recover-home.sh $STATE/bin/recover-home.sh
    cp $D/lib/* $STATE/lib/
    chmod +x $STATE/bin/*
    $STATE/bin/lumatv-sm --print-fingerprint > $STATE/fingerprint
    stat -c '%s %Y' $LG > $STATE/lg-binary
    echo 0 > $STATE/failures
    rm -f $STATE/escaped $STATE/last-fallback $STATE/enabled
    echo "installed (not enabled): fingerprint $(cat $STATE/fingerprint); LG binary $(cat $STATE/lg-binary); $(du -sh $STATE | cut -f1)"
    ;;
probe)
    [ -x $STATE/bin/lumatv-sm ] || { echo "run install first"; exit 1; }
    if is_mounted; then echo "already mounted; refusing"; exit 1; fi
    mount --bind $STATE/bin/lumatv-sm $LG
    $LG --probe --state $STATE
    rc=$?
    umount $LG || umount -l $LG
    echo "probe exit $rc; mounted now: $(is_mounted && echo yes || echo no); LG's compositor pid $(pgrep -x surface-manager | head -1) $(systemctl is-active surface-manager-daemon)"
    exit $rc
    ;;
status)
    echo "mounted over $LG: $(is_mounted && echo yes || echo no)"
    echo "surface-manager-daemon: $(systemctl is-active surface-manager-daemon) pid $(pgrep -x surface-manager | head -1); webapp-mgr: $(systemctl is-active webapp-mgr)"
    echo "lumatv-sm: $(pgrep -f lumatv-sm | head -1 || echo none); compositor: $(pgrep -f 'bin/lumatv-compositor' | head -1 || echo none)"
    ls -la $STATE 2>/dev/null | grep -vE 'bin|lib'
    tail -5 $STATE/sm.log 2>/dev/null
    ;;
trial|enable)
    secs=${2:-120}
    [ -x $STATE/bin/lumatv-sm ] || { echo "run install first"; exit 1; }
    if is_mounted; then echo "already mounted; 'cutover.sh status' says what runs"; exit 1; fi
    echo 0 > $STATE/failures; rm -f $STATE/escaped
    touch $STATE/enabled
    if [ "$1" = trial ]; then trap 'rm -f $STATE/enabled; revert' EXIT INT TERM; fi
    mount --bind $STATE/bin/lumatv-sm $LG
    echo "mounted; restarting the unit"
    systemctl restart surface-manager-daemon
    # WebAppMgr loses its socket with the old compositor and restarts into none until systemd's start limit
    # leaves it failed (webapp-mgr.service is PartOf surface-manager.service, not the daemon unit, so nothing
    # restarts it for us); the same recovery as after a fallback brings it and Home back once the supervisor
    # reports ready. It holds /tmp/lumatv-recover.lock while it runs.
    ( sh $STATE/bin/recover-home.sh $STATE/sm.log > /dev/null 2>&1 & )
    t=0
    while [ $t -lt $secs ]; do
        sleep 5; t=$((t+5))
        sm=$(pgrep -x surface-manager | head -1)
        echo "t=${t}s unit=$(systemctl is-active surface-manager-daemon) exe=$(readlink /proc/$sm/exe 2>/dev/null) compositor=$(pgrep -f 'bin/lumatv-compositor' | head -1 || echo none) mounted=$(is_mounted && echo yes || echo no) wam=$(systemctl is-active webapp-mgr)"
        tail -2 $STATE/sm.log 2>/dev/null | sed 's/^/   /'
        if ! is_mounted; then echo "the supervisor fell back to LG's compositor: $(cat $STATE/last-fallback 2>/dev/null)"; break; fi
        if [ "$1" = enable ] && [ $t -ge 30 ]; then break; fi
    done
    if [ "$1" = trial ]; then
        # Evidence for the note before LG's compositor comes back: who connected, which windows mapped, what the
        # compositor journalled, and the panel as LG's capture service reads it.
        O=$D/out/trial; mkdir -p $O
        if pgrep -f 'bin/lumatv-compositor' > /dev/null; then
            for c in status windows journal luna; do
                $STATE/lib/ld-linux-aarch64.so.1 --library-path $STATE/lib $STATE/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg $c > $O/$c.json 2>/dev/null
            done
            # SAM's own view next to the compositor's: the foreground SAM reports is what the gate's checks read.
            timeout 10 luna-send -n 1 -f luna://com.webos.applicationManager/getForegroundAppInfo '{}' > $O/sam-foreground.json 2>&1
            timeout 10 luna-send -n 1 luna://com.webos.service.capture/executeOneShot "{\"path\":\"$O/panel.png\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}" > /dev/null 2>&1
            timeout 10 luna-send -n 1 -f luna://com.webos.surfacemanager/getForegroundAppInfo '{}' > $O/foreground.json 2>&1
            echo "trial evidence: $(grep -o '"summary":"[^"]*"' $O/status.json) windows $(grep -o '"appId":"[^"]*"' $O/windows.json | sort -u | tr '\n' ' ')"
        fi
        cp $STATE/sm.log $STATE/compositor.log $O/ 2>/dev/null
        trap - EXIT INT TERM
        rm -f $STATE/enabled
        revert
    else
        n=0; while [ -d /tmp/lumatv-recover.lock ] && [ $n -lt 90 ]; do sleep 1; n=$((n+1)); done
        echo "enabled: LumaTV's compositor stays live (recovery done after ${n}s more); 'cutover.sh disable' restores LG's"
    fi
    ;;
disable)
    rm -f $STATE/enabled
    revert
    ;;
revert) revert ;;
*) sed -n 2,16p $0 ;;
esac
