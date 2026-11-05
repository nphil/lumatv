#!/bin/sh
# C4 cutover, executed ON THE SET as /tmp/lumatv/cutover.sh. LumaTV's compositor becomes the live one with LG's
# compositor kept installed: the supervisor (lumatv-sm) is bind-mounted over /usr/bin/surface-manager and the unit
# restarted; on any failure the supervisor itself execs LG's binary at that path and brings LG's Home back. The
# early boot hook (tools/lumatv compositor-boot, tools/compositor.sh) places the mount at boot while a persistent
# enabled flag exists and the firmware is a build the gate passed on. The checks are tools/compositor.sh's (deployed
# beside this script as /tmp/lumatv/compositor.sh); nothing here is a second copy of them.
#
#   cutover.sh install     copy the bundle to /var/lib/lumatv/compositor and record what the boot checks compare with:
#                          the supervisor's fingerprint, LG's binary, the SHA-256 of LG's binary and libraries, the
#                          bundle's files, verified.list; reset failures (not enabled)
#   cutover.sh probe       ask ls-hubd for the real bus names from the real path with LG's compositor running: tells
#                          whether the role allows them to this file (exit 0) or not (exit 2); ~2 s, nothing restarts
#   cutover.sh trial SECS  mount, restart the unit, watch for SECS seconds, restore LG's (default 120); a session:
#                          the flag names this boot and is gone when the trial ends, on any firmware
#   cutover.sh enable      persistent flag, mount, restart the unit and leave LumaTV's compositor live (also after
#                          reboots); refused on a firmware build that is not on verified.list (exit 5); exit 9 when
#                          the supervisor fell back to LG's compositor during the enable (the flag is removed)
#   cutover.sh enable --session [--max-minutes N]   the same for this boot only (what the gate runs on a build it has
#                          not passed on), with a safety timer: a detached job runs `disable` after N minutes (whole
#                          minutes, 1 to 1440, default 15) unless the session has ended by then, so that a test whose
#                          workstation dies does not leave the set on LumaTV's compositor. A trial's timer is its SECS
#                          plus 5 minutes, for the evidence and the revert.
#   cutover.sh disable     unmount, restart LG's compositor, bring Home back; the enabled flag is removed; the timer
#                          is cancelled
#   cutover.sh status      what is live now, and the timer
#   cutover.sh revert      restore stock UI and disable LumaTV; the timer is cancelled
#   cutover.sh firmware check    hash LG's binary and libraries and say whether this build is on verified.list (exit 0/5)
#   cutover.sh firmware record   print the verified.list entry for this build, for the gate to add once it has passed
#   cutover.sh boot-check  what the boot hook would decide now (JSON), changing nothing
STATE=${LUMATV_STATE:-/var/lib/lumatv/compositor}
D=${LUMATV_D:-/tmp/lumatv}
LG=${LUMATV_LG:-/usr/bin/surface-manager}

COMP=$STATE
COMP_LG=$LG
COMP_MOUNTINFO=${LUMATV_MOUNTINFO:-/proc/self/mountinfo}
COMP_LOCK=${LUMATV_RECOVER_LOCK:-/tmp/lumatv-recover.flock}
LUMATV_TOOL=${LUMATV_TOOL:-/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv}
[ -f $D/compositor.sh ] || { echo "deploy first: $D/compositor.sh is missing"; exit 1; }
log_line() { :; }
. $D/compositor.sh

is_mounted() { comp_mounted; }

# The safety timer. Its job is this script run as `safety-run MINUTES`, detached from the caller (setsid, and no
# descriptor of the caller's: a PTY that a background job holds open never reaches its end of file, and the job has to
# outlive the SSH session that may die). Its record is one line, "PID END MINUTES", with END in seconds of /proc/uptime
# (no clock step moves it); the job writes it itself, so that a pid changed by setsid's fork cannot be the wrong one.
SELF=$0
case $SELF in /*) ;; *) SELF=$PWD/$SELF ;; esac
SAFETY=$STATE/safety-timer
SAFETY_DEFAULT=15
SAFETY_MINUTE=${LUMATV_SAFETY_MINUTE:-60}   # seconds in one of the timer's minutes; the host test shortens it

now_s() { read -r n_up _ < /proc/uptime; echo "${n_up%.*}"; }
safety_log() { python3 "$D/logrelay.py" send lumatv-sm "safety: [$(date +%H:%M:%S)] $*" 2>/dev/null || :; }
safety_owner() { o_pid=""; { read -r o_pid _ < $SAFETY; } 2>/dev/null; echo "$o_pid"; }

# The pid the record names, when that process is a timer job: a job that was killed leaves its record behind, and the
# pid may be any process by now.
safety_pid() {
    p_pid=$(safety_owner)
    case "$p_pid" in ''|*[!0-9]*) return 1 ;; esac
    tr '\0' ' ' < /proc/$p_pid/cmdline 2>/dev/null | grep -q ' safety-run [0-9]' || return 1
    echo "$p_pid"
}

safety_cancel() {
    c_pid=$(safety_pid)
    rm -f $SAFETY
    [ -n "$c_pid" ] || return 0
    kill $(pgrep -P $c_pid 2>/dev/null) $c_pid 2>/dev/null
    safety_log "cancelled (pid $c_pid)"
    echo "safety timer cancelled"
}

safety_state() {
    if s_pid=$(safety_pid); then
        set -- $(cut -d ' ' -f 2,3 $SAFETY)
        echo "pid $s_pid, $(( ($1 - $(now_s) + SAFETY_MINUTE - 1) / SAFETY_MINUTE )) of $2 min left"
    else
        echo none
    fi
}

# Starts the job and waits for its record: a session starts only with a timer behind it.
safety_start() {
    safety_cancel > /dev/null
    if command -v setsid > /dev/null 2>&1; then
        setsid sh "$SELF" safety-run "$1" < /dev/null > /dev/null 2>&1 &
    else
        sh "$SELF" safety-run "$1" < /dev/null > /dev/null 2>&1 &
    fi
    s0=$(now_s)
    while ! safety_pid > /dev/null && [ $(( $(now_s) - s0 )) -lt 3 ]; do sleep 0.1; done
    safety_pid > /dev/null
}

revert() {
    safety_cancel
    "$LUMATV_TOOL" compositor off
}

# The bundle's files as install saw them: size of every file under bin/ and lib/, and the SHA-256 of the ones under
# bin/ (the 37 MB of lib/ is checked by size alone at a boot).
write_bundle_manifest() {
    : > $STATE/bundle.manifest.new
    for f in $STATE/bin/* $STATE/lib/*; do
        [ -f "$f" ] || continue
        rel=${f#$STATE/}
        size=$(stat -c %s "$f")
        case "$rel" in bin/*) sum=$(comp_sha256 "$f") ;; *) sum=- ;; esac
        echo "$size ${sum:--} $rel" >> $STATE/bundle.manifest.new
    done
    mv $STATE/bundle.manifest.new $STATE/bundle.manifest
}

case "${1:-help}" in
install)
    # LG's binary has to be the one at its path: with the mount in place the path shows the supervisor.
    if is_mounted; then echo "LumaTV's compositor is mounted over LG's; run disable first"; exit 1; fi
    [ -d "$D/assets/ui" ] || { echo "deploy first: $D/assets/ui is missing"; exit 1; }
    [ -f "$D/feeder/lumatv-gfn-feeder" ] || { echo "deploy first: $D/feeder is missing"; exit 1; }
    storage_guard "$STATE" "$D/bin" "$D/lib" "$D/assets/ui" "$D/recover-home.sh" "$D/logrelay.py" "$D/verified.list" || { echo "$STORAGE_WHY"; exit 1; }
    mkdir -p $STATE/bin $STATE/lib
    cp $D/bin/lumatv-compositor $D/bin/lumatv-sm $STATE/bin/
    # The Rust shell (a bundle built with `ui`) is looked for beside the compositor, which the supervisor runs from here; one left by an
    # earlier bundle is not kept when this one has none.
    if [ -f $D/bin/lumatv-ui ]; then cp $D/bin/lumatv-ui $STATE/bin/; else rm -f $STATE/bin/lumatv-ui; fi
    # The camera feeder lives in the app's data directory, not the bundle's: feeder.sh stages it there when `lumatv feeder install` has been run.
    if [ -f $D/feeder/lumatv-gfn-feeder ]; then
        sh $D/feeder/feeder.sh update $D/feeder/lumatv-gfn-feeder "${LUMATV_DATA_DIR:-/var/lib/lumatv}" || { echo "feeder update failed"; exit 1; }
        echo "feeder: $(sha256sum $D/feeder/lumatv-gfn-feeder | cut -d' ' -f1)"
    fi
    cp $D/recover-home.sh $STATE/bin/recover-home.sh
    cp $D/logrelay.py $STATE/bin/logrelay.py
    cp $D/lib/* $STATE/lib/
    chmod +x $STATE/bin/*
    # Remove the old shell so it cannot be auto-started.
    rm -rf "$STATE/shell"
    mkdir -p "$STATE/assets"
    rm -rf "$STATE/assets/ui.new"
    cp -R "$D/assets/ui" "$STATE/assets/ui.new" || exit 1
    rm -rf "$STATE/assets/ui"
    mv "$STATE/assets/ui.new" "$STATE/assets/ui" || exit 1
    # The builds the gate passed on come with the deploy (compositor/firmware/verified.list); a set that was never
    # given one keeps what it has, or an empty list, which verifies nothing.
    if [ -f $D/verified.list ]; then cp $D/verified.list $STATE/verified.list
    elif [ ! -f $STATE/verified.list ]; then echo "# no build has passed the gate yet" > $STATE/verified.list; fi
    $STATE/bin/lumatv-sm --print-fingerprint > $STATE/fingerprint
    stat -c '%s %Y' $LG > $STATE/lg-binary
    comp_fw_manifest > $STATE/manifest
    write_bundle_manifest
    echo 3 > $STATE/supervisor-api
    # 2: this bundle's compositor reads <data dir>/boot-direct (tools/lumatv boot-direct) for its UI start only, and its supervisor's escape (Home
    # held 10 s) is the one way to LG's Home. The tool places the Home stub only on a bundle that says so: marker 1 is the compositor that opened
    # LG's Home on a Home hold of 5 s, through a tool verb that is gone.
    echo 2 > $STATE/boot-direct-api
    echo 0 > $STATE/failures
    rm -f $STATE/escaped $STATE/last-fallback $STATE/enabled $STATE/running $STATE/fw-verified $STATE/fw-unverified
    fw=$(comp_fw_check force)
    echo "installed (not enabled): fingerprint $(cat $STATE/fingerprint); LG binary $(cat $STATE/lg-binary); $(du -sh $STATE | cut -f1)"
    echo "firmware: $fw"
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
    echo "flag: $(comp_flag); firmware: $(comp_fw_state); supervisor-api: $(cat $STATE/supervisor-api 2>/dev/null || echo none)"
    echo "safety timer: $(safety_state)"
    ls -la $STATE 2>/dev/null | grep -vE 'bin|lib'
    comp_log lumatv-sm | tail -5
    ;;
firmware)
    case "${2:-check}" in
    check) out=$(comp_fw_check force); rc=$?; echo "$out"; exit $rc ;;
    record) [ -x $STATE/bin/lumatv-sm ] || { echo "run install first"; exit 1; }; comp_fw_block ;;
    *) echo "firmware check | record"; exit 1 ;;
    esac
    ;;
boot-check)
    comp_check_json; echo
    ;;
trial|enable)
    cmd=$1; shift
    secs=120; mode=persistent; mins=""
    case "$cmd" in
    trial)
        secs=${1:-120}; mode=session
        case "$secs" in ''|*[!0-9]*|0?*) echo "trial [SECS]"; exit 1 ;; esac ;;
    enable)
        while [ $# -gt 0 ]; do
            case "$1" in
            --session) mode=session ;;
            --max-minutes)
                shift; mins=${1:-}
                case "$mins" in ''|*[!0-9]*|0?*|0) echo "--max-minutes N: whole minutes, 1 to 1440"; exit 1 ;; esac
                [ "$mins" -le 1440 ] || { echo "--max-minutes N: whole minutes, 1 to 1440"; exit 1; } ;;
            *) echo "enable [--session [--max-minutes N]]"; exit 1 ;;
            esac
            shift
        done
        [ -z "$mins" ] || [ $mode = session ] || { echo "--max-minutes goes with --session: a persistent enable has no timer"; exit 1; } ;;
    esac
    [ -x "$LUMATV_TOOL" ] || { echo "LumaTV tool is not installed: $LUMATV_TOOL"; exit 1; }
    if [ $mode = session ]; then
        if [ $cmd = trial ]; then mins=$(( (secs + 59) / 60 + 5 )); else mins=${mins:-$SAFETY_DEFAULT}; fi
        if ! safety_start $mins; then
            safety_cancel > /dev/null
            echo "the safety timer did not start; nothing was mounted"
            exit 8
        fi
        echo "safety timer: this session ends by itself after $mins min if it is still live ('cutover.sh disable' cancels the timer)"
    fi
    if [ "$cmd" = trial ]; then trap 'revert' EXIT; trap 'exit 1' INT TERM; fi
    if "$LUMATV_TOOL" compositor on "$mode"; then
        :
    else
        switch_rc=$?
        safety_cancel > /dev/null
        [ "$cmd" = trial ] && trap - EXIT INT TERM
        exit "$switch_rc"
    fi
    t=0
    while [ $t -lt $secs ]; do
        sleep 5; t=$((t+5))
        sm=$(pgrep -x surface-manager | head -1)
        echo "t=${t}s unit=$(systemctl is-active surface-manager-daemon) exe=$(readlink /proc/$sm/exe 2>/dev/null) compositor=$(pgrep -f 'bin/lumatv-compositor' | head -1 || echo none) mounted=$(is_mounted && echo yes || echo no) wam=$(systemctl is-active webapp-mgr)"
        comp_log lumatv-sm | tail -2 | sed 's/^/   /'
        if ! is_mounted; then echo "the supervisor fell back to LG's compositor: $(cat $STATE/last-fallback 2>/dev/null)"; break; fi
        if [ $cmd = enable ] && [ $t -ge 30 ]; then break; fi
    done
    if [ $cmd = trial ]; then
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
        comp_log lumatv-sm > $O/sm.log; comp_log lumatv-compositor > $O/compositor.log
        trap - EXIT INT TERM
        revert || exit 1
    else
        n=0; while comp_recovering && [ $n -lt 90 ]; do sleep 1; n=$((n+1)); done
        # The watch above ends at the first fallback too: LG's compositor is then the live one, and an enable that
        # leaves its flag behind would try LumaTV's again at the next boot.
        if ! is_mounted; then
            revert
            echo "enable failed: the supervisor fell back to LG's compositor ($(cat $STATE/last-fallback 2>/dev/null))"
            exit 9
        fi
        echo "enabled ($mode): LumaTV's compositor stays live (recovery done after ${n}s more); 'cutover.sh disable' restores LG's"
    fi
    ;;
disable)
    revert
    ;;
revert) revert ;;
safety-run)
    # Not typed by hand: the job safety_start detaches. The shell has parsed this whole statement before it runs, so a
    # deploy over this file while the job sleeps changes nothing for it.
    trap '' HUP
    mins=${2:-$SAFETY_DEFAULT}
    end=$(( $(now_s) + mins * SAFETY_MINUTE ))
    echo "$$ $end $mins" > $SAFETY
    while [ "$(now_s)" -lt "$end" ]; do
        [ "$(safety_owner)" = "$$" ] || exit 0
        left=$(( end - $(now_s) ))
        [ $left -le 20 ] || left=20
        sleep $left
    done
    [ "$(safety_owner)" = "$$" ] || exit 0
    # Claimed: the disable below must not cancel the job it runs under.
    rm -f $SAFETY
    if [ "$(comp_flag)" = session ]; then
        safety_log "$mins min are up with the session still live: disable"
        python3 "$D/logrelay.py" output lumatv-sm sh "$SELF" disable
        safety_log "disable exit $?"
    else
        safety_log "$mins min are up; the session had already ended (mounted $(is_mounted && echo yes || echo no), flag $(comp_flag))"
    fi
    exit 0
    ;;
*) sed -n '2,/^[^#]/{/^#/p;}' $0 ;;
esac
