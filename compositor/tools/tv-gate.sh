#!/bin/sh
# The C4 gate, executed ON THE SET as /tmp/lumatv/gate.sh while LumaTV's compositor is live (cutover.sh enable --session).
# Each check prints one "GATE <name>: PASS|FAIL <detail>" line; files under /tmp/lumatv/out/gate/. Driven from
# the workstation by tools/tv_gate.py, which mutes the receiver, wakes the set and orders the checks.
#
#   gate.sh live [LABEL]  LumaTV is the compositor: mounted, 12 names registered, no fallback, WAM up, Home (the web
#                     stub) mapped and on the panel, the watchdog reading the compositor's counters and the power state
#   gate.sh x1 SECS   the X1 probe client (64-bit, LG's blob) for SECS: the frame rule below
#   gate.sh netflix   the Dolby Vision test film by deep link, from a fresh Netflix instance (a running one is
#                     closed first): card mapped, video window exported, Picture Mode dolbyHdrCinema while
#                     playing, filmMaker back on Home
#   gate.sh hdmi      HDMI 2 (the receiver) as the foreground app, its card mapped, then Home and HDMI 2 ended through SAM
#   gate.sh keys      the remote's own keys through the compositor's routing (keys.rs): a gear tap, Input, the Netflix button, a digit tapped
#                     and held, Exit; each judged by the compositor's journal and by what SAM then has in front
#   gate.sh ime       a text field in LG's Search app brings up maliit's keyboard: an input method bound, a context made for the field,
#                     the keyboard's window on the screen at the bottom, a key typed on it reaching the field, the keyboard closing
#   gate.sh shell     the shell client (shell.rs, compositor/shell): connected, LumaTV's Home drawn in the stub's place and taking the
#                     D-pad, the quick panel and the input picker on the gear and Input (on Home and over an app, where the system
#                     layer takes the keys and lets go), the volume display on a volume key, a toast, the screensaver when tvpower
#                     starts it; Home in front and no app left at the end
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
#   gate.sh wd-crash  live: the compositor is killed (SIGKILL); the supervisor falls back to LG's, SAM untouched
#
# The frame rule (PLAN, C4: "the X1 test with zero frames over 20 ms in 60 s"). The client's swap-to-swap gaps over
# the run: none over 20 ms, the first second included (no warm-up is excused); the run is the whole 60 s and holds 60 Hz
# (59.5 fps or more); the pacer's `late` did not move; and the compositor's own log of the run (from the client's start
# to its end) records no loop pass, client frame gap or page-flip interval over 20 ms either.
STATE=${LUMATV_STATE:-/var/lib/lumatv/compositor}
D=${LUMATV_D:-/tmp/lumatv}
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
BLOB=$LIB/libmali-bifrost-g52-g24p0-wayland-gbm.so
OUT=${LUMATV_OUT:-$D/out/gate}
LGBIN=${LUMATV_LG:-/usr/bin/surface-manager}
MOUNTINFO=${LUMATV_MOUNTINFO:-/proc/self/mountinfo}
RECOVER_LOCK=${LUMATV_RECOVER_LOCK:-/tmp/lumatv-recover.lock}
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
# The app ids of the compositor's mapped windows, one a line. SAM's foreground answer names the front card only: LG's Settings and Search are
# overlays, which it never names (round 11), so an app a key launched is looked for in the compositor's own window list.
mapped_apps() { ctl windows | sed 's/{"appId"/\n&/g' | grep '"mapped":true' | sed -n 's/^{"appId":"\([^"]*\)".*/\1/p'; }
app_mapped() { mapped_apps | grep -qx "$1"; }
# Waits, SECS at most, for a window of APP to be mapped.
wait_app() { w=0; while [ "$w" -lt "$2" ]; do app_mapped "$1" && return 0; sleep 1; w=$((w+1)); done; app_mapped "$1"; }
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
# refresh skipped with nothing due at it is logged as `page-flip interval ... nothing due`, and is not one).
comp_over_limit() {
    [ -r "$1" ] || { echo unreadable; return 0; }
    awk -v a="$2" -v b="$3" -v lim="$GAP_LIMIT_MS" '
        /^\[srv/ { t = $2; sub(/\]/, "", t); t += 0; if (t < a || t > b) next }
        /loop: [0-9.]+ ms in one pass/ { v = $0; sub(/.*loop: /, "", v); if (v + 0 > lim) n++ }
        /frame gap [0-9.]+ ms on surface/ { v = $0; sub(/.*frame gap /, "", v); if (v + 0 > lim) n++ }
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

# x1_rule RESULT SECONDS_ASKED LATE_BEFORE LATE_AFTER COMPOSITOR_OVER (a count, or "unreadable"): prints "PASS what held"
# or "FAIL why", returns 0 or 1.
x1_rule() {
    r_res=$1; r_asked=${2:-0}; r_lb=${3:-}; r_la=${4:-}; r_comp=${5:-}
    r_n=$(x1_numbers "$r_res")
    if [ -z "$r_n" ]; then echo "FAIL the X1 client printed no result"; return 1; fi
    case "$r_asked" in ''|*[!0-9]*) r_asked=0 ;; esac
    set -- $r_n
    r_secs=$2; r_fps=$3; r_over=$(( $6 + $7 )); r_worst=$8
    r_why=""
    if [ "$r_over" -gt 0 ]; then r_why="$r_why; $r_over gap(s) over ${GAP_LIMIT_MS} ms in the client's swaps (worst $r_worst ms)"; fi
    if [ "$r_asked" -lt "$GATE_SECONDS" ]; then r_why="$r_why; the run asked for $r_asked s, the rule is ${GATE_SECONDS} s"; fi
    if awk -v s="$r_secs" -v m="$GATE_SECONDS" 'BEGIN { exit !(s + 0 < m - 0.5) }'; then r_why="$r_why; the run lasted $r_secs s"; fi
    if awk -v f="$r_fps" -v m="$MIN_FPS" 'BEGIN { exit !(f + 0 < m + 0) }'; then r_why="$r_why; $r_fps fps is under $MIN_FPS"; fi
    if [ -z "$r_lb" ] || [ -z "$r_la" ]; then r_why="$r_why; the pacer's late counter was not read"
    elif [ "$r_lb" != "$r_la" ]; then r_why="$r_why; pacer late $r_lb -> $r_la"; fi
    case "$r_comp" in
    ''|*[!0-9]*) r_why="$r_why; the compositor's log of the run could not be read" ;;
    0) ;;
    *) r_why="$r_why; the compositor logged $r_comp loop pass(es), client frame gap(s) or page-flip interval(s) over ${GAP_LIMIT_MS} ms during the run" ;;
    esac
    if [ -n "$r_why" ]; then echo "FAIL ${r_why#; }"; return 1; fi
    echo "PASS $r_fps fps over $r_secs s, no gap over ${GAP_LIMIT_MS} ms, pacer late unchanged, nothing over ${GAP_LIMIT_MS} ms in the compositor's log"
    return 0
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
    XDG_RUNTIME_DIR=/tmp/xdg WAYLAND_DISPLAY=wayland-0 X1_SECONDS=$secs X1_WINDOW_TYPE=_WEBOS_WINDOW_TYPE_OVERLAY \
        $LD --library-path $LIB --preload $LIB/x1heapshim.so $D/bin/x1client $BLOB > $OUT/x1.log 2>&1
    res=$(grep RESULT $OUT/x1.log)
    after=$(ctl status | sed -n 's/.*\(flips [0-9]* late [0-9]* lost [0-9]*\).*/\1/p')
    sched=$(echo "$sched_before $(comp_sched)" | awk 'function pol(v) { return (v == 1) ? "SCHED_FIFO" : (v == 0) ? "SCHED_OTHER" : "policy " v }
        { f = ($1 ~ /^[0-9]+$/ && $7 ~ /^[0-9]+$/) ? sprintf("+%d", $7 - $1) : "not read"; w = ($2 ~ /^[0-9]+$/ && $8 ~ /^[0-9]+$/) ? sprintf("+%.1f ms", ($8 - $2) / 1e6) : "not read";
          mw = ($5 ~ /^[0-9]+$/ && $11 ~ /^[0-9]+$/) ? sprintf("+%.1f ms", ($11 - $5) / 1e6) : "not read";
          print "major faults " f ", run-queue wait " w ", loop thread " pol($9) " prio " $10 ", mali-cmar-backe run-queue wait " mw " " pol($12) }')
    # `late` alone: `lost` is the pacer's own measure, which leaves a frame's render time out.
    late_before=$(echo "$before" | awk '{print $4}'); late_after=$(echo "$after" | awk '{print $4}')
    # The compositor's own view of the run (its log is on CLOCK_MONOTONIC like x1's): loop stalls, frame gaps of the
    # X1 surface, clients coming and going, refused commits, lost flips, a flip the loop reached late and where the loop
    # was then, a frame held while a driver buffer warms, a surface stood in for, each buffer's warm, buffers freed. The
    # gaps' cause is read from these lines; only the ones over the limit count against the rule (comp_over_limit).
    t0=$(sed -n 's/.*starting at monotonic \([0-9.]*\).*/\1/p' $OUT/x1.log); t1=$(sed -n 's/.*ended at monotonic \([0-9.]*\).*/\1/p' $OUT/x1.log)
    awk -v a="${t0:-0}" -v b="${t1:-0}" '/^\[srv/ { t = $2; sub(/\]/, "", t); t += 0; if (t >= a && t <= b + 1 && $0 ~ /loop:|frame gap|client (dis)?connected|atomic commit|page-flip|callbacks held|flip reached|frame held|warm done|warm:|standin:|trace:|bury:|tick:/) print }' \
        $STATE/compositor.log > $OUT/x1-compositor.txt
    stalls=$(grep -c 'loop: [0-9.]* ms in one pass' $OUT/x1-compositor.txt); gaps=$(grep -c 'frame gap' $OUT/x1-compositor.txt); churn=$(grep -c 'client connected' $OUT/x1-compositor.txt)
    # What counts against the rule is what the log says over the limit while the client was alive (not the second after).
    comp_over=$(comp_over_limit $STATE/compositor.log "${t0:-0}" "${t1:-0}")
    detail="$res | pacer before: $before after: $after | compositor: loop stalls $stalls, frame gaps $gaps, clients connected $churn, over ${GAP_LIMIT_MS} ms $comp_over; $sched"
    echo "$detail" > $OUT/x1-result.txt
    verdict=$(x1_rule "$res" "$secs" "$late_before" "$late_after" "$comp_over")
    case "$verdict" in PASS*) pass x1 "${verdict#PASS }" ;; *) fail x1 "${verdict#FAIL } | $detail" ;; esac
    ;;
netflix)
    # A Netflix instance that was preloaded under another compositor (or while this one's services were still
    # coming up) keeps what it bound at its start and shows tvq-pb-101 at play (third and fifth gates; the same
    # error under LG's compositor after declutter's runtime changes). The check is a fresh start: a running
    # instance is ended and the preload manager loads a new one, which the launch then takes.
    closed=0
    held=$(held_port)
    if ls1 com.webos.applicationManager/running '{}' | grep -q '"id":"netflix"'; then
        ls1 com.webos.applicationManager/closeByAppId '{"id":"netflix"}' > /dev/null
        closed=1
        n=0; while [ $n -lt 15 ] && ls1 com.webos.applicationManager/running '{}' | grep -q '"id":"netflix"'; do sleep 1; n=$((n+1)); done
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
    # serde_json writes keys in order: appId, id, keyMask, mapped.
    mapped=$(grep -o '"appId":"com.webos.app.hdmi2","id":[0-9]*,"keyMask":[0-9]*,"mapped":true' $OUT/hdmi-windows.json | wc -l)
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
    close_app com.webos.app.magicnum; sleep 2; home; sleep 3
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
    if wait_shell_bool osd true 8; then res ok osd; else res no osd "osd $(shell_bool osd) after Volume+ (audio topic: $(ctl shell | grep -o '"audio":{[^}]*' | cut -c1-120))"; fi
    # A toast through the notification service, as any app raises one.
    ls1 com.webos.notification/createToast '{"message":"LumaTV gate: a toast under its own compositor","sourceId":"org.nphil.lumatv"}' > /dev/null
    if wait_shell_num_ge toasts 1 10; then res ok toast
    else res no toast "toasts $(shell_num toasts) after createToast"; fi
    # Over an app (the Home Hub, a card): the system layer draws the quick panel, takes the keys while it is up and lets go on Back.
    ls1 com.webos.applicationManager/launch '{"id":"com.webos.app.homeconnect"}' > /dev/null
    if wait_app com.webos.app.homeconnect 20; then
        sleep 3
        ctl note shell-quick-app > /dev/null; key_tap 139
        if wait_shell_str overlay quick 10 && wait_shell_bool wants true 4 && [ "$(shell_bool systemMapped)" = true ]; then
            key_tap 412
            if wait_shell_str overlay "" 10 && wait_shell_bool wants false 4; then res ok quick-app; else res no quick-app "Back left overlay '$(shell_str overlay)', wants $(shell_bool wants)"; fi
        else res no quick-app "overlay '$(shell_str overlay)', wants $(shell_bool wants), system mapped $(shell_bool systemMapped), home drawn $(shell_bool homeDrawn)"; fi
        journal_after shell-quick-app > $OUT/shell-quick-app.txt
    else res no quick-app "the Home Hub did not map: $(where_now)"; fi
    close_app com.webos.app.homeconnect; sleep 3; home; sleep 4
    # The screensaver: tvpower asked to start it; LumaTV's own answers and draws (on Home, LumaHome's). Any key ends it.
    ls1 com.webos.service.tvpower/power2/turnOnScreenSaver '{}' > /dev/null
    if wait_shell_bool saver true 15; then key_tap 106; if wait_shell_bool saver false 10; then res ok saver; else res no saver "a key did not end it"; fi
    else res no saver "saver $(shell_bool saver) after turnOnScreenSaver"; fi
    sleep 2; home; sleep 2
    ctl shell > $OUT/shell-end.json
    detail="${sd# }${bad:+ NOT:$bad} (shell pid $(shell_num pid), starts $(ctl shell | grep -o '"starts":[0-9]*' | cut -d: -f2), exits $(ctl shell | grep -o '"exits":[0-9]*' | cut -d: -f2), luna calls $(ctl shell | grep -o '"lunaCalls":[0-9]*' | cut -d: -f2), refused $(ctl shell | grep -o '"lunaRefused":[0-9]*' | cut -d: -f2); $(where_now))"
    echo "$detail" > $OUT/shell-result.txt
    if [ -z "$bad" ]; then pass shell "$detail"; else fail shell "$detail"; fi
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
        bottom=no; if [ -n "$ph" ] && [ -n "$py" ] && [ $((py + ph)) -eq 1080 ]; then bottom=yes; fi
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
    cp $STATE/compositor.log $OUT/compositor.log; run_log > $OUT/sm.log
    echo "evidence: $(grep -c applicationManager $OUT/sm-subscriptions.txt) subscription(s) from SAM, $(wc -l < $OUT/clients.txt) client lines, $(grep -c 'loop:' $OUT/compositor.log) loop stalls, $(wc -l < $OUT/sm-watchdog.txt) watchdog lines"
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
        detail="unmounted after $((took/2)).$((took%2*5)) s; reason: $reason; failures $f0 -> $f1; escaped $([ -f $STATE/escaped ] && echo yes || echo no); compositor left: ${gone:-none}; panel ${b:-0} bytes; wam $(systemctl is-active webapp-mgr); sam $([ "$sam0" = "$sam1" ] && echo unchanged || echo "CHANGED $sam0 -> $sam1")"
        echo "$detail" > $OUT/$1-result.txt
        if ! is_mounted && [ "$took" -le "$limit" ] && echo "$reason" | grep -q "$want" && [ "$f1" = "$((f0+1))" ] && [ ! -f $STATE/escaped ] && [ -z "$gone" ] && [ "${b:-0}" -gt 4096 ] && [ "$sam0" = "$sam1" ]; then pass $1 "$detail"; else fail $1 "$detail"; fi
    fi
    ;;
key)
    # One remote key through the compositor's own routing (tv_gate.py's volume check).
    ctl remote "$2" tap
    ;;
*) sed -n '2,/^[^#]/{/^#/p;}' $0 ;;
esac
