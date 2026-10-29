#!/bin/sh
# The C4 gate, executed ON THE SET as /tmp/lumatv/gate.sh while LumaTV's compositor is live (cutover.sh enable --session).
# Each check prints one "GATE <name>: PASS|FAIL <detail>" line; files under /tmp/lumatv/out/gate/. Driven from
# the workstation by tools/tv_gate.py, which mutes the receiver, wakes the set and orders the checks.
#
#   gate.sh live [LABEL]  LumaTV is the compositor: mounted, 12 names registered, no fallback, WAM up, Home (the web
#                     stub) mapped and on the panel, the watchdog reading the compositor's counters and the power state
#   gate.sh x1 SECS   the X1 probe client (64-bit, LG's blob) for SECS: the frame rule below. LUMATV_GATE_KWAIT traces the run, which is what lets the
#                     rule tell the compositor's gaps from the client's (tv_gate.py sets it unless --x1-strict); LUMATV_X1_STRICT is the PLAN's rule
#                     (every gap, untraced). A/Bs, the rule unchanged: LUMATV_X1_RT=N the client runs SCHED_FIFO N; LUMATV_X1_CALLBACKS=early|landing
#                     the clients' frame callbacks go out with the frame's draw or at the flip (tv_gate.py --x1-rt, --x1-callbacks)
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
#   gate.sh shell     the shell client (shell.rs, compositor/shell): connected, LumaTV's Home drawn in the stub's place and taking the
#                     D-pad, the quick panel and the input picker on the gear and Input (on Home and over an app, where the system
#                     layer takes the keys and lets go), the volume display on a volume key, a toast, the screensaver when tvpower
#                     starts it, and the idle cost (commits a second and CPU on Home at rest, none with an app covering Home);
#                     Home in front and no app left at the end
#   gate.sh rust      the Rust shell's kinds one at a time (LUMATV_RUST_KINDS names them; default: all): on, shown, first frame, frames drawn and GL errors, closed,
#                     off, put back as found; a kind the compositor or the client does not have is SKIPPED (tv_gate.py --rust-kinds)
#   gate.sh evidence  no check: SAM's subscriptions, the clients (command lines, globals) and both logs, while live
#   gate.sh key CODE  one remote key through the compositor's routing (the volume check in tv_gate.py)
#   gate.sh standby   power off (remoteKey); the driver wakes the set; `gate.sh woke` then checks the rounds
#   gate.sh woke      after the wake: PreparePowerOff and PreparePowerOn acked, no fallback, Home on the panel
#   gate.sh escape    Home (773) held 1 s through the compositor's own routing: the supervisor falls back to LG's,
#                     records the escape, LG's Home comes back, SAM was not restarted
#   gate.sh firmware  not live: the firmware check on this set's own files and tools, against a scratch list
#   gate.sh homeheld  not live: a virtual remote holds Home (773); `lumatv-sm --home-held` sees it, then stops seeing it
#   gate.sh homekey   live: the supervisor's own Home reader (773), on a virtual remote the compositor never opened
#   gate.sh wd-hang   live: the compositor is stopped (SIGSTOP); the heartbeat falls back to LG's, SAM untouched
#   gate.sh wd-crash  live: the compositor is killed (SIGKILL); the supervisor falls back to LG's, SAM untouched; the crash bundle it left
#   gate.sh wake [reset|probe SECS]  the loop's wake-up latency (p50/p99/max of the tick, the flip, the input) since the last reset, or a
#                     SECS-long cyclictest-style probe at the loop's class, the Mali submission threads' class and a time-shared one. A report, no rule
#   gate.sh mem       what the compositor has resident and pinned, malloc's mmapped blocks, and what the set has left (report)
#   gate.sh load start SECS [KINDS] | stop | status   synthetic background load (decode, ws, ble, download) for the frame gate's loaded run
#   gate.sh scanout [direct|off|read]   direct scan-out (docs/research/scanout-research.md): `direct` asks the shell for buffers the plane takes (`ctl scanout
#                     shell-env WAYLAND_AFBC=0`), switches the compositor's scan-out on and starts the shell again, so that its Home window makes buffers the
#                     protocol tap sees; `off` switches it off; `read` (the default) only reports: frames shown direct and composed, the reason of each
#                     composed one, the last frame's state. A report, no rule
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
RECOVER_LOCK=${LUMATV_RECOVER_LOCK:-/tmp/lumatv-recover.lock}
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
# The shell client (`ctl shell`): whether it says it can show a panel of KIND now, and its own state's fields.
shell_can() { ctl shell | grep -o '"can":\[[^]]*\]' | grep -q "\"$1\""; }
shell_str() { ctl shell | jstr "$1"; }
shell_bool() { ctl shell | jbool "$1"; }
shell_num() { ctl shell | jnum "$1"; }
# Waits, SECS at most, for the shell's string field NAME to be VALUE.
wait_shell_str() { w=0; while [ "$w" -lt "$3" ]; do [ "$(shell_str "$1")" = "$2" ] && return 0; sleep 0.5; w=$((w+1)); done; [ "$(shell_str "$1")" = "$2" ]; }
wait_shell_bool() { w=0; while [ "$w" -lt "$3" ]; do [ "$(shell_bool "$1")" = "$2" ] && return 0; sleep 0.5; w=$((w+1)); done; [ "$(shell_bool "$1")" = "$2" ]; }
wait_shell_num_ge() { w=0; while [ "$w" -lt "$3" ]; do [ "$(shell_num "$1")" -ge "$2" ] 2>/dev/null && return 0; sleep 0.5; w=$((w+1)); done; return 1; }
# The shell's idle cost over SECS: "commits-per-second cpu-percent" of its Home window and of qml-runner (every thread, one core = 100).
shell_idle_cost() {
    sp=$(shell_num pid); c0=$(shell_num homeCommits); t0=$(awk '{print $14+$15}' /proc/$sp/stat 2>/dev/null)
    sleep "$1"
    c1=$(shell_num homeCommits); t1=$(awk '{print $14+$15}' /proc/$sp/stat 2>/dev/null)
    hz=$(getconf CLK_TCK 2>/dev/null || echo 100)
    echo "$(( (${c1:-0} - ${c0:-0}) / $1 )) $(( (${t1:-0} - ${t0:-0}) * 100 / hz / $1 ))"
}

# The Rust shell's kinds (`gate.sh rust`). Its report is `ctl shell`'s `rust` object: `kinds` (what the compositor was asked for, kept in
# shell-rust.conf), `connected`, and `ui`, the client's own state, whose `kinds` are the ones the client built and answers. rustjson.py reads
# a dotted path out of JSON on stdin.
RUSTJSON=$D/rustjson.py
# Seconds a restart of the client is waited for, and a kind's trigger (the host tests shorten both).
RUST_WAIT=${LUMATV_RUST_WAIT:-30}
RUST_OPEN=${LUMATV_RUST_OPEN_WAIT:-10}
rust_get() { ctl shell | python3 $RUSTJSON "$1"; }
rust_kinds_on() { rust_get rust.kinds; }
# A kinds list in one order (the compositor keeps its own sorted; a list is compared as a set).
rust_sorted() { echo "$1" | tr ',' '\n' | sort | tr '\n' ','; }
rust_answers() { [ "$(rust_get rust.connected)" = true ] && case ",$(rust_get rust.ui.kinds)," in *",$1,"*) return 0 ;; esac; return 1; }
# Waits, SECS at most, for the client to answer KIND (WANT yes) or not to (WANT no: the client is gone or its kinds lack it).
rust_wait_answers() {
    end=$(( $(uptime_cs) + $3 * 100 ))
    while :; do
        if rust_answers "$1"; then have=yes; else have=no; fi
        [ "$have" = "$2" ] && return 0
        [ "$(uptime_cs)" -ge "$end" ] && return 1
        sleep 0.3
    done
}
# Where each kind says it is shown (a path of `ctl shell`) and what that reading must be. A panel's phase is opening, open, closing, parked or gone.
rust_path() {
    case $1 in
        osd) echo rust.ui.osd ;; toast) echo rust.ui.toasts ;; alert) echo rust.ui.alerts ;; saver) echo rust.ui.saver ;; home) echo rust.ui.home.home ;;
        quick|picker|drawer) echo rust.ui.panels.$1 ;; settings) echo rust.ui.settings.open ;; cameras) echo rust.ui.cameras.screen.mode ;;
    esac
}
rust_is_open() {
    case $1 in
        osd|saver|home|settings) [ "$2" = true ] ;;
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
        quick|picker|drawer|settings|cameras) ctl shell show "$1" > /dev/null ;;
    esac
}
# The way back: the volume key undoing the one pressed, OK on the alert, a key ending the saver, Back (one step each) for a panel or a page.
rust_close() {
    case $1 in
        osd) key_tap 114 ;;
        alert) key_tap 28 ;;
        saver) key_tap 106 ;;
        quick|picker|drawer|settings|cameras) n=0; while [ $n -lt 4 ]; do key_tap 412; sleep 0.7; rust_is_open "$1" "$(rust_get "$(rust_path "$1")")" || break; n=$((n+1)); done ;;
    esac
}
# Why a kind that is built is not driven here (empty: it is).
rust_undriven() {
    case $1 in
        menu) echo "a tile's menu opens on a hold of OK on Home, which the gate does not press" ;;
        confirm) echo "a panel asks it for an action, which the gate does not take" ;;
        switcher) echo "the app switcher needs open apps; the frames gate's switcher surface covers it" ;;
        cards|popup) echo "the owner's switch (overlays.enabled, cameras) is off and the gate does not set it" ;;
        capture) echo "needs capture.enabled and the encoder, which are the owner's (docs/research/capture-result.md)" ;;
        layout) echo "Back asks whether to save the layout; the gate does not answer it" ;;
    esac
}
# The documented bar for a kind's first frame, in ms (rust-panels-result.md: a panel opens in under 150 ms), or nothing: the reading is then INFO.
rust_budget() { case $1 in quick|picker|drawer) echo 150 ;; esac; }
# The compositor's counter of frames drawn and of GL errors, from `ctl status`.
rust_drawn() { ctl status | python3 $RUSTJSON stats.drawn; }
rust_gl_errors() { ctl status | sed -n 's/.*gl errors \([0-9][0-9]*\).*/\1/p' | head -n 1; }
# What one `ctl shell` takes (ms, to three reads): a first-frame reading is a poll of it, so it is good to about twice this.
rust_poll_ms() { t=$(uptime_cs); for i in 1 2 3; do rust_get rust.connected > /dev/null; done; echo $(( ($(uptime_cs) - t) * 10 / 3 )); }
# Leaves KIND as it was found (WAS yes or no): back on when it was, off when it was not, and the client's answer waited for.
rust_restore() {
    if [ "$2" = yes ]; then ctl shell rust +$1 > /dev/null; rust_wait_answers $1 yes $RUST_WAIT; else ctl shell rust -$1 > /dev/null; rust_wait_answers $1 no $RUST_WAIT; fi
}
# One kind's checks, in this shell: sets RC (ok, no or skip) and RT (the words for the gate's line). Not built (the compositor refuses the name, or the
# client's kinds lack it after the restart a change of kinds makes) is skip; so is a kind that is built and that the gate has no way to open (why: rust_undriven).
rust_check_kind() {
    k=$1; RC=ok; RT=""
    was=no; case ",$(rust_kinds_on)," in *",$k,"*) was=yes ;; esac
    ans=$(ctl shell rust +$k)
    case "$ans" in
        *'"ok":true'*) ;;
        *) RC=skip; RT="not built: the compositor refuses it ($(echo "$ans" | sed -n 's/.*"error":"\([^"]*\)".*/\1/p'))"; return ;;
    esac
    if ! rust_wait_answers $k yes $RUST_WAIT; then
        RC=skip; RT="not built: $k is not in rust.ui.kinds [$(rust_get rust.ui.kinds)], connected $(rust_get rust.connected), error '$(rust_get rust.error)'"
        rust_restore $k $was
        return
    fi
    why=$(rust_undriven $k)
    if [ -n "$why" ]; then RC=skip; RT="not driven: $why"; rust_restore $k $was; return; fi
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
            # The D-pad as the QML check reads it: Up from where Home's focus is moves it to another layer and Down brings it back. The frames it draws are Home's.
            f0=$(rust_get rust.ui.home.focusLayer); key_tap 103; sleep 1
            f1=$(rust_get rust.ui.home.focusLayer); key_tap 108; sleep 1
            f2=$(rust_get rust.ui.home.focusLayer)
            if [ "$f1" != "$f0" ] && [ "$f2" = "$f0" ]; then dpad="; dpad ok ($f0 -> $f1 -> $f2)"; else RC=no; dpad="; dpad: focus '$f0' then '$f1' after Up, '$f2' after Down"; fi
        fi
        ctl shell > $OUT/rust-$k.json
        d1=$(rust_drawn); g1=$(rust_gl_errors)
        drawn=$(( ${d1:-0} - ${d0:-0} )); gl=$(( ${g1:-0} - ${g0:-0} ))
        RT="${first:+first $first ms (a poll of about $pm ms), }drawn +$drawn, gl errors +$gl$dpad"
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
        RC=no; RT="not shown within $RUST_OPEN s of the trigger ($path reads '$(rust_get $path)'; can $(rust_get rust.can))"
        [ $k = home ] || rust_close $k
    fi
    journal_after rust-$k > $OUT/rust-$k.txt
    # The off switch: the kind goes from the list at once and the client stops answering it; then it is left as it was found.
    ctl shell rust -$k > /dev/null
    case ",$(rust_kinds_on)," in *",$k,"*) RC=no; RT="$RT; still on the compositor's list after -$k" ;; esac
    if rust_wait_answers $k no $RUST_WAIT; then RT="$RT; off ok"; else RC=no; RT="$RT; the client still answers $k $RUST_WAIT s after -$k"; fi
    rust_restore $k $was
}
# What the input method has sent to the fields so far (commits, preedits and keysyms), from the `ctl ime` JSON in file $1.
ime_typed() { c=$(jnum commits < "$1"); p=$(jnum preedits < "$1"); k=$(jnum keysyms < "$1"); echo $(( ${c:-0} + ${p:-0} + ${k:-0} )); }
# With the Home stub mounted (tools/lumatv homestub) Home is a transparent page by design and LumaHome lives in
# LG's compositor's QML, so a Home on the panel under LumaTV's compositor is black: the panel test then passes on
# the mapped window alone and the detail line carries home-stub 1.
stub_mounted() { grep -c ' /usr/palm/applications/com.webos.app.home ' $MOUNTINFO; }
home_panel_ok() { [ "$(stub_mounted)" -ge 1 ] || [ "${1:-0}" -gt 4096 ]; }
# The kernel's audio decoder port an input left open (Open=1, Connect=HDMI_PORTn) with no input app running: Netflix's
# sound then fails with tvq-pb-101 whatever the compositor (lumatv close_input_apps, round 9 result note, Round 10).
held_port() {
    for f in /proc/lgtv-driver/audio/adec.p*; do
        grep -qs '^Open=1' "$f" && sed -n 's/^Connect=\(HDMI_PORT[0-9]*\|ATV\|DTV\).*/\1/p' "$f"
    done | head -n1
}
# sm.log is appended by every supervisor start (trials, earlier gates): the checks read the current run only.
run_log() { awk '/\] start: /{n=0} {l[n++]=$0} END{for(i=0;i<n;i++) print l[i]}' $STATE/sm.log 2>/dev/null; }
# SAM (sam.service) is never restarted by anything of LumaTV's: eARC, CEC and the picture mode are lost with it (PLAN,
# "App manager restarts"). Its pid and start time before and after a fallback say it was not.
sam_id() { systemctl show sam.service -p MainPID -p ActiveEnterTimestampMonotonic 2>/dev/null | tr '\n' ' '; }
# After a fallback: wait for the unmount (half seconds, at most $1 of them) and for the recovery of LG's Home.
wait_unmount() { u=0; while is_mounted && [ $u -lt ${1:-40} ]; do sleep 0.5; u=$((u+1)); done; }
wait_recovery() { n=0; while [ -d $RECOVER_LOCK ] && [ $n -lt 120 ]; do sleep 1; n=$((n+1)); done; }

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
    # LUMATV_X1_CALLBACKS=early|landing (tv_gate.py --x1-callbacks) sets when every client's frame callbacks go out for this run (`ctl callbacks
    # clients`): with the frame's draw (early, the compositor's default since 112d9cc) or at the flip (landing), and early again after it.
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
    [ -z "${LUMATV_X1_CALLBACKS:-}" ] || ctl callbacks clients early > /dev/null
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
    awk -v a="${t0:-0}" -v b="${t1:-0}" '/^\[srv/ { t = $2; sub(/\]/, "", t); t += 0; if (t >= a && t <= b + 1 && $0 ~ /loop:|frame gap|client (dis)?connected|atomic commit|page-flip|callbacks held|flip reached|frame held|warm done|warm:|standin:|trace:|bury:|tick:|boxdim:/) print }' \
        $STATE/compositor.log > $OUT/x1-compositor.txt
    stalls=$(grep -c 'loop: [0-9.]* ms in one pass' $OUT/x1-compositor.txt); gaps=$(grep -c 'frame gap' $OUT/x1-compositor.txt); churn=$(grep -c 'client connected' $OUT/x1-compositor.txt)
    # What counts against the rule is what the log says over the limit while the client was alive (not the second after).
    comp_over=$(comp_over_limit $STATE/compositor.log "${t0:-0}" "${t1:-0}")
    # Keep the original strict verdict and raw swaps even when a complete trace attributes gaps to the client.
    raw_strict=$(x1_rule "$res" "$secs" "$late_before" "$late_after" "$comp_over")
    echo "$raw_strict" > $OUT/x1-strict-result.txt
    attrib=""; own=""; coverage="missing capture"
    if [ -n "$kw" ]; then
        coverage=$(sed -n 's/^x1 attribution: //p' $OUT/x1-kwait.txt 2>/dev/null | head -n 1)
        attrib=$(sed -n 's/^x1 gaps: .*compositor-attributable \([0-9]*\) .*client-only \([0-9]*\), unexplained \([0-9]*\)$/\1 \2 \3/p' $OUT/x1-kwait.txt 2>/dev/null | head -n 1)
        [ -z "$attrib" ] || own=$(comp_over_limit $STATE/compositor.log "${t0:-0}" "${t1:-0}" own)
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
    drops=$(grep -c 'message too short\|error in client communication' $STATE/compositor.log)
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
    # end, and every app a key opened is closed again. The gear's three taps and Mute held are left to the host tests: the first opens a
    # service menu, the second would unmute the receiver the gate holds muted if it went wrong.
    kd=""; bad=""
    res() { if [ "$1" = ok ]; then kd="$kd $2 ok;"; else bad="$bad $2 ($3);"; fi; }
    home; sleep 3
    # The gear's tap: LumaTV's quick panel when the shell client says it can show it (`gate.sh shell` checks the panel itself); with no shell,
    # LG's Settings stands in for Quick Settings. Settings is an overlay and slow to map (8.1 s after the tap on the set, window 3 of the
    # first TV run), hence 25 s.
    ctl note keys-gear > /dev/null
    if shell_can quick; then
        key_tap 139
        if wait_shell_str layer quick 10 && journal_after keys-gear | grep -q "input: gear tap: Quick Settings -> LumaTV's quick panel"; then res ok gear-shell
        else res no gear-shell "the shell's layer is '$(shell_str layer)'; or no journal line"; fi
        key_tap 412; sleep 2
    else
        key_tap 139
        if wait_app com.palm.app.settings 25 && journal_after keys-gear | grep -q 'input: gear tap: Quick Settings .*launching com.palm.app.settings'; then res ok gear
        else res no gear "$(where_now); or no journal line"; fi
        close_app com.palm.app.settings; sleep 2; home; sleep 3
    fi
    journal_after keys-gear > $OUT/keys-gear.txt
    # Input's tap: LumaTV's input picker through the shell, or the Home Hub as the picker's stand-in.
    ctl note keys-input > /dev/null
    if shell_can picker; then
        key_tap 241
        if wait_shell_str layer picker 10 && journal_after keys-input | grep -q "input: Input tap: the input picker -> LumaTV's input picker"; then res ok input-shell
        else res no input-shell "the shell's layer is '$(shell_str layer)'; or no journal line"; fi
        key_tap 412; sleep 2
    else
        key_tap 241
        if wait_app com.webos.app.homeconnect 15 && journal_after keys-input | grep -q 'input: Input tap: .*launching com.webos.app.homeconnect'; then res ok input
        else res no input "$(where_now); or no journal line"; fi
        close_app com.webos.app.homeconnect; sleep 2; home; sleep 3
    fi
    journal_after keys-input > $OUT/keys-input.txt
    # The Netflix button follows Settings > Remote: no choice is LG's own target (netflix), a choice is that app, "" is nothing.
    nk=$(ctl keys | grep -o '"netflix":"[^"]*"' | head -n 1)
    if [ -z "$nk" ]; then napp=netflix; nb=netflix-button
    else napp=$(echo "$nk" | cut -d'"' -f4); nb="netflix-button(chosen: ${napp:-nothing})"; fi
    ctl note keys-netflix > /dev/null; key_tap 1037
    if [ -z "$napp" ]; then
        sleep 3
        if [ "$(fg)" = com.webos.app.home ] && ! journal_after keys-netflix | grep -q 'input: app button'; then res ok "$nb"
        else res no "$nb" "it opens nothing, yet foreground $(fg) or an app launch was journalled"; fi
    elif wait_app "$napp" 25 && journal_after keys-netflix | grep -q "input: app button: $napp, launching $napp"; then res ok "$nb"
    else res no "$nb" "$(where_now); or no journal line"; fi
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
    # The shell client under LumaTV's compositor: its surfaces on Home and over an app, each read back from the client's own report
    # (`ctl shell`, the `ui` object the client sends every second and on each change) and the compositor's journal. Keys go through
    # `ctl remote`, the path the remote's take. Home is in front at the start and at the end; the Home Hub, the one app opened, is closed.
    sd=""; bad=""
    res() { if [ "$1" = ok ]; then sd="$sd $2 ok;"; else bad="$bad $2 ($3);"; fi; }
    home; sleep 3
    ctl shell > $OUT/shell-start.json
    if [ "$(shell_bool connected)" = true ] && [ "$(shell_bool homeMapped)" = true ] && [ "$(shell_bool homeDrawn)" = true ]; then res ok connected
    else res no connected "connected $(shell_bool connected), home mapped $(shell_bool homeMapped), drawn $(shell_bool homeDrawn), pid $(shell_num pid)"; fi
    if [ "$(shell_bool homeLoaded)" = true ] && [ "$(shell_bool hostActive)" = true ] && [ "$(shell_bool beat)" = true ]; then res ok home-loaded
    else res no home-loaded "homeLoaded $(shell_bool homeLoaded), hostActive $(shell_bool hostActive), beat $(shell_bool beat)"; fi
    # Idle on Home: the client commits at the wallpaper's tick at most (10 a second) and stays under a third of a core; the compositor
    # takes Home's buffer to the plane whole (the opaque rule in scene.rs), so each costs it a flip.
    set -- $(shell_idle_cost 10); ic=$1; icpu=$2
    if [ "${ic:-99}" -le 12 ] && [ "${icpu:-999}" -le 35 ]; then res ok "idle-home(${ic}/s,${icpu}%)"; else res no idle-home "$ic commits/s, $icpu % cpu on Home at rest"; fi
    # The D-pad: Up from the dock reaches the widgets, Down comes back; the shell counts the keys it got.
    k0=$(shell_num keys); key_tap 103; sleep 1
    f1=$(shell_str focus); key_tap 108; sleep 1
    f2=$(shell_str focus); k1=$(shell_num keys)
    if [ "$f1" = widgets ] && [ "$f2" = dock ] && [ "${k1:-0}" -ge $(( ${k0:-0} + 4 )) ]; then res ok dpad
    else res no dpad "focus '$f1' after Up, '$f2' after Down, keys $k0 -> $k1"; fi
    # The quick panel and the input picker on Home: the gear's tap and Input, Back closes each.
    ctl note shell-quick > /dev/null; key_tap 139
    if wait_shell_str layer quick 10; then key_tap 412; if wait_shell_str layer "" 10; then res ok quick-home; else res no quick-home "Back left the layer '$(shell_str layer)'"; fi
    else res no quick-home "layer '$(shell_str layer)' after the gear; can $(ctl shell | grep -o '"can":\[[^]]*\]')"; fi
    journal_after shell-quick > $OUT/shell-quick.txt
    ctl note shell-picker > /dev/null; key_tap 241
    if wait_shell_str layer picker 10; then key_tap 412; if wait_shell_str layer "" 10; then res ok picker-home; else res no picker-home "Back left the layer '$(shell_str layer)'"; fi
    else res no picker-home "layer '$(shell_str layer)' after Input"; fi
    journal_after shell-picker > $OUT/shell-picker.txt
    # The volume display: a volume key through the compositor's own route; the audio service's push reaches the client as the audio topic.
    key_tap 115
    if wait_shell_bool osd true 8 && wait_shell_bool systemMapped true 6; then res ok osd
    else res no osd "osd $(shell_bool osd), system window mapped $(shell_bool systemMapped) after Volume+ (audio topic: $(ctl shell | grep -o '"audio":{[^}]*' | cut -c1-120))"; fi
    # A toast through the notification service, as any app raises one.
    ls1 com.webos.notification/createToast '{"message":"LumaTV gate: a toast under its own compositor","sourceId":"org.nphil.lumatv"}' > /dev/null
    if wait_shell_num_ge toasts 1 10 && wait_shell_bool systemMapped true 6; then res ok toast
    else res no toast "toasts $(shell_num toasts), system window mapped $(shell_bool systemMapped) after createToast"; fi
    # Over an app (the Home Hub, a card): the system layer draws the quick panel, takes the keys while it is up and lets go on Back.
    ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.homeconnect"}' > /dev/null
    if wait_app com.webos.app.homeconnect 20; then
        sleep 3
        ctl note shell-quick-app > /dev/null; key_tap 139
        if wait_shell_str overlay quick 10 && wait_shell_bool wants true 4 && wait_shell_bool systemMapped true 6; then
            key_tap 412
            if wait_shell_str overlay "" 10 && wait_shell_bool wants false 4; then res ok quick-app; else res no quick-app "Back left overlay '$(shell_str overlay)', wants $(shell_bool wants)"; fi
        else res no quick-app "overlay '$(shell_str overlay)', wants $(shell_bool wants), system mapped $(shell_bool systemMapped), home drawn $(shell_bool homeDrawn)"; fi
        journal_after shell-quick-app > $OUT/shell-quick-app.txt
        # Covered by the app: Home draws nothing and the client sleeps (its tick is held, `covered` in the foreground topic).
        sleep 2
        set -- $(shell_idle_cost 10); ac=$1; acpu=$2
        if [ "${ac:-99}" -eq 0 ] && [ "${acpu:-999}" -le 3 ]; then res ok "idle-covered(${acpu}%)"; else res no idle-covered "$ac commits/s, $acpu % cpu with the Home Hub in front; covered $(ctl shell | grep -o '"covered":[a-z]*')"; fi
    else res no quick-app "the Home Hub did not map: $(where_now)"; fi
    close_app com.webos.app.homeconnect; sleep 3; home; sleep 4
    # The screensaver: tvpower asked to start it; LumaTV's own answers and draws (on Home, LumaHome's). Any key ends it.
    ls1 com.webos.service.tvpower/power2/turnOnScreenSaver '{}' > /dev/null
    if wait_shell_bool saver true 15; then key_tap 106; if wait_shell_bool saver false 10; then res ok saver; else res no saver "a key did not end it"; fi
    else res no saver "saver $(shell_bool saver) after turnOnScreenSaver; $(ctl shell | grep -oE '"vote":\{"shell":(null|\{[^}]*\}),"home":(null|\{[^}]*\})\}')"; fi
    sleep 2; home; sleep 2
    ctl shell > $OUT/shell-end.json
    detail="${sd# }${bad:+ NOT:$bad} (shell pid $(shell_num pid), starts $(ctl shell | grep -o '"starts":[0-9]*' | cut -d: -f2), exits $(ctl shell | grep -o '"exits":[0-9]*' | cut -d: -f2), luna calls $(ctl shell | grep -o '"lunaCalls":[0-9]*' | cut -d: -f2), refused $(ctl shell | grep -o '"lunaRefused":[0-9]*' | cut -d: -f2); $(where_now))"
    echo "$detail" > $OUT/shell-result.txt
    if [ -z "$bad" ]; then pass shell "$detail"; else fail shell "$detail"; fi
    ;;
rust)
    # The Rust shell's kinds one at a time (docs/research/rust-shell-plan.md, section 6; LUMATV_RUST_KINDS, space or comma separated, names the kinds,
    # tv_gate.py --rust-kinds): each is switched on (`ctl shell rust +KIND`, which restarts the client with it), read back from the client's own report (it
    # must be in rust.ui.kinds), shown the way the set shows it, timed to the first reading that says it is up, checked for frames drawn and GL errors
    # while it is up, closed, and switched off (`-KIND`: gone from the list and from the client's kinds), then left as it was found. A kind the compositor
    # refuses or the client does not have is SKIPPED, not failed, and so is a built one the gate has no way to open (rust_undriven); the line says which and why.
    # With the Rust shell unavailable (a bundle without `ui`) nothing runs, and the line says so without a verdict. The kinds list is put back exactly.
    asked=$(echo "${LUMATV_RUST_KINDS:-home osd toast alert quick picker drawer settings cameras saver menu confirm switcher cards popup capture layout}" | tr ',' ' ')
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
    # Whatever happened above, the compositor's list is what it was.
    now=$(rust_kinds_on)
    if [ "$(rust_sorted "$now")" != "$(rust_sorted "$k0")" ]; then
        for k in $(echo "$k0" | tr ',' ' '); do ctl shell rust +$k > /dev/null; done
        for k in $(echo "$now" | tr ',' ' '); do case ",$k0," in *",$k,"*) ;; *) ctl shell rust -$k > /dev/null ;; esac; done
        [ "$(rust_sorted "$(rust_kinds_on)")" = "$(rust_sorted "$k0")" ] || bad="$bad kinds not restored (found [$k0], now [$(rust_kinds_on)]);"
    fi
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
    grep -E 'client (dis)?connected' $STATE/compositor.log > $OUT/clients.txt
    # The wire as a client sees it (globals, seats, output, webos seats, starfish timer), to set against LG's run of 2026-10-28.
    [ -f $D/wlprobe.py ] && XDG_RUNTIME_DIR=/tmp/xdg timeout 20 python3 $D/wlprobe.py /tmp/xdg/wayland-0 2 > $OUT/wlprobe.txt 2>&1
    cp $STATE/compositor.log $OUT/compositor.log; run_log > $OUT/sm.log
    echo "evidence: $(grep -c applicationManager $OUT/sm-subscriptions.txt) subscription(s) from SAM, $(wc -l < $OUT/clients.txt) client lines, $(grep -c 'loop:' $OUT/compositor.log) loop stalls, $(wc -l < $OUT/sm-watchdog.txt) watchdog lines"
    ;;
callbacks)
    # The shell client's frame callbacks (`ctl callbacks shell early|landing`): with an argument it sets them, then says how many of all
    # clients' callbacks went out ahead of their flip so far (every client's go ahead by default). Not a check: no GATE line.
    [ -z "${2:-}" ] || ctl callbacks shell "$2" > /dev/null
    st=$(ctl status)
    echo "callbacks${2:+ (the shell's set to $2)}: $(echo "$st" | grep -o '"callbacksAhead":[0-9]*' | cut -d: -f2) of $(echo "$st" | grep -o '"callbacks":[0-9]*' | cut -d: -f2) sent ahead of their flip"
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
    r_shell=$(ctl shell)
    shell_hz=$(echo "$r_shell" | grep -o '"refreshHz":[0-9]*' | head -n 1 | cut -d: -f2)
    # the per-frame motions' own count (kit/FrameMotion.qml): the last few slides as the shell's state reports them (`Lu.slides`, steps/ms/duration
    # each; round 15 found no line of theirs in the shell's stdout file), else the `[FrameMotion]` lines of the runner's log, which QML's console
    # writes to PmLog, /var/log/messages (it rotates about every minute: the file and the ones it rotated to, oldest first). `gate.py` reads the
    # same lines per surface (`slides:`).
    # (by key: the compositor's JSON has its keys in alphabetical order, not the shell's)
    slides=$(echo "$r_shell" | grep -o '"slides":\[[^]]*\]' | head -n 1 | grep -o '{[^}]*}' | awk '{ s = m = d = "?"
        if (match($0, /"steps":[0-9]+/)) s = substr($0, RSTART + 8, RLENGTH - 8)
        if (match($0, /"ms":[0-9]+/)) m = substr($0, RSTART + 5, RLENGTH - 5)
        if (match($0, /"duration":[0-9]+/)) d = substr($0, RSTART + 11, RLENGTH - 11)
        printf "%s/%s/%s ", s, m, d }')
    [ -n "$slides" ] || slides=$( { cat $STATE/shell.log 2>/dev/null; for f in $(ls -tr $MESSAGES $MESSAGES.* 2>/dev/null); do case $f in *.gz|*.xz|*.bz2) ;; *) cat "$f" ;; esac; done; } | grep -o '\[FrameMotion\] [0-9]* steps in [0-9]* ms for [0-9]*' | tail -n 6 | sed 's/\[FrameMotion\] \([0-9]*\) steps in \([0-9]*\) ms for \([0-9]*\)/\1\/\2\/\3/' | tr '\n' ' ')
    echo "$m" > $OUT/refresh-${r_hz:-x}-${2:-read}.json
    echo "GATE refresh: measured hz ${r_hz:-?} divider ${r_div:-unreadable} shell ${shell_hz:-?} over ${r_secs} s: OSD flips/s $(per_s "$f0" "$f1"), DE2VSYNC/s $(per_s "$v0" "$v1"), osd_irq/s $(per_s "$o0" "$o1"); compositor: $(echo "$m" | grep -o '"intervals":{[^}]*}') $(echo "$m" | grep -o '"frames":{[^}]*}') $(echo "$m" | grep -o '"pipeline_ms":{[^}]*}') flips/s $(echo "$m" | grep -o '"flips_per_second":[0-9.]*' | cut -d: -f2); mali busy ${mali} %; temperature ${temp:-unreadable}; slides (steps/ms/for): ${slides:-none}"
    ;;
scanout)
    # Direct scan-out (`ctl scanout`): `direct` sets the shell client's environment for the layout the plane takes, turns the switch on (the protocol
    # tap goes on the display now) and starts the shell again, since a buffer made before the tap has no descriptor and is composed; `off` turns it off;
    # `read` only reports. A FAIL is a refused command; what the counters say is for the reader.
    case "${2:-read}" in
    direct)
        ctl scanout shell-env WAYLAND_AFBC=0 > /dev/null
        r_set=$(ctl scanout direct)
        if ! echo "$r_set" | grep -q '"ok":true'; then echo "GATE scanout: FAIL $r_set"; exit 1; fi
        ctl shell restart > /dev/null
        wait_shell_bool homeDrawn true 40
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
escape)
    # Home as LG's RF_HOME (773, what PlainHome reads as the Magic Remote's Home and perf/kb.py injects); the compositor's tests cover 172 and 102.
    sam0=$(sam_id)
    ctl remote 773 press > /dev/null
    sleep 1
    ctl remote 773 release > /dev/null 2>&1
    wait_unmount 40
    sleep 2
    fb=$(grep 'fallback:' $STATE/sm.log | tail -1)
    esc=$([ -f $STATE/escaped ] && echo yes || echo no)
    exe=$(readlink /proc/$(pgrep -x surface-manager | head -1)/exe 2>/dev/null)
    # recover-home.sh (started by the supervisor) brings Home back; wait for it.
    wait_recovery
    sleep 2
    b=$(cap $OUT/escape.png)
    sam1=$(sam_id)
    detail="unmounted after $((u/2))s; $fb; escaped $esc; exe $exe; recovery ${n}s; panel ${b:-0} bytes; wam $(systemctl is-active webapp-mgr); sam $([ "$sam0" = "$sam1" ] && echo unchanged || echo "CHANGED $sam0 -> $sam1")"
    echo "$detail" > $OUT/escape-result.txt
    if ! is_mounted && [ "$esc" = yes ] && echo "$fb" | grep -q 'hold Home' && [ "${b:-0}" -gt 4096 ] && [ "$sam0" = "$sam1" ]; then pass escape "$detail"; else fail escape "$detail"; fi
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
    # A virtual remote holds Home for 3 s (uinput); the supervisor's question for the early boot hook, asked from a fresh
    # process in the middle of it, says yes, and after the release says no. Run while LG's compositor is live: the
    # supervisor's own reader would take the key as the escape.
    if [ ! -c /dev/uinput ] || [ ! -x $STATE/bin/lumatv-sm ] || [ ! -f $D/uinput-key.py ]; then
        fail homeheld "no /dev/uinput, or the bundle or $D/uinput-key.py is missing"
    else
        python3 $D/uinput-key.py hold 3 --settle 0.5 > $OUT/homeheld-inject.txt 2>&1 &
        ip=$!
        sleep 2
        $STATE/bin/lumatv-sm --state $STATE --home-held 700; held=$?
        wait $ip; injrc=$?
        sleep 0.5
        $STATE/bin/lumatv-sm --state $STATE --home-held 700; after=$?
        detail="while held: exit $held (0 = held); after the release: exit $after (1 = not held); injector exit $injrc"
        echo "$detail" > $OUT/homeheld-result.txt
        if [ "$held" = 0 ] && [ "$after" = 1 ] && [ "$injrc" = 0 ]; then pass homeheld "$detail"; else fail homeheld "$detail"; fi
    fi
    ;;
homekey)
    # The supervisor's own reader: the virtual remote is made after the compositor started, so the compositor (which
    # opens the event nodes once) never sees it and only the supervisor's rescan can. Home held 1.5 s on it must end the
    # session as an escape: recorded as one, not counted as a failure, SAM untouched.
    if ! is_mounted || [ ! -c /dev/uinput ] || [ ! -f $D/uinput-key.py ]; then
        fail homekey "not live, or no /dev/uinput, or $D/uinput-key.py is missing"
    else
        sam0=$(sam_id); f0=$(cat $STATE/failures 2>/dev/null || echo 0)
        python3 $D/uinput-key.py hold 1.5 --settle 9 > $OUT/homekey-inject.txt 2>&1 &
        ip=$!
        wait_unmount 60
        wait $ip; injrc=$?
        sleep 2
        wait_recovery
        sleep 2
        fb=$(grep 'fallback:' $STATE/sm.log | tail -1)
        esc=$([ -f $STATE/escaped ] && echo yes || echo no)
        f1=$(cat $STATE/failures 2>/dev/null || echo 0)
        b=$(cap $OUT/homekey.png)
        sam1=$(sam_id)
        detail="unmounted after $((u/2))s; $fb; escaped $esc; failures $f0 -> $f1; panel ${b:-0} bytes; injector exit $injrc; sam $([ "$sam0" = "$sam1" ] && echo unchanged || echo "CHANGED $sam0 -> $sam1")"
        echo "$detail" > $OUT/homekey-result.txt
        if ! is_mounted && [ "$esc" = yes ] && echo "$fb" | grep -q 'hold Home 650 ms (supervisor)' && [ "$f0" = "$f1" ] && [ "${b:-0}" -gt 4096 ] && [ "$sam0" = "$sam1" ]; then pass homekey "$detail"; else fail homekey "$detail"; fi
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
        wait_recovery
        sleep 2
        reason=$(cat $STATE/last-fallback 2>/dev/null)
        f1=$(cat $STATE/failures 2>/dev/null || echo 0)
        gone=$(pgrep -f 'bin/lumatv-compositor' | head -1)
        b=$(cap $OUT/$1.png)
        sam1=$(sam_id)
        detail="unmounted after $((took/2)).$((took%2*5)) s; reason: $reason; failures $f0 -> $f1; escaped $([ -f $STATE/escaped ] && echo yes || echo no); compositor left: ${gone:-none}; panel ${b:-0} bytes; wam $(systemctl is-active webapp-mgr); sam $([ "$sam0" = "$sam1" ] && echo unchanged || echo "CHANGED $sam0 -> $sam1"); crash bundle $(crash_summary)"
        echo "$detail" > $OUT/$1-result.txt
        if ! is_mounted && [ "$took" -le "$limit" ] && echo "$reason" | grep -q "$want" && [ "$f1" = "$((f0+1))" ] && [ ! -f $STATE/escaped ] && [ -z "$gone" ] && [ "${b:-0}" -gt 4096 ] && [ "$sam0" = "$sam1" ]; then pass $1 "$detail"; else fail $1 "$detail"; fi
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
