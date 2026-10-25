#!/bin/sh
# C4 trial cutover, executed ON THE SET as /tmp/lumatv/cutover.sh. LumaTV's compositor becomes the live one for a
# bounded trial with LG's compositor kept installed: the supervisor (lumatv-sm) is bind-mounted over
# /usr/bin/surface-manager and the unit restarted; on any failure the supervisor itself execs LG's binary, and at
# the end of the trial this script restores LG's. The early boot hook that would re-apply the mount on every boot
# is tools/lumatv's (not here): until it exists, LG's compositor is back after a reboot.
#
#   cutover.sh install     copy the bundle to /var/lib/lumatv/compositor, record the fingerprint, reset failures
#   cutover.sh trial SECS  mount, restart the unit, watch for SECS seconds, restore LG's (default 120)
#   cutover.sh status      what is live now
#   cutover.sh revert      unmount and restart LG's compositor
STATE=/var/lib/lumatv/compositor
D=/tmp/lumatv
LG=/usr/bin/surface-manager

is_mounted() { grep -q " $LG " /proc/self/mountinfo; }

revert() {
    if is_mounted; then umount "$LG" || umount -l "$LG"; fi
    systemctl restart surface-manager-daemon
    echo "revert: $(systemctl is-active surface-manager-daemon); mounted: $(is_mounted && echo yes || echo no); exe: $(readlink /proc/$(pgrep -x surface-manager | head -1)/exe 2>/dev/null)"
    sleep 30
    rm -f /tmp/var/log/reports/librdx/* 2>/dev/null
}

case "${1:-help}" in
install)
    mkdir -p $STATE/bin $STATE/lib
    cp $D/bin/lumatv-compositor $D/bin/lumatv-sm $STATE/bin/
    cp $D/lib/* $STATE/lib/
    chmod +x $STATE/bin/*
    $STATE/bin/lumatv-sm --print-fingerprint > $STATE/fingerprint
    echo 0 > $STATE/failures
    rm -f $STATE/escaped $STATE/last-fallback
    touch $STATE/enabled
    echo "installed: fingerprint $(cat $STATE/fingerprint); $(du -sh $STATE | cut -f1)"
    ;;
status)
    echo "mounted over $LG: $(is_mounted && echo yes || echo no)"
    echo "surface-manager-daemon: $(systemctl is-active surface-manager-daemon) pid $(pgrep -x surface-manager | head -1)"
    echo "lumatv-sm: $(pgrep -f lumatv-sm | head -1 || echo none); compositor: $(pgrep -f 'bin/lumatv-compositor' | head -1 || echo none)"
    ls -la $STATE 2>/dev/null | grep -vE 'bin|lib'
    tail -5 $STATE/sm.log 2>/dev/null
    ;;
trial)
    secs=${2:-120}
    [ -f $STATE/enabled ] || { echo "run install first"; exit 1; }
    trap revert EXIT INT TERM
    mount --bind $STATE/bin/lumatv-sm $LG
    echo "mounted; restarting the unit"
    systemctl restart surface-manager-daemon
    t=0
    while [ $t -lt $secs ]; do
        sleep 5; t=$((t+5))
        sm=$(pgrep -f 'lumatv-sm|surface-manager' | head -1)
        echo "t=${t}s unit=$(systemctl is-active surface-manager-daemon) exe=$(readlink /proc/$sm/exe 2>/dev/null) compositor=$(pgrep -f 'bin/lumatv-compositor' | head -1 || echo none) mounted=$(is_mounted && echo yes || echo no)"
        tail -2 $STATE/sm.log 2>/dev/null | sed 's/^/   /'
        if ! is_mounted; then echo "the supervisor fell back to LG's compositor: $(cat $STATE/last-fallback 2>/dev/null)"; break; fi
    done
    trap - EXIT INT TERM
    revert
    ;;
revert) revert ;;
*) sed -n 2,12p $0 ;;
esac
