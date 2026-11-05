#!/bin/sh
# ON THE SET (busybox sh, as root), with LumaTV's compositor live and boot direct on, the web stub in. Tries the native Home stub
# (docs/research/boot-direct.md, "A native stub") in about four minutes and puts the web stub back, whatever happens.
#
#   sh homestub-native-tv.sh [--keep] [--no-app] [--no-saver] [--no-wake] [--out DIR] [--budget SECONDS]
#
#   --keep       leave the native stub in place when every check that ran passed (default: the web stub is back at the end)
#   --no-app     skip the app over Home (HDMI 2 by default, PROBE_APP=<id> for another): launch, Home key, closed again
#   --no-saver   skip the screensaver over Home
#   --no-wake    skip the standby and wake (it is skipped by itself when LG's panel refresh is due: a wake would cut it short)
#   --budget N   seconds after which a detached switch gives the web stub back if this script was killed (default 420)
#
# What it needs on the set first (copy only; nothing here installs anything): tools/lumatv with the homestub-kind verb, payload/homestub-native/
# (appinfo.json and the program, `sh homestub/build.sh` builds it), the compositor bundle installed, `lumatv boot-direct on` with the web stub in.
#
# Checks, each one line `PROBE name: PASS|FAIL|INFO|NOT RUN detail`: the swap; SAM's type for Home is native; SAM lists Home running; one
# stub process; the compositor has Home's card, mapped, a card, one pixel; Home is in front; the program's RSS and PSS against the 2 MB target;
# `luna-send launch` three times (and whether the process was kept or restarted); a preload start of Home, which maps nothing, and the launch
# after it; an app over Home and the Home key and a launch each bringing Home back; the screensaver over Home; a standby and wake with Home
# alive after it; LG's Flutter Home never started; the web stub back.
# The last line is the verdict and the receipt directory (the log, SAM's and the compositor's views, /proc of the program, the hashes).
# Exit: 0 every check that ran passed, 1 a check failed (the web stub is back), 2 refused before changing anything.
#
# Home held 10 s on the remote is the escape if the set is left on anything but LumaTV's Home.
LUMATV=${LUMATV_TOOL:-/media/developer/apps/usr/palm/applications/org.nphil.lumatv/tools/lumatv}
COMP=${PROBE_COMPOSITOR:-/var/lib/lumatv/compositor}
PROC=${PROBE_PROC:-/proc}
MSGLOG=${PROBE_MESSAGES:-/var/log/messages}
APP=${PROBE_APP:-com.webos.app.hdmi2}
HOME_ID=com.webos.app.home
COMM=lumatv-homestub
RSS_LIMIT_KB=2048
KEEP=0; DO_APP=1; DO_SAVER=1; DO_WAKE=1; BUDGET=420; OUT=
while [ $# -gt 0 ]; do
    case "$1" in
        --keep) KEEP=1 ;; --no-app) DO_APP=0 ;; --no-saver) DO_SAVER=0 ;; --no-wake) DO_WAKE=0 ;;
        --out) OUT=$2; shift ;; --budget) BUDGET=$2; shift ;;
        *) sed -n '2,25p' "$0"; exit 2 ;;
    esac
    shift
done

up() { read -r _u _ < "$PROC/uptime"; echo "${_u%.*}"; }
T0=$(up)
[ -n "$OUT" ] || OUT=/tmp/homestub-native-probe-$T0
mkdir -p "$OUT" || exit 2
LOG=$OUT/probe.log
say() { printf '%s\n' "$*" | tee -a "$LOG"; }
PASSED=0; FAILED=0; NOTRUN=0; NOTRUN_LIST=
result() { # NAME PASS|FAIL|INFO|NOT-RUN DETAIL
    case "$2" in
        PASS) PASSED=$((PASSED+1)) ;; FAIL) FAILED=$((FAILED+1)) ;;
        NOT-RUN) NOTRUN=$((NOTRUN+1)); NOTRUN_LIST="$NOTRUN_LIST $1" ;;
    esac
    say "PROBE $1: $(printf '%s' "$2" | tr '-' ' ') $3"
}
pass() { result "$1" PASS "$2"; }
fail() { result "$1" FAIL "$2"; }
info() { result "$1" INFO "$2"; }
skip() { result "$1" NOT-RUN "$2"; }
elapsed() { echo $(( $(up) - T0 )); }

# ---- the set, as the other TV scripts read it
ls1() { timeout 10 luna-send -n 1 -f "luna://$1" "$2" 2>/dev/null; }
flat() { tr -d ' \r\n'; }
jstr() { grep -o "\"$1\":\"[^\"]*\"" | head -n 1 | cut -d'"' -f4; }
jnum() { grep -o "\"$1\":-\{0,1\}[0-9]*" | head -n 1 | cut -d: -f2; }
jbool() { grep -o "\"$1\":\(true\|false\)" | head -n 1 | cut -d: -f2; }
ctl() { "$COMP/lib/ld-linux-aarch64.so.1" --library-path "$COMP/lib" "$COMP/bin/lumatv-compositor" ctl --runtime-dir /tmp/xdg "$@" 2>/dev/null; }
fg() { ls1 com.webos.applicationManager/getForegroundAppInfo '{}' | flat | jstr appId; }
running_entry() { ls1 com.webos.applicationManager/running '{}' | flat | grep -o "{[^{}]*\"id\":\"$1\"[^{}]*}" | head -n 1; }
sam_type() { ls1 com.webos.applicationManager/getAppInfo "{\"id\":\"$HOME_ID\"}" | tr -d '\n' | sed -n 's/.*"type"[ ]*:[ ]*"\([^"]*\)".*/\1/p'; }
window_of() { ctl windows | sed 's/{"appId"/\n&/g' | grep "^{\"appId\":\"$1\"" | head -n 1; }
mapped_of() { window_of "$1" | jbool mapped; }
stub_pids() { pgrep -x "$COMM" 2>/dev/null; }
stub_count() { stub_pids | grep -c .; }
stub_pid() { stub_pids | head -n 1; }
flutter_home() { pgrep -f "flutter-client -i $HOME_ID" >/dev/null 2>&1; }
power_state() { ls1 com.webos.service.tvpower/power/getPowerState '{}' | flat | jstr state | tr 'A-Z' 'a-z'; }
# Waits, SECS at most, for a command to succeed (half-second steps).
wait_for() { w=0; while [ "$w" -lt $(( $1 * 2 )) ]; do eval "$2" && return 0; sleep 0.5; w=$((w+1)); done; eval "$2"; }
home_front() { [ "$(fg)" = "$HOME_ID" ]; }
home_runs() { [ -n "$(running_entry $HOME_ID)" ]; }
home_card() { [ "$(mapped_of $HOME_ID)" = true ]; }
no_flutter_mark() { FLUTTER_SEEN=$(( ${FLUTTER_SEEN:-0} + $(flutter_home && echo 1 || echo 0) )); }

snap() { # NAME: what SAM and the compositor say, and the program's own files
    {
        echo "== uptime $(up) elapsed $(elapsed)"; "$LUMATV" homestub-kind status; "$LUMATV" boot-direct status
        ls1 com.webos.applicationManager/running '{}'; ls1 com.webos.applicationManager/getForegroundAppInfo '{}'
        sam_type; ctl windows; stub_pids
    } > "$OUT/$1.txt" 2>&1
    p=$(stub_pid)
    if [ -n "$p" ]; then
        { tr '\0' ' ' < "$PROC/$p/cmdline"; echo; readlink "$PROC/$p/exe"; cat "$PROC/$p/status" "$PROC/$p/smaps_rollup"; } > "$OUT/$1-program.txt" 2>&1
    fi
    # what SAM and the program said to the system log (the program's own words go to stderr, and only on a failure)
    grep -a -e "$COMM" -e "$HOME_ID" "$MSGLOG" 2>/dev/null | tail -n 60 > "$OUT/$1-messages.txt"
}

# ---- preflight: nothing changes before this passes
refuse() { say "PROBE refused: $1"; echo "PROBE: REFUSED (nothing was changed) - $1"; exit 2; }
[ -x "$LUMATV" ] || refuse "no tool at $LUMATV"
kind=$("$LUMATV" homestub-kind status 2>&1)
case "$kind" in *'"ok":true'*) ;; *) refuse "the installed tool has no homestub-kind verb: $kind" ;; esac
case "$kind" in *'"nativeInstalled":true'*) ;; *) refuse "payload/homestub-native is not installed (appinfo.json and the program): $kind" ;; esac
case "$kind" in *'"mounted":"web"'*'"sam":"web"'*) ;; *) refuse "the web stub is not the one in place (lumatv boot-direct on first): $kind" ;; esac
pgrep -f 'bin/lumatv-compositor' >/dev/null 2>&1 || refuse "LumaTV's compositor is not live"
[ "$(power_state)" = active ] || refuse "the set is not Active (power state: $(power_state))"
[ -x "$COMP/bin/lumatv-compositor" ] || refuse "no compositor ctl at $COMP/bin/lumatv-compositor (PROBE_COMPOSITOR)"
[ "$(stub_count)" = 0 ] || refuse "a $COMM process is running already"
flutter_home && refuse "LG's Flutter Home is running"
home_card || refuse "no mapped Home card in the compositor"
HASH_TOOL=$(sha256sum "$LUMATV" | cut -d' ' -f1)
APPDIR=$(dirname "$(dirname "$LUMATV")")
HASH_BIN=$(sha256sum "$APPDIR/payload/homestub-native/lumatv-homestub" 2>/dev/null | cut -d' ' -f1)
HASH_INFO=$(sha256sum "$APPDIR/payload/homestub-native/appinfo.json" 2>/dev/null | cut -d' ' -f1)
say "homestub-native probe, uptime $T0, receipt $OUT"
say "hashes: tool $HASH_TOOL program $HASH_BIN appinfo $HASH_INFO"
nl_shown() { _n=$(grep -c NL_HOME_SHOWN "$MSGLOG" 2>/dev/null); echo "${_n:-0}"; }
NL_BEFORE=$(nl_shown)
snap before

# ---- the way back, armed before the first change: normal ends, signals, and a detached switch for a script that is killed
SWAPPED=0; KEPT=0; DONE=0
MARK=$OUT/deadman.armed
revert() {
    [ "$SWAPPED" = 1 ] || return 0
    SWAPPED=0
    say "reverting to the web stub ($(elapsed) s in)"
    back=$("$LUMATV" homestub-kind web 2>&1)
    say "  homestub-kind web: $back"
    wait_for 20 'home_runs && [ "$(sam_type)" = web ] && [ "$(stub_count)" = 0 ]' >/dev/null 2>&1
    if [ "$(sam_type)" = web ] && home_runs && [ "$(stub_count)" = 0 ] && ! flutter_home; then
        pass revert "the web stub is back: SAM reads web, Home runs, no $COMM process, $(fg) in front"
    else
        fail revert "NOT BACK: SAM reads '$(sam_type)', Home runs: $(home_runs && echo yes || echo no), $COMM processes: $(stub_count); Home held 10 s on the remote, or 'lumatv boot-direct off', is the way out"
        REVERT_FAILED=1
    fi
}
finish() {
    [ "$DONE" = 1 ] && return
    DONE=1
    [ "$KEPT" = 1 ] || revert
    rm -f "$MARK"
    if [ -n "${DEADMAN_PID:-}" ]; then kill -- "-$DEADMAN_PID" 2>/dev/null || kill "$DEADMAN_PID" 2>/dev/null; fi
    snap after
    nl=$(nl_shown)
    [ "$nl" = "$NL_BEFORE" ] && [ "${FLUTTER_SEEN:-0}" = 0 ] \
        && pass flutter "LG's Flutter Home never ran: no process at any check, NL_HOME_SHOWN $NL_BEFORE -> $nl" \
        || fail flutter "LG's Flutter Home started: processes seen ${FLUTTER_SEEN:-0}, NL_HOME_SHOWN $NL_BEFORE -> $nl"
    verdict=PASS; code=0
    [ "$FAILED" = 0 ] || { verdict=FAIL; code=1; }
    [ "${REVERT_FAILED:-0}" = 0 ] || { verdict="FAIL, THE WEB STUB IS NOT BACK"; code=1; }
    extra=
    [ "$NOTRUN" = 0 ] || extra="; NOT RUN:$NOTRUN_LIST"
    [ "$KEPT" = 0 ] || extra="$extra; the native stub is left in place (lumatv homestub-kind web puts the web stub back)"
    say "PROBE: $verdict ($PASSED passed, $FAILED failed, $NOTRUN not run$extra) in $(elapsed) s; receipt $OUT"
    exit $code
}
trap 'say "PROBE interrupted"; FAILED=$((FAILED+1)); finish' INT TERM HUP
trap finish EXIT
: > "$MARK"
if [ "${PROBE_DEADMAN:-1}" = 1 ]; then
    # detached: it outlives this shell, and does nothing once the mark is gone
    setsid sh -c "sleep $BUDGET; [ -e '$MARK' ] && { echo \"deadman \$(date +%T): the probe did not finish, reverting\" >> '$LOG'; '$LUMATV' homestub-kind web >> '$LOG' 2>&1; }" </dev/null >/dev/null 2>&1 &
    DEADMAN_PID=$!
fi

# ---- the swap
t=$(up)
swap=$("$LUMATV" homestub-kind native 2>&1)
SWAPPED=1
say "homestub-kind native: $swap ($(( $(up) - t )) s)"
if ! printf '%s' "$swap" | grep -q '"ok":true'; then
    fail swap "the verb refused or did not take: $swap"
    SWAPPED=0
    snap swap-failed
    finish
fi
case "$swap" in *'"mounted":"native"'*) pass swap "native mounted over LG's Home directory in $(( $(up) - t )) s" ;; *) fail swap "not mounted as native: $swap" ;; esac
sleep 2

# ---- what SAM and the compositor make of it
st=$(sam_type)
[ "$st" = native ] && pass sam-type "SAM reads Home's manifest as native" || fail sam-type "SAM reads '$st'"
entry=$(running_entry $HOME_ID)
if [ -n "$entry" ]; then pass running "SAM lists Home running: $entry"; else fail running "SAM does not list Home as running"; fi
n=$(stub_count)
[ "$n" = 1 ] && pass process "one $COMM process, pid $(stub_pid)" || fail process "$n $COMM processes"
pid=$(stub_pid)
if [ -n "$pid" ]; then
    info process "SAM's processid $(printf '%s' "$entry" | jstr processid), uid $(awk '/^Uid:/ {print $2}' "$PROC/$pid/status" 2>/dev/null), launched as: $(tr '\0' ' ' < "$PROC/$pid/cmdline" 2>/dev/null | cut -c1-200)"
fi
win=$(window_of $HOME_ID)
if [ "$(printf '%s' "$win" | jbool mapped)" = true ] && [ "$(printf '%s' "$win" | jstr type)" = Card ] && printf '%s' "$win" | grep -q '"size":\[1,1\]'; then
    pass window "Home's card is mapped, a Card, 1 pixel, state $(printf '%s' "$win" | jstr state)"
else
    fail window "the compositor's Home window: $(printf '%s' "$win" | cut -c1-300)"
fi
wait_for 5 home_front >/dev/null 2>&1
home_front && pass foreground "SAM's foreground is Home" || fail foreground "SAM's foreground is '$(fg)'"
no_flutter_mark
if [ -n "$pid" ]; then
    rss=$(awk '/^Rss:/ {print $2}' "$PROC/$pid/smaps_rollup" 2>/dev/null); pss=$(awk '/^Pss:/ {print $2}' "$PROC/$pid/smaps_rollup" 2>/dev/null)
    [ -n "$rss" ] || rss=$(awk '/^VmRSS:/ {print $2}' "$PROC/$pid/status" 2>/dev/null)
    if [ -n "$rss" ] && [ "$rss" -le "$RSS_LIMIT_KB" ]; then pass memory "RSS $rss kB, PSS ${pss:-?} kB (target $RSS_LIMIT_KB kB; the web stub was 32.4 MB and 15.3 MB)"; else fail memory "RSS ${rss:-unread} kB, PSS ${pss:-?} kB, over the $RSS_LIMIT_KB kB target"; fi
else
    fail memory "no process to measure"
fi

# ---- luna-send launch: what SAM does with a launch of an app that is running
before=$(stub_pid); L=0; restarts=0
while [ "$L" -lt 3 ]; do
    ls1 com.webos.applicationManager/launch "{\"id\":\"$HOME_ID\"}" >/dev/null
    sleep 1.5
    now=$(stub_pid)
    [ "$now" = "$before" ] || restarts=$((restarts+1)); before=$now
    L=$((L+1))
done
if home_runs && [ "$(stub_count)" = 1 ] && home_card && home_front; then
    pass launch "3 launches: Home runs, one process, card mapped, in front"
    [ "$restarts" = 0 ] && info launch "SAM keeps the running process on a relaunch" || info launch "SAM restarted the process $restarts time(s) of 3 (the new one replaces the old: one card at a time)"
else
    fail launch "after 3 launches: running $(home_runs && echo yes || echo no), processes $(stub_count), card $(mapped_of $HOME_ID), front '$(fg)'"
fi
no_flutter_mark

# ---- a preload: the preload manager loads Home again whenever it is not running, with a reason that says preload; a card with a buffer is at
# the front in both compositors, so the stub makes its window and maps nothing until it is launched for real
if [ "$(elapsed)" -lt 150 ]; then
    pkill -x "$COMM" >/dev/null 2>&1
    wait_for 5 '[ "$(stub_count)" = 0 ]' >/dev/null 2>&1
    wait_for 8 'home_runs && [ "$(stub_count)" = 1 ]' >/dev/null 2>&1
    if [ "$(stub_count)" = 1 ]; then
        info preload "Home was started again by itself after the program was ended: $(tr '\0' ' ' < "$PROC/$(stub_pid)/cmdline" 2>/dev/null | cut -c1-160) (window mapped: $(mapped_of $HOME_ID))"
    else
        ls1 com.webos.applicationManager/launch "{\"id\":\"$HOME_ID\",\"preload\":\"full\"}" >/dev/null
        wait_for 6 '[ "$(stub_count)" = 1 ]' >/dev/null 2>&1
        info preload "SAM did not start Home by itself; a launch asking for a preload started $(stub_count) program(s): $(tr '\0' ' ' < "$PROC/$(stub_pid)/cmdline" 2>/dev/null | cut -c1-160) (window mapped: $(mapped_of $HOME_ID))"
    fi
    if [ "$(stub_count)" = 1 ] && [ "$(mapped_of $HOME_ID)" != true ]; then
        pass preload "a preloaded Home is made and not mapped: the screen stays with what is on it"
    elif [ "$(stub_count)" = 1 ]; then
        info preload "this start was mapped at once: no preload hint reached the program (the launch argument is in the line above)"
    else
        fail preload "no Home program after the preload launch"
    fi
    ls1 com.webos.applicationManager/launch "{\"id\":\"$HOME_ID\"}" >/dev/null
    if wait_for 8 'home_card && home_front'; then pass preload-launch "a launch for real after it: Home's card is mapped and in front (program pid $(stub_pid), $(stub_count) process(es))"; else fail preload-launch "a launch after the preload did not map Home: card $(mapped_of $HOME_ID), front '$(fg)', $(stub_count) process(es), SAM's reply to a launch of a running app is not a relaunch the program can see"; fi
else
    skip preload "the preload start (the time budget)"
fi
no_flutter_mark

# ---- an app over Home, and the ways back
if [ "$DO_APP" = 1 ] && [ "$(elapsed)" -lt 200 ]; then
    app_in_front() { [ "$(fg)" = "$APP" ] || [ "$(mapped_of $APP)" = true ]; }
    ls1 com.webos.applicationManager/launch "{\"id\":\"$APP\"}" >/dev/null
    if wait_for 20 'app_in_front' && sleep 2 && ! home_front; then
        pass app "$APP is in front over Home"
        sp=$(stub_pid)
        ctl remote 773 tap >/dev/null
        if wait_for 6 home_front; then pass home-key "the Home key brought Home back over $APP (stub pid $sp -> $(stub_pid))"; else fail home-key "the Home key did not bring Home back: front is '$(fg)' (SAM did not raise the stub on its relaunch: the compositor would have to raise Home's card itself when Home is asked for)"; fi
        ls1 com.webos.applicationManager/launch "{\"id\":\"$APP\"}" >/dev/null
        wait_for 15 'app_in_front' >/dev/null 2>&1; sleep 2
        ls1 com.webos.applicationManager/launch "{\"id\":\"$HOME_ID\"}" >/dev/null
        if wait_for 6 home_front; then pass launch-over-app "luna-send launch Home brought Home back over $APP"; else fail launch-over-app "launch Home did not bring Home back: front is '$(fg)'"; fi
        [ "$(stub_count)" = 1 ] && pass app-process "still one $COMM process after the app came and went" || fail app-process "$(stub_count) $COMM processes after the app"
        ls1 com.webos.applicationManager/closeByAppId "{\"id\":\"$APP\"}" >/dev/null
        sleep 2
        home_front || ls1 com.webos.applicationManager/launch "{\"id\":\"$HOME_ID\"}" >/dev/null
    else
        fail app "$APP did not come to the front over Home (front '$(fg)')"
        ls1 com.webos.applicationManager/closeByAppId "{\"id\":\"$APP\"}" >/dev/null
    fi
else
    skip app "the app over Home, the Home key and the launch over an app$([ "$DO_APP" = 1 ] && echo ' (the time budget)')"
fi
no_flutter_mark

# ---- the screensaver over Home (LumaTV's own, as tools/gate's shell check starts it)
if [ "$DO_SAVER" = 1 ] && [ "$(elapsed)" -lt 220 ]; then
    home_front || { ls1 com.webos.applicationManager/launch "{\"id\":\"$HOME_ID\"}" >/dev/null; sleep 2; }
    sp=$(stub_pid)
    saver() { [ "$(ctl shell | jbool saver)" = true ]; }
    ls1 com.webos.service.tvpower/power2/turnOnScreenSaver '{}' >/dev/null
    if wait_for 15 saver; then
        pass saver "the saver started over Home"
        ctl remote 106 tap >/dev/null
        wait_for 10 '! saver' >/dev/null 2>&1
        if ! saver && [ "$(stub_pid)" = "$sp" ] && home_card; then pass saver-end "a key ended it, the stub is the same process (pid $sp), Home's card is mapped"; else fail saver-end "saver $(ctl shell | jbool saver), stub pid $sp -> $(stub_pid), card $(mapped_of $HOME_ID)"; fi
    else
        fail saver "the saver did not start over Home in 15 s"
    fi
else
    skip saver "the screensaver over Home"
fi
no_flutter_mark

# ---- standby and wake: bootd launches Home at a wake. Skipped when LG's panel refresh may be due: waking cuts it short, and waiting it out
# (15 minutes of standby) is not what a four minute probe can do.
refresh_due() {
    a=$(ls1 com.webos.service.oledepl/getPixelRefresherInfoList '{}'); b=$(timeout 20 luna-send -n 1 -f luna://com.webos.service.panelcontroller/getPanelUsageTime '{"subscribe":false}' 2>/dev/null)
    python3 - "$a" "$b" <<'PY' 2>/dev/null
import json, sys
def first(t):
    i = t.find("{")
    return json.JSONDecoder().raw_decode(t[i:])[0]
def offrs(v):
    if isinstance(v, dict):
        if any(k.startswith("offrs") for k in v):
            return v
        v = list(v.values())
    if isinstance(v, list):
        for x in v:
            o = offrs(x)
            if o is not None:
                return o
try:
    o = offrs(first(sys.argv[1]))
    usage = float(first(sys.argv[2])["panelUsageTime"]) / 6.0
    print("due" if usage >= float(o["offrsLastTime"]) + float(o["offrsInterval"]) else "not-due")
except Exception:
    print("unknown")
PY
}
if [ "$DO_WAKE" = 1 ] && [ "$(elapsed)" -lt 170 ]; then
    due=$(refresh_due)
    if [ "$due" != not-due ]; then
        skip wake "LG's panel refresh is $due: a wake after 20 s of standby would cut it short (run again after it, or wait out 15 minutes of standby by hand)"
    else
        home_front || { ls1 com.webos.applicationManager/launch "{\"id\":\"$HOME_ID\"}" >/dev/null; sleep 2; }
        sp=$(stub_pid)
        ls1 com.webos.service.tvpower/power/powerOff '{"reason":"remoteKey"}' >/dev/null
        wait_for 20 '[ "$(power_state)" != active ]' >/dev/null 2>&1
        info wake "standby: power state '$(power_state)', stub pid $(stub_pid)"
        sleep 20
        ls1 com.webos.service.tvpower/power/powerOn '{"reason":"remoteKey"}' >/dev/null
        wait_for 40 '[ "$(power_state)" = active ]' >/dev/null 2>&1
        ls1 com.webos.service.tvpower/power/turnOnScreen '{}' >/dev/null
        wait_for 20 'home_runs && home_card' >/dev/null 2>&1
        info wake "after the wake: power $(power_state), front '$(fg)', stub pid $sp -> $(stub_pid)"
        landed=$(fg)
        # bootd's own launch is the wake path; what the stub has to show is that Home is alive and a launch brings it forward
        ls1 com.webos.applicationManager/launch "{\"id\":\"$HOME_ID\"}" >/dev/null
        wait_for 10 home_front >/dev/null 2>&1
        if [ "$(power_state)" = active ] && home_runs && [ "$(stub_count)" = 1 ] && home_card && home_front; then
            pass wake "Home is alive after standby and wake (the wake landed on '$landed'), one process, card mapped, in front after a launch"
        else
            fail wake "power $(power_state), Home runs $(home_runs && echo yes || echo no), $(stub_count) process(es), card $(mapped_of $HOME_ID), front '$(fg)'"
        fi
    fi
else
    skip wake "standby and wake$([ "$DO_WAKE" = 1 ] && echo ' (the time budget)')"
fi
no_flutter_mark

# ---- the end: the web stub back unless the owner asked to keep this one and everything that ran passed
snap native-end
if [ "$KEEP" = 1 ] && [ "$FAILED" = 0 ]; then
    KEPT=1
    info keep "left native in place: $("$LUMATV" homestub-kind status)"
fi
finish
