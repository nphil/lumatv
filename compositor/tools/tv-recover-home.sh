#!/bin/sh
# WebAppMgr keeps a connection to the compositor that went away. Recovery resets its
# failed unit, waits for the replacement bus, restarts the existing GUI services,
# and launches Home. Stock recovery first removes only LumaTV-owned UI shadows.
# Installed as bin/recover-home.sh; runs on the TV, never on the build host.
WAM=webapp-mgr.service
SM=surface-manager-daemon.service
LOG_RELAY=${LUMATV_LOG_RELAY:-$(dirname "$0")/logrelay.py}
CAP=/tmp/lumatv-recover.png
STOCK_HOLD=/var/lib/lumatv/compositor/stock-ui-held
LOCK=/tmp/lumatv-recover.flock

clock_s() {
    read -r uptime rest < /proc/uptime || return 1
    printf '%s\n' "${uptime%%.*}"
}
started=$(clock_s) || exit 1
# Reserve the last four seconds for a final Home request. These are userspace
# bounds, not a guarantee against uninterruptible kernel or scheduling stalls.
work_deadline=$((started + 116))
remaining() { now=$(clock_s) || return 1; printf '%s\n' "$((work_deadline - now))"; }
has_budget() { left=$(remaining) && [ "$left" -gt 0 ]; }
bounded() {
    limit=$1; shift
    left=$(remaining) || return 124
    [ "$left" -gt 0 ] || return 124
    [ "$limit" -le "$left" ] || limit=$left
    timeout -k 1 "$limit" "$@" 8>&-
}
say() {
    echo "recover: $*"
    bounded 3 python3 "$LOG_RELAY" send lumatv-sm "recover: $*" 2>/dev/null || :
}
ls_ok() { grep -q '"returnValue"[[:space:]]*:[[:space:]]*true' 8>&-; }
ours() { grep -q ' /usr/bin/surface-manager ' /proc/self/mountinfo 8>&- && pgrep -f 'bin/lumatv-compositor' 8>&- >/dev/null; }
home_running() { bounded 10 luna-send -n 1 luna://com.palm.webappmanager/listRunningApps '{}' 2>/dev/null | grep -q '"com.webos.app.home"' 8>&-; }
pause() { has_budget && sleep 1; }

failed=0
stock=0
[ "${1:-}" != "--stock" ] && [ ! -e "$STOCK_HOLD" ] || stock=1
stock_yield() {
    if [ "$stock" = 0 ] && [ -e "$STOCK_HOLD" ]; then
        say "stock recovery requested; yielding the serialized handoff"
        exit 0
    fi
}

stock_generation() { cat "$STOCK_HOLD" 8>&- 2>/dev/null || :; }
checkpoint() {
    stock_yield
    [ "$stock" = 0 ] || [ "$(stock_generation)" = "$generation" ]
}

launch_home() {
    if launch_reply=$(timeout -k 1 3 luna-send -n 1 luna://com.webos.applicationManager/launch '{"id":"com.webos.app.home"}' 8>&- 2>/dev/null) && printf '%s\n' "$launch_reply" | ls_ok; then
        say "Home launch accepted; display acceptance remains unverified"
    else
        failed=1
        say "Home launch refused, failed or timed out"
    fi
}

# The panel's OLED pixel brightness a compositor killed under a dim (the saver's) left behind: its journal, paneldim.json, holds the
# user's value, the value it wrote and the picture mode and input they belong to. The supervisor's SIGKILL of a hung compositor is the case;
# the helper's watch and a restarted LumaTV do the same, and each puts back only a value that is still the one written, so it is safe in
# any order. Only the settings service is called.
PANELDIM_JOURNAL=${LUMATV_PANELDIM_JOURNAL:-/var/lib/lumatv/paneldim.json}
journal_num() { printf '%s' "$1" | sed -n "s/.*\"$2\":\([0-9][0-9]*\).*/\1/p" 8>&-; }
journal_str() { printf '%s' "$1" | sed -n "s/.*\"$2\":\"\([^\"]*\)\".*/\1/p" 8>&-; }
restore_panel() {
    [ -r "$PANELDIM_JOURNAL" ] || return 0
    pj=$(cat "$PANELDIM_JOURNAL" 8>&-) || return 0
    orig=$(journal_num "$pj" orig); written=$(journal_num "$pj" written); cpid=$(journal_num "$pj" pid)
    [ -n "$orig" ] && [ -n "$written" ] || return 0
    if [ -n "$cpid" ] && grep -q lumatv-compositor "/proc/$cpid/cmdline" 2>/dev/null; then return 0; fi
    mode=$(journal_str "$pj" pictureMode); input=$(journal_str "$pj" input)
    dim=
    [ -z "$mode" ] || [ -z "$input" ] || dim=",\"dimension\":{\"pictureMode\":\"$mode\",\"input\":\"$input\"}"
    now=$(bounded 8 luna-send -n 1 luna://com.webos.service.settings/getSystemSettings "{\"category\":\"picture\",\"keys\":[\"backlight\"]$dim}" 2>/dev/null | sed -n 's/.*"backlight"[^0-9]*\([0-9][0-9]*\).*/\1/p')
    if [ -z "$now" ]; then say "panel brightness not read; the journal stays for the helper"; return 0; fi
    if [ "$now" != "$written" ]; then
        rm -f "$PANELDIM_JOURNAL"
        say "panel brightness is $now, not the $written a dim wrote: left alone"
        return 0
    fi
    if bounded 8 luna-send -n 1 luna://com.webos.service.settings/setSystemSettings "{\"category\":\"picture\",\"settings\":{\"backlight\":\"$orig\"}$dim}" 2>/dev/null | ls_ok; then
        rm -f "$PANELDIM_JOURNAL"
        say "panel brightness put back to $orig from the dim's $written"
    else
        say "panel brightness not put back; the journal stays for the helper"
    fi
}

# Hold the lock through the complete recovery, including visibility checks. A
# later stock request may run sequentially: an already-listed Home can still be
# the cached transparent stub and must not bypass the restored stock GUI restart.
exec 8>>"$LOCK" || exit 1
wait_lock=10
[ "$stock" = 0 ] || wait_lock=30
if ! flock -x -w "$wait_lock" 8; then
    say "another recovery still owns the handoff"
    exit 1
fi
stock_yield
recovery_pass() {
if [ "$stock" = 1 ]; then
    generation=$(stock_generation)
    restore_panel
    n=0
    while has_budget && grep -q ' /usr/bin/surface-manager ' /proc/self/mountinfo 8>&- && [ "$n" -lt 10 ]; do
        pause; n=$((n+1))
    done
    if bounded 30 /media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv stock-ui-restore; then
        say "owned stock UI restoration completed"
    else
        failed=1
        say "owned stock UI restoration failed or timed out; attempting Home"
    fi
    checkpoint || return 2
fi

n=0
while has_budget && [ "$n" -lt 5 ]; do
    checkpoint || return 2
    [ "$(bounded 3 systemctl is-active "$SM" 2>/dev/null)" != active ] || break
    pause; n=$((n+1))
done
say "$SM wait: $n attempts"
checkpoint || return 2
# A live unit and an old socket do not prove the replacement compositor has
# registered its bus name. Keep the existing bus-readiness check.
n=0
while has_budget && [ "$n" -lt 3 ]; do
    checkpoint || return 2
    bounded 5 luna-send -n 1 luna://com.webos.surfacemanager/getForegroundAppInfo '{}' 2>/dev/null | ls_ok && break
    pause; n=$((n+1))
done
say "compositor bus wait: $n attempts"
checkpoint || return 2
has_budget && sleep 2

# Only services already running are try-restarted; no inactive booster or input
# method is started. Check stock intent between bounded service operations.
bounded 10 systemctl reset-failed "$WAM" 2>/dev/null || :
checkpoint || return 2
bounded 20 systemctl restart "$WAM" || failed=1
checkpoint || return 2
bounded 20 systemctl try-restart booster.service maliit-server.service 2>/dev/null || :
checkpoint || return 2
n=0
while has_budget && [ "$n" -lt 3 ]; do
    checkpoint || return 2
    bounded 10 luna-send -n 1 luna://com.palm.webappmanager/listRunningApps '{}' 2>/dev/null | ls_ok && break
    pause; n=$((n+1))
done
say "WebAppMgr wait: $n attempts"
checkpoint || return 2
pause
checkpoint || return 2
# This final bounded request is attempted even if an earlier phase exhausted its
# work budget. Service errors must not turn the emergency path into a no-op.
launch_home
checkpoint || return 2
n=0
if ours; then
    while has_budget && [ "$n" -lt 3 ]; do
        checkpoint || return 2
        home_running && break
        pause; n=$((n+1))
    done
    if home_running; then say "Home is listed under LumaTV"; else say "Home listing unavailable"; fi
else
    b=
    while has_budget && [ "$n" -lt 3 ]; do
        checkpoint || return 2
        b=$(bounded 10 luna-send -n 1 luna://com.webos.service.capture/executeOneShot "{\"path\":\"$CAP\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":320,\"height\":180}" 2>/dev/null | tr ',' '\n' 8>&- | grep writtenBytes 8>&- | tr -dc '0-9' 8>&-)
        [ -n "$b" ] && [ "$b" -gt 4096 ] && break
        pause; n=$((n+1))
    done
    say "display capture: ${b:-unavailable} bytes; not visual Home verification"
fi
checkpoint || return 2
return 0
}

# A newer fallback may arrive after a previous stock restore. Repeat the ordered
# recovery for its generation without extending the original work deadline.
while :; do
    if recovery_pass; then break; fi
    if ! has_budget; then
        failed=1
        launch_home
        break
    fi
done
rm -f "$CAP" 8>&-
exit "$failed"
