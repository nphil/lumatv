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
#   cutover.sh install --delta   the same for a deploy tree that holds only the parts that changed: a part it lacks keeps the installed copy
#   cutover.sh ui-swap     replace bin/lumatv-ui and assets/ui of the deploy tree under the live compositor and restart only the shell client
#                          (`ctl shell restart`): the app in front and the video plane stay as they are; the previous shell comes back when the
#                          new one does not
#   cutover.sh staged status | commit | discard | boot | accept | apply [--standby | --now]
#                          the staged install: the changed parts wait on the set (staged/) and are applied at the next boot before the
#                          compositor starts (`boot`), or by `apply --now` when the panel is on (Active) with Home in front and no game running;
#                          `apply --standby` (Active Standby) is for a proof run by hand and nothing triggers it. An apply is disable, install
#                          --delta, probe, enable and the video check, and any failure puts the previous files back; exit 10 means not now, 12 no
#                          picture with the new files or the previous ones (files back; a reboot is the owner's call). A boot apply stays
#                          pending, with the previous files kept: the next boot puts them back on the supervisor's failure records, and `accept`
#                          (panel on, Home in front, no failure record, a video check that shows a picture) drops them
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
#                          is cancelled. The sysbus repair (lumatv sysbus) and the Rust gateway on port 3001 (lumatv
#                          ssgw) are left as the owner set them.
#   cutover.sh status      what is live now, and the timer
#   cutover.sh revert      disable, and take the sysbus repair and the Rust gateway off too (a return to LG's UI, not a
#                          step of an install); the timer is cancelled
#   cutover.sh firmware check    hash LG's binary and libraries and say whether this build is on verified.list (exit 0/5)
#   cutover.sh firmware record   print the verified.list entry for this build, for the gate to add once it has passed
#   cutover.sh boot-check  what the boot hook would decide now (JSON), changing nothing
#
# enable and disable end with a video check (tools/compositor.sh comp_video_check, skipped while the panel is off): exit 12
# when no picture gets from LG's decoder to the panel. The switch is kept, since the set needs a reboot (`reboot`), the only
# recovery known (2026-11-17).
# luna-send answers nothing without a controlling terminal, so over a plain `ssh set cutover.sh ...` the Home recovery and
# the video check fail and an enable reverts to LG's compositor (2026-11-18). On the set (its own /usr/bin/luna-send), a run
# without one goes again under util-linux script's pty; -e keeps the exit status (exit 12 and the rest).
if [ ! -t 0 ] && [ -z "${LUMATV_CUTOVER_PTY:-}" ] && [ -x /usr/bin/luna-send ] && command -v script > /dev/null 2>&1; then
    LUMATV_CUTOVER_PTY=1; export LUMATV_CUTOVER_PTY
    cmd="sh '$0'"
    for a in "$@"; do cmd="$cmd '$(printf %s "$a" | sed "s/'/'\\\\''/g")'"; done
    exec script -q -e -c "$cmd" /dev/null
fi

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

video_check() {
    if comp_video_check "$D/feeder/check.h264"; then
        echo "video check: $COMP_VC ($COMP_VC_WHY)"
        return 0
    fi
    echo "VIDEO CHECK FAILED after $1: $COMP_VC_WHY. Video stays black in every app until the set reboots: run reboot. Readings: $COMP_VC_OUT/readings"
    exit 12
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

# Copies SRC to DEST unless the bytes are the same: a delta carries support files the bundle already has.
put_file() { cmp -s "$1" "$2" 2>/dev/null || cp "$1" "$2"; }

# install_bundle SRC [--delta]: puts the bundle in SRC on the set and records what the boot checks compare with (the `install` verb).
# With --delta, SRC holds only the parts that changed (what a staged set carries): a part it does not have keeps the installed copy, where
# a whole bundle that lacks one is refused (or, for the Rust shell, removes the old one).
install_bundle() {
    src=$1; delta=${2:-}
    case "$delta" in ""|--delta) ;; *) echo "install [--delta]"; return 1 ;; esac
    # LG's binary has to be the one at its path: with the mount in place the path shows the supervisor.
    if is_mounted; then echo "LumaTV's compositor is mounted over LG's; run disable first"; return 1; fi
    if [ -z "$delta" ]; then
        [ -d "$src/assets/ui" ] || { echo "deploy first: $src/assets/ui is missing"; return 1; }
        [ -f "$src/feeder/lumatv-gfn-feeder" ] || { echo "deploy first: $src/feeder is missing"; return 1; }
    fi
    storage_guard "$STATE" "$src/bin" "$src/lib" "$src/assets/ui" "$src/recover-home.sh" "$src/logrelay.py" "$src/verified.list" || { echo "$STORAGE_WHY"; return 1; }
    mkdir -p $STATE/bin $STATE/lib
    if [ -z "$delta" ]; then
        cp $src/bin/lumatv-compositor $src/bin/lumatv-sm $STATE/bin/ || return 1
    else
        for f in lumatv-compositor lumatv-sm; do
            [ ! -f $src/bin/$f ] || put_file $src/bin/$f $STATE/bin/$f || return 1
        done
    fi
    # The Rust shell (a bundle built with `ui`) is looked for beside the compositor, which the supervisor runs from here; one left by an
    # earlier bundle is not kept when this one has none.
    if [ -f $src/bin/lumatv-ui ]; then put_file $src/bin/lumatv-ui $STATE/bin/lumatv-ui || return 1
    elif [ -z "$delta" ]; then rm -f $STATE/bin/lumatv-ui; fi
    # The camera feeder lives in the app's data directory, not the bundle's: feeder.sh stages it there when `lumatv feeder install` has been run.
    if [ -f $src/feeder/lumatv-gfn-feeder ]; then
        sh $D/feeder/feeder.sh update $src/feeder/lumatv-gfn-feeder "${LUMATV_DATA_DIR:-/var/lib/lumatv}" || { echo "feeder update failed"; return 1; }
        echo "feeder: $(sha256sum $src/feeder/lumatv-gfn-feeder | cut -d' ' -f1)"
    fi
    if [ -f $src/recover-home.sh ]; then put_file $src/recover-home.sh $STATE/bin/recover-home.sh || return 1
    elif [ -z "$delta" ]; then cp $src/recover-home.sh $STATE/bin/recover-home.sh; fi
    if [ -f $src/logrelay.py ]; then put_file $src/logrelay.py $STATE/bin/logrelay.py || return 1
    elif [ -z "$delta" ]; then cp $src/logrelay.py $STATE/bin/logrelay.py; fi
    if [ -z "$delta" ]; then cp $src/lib/* $STATE/lib/
    else
        for f in $src/lib/*; do
            [ ! -f "$f" ] || put_file "$f" "$STATE/lib/${f##*/}" || return 1
        done
    fi
    # The loader is run directly, and cp onto an existing file keeps its old mode: a bundle staged without the exec bit (2026-11-21)
    # worked only on sets installed before, and would leave a fresh set unable to start the compositor.
    chmod +x $STATE/bin/* $STATE/lib/ld-linux-aarch64.so.1
    # Remove the old shell so it cannot be auto-started.
    rm -rf "$STATE/shell"
    if [ -z "$delta" ] || [ -d "$src/assets/ui" ]; then
        mkdir -p "$STATE/assets"
        rm -rf "$STATE/assets/ui.new"
        cp -R "$src/assets/ui" "$STATE/assets/ui.new" || return 1
        rm -rf "$STATE/assets/ui"
        mv "$STATE/assets/ui.new" "$STATE/assets/ui" || return 1
    fi
    # The builds the gate passed on come with the deploy (compositor/firmware/verified.list); a set that was never
    # given one keeps what it has, or an empty list, which verifies nothing.
    if [ -f $src/verified.list ]; then put_file $src/verified.list $STATE/verified.list
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
}

# ---------------------------------------------------------------- quiet UI swap and staged installs
# A change to the UI alone needs no compositor restart: ui_swap replaces the shell's binary and assets and restarts only the shell client
# (`ctl shell restart`), so the app in front and the video plane stay as they are. Whatever else the cutover installs is staged on the set and
# applied at the next boot, before the compositor starts (nothing is on the screen and no video is playing), or by `staged apply --now` when the
# panel is on with Home in front. There is no automatic trigger at standby: it is not proven not to wake the panel or cut into LG's panel care.
# A manifest with boot-only may only be applied by the early boot hook, never by a manual apply.
STAGED=$STATE/staged
DATA=${LUMATV_DATA_DIR:-/var/lib/lumatv}
HELPER_PAYLOAD="${LUMATV_TOOL%/*}/../payload/helper/lumatv-helper"
HELPER_BIN="$DATA/bin/lumatv-helper"
HOME_APP=com.webos.app.home
# Processes of the app a restart of the compositor would end (LumaGame).
BUSY_PATTERNS=${LUMATV_BUSY_PATTERNS:-"org.nphil.lumagame/"}
# The compositor ends itself when a restarted shell is not ready 40 s after it started (READY_TIMEOUT): the wait is shorter, so the previous shell can still be put back.
UI_WAIT=${LUMATV_UI_WAIT:-30}

# The journal is the record of what was applied or undone; the supervisor's log carries the same line.
staged_log() {
    mkdir -p "$STAGED"
    echo "[$(date '+%Y-%m-%d %H:%M:%S')] $*" >> "$STAGED/journal"
    tail -n 200 "$STAGED/journal" > "$STAGED/journal.new" && mv "$STAGED/journal.new" "$STAGED/journal"
    python3 "$D/logrelay.py" send lumatv-sm "staged: $*" 2>/dev/null || :
    echo "$*"
}

staged_power() {
    comp_vc_luna luna://com.webos.service.tvpower/power/getPowerState '{}' 2>/dev/null | tr -d '\r\n' | sed -n 's/.*"state"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p'
}

staged_foreground() {
    comp_vc_luna luna://com.webos.applicationManager/getForegroundAppInfo '{}' 2>/dev/null | tr -d ' \r\n' | sed -n 's/.*"appId":"\([^"]*\)".*/\1/p'
}

# The video plane is connected to the decoder and not muted: a picture is on the panel.
staged_video() {
    grep -q '^connect_src=VDEC' "$COMP_VC_SCALER" 2>/dev/null && grep -q '^mute=0' "$COMP_VC_SCALER" 2>/dev/null
}

# staged_decide MODE: whether a staged set may be applied now. Prints "apply" (exit 0) or "wait: WHY" (exit 10).
#   boot      LumaTV's compositor has not started yet: always
#   now       the panel is on (Active) with Home in front and no game or app that a restart would end running; with an app in front (video
#             playing among them), or any other power state, the answer is wait
#   standby   Active Standby only. Run by hand to prove that a compositor switch there does not wake the panel; nothing calls it by itself.
# A power state that cannot be read is wait: the set is never assumed to be asleep.
staged_decide() {
    case "$1" in
    boot) echo apply; return 0 ;;
    standby|now) ;;
    *) echo "wait: unknown mode $1"; return 10 ;;
    esac
    d_power=$(staged_power)
    if [ -z "$d_power" ]; then echo "wait: the power state is unknown"; return 10; fi
    if [ "$1" = standby ]; then
        if [ "$d_power" = "Active Standby" ]; then echo apply; return 0; fi
        echo "wait: the set is in $d_power, not Active Standby"; return 10
    fi
    if [ "$d_power" != Active ]; then echo "wait: the set is in $d_power; --now needs the panel on with Home in front (the next boot applies the set)"; return 10; fi
    d_fg=$(staged_foreground)
    if [ -z "$d_fg" ]; then echo "wait: the app in front is unknown"; return 10; fi
    if [ "$d_fg" != "$HOME_APP" ]; then
        if staged_video; then echo "wait: video is playing in $d_fg"; else echo "wait: $d_fg is in front"; fi
        return 10
    fi
    for d_pat in $BUSY_PATTERNS; do
        if pgrep -f "$d_pat" > /dev/null 2>&1; then echo "wait: a game or app that a restart would end is running ($d_pat)"; return 10; fi
    done
    echo apply
}

staged_boot_only() { grep -q '  boot-only$' "$1/manifest${2:-}" 2>/dev/null; }

staged_helper_put() {
    mkdir -p "$(dirname "$2")" &&
        cp "$1" "$2.staged" && chmod 755 "$2.staged" && cmp -s "$1" "$2.staged" &&
        mv -f "$2.staged" "$2" && cmp -s "$1" "$2"
}

staged_helper_ready() {
    sha256sum -c "$STAGED/judge/helper.sha256" >/dev/null 2>&1 || return 1
    h_status=$(timeout 5 "$LUMATV_TOOL" helper status 2>/dev/null | tr -d ' \r\n')
    printf '%s\n' "$h_status" | grep -q '"running":true' &&
        printf '%s\n' "$h_status" | grep -Eq '"pid":[1-9][0-9]*[,}]' &&
        ! printf '%s\n' "$h_status" | grep -q '"healthy":false'
}

# Where a file of a staged set lives on the set; nothing for a support file (the scripts, the video check's clip) that is not installed.
staged_installed() {
    case "$1" in
    bin/*|lib/*|assets/*|verified.list) echo "$STATE/$1" ;;
    recover-home.sh|logrelay.py) echo "$STATE/bin/$1" ;;
    feeder/lumatv-gfn-feeder) echo "$DATA/bin/lumatv-gfn-feeder" ;;
    feeder/lib/*) echo "$DATA/bin/lib/${1#feeder/lib/}" ;;
    *) return 1 ;;
    esac
}

# The installed copy of everything the staged set replaces goes to prev/ (in the layout install --delta reads), and what changes, as
# "file old -> new" (8 hex digits of the SHA-256), to changes. The assets are one unit: the whole installed tree is kept, since a file the new
# tree no longer has must come back too.
staged_backup() {
    rm -rf "$STAGED/prev" "$STAGED/changes"; mkdir -p "$STAGED/prev"
    : > "$STAGED/changes"
    while read -r b_sum b_rel; do
        [ -n "$b_rel" ] || continue
        if [ "$b_rel" = helper/lumatv-helper ]; then
            mkdir -p "$STAGED/prev/helper" || return 1
            [ ! -f "$HELPER_PAYLOAD" ] || cp -p "$HELPER_PAYLOAD" "$STAGED/prev/helper/payload" || return 1
            [ ! -f "$HELPER_BIN" ] || cp -p "$HELPER_BIN" "$STAGED/prev/helper/bin" || return 1
            echo "helper/lumatv-helper -> $(printf %s "$b_sum" | cut -c1-8)" >> "$STAGED/changes"
            continue
        fi
        b_inst=$(staged_installed "$b_rel") || continue
        case "$b_rel" in assets/*) continue ;; esac
        b_old=-
        if [ -f "$b_inst" ]; then
            mkdir -p "$STAGED/prev/$(dirname "$b_rel")"
            cp "$b_inst" "$STAGED/prev/$b_rel" || return 1
            b_old=$(comp_sha256 "$b_inst" | cut -c1-8)
        elif [ "$b_rel" = bin/lumatv-ui ]; then
            : > "$STAGED/prev/no-ui-bin" || return 1
        fi
        [ "$b_old" = "$(echo "$b_sum" | cut -c1-8)" ] || echo "$b_rel $b_old -> $(echo "$b_sum" | cut -c1-8)" >> "$STAGED/changes"
    done < "$STAGED/manifest"
    if [ -d "$STAGED/dep/assets/ui" ]; then
        mkdir -p "$STAGED/prev/assets" || return 1
        if [ -d "$STATE/assets/ui" ]; then
            cp -R "$STATE/assets/ui" "$STAGED/prev/assets/ui" || return 1
        else
            : > "$STAGED/prev/no-ui-assets" || return 1
        fi
        echo "assets/ui (tree)" >> "$STAGED/changes"
    fi
}

# A boot apply that is not confirmed yet keeps prev/ and flag.save: they are what its rollback puts back.
staged_clear() {
    rm -rf "$STAGED/manifest" "$STAGED/dep" "$STAGED/dep.new" "$STAGED/applying"
    [ -f "$STAGED/boot-pending" ] || rm -rf "$STAGED/prev" "$STAGED/flag.save"
}

# One cutover step of the staged set, run as the staged script itself with its own files.
staged_step() {
    s_label=$1; shift
    LUMATV_D="$STAGED/dep" sh "$STAGED/dep/cutover.sh" "$@" > "$STAGED/step.out" 2>&1; s_rc=$?
    staged_log "step $s_label: exit $s_rc: $(tail -n 2 "$STAGED/step.out" | tr '\n' ' ' | cut -c1-240)"
    return $s_rc
}

# What the video check of the last staged_step saw: picture, none (exit 12) or unchecked (it did not run, or the step failed otherwise).
staged_seen() {
    if [ "$s_rc" -eq 12 ]; then echo none
    elif [ "$s_rc" -eq 0 ] && grep -q '^video check: picture' "$STAGED/step.out" 2>/dev/null; then echo picture
    else echo unchecked; fi
}

# Puts the files of prev/ back and, outside the boot (which mounts by itself), brings the previous build up again. 0 when the set is as before;
# R_SEEN is what the video check said of the previous build, up again (picture, none or unchecked).
staged_restore() {
    r_mode=$1; r_live=$2; R_SEEN=unchecked
    if comp_mounted; then
        staged_step "restore disable" disable; r_rc=$?
        # 12: LG's compositor is back and shows no picture; the files still go back.
        if [ "$r_rc" -ne 0 ] && [ "$r_rc" -ne 12 ]; then return 1; fi
    fi
    if [ -d "$STAGED/prev/helper" ]; then
        for r_copy in payload bin; do
            case "$r_copy" in payload) r_target=$HELPER_PAYLOAD ;; bin) r_target=$HELPER_BIN ;; esac
            if [ -f "$STAGED/prev/helper/$r_copy" ]; then
                staged_helper_put "$STAGED/prev/helper/$r_copy" "$r_target" || return 1
            else
                rm -f "$r_target" || return 1
            fi
            rm -f "$r_target.staged" || return 1
        done
    fi
    [ ! -f "$STAGED/prev/no-ui-bin" ] || rm -f "$STATE/bin/lumatv-ui" || return 1
    [ ! -f "$STAGED/prev/no-ui-assets" ] || rm -rf "$STATE/assets/ui" || return 1
    install_bundle "$STAGED/prev" --delta > "$STAGED/install.out" 2>&1 || { staged_log "ROLLBACK FAILED: $(tail -n 1 "$STAGED/install.out")"; return 1; }
    if [ "$r_mode" != boot ] && [ "$r_live" = yes ]; then
        staged_step "restore enable" enable; r_rc=$?
        R_SEEN=$(staged_seen)
        if [ "$r_rc" -ne 0 ] && [ "$r_rc" -ne 12 ]; then staged_log "the previous build did not come back up (enable exit $r_rc)"; return 1; fi
    elif [ -f "$STAGED/flag.save" ]; then
        cp "$STAGED/flag.save" "$STATE/enabled"
    fi
    return 0
}

# Failure evidence in the supervisor's own records, which a boot of a run that did not go well leaves behind: LG's compositor held after a
# recovery, a fallback, a start that never settled (the running mark goes after two minutes healthy) and a count of failures (cleared after ten).
# Prints why and returns 0 when there is some; 1 when there is none, which is not proof of health.
staged_health_bad() {
    if [ -f "$STAGED/judge/helper-failed" ]; then echo "the staged helper failed to start"; return 0; fi
    if [ -e "$STATE/stock-ui-held" ]; then echo "LG's UI is held after a recovery"; return 0; fi
    if [ -e "$STATE/last-fallback" ]; then echo "the supervisor fell back: $(head -n 1 "$STATE/last-fallback" 2>/dev/null)"; return 0; fi
    if [ -e "$STATE/running" ]; then echo "the last start has not settled or ended cleanly"; return 0; fi
    h_f=$(cat "$STATE/failures" 2>/dev/null)
    case "$h_f" in ''|*[!0-9]*) h_f=0 ;; esac
    if [ "$h_f" -gt 0 ]; then echo "failures on record: $h_f"; return 0; fi
    return 1
}

# At a boot with an unconfirmed boot apply (run from judge/, the scripts it kept): failure evidence puts the previous files back before the
# compositor starts. No evidence leaves it pending, which blocks a later set until `staged accept` has seen a healthy run.
# 0 nothing pending, 20 pending and unconfirmed, 21 rolled back, 22 the rollback failed (retried at the next boot).
staged_judge_boot() {
    [ -f "$STAGED/boot-pending" ] || return 0
    exec 7> "$STAGED/lock"
    flock -n 7 || { echo "wait: another staged operation is running"; return 20; }
    [ -f "$STAGED/boot-pending" ] || { exec 7>&-; return 0; }
    [ ! -f "$STAGED/applying" ] || staged_clear
    if [ -f "$STAGED/judge/boot-only" ]; then
        if [ "${LUMATV_STAGED_BOOT:-0}" != 1 ] || [ "$(cat "$STAGED/judge/boot-only")" = "$(comp_boot_id)" ] ||
                pgrep -x lumatv-helper >/dev/null 2>&1; then
            echo "wait: the boot-only set can roll back only at a later early boot"
            return 20
        fi
    fi
    if comp_mounted; then echo "wait: LumaTV's compositor is already mounted; the boot apply is judged at a boot"; return 20; fi
    j_why=$(staged_health_bad); j_rc=$?
    if [ "$j_rc" -ne 0 ]; then echo "wait: the boot apply ($(cat "$STAGED/boot-pending")) is not confirmed yet; no failure on record (staged accept confirms it)"; return 20; fi
    D=$STAGED/judge
    staged_log "the boot apply did not run well ($j_why): putting the previous files back"
    if staged_restore boot no; then
        rm -rf "$STAGED/prev" "$STAGED/flag.save" "$STAGED/judge" "$STAGED/boot-pending"
        staged_log "ROLLED BACK (boot): the files of the boot apply are replaced by the previous ones"
        return 21
    fi
    staged_log "ROLLBACK FAILED (boot): the next boot tries again"
    return 22
}

# staged boot: what the boot hook runs. The earlier boot apply is judged first; a set staged since is applied only when nothing is pending, and
# never in the boot that rolled something back.
staged_boot() {
    D=$STAGED/dep; [ -d "$D" ] || D=$STAGED/judge
    staged_judge_boot; b_rc=$?
    case "$b_rc" in
    0) staged_apply boot; return $? ;;
    20) return 10 ;;
    21) [ ! -s "$STAGED/manifest" ] || echo "a staged set waits for the next boot"; return 1 ;;
    *) return 1 ;;
    esac
}

# staged accept: a boot apply becomes the installed state only on proof. Run by hand (or by whoever owns the install), never by the install tool:
# the panel on (Active) with Home in front and no game, no failure on record, the compositor live with its shell ready, and a video check that
# shows a picture on the clip the package stores (LUMATV_CLIP, default judge/feeder/check.h264). A check that was skipped (no clip, no feeder,
# panel off) is no proof. Any shortfall leaves everything as it is, the previous files included; 10 not now, 12 no picture, 1 not confirmed.
staged_accept() {
    [ -f "$STAGED/boot-pending" ] || { echo "nothing to confirm"; return 0; }
    exec 7> "$STAGED/lock"
    flock -n 7 || { echo "another staged apply is running"; return 11; }
    [ -f "$STAGED/boot-pending" ] || { echo "nothing to confirm"; return 0; }
    if c_why=$(staged_health_bad); then echo "not confirmed: $c_why"; return 1; fi
    c_why=$(staged_decide now); c_rc=$?
    if [ "$c_rc" -ne 0 ]; then echo "not confirmed, $c_why"; return 10; fi
    if ! { comp_live && comp_ready > /dev/null 2>&1 && ui_ready "$(staged_foreground)"; }; then
        echo "not confirmed: LumaTV's compositor is not live with its shell ready"; return 1
    fi
    if [ -f "$STAGED/judge/helper.sha256" ] && ! staged_helper_ready; then
        echo "not confirmed: the staged helper files do not match or the helper is not ready"
        return 1
    fi
    c_clip=${LUMATV_CLIP:-$STAGED/judge/feeder/check.h264}
    if [ ! -f "$c_clip" ]; then echo "not confirmed: the video check's clip is missing ($c_clip)"; return 1; fi
    comp_video_check "$c_clip"; c_vc=$?
    if [ "$c_vc" -eq 12 ]; then
        echo "not confirmed: no picture ($COMP_VC_WHY). The previous files are kept; readings: $COMP_VC_OUT/readings"
        staged_log "accept refused: no picture ($COMP_VC_WHY)"
        return 12
    fi
    if [ "$COMP_VC" != picture ]; then echo "not confirmed: the video check did not run ($COMP_VC_WHY)"; return 1; fi
    D=$STAGED/judge
    [ ! -f "$STAGED/applying" ] || staged_clear
    rm -rf "$STAGED/prev" "$STAGED/flag.save" "$STAGED/judge" "$STAGED/boot-pending"
    staged_log "ACCEPTED (boot apply): the compositor runs on the new files and the video check shows a picture; the previous ones are dropped"
}

# staged_apply MODE (boot, standby or now). Exit 0 applied, 10 not now (the set stays staged), 11 nothing to do, 1 failed and the previous files
# are back (or the set was discarded), 12 the video check found no picture with the new files and none with the previous ones either (they are
# back; the set needs a reboot, which is not done here). A set is applied once: whatever the outcome, it is gone afterwards, except a rollback that
# itself failed, which the next apply or boot repeats.
staged_apply() {
    a_mode=$1
    D=$STAGED/dep
    { [ -s "$STAGED/manifest" ] && [ -d "$STAGED/dep" ]; } || { echo "nothing is staged"; return 11; }
    exec 7> "$STAGED/lock"
    flock -n 7 || { echo "another staged apply is running"; return 11; }
    if [ -f "$STAGED/boot-pending" ]; then echo "wait: the boot apply of an earlier set is not confirmed yet (staged accept)"; return 10; fi
    { [ -s "$STAGED/manifest" ] && [ -d "$STAGED/dep" ]; } || { echo "nothing is staged"; return 11; }
    a_defer=no
    if staged_boot_only "$STAGED"; then
        a_defer=yes
        if [ "$a_mode" != boot ] || [ "${LUMATV_STAGED_BOOT:-0}" != 1 ]; then
            echo "wait: this set can only be applied by the early boot hook"; return 10
        fi
        if pgrep -x lumatv-helper >/dev/null 2>&1; then
            echo "wait: the helper has already started"; return 10
        fi
    fi
    a_why=$(staged_decide "$a_mode"); a_rc=$?
    if [ "$a_rc" -ne 0 ]; then echo "$a_why"; return 10; fi
    # An apply that was cut short (power, a kill) left files half copied: the old ones go back first, and the set is dropped. That is a compositor
    # restart as well, so it waits for the same moment.
    if [ -f "$STAGED/applying" ]; then
        read -r i_mode i_live < "$STAGED/applying"
        staged_log "an earlier apply did not finish: putting the previous files back"
        staged_restore "$a_mode" "$i_live"; i_rc=$?
        if [ "$i_rc" -eq 0 ]; then rm -f "$STAGED/applying"; staged_clear; fi
        return 1
    fi
    if [ "$(comp_flag)" = session ]; then echo "wait: a gate session is live"; return 10; fi
    if [ "$a_mode" = boot ] && comp_mounted; then echo "wait: LumaTV's compositor is already mounted"; return 10; fi
    if ! (cd "$STAGED/dep" && sha256sum -c "$STAGED/manifest" > /dev/null 2>&1); then
        staged_log "not applied: the staged files do not match their manifest"
        staged_clear; return 1
    fi
    if ! storage_guard "$STATE" "$STAGED/dep"; then
        staged_log "not applied: $STORAGE_WHY"
        staged_clear; return 1
    fi
    a_helper=$(sed -n 's/  helper\/lumatv-helper$//p' "$STAGED/manifest")
    if [ "$a_defer" = yes ]; then
        a_boot_id=$(comp_boot_id)
        comp_boot_decide
        if [ -z "$a_boot_id" ] || [ "$COMP_WORD" != ok ]; then
            staged_log "wait: boot-only apply needs an enabled, runnable LumaTV compositor and a boot identifier"
            return 10
        fi
    fi
    if [ -n "$a_helper" ]; then
        if [ "$a_defer" != yes ]; then
            staged_log "not applied: the helper needs a boot-only set"; staged_clear; return 1
        fi
        if ! storage_guard "$(dirname "$HELPER_PAYLOAD")" "$STAGED/dep/helper/lumatv-helper" ||
           ! storage_guard "$DATA" "$STAGED/dep/helper/lumatv-helper"; then
            staged_log "not applied: $STORAGE_WHY"; staged_clear; return 1
        fi
    fi
    a_flag=$(comp_flag); a_live=no
    if comp_mounted; then a_live=yes; fi
    staged_backup || { staged_log "not applied: the previous files could not be kept"; staged_clear; return 1; }
    rm -f "$STAGED/flag.save"
    if [ "$a_flag" = persistent ] && ! cp "$STATE/enabled" "$STAGED/flag.save"; then
        staged_log "not applied: the enable flag could not be kept"; staged_clear; return 1
    fi
    echo "$a_mode $a_live" > "$STAGED/applying" || return 1
    staged_log "applying ($a_mode; flag $a_flag; live $a_live): $(tr '\n' ';' < "$STAGED/changes")"
    a_fail=""; a_video=0; a_changed=1
    if [ "$a_mode" = boot ]; then
        # Nothing is mounted yet and the supervisor's own question is what the boot hook asks next: the new files must give the same answer.
        comp_boot_decide; a_before=$COMP_WORD
        if install_bundle "$STAGED/dep" --delta > "$STAGED/install.out" 2>&1; then
            [ ! -f "$STAGED/flag.save" ] || cp "$STAGED/flag.save" "$STATE/enabled"
            if ! comp_bundle_ok full; then a_fail="the new bundle is not intact: $COMP_WHY"
            else
                comp_boot_decide; a_after=$COMP_WORD
                [ "$a_after" = "$a_before" ] || a_fail="the boot check says $a_after, it said $a_before"
            fi
        else a_fail="install failed: $(tail -n 1 "$STAGED/install.out")"; fi
        if [ -z "$a_fail" ] && [ "$a_defer" = yes ] && grep -Eq '  (bin/lumatv-ui$|assets/ui/)' "$STAGED/manifest"; then
            if ! (cd "$STATE" && grep -E '  (bin/lumatv-ui$|assets/ui/)' "$STAGED/manifest" | sha256sum -c - >/dev/null 2>&1); then
                a_fail="the installed UI does not match the staged files"
            fi
        fi
        # The boot check of a later boot runs from these copies once the staged set is gone.
        if [ -z "$a_fail" ]; then
            rm -rf "$STAGED/judge"; mkdir -p "$STAGED/judge/feeder"
            cp "$STAGED/dep/cutover.sh" "$STAGED/dep/compositor.sh" "$STAGED/dep/logrelay.py" "$STAGED/judge/" &&
                cp "$STAGED/dep/feeder/feeder.sh" "$STAGED/judge/feeder/" || a_fail="the scripts for the boot check could not be kept"
            if [ -z "$a_fail" ] && [ "$a_defer" = yes ]; then
                printf '%s\n' "$a_boot_id" > "$STAGED/judge/boot-only" || a_fail="the boot identifier could not be kept"
            fi
            if [ -z "$a_fail" ] && [ -n "$a_helper" ]; then
                a_src="$STAGED/dep/helper/lumatv-helper"
                printf '%s  %s\n' "$a_helper" "$HELPER_PAYLOAD" "$a_helper" "$HELPER_BIN" > "$STAGED/judge/helper.sha256" &&
                    staged_helper_put "$a_src" "$HELPER_PAYLOAD" &&
                    staged_helper_put "$a_src" "$HELPER_BIN" &&
                    sha256sum -c "$STAGED/judge/helper.sha256" >/dev/null 2>&1 || a_fail="the staged helper could not be installed and verified"
            fi
        fi
    else
        if [ "$a_live" = yes ]; then
            staged_step disable disable; s_rc=$?
            case "$s_rc" in
            0) ;;
            12) a_fail="no picture after LG's compositor came back, before any file changed"; a_video=1; a_changed=0 ;;
            *) a_fail="disable exit $s_rc"; a_changed=0 ;;
            esac
        fi
        if [ -z "$a_fail" ]; then
            install_bundle "$STAGED/dep" --delta > "$STAGED/install.out" 2>&1 || a_fail="install failed: $(tail -n 1 "$STAGED/install.out")"
            # install clears the flag, and enable sets it again; a set that was not live keeps its flag for the next boot.
            if [ "$a_live" = no ] && [ -f "$STAGED/flag.save" ]; then cp "$STAGED/flag.save" "$STATE/enabled"; fi
        fi
        if [ -z "$a_fail" ]; then
            staged_step probe probe || a_fail="probe exit $?"
        fi
        if [ -z "$a_fail" ] && [ "$a_live" = yes ]; then
            staged_step enable enable; s_rc=$?
            case "$s_rc" in 0) ;; 12) a_fail="the new build shows no picture (video check)"; a_video=1 ;; *) a_fail="enable exit $s_rc" ;; esac
        fi
    fi
    if [ -n "$a_fail" ]; then
        a_rc=1; a_seen=unchecked; a_label="NOT APPLIED"
        rm -rf "$STAGED/judge"
        if [ "$a_changed" = 1 ]; then
            a_label="ROLLED BACK"
            if staged_restore "$a_mode" "$a_live"; then a_seen=$R_SEEN
            else
                staged_log "ROLLBACK FAILED ($a_mode): $a_fail; the next apply or boot tries the previous files again"
                return 1
            fi
        elif [ "$a_live" = yes ]; then
            staged_step "re-enable" enable; a_seen=$(staged_seen)
            if [ "$s_rc" -ne 0 ] && [ "$s_rc" -ne 12 ]; then
                staged_log "NOT APPLIED ($a_mode): $a_fail; and LumaTV's compositor did not come back (enable exit $s_rc)"
                rm -f "$STAGED/applying"; staged_clear
                return 1
            fi
        fi
        if [ "$a_video" = 1 ] && [ "$a_changed" = 0 ]; then
            staged_log "NO PICTURE ($a_mode): $a_fail. No file was changed and the new build was not tried. The set needs a reboot (not done here)"
            a_rc=12
        elif [ "$a_video" = 1 ] && [ "$a_seen" = none ]; then
            # Exit 12 is a fault of the set's video path after any compositor switch: it showed with the previous files too, so the files are not
            # what to blame. Nothing here reboots or launches anything; the previous files are in place and a reboot is the owner's call.
            staged_log "NO PICTURE ($a_mode): $a_fail. The previous files are back and still show no picture; the new build is not shown to be at fault. The set needs a reboot (not done here)"
            a_rc=12
        elif [ "$a_video" = 1 ] && [ "$a_seen" = picture ]; then
            staged_log "$a_label ($a_mode): $a_fail. The previous build shows a picture on the same check, so the new build is at fault"
        elif [ "$a_video" = 1 ]; then
            staged_log "$a_label ($a_mode): $a_fail. The previous build's picture was not confirmed (the check did not run), so the new build is not shown to be at fault"
        else
            staged_log "$a_label ($a_mode): $a_fail"
        fi
        rm -f "$STAGED/applying"; staged_clear
        return $a_rc
    fi
    if [ "$a_mode" = standby ] && [ "$(staged_power)" != "Active Standby" ]; then
        staged_log "NOTE: the set was not in Active Standby when the apply ended ($(staged_power)): the panel may have woken"
    fi
    staged_log "applied ($a_mode): $(tr '\n' ';' < "$STAGED/changes")"
    if [ "$a_mode" = boot ]; then
        # Not accepted yet: the compositor has not run once with these files. The previous ones stay until a healthy run is on record (staged
        # accept), and the next boot puts them back when the supervisor's own records say the run failed.
        if ! { echo "set $(sha256sum "$STAGED/manifest" | cut -c1-8), applied $(date '+%Y-%m-%d %H:%M:%S')" > "$STAGED/boot-pending.new" &&
                mv "$STAGED/boot-pending.new" "$STAGED/boot-pending"; }; then
            staged_log "boot pending marker could not be kept; the next early boot restores the previous files"
            return 1
        fi
        staged_log "the boot apply stays pending: the previous files are kept until a healthy run is confirmed (staged accept)"
        rm -rf "$STAGED/manifest" "$STAGED/dep" "$STAGED/dep.new" "$STAGED/applying"
    else
        rm -f "$STAGED/applying"; staged_clear
    fi
    return 0
}

# staged_commit: the files the host uploaded to staged/dep.new and their manifest (manifest.new) become the staged set. Run as dep.new/cutover.sh.
# Nothing watches for a moment to apply it: the next boot applies it (before the compositor starts), or `staged apply --now` does, by the owner's
# word under the TV lease. A standby apply (`staged apply --standby`) is a command, never a trigger, until it has been proven not to wake the panel.
staged_commit() {
    [ -f "$STAGED/manifest.new" ] && [ -d "$STAGED/dep.new" ] || { echo "nothing was uploaded to $STAGED/dep.new"; return 1; }
    exec 7> "$STAGED/lock"
    flock -n 7 || { echo "an apply is running"; return 1; }
    if [ -f "$STAGED/applying" ]; then echo "an apply is in flight or was cut short: it must finish first"; return 1; fi
    c_defer=no
    if staged_boot_only "$STAGED" .new; then c_defer=yes; fi
    if staged_boot_only "$STAGED" || { [ "$c_defer" = yes ] && { [ -e "$STAGED/manifest" ] || [ -e "$STAGED/boot-pending" ]; }; }; then
        echo "an existing staged set is kept"; return 1
    fi
    if ! (cd "$STAGED/dep.new" && sha256sum -c "$STAGED/manifest.new" > /dev/null 2>&1); then
        rm -rf "$STAGED/dep.new" "$STAGED/manifest.new"
        echo "the upload does not match its manifest; discarded"; return 1
    fi
    rm -f "$STAGED/manifest"; rm -rf "$STAGED/dep"
    mv "$STAGED/dep.new" "$STAGED/dep" || return 1
    mv "$STAGED/manifest.new" "$STAGED/manifest" || return 1
    exec 7>&-
    D=$STAGED/dep
    if [ "$c_defer" = yes ]; then c_when="only at early boot"; else c_when="at the next boot, or by staged apply --now"; fi
    staged_log "staged $(wc -l < "$STAGED/manifest" | tr -d ' ') files, set $(sha256sum "$STAGED/manifest" | cut -c1-8); applies $c_when"
}

staged_status() {
    if [ -s "$STAGED/manifest" ]; then
        echo "staged: $(wc -l < "$STAGED/manifest" | tr -d ' ') files, set $(sha256sum "$STAGED/manifest" | cut -c1-8)"
        if staged_boot_only "$STAGED"; then echo "activation: early boot only"; fi
    else
        echo "staged: nothing"
    fi
    echo "apply in flight or cut short: $([ -f "$STAGED/applying" ] && cat "$STAGED/applying" || echo no)"
    echo "boot apply awaiting confirmation: $([ -f "$STAGED/boot-pending" ] && cat "$STAGED/boot-pending" || echo no)"
    tail -n 8 "$STAGED/journal" 2>/dev/null
}

# ui_ready FG: the shell client is connected and drawing, and Home is drawn when Home is what is in front.
ui_ready() {
    u_rep=$(comp_ctl shell 2>/dev/null) || return 1
    printf '%s' "$u_rep" | grep -Eq '"ready"[[:space:]]*:[[:space:]]*true' || return 1
    printf '%s' "$u_rep" | grep -Eq '"gaveUp"[[:space:]]*:[[:space:]]*false' || return 1
    if [ "$1" = "$HOME_APP" ]; then printf '%s' "$u_rep" | grep -Eq '"homeDrawn"[[:space:]]*:[[:space:]]*true' || return 1; fi
    return 0
}

ui_pid() {
    comp_ctl shell 2>/dev/null | sed -n 's/.*"pid"[[:space:]]*:[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -n 1
}

# Restarts the shell client alone and waits (UI_WAIT s) for its replacement to be connected and drawing. 1 when it is not, or the compositor
# went away (a shell that cannot start ends the compositor, and the supervisor falls back to LG's).
ui_restart() {
    u_fg=$(staged_foreground); u_before=$(ui_pid)
    comp_ctl shell restart > /dev/null 2>&1 || return 1
    u_t=0
    while [ "$u_t" -lt "$UI_WAIT" ]; do
        sleep 1; u_t=$((u_t + 1))
        comp_live || return 1
        u_pid=$(ui_pid)
        if [ -n "$u_pid" ] && [ "$u_pid" != "$u_before" ] && ui_ready "$u_fg"; then
            echo "shell client restarted: pid ${u_before:-none} -> $u_pid, ready after ${u_t}s"
            return 0
        fi
    done
    return 1
}

# ui_swap: bin/lumatv-ui and assets/ui of $D replace the installed ones, each by a rename of a copy beside it (a reader sees the old file or
# the new one), the bundle's record of them follows at once (the boot refuses a bundle whose files are not the recorded ones), and the shell
# client restarts when LumaTV's compositor is live. A shell that does not come back is replaced by the one that was there.
ui_swap() {
    [ -f "$D/bin/lumatv-ui" ] || [ -d "$D/assets/ui" ] || { echo "ui-swap: no bin/lumatv-ui or assets/ui in $D"; return 1; }
    [ -x "$STATE/bin/lumatv-compositor" ] || { echo "ui-swap: no bundle is installed; run install first"; return 1; }
    if [ -f "$D/bin/lumatv-ui" ]; then
        # A file that is not a program for this loader, or whose libraries are missing, is found here and not by a restart.
        "$STATE/lib/ld-linux-aarch64.so.1" --library-path "$STATE/lib" --list "$D/bin/lumatv-ui" > /dev/null 2>&1 || { echo "ui-swap: the new lumatv-ui does not load on this set"; return 1; }
    fi
    storage_guard "$STATE" "$D/bin/lumatv-ui" "$D/assets/ui" || { echo "$STORAGE_WHY"; return 1; }
    rm -rf "$STATE/ui-prev"; mkdir -p "$STATE/ui-prev"
    [ ! -f "$STATE/bin/lumatv-ui" ] || cp "$STATE/bin/lumatv-ui" "$STATE/ui-prev/lumatv-ui" || return 1
    [ ! -d "$STATE/assets/ui" ] || cp -R "$STATE/assets/ui" "$STATE/ui-prev/ui" || return 1
    u_old=$(comp_sha256 "$STATE/bin/lumatv-ui" 2>/dev/null | cut -c1-8)
    ui_put "$D/bin/lumatv-ui" "$D/assets/ui" || { staged_log "ui-swap: the files could not be put in place"; ui_put_back; return 1; }
    write_bundle_manifest
    u_new=$(comp_sha256 "$STATE/bin/lumatv-ui" 2>/dev/null | cut -c1-8)
    if ! comp_live; then
        staged_log "ui-swap: lumatv-ui ${u_old:--} -> ${u_new:--}; LumaTV's compositor is not live, so nothing restarted"
        return 0
    fi
    if ui_restart; then
        staged_log "ui-swap: lumatv-ui ${u_old:--} -> ${u_new:--}, shell client restarted, compositor untouched"
        return 0
    fi
    staged_log "ui-swap: the new shell did not come up; putting the previous one back"
    ui_put_back
    write_bundle_manifest
    if comp_live && ui_restart; then staged_log "ui-swap: the previous shell is back"; else staged_log "ui-swap: the previous shell did not come back either"; fi
    return 1
}

# ui_put NEWUI NEWASSETS: each part given is put in place by a rename.
ui_put() {
    if [ -f "$1" ]; then
        cp "$1" "$STATE/bin/.lumatv-ui.new" || return 1
        chmod 755 "$STATE/bin/.lumatv-ui.new"
        mv -f "$STATE/bin/.lumatv-ui.new" "$STATE/bin/lumatv-ui" || return 1
    fi
    if [ -d "$2" ]; then
        rm -rf "$STATE/assets/ui.new" "$STATE/assets/ui.old"; mkdir -p "$STATE/assets"
        cp -R "$2" "$STATE/assets/ui.new" || return 1
        [ ! -d "$STATE/assets/ui" ] || mv "$STATE/assets/ui" "$STATE/assets/ui.old" || return 1
        mv "$STATE/assets/ui.new" "$STATE/assets/ui" || return 1
        rm -rf "$STATE/assets/ui.old"
    fi
}

ui_put_back() {
    ui_put "$STATE/ui-prev/lumatv-ui" "$STATE/ui-prev/ui"
}

case "${1:-help}" in
install)
    install_bundle "$D" "${2:-}" || exit $?
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
        video_check enable
    fi
    ;;
disable)
    revert || exit $?
    video_check disable
    ;;
revert)
    # `disable` is what installs run in between; only this verb is a real return to LG's UI, which takes the repaired unified-server
    # launcher and the Rust gateway back with it (the escape's recovery does the same through `lumatv stock-ui-restore`).
    revert; rc=$?
    "$LUMATV_TOOL" sysbus off || rc=1
    "$LUMATV_TOOL" ssgw off || rc=1
    exit $rc
    ;;
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
ui-swap)
    ui_swap || exit $?
    ;;
staged)
    case "${2:-status}" in
    status) staged_status ;;
    commit) staged_commit || exit $? ;;
    discard)
        exec 7> "$STAGED/lock"
        flock -n 7 || { echo "another staged operation is running"; exit 1; }
        if [ -f "$STAGED/applying" ]; then echo "an apply is in flight or was cut short: it puts the previous files back first"; exit 1; fi
        staged_clear; echo "staged: discarded" ;;
    apply)
        case "${3:---now}" in
        --boot) staged_boot ;;
        --standby) staged_apply standby ;;
        --now) staged_apply now ;;
        *) echo "staged apply [--boot | --standby | --now]"; exit 1 ;;
        esac
        exit $?
        ;;
    boot) staged_boot; exit $? ;;
    accept) staged_accept; exit $? ;;
    *) echo "staged status | commit | discard | boot | accept | apply [--boot | --standby | --now]"; exit 1 ;;
    esac
    ;;
*) sed -n '2,/^[^#]/{/^#/p;}' $0 ;;
esac
