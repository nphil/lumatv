#!/bin/sh
# LG's Luna binding for node, repaired for the one process that hosts LG's unified JS services. POSIX sh / busybox ash.
#
#   sysbus.sh on [--restart]   build the repaired module, lay LG's launcher with the module first on node's search path, remember it
#   sysbus.sh off [--restart]  take the launcher back, delete what was staged, forget it
#   sysbus.sh early            boot step: lay it again when it was switched on; always exit 0, bounded, never restarts anything
#   sysbus.sh restart          end the unified server's node process so its next start uses the launcher as it is now
#   sysbus.sh status           JSON: the launcher, the module, the running process and whether it needs a restart
#
# Why: `ss.apiadapter` (the unified server, one node process hosting apiadapter, the second-screen gateway and the other `run-js-service -u`
# services) spent 23 % of a core on sockets whose peer had died: /usr/lib/nodejs/webos-sysbus.node polls them through libuv and never
# closes them (tools/sysbus_fix.py has the details and builds the repaired copy from the set's own files, so none of LG's files is in
# the repository).
#
# What it touches: /usr/bin/run-unified-service-server, and nothing else. The script is `sh`; with one argument it only asks the running
# server to load a service, and with none it exports NODE_PATH and runs node. The wrapper is the set's own script with a copy of
# `export NODE_PATH=` changed to put $SB/mod in front, so `require('palmbus')` resolves to the module's palmbus.js, which loads the
# repaired webos-sysbus.node beside it. Every other node host (Homebrew Channel's service, iotjs, the services without `-u`) keeps
# /usr/lib/nodejs untouched: /usr/lib/nodejs/webos-sysbus.node is never bound over. The one-argument branch is not changed.
#
# The wrapper is bound over the launcher (mount --bind, nothing written into /usr, which is a read-only overlay) only when
#   - the launcher is the one this was made for (sha256 pinned below; any other firmware: nothing is changed, LG's script stays),
#   - no mount is on the launcher yet (a mount that is not this tool's is never stacked on or removed),
#   - the module was built from the set's own files by tools/sysbus_fix.py and verified there.
# The wrapper looks for the module itself and puts nothing in front of NODE_PATH when it is missing, so a lost state directory leaves
# LG's search path. A reboot also puts LG's script back; `early` lays it again from the boot hook while `enabled` is there. The boot hook
# (about 13 s after boot) is not known to run before the unified server's first start: status `launcher` is what is bound and `loaded` is
# what the running server mapped, and only `loaded: patched` means the process is repaired. `off` deletes `enabled` before anything else.
#
# A running process keeps the module it mapped: a restart (`restart`, or `on|off --restart`) is what moves it. That ends exactly the
# node process that is a child of `/bin/sh /usr/bin/run-unified-service-server`; ls-hubd starts the server again on the next call, and
# the services it hosts come back on their own calls (the gateway when its activator next pings it; Home Assistant's sessions drop).
set -u

APPDIR=${APPDIR:-$(cd "$(dirname "$0")/.." && pwd)}
if [ -z "${STATE:-}" ]; then
  . "$APPDIR/tools/brand.sh"
  brand_load "$APPDIR/brand.json" 2>/dev/null || { printf '{"ok":false,"error":"brand.json is missing or invalid"}\n'; exit 1; }
  STATE="$BRAND_DATA_DIR"
fi

SB=${SYSBUS_DIR:-$STATE/sysbus}
MOD="$SB/mod"
WRAPPER="$SB/run-unified-service-server"
ORIG="$SB/run-unified-service-server.orig"
ENABLED="$SB/enabled"
RECORD="$SB/record.json"
LOCKDIR="$SB/.lock"
LOGFILE=${LOGFILE:-$STATE/log}

LAUNCHER=${SYSBUS_LAUNCHER:-/usr/bin/run-unified-service-server}
# The launcher of webOS 10.2.1 (1240 bytes); the module is pinned in tools/sysbus_fix.py.
LAUNCHER_SHA=${SYSBUS_LAUNCHER_SHA:-175c9e7806fdee116e8895c27fbac938adfd407077ce556e1ac994e4a4cf68b9}
SRC_DIR=${SYSBUS_SRC:-/usr/lib/nodejs}
NODE_EXE=${SYSBUS_NODE:-/usr/bin/node}
FIX_TOOL=${SYSBUS_FIX_TOOL:-$APPDIR/tools/sysbus_fix.py}
PYTHON=${PYTHON:-python3}
MOUNT=${MOUNT:-mount}
UMOUNT=${UMOUNT:-umount}
KILL=${KILL:-kill}
LUNA_SEND=${LUNA_SEND:-luna-send}
PROC_MOUNTS=${PROC_MOUNTS:-/proc/mounts}
PROC_ROOT=${PROC_ROOT:-/proc}
LOCK_WAIT=${SYSBUS_LOCK_WAIT:-30}
STOP_STEPS=${SYSBUS_STOP_STEPS:-20}
STOP_PAUSE=${SYSBUS_STOP_PAUSE:-0.5}
START_STEPS=${SYSBUS_START_STEPS:-20}
# apiadapter answers system info only from the unified server, which ls-hubd starts on that call.
PROBE_URI=${SYSBUS_PROBE_URI:-luna://com.webos.service.apiadapter/system/getSystemInfo}
CONFIG=${SYSBUS_CONFIG:-$STATE/config.json}

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n\r\t' '   '
}

emit_error() {
  printf '{"ok":false,"error":"%s"}\n' "$(json_escape "$1")"
}

log_line() {
  mkdir -p "$(dirname "$LOGFILE")" 2>/dev/null
  printf '%s sysbus: %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$(printf '%s' "$1" | tr '\n' ' ')" >> "$LOGFILE" 2>/dev/null
}

sha_of() {
  sha256sum "$1" 2>/dev/null | cut -d' ' -f1
}

# The module's directory goes into a generated shell line.
case "$MOD" in
  ''|*[!A-Za-z0-9/._-]*) emit_error "the state directory has characters a shell line cannot carry"; exit 1 ;;
esac

take_lock() {
  mkdir -p "$SB" 2>/dev/null || return 1
  tl_i=0
  while ! mkdir "$LOCKDIR" 2>/dev/null; do
    # A lock older than ten minutes belongs to a run that died.
    if [ -n "$(find "$LOCKDIR" -maxdepth 0 -mmin +10 2>/dev/null)" ]; then
      rmdir "$LOCKDIR" 2>/dev/null
      continue
    fi
    tl_i=$((tl_i + 1))
    [ "$tl_i" -gt "$LOCK_WAIT" ] && return 1
    sleep 1
  done
  trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT
  trap 'exit 1' HUP INT TERM
  return 0
}

mounts_on_launcher() {
  awk -v p="$LAUNCHER" '$2 == p { n++ } END { print n + 0 }' "$PROC_MOUNTS" 2>/dev/null
}

# stock: LG's script, nothing mounted. ours: exactly one mount and it shows the wrapper file itself (same device and inode: the same
# bytes on another inode are somebody else's mount). foreign: any other mount on the launcher. unknown: nothing mounted and not the
# script this was made for.
launcher_state() {
  ls_n=$(mounts_on_launcher)
  if [ "$ls_n" -gt 0 ]; then
    if [ "$ls_n" = 1 ] && [ -f "$WRAPPER" ] && [ "$LAUNCHER" -ef "$WRAPPER" ]; then
      echo ours
    else
      echo foreign
    fi
    return 0
  fi
  if [ -f "$LAUNCHER" ] && [ "$(sha_of "$LAUNCHER")" = "$LAUNCHER_SHA" ]; then
    echo stock
  else
    echo unknown
  fi
}

# LG's script with the one NODE_PATH line put behind a variable that is the module's directory when the module is there.
build_wrapper() {
  awk -v mod="$MOD" '
    BEGIN { key = "export NODE_PATH=" }
    {
      at = index($0, key)
      if (at > 0 && $0 ~ /^[ \t]*export NODE_PATH=/) {
        n++
        indent = substr($0, 1, at - 1)
        print indent "SYSBUS_MOD=" mod
        print indent "SYSBUS_PATH="
        print indent "[ -f \"$SYSBUS_MOD/palmbus.js\" ] && [ -f \"$SYSBUS_MOD/webos-sysbus.node\" ] && SYSBUS_PATH=$SYSBUS_MOD:"
        print substr($0, 1, at - 1 + length(key)) "${SYSBUS_PATH}" substr($0, at + length(key))
        next
      }
      print
    }
    END { exit n == 1 ? 0 : 1 }' "$1" > "$2"
}

# The repaired module for what the set has now, built beside the one in use and put in its place only when it differs. Prints
# nothing; the reason of a failure is in $sb_err.
stage_module() {
  rm -rf "$MOD.new"
  if ! sm_out=$("$PYTHON" "$FIX_TOOL" build "$SRC_DIR" "$MOD.new" 2>&1); then
    rm -rf "$MOD.new"
    sb_err="the module was not built: $sm_out"
    return 1
  fi
  if ! sm_out=$("$PYTHON" "$FIX_TOOL" verify "$MOD.new" 2>&1); then
    rm -rf "$MOD.new"
    sb_err="the built module does not verify: $sm_out"
    return 1
  fi
  if [ -d "$MOD" ] && "$PYTHON" "$FIX_TOOL" verify "$MOD" >/dev/null 2>&1 && cmp -s "$MOD/manifest.json" "$MOD.new/manifest.json"; then
    rm -rf "$MOD.new"
    return 0
  fi
  rm -rf "$MOD.old"
  [ -d "$MOD" ] && mv "$MOD" "$MOD.old"
  if ! mv "$MOD.new" "$MOD"; then
    [ -d "$MOD.old" ] && mv "$MOD.old" "$MOD"
    rm -rf "$MOD.new"
    sb_err="the module could not be put in place"
    return 1
  fi
  rm -rf "$MOD.old"
  return 0
}

# Takes back what a failed run laid. $1 = true when this run made the module directory. Only a mount that is proven ours is unmounted;
# the files go only once nothing is mounted over the launcher. Returns 1 when something is still mounted.
undo_bind() {
  [ "$(launcher_state)" = ours ] && "$UMOUNT" "$LAUNCHER" 2>/dev/null
  [ "$(mounts_on_launcher)" = 0 ] || return 1
  rm -f "$WRAPPER" "$ORIG"
  "$1" && rm -rf "$MOD"
  return 0
}

# Lays the wrapper. $1 = remember: write `enabled` once it stands. Sets $sb_result (bound, already) or $sb_err.
apply() {
  sb_err="" sb_result=""
  mkdir -p "$SB" 2>/dev/null || { sb_err="$SB cannot be created"; return 1; }
  ap_state=$(launcher_state)
  case "$ap_state" in
    foreign) sb_err="another mount is on $LAUNCHER; nothing was changed"; return 1 ;;
    unknown) sb_err="$LAUNCHER is not the script this was made for (sha256 $(sha_of "$LAUNCHER")); nothing was changed"; return 1 ;;
  esac
  ap_fresh=false
  [ -d "$MOD" ] || ap_fresh=true
  stage_module || return 1
  if [ "$ap_state" = stock ]; then
    ap_tmp_orig="$ORIG.new"
    ap_tmp_wrap="$WRAPPER.new"
    rm -f "$ap_tmp_orig" "$ap_tmp_wrap"
    if ! cp "$LAUNCHER" "$ap_tmp_orig" || ! build_wrapper "$ap_tmp_orig" "$ap_tmp_wrap" || ! chmod 755 "$ap_tmp_wrap" "$ap_tmp_orig" \
        || ! sh -n "$ap_tmp_wrap" 2>/dev/null; then
      rm -f "$ap_tmp_orig" "$ap_tmp_wrap"
      "$ap_fresh" && rm -rf "$MOD"
      sb_err="the launcher could not be turned into the wrapper"
      return 1
    fi
    mv "$ap_tmp_orig" "$ORIG" && mv "$ap_tmp_wrap" "$WRAPPER" || { rm -f "$ap_tmp_orig" "$ap_tmp_wrap"; sb_err="the wrapper could not be put in place"; return 1; }
    if ! "$MOUNT" --bind "$WRAPPER" "$LAUNCHER" 2>/dev/null || [ "$(launcher_state)" != ours ]; then
      sb_err="the wrapper could not be bound over $LAUNCHER"
      undo_bind "$ap_fresh" || sb_err="$sb_err, and a mount on it could not be taken back"
      return 1
    fi
    sb_result=bound
  else
    sb_result=already
  fi
  if [ "${1:-no}" = remember ]; then
    if ! { printf 'enabled\n' > "$ENABLED.tmp" && mv "$ENABLED.tmp" "$ENABLED"; }; then
      rm -f "$ENABLED.tmp"
      sb_err="$ENABLED could not be written"
      # A bind laid by this call goes with it; one that was already there stays as it was.
      [ "$sb_result" = bound ] && { undo_bind "$ap_fresh" || sb_err="$sb_err, and the bind could not be taken back"; }
      return 1
    fi
  fi
  printf '{"result":"%s","launcherSha256":"%s","moduleSha256":"%s","time":"%s"}\n' "$sb_result" "$LAUNCHER_SHA" \
    "$(sha_of "$MOD/webos-sysbus.node")" "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" > "$RECORD.tmp" && mv "$RECORD.tmp" "$RECORD"
  return 0
}

# ---------------------------------------------------------------- the process

# "pid:starttime" of node processes whose parent is exactly `/bin/sh <launcher>` (that argv and no other): the unified server. The server
# renames itself (ss.apiadapter) and its command line with it, so the parent is what identifies it. The start time (field 22 of
# /proc/PID/stat) tells a process from a later one that got the same pid.
server_procs() {
  for sp_d in "$PROC_ROOT"/[0-9]*; do
    [ -d "$sp_d" ] || continue
    [ "$(readlink "$sp_d/exe" 2>/dev/null)" = "$NODE_EXE" ] || continue
    sp_f=$(sed 's/^.*) //' "$sp_d/stat" 2>/dev/null | awk '{ print $2 " " $20 }')
    sp_pp=${sp_f% *}
    sp_start=${sp_f#* }
    case "$sp_pp$sp_start" in ''|*[!0-9]*) continue ;; esac
    [ "$(tr '\0' ' ' < "$PROC_ROOT/$sp_pp/cmdline" 2>/dev/null)" = "/bin/sh $LAUNCHER " ] || continue
    printf '%s:%s\n' "${sp_d##*/}" "$sp_start"
  done
}

proc_start() {
  sed 's/^.*) //' "$PROC_ROOT/$1/stat" 2>/dev/null | awk '{ print $20 }'
}

# Whether pid $1 is still the process that started at $2.
same_proc() {
  [ -n "$2" ] && [ "$(proc_start "$1")" = "$2" ]
}

# patched: only the repaired module is mapped; stock: only LG's; mixed: both in one process; unknown: neither (still starting, or unreadable).
server_module() {
  sm_p=false sm_s=false
  grep -qF "$MOD/webos-sysbus.node" "$PROC_ROOT/$1/maps" 2>/dev/null && sm_p=true
  grep -qF "$SRC_DIR/webos-sysbus.node" "$PROC_ROOT/$1/maps" 2>/dev/null && sm_s=true
  if [ "$sm_p" = true ] && [ "$sm_s" = true ]; then echo mixed
  elif [ "$sm_p" = true ]; then echo patched
  elif [ "$sm_s" = true ]; then echo stock
  else echo unknown
  fi
}

# TERM, then KILL, only while the pid is still the process that was pinned: a pid that has been reused is nobody's to signal.
stop_proc() {
  same_proc "$1" "$2" || return 0
  "$KILL" -TERM "$1" 2>/dev/null
  sx_i=0
  while same_proc "$1" "$2" && [ "$sx_i" -lt "$STOP_STEPS" ]; do
    sleep "$STOP_PAUSE"
    sx_i=$((sx_i + 1))
  done
  if same_proc "$1" "$2"; then
    log_line "pid $1 did not end on TERM; killed"
    "$KILL" -KILL "$1" 2>/dev/null
    sleep 1
  fi
  ! same_proc "$1" "$2"
}

# Ends the unified server's node process, asks apiadapter for system info (which starts the server again) and reads the new process:
# it fails unless the question is answered and a new process maps the module the launcher now implies. Prints JSON members into $sb_json.
restart_server() {
  rs_old=$(server_procs | tr '\n' ' ')
  rs_ended=""
  for rs_e in $rs_old; do
    if stop_proc "${rs_e%%:*}" "${rs_e#*:}"; then
      rs_ended="$rs_ended ${rs_e%%:*}"
    else
      sb_err="pid ${rs_e%%:*} did not end"
      return 1
    fi
  done
  [ -n "$rs_ended" ] && log_line "restart: ended pid$rs_ended"
  rs_note="ended [${rs_ended# }]"
  case "$(launcher_state)" in ours) rs_want=patched ;; stock) rs_want=stock ;; *) rs_want="" ;; esac
  rs_reply=$(timeout 15 "$LUNA_SEND" -n 1 "$PROBE_URI" '{}' </dev/null 2>/dev/null)
  if [ $? -ne 0 ] || ! printf '%s' "$rs_reply" | grep -q '"returnValue"[[:space:]]*:[[:space:]]*true'; then
    sb_err="$rs_note; $PROBE_URI did not answer, so there is no new server process to check"
    return 1
  fi
  rs_i=0
  while :; do
    rs_new="" rs_unsettled=false
    for rs_e in $(server_procs); do
      case " $rs_old " in *" $rs_e "*) continue ;; esac
      rs_new="$rs_new $rs_e"
      [ "$(server_module "${rs_e%%:*}")" = unknown ] && rs_unsettled=true
    done
    { [ -n "$rs_new" ] && [ "$rs_unsettled" = false ]; } && break
    rs_i=$((rs_i + 1))
    [ "$rs_i" -gt "$START_STEPS" ] && break
    sleep "$STOP_PAUSE"
  done
  [ -n "$rs_new" ] || { sb_err="$rs_note; no new unified server process appeared"; return 1; }
  rs_mod=""
  rs_pids=""
  for rs_e in $rs_new; do
    rs_m=$(server_module "${rs_e%%:*}")
    rs_mod="${rs_mod:-$rs_m}"
    [ "$rs_mod" = "$rs_m" ] || rs_mod=mixed
    rs_pids="$rs_pids ${rs_e%%:*}"
    case "$rs_m" in
      patched|stock) [ -z "$rs_want" ] || [ "$rs_m" = "$rs_want" ] || { sb_err="$rs_note; the new server pid ${rs_e%%:*} loaded the $rs_m module, expected $rs_want"; return 1; } ;;
      *) sb_err="$rs_note; the new server pid ${rs_e%%:*} shows module state $rs_m"; return 1 ;;
    esac
  done
  sb_json=$(printf '"ended":%s,"running":%s,"module":"%s"' "$(pid_list "$rs_ended")" "$(pid_list "$rs_pids")" "$rs_mod")
  return 0
}

pid_list() {
  pl_o=""
  for pl_p in $1; do pl_o="$pl_o${pl_o:+,}$pl_p"; done
  printf '[%s]' "$pl_o"
}

# ---------------------------------------------------------------- verbs

verb_on() {
  take_lock || { emit_error "another sysbus run is in progress"; return 1; }
  if ! apply remember; then
    log_line "on refused: $sb_err"
    emit_error "$sb_err"
    return 1
  fi
  log_line "on: $sb_result"
  von_restart=""
  if [ "${1:-}" = --restart ]; then
    restart_server || { emit_error "applied, but $sb_err"; return 1; }
    von_restart=",\"restart\":{$sb_json}"
  fi
  printf '{"ok":true,"sysbus":"%s"%s}\n' "$sb_result" "$von_restart"
}

# Nothing staged, nothing mounted, nobody working: there is nothing for `off` to do and it leaves no trace.
off_idle() {
  [ ! -d "$LOCKDIR" ] && [ "$(mounts_on_launcher)" = 0 ] || return 1
  for oi_f in "$ENABLED" "$ENABLED.tmp" "$RECORD" "$RECORD.tmp" "$WRAPPER" "$WRAPPER.new" "$ORIG" "$ORIG.new" "$MOD" "$MOD.new" "$MOD.old"; do
    [ -e "$oi_f" ] && return 1
  done
  return 0
}

verb_off() {
  if [ "${1:-}" != --restart ] && off_idle; then
    printf '{"ok":true,"sysbus":"off","was":"%s"}\n' "$(launcher_state)"
    return 0
  fi
  take_lock || { emit_error "another sysbus run is in progress; off was not applied"; return 1; }
  # With the lock held no `on` or boot step can write the marker back; it goes before anything that can fail, so a boot does not lay
  # the wrapper again whatever else happens.
  rm -f "$ENABLED" "$ENABLED.tmp"
  vo_state=$(launcher_state)
  case "$vo_state" in
    foreign) log_line "off refused: a mount that is not ours is on the launcher"; emit_error "another mount is on $LAUNCHER and was left alone; the boot marker is removed"; return 1 ;;
    ours)
      "$UMOUNT" "$LAUNCHER" 2>/dev/null
      if [ "$(mounts_on_launcher)" != 0 ]; then
        log_line "off: the wrapper could not be unmounted"
        emit_error "the wrapper could not be unmounted; the boot marker is removed and the wrapper stays until the next boot"
        return 1
      fi ;;
  esac
  rm -f "$RECORD" "$RECORD.tmp" "$WRAPPER" "$ORIG" "$WRAPPER.new" "$ORIG.new"
  rm -rf "$MOD" "$MOD.new" "$MOD.old"
  log_line "off: $vo_state"
  vo_restart=""
  if [ "${1:-}" = --restart ]; then
    restart_server || { emit_error "taken off, but $sb_err"; return 1; }
    vo_restart=",\"restart\":{$sb_json}"
  fi
  printf '{"ok":true,"sysbus":"off","was":"%s"%s}\n' "$vo_state" "$vo_restart"
}

verb_restart() {
  take_lock || { emit_error "another sysbus run is in progress"; return 1; }
  restart_server || { emit_error "$sb_err"; return 1; }
  printf '{"ok":true,"restart":{%s}}\n' "$sb_json"
}

# The boot hook's step. Always exits 0: a failure is a line in the log and a record, never a held-up boot.
verb_early() {
  if [ ! -f "$ENABLED" ]; then
    printf '{"ok":true,"applied":false,"why":"not switched on"}\n'
    return 0
  fi
  LOCK_WAIT=10
  if ! take_lock; then
    log_line "early: another run holds the lock"
    printf '{"ok":true,"applied":false,"why":"another run is in progress"}\n'
    return 0
  fi
  if apply no; then
    log_line "early: $sb_result at uptime $(cut -d. -f1 /proc/uptime 2>/dev/null) s"
    printf '{"ok":true,"applied":true,"sysbus":"%s"}\n' "$sb_result"
  else
    log_line "early: not applied ($sb_err)"
    printf '{"ok":true,"applied":false,"why":"%s"}\n' "$(json_escape "$sb_err")"
  fi
  return 0
}

verb_status() {
  st_state=$(launcher_state)
  st_enabled=false
  [ -f "$ENABLED" ] && st_enabled=true
  st_mod=false
  [ -d "$MOD" ] && "$PYTHON" "$FIX_TOOL" verify "$MOD" >/dev/null 2>&1 && st_mod=true
  st_procs=""
  st_need=false
  st_loaded=none
  for st_e in $(server_procs); do
    st_p=${st_e%%:*}
    st_m=$(server_module "$st_p")
    st_procs="$st_procs${st_procs:+,}{\"pid\":$st_p,\"module\":\"$st_m\"}"
    if [ "$st_loaded" = none ]; then st_loaded=$st_m; elif [ "$st_loaded" != "$st_m" ]; then st_loaded=mixed; fi
    case "$st_m" in
      mixed|unknown) st_need=true ;;
      stock) [ "$st_state" = ours ] && st_need=true ;;
      patched) [ "$st_state" != ours ] && st_need=true ;;
    esac
  done
  st_persist=false
  grep -q '"persist"[[:space:]]*:[[:space:]]*true' "$CONFIG" 2>/dev/null && st_persist=true
  # enabled/launcher say what is configured and bound; loaded says what the running server mapped (none: no server process).
  printf '{"ok":true,"enabled":%s,"launcher":"%s","moduleOk":%s,"bootHook":%s,"loaded":"%s","processes":[%s],"restartNeeded":%s}\n' \
    "$st_enabled" "$st_state" "$st_mod" "$st_persist" "$st_loaded" "$st_procs" "$st_need"
}

case "${1:-status}" in
  on) verb_on "${2:-}" ;;
  off) verb_off "${2:-}" ;;
  early) verb_early ;;
  restart) verb_restart ;;
  status) verb_status ;;
  *) emit_error "usage: sysbus.sh on|off [--restart] | early | restart | status"; exit 1 ;;
esac
