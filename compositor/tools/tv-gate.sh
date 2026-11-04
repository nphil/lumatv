#!/bin/sh
# The C4 gate, executed ON THE SET as /tmp/lumatv/gate.sh while LumaTV's compositor is live (cutover.sh enable --session).
# Each check prints one "GATE <name>: PASS|FAIL <detail>" line; files under /tmp/lumatv/out/gate/. Driven from
# the workstation by tools/tv_gate.py, which mutes the receiver, wakes the set and orders the checks.
#
#   gate.sh live [LABEL]  LumaTV is the compositor: mounted, 12 names registered, no fallback, WAM up, Home (the web
#                     stub) mapped and on the panel, the watchdog reading the compositor's counters and the power state
#   gate.sh x1 SECS   the X1 probe client (64-bit, LG's blob) for SECS: the frame rule below. LUMATV_GATE_KWAIT traces the run, which is what lets the
#                     rule tell the compositor's gaps from the client's (tv_gate.py sets it unless --x1-strict); LUMATV_X1_STRICT is the PLAN's rule
#                     (every gap, untraced). A/Bs, the rule unchanged: LUMATV_X1_RT=N the client runs SCHED_FIFO N; LUMATV_X1_CALLBACKS=commit|early|landing
#                     the clients' frame callbacks go out with the commit (the default), the frame's draw or at the flip (tv_gate.py --x1-rt, --x1-callbacks)
#   gate.sh netflix   the Dolby Vision test film by deep link, from a fresh Netflix instance (a running one is
#                     closed first): card mapped, video window exported, Picture Mode dolbyHdrCinema while
#                     playing, filmMaker back on Home
#   gate.sh hdmi      HDMI 2 (the receiver) as the foreground app, its card mapped, then Home and HDMI 2 ended through SAM
#   gate.sh apps      YouTube (cobalt) and PlxNative (LG's SDL) from a fresh start: card mapped and foreground within 5 s, still running
#                     10 s on with its connection kept, ended through SAM within 15 s with no exit status logged; then GeForce NOW,
#                     which opens LG's browser: Home launched over it still in front 6 s later (the apps that failed under LumaTV's
#                     compositor on 2026-10-28 while every other ran)
#   gate.sh keys      the remote's own keys through the compositor's routing (keys.rs): a gear tap, Input, the Netflix button, a digit tapped
#                     and held, Exit; each judged by the compositor's journal and by what SAM then has in front
#   gate.sh ime       a text field in LG's Search app brings up maliit's keyboard: an input method bound, a context made for the field,
#                     the keyboard's window on the screen at the bottom, a key typed on it reaching the field, the keyboard closing
#   gate.sh shell     native UI readiness, Home navigation, panels, volume, toast and saver
#   gate.sh rust      native UI kinds: readiness, display, drawing and close checks (LUMATV_RUST_KINDS selects kinds)
#   gate.sh evidence  no check: SAM's subscriptions, the clients (command lines, globals) and both logs, while live
#   gate.sh key CODE  one remote key through the compositor's routing (the volume check in tv_gate.py)
#   gate.sh standby   power off (remoteKey); the driver wakes the set; `gate.sh woke` then checks the rounds
#   gate.sh woke      after the wake: PreparePowerOff and PreparePowerOn acked, no fallback, Home on the panel
#   gate.sh firmware  not live: the firmware check on this set's own files and tools, against a scratch list
#   gate.sh homeheld  not live: a virtual remote holds Home (773); `lumatv-sm --home-held` sees it, then stops seeing it
#   gate.sh homekey   live: the supervisor's own Home reader (773), on a virtual remote the compositor never opened
#   gate.sh wd-hang   live: the compositor is stopped (SIGSTOP); the heartbeat falls back to LG's, SAM untouched
#   gate.sh wd-crash  live: the compositor is killed (SIGKILL); the supervisor falls back to LG's, SAM untouched; the crash bundle it left
#   gate.sh wake [reset|probe SECS]  the loop's wake-up latency (p50/p99/max of the tick, the flip, the input) since the last reset, or a
#                     SECS-long cyclictest-style probe at the loop's class, the Mali submission threads' class and a time-shared one. A report, no rule
#   gate.sh video [youtube|netflix|plex|current] [SECS]   a video app full screen: the pages AR24 and fb0's constant alpha off, the DISPLAY capture's
#                     middle not black, the compositor's cost with nothing over the video, and each overlay (volume, quick panel, picker, drawer, toast, alert)
#                     over it: first frame, video seen around it, no frame of its motion dropped, idle again after the close (tv_gate.py --video-apps)
#   gate.sh mem       what the compositor has resident and pinned, malloc's mmapped blocks, and what the set has left (report)
#   gate.sh callbacks   report client frame callbacks; LUMATV_CLIENTS_CALLBACKS sets their pacing
#   gate.sh load start SECS [KINDS] | stop | status   synthetic background load (decode, ws, ble, download) for the frame gate's loaded run
#   gate.sh scanout [direct|off|read]   set or report direct scan-out
#   gate.sh refresh [60|120|reset|read] [SECS]   the refresh test mode (`ctl refresh`, docs/research/ui120-result.md): with 60 or 120 switches the
#                     output to it (the OSD vblank divider, the pacer, the tick, the shell), then measures SECS (default 5) of the screen as it is:
#                     flips a second, DE2VSYNC and osd_irq a second, the compositor's measure (intervals in refreshes, late, lost, missed,
#                     pipeline p50/p99/max), the refresh the shell draws for, Mali busy, the SoC's temperature. `reset` starts the measure over, `read` (the default) only measures. A report
#
# The frame rule (PLAN, C4: "the X1 test with zero frames over 20 ms in 60 s"), strict: the client's swap-to-swap gaps over
# the run: none over 20 ms, the first second included (no warm-up is excused); the run is the whole 60 s and holds 60 Hz
# (59.5 fps or more); the pacer's `late` did not move; and the compositor's own log of the run (from the client's start
# to its end) records no loop pass, client frame gap or page-flip interval over 20 ms either (`tv_gate.py --x1-strict`, LUMATV_X1_STRICT).
# The rule the gate runs (round 17: the strict one failed 4 of 4 runs with the compositor on time and the client waking late on its callbacks, a
# rule that has passed about a third of the runs on this set): the run is traced, and kwait.py (`x1 gaps:` line) says whose each gap is. A gap over
# 20 ms that is the compositor's (a late flip, the loop slow to take a flip up or waiting, callbacks that came late) or that nothing explains fails;
# one that is the client's own wake-up after callbacks on time is reported with its count and does not. The length, the fps, the pacer's `late` and
# the compositor's log (its loop passes and flip intervals; the client's frame gaps are late commits and are reported) are as in the strict rule.
# Raw swaps and RAW STRICT remain in every result, also saved as x1-strict-result.txt. Missing, truncated, lost or mismatched trace evidence is
# INCONCLUSIVE and cannot pass attribution. Only explicitly requested --x1-strict may pass without a trace.
STATE=${LUMATV_STATE:-/var/lib/lumatv/compositor}
D=${LUMATV_D:-/tmp/lumatv}
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
BLOB=$LIB/libmali-bifrost-g52-g24p0-wayland-gbm.so
OUT=${LUMATV_OUT:-$D/out/gate}
LGBIN=${LUMATV_LG:-/usr/bin/surface-manager}
MOUNTINFO=${LUMATV_MOUNTINFO:-/proc/self/mountinfo}
COMP_LOCK=${LUMATV_RECOVER_LOCK:-/tmp/lumatv-recover.flock}
MESSAGES=${LUMATV_MESSAGES:-/var/log/messages}
# PlxNative's own crash log (a `*** SIGNAL` report per crash): SAM's exit line for its crash at the close was missing or read as a
# clean close in 4 of 9 crashes (round 17), and the report is written about 1.05 s after the close.
PLX_CRASH_LOG=${LUMATV_PLX_CRASH_LOG:-/tmp/plxnative-crash.log}
CRASH_SETTLE=${LUMATV_CRASH_SETTLE:-3}
PROC=${LUMATV_PROC:-/proc}
GAP_LIMIT_MS=20
GATE_SECONDS=60
MIN_FPS=59.5
mkdir -p $OUT
# The firmware functions of tools/compositor.sh, deployed beside this script.
COMP=$STATE
COMP_LG=$LGBIN
COMP_MOUNTINFO=$MOUNTINFO
log_line() { :; }
[ -f $D/compositor.sh ] && . $D/compositor.sh
ctl() { $LD --library-path $LIB $D/bin/lumatv-compositor ctl --runtime-dir /tmp/xdg "$@" 2>/dev/null; }
ls1() { timeout 10 luna-send -n 1 -f "luna://$1" "$2" 2>/dev/null; }
cap() { timeout 10 luna-send -n 1 luna://com.webos.service.capture/executeOneShot "{\"path\":\"$1\",\"method\":\"DISPLAY\",\"format\":\"PNG\",\"width\":960,\"height\":540}" 2>/dev/null | tr ',' '\n' | grep writtenBytes | tr -dc '0-9'; }
is_mounted() { grep -q " $LGBIN " $MOUNTINFO; }
pass() { echo "GATE $1: PASS $2"; }
fail() { echo "GATE $1: FAIL $2"; }
picture_mode() { ls1 com.webos.settingsservice/getSystemSettings '{"category":"picture","keys":["pictureMode"]}' | tr -d ' \n' | sed -n 's/.*"pictureMode":"\([^"]*\)".*/\1/p'; }
home() { ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.home"}' > /dev/null; }
fg() { ls1 com.webos.applicationManager/getForegroundAppInfo '{}' | tr -d ' \n' | sed -n 's/.*"appId":"\([^"]*\)".*/\1/p'; }
# The remote's keys as the compositor's own routing takes them (`ctl remote`): a tap, or a hold of the given seconds.
key_tap() { ctl remote "$1" tap > /dev/null; }
key_hold() { ctl remote "$1" press > /dev/null; sleep "$2"; ctl remote "$1" release > /dev/null; }
close_app() { ls1 com.webos.applicationManager/closeByAppId "{\"id\":\"$1\"}" > /dev/null; }
# Whether SAM lists APP as running, and a wait of SECS at most for it to stop. `luna-send -f` prints the answer indented, `"id": "APP"`.
app_running() { ls1 com.webos.applicationManager/running '{}' | tr -d ' \n' | grep -q "\"id\":\"$1\""; }
wait_app_gone() { w=0; while [ "$w" -lt "$2" ]; do app_running "$1" || return 0; sleep 1; w=$((w+1)); done; ! app_running "$1"; }
# The app ids of the compositor's mapped windows, one a line. SAM's foreground answer names the front card only: LG's Settings and Search are
# overlays, which it never names (round 11), so an app a key launched is looked for in the compositor's own window list.
mapped_apps() { ctl windows | sed 's/{"appId"/\n&/g' | grep '"mapped":true' | sed -n 's/^{"appId":"\([^"]*\)".*/\1/p'; }
app_mapped() { mapped_apps | grep -qx "$1"; }
# Waits, SECS at most, for a window of APP to be mapped.
wait_app() { w=0; while [ "$w" -lt "$2" ]; do app_mapped "$1" && return 0; sleep 1; w=$((w+1)); done; app_mapped "$1"; }
# SAM logs how an app ended: `APP_CLOSED {"app_id":"APP","type":"native","pid":"N","exit_status":S}`, in /var/log/messages, which rotates about every
# minute, so a line is gone before a check that runs 30 s and more reads it at its end. exit_watch_start APP FILE copies each such line out every
# second (the file and the one it rotated to) from before the launch, until exit_watch_stop; exit_status_of FILE [PID] is the nonzero status
# of that pid in what was copied (a signal is one: SIGSEGV 11, SIGKILL 9), no output for none. FILE.before holds the lines that were there already.
exit_lines() { for f in $MESSAGES*; do case $f in *.gz|*.xz|*.bz2) ;; *) cat "$f" 2>/dev/null ;; esac; done | grep -o "APP_CLOSED {\"app_id\":\"$1\"[^}]*}" | sort -u; }
exit_watch_start() {
    exit_lines "$1" > "$2.before"
    : > "$2"
    ( while :; do exit_lines "$1" >> "$2"; sleep 1; done ) > /dev/null 2>&1 &
    EXIT_WATCH=$!
}
exit_watch_stop() {
    kill "$EXIT_WATCH" 2> /dev/null; wait "$EXIT_WATCH" 2> /dev/null
    exit_lines "$1" >> "$2"
}
exit_status_of() {
    sort -u "$1" | grep -vxFf "$1.before" | grep "\"pid\":\"${2:-[0-9]*}\"" | sed -n 's/.*"app_id":"\([^"]*\)".*"pid":"\([0-9]*\)".*"exit_status":\([1-9][0-9]*\).*/APP_CLOSED {"app_id":"\1","type":"native","pid":"\2","exit_status":\3/p' | head -n 1
}
# SAM logs an app's APP_CLOSED line about a second after the app left its running list (round 16: Plex's crash, 0.5 s after the close,
# was logged after the check had read the log and passed): the line of PID is waited for in FILE, SECS at most.
exit_wait() {
    end=$(($(date +%s) + $3))
    while [ "$(date +%s)" -lt "$end" ] && ! sort -u "$1" | grep -vxFf "$1.before" | grep -q "\"pid\":\"$2\""; do sleep 1; done
}
# An app's own crash log (PlxNative keeps one), the reports in it, and the wait until SECS ($CRASH_SETTLE) after the close request
# taken at UPTIME_CS, on /proc/uptime: `date +%s` would cut a wait in its first second to nothing.
crash_log() { case $1 in com.beb.plxnative) echo "$PLX_CRASH_LOG" ;; esac; }
crash_reports() { if [ -f "$1" ]; then grep -c '^\*\*\* SIGNAL' "$1" || :; else echo 0; fi; }
uptime_cs() { awk '{ printf "%d", $1 * 100 }' /proc/uptime; }
settle_after() {
    end=$(($1 + $(awk -v s="$CRASH_SETTLE" 'BEGIN { printf "%d", s * 100 }')))
    while [ "$(uptime_cs)" -lt "$end" ]; do sleep 0.1; done
}
# Where the screen stands, for a check's failure line.
where_now() { echo "foreground $(fg), windows mapped: $(mapped_apps | tr '\n' ' ' | sed 's/ $//')"; }
# The compositor's journal after the line a `ctl note` left (the journal is one JSON array of strings), one entry a line; the quotes
# inside an entry become '.
journal_after() {
    ctl journal | sed -e "s/\\\\\"/'/g" -e 's/","/\n/g' -e 's/^{"lines":\["//' -e 's/"\],"ok":true}$//' | awk -v m="note: $1" 'f { print } index($0, m) { f = 1 }'
}
# The first number of that name in the JSON on stdin.
jnum() { grep -o "\"$1\":-\{0,1\}[0-9]*" | head -n 1 | cut -d: -f2; }
# The first string, or the first true/false, of that name in the JSON on stdin.
jstr() { grep -o "\"$1\":\"[^\"]*\"" | head -n 1 | cut -d'"' -f4; }
jbool() { grep -o "\"$1\":\(true\|false\)" | head -n 1 | cut -d: -f2; }
# Native UI report and its client state.
RUSTJSON=$D/rustjson.py
shell_json() { ctl shell | python3 $RUSTJSON; }
RUST_OPEN=${LUMATV_RUST_OPEN_WAIT:-10}
rust_get() { ctl shell | python3 $RUSTJSON "$1"; }
rust_kinds_on() { rust_get kinds; }
rust_answers() { [ "$(rust_get ready)" = true ] && case ",$(rust_get ui.kinds)," in *",$1,"*) return 0 ;; esac; return 1; }
wait_panel() {
    n=0
    while [ "$n" -lt "$3" ]; do
        case "$(rust_get "ui.panels.$1")" in opening|open) state=open ;; closing|parked|gone) state=closed ;; *) state=unknown ;; esac
        [ "$state" = "$2" ] && return 0
        sleep 0.5; n=$((n+1))
    done
    return 1
}
# Where each kind says it is shown (a path of `ctl shell`) and what that reading must be. A panel's phase is opening, open, closing, parked or gone.
rust_path() {
    case $1 in
        osd) echo ui.osd ;; toast) echo ui.toasts ;; alert) echo ui.alerts ;; saver) echo ui.saver ;; home) echo ui.home.home ;;
        quick|picker|drawer) echo ui.panels.$1 ;; settings) echo ui.settings.open ;; cameras) echo ui.cameras.screen.mode ;; guide) echo ui.guide.open ;;
    esac
}
rust_is_open() {
    case $1 in
        osd|saver|home|settings|guide) [ "$2" = true ] ;;
        toast|alert) [ "${2:-0}" -ge 1 ] 2> /dev/null ;;
        quick|picker|drawer) [ "$2" = opening ] || [ "$2" = open ] ;;
        cameras) [ "$2" = grid ] || [ "$2" = full ] ;;
        *) return 1 ;;
    esac
}
# What opens each kind the way the set does: the volume key, LG's notification service, tvpower, or the compositor's `show` (the gear and Input do the same).
rust_trigger() {
    case $1 in
        osd) key_tap 115 ;;
        toast) ls1 com.webos.notification/createToast '{"message":"LumaTV gate: a toast of the Rust shell","sourceId":"org.nphil.lumatv"}' > /dev/null ;;
        alert) ls1 com.webos.notification/createAlert '{"sourceId":"com.webos.app.home","title":"LumaTV gate","message":"Closed with OK.","showTimeout":false,"buttons":[{"label":"OK"}]}' > /dev/null ;;
        saver) ls1 com.webos.service.tvpower/power2/turnOnScreenSaver '{}' > /dev/null ;;
        quick|picker|drawer|settings|cameras|guide) ctl shell show "$1" > /dev/null ;;
    esac
}
# The way back: the volume key undoing the one pressed, OK on the alert, a key ending the saver, Back (one step each) for a panel or a page.
rust_close() {
    case $1 in
        osd) key_tap 114 ;;
        alert) key_tap 28 ;;
        saver) key_tap 106 ;;
        quick|picker|drawer|settings|cameras|guide) n=0; while [ $n -lt 4 ]; do key_tap 412; sleep 0.7; rust_is_open "$1" "$(rust_get "$(rust_path "$1")")" || break; n=$((n+1)); done ;;
    esac
}
# Why a kind that is built is not driven here (empty: it is).
rust_undriven() {
    case $1 in
        menu) echo "a tile's menu opens on a hold of OK on Home, which the gate does not press" ;;
        confirm) echo "a panel asks it for an action, which the gate does not take" ;;
        switcher) echo "the app switcher needs open apps; the frames gate's switcher surface covers it" ;;
        cards) echo "the native card queue requires plugin-broker records; the control socket cannot inject a card" ;;
        popup) echo "the camera pop-up requires a camera event and the owner's settings" ;;
        capture) echo "needs capture.enabled and the encoder, which are the owner's (docs/research/capture-result.md)" ;;
        layout) echo "Back asks whether to save the layout; the gate does not answer it" ;;
        entry) echo "a screen must request text entry" ;;
        keyboard) echo "an application text field must request the keyboard; the ime check covers it" ;;
        ai) echo "an AI prompt must be requested by its service" ;;
        banner) echo "a banner requires an input event or the owner's banner setting" ;;
        displaylab) echo "the display lab requires navigation through Picture settings" ;;
    esac
}
# The documented bar for a kind's first frame, in ms (rust-panels-result.md: a panel opens in under 150 ms), or nothing: the reading is then INFO.
rust_budget() { case $1 in quick|picker|drawer) echo 150 ;; esac; }
# The compositor's counter of frames drawn and of GL errors, from `ctl status`.
rust_drawn() { ctl status | python3 $RUSTJSON stats.drawn; }
# The frames in which the UI engine drew something (the video app's own commits are composed frames too, and are not the shell's).
rust_ui_drawn() { ctl status | python3 $RUSTJSON stats.uiDrawn; }
rust_gl_errors() { ctl status | sed -n 's/.*gl errors \([0-9][0-9]*\).*/\1/p' | head -n 1; }
# What one `ctl shell` takes (ms, to three reads): a first-frame reading is a poll of it, so it is good to about twice this.
rust_poll_ms() { t=$(uptime_cs); for i in 1 2 3; do rust_get connected > /dev/null; done; echo $(( ($(uptime_cs) - t) * 10 / 3 )); }
# The Rust Home's D-pad: Up from where Home's focus is moves it to another layer and Down brings it back. The Rust Home starts on its top layer (the
# status bar), where Up moves nothing: then Down moves the focus away and Up brings it back. Sets DPAD_OK (yes or no), the three layers read
# (DPAD_F0 before, DPAD_F1 after the first key, DPAD_F2 after the second) and DPAD_KEYS (the order pressed).
rust_home_dpad() {
    DPAD_F0=$(rust_get ui.home.focusLayer); key_tap 103; sleep 1
    DPAD_F1=$(rust_get ui.home.focusLayer)
    if [ "$DPAD_F1" = "$DPAD_F0" ]; then
        key_tap 108; sleep 1; DPAD_F1=$(rust_get ui.home.focusLayer)
        key_tap 103; sleep 1; DPAD_F2=$(rust_get ui.home.focusLayer)
        DPAD_KEYS="Down then Up"
    else
        key_tap 108; sleep 1; DPAD_F2=$(rust_get ui.home.focusLayer)
        DPAD_KEYS="Up then Down"
    fi
    if [ "$DPAD_F1" != "$DPAD_F0" ] && [ "$DPAD_F2" = "$DPAD_F0" ]; then DPAD_OK=yes; else DPAD_OK=no; fi
}
# Check only declared native kinds; the gate never changes the running UI configuration.
rust_check_kind() {
    k=$1; RC=ok; RT=""
    if ! rust_answers "$k"; then
        RC=no; RT="native UI not ready or $k absent: ready $(rust_get ready), kinds [$(rust_get ui.kinds)], error '$(rust_get error)'"; return
    fi
    why=$(rust_undriven $k)
    if [ -n "$why" ]; then RC=skip; RT="not driven: $why"; return; fi
    home; sleep 2
    ctl note rust-$k > /dev/null
    path=$(rust_path $k); pm=$(rust_poll_ms)
    d0=$(rust_drawn); g0=$(rust_gl_errors)
    open=no; first=""
    if [ $k = home ]; then
        rust_is_open home "$(rust_get $path)" && open=yes
    else
        t0=$(uptime_cs); end=$((t0 + RUST_OPEN * 100))
        rust_trigger $k
        while [ "$(uptime_cs)" -lt "$end" ]; do
            if rust_is_open $k "$(rust_get $path)"; then open=yes; first=$((($(uptime_cs) - t0) * 10)); break; fi
            sleep 0.05
        done
    fi
    if [ $open = yes ]; then
        sleep 1
        dpad=""
        if [ $k = home ]; then
            # Home navigation must change focus and return to its starting layer.
            rust_home_dpad
            if [ "$DPAD_OK" = yes ]; then dpad="; dpad ok ($DPAD_F0 -> $DPAD_F1 -> $DPAD_F2)"; else RC=no; dpad="; dpad: focus '$DPAD_F0', then '$DPAD_F1' and '$DPAD_F2' after $DPAD_KEYS"; fi
        fi
        ctl shell > $OUT/rust-$k.json
        d1=$(rust_drawn); g1=$(rust_gl_errors)
        if [ -z "$d0" ] || [ -z "$d1" ] || [ -z "$g0" ] || [ -z "$g1" ]; then
            RC=no; RT="missing frame or GL error counters"; drawn=0; gl=0
        else
            drawn=$(( d1 - d0 )); gl=$(( g1 - g0 ))
            RT="${first:+first $first ms (a poll of about $pm ms), }drawn +$drawn, gl errors +$gl$dpad"
        fi
        bud=$(rust_budget $k)
        if [ -n "$bud" ] && [ "${first:-0}" -gt $(( bud + 2 * pm )) ]; then RC=no; RT="$RT; first frame over the $bud ms bar"; fi
        [ "$drawn" -ge 1 ] || { RC=no; RT="$RT; the compositor drew no frame while it was up"; }
        [ "$gl" -eq 0 ] || { RC=no; RT="$RT; GL errors"; }
        if [ $k != home ]; then
            rust_close $k
            n=0; while [ $n -lt $((RUST_OPEN * 2)) ] && rust_is_open $k "$(rust_get $path)"; do sleep 0.5; n=$((n+1)); done
            if rust_is_open $k "$(rust_get $path)"; then RC=no; RT="$RT; still up $RUST_OPEN s after its close"; fi
        fi
    else
        RC=no; RT="not shown within $RUST_OPEN s of the trigger ($path reads '$(rust_get $path)'; can $(rust_get can))"
        [ $k = home ] || rust_close $k
    fi
    journal_after rust-$k > $OUT/rust-$k.txt
}
# What the input method has sent to the fields so far (commits, preedits and keysyms), from the `ctl ime` JSON in file $1.
ime_typed() { c=$(jnum commits < "$1"); p=$(jnum preedits < "$1"); k=$(jnum keysyms < "$1"); echo $(( ${c:-0} + ${p:-0} + ${k:-0} )); }
# Native Home must produce a capture even when LG's Home stub is mounted.
stub_mounted() { grep -c ' /usr/palm/applications/com.webos.app.home ' $MOUNTINFO; }
home_panel_ok() { [ "${1:-0}" -gt 4096 ]; }
# The kernel's audio decoder port an input left open (Open=1, Connect=HDMI_PORTn) with no input app running: Netflix's
# sound then fails with tvq-pb-101 whatever the compositor (lumatv close_input_apps, round 9 result note, Round 10).
held_port() {
    for f in /proc/lgtv-driver/audio/adec.p*; do
        grep -qs '^Open=1' "$f" && sed -n 's/^Connect=\(HDMI_PORT[0-9]*\|ATV\|DTV\).*/\1/p' "$f"
    done | head -n1
}
# The collector keeps prior starts too; checks read only the supervisor's current run.
run_log() { comp_log lumatv-sm | awk '/\] start: /{n=0} {l[n++]=$0} END{for(i=0;i<n;i++) print l[i]}'; }
# SAM (sam.service) is never restarted by anything of LumaTV's: eARC, CEC and the picture mode are lost with it (PLAN,
# "App manager restarts"). Its pid and start time before and after a fallback say it was not.
sam_id() { systemctl show sam.service -p MainPID -p ActiveEnterTimestampMonotonic 2>/dev/null | tr '\n' ' '; }
# After a fallback: wait for the unmount (half seconds, at most $1 of them) and for the recovery of LG's Home.
wait_unmount() { u=0; while is_mounted && [ $u -lt ${1:-40} ]; do sleep 0.5; u=$((u+1)); done; }
wait_recovery() { command -v comp_recovering >/dev/null 2>&1 || return 1; n=0; while comp_recovering && [ $n -lt 120 ]; do sleep 1; n=$((n+1)); done; ! comp_recovering; }
fallback_evidence() {
    fb=$(comp_log lumatv-sm | grep 'fallback:' | tail -1)
    [ -n "$fb" ] || fb=$(cat "$STATE/last-fallback" 2>/dev/null)
    printf '%s\n' "$fb"
}

# ---- the frame rule, as functions the tests source (GATE_LIB=1) ----
# The client's RESULT line as "frames seconds fps b17 b20 b33 bover worst".
x1_numbers() {
    printf '%s\n' "$1" | sed -n 's/.*frames=\([0-9]*\) in \([0-9.]*\) s = \([0-9.]*\) fps; gaps: <=17.5 ms \([0-9]*\), 17.5-20 \([0-9]*\), 20-33.5 \([0-9]*\), >33.5 \([0-9]*\); worst \([0-9.]*\) ms.*/\1 \2 \3 \4 \5 \6 \7 \8/p'
}

# What the compositor's log records over GAP_LIMIT_MS between two monotonic times: a loop pass, a client's frame gap or
# a page-flip interval. The log is on CLOCK_MONOTONIC like the client's; each such line is a frame the panel missed (a
# refresh skipped with nothing due at it is logged as `page-flip interval ... nothing due`, and is not one). With a fourth
# argument (`own`) the client's frame gaps, which are commits that came late, are left out: the compositor's own are the loop's passes
# and the flip intervals.
comp_over_limit() {
    [ -r "$1" ] || { echo unreadable; return 0; }
    awk -v a="$2" -v b="$3" -v lim="$GAP_LIMIT_MS" -v own="${4:-}" '
        /^\[srv/ { t = $2; sub(/\]/, "", t); t += 0; if (t < a || t > b) next }
        /loop: [0-9.]+ ms in one pass/ { v = $0; sub(/.*loop: /, "", v); if (v + 0 > lim) n++ }
        /frame gap [0-9.]+ ms on surface/ { if (own == "") { v = $0; sub(/.*frame gap /, "", v); if (v + 0 > lim) n++ } }
        /page-flip stamp [0-9.]+ ms after the last/ { v = $0; sub(/.*page-flip stamp /, "", v); if (v + 0 > lim) n++ }
        END { print n + 0 }' "$1" 2>/dev/null
}

# The compositor's major page faults, its loop thread's run-queue wait in ns, the thread's scheduling class and priority,
# and the Mali blob's submission thread's (mali-cmar-backe) run-queue wait and class, as "MAJFLT WAIT POLICY PRIO MWAIT
# MPOLICY": what holds the loop without a line of its own (a page read back from swap, the CPU given to another task, the
# submission the loop waits for in a fence export), read before and after a run. Policy 1 is SCHED_FIFO, 0 SCHED_OTHER;
# prio is the kernel's number (-21 for FIFO 20, 20 for nice 0).
comp_sched() {
    c_pid=$(pgrep -f 'bin/lumatv-compositor' | head -1)
    c_mf=$(awk '{print $12}' /proc/$c_pid/stat 2>/dev/null)
    c_w=$(awk '{print $2}' /proc/$c_pid/task/$c_pid/schedstat 2>/dev/null)
    c_pol=$(awk '{print $41}' /proc/$c_pid/task/$c_pid/stat 2>/dev/null)
    c_pri=$(awk '{print $18}' /proc/$c_pid/task/$c_pid/stat 2>/dev/null)
    c_mw=-; c_mpol=-
    for c_t in /proc/$c_pid/task/*; do
        [ "$(cat $c_t/comm 2>/dev/null)" = mali-cmar-backe ] || continue
        c_mw=$(awk '{print $2}' $c_t/schedstat 2>/dev/null); c_mpol=$(awk '{print $41}' $c_t/stat 2>/dev/null); break
    done
    echo "${c_mf:--} ${c_w:--} ${c_pol:--} ${c_pri:--} ${c_mw:--} ${c_mpol:--}"
}

# The loop thread's and the Mali submission thread's waits over a run, from the scheduler's own statistics: how long a wake-up waited for a
# CPU on average and at worst (`schedstat`'s run-queue wait over its time slices; `sched`'s wait_max), and the longest uninterruptible
# (D-state) wait, the kernel wait a lock, the driver or an I/O puts a thread in (`sched`'s block_max, with the count of I/O waits). The detailed
# statistics need `kernel.sched_schedstats`: `waits_reset` turns it on and zeroes them, `waits_line` reads them and puts the switch back.
task_ids() {
    t_pid=$(pgrep -f 'bin/lumatv-compositor' | head -1)
    [ -n "$t_pid" ] || return 1
    echo "$PROC/$t_pid/task/$t_pid"
    for t_t in $PROC/$t_pid/task/*; do
        [ "$(cat $t_t/comm 2>/dev/null)" = mali-cmar-backe ] && { echo "$t_t"; break; }
    done
}
waits_reset() {
    { cat $PROC/sys/kernel/sched_schedstats > $OUT/schedstats.old; } 2>/dev/null
    { echo 1 > $PROC/sys/kernel/sched_schedstats; } 2>/dev/null
    w_n=0
    for w_t in $(task_ids); do
        { echo 0 > $w_t/sched; } 2>/dev/null
        cp $w_t/schedstat $OUT/.waits-$w_n 2>/dev/null
        w_n=$((w_n+1))
    done
}
waits_line() {
    w_n=0; w_out=""
    for w_t in $(task_ids); do
        who=loop; [ $w_n = 1 ] && who=mali-cmar-backe
        s0=$(cat $OUT/.waits-$w_n 2>/dev/null); s1=$(cat $w_t/schedstat 2>/dev/null)
        mean=$(echo "$s0 $s1" | awk 'NF == 6 && $6 > $3 { printf "%.0f", ($5 - $2) / ($6 - $3) / 1000; exit } { print "-" }')
        st=$(awk -F: '/se.statistics.block_max/ { b = $2 + 0; s = 1 } /se.statistics.wait_max/ { w = $2 + 0 } /se.statistics.iowait_count/ { i = $2 + 0 } END { if (s) printf "D-wait max %.1f ms (%d I/O), run-queue wait max %.1f ms", b, i, w; else printf "no per-wait statistics" }' $w_t/sched 2>/dev/null)
        w_out="$w_out${w_out:+; }$who: run-queue wait ${mean} us a wake-up on average, $st"
        w_n=$((w_n+1))
    done
    old=$(cat $OUT/schedstats.old 2>/dev/null); [ -n "$old" ] && { echo "$old" > $PROC/sys/kernel/sched_schedstats; } 2>/dev/null
    echo "${w_out:-no compositor}"
}

# The crash bundle the supervisor saved for the last fallback (`<state>/../crash`, compositor/sm/lumatv-sm.c): its name, whether `pending` names it (what
# Settings shows as "LumaTV recovered from a problem"), and what it holds: the compositor's last log lines, the trace ring's lines, the kind.
crash_summary() {
    c_dir=${LUMATV_CRASH:-$STATE/../crash}
    c_name=$(cat $c_dir/latest 2>/dev/null)
    [ -n "$c_name" ] || { echo "none"; return; }
    c_b=$c_dir/$c_name
    echo "$c_name (pending $([ "$(cat $c_dir/pending 2>/dev/null)" = "$c_name" ] && echo yes || echo no); $(grep -c . $c_b/compositor.log.tail 2>/dev/null) log lines, $(grep -c . $c_b/trace.txt 2>/dev/null) ring lines, kind $(sed -n 's/^kind: //p' $c_b/reason.txt 2>/dev/null))"
}

# x1_rule RESULT SECONDS_ASKED LATE_BEFORE LATE_AFTER COMPOSITOR_OVER [ATTRIBUTION [COMPOSITOR_OWN]]: prints "PASS what held" or "FAIL why",
# returns 0 or 1. COMPOSITOR_OVER is a count, or "unreadable". The rule as the PLAN has it (no gap over 20 ms in the client's swaps, the
# compositor's log clean of every kind) is the strict one. With ATTRIBUTION, "A C U" from `kwait.py --x1` over a traced run (A gaps and late flips
# that are the compositor's, C gaps that are the client's own wake-up after callbacks that came on time, U gaps it could not explain), only A
# and U fail: C are reported and not counted. Then COMPOSITOR_OWN, the compositor's log without the client's frame gaps (`comp_over_limit ... own`),
# stands for COMPOSITOR_OVER. The run's length, its fps and the pacer's late counter are the same in both.
x1_rule() {
    r_res=$1; r_asked=${2:-0}; r_lb=${3:-}; r_la=${4:-}; r_comp=${5:-}; r_att=${6:-}; r_own=${7:-}
    r_n=$(x1_numbers "$r_res")
    if [ -z "$r_n" ]; then echo "FAIL the X1 client printed no result"; return 1; fi
    case "$r_asked" in ''|*[!0-9]*) r_asked=0 ;; esac
    set -- $r_n
    r_secs=$2; r_fps=$3; r_over=$(( $6 + $7 )); r_worst=$8
    r_ours=0; r_unex=0; r_cli=0
    [ -z "$r_att" ] || { set -- $r_att; r_ours=${1:-0}; r_cli=${2:-0}; r_unex=${3:-0}; }
    r_why=""
    if [ -z "$r_att" ]; then
        if [ "$r_over" -gt 0 ]; then r_why="$r_why; $r_over gap(s) over ${GAP_LIMIT_MS} ms in the client's swaps (worst $r_worst ms)"; fi
    else
        if [ "$r_ours" -gt 0 ]; then r_why="$r_why; $r_ours gap(s) or late flip(s) are the compositor's (a late flip, the loop slow to take one up, a wait of the loop's) in a run with $r_over gap(s) over ${GAP_LIMIT_MS} ms in the client's swaps (worst $r_worst ms)"; fi
        if [ "$r_unex" -gt 0 ]; then r_why="$r_why; $r_unex gap(s) over ${GAP_LIMIT_MS} ms in the client's swaps the trace cannot explain (worst $r_worst ms)"; fi
    fi
    if [ "$r_asked" -lt "$GATE_SECONDS" ]; then r_why="$r_why; the run asked for $r_asked s, the rule is ${GATE_SECONDS} s"; fi
    if awk -v s="$r_secs" -v m="$GATE_SECONDS" 'BEGIN { exit !(s + 0 < m - 0.5) }'; then r_why="$r_why; the run lasted $r_secs s"; fi
    if awk -v f="$r_fps" -v m="$MIN_FPS" 'BEGIN { exit !(f + 0 < m + 0) }'; then r_why="$r_why; $r_fps fps is under $MIN_FPS"; fi
    if [ -z "$r_lb" ] || [ -z "$r_la" ]; then r_why="$r_why; the pacer's late counter was not read"
    elif [ "$r_lb" != "$r_la" ]; then r_why="$r_why; pacer late $r_lb -> $r_la"; fi
    if [ -z "$r_att" ]; then
        case "$r_comp" in
        ''|*[!0-9]*) r_why="$r_why; the compositor's log of the run could not be read" ;;
        0) ;;
        *) r_why="$r_why; the compositor logged $r_comp loop pass(es), client frame gap(s) or page-flip interval(s) over ${GAP_LIMIT_MS} ms during the run" ;;
        esac
    else
        case "$r_own" in
        ''|*[!0-9]*) r_why="$r_why; the compositor's log of the run could not be read" ;;
        0) ;;
        *) r_why="$r_why; the compositor logged $r_own loop pass(es) or page-flip interval(s) over ${GAP_LIMIT_MS} ms during the run" ;;
        esac
    fi
    if [ -n "$r_why" ]; then echo "FAIL ${r_why#; }"; return 1; fi
    if [ -z "$r_att" ]; then
        echo "PASS $r_fps fps over $r_secs s, no gap over ${GAP_LIMIT_MS} ms, pacer late unchanged, nothing over ${GAP_LIMIT_MS} ms in the compositor's log"
        return 0
    fi
    r_note=""
    if [ "$r_over" -gt 0 ]; then r_note="; $r_cli client-only of $r_over gap(s) over ${GAP_LIMIT_MS} ms (worst $r_worst ms) reported, not counted: the client woke late on callbacks that came on time"; fi
    case "$r_comp$r_own" in *[!0-9]*|'') ;; *) if [ "$r_comp" -gt "$r_own" ]; then r_note="$r_note; the compositor logged $(( r_comp - r_own )) client frame gap(s) (late commits)"; fi ;; esac
    echo "PASS $r_fps fps over $r_secs s, no compositor-attributable gap over ${GAP_LIMIT_MS} ms, pacer late unchanged, no loop pass or page-flip interval over ${GAP_LIMIT_MS} ms in the compositor's log$r_note"
    return 0
}

# Raw strict evidence is never replaced by attribution. A missing/incomplete capture is inconclusive, not a smooth-run pass.
# RESULT SECS LATE_BEFORE LATE_AFTER COMPOSITOR_OVER ATTRIBUTION COMPOSITOR_OWN COVERAGE STRICT
x1_decision() {
    d_res=$1; d_secs=$2; d_before=$3; d_after=$4; d_comp=$5; d_att=$6; d_own=$7; d_cover=$8; d_strict=${9:-}
    d_raw=$(x1_rule "$d_res" "$d_secs" "$d_before" "$d_after" "$d_comp")
    if [ -n "$d_strict" ]; then
        echo "$d_raw | RAW RESULT: $d_res"
        case "$d_raw" in PASS*) return 0 ;; *) return 1 ;; esac
    fi
    if [ "$d_cover" != COMPLETE ] || [ -z "$d_att" ]; then
        echo "INCONCLUSIVE attribution: missing or incomplete trace ($d_cover) | RAW STRICT: $d_raw | RAW RESULT: $d_res"
        return 2
    fi
    d_judged=$(x1_rule "$d_res" "$d_secs" "$d_before" "$d_after" "$d_comp" "$d_att" "$d_own")
    echo "$d_judged | RAW STRICT: $d_raw | RAW RESULT: $d_res"
    case "$d_judged" in PASS*) return 0 ;; *) return 1 ;; esac
}

# ime_rule METHODS UP CONTEXT TYPED CLOSED BOTTOM: "PASS what held" or "FAIL why" (returns 0 or 1). METHODS is how many input methods were
# bound, UP, CONTEXT, CLOSED and BOTTOM are yes or no, TYPED is how many commits, preedits and keysyms came out at the field.
ime_rule() {
    i_why=""
    [ "${1:-0}" -ge 1 ] 2> /dev/null || i_why="$i_why; no input method was bound"
    [ "${2:-no}" = yes ] || i_why="$i_why; the keyboard's window never came on the screen"
    [ "${3:-no}" = yes ] || i_why="$i_why; the field had no context at the input method"
    [ "${4:-0}" -ge 1 ] 2> /dev/null || i_why="$i_why; nothing typed on the keyboard reached the field"
    [ "${5:-no}" = yes ] || i_why="$i_why; the keyboard did not close"
    [ "${6:-no}" = yes ] || i_why="$i_why; the keyboard's window is not at the bottom of the screen"
    if [ -n "$i_why" ]; then echo "FAIL ${i_why#; }"; return 1; fi
    echo "PASS an input method bound, the field's context made, the keyboard's window at the bottom, a key typed on it reached the field, the keyboard closed"
    return 0
}

if [ -n "${GATE_LIB:-}" ]; then return 0 2>/dev/null || exit 0; fi

# The video check (`gate.sh video`): what the set shows and what the compositor costs while a video app plays full screen.
PNGSTAT=$D/pngstat.py
FBDEV=${LUMATV_FBDEV:-/proc/lg/fbdev}
MALI_UTIL=${LUMATV_MALI_UTIL:-/sys/kernel/debug/mali0/dvfs_utilization}
# A line of $FBDEV/fbN/status by its key (" hdr.global_alpha_en: 0"), the OSD's flip count, Mali's busy and idle times, a process's CPU ticks.
fb_field() { sed -n "s/^ *$2 *[:=] *//p" $FBDEV/fb$1/status 2>/dev/null | head -n 1 | tr -d ' \r'; }
fb_flips() { tr -dc '0-9' < $FBDEV/fb0/flip_cnt 2>/dev/null; }
mali_count() { sed -n "s/.*$1:[ ]*\([0-9]*\).*/\1/p" $MALI_UTIL 2>/dev/null | head -n 1; }
cpu_ticks() { awk '{ print $14 + $15 }' $PROC/$1/stat 2>/dev/null; }
per_s() { awk -v a="${1:-0}" -v b="${2:-0}" -v s="$3" 'BEGIN { printf "%.1f", (b - a) / s }'; }
# Whether a window of the compositor has exported a video element (a hole, or one expected): `ctl windows` lists them under "video".
video_up() { ctl windows | grep -o '"video":\[[^]]*\]' | head -n 1 | grep -q '{'; }
# The compositor's work over SECS seconds of the screen as it is: VS_CPU (percent of one core), VS_DRAWN (frames composed a second), VS_UI (the frames
# a second in which the shell's UI engine drew something: `stats.uiDrawn`), VS_FLIPS (the OSD's page flips a second) and VS_MALI (the GPU's busy
# share, percent). With nothing over a video the shell has nothing to draw; the frames composed are the video app's own commits.
video_sample() {
    vs_pid=$(pgrep -f 'bin/lumatv-compositor' | head -n 1)
    vs_hz=$(getconf CLK_TCK 2>/dev/null || echo 100)
    c0=$(cpu_ticks "${vs_pid:-self}"); d0=$(ctl status | python3 $RUSTJSON stats.drawn); u0=$(rust_ui_drawn); f0=$(fb_flips); b0=$(mali_count busy_time); i0=$(mali_count idle_time)
    sleep "$1"
    c1=$(cpu_ticks "${vs_pid:-self}"); d1=$(ctl status | python3 $RUSTJSON stats.drawn); u1=$(rust_ui_drawn); f1=$(fb_flips); b1=$(mali_count busy_time); i1=$(mali_count idle_time)
    VS_CPU=$(awk -v a="${c0:-0}" -v b="${c1:-0}" -v hz="$vs_hz" -v s="$1" 'BEGIN { printf "%.1f", (b - a) * 100 / hz / s }')
    VS_DRAWN=$(per_s "$d0" "$d1" "$1"); VS_UI=$(per_s "$u0" "$u1" "$1"); VS_FLIPS=$(per_s "$f0" "$f1" "$1")
    VS_MALI=$(awk -v b0="${b0:-0}" -v b1="${b1:-0}" -v i0="${i0:-0}" -v i1="${i1:-0}" 'BEGIN { t = (b1 - b0) + (i1 - i0); if (t > 0) printf "%.1f", 100 * (b1 - b0) / t; else print "?" }')
}
# A DISPLAY capture (LG's capture service: the blend of the video plane and the OSD, what the panel shows) of the region as pngstat.py reads it:
# prints its line, `black=yes` where nothing shows (the display engine's opaque black under a video hole).
video_look() { f=$1; shift; b=$(cap "$f"); [ "${b:-0}" -gt 0 ] || { echo "w=0 h=0 mean=0 std=0 bright=0 black=yes error=no_capture"; return; }; python3 $PNGSTAT "$f" "$@"; }
# The first number of a pngstat line's key.
ps_val() { echo "$1" | tr ' ' '\n' | sed -n "s/^$2=//p" | head -n 1; }

case "${1:-help}" in
live)
    name=live${2:+-$2}
    m=$(is_mounted && echo yes || echo no)
    regs=$(run_log | grep -c '^\[.*\] registered com\.')
    fb=$(run_log | grep -c 'fallback:')
    wam=$(systemctl is-active webapp-mgr)
    ctl windows > $OUT/live-windows.json
    homewin=$(grep -c '"com.webos.app.home"' $OUT/live-windows.json)
    b=$(cap $OUT/$name.png)
    # The watchdog's inputs: the compositor's counters and windows (the frozen-output and empty-panel rules) and
    # tvpower's state (the set being on), each logged by the supervisor when it first read them.
    wd=$(run_log | grep -c 'watchdog: progress rules on (frozen output watched, empty panel watched)')
    pw=$(run_log | grep -c 'watchdog: the set is ')
    detail="mounted $m registered $regs fallbacks $fb wam $wam home-window $homewin home-stub $(stub_mounted) panel ${b:-0} bytes fg $(fg) watchdog $wd power-seen $pw"
    if [ "$m" = yes ] && [ "$regs" -ge 12 ] && [ "$fb" = 0 ] && [ "$wam" = active ] && [ "$homewin" -ge 1 ] && home_panel_ok "$b" && [ "$wd" -ge 1 ] && [ "$pw" -ge 1 ]; then pass $name "$detail"; else fail $name "$detail"; fi
    ;;
x1)
    secs=${2:-60}
    before=$(ctl status | sed -n 's/.*\(flips [0-9]* late [0-9]* lost [0-9]*\).*/\1/p')
    sched_before=$(comp_sched)
    ctl wake reset > /dev/null; waits_reset
    # LUMATV_X1_CALLBACKS=commit|early|landing (tv_gate.py --x1-callbacks) sets when every client's frame callbacks go out for this run
    # (`ctl callbacks clients`): with the commit (the compositor's default since round 19), with the frame's draw (early) or at the flip
    # (landing), and the default again after it.
    cbnote=""
    if [ -n "${LUMATV_X1_CALLBACKS:-}" ]; then
        r_cb=$(ctl callbacks clients "$LUMATV_X1_CALLBACKS")
        case "$r_cb" in *'"ok":true'*) cbnote="; the clients' callbacks $LUMATV_X1_CALLBACKS" ;; *) cbnote="; the clients' callbacks NOT set to $LUMATV_X1_CALLBACKS ($r_cb)" ;; esac
    fi
    # With LUMATV_GATE_KWAIT set (tv_gate.py --kwait) and capture.sh and kwait.py on the set: the run is traced and each late frame's kernel
    # waits are named (capture.sh's kwait mode: the compositor's own scheduler events and the stacks of its D-state sleeps). The client's own gaps
    # are no late frame while the flips are on time: kwait.py also says how long after each flip the client's next GPU job began, and sets the
    # client's gap lines (`--x1`) beside the gaps that implies. The loop thread is the one capture.sh marked (`loop TID`).
    kw=""
    if [ -n "${LUMATV_GATE_KWAIT:-}" ] && [ -f /tmp/perf/capture.sh ] && [ -f /tmp/perf/kwait.py ]; then
        sh /tmp/perf/capture.sh start x1 kwait > $OUT/x1-capture.txt 2>&1 && kw=1
    fi
    XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0 X1_SECONDS=$secs X1_WINDOW_TYPE=_WEBOS_WINDOW_TYPE_OVERLAY X1_RT=${LUMATV_X1_RT:-} \
        $LD --library-path $LIB --preload $LIB/x1heapshim.so $D/bin/x1client $BLOB > $OUT/x1.log 2>&1
    [ -z "${LUMATV_X1_CALLBACKS:-}" ] || ctl callbacks clients commit > /dev/null
    res=$(grep RESULT $OUT/x1.log)
    after=$(ctl status | sed -n 's/.*\(flips [0-9]* late [0-9]* lost [0-9]*\).*/\1/p')
    sched=$(echo "$sched_before $(comp_sched)" | awk 'function pol(v) { return (v == 1) ? "SCHED_FIFO" : (v == 0) ? "SCHED_OTHER" : "policy " v }
        { f = ($1 ~ /^[0-9]+$/ && $7 ~ /^[0-9]+$/) ? sprintf("+%d", $7 - $1) : "not read"; w = ($2 ~ /^[0-9]+$/ && $8 ~ /^[0-9]+$/) ? sprintf("+%.1f ms", ($8 - $2) / 1e6) : "not read";
          mw = ($5 ~ /^[0-9]+$/ && $11 ~ /^[0-9]+$/) ? sprintf("+%.1f ms", ($11 - $5) / 1e6) : "not read";
          print "major faults " f ", run-queue wait " w ", loop thread " pol($9) " prio " $10 ", mali-cmar-backe run-queue wait " mw " " pol($12) }')
    wakeline=$(ctl wake | jstr line); waitsline=$(waits_line)
    kwsum=""
    if [ -n "$kw" ]; then
        if sh /tmp/perf/capture.sh stop x1 >> $OUT/x1-capture.txt 2>&1 &&
            python3 /tmp/perf/kwait.py /tmp/perf/x1.trace --x1 $OUT/x1.log --json $OUT/x1-kwait.json > $OUT/x1-kwait.txt 2>&1; then
            kwsum=$(tail -n 1 $OUT/x1-kwait.txt)
        else
            kw=""; kwsum="x1 attribution INCONCLUSIVE: capture or analysis failed"
        fi
        gzip -c /tmp/perf/x1.trace > $OUT/x1.trace.gz 2>/dev/null; rm -f /tmp/perf/x1.trace
    fi
    # `late` alone: `lost` is the pacer's own measure, which leaves a frame's render time out.
    late_before=$(echo "$before" | awk '{print $4}'); late_after=$(echo "$after" | awk '{print $4}')
    # The compositor's own view of the run (its log is on CLOCK_MONOTONIC like x1's): loop stalls, frame gaps of the
    # X1 surface, clients coming and going, refused commits, lost flips, a flip the loop reached late and where the loop
    # was then, a frame held while a driver buffer warms, a surface stood in for, each buffer's warm, buffers freed. The
    # gaps' cause is read from these lines; only the ones over the limit count against the rule (comp_over_limit).
    t0=$(sed -n 's/.*starting at monotonic \([0-9.]*\).*/\1/p' $OUT/x1.log); t1=$(sed -n 's/.*ended at monotonic \([0-9.]*\).*/\1/p' $OUT/x1.log)
    comp_log lumatv-compositor | awk -v a="${t0:-0}" -v b="${t1:-0}" '/^\[srv/ { t = $2; sub(/\]/, "", t); t += 0; if (t >= a && t <= b + 1 && $0 ~ /loop:|frame gap|client (dis)?connected|atomic commit|page-flip|callbacks held|flip reached|frame held|warm done|warm:|standin:|trace:|bury:|tick:|boxdim:/) print }' \
        > $OUT/x1-compositor.txt
    stalls=$(grep -c 'loop: [0-9.]* ms in one pass' $OUT/x1-compositor.txt); gaps=$(grep -c 'frame gap' $OUT/x1-compositor.txt); churn=$(grep -c 'client connected' $OUT/x1-compositor.txt)
    # What counts against the rule is what the log says over the limit while the client was alive (not the second after).
    comp_over=$(comp_over_limit $OUT/x1-compositor.txt "${t0:-0}" "${t1:-0}")
    # Keep the original strict verdict and raw swaps even when a complete trace attributes gaps to the client.
    raw_strict=$(x1_rule "$res" "$secs" "$late_before" "$late_after" "$comp_over")
    echo "$raw_strict" > $OUT/x1-strict-result.txt
    attrib=""; own=""; coverage="missing capture"
    if [ -n "$kw" ]; then
        coverage=$(sed -n 's/^x1 attribution: //p' $OUT/x1-kwait.txt 2>/dev/null | head -n 1)
        attrib=$(sed -n 's/^x1 gaps: .*compositor-attributable \([0-9]*\) .*client-only \([0-9]*\), unexplained \([0-9]*\)$/\1 \2 \3/p' $OUT/x1-kwait.txt 2>/dev/null | head -n 1)
        [ -z "$attrib" ] || own=$(comp_over_limit $OUT/x1-compositor.txt "${t0:-0}" "${t1:-0}" own)
    fi
    detail="RAW STRICT: $raw_strict | RAW RESULT: $res | attribution coverage: $coverage | pacer before: $before after: $after | compositor: loop stalls $stalls, frame gaps $gaps, clients connected $churn, over ${GAP_LIMIT_MS} ms $comp_over${attrib:+ (the loop's and the flips' own: $own)}; $sched${LUMATV_X1_RT:+; the client SCHED_FIFO $LUMATV_X1_RT}$cbnote${LUMATV_X1_STRICT:+; the strict rule} | wake-up latency: ${wakeline:-not read} | waits: $waitsline${kwsum:+ | $kwsum}"
    echo "$detail" > $OUT/x1-result.txt
    verdict=$(x1_decision "$res" "$secs" "$late_before" "$late_after" "$comp_over" "$attrib" "$own" "$coverage" "${LUMATV_X1_STRICT:-}")
    case "$verdict" in PASS*) pass x1 "${verdict#PASS }" ;; *) fail x1 "${verdict#FAIL } | $detail" ;; esac
    ;;
netflix)
    # A Netflix instance that was preloaded under another compositor (or while this one's services were still
    # coming up) keeps what it bound at its start and shows tvq-pb-101 at play (third and fifth gates; the same
    # error under LG's compositor after declutter's runtime changes). The check is a fresh start: a running
    # instance is ended and the preload manager loads a new one, which the launch then takes.
    closed=0
    held=$(held_port)
    if app_running netflix; then
        close_app netflix
        closed=1
        wait_app_gone netflix 15
        sleep 5
    fi
    ls1 com.webos.applicationManager/launch '{"id":"netflix","params":{"contentTarget":"m=https://www.netflix.com/watch/80141336&source_type=4"}}' > /dev/null
    sleep 12
    ctl remote 28 tap > /dev/null
    sleep 25
    ctl windows > $OUT/netflix-windows.json
    ctl journal > $OUT/netflix-journal.json
    card=$(grep -c '"netflix"' $OUT/netflix-windows.json)
    video=$(grep -o '"video":\[[^]]*\]' $OUT/netflix-windows.json | head -c 300)
    placements=$(grep -o 'setDisplayWindow\|setCropRegion' $OUT/netflix-journal.json | wc -l)
    drops=$(comp_log lumatv-compositor | grep -c 'message too short\|error in client communication')
    pm=$(picture_mode)
    fgp=$(fg)
    b=$(cap $OUT/netflix.png)
    ls1 com.webos.applicationManager/closeByAppId '{"id":"netflix"}' > /dev/null
    sleep 2; home; sleep 6
    pm2=$(picture_mode)
    detail="netflix windows $card video $video placements $placements protocol errors $drops pictureMode playing $pm after $pm2 fg playing $fgp now $(fg) panel ${b:-0} bytes stale-instance-closed $closed audio-port-held-before ${held:-none}"
    echo "$detail" > $OUT/netflix-result.txt
    if [ "$card" -ge 1 ] && [ "$pm" = dolbyHdrCinema ] && [ "$pm2" = filmMaker ]; then pass netflix "$detail"; else fail netflix "$detail"; fi
    ;;
hdmi)
    ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.hdmi2"}' > /dev/null
    sleep 10
    ctl windows > $OUT/hdmi-windows.json
    ctl journal > $OUT/hdmi-journal.json
    card=$(grep -c '"com.webos.app.hdmi2"' $OUT/hdmi-windows.json)
    # serde_json writes keys in order: appId, exposed, front, id, inGroup, keyMask, mapped (the props, with braces, follow).
    mapped=$(grep -o '"appId":"com.webos.app.hdmi2"[^}]*"mapped":true' $OUT/hdmi-windows.json | wc -l)
    bare=$(grep -o 'committed without a buffer ([A-Za-z]* app \\"com.webos.app.hdmi2' $OUT/hdmi-journal.json | wc -l)
    f=$(fg)
    b=$(cap $OUT/hdmi.png)
    home; sleep 6
    # Ended through SAM: left behind Home, the input dies with the compositor in wd-hang, wd-crash or escape and its
    # audio decoder port stays open for the next gate's Netflix.
    ls1 com.webos.applicationManager/closeByAppId '{"id":"com.webos.app.hdmi2"}' > /dev/null
    n=0; while [ $n -lt 10 ] && [ -n "$(held_port)" ]; do sleep 1; n=$((n+1)); done
    held=$(held_port)
    detail="hdmi2 windows $card mapped $mapped bufferless commits $bare fg while open $f now $(fg) panel ${b:-0} bytes audio-port-held-after-close ${held:-none}"
    echo "$detail" > $OUT/hdmi-result.txt
    if [ "$mapped" -ge 1 ] && [ "$f" = com.webos.app.hdmi2 ]; then pass hdmi "$detail"; else fail hdmi "$detail"; fi
    ;;
apps)
    # YouTube (cobalt) and PlxNative (LG's SDL) died under LumaTV's compositor while every other app ran (2026-10-28): Cobalt wants
    # touch on every seat and no wl_seat.name, LG's SDL wants wl_starfish_pointer, and PlxNative's close needs the frame callback of
    # its last frame before SDL destroys its surface (it crashed 0.5 to 1 s after the close otherwise, SIGSEGV 11). Each is launched
    # from a fresh start: its card mapped and SAM naming it foreground within 5 s, still running and mapped 10 s later with its
    # connection kept, ended through SAM within 15 s, SAM logging no exit status for its pid, and no new report in the app's own crash
    # log 3 s after the close. YouTube may play sound; the driver muted the receiver. Then NVIDIA's GeForce NOW, which opens LG's
    # browser and ends itself: Home launched over the browser stays in front (the browser took the front back the same day). A later
    # app that fails the same way is added here.
    # LUMATV_APPS_REPEAT=N (tv_gate.py --apps-repeat N) runs the YouTube-then-PlxNative pair N times (round 17: PlxNative's close crashed in 9 of 10
    # after YouTube): every pass must be clean, a failing one names its number, and the evidence files of a pass carry `-rN`.
    bad=""; kd=""; reps=${LUMATV_APPS_REPEAT:-1}; rep=1
    while [ "$rep" -le "$reps" ]; do
        sfx=""; tag=""; [ "$reps" -le 1 ] || { sfx="-r$rep"; tag="[pass $rep of $reps] "; }
        for app in youtube.leanback.v4 com.beb.plxnative; do
            if app_running "$app"; then close_app "$app"; wait_app_gone "$app" 15; sleep 3; fi
            ctl note apps-$app$sfx > /dev/null
            log=$(crash_log "$app"); reports=$(crash_reports "$log")
            exit_watch_start "$app" $OUT/apps-$app$sfx.exit
            ls1 com.webos.applicationManager/launch "{\"id\":\"$app\"}" > /dev/null
            n=0; up=none
            while [ $n -lt 10 ]; do
                if app_mapped "$app" && [ "$(fg)" = "$app" ]; then up=$((n * 500)); break; fi
                sleep 0.5; n=$((n+1))
            done
            # Its pid from the compositor's own connection line (an app that died is already off SAM's list), SAM's as the fallback.
            pid=$(journal_after apps-$app$sfx | sed -n "s/^client connected: pid \([0-9]*\) .*$app.*/\1/p" | head -n 1)
            [ -n "$pid" ] || pid=$(ls1 com.webos.applicationManager/running '{}' | tr -d ' \n' | sed -n "s/.*\"id\":\"$app\",\"processid\":\"\([0-9]*\)\".*/\1/p")
            sleep 10
            alive=no; app_running "$app" && app_mapped "$app" && alive=yes
            dropped=$(journal_after apps-$app$sfx | grep -c "client disconnected: pid ${pid:-none} ")
            closed_cs=$(uptime_cs)
            close_app "$app"
            gone=no; wait_app_gone "$app" 15 && gone=yes
            exit_wait $OUT/apps-$app$sfx.exit "${pid:-none}" 5
            exit_watch_stop "$app" $OUT/apps-$app$sfx.exit
            died=$(exit_status_of $OUT/apps-$app$sfx.exit "$pid")
            crashed=0
            if [ -n "$log" ]; then
                settle_after "$closed_cs"
                now=$(crash_reports "$log")
                # A log that shrank was started anew: all of it is new.
                crashed=$((now >= reports ? now - reports : now))
            fi
            journal_after apps-$app$sfx > $OUT/apps-$app$sfx.txt
            what="$tag$app pid ${pid:-none} foreground-in-ms ${up} alive-at-10s $alive connection-lost $dropped closed-in-15s $gone${died:+ $died}"
            [ "$crashed" = 0 ] || what="$what crash-reports +$crashed"
            if [ "$up" != none ] && [ "$alive" = yes ] && [ "$dropped" = 0 ] && [ "$gone" = yes ] && [ -z "$died" ] && [ "$crashed" = 0 ]; then
                kd="$kd $what;"
            else
                bad="$bad $what;"
            fi
            home; sleep 4
        done
        rep=$((rep + 1))
    done
    if ls1 com.webos.applicationManager/getAppInfo '{"id":"com.geforcenow.play"}' | tr -d ' \n' | grep -q '"returnValue":true'; then
        for a in com.webos.app.browser com.geforcenow.play; do
            if app_running $a; then close_app $a; wait_app_gone $a 15; sleep 3; fi
        done
        ctl note apps-geforcenow > /dev/null
        ls1 com.webos.applicationManager/launch '{"id":"com.geforcenow.play"}' > /dev/null
        n=0; br=no
        while [ $n -lt 16 ]; do
            if [ "$(fg)" = com.webos.app.browser ]; then br=yes; break; fi
            sleep 0.5; n=$((n+1))
        done
        # The browser's own requests for fullscreen start about 3.4 s after the launch; Home comes 8 s after it, as on 2026-10-28.
        sleep 4
        home; sleep 6
        f=$(fg)
        journal_after apps-geforcenow > $OUT/apps-geforcenow.txt
        close_app com.webos.app.browser; close_app com.geforcenow.play
        wait_app_gone com.webos.app.browser 15
        home; sleep 3
        what="com.geforcenow.play browser-in-front $br front-6s-after-home ${f:-none}"
        if [ "$br" = yes ] && [ "$f" = com.webos.app.home ]; then kd="$kd $what;"; else bad="$bad $what;"; fi
    else
        kd="$kd com.geforcenow.play not installed;"
    fi
    detail="${bad:+failed:$bad }ok:${kd:- none}"
    echo "$detail" > $OUT/apps-result.txt
    if [ -z "$bad" ]; then pass apps "$detail"; else fail apps "$detail"; fi
    ;;
keys)
    # The remote's own keys under LumaTV's compositor, each injected through `ctl remote` (the path a key from the remote's device takes)
    # and judged by what the compositor's journal says it decided and what SAM then has in front. Home is in front at the start and at the
    # end, and every app a key opened is closed again. Mute held is left to the host tests: it would unmute the receiver the gate holds
    # muted if it went wrong. The gear's three taps and its hold open nothing of LG's (host tests in tests/keys.rs).
    kd=""; bad=""
    res() { if [ "$1" = ok ]; then kd="$kd $2 ok;"; else bad="$bad $2 ($3);"; fi; }
    home; sleep 3
    # The native quick panel must be ready; LG Settings is not a substitute.
    ctl note keys-gear > /dev/null
    if rust_answers quick; then
        key_tap 139
        if wait_panel quick open 10 && journal_after keys-gear | grep -q "input: gear tap: Quick Settings -> LumaTV's quick panel"; then res ok gear-shell
        else res no gear-shell "quick panel is '$(rust_get ui.panels.quick)'; or no journal line"; fi
        key_tap 412; sleep 2
    else
        res no gear-shell "native quick panel is not ready"
    fi
    journal_after keys-gear > $OUT/keys-gear.txt
    # Input's tap must open the native input picker.
    ctl note keys-input > /dev/null
    if rust_answers picker; then
        key_tap 241
        if wait_panel picker open 10 && journal_after keys-input | grep -q "input: Input tap: the input picker -> LumaTV's input picker"; then res ok input-shell
        else res no input-shell "picker is '$(rust_get ui.panels.picker)'; or no journal line"; fi
        key_tap 412; sleep 2
    else
        res no input-shell "native input picker is not ready"
    fi
    journal_after keys-input > $OUT/keys-input.txt
    # Remote choices are typed actions, not raw app IDs. Judge visible results rather than diagnostic wording.
    nk=$(ctl keys | grep -o '"netflix":"[^"]*"' | head -n 1)
    nsupported=yes; napp=""
    if [ -z "$nk" ]; then napp=netflix; nb=netflix-button
    else
        naction=$(echo "$nk" | cut -d'"' -f4)
        case "$naction" in
            app:*) napp=${naction#app:}; nb="netflix-button(chosen: $napp)" ;;
            nothing) nb="netflix-button(chosen: nothing)" ;;
            *) nsupported=no; nb="netflix-button(chosen: $naction)" ;;
        esac
    fi
    ctl note keys-netflix > /dev/null
    if [ "$nsupported" = no ]; then res no "$nb" "the gate cannot judge this chosen action"
    else
        n_before=$(mapped_apps | sort -u)
        key_tap 1037
        if [ -z "$napp" ]; then
            sleep 3
            if [ "$(fg)" = com.webos.app.home ] && [ "$(mapped_apps | sort -u)" = "$n_before" ]; then res ok "$nb"
            else res no "$nb" "it opens nothing, yet $(where_now)"; fi
        elif wait_app "$napp" 25 && { [ "$(fg)" = "$napp" ] || ! printf '%s\n' "$n_before" | grep -Fqx -- "$napp"; }; then res ok "$nb"
        else res no "$nb" "$(where_now)"; fi
    fi
    journal_after keys-netflix > $OUT/keys-netflix.txt
    [ -z "$napp" ] || close_app "$napp"
    sleep 4; home; sleep 4
    # A digit: tapped it is the app's (nothing asks for Quick Access), held a second it asks for it.
    ctl note keys-digit > /dev/null; key_tap 6; sleep 2
    tapped=$(journal_after keys-digit | grep -c 'input: 5 held')
    ctl note keys-digit-hold > /dev/null; key_hold 6 1.4; sleep 3
    if [ "$tapped" = 0 ]; then res ok digit-tap; else res no digit-tap "the tap asked for Quick Access"; fi
    if journal_after keys-digit-hold | grep -q 'input: 5 held: Quick Access slot 5 .*launching com.webos.app.magicnum'; then res ok digit-hold
    else res no digit-hold "no journal line"; fi
    journal_after keys-digit-hold > $OUT/keys-digit.txt
    # The held digit's press also reaches SAM, which launches the slot's app (com.webos.app.browser, 0.63 s into the hold on the set
    # on 2026-10-27) beside Quick Access; left open it stays in front of Home for every check that follows.
    close_app com.webos.app.magicnum; close_app com.webos.app.browser; sleep 2; home; sleep 3
    # Exit: the app that does not take it is closed and the TV comes back to Home.
    ls1 com.webos.applicationManager/launch '{"id":"com.palm.app.settings"}' > /dev/null
    wait_app com.palm.app.settings 25; sleep 2
    ctl note keys-exit > /dev/null; key_tap 174
    n=0; while app_mapped com.palm.app.settings && [ $n -lt 10 ]; do sleep 1; n=$((n+1)); done
    if ! app_mapped com.palm.app.settings && journal_after keys-exit | grep -q 'input: Back or Exit: closing com.palm.app.settings'; then res ok exit
    else res no exit "$(where_now); or no journal line"; fi
    journal_after keys-exit > $OUT/keys-exit.txt
    close_app com.palm.app.settings; sleep 2; home; sleep 2
    ctl keys > $OUT/keys.json
    names=$(grep -o '"code":[0-9]*,"name":"[^"]*"' $OUT/keys.json | sed 's/"code":\([0-9]*\),"name":"\([^"]*\)"/\2=\1/' | tr '\n' ' ')
    unnamed=$(grep -o '"unnamed":\[[^]]*\]' $OUT/keys.json | sed 's/"unnamed"://')
    detail="${kd# }${bad:+ NOT:$bad} (key table: ${names% }; unnamed keys seen: ${unnamed:-none})"
    echo "$detail" > $OUT/keys-result.txt
    if [ -z "$bad" ]; then pass keys "$detail"; else fail keys "$detail"; fi
    ;;
shell)
    sd=""; bad=""
    res() { if [ "$1" = ok ]; then sd="$sd $2 ok;"; else bad="$bad $2 ($3);"; fi; }
    home; sleep 3
    ctl shell > $OUT/shell-start.json
    if [ "$(rust_get ready)" = true ]; then res ok ready
    else res no ready "connected $(rust_get connected), engine $(rust_get engine), frameReady $(rust_get frameReady), error $(rust_get error)"; fi
    rust_home_dpad
    if [ "$DPAD_OK" = yes ]; then res ok "dpad($DPAD_F0 -> $DPAD_F1 -> $DPAD_F2)"; else res no dpad "focus '$DPAD_F0', '$DPAD_F1', '$DPAD_F2' after $DPAD_KEYS"; fi
    key_tap 139
    if wait_panel quick open 10; then key_tap 412; if wait_panel quick closed 10; then res ok quick-home; else res no quick-home "Back did not close quick"; fi
    else res no quick-home "quick $(rust_get ui.panels.quick)"; fi
    key_tap 241
    if wait_panel picker open 10; then key_tap 412; if wait_panel picker closed 10; then res ok picker-home; else res no picker-home "Back did not close picker"; fi
    else res no picker-home "picker $(rust_get ui.panels.picker)"; fi
    for k in osd toast saver; do
        rust_check_kind "$k"
        if [ "$RC" = ok ]; then res ok "$k"; else res no "$k" "$RT"; fi
    done
    ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.homeconnect"}' > /dev/null
    if wait_app com.webos.app.homeconnect 20; then
        sleep 3; key_tap 139
        if wait_panel quick open 10 && [ "$(rust_get wants)" = true ]; then
            key_tap 412
            if wait_panel quick closed 10 && [ "$(rust_get wants)" = false ]; then res ok quick-app; else res no quick-app "quick $(rust_get ui.panels.quick), wants $(rust_get wants)"; fi
        else res no quick-app "quick $(rust_get ui.panels.quick), wants $(rust_get wants)"; fi
    else res no quick-app "the Home Hub did not map: $(where_now)"; fi
    close_app com.webos.app.homeconnect; sleep 3; home; sleep 2
    ctl shell > $OUT/shell-end.json
    detail="${sd# }${bad:+ NOT:$bad} (pid $(rust_get pid), starts $(rust_get stats.starts), exits $(rust_get stats.exits); $(where_now))"
    echo "$detail" > $OUT/shell-result.txt
    if [ -z "$bad" ]; then pass shell "$detail"; else fail shell "$detail"; fi
    ;;
rust)
    asked=$(echo "${LUMATV_RUST_KINDS:-home osd toast alert quick picker drawer settings cameras saver menu confirm switcher cards popup capture layout entry keyboard ai banner displaylab guide}" | tr ',' ' ')
    home; sleep 3
    k0=$(rust_kinds_on)
    ctl shell > $OUT/rust-start.json
    sd=""; bad=""; ran=0; skipped=0
    for k in $asked; do
        rust_check_kind $k
        case $RC in
            ok) ran=$((ran+1)); sd="$sd $k ok ($RT);" ;;
            skip) skipped=$((skipped+1)); sd="$sd $k SKIPPED ($RT);" ;;
            *) ran=$((ran+1)); bad="$bad $k ($RT);" ;;
        esac
    done
    home; sleep 1
    detail="${sd# }${bad:+ NOT:$bad} (kinds found [$k0]; $ran run, $skipped skipped)"
    echo "$detail" > $OUT/rust-result.txt
    if [ -n "$bad" ]; then fail rust "$detail"
    elif [ "$ran" -eq 0 ]; then echo "GATE rust: SKIPPED no kind ran: $detail"
    else pass rust "$detail"; fi
    ;;
ime)
    # A text field brings up maliit's keyboard under LumaTV's compositor. LG's Search app (a Qt app, com.webos.app.voice) is launched in its
    # keyboard mode (launchMode vkb: the field focused, the keyboard asked for); if no keyboard comes up by itself, OK on the field asks, at
    # 10 s and again at 18 s. The check reads `ctl ime`: an input method bound (maliit-server, started by LG's own request if it was not
    # running: the detail says what it was), a context made for the field, the keyboard's window on the screen at the bottom, a key typed on
    # it (OK presses the key that has its focus) reaching the field, and Back, or else Exit, closing the keyboard.
    mal0=$(systemctl is-active maliit-server.service); mal1=$(pgrep -x MaliitServer | head -n 1); mal1=${mal1:+pid $mal1}; mal1=${mal1:-not running}
    home; sleep 3
    ctl note ime-open > /dev/null
    ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.voice","params":{"activateType":"mrcu","launchMode":"vkb"}}' > /dev/null
    if ! wait_app com.webos.app.voice 15; then
        journal_after ime-open > $OUT/ime-journal.txt; ctl windows > $OUT/ime-windows.json
        detail="LG's Search app (com.webos.app.voice) mapped no window within 15 s ($(where_now); before: maliit-server.service $mal0, MaliitServer $mal1)"
        echo "$detail" > $OUT/ime-result.txt
        fail ime "$detail"
    else
        up=no; tries=0; n=0
        while [ $n -lt 28 ]; do
            sleep 1; n=$((n+1))
            ctl ime > $OUT/ime-state.json
            if grep -q '"panel":{' $OUT/ime-state.json; then up=yes; break; fi
            if [ $n = 10 ] || [ $n = 18 ]; then key_tap 28; tries=$((tries+1)); fi
        done
        sleep 1
        b=$(cap $OUT/ime.png)
        mal2=$(systemctl is-active maliit-server.service); mal3=$(pgrep -x MaliitServer | head -n 1); mal3=${mal3:+pid $mal3}; mal3=${mal3:-not running}
        ctl ime > $OUT/ime-up.json
        methods=$(grep -o '"display":[0-9]' $OUT/ime-up.json | wc -l)
        ctx=no; grep -q '"context":true' $OUT/ime-up.json && ctx=yes
        ph=$(jnum h < $OUT/ime-up.json); py=$(jnum y < $OUT/ime-up.json); asked=$(jnum asked < $OUT/ime-up.json)
        # y plus h at the bottom edge is not enough: a full-screen window at y 0 is "at the bottom" too (1080 high, in four of six runs,
        # with the keyboard drawn at its top), so the keyboard must also be shorter than half the screen.
        bottom=no; if [ -n "$ph" ] && [ -n "$py" ] && [ $((py + ph)) -eq 1080 ] && [ "$ph" -lt 540 ]; then bottom=yes; fi
        typed=0; closed=no; by=none
        if [ $up = yes ]; then
            before=$(ime_typed $OUT/ime-up.json)
            # OK presses the key that has the keyboard's focus; if that key types nothing, the arrows move to the next one
            for steps in "28" "106 28" "108 28"; do
                for k in $steps; do key_tap $k; sleep 1; done
                sleep 2
                ctl ime > $OUT/ime-typed.json
                typed=$(( $(ime_typed $OUT/ime-typed.json) - before ))
                [ $typed -ge 1 ] && break
            done
            for k in 412 174; do
                key_tap $k; n=0
                while [ $n -lt 6 ]; do
                    sleep 1; n=$((n+1))
                    ctl ime > $OUT/ime-closed.json
                    if grep -q '"panel":null' $OUT/ime-closed.json; then closed=yes; break; fi
                done
                [ $closed = yes ] && { by=$([ $k = 412 ] && echo Back || echo Exit); break; }
            done
        fi
        journal_after ime-open > $OUT/ime-journal.txt; ctl windows > $OUT/ime-windows.json
        detail="before: maliit-server.service $mal0, MaliitServer $mal1; after the keyboard came up: unit $mal2, MaliitServer $mal3; input methods bound $methods; asked for one $asked time(s); OK pressed $tries time(s); keyboard up $up (panel y $py h $ph, capture ${b:-0} bytes); context $ctx; typed $typed; closed by $by"
        echo "$detail" > $OUT/ime-result.txt
        verdict=$(ime_rule "$methods" "$up" "$ctx" "$typed" "$closed" "$bottom")
        case "$verdict" in PASS*) pass ime "$detail | ${verdict#PASS }" ;; *) fail ime "${verdict#FAIL } | $detail" ;; esac
    fi
    close_app com.webos.app.voice; sleep 2
    home
    ;;
evidence)
    # While LumaTV's compositor is live: who subscribed to what (SAM's foreground subscription first), the clients
    # with their command lines and globals, and both logs as they stand. Not a check: no GATE line.
    ctl luna > $OUT/luna.json
    ctl status > $OUT/status.json
    run_log | grep -E 'subscription|bridge:' > $OUT/sm-subscriptions.txt
    run_log | grep -E 'watchdog:|settled|previous run' > $OUT/sm-watchdog.txt
    comp_log lumatv-compositor | grep -E 'client (dis)?connected' > $OUT/clients.txt
    # The wire as a client sees it (globals, seats, output, webos seats, starfish timer), to set against LG's run of 2026-10-28.
    [ -f $D/wlprobe.py ] && XDG_RUNTIME_DIR=/tmp/xdg timeout 20 python3 $D/wlprobe.py /tmp/xdg/wayland-0 2 > $OUT/wlprobe.txt 2>&1
    comp_log lumatv-compositor > $OUT/compositor.log; run_log > $OUT/sm.log
    echo "evidence: $(grep -c applicationManager $OUT/sm-subscriptions.txt) subscription(s) from SAM, $(wc -l < $OUT/clients.txt) client lines, $(grep -c 'loop:' $OUT/compositor.log) loop stalls, $(wc -l < $OUT/sm-watchdog.txt) watchdog lines"
    ;;
callbacks)
    # Report client callback pacing; optionally set it for the frame gate.
    [ -z "${LUMATV_CLIENTS_CALLBACKS:-}" ] || ctl callbacks clients "$LUMATV_CLIENTS_CALLBACKS" > /dev/null
    st=$(ctl status)
    echo "callbacks${LUMATV_CLIENTS_CALLBACKS:+ (clients set to $LUMATV_CLIENTS_CALLBACKS)}: $(echo "$st" | jnum callbacksAhead) of $(echo "$st" | jnum callbacks) sent ahead of their flip, $(echo "$st" | jnum callbacksAtCommit) with their commit; clients $(echo "$st" | jstr clientCallbacks)"
    ;;
refresh)
    # The refresh test mode: switch (60, 120), reset or read; then SECS of the screen as it is, counted three ways (the OSD plane's flips,
    # the display engine's vsync and the OSD's interrupt), the compositor's measure (`read` leaves it running: after the surfaces it then
    # covers them), what refresh the shell client draws for (`ctl shell`, the client's own report), Mali's busy share (the cumulative busy
    # and idle times, as vplane/tools/measure.py reads them) and the SoC's temperature. The divider is confirmed from its file: a kernel
    # that ignored the write reads 2 at 120.
    r_secs=${3:-5}
    DIVIDER=/sys/module/lg1k/parameters/fbdev_vsync_cnt_on_flip
    case "${2:-read}" in
    60|120|reset)
        r_set=$(ctl refresh $2)
        if ! echo "$r_set" | grep -q '"ok":true'; then echo "GATE refresh: FAIL $r_set"; exit 1; fi
        sleep 2 ;;
    read) ;;
    *) echo "GATE refresh: FAIL refresh [60|120|reset|read] [SECS]"; exit 1 ;;
    esac
    r_hz=$(ctl refresh | jnum hz)
    r_div=$(tr -d ' \n' < $DIVIDER 2>/dev/null)
    irq_count() { awk -v n="$1" '$0 ~ n { for (i = 2; i <= 5; i++) if ($i ~ /^[0-9]+$/) s += $i; print s + 0 }' $PROC/interrupts 2>/dev/null; }
    mali_count() { sed -n "s/.*$1:[ ]*\([0-9]*\).*/\1/p" /sys/kernel/debug/mali0/dvfs_utilization 2>/dev/null | head -n 1; }
    f0=$(tr -dc '0-9' < /proc/lg/fbdev/fb0/flip_cnt 2>/dev/null); v0=$(irq_count DE2VSYNC); o0=$(irq_count osd_irq)
    b0=$(mali_count busy_time); i0=$(mali_count idle_time)
    sleep "$r_secs"
    f1=$(tr -dc '0-9' < /proc/lg/fbdev/fb0/flip_cnt 2>/dev/null); v1=$(irq_count DE2VSYNC); o1=$(irq_count osd_irq)
    b1=$(mali_count busy_time); i1=$(mali_count idle_time)
    per_s() { awk -v a="${1:-0}" -v b="${2:-0}" -v s="$r_secs" 'BEGIN { printf "%.1f", (b - a) / s }'; }
    mali=$(awk -v b0="${b0:-0}" -v b1="${b1:-0}" -v i0="${i0:-0}" -v i1="${i1:-0}" 'BEGIN { t = (b1 - b0) + (i1 - i0); if (t > 0) printf "%.1f", 100 * (b1 - b0) / t; else print "?" }')
    temp=$(tr -d '\n' < /proc/lg/pm/temperature 2>/dev/null | head -c 40)
    m=$(ctl refresh)
    r_shell=$(shell_json)
    shell_hz=$(echo "$r_shell" | grep -o '"refreshHz":[0-9]*' | head -n 1 | cut -d: -f2)
    # (by key: the compositor's JSON has its keys in alphabetical order, not the shell's)
    slides=$(echo "$r_shell" | grep -o '"slides":\[[^]]*\]' | head -n 1 | grep -o '{[^}]*}' | awk '{ s = m = d = "?"
        if (match($0, /"steps":[0-9]+/)) s = substr($0, RSTART + 8, RLENGTH - 8)
        if (match($0, /"ms":[0-9]+/)) m = substr($0, RSTART + 5, RLENGTH - 5)
        if (match($0, /"duration":[0-9]+/)) d = substr($0, RSTART + 11, RLENGTH - 11)
        printf "%s/%s/%s ", s, m, d }')
    echo "$m" > $OUT/refresh-${r_hz:-x}-${2:-read}.json
    echo "GATE refresh: measured hz ${r_hz:-?} divider ${r_div:-unreadable} shell ${shell_hz:-?} over ${r_secs} s: OSD flips/s $(per_s "$f0" "$f1"), DE2VSYNC/s $(per_s "$v0" "$v1"), osd_irq/s $(per_s "$o0" "$o1"); compositor: $(echo "$m" | grep -o '"intervals":{[^}]*}') $(echo "$m" | grep -o '"frames":{[^}]*}') $(echo "$m" | grep -o '"pipeline_ms":{[^}]*}') flips/s $(echo "$m" | grep -o '"flips_per_second":[0-9.]*' | cut -d: -f2); mali busy ${mali} %; temperature ${temp:-unreadable}; slides (steps/ms/for): ${slides:-none}"
    ;;
scanout)
    # Direct scan-out counters describe eligible client surfaces.
    case "${2:-read}" in
    direct)
        r_set=$(ctl scanout direct)
        if ! echo "$r_set" | grep -q '"ok":true'; then echo "GATE scanout: FAIL $r_set"; exit 1; fi
        sleep 2 ;;
    off)
        r_set=$(ctl scanout off)
        if ! echo "$r_set" | grep -q '"ok":true'; then echo "GATE scanout: FAIL $r_set"; exit 1; fi
        sleep 1 ;;
    read) ;;
    *) echo "GATE scanout: FAIL scanout [direct|off|read]"; exit 1 ;;
    esac
    s=$(ctl scanout)
    echo "$s" > $OUT/scanout-${2:-read}.json
    echo "GATE scanout: mode $(echo "$s" | jstr mode) frames $(echo "$s" | grep -o '"frames":{[^}]*}' | sed 's/^"frames"://') composed because $(echo "$s" | grep -o '"refused":{[^}]*}' | sed 's/^"refused"://') transitions $(echo "$s" | jnum transitions) tap $(echo "$s" | grep -o '"tap":{[^}]*}' | sed 's/^"tap"://') shell env $(echo "$s" | grep -o '"shellEnv":\[[^]]*\]' | sed 's/^"shellEnv"://') last $(echo "$s" | grep -o '"last":{[^}]*}' | sed 's/^"last"://')"
    ;;
standby)
    ctl journal > $OUT/standby-journal-before.json
    cat /proc/uptime > $OUT/standby-uptime-before
    ls1 com.webos.service.tvpower/power/powerOff '{"reason":"remoteKey"}' > $OUT/standby-poweroff.txt
    echo "GATE standby: requested ($(head -c 120 $OUT/standby-poweroff.txt))"
    ;;
woke)
    sleep 8
    ctl journal > $OUT/woke-journal.json
    ctl status > $OUT/woke-status.json
    off=$(grep -c 'PreparePowerOff' $OUT/woke-journal.json); on=$(grep -c 'PreparePowerOn' $OUT/woke-journal.json)
    fb=$(run_log | grep -c 'fallback:')
    st=$(ls1 com.webos.service.tvpower/power/getPowerState '{}' | tr -d ' \n' | sed -n 's/.*"state":"\([^"]*\)".*/\1/p' | tr 'A-Z' 'a-z')
    home; sleep 8
    b=$(cap $OUT/woke.png)
    detail="rounds: PreparePowerOff $off PreparePowerOn $on; fallbacks $fb; state $st; uptime before $(cut -d' ' -f1 $OUT/standby-uptime-before) now $(cut -d' ' -f1 /proc/uptime); mounted $(is_mounted && echo yes || echo no); panel ${b:-0} bytes"
    echo "$detail" > $OUT/woke-result.txt
    if [ "$off" -ge 1 ] && [ "$on" -ge 1 ] && [ "$fb" = 0 ] && [ "$st" = active ] && is_mounted && home_panel_ok "$b"; then pass standby "$detail"; else fail standby "$detail"; fi
    ;;
firmware)
    # The firmware check on this set's real files, tools and hash speed, against a scratch state directory: nothing the
    # set holds changes. The entry made from these files must verify; a flipped digit, another release line and an empty
    # list must not. Then the set's own list and the supervisor's own answer are read and reported.
    S=$OUT/fw-state
    rm -rf $S; mkdir -p $S/bin
    if ! command -v comp_fw_check > /dev/null || [ ! -x $STATE/bin/lumatv-sm ]; then
        fail firmware "tools/compositor.sh is not deployed ($D/compositor.sh) or the bundle is not installed"
    else
        fwq() { ( COMP=$S; "$@" ); }
        cp $STATE/manifest $STATE/lg-binary $S/ 2>/dev/null
        ln -s $STATE/bin/lumatv-sm $S/bin/lumatv-sm
        h0=$(date +%s)
        blk=$(fwq comp_fw_block)
        h1=$(date +%s)
        printf '%s\n' "$blk" > $S/good.list
        nfiles=$(printf '%s\n' "$blk" | grep -c '^file ')
        covered=$(comp_fw_files | grep -c .)
        unread=$(printf '%s\n' "$blk" | grep '^file - ' | cut -d ' ' -f 4 | tr '\n' ' ')
        fp=$(printf '%s\n' "$blk" | sed -n 's/^fingerprint //p')
        cp $S/good.list $S/verified.list
        pos=$(fwq comp_fw_check force); posrc=$?
        awk '$1 == "file" && !seen { h = $2; c = substr(h, 1, 1); $2 = (c == "0" ? "1" : "0") substr(h, 2); seen = 1 } { print }' $S/good.list > $S/verified.list
        neg1=$(fwq comp_fw_check force); neg1rc=$?
        sed 's/^release .*/release Some Other Release 1.0/' $S/good.list > $S/verified.list
        neg2=$(fwq comp_fw_check force); neg2rc=$?
        printf '# nothing listed\n' > $S/verified.list
        neg3=$(fwq comp_fw_check force); neg3rc=$?
        # the set's own list: the verdict a persistent enable would get, read from a copy so that the caches the set keeps stay as they are
        O=$OUT/fw-own; rm -rf $O; mkdir -p $O
        cp $STATE/verified.list $STATE/manifest $STATE/lg-binary $O/ 2>/dev/null
        own=$(COMP=$O comp_fw_check force 2>&1); ownrc=$?
        smsays=$($STATE/bin/lumatv-sm --state $STATE --would-run 2>&1); smrc=$?
        ok=1
        [ "$posrc" = 0 ] || ok=0
        { [ "$neg1rc" = 5 ] && echo "$neg1" | grep -q 'is not the verified file'; } || ok=0
        { [ "$neg2rc" = 5 ] && echo "$neg2" | grep -q 'the release is'; } || ok=0
        { [ "$neg3rc" = 5 ] && echo "$neg3" | grep -q 'has no entry'; } || ok=0
        { [ "${nfiles:-0}" = "$covered" ] && [ -z "$unread" ]; } || ok=0
        echo "$fp" | grep -qE '^[0-9a-f]{16}$' || ok=0
        { [ "$smrc" = 0 ] || [ "$smrc" = 3 ]; } || ok=0
        detail="$(comp_release_line); fingerprint $fp; $nfiles files hashed in $((h1-h0)) s, unreadable: ${unread:-none}; this set's entry: $pos; a flipped digit: $neg1rc; another release: $neg2rc; an empty list: $neg3rc; the set's own list: $own; the supervisor: $smsays (exit $smrc)"
        echo "$detail" > $OUT/firmware-result.txt
        printf '%s\n' "$blk" > $OUT/firmware-entry.txt
        if [ $ok = 1 ]; then pass firmware "$detail"; else fail firmware "$detail"; fi
    fi
    ;;
homeheld)
    # A virtual remote holds Home for 13 s (uinput); the supervisor's hold reader, asked from a fresh
    # process in the middle of it, says yes, and after the release says no. Run while LG's compositor is live: the
    # supervisor's own reader would take the key as the escape.
    if [ ! -c /dev/uinput ] || [ ! -x $STATE/bin/lumatv-sm ] || [ ! -f $D/uinput-key.py ]; then
        fail homeheld "no /dev/uinput, or the bundle or $D/uinput-key.py is missing"
    else
        python3 $D/uinput-key.py hold 13 --settle 0.5 > $OUT/homeheld-inject.txt 2>&1 &
        ip=$!
        sleep 2
        $STATE/bin/lumatv-sm --state $STATE --home-held 10000; held=$?
        wait $ip; injrc=$?
        sleep 0.5
        $STATE/bin/lumatv-sm --state $STATE --home-held 10000; after=$?
        detail="while held: exit $held (0 = held); after the release: exit $after (1 = not held); injector exit $injrc"
        echo "$detail" > $OUT/homeheld-result.txt
        if [ "$held" = 0 ] && [ "$after" = 1 ] && [ "$injrc" = 0 ]; then pass homeheld "$detail"; else fail homeheld "$detail"; fi
    fi
    ;;
homekey)
    # The supervisor's own reader: the virtual remote is made after the compositor started, so the compositor (which
    # opens the event nodes once) never sees it and only the supervisor's rescan can. Home held 12 s on it must end the
    # session as an escape: recorded as one, not counted as a failure, SAM untouched.
    if ! is_mounted || [ ! -c /dev/uinput ] || [ ! -f $D/uinput-key.py ]; then
        fail homekey "not live, or no /dev/uinput, or $D/uinput-key.py is missing"
    else
        sam0=$(sam_id); f0=$(cat $STATE/failures 2>/dev/null || echo 0)
        python3 $D/uinput-key.py hold 12 --settle 9 > $OUT/homekey-inject.txt 2>&1 &
        ip=$!
        wait_unmount 60
        wait $ip; injrc=$?
        sleep 2
        recovery=complete; wait_recovery || recovery=timeout
        sleep 2
        fb=$(fallback_evidence)
        esc=$([ -f $STATE/escaped ] && echo yes || echo no)
        f1=$(cat $STATE/failures 2>/dev/null || echo 0)
        b=$(cap $OUT/homekey.png)
        sam1=$(sam_id)
        detail="unmounted after $((u/2))s; $fb; recovery $recovery; escaped $esc; failures $f0 -> $f1; panel ${b:-0} bytes; injector exit $injrc; sam $([ "$sam0" = "$sam1" ] && echo unchanged || echo "CHANGED $sam0 -> $sam1")"
        echo "$detail" > $OUT/homekey-result.txt
        if ! is_mounted && [ "$recovery" = complete ] && [ "$esc" = yes ] && echo "$fb" | grep -q 'hold Home 10 s (supervisor)' && [ "$f0" = "$f1" ] && [ "${b:-0}" -gt 4096 ] && [ "$sam0" = "$sam1" ]; then pass homekey "$detail"; else fail homekey "$detail"; fi
    fi
    ;;
wd-hang|wd-crash)
    # The watchdog on the real set: the compositor is stopped or killed while live. Either way the supervisor hands the
    # screen back to LG's by exec in place (no unit restart), counts one failure, brings Home back, and SAM is not touched.
    pid=$(pgrep -f 'bin/lumatv-compositor' | head -1)
    if [ -z "$pid" ] || ! is_mounted; then
        fail $1 "not live (compositor pid ${pid:-none}, mounted $(is_mounted && echo yes || echo no))"
    else
        sam0=$(sam_id); f0=$(cat $STATE/failures 2>/dev/null || echo 0)
        if [ "$1" = wd-hang ]; then kill -STOP $pid; want='heartbeat: two beats missed'; limit=30; else kill -9 $pid; want='killed by signal 9'; limit=20; fi
        wait_unmount 60
        took=$u
        sleep 2
        recovery=complete; wait_recovery || recovery=timeout
        sleep 2
        reason=$(cat $STATE/last-fallback 2>/dev/null)
        f1=$(cat $STATE/failures 2>/dev/null || echo 0)
        gone=$(pgrep -f 'bin/lumatv-compositor' | head -1)
        b=$(cap $OUT/$1.png)
        sam1=$(sam_id)
        detail="unmounted after $((took/2)).$((took%2*5)) s; reason: $reason; recovery $recovery; failures $f0 -> $f1; escaped $([ -f $STATE/escaped ] && echo yes || echo no); compositor left: ${gone:-none}; panel ${b:-0} bytes; wam $(systemctl is-active webapp-mgr); sam $([ "$sam0" = "$sam1" ] && echo unchanged || echo "CHANGED $sam0 -> $sam1"); crash bundle $(crash_summary)"
        echo "$detail" > $OUT/$1-result.txt
        if ! is_mounted && [ "$recovery" = complete ] && [ "$took" -le "$limit" ] && echo "$reason" | grep -q "$want" && [ "$f1" = "$((f0+1))" ] && [ ! -f $STATE/escaped ] && [ -z "$gone" ] && [ "${b:-0}" -gt 4096 ] && [ "$sam0" = "$sam1" ]; then pass $1 "$detail"; else fail $1 "$detail"; fi
    fi
    ;;
wake)
    # The loop's wake-up latency: the frame timer, the page flip and the input events as the loop saw them (p50, p99, max in ms), and
    # what the scheduler gives a thread of each class under the load of the moment, and a FIFO 20 thread kept on CPU 0 (where the USB host's
    # interrupt is served) against one kept on CPU 1 (`fifo20@0`, `fifo20@1`). The distributions are the compositor's (`ctl wake`);
    # a report with no rule until a baseline exists (docs/research/lpc-tv-result.md).
    case "${2:-}" in
    reset) ctl wake reset > /dev/null; echo "GATE wake: reset" ;;
    probe)
        secs=${3:-10}
        ctl wake reset > /dev/null
        ctl wake probe start fifo20,fifo19,other,fifo20@0,fifo20@1 1000 > /dev/null
        sleep "$secs"
        line=$(ctl wake | jstr line)
        ctl wake probe stop > /dev/null
        echo "GATE wake: measured over ${secs} s with probes: ${line:-not read}" ;;
    *) line=$(ctl wake | jstr line); echo "GATE wake${2:+ ($2)}: measured ${line:-not read}" ;;
    esac
    ;;
video)
    # A video app full screen, the page format and what it costs: `gate.sh video [youtube|netflix|plex|current] [SECS]` (tv_gate.py --video-apps).
    # The video is on the display engine's plane under LumaTV's page; its hole is alpha 0 in an AR24 page. The rules:
    #  1 the pages are AR24 (`ctl status` pageFormat, videoHolesOpaque false) and fb0 reads hdr.global_alpha_en 0: with XR24 the display driver forces a
    #    constant alpha and every hole is opaque black, a video app with sound and no picture (round 19, 2026-10-29);
    #  2 the DISPLAY capture's middle is not black (mean luma and spread: tv-pngstat.py) in at least two of three captures 2 s apart;
    #  3 nothing over the video: the compositor's CPU a second stays under LUMATV_VIDEO_MAX_CPU percent of a core (default 10, a starting bar for the
    #    first round: the numbers are in the line, tighten it from them), and the frames in which the shell's UI engine drew something (`stats.uiDrawn`) are
    #    under LUMATV_VIDEO_MAX_UI_FPS a second (default 0.1): a Home that went on drawing behind a video was 2.5 frames a second of 12 ms of the GPU each
    #    (round 19, found by the ring's flips). The frames composed are in the line too: the video app's own commits make them (YouTube's player UI: 2 to
    #    50 a second, each the whole page), and no rule is made of them; the OSD's flips are in the line;
    #  4 each overlay over the video (the volume display, the quick panel, the input picker, the notifications drawer, a toast, an alert; LUMATV_VIDEO_OVERLAYS
    #    names others or fewer): it shows (first frame under the panels' 150 ms bar, a reading good to the gate's own polling: the ring says when the
    #    first flip landed), the video is still seen around it (the upper left of the capture), no frame of its motion dropped (`ctl refresh` `motion`: a
    #    flip over 20 ms after the frame of a running animation; the video app's own gaps and the pause between a motion's phases are none), no GL error, and
    #    3 s after its close the shell has drawn at most one more frame. Each overlay's refresh, ring and wake reports are kept (video-APP-KIND-*.json;
    #    tv-ringflips.py reads the ring).
    # The apps: youtube (a video by its id, LUMATV_VIDEO_YT), netflix (the Dolby Vision test film as the netflix check does), plex (PlxNative: it is
    # launched and the owner starts a film; the check waits LUMATV_VIDEO_WAIT s for its video), current (whatever plays now, nothing launched or closed).
    v_app=${2:-current}; v_secs=${3:-120}; v_wait=${LUMATV_VIDEO_WAIT:-45}; v_cpu_max=${LUMATV_VIDEO_MAX_CPU:-10}; v_ui_max=${LUMATV_VIDEO_MAX_UI_FPS:-0.1}
    v_overlays=${LUMATV_VIDEO_OVERLAYS:-osd,quick,picker,drawer,toast,alert}
    case $v_app in
        youtube) v_id=youtube.leanback.v4; v_launch="{\"id\":\"$v_id\",\"params\":{\"contentTarget\":\"https://www.youtube.com/tv?v=${LUMATV_VIDEO_YT:-aqz-KE-bpKQ}\"}}" ;;
        netflix) v_id=netflix; v_launch='{"id":"netflix","params":{"contentTarget":"m=https://www.netflix.com/watch/80141336&source_type=4"}}' ;;
        plex) v_id=com.beb.plxnative; v_launch="{\"id\":\"$v_id\"}" ;;
        current) v_id=""; v_launch="" ;;
        *) echo "GATE video: FAIL video [youtube|netflix|plex|current] [SECS]"; exit 1 ;;
    esac
    name=video-$v_app; bad=""; info=""
    if [ -n "$v_id" ]; then
        if app_running "$v_id"; then close_app "$v_id"; wait_app_gone "$v_id" 15; sleep 5; fi
        ctl note video-$v_app > /dev/null
        ls1 com.webos.applicationManager/launch "$v_launch" > /dev/null
        [ $v_app != netflix ] || { sleep 12; key_tap 28; }
    fi
    n=0; until video_up || [ $n -ge $v_wait ]; do sleep 1; n=$((n+1)); done
    if ! video_up; then
        [ -z "$v_id" ] || { close_app "$v_id"; home; }
        fail $name "no video element came up within $v_wait s (foreground $(fg)); a film that plays is the first thing this check needs"
        exit 0
    fi
    sleep 5
    # 1 the page format and the plane's state
    v_st=$(ctl status); v_fmt=$(echo "$v_st" | jstr pageFormat); v_opq=$(echo "$v_st" | jbool videoHolesOpaque); v_ga=$(fb_field 0 hdr.global_alpha_en)
    info="pages ${v_fmt:-?} holes-opaque ${v_opq:-?} fb0 global_alpha_en ${v_ga:-unreadable}"
    [ "$v_fmt" = AR24 ] && [ "$v_opq" = false ] || bad="$bad the pages are not AR24 with transparent holes;"
    [ "$v_ga" = 0 ] || bad="$bad fb0 hdr.global_alpha_en is '${v_ga:-unreadable}', not 0;"
    # 2 the picture is there: three captures of the middle half
    v_seen=0; v_lines=""
    for k in 1 2 3; do
        l=$(video_look $OUT/$name-$k.png)
        v_lines="$v_lines [$k: $(ps_val "$l" mean) mean $(ps_val "$l" std) spread]"
        [ "$(ps_val "$l" black)" = no ] && v_seen=$((v_seen + 1))
        [ $k = 3 ] || sleep 2
    done
    info="$info; middle of the DISPLAY capture non-black in $v_seen of 3:$v_lines"
    [ "$v_seen" -ge 2 ] || bad="$bad the DISPLAY capture's middle is black in $((3 - v_seen)) of 3 (the video does not show);"
    # 3 nothing over the video
    ctl refresh reset > /dev/null
    video_sample "$v_secs"
    info="$info; over $v_secs s with nothing over the video: compositor CPU $VS_CPU % of a core, the shell drew $VS_UI frames/s, composed $VS_DRAWN frames/s (the video app's own commits included), OSD flips $VS_FLIPS/s, Mali busy $VS_MALI %"
    awk -v c="$VS_CPU" -v m="$v_cpu_max" 'BEGIN { exit !(c > m) }' && bad="$bad the compositor used $VS_CPU % of a core under a video with nothing over it (bar $v_cpu_max %);"
    awk -v f="$VS_UI" -v m="$v_ui_max" 'BEGIN { exit !(f > m) }' && bad="$bad the shell drew $VS_UI frames/s under a video with nothing over it (bar $v_ui_max): something of it draws behind the video;"
    ctl ring 1500 > $OUT/$name-idle-ring.json
    # 4 each overlay over the video
    v_pm=$(rust_poll_ms); v_ov=""
    for k in $(echo "$v_overlays" | tr ',' ' '); do
        if ! rust_answers "$k"; then bad="$bad $k: native UI not ready or kind absent;"; continue; fi
        ctl note video-overlay-$k > /dev/null
        ctl refresh reset > /dev/null
        g0=$(rust_gl_errors); c0=$(cpu_ticks "${vs_pid:-self}"); d0=$(rust_drawn); f0=$(fb_flips)
        v_path=$(rust_path $k); first=""
        # (a note in the ring right before the trigger: `tv-ringflips.py RING video-trigger-KIND` dates the first flip from it, to the millisecond, where `first`
        # below is a poll of the shell's own report)
        ctl note video-trigger-$k > /dev/null
        t0=$(uptime_cs)
        rust_trigger $k
        while [ $(( $(uptime_cs) - t0 )) -lt $((RUST_OPEN * 100)) ]; do
            if rust_is_open $k "$(rust_get $v_path)"; then first=$((($(uptime_cs) - t0) * 10)); break; fi
            sleep 0.05
        done
        sleep 2
        c1=$(cpu_ticks "${vs_pid:-self}"); d1=$(rust_drawn); f1=$(fb_flips)
        # The flips are read before the capture, whose own second is not the overlay's; the ring (every flip and frame with its stamp, owed pixels and
        # cost) and the wake-up latencies go to the evidence with it.
        r=$(ctl refresh); hz=$(echo "$r" | jnum hz)
        echo "$r" > $OUT/$name-$k-refresh.json; ctl ring 1500 > $OUT/$name-$k-ring.json; ctl wake > $OUT/$name-$k-wake.json
        l=$(video_look $OUT/$name-$k.png 0.05 0.05 0.4 0.4)
        # A drop is a flip over 20 ms after the frame of an animation that was still running (`motion` of the measure): the video app's own commits
        # come about twice a second with nothing over it, and each gap between them is an interval over 20 ms that no overlay made. The raw count is kept in the line.
        stalls=$(echo "$r" | jnum dropped); stall_worst=$(echo "$r" | grep -o '"dropped_worst_ms":[0-9.]*' | head -n 1 | cut -d: -f2)
        two=$(echo "$r" | grep -o '"two":[0-9]*' | head -n 1 | cut -d: -f2); three=$(echo "$r" | grep -o '"three":[0-9]*' | head -n 1 | cut -d: -f2); more=$(echo "$r" | grep -o '"more":[0-9]*' | head -n 1 | cut -d: -f2)
        # an interval of two refreshes is 33 ms at 60 Hz (over 20) and 16.7 ms at 120 (under)
        if [ "${hz:-60}" -ge 100 ] 2> /dev/null; then over=$(( ${three:-0} + ${more:-0} )); else over=$(( ${two:-0} + ${three:-0} + ${more:-0} )); fi
        late=$(echo "$r" | jnum late); lost=$(echo "$r" | jnum lost); missed=$(echo "$r" | jnum missed)
        g1=$(rust_gl_errors)
        rust_close $k
        w=0; while [ $w -lt $((RUST_OPEN * 2)) ] && rust_is_open $k "$(rust_get $v_path)"; do sleep 0.5; w=$((w+1)); done
        sleep 1
        d2=$(rust_drawn); u2=$(rust_ui_drawn); sleep 3; d3=$(rust_drawn); u3=$(rust_ui_drawn)
        cpu=$(awk -v a="${c0:-0}" -v b="${c1:-0}" -v hz="$vs_hz" 'BEGIN { printf "%.0f", (b - a) * 100 / hz / 2 }')
        after=$(per_s "$d2" "$d3" 3); after_ui=$((${u3:-0} - ${u2:-0}))
        what="$k: first frame ${first:-none} ms, composed +$(( ${d1:-0} - ${d0:-0} )) frames and $(( $(fb_flips) - ${f0:-0} )) flips in the open, CPU about $cpu %, animation frames dropped ${stalls:-?} (worst ${stall_worst:-0} ms; $over intervals over 20 ms counting the video's own gaps), frames late ${late:-?} lost ${lost:-?} missed ${missed:-?}, around it $(ps_val "$l" mean) mean $(ps_val "$l" std) spread, composed $after/s and the shell drew $after_ui frames in the 3 s after the close;"
        v_ov="$v_ov $what"
        [ -n "$first" ] || bad="$bad $k did not show;"
        case $k in quick|picker|drawer) [ -z "$first" ] || [ "$first" -le $((150 + 2 * v_pm)) ] || bad="$bad $k's first frame ${first} ms is over the 150 ms bar;" ;; esac
        [ "$(ps_val "$l" black)" = no ] || bad="$bad the video is black around $k;"
        [ "${stalls:-0}" -eq 0 ] || bad="$bad $k's open dropped $stalls frames of its motion (over 20 ms after a frame of a running animation, worst ${stall_worst:-0} ms);"
        [ "$(( ${g1:-0} - ${g0:-0} ))" -eq 0 ] || bad="$bad GL errors while $k was up;"
        [ "$after_ui" -le 1 ] || bad="$bad the shell drew $after_ui frames in the 3 s after $k closed;"
    done
    info="$info; overlays:$v_ov"
    echo "$info" > $OUT/$name-result.txt
    ctl status > $OUT/$name-status.json
    if [ -n "$v_id" ]; then close_app "$v_id"; wait_app_gone "$v_id" 15; home; sleep 3; fi
    if [ -z "$bad" ]; then pass $name "$info"; else fail $name "${bad# } | $info"; fi
    ;;
mem)
    # What the compositor pinned and what the set has left, read only: `ctl mem` (resident, locked, swapped, malloc's mmapped blocks and what the
    # lock did at start), the compositor's own limit on locked memory, the set's free memory, CMA and swap.
    m_pid=$(pgrep -f 'bin/lumatv-compositor' | head -1)
    j=$(ctl mem)
    blocks=$(echo "$j" | grep -o '"mmapped_blocks":[0-9]*' | cut -d: -f2)
    avail=$(awk '/^MemAvailable:/ { printf "%d", $2 / 1024 }' $PROC/meminfo 2>/dev/null)
    cma=$(awk '/^CmaFree:/ { printf "%d", $2 / 1024 }' $PROC/meminfo 2>/dev/null)
    swap=$(awk 'NR > 1 { used += $4 } END { printf "%d", used / 1024 }' $PROC/swaps 2>/dev/null)
    limit=$(grep 'Max locked memory' $PROC/${m_pid:-self}/limits 2>/dev/null | tr -s ' ' | cut -d ' ' -f4-)
    echo "GATE mem: measured rss $(echo "$j" | jnum rss_kib) KiB, locked $(echo "$j" | jnum locked_kib) KiB, swapped $(echo "$j" | jnum swap_kib) KiB, malloc mmapped blocks ${blocks:-?}; lock: $(echo "$j" | jstr lock); the process's limit on locked memory: ${limit:-unreadable}; set: MemAvailable ${avail:-?} MiB, CmaFree ${cma:-?} MiB, swap in use ${swap:-0} MiB"
    ;;
load)
    # Synthetic background load for the frame gate's loaded run (bin/lumatv-loadgen): a camera-like decode, Home Assistant websocket chatter,
    # a BLE scan stand-in and a low-priority download to /tmp. `start SECS [decode,ws,ble,download]` runs it in the background, `stop` ends it
    # and prints what it did, `status` says whether it runs.
    case "${2:-status}" in
    start)
        [ -f $OUT/loadgen.pid ] && kill -0 "$(cat $OUT/loadgen.pid)" 2>/dev/null && { echo "GATE load: already running"; exit 0; }
        $LD --library-path $LIB $D/bin/lumatv-loadgen run "${3:-900}" ${4:+--only $4} --download-mib 48 > $OUT/loadgen.log 2>&1 &
        echo $! > $OUT/loadgen.pid
        sleep 1
        if kill -0 "$(cat $OUT/loadgen.pid)" 2>/dev/null; then echo "GATE load: started for ${3:-900} s (${4:-decode,ws,ble,download})"; else echo "GATE load: did not start: $(cat $OUT/loadgen.log)"; fi ;;
    stop)
        p=$(cat $OUT/loadgen.pid 2>/dev/null)
        [ -n "$p" ] && kill -TERM "$p" 2>/dev/null
        n=0; while [ -n "$p" ] && kill -0 "$p" 2>/dev/null && [ $n -lt 20 ]; do sleep 0.5; n=$((n+1)); done
        rm -f $OUT/loadgen.pid
        echo "GATE load: measured under it, $(tail -n 1 $OUT/loadgen.log 2>/dev/null)" ;;
    *) p=$(cat $OUT/loadgen.pid 2>/dev/null); if [ -n "$p" ] && kill -0 "$p" 2>/dev/null; then echo "GATE load: running (pid $p)"; else echo "GATE load: not running"; fi ;;
    esac
    ;;
key)
    # One remote key through the compositor's own routing (tv_gate.py's volume check).
    ctl remote "$2" tap
    ;;
*) sed -n '2,/^[^#]/{/^#/p;}' $0 ;;
esac
