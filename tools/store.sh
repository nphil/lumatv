#!/bin/sh
# LG's Content Store (com.webos.app.discovery) in the Lucent look: one script, WAM's user script for the app, laid into LG's app
# directory by an overlay mount. POSIX sh / busybox ash. Nothing is written into the root file system or into LG's directory: the
# script and the overlay's upper layer live under $STATE, the only thing that touches LG's directory is a mount over it, and `off`
# (or a reboot) puts LG's back exactly.
#
#   store.sh on [parts]     unmount, build the script, mount it over LG's directory, end the store's running instance (it comes back
#                           on the next launch with the theme); parts are look, motion, canvas and lazy, commas or spaces ("" = look,motion)
#   store.sh off            unmount, delete what was staged, end the running instance (it comes back as LG's), end the probe's receiver
#   store.sh probe [parts]  the same as `on`, with the probe part: the page posts what it is made of to a receiver this starts on a loopback
#                           port (tools/store_probe.py), which writes it under $STATE/store-probe. With no parts the page is LG's as it is;
#                           with parts, the theme and the probe together. Never persisted: a reboot ends it, so does `on` or `off`
#   store.sh stage [parts]  only the staging (the upper layer under $STATE/store/upper), no mount: what the host tests run
#   store.sh status         JSON: where LG's directory is, whether the script is staged and mounted, the parts, whether the store runs, the probe
#
# Each verb prints one JSON object. A build that fails leaves LG's store in force: nothing is mounted from a half-built script.
#
# Where the app is: the store is a web app of the firmware's `otncabi` partition, a read-only squashfs mounted at /mnt/otncabi
# (appinfo folderPath /mnt/otncabi/usr/palm/applications/com.webos.app.discovery), not of /usr/palm/applications where the settings
# apps are, so the directory is what SAM says it is. The overlay's lower layer is that directory itself, so a firmware update's new
# copy is what shows through, and the upper layer holds webOSUserScripts/userScript.js and nothing else.
#
# What the script is and why it is the only file: the store's loader (index.html, js/Loader.js) sends the view to LG's cloud page, which
# a stylesheet beside the page cannot reach; WAM runs webOSUserScripts/userScript.js in every document the view loads, the cloud
# pages included (tools/store_theme.py builds it, docs/research/store-result.md says why). Presentation only, and the probe's reading.
#
# The store is not a keep-alive app (configd's keepAliveApps), so closeByAppId ends it; WAM's killApp is the second ask for a page
# that is still listed after the close. Neither restarts SAM or WAM.
set -u

APPDIR=${APPDIR:-$(cd "$(dirname "$0")/.." && pwd)}
if [ -z "${STATE:-}" ]; then
  . "$APPDIR/tools/brand.sh"
  brand_load "$APPDIR/brand.json" 2>/dev/null || { printf '{"ok":false,"error":"brand.json is missing or invalid"}\n'; exit 1; }
  STATE="$BRAND_DATA_DIR"
fi

mkdir -p "$STATE" 2>/dev/null
STORE_ID=${STORE_ID:-com.webos.app.discovery}
STORE_STAGE="$STATE/store"
STORE_UPPER="$STORE_STAGE/upper"
STORE_WORK="$STORE_STAGE/work"
STORE_PARTS_FILE="$STORE_STAGE/parts"
STORE_SRC=${STORE_SRC:-$APPDIR/payload/store}
STORE_BUILD_TOOL=${STORE_BUILD_TOOL:-$APPDIR/tools/store_theme.py}
STORE_PROBE_TOOL=${STORE_PROBE_TOOL:-$APPDIR/tools/store_probe.py}
# The probe's receiver: what it wrote, who it is, where it listens. Outside $STORE_STAGE, which `off` deletes: the files are what the
# round is for, and the last session's are kept (as store-probe.prev) when the next one starts.
PROBE_DIR=${PROBE_DIR:-$STATE/store-probe}
PROBE_PID="$STATE/.store-probe.pid"
PROBE_PORT_FILE="$STATE/.store-probe.port"
PROBE_PORT=${STORE_PROBE_PORT:-18742}
PROBE_SECONDS=${STORE_PROBE_SECONDS:-1800}
PROBE_STEP=${PROBE_STEP:-1}
THEME_SRC=${THEME_SRC:-$APPDIR/payload/theme}
THEME_DIR=${THEME_DIR:-$STATE/theme}
LOGFILE=${LOGFILE:-$STATE/log}
LUNA_SEND=${LUNA_SEND:-luna-send}
SAM_TIMEOUT=${SAM_TIMEOUT:-10}
SAM_URI=luna://com.webos.applicationManager
WAM_URI=luna://com.palm.webappmanager
MOUNT=${MOUNT:-mount}
UMOUNT=${UMOUNT:-umount}
PROC_MOUNTS=${PROC_MOUNTS:-/proc/mounts}
PROC_ROOT=${PROC_ROOT:-/proc}
STORE_STEP=${STORE_STEP:-1}
STORE_CLOSE_STEPS=${STORE_CLOSE_STEPS:-3}
STORE_CANDIDATES=${STORE_CANDIDATES:-"/mnt/otncabi/usr/palm/applications/$STORE_ID /usr/palm/applications/$STORE_ID"}

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n\r\t' '   '
}

emit_error() {
  printf '{"ok":false,"error":"%s"}\n' "$(json_escape "$1")"
}

log_line() {
  mkdir -p "$STATE" 2>/dev/null
  printf '%s store: %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$(printf '%s' "$1" | tr '\n' ' ')" >> "$LOGFILE" 2>/dev/null
}

store_sam() {
  timeout "$SAM_TIMEOUT" $LUNA_SEND -n 1 "$SAM_URI/$1" "$2" 2>/dev/null | tr -d ' \r\n'
}

store_wam() {
  timeout "$SAM_TIMEOUT" $LUNA_SEND -n 1 "$WAM_URI/$1" "$2" 2>/dev/null | tr -d ' \r\n'
}

# A directory is the store's when it is called so, holds the app's appinfo and names the store's id in it. The override is for the host tests.
store_ok_dir() {
  [ -f "$1/appinfo.json" ] || return 1
  case "$1" in */usr/palm/applications/"$STORE_ID") ;; *) [ -n "${STORE_DIR_OVERRIDE:-}" ] && [ "$1" = "$STORE_DIR_OVERRIDE" ] || return 1 ;; esac
  grep -q "\"$STORE_ID\"" "$1/appinfo.json" 2>/dev/null
}

# LG's directory of the app: what SAM says (folderPath), else the first known home that holds it.
store_dir() {
  if [ -n "${STORE_DIR_OVERRIDE:-}" ]; then
    store_ok_dir "$STORE_DIR_OVERRIDE" && printf '%s\n' "$STORE_DIR_OVERRIDE"
    return 0
  fi
  sd=$(store_sam getAppInfo "{\"id\":\"$STORE_ID\"}" | sed -n 's/.*"folderPath":"\([^"]*\)".*/\1/p')
  if [ -n "$sd" ] && store_ok_dir "$sd"; then printf '%s\n' "$sd"; return 0; fi
  for sd in $STORE_CANDIDATES; do
    if store_ok_dir "$sd"; then printf '%s\n' "$sd"; return 0; fi
  done
  return 0
}

store_mounted() {
  grep -q " $1 overlay " "$PROC_MOUNTS" 2>/dev/null
}

# Unmounts every layer of ours over $1 (a remount stacks, so the loop is not for show).
store_umount() {
  n=0
  while store_mounted "$1" && [ "$n" -lt 8 ]; do
    $UMOUNT "$1" >/dev/null 2>&1 || break
    n=$((n+1))
  done
  store_mounted "$1" && return 1
  return 0
}

# The script, built into a new upper layer that replaces the old one only when it is whole. $2 = the probe's port, when the parts have the probe.
store_stage() {
  parts="$1"
  rm -rf "$STORE_STAGE/upper.new" 2>/dev/null
  mkdir -p "$STORE_STAGE/upper.new/webOSUserScripts" 2>/dev/null || return 1
  if [ -n "${2:-}" ]; then
    STORE_BUILD=$(python3 "$STORE_BUILD_TOOL" build "$STORE_SRC" "$THEME_SRC/theme.json" "$THEME_DIR/theme.json" \
      "$STORE_STAGE/upper.new/webOSUserScripts/userScript.js" --parts "$parts" --probe-port "$2" < /dev/null 2>>"$LOGFILE")
  else
    STORE_BUILD=$(python3 "$STORE_BUILD_TOOL" build "$STORE_SRC" "$THEME_SRC/theme.json" "$THEME_DIR/theme.json" \
      "$STORE_STAGE/upper.new/webOSUserScripts/userScript.js" --parts "$parts" < /dev/null 2>>"$LOGFILE")
  fi
  # shellcheck disable=SC2181
  [ $? -eq 0 ] || {
    log_line "the build failed: $STORE_BUILD"
    rm -rf "$STORE_STAGE/upper.new"
    return 1
  }
  [ -s "$STORE_STAGE/upper.new/webOSUserScripts/userScript.js" ] || { rm -rf "$STORE_STAGE/upper.new"; return 1; }
  # WAM reads the file as its own user.
  chmod 755 "$STORE_STAGE/upper.new" "$STORE_STAGE/upper.new/webOSUserScripts" 2>/dev/null
  chmod 644 "$STORE_STAGE/upper.new/webOSUserScripts/userScript.js" 2>/dev/null
  rm -rf "$STORE_STAGE/upper.old" 2>/dev/null
  [ -d "$STORE_UPPER" ] && mv "$STORE_UPPER" "$STORE_STAGE/upper.old" 2>/dev/null
  mv "$STORE_STAGE/upper.new" "$STORE_UPPER" 2>/dev/null || return 1
  rm -rf "$STORE_STAGE/upper.old" 2>/dev/null
  printf '%s\n' "$parts" > "$STORE_PARTS_FILE"
  return 0
}

store_mount() {
  # overlayfs wants an empty work directory on the upper layer's file system.
  rm -rf "$STORE_WORK" 2>/dev/null
  mkdir -p "$STORE_WORK" 2>/dev/null || return 1
  $MOUNT -t overlay lumatv-store -o "lowerdir=$1,upperdir=$STORE_UPPER,workdir=$STORE_WORK" "$1" >>"$LOGFILE" 2>&1
}

store_running() {
  store_sam running '{}' | grep -q "\"id\":\"$STORE_ID\""
}

# A running instance holds the page it loaded: end it, so that the next launch loads what is mounted now.
store_close() {
  store_running || { STORE_CLOSED=false; return 0; }
  store_sam closeByAppId "{\"id\":\"$STORE_ID\"}" >/dev/null
  w=0
  while store_running && [ "$w" -lt "$STORE_CLOSE_STEPS" ]; do
    sleep "$STORE_STEP"
    w=$((w+1))
  done
  if store_running; then store_wam killApp "{\"appId\":\"$STORE_ID\"}" >/dev/null; fi
  STORE_CLOSED=true
  return 0
}

store_parts() {
  python3 "$STORE_BUILD_TOOL" parts "${1:-}" < /dev/null 2>/dev/null
}

# ---- the probe's receiver (tools/store_probe.py): a process of ours on a loopback port, found by its pid file and by what it runs

probe_pid() {
  pp=""
  [ -f "$PROBE_PID" ] && pp=$(sed -n 1p "$PROBE_PID" 2>/dev/null)
  case "$pp" in ''|*[!0-9]*) return 1 ;; esac
  [ -r "$PROC_ROOT/$pp/cmdline" ] && tr '\0' ' ' < "$PROC_ROOT/$pp/cmdline" 2>/dev/null | grep -q 'store_probe.py' || return 1
  printf '%s\n' "$pp"
}

probe_stop() {
  pid=$(probe_pid) || { rm -f "$PROBE_PID" "$PROBE_PORT_FILE" 2>/dev/null; return 0; }
  kill "$pid" 2>/dev/null
  w=0
  while [ -r "$PROC_ROOT/$pid/cmdline" ] && [ "$w" -lt 20 ]; do sleep "$PROBE_STEP"; w=$((w+1)); done
  [ -r "$PROC_ROOT/$pid/cmdline" ] && kill -9 "$pid" 2>/dev/null
  rm -f "$PROBE_PID" "$PROBE_PORT_FILE" 2>/dev/null
  log_line "probe: receiver $pid ended"
  return 0
}

# Starts the receiver on PROBE_PORT (0 = any free one), waits for it to say it is listening (its port file), and leaves the port it bound in
# $PROBE_BOUND. The last session's files are kept as store-probe.prev.
probe_start() {
  probe_stop
  rm -rf "$PROBE_DIR.prev" 2>/dev/null
  [ -d "$PROBE_DIR" ] && mv "$PROBE_DIR" "$PROBE_DIR.prev" 2>/dev/null
  mkdir -p "$PROBE_DIR" 2>/dev/null || return 1
  # Its own session, and without the descriptors of lumatv's locks (8 and 9): a child that kept the mutation lock would hold every apply, revert
  # and `store off` out for as long as it ran.
  setsid nohup python3 "$STORE_PROBE_TOOL" serve --dir "$PROBE_DIR" --port "$PROBE_PORT" --seconds "$PROBE_SECONDS" \
    --pidfile "$PROBE_PID" --portfile "$PROBE_PORT_FILE" 8>&- 9>&- >>"$PROBE_DIR/receiver.out" 2>&1 < /dev/null &
  w=0
  while [ ! -s "$PROBE_PORT_FILE" ] && [ "$w" -lt 20 ]; do sleep "$PROBE_STEP"; w=$((w+1)); done
  [ -s "$PROBE_PORT_FILE" ] || { log_line "probe: the receiver did not come up: $(tail -n 2 "$PROBE_DIR/receiver.out" 2>/dev/null | tr '\n' ' ')"; return 1; }
  PROBE_BOUND=$(sed -n 1p "$PROBE_PORT_FILE")
  log_line "probe: receiver on 127.0.0.1:$PROBE_BOUND for $PROBE_SECONDS s, files in $PROBE_DIR"
  return 0
}

probe_json() {
  running=false; port=0
  if probe_pid >/dev/null 2>&1; then running=true; port=$(sed -n 1p "$PROBE_PORT_FILE" 2>/dev/null); fi
  case "$port" in ''|*[!0-9]*) port=0 ;; esac
  files=0
  [ -d "$PROBE_DIR" ] && files=$(ls "$PROBE_DIR" 2>/dev/null | grep -c '^[0-9][0-9]*-')
  printf '{"running":%s,"port":%s,"files":%s,"dir":"%s"}' "$running" "$port" "$files" "$(json_escape "$PROBE_DIR")"
}

verb_stage() {
  parts=$(store_parts "${1:-}") || { emit_error "unknown part: ${1:-} (parts: look, motion, canvas, lazy)"; return 1; }
  case ",$parts," in *,probe,*) emit_error "the probe part is for \`probe\`: it needs the receiver's port"; return 1 ;; esac
  if store_stage "$parts"; then
    printf '{"ok":true,"staged":true,"parts":"%s","build":%s}\n' "$parts" "${STORE_BUILD:-null}"
  else
    emit_error "could not build the theme; LG's store is in place"
    return 1
  fi
}

verb_on() {
  parts=$(store_parts "${1:-}") || { emit_error "unknown part: ${1:-} (parts: look, motion, canvas, lazy)"; return 1; }
  case ",$parts," in *,probe,*) emit_error "the probe part is for \`probe\`: it needs the receiver's port"; return 1 ;; esac
  dir=$(store_dir)
  [ -n "$dir" ] || { emit_error "LG's $STORE_ID directory was not found"; return 1; }
  probe_stop
  store_umount "$dir" || { emit_error "could not unmount the overlay over $dir"; return 1; }
  if ! store_stage "$parts"; then
    emit_error "could not build the theme; LG's store is in place"
    return 1
  fi
  if ! store_mount "$dir" || ! store_mounted "$dir" || [ ! -s "$dir/webOSUserScripts/userScript.js" ]; then
    store_umount "$dir"
    emit_error "could not mount the theme over $dir; LG's store is in place"
    return 1
  fi
  store_close
  log_line "on ($parts): mounted over $dir, closed the running instance: ${STORE_CLOSED:-false}"
  printf '{"ok":true,"store":"on","dir":"%s","parts":"%s","closed":%s,"build":%s}\n' "$(json_escape "$dir")" "$parts" "${STORE_CLOSED:-false}" "${STORE_BUILD:-null}"
}

verb_probe() {
  if [ -n "${1:-}" ]; then
    parts=$(store_parts "$1 probe") || { emit_error "unknown part: $1 (parts: look, motion, canvas, lazy)"; return 1; }
  else
    parts=probe
  fi
  dir=$(store_dir)
  [ -n "$dir" ] || { emit_error "LG's $STORE_ID directory was not found"; return 1; }
  store_umount "$dir" || { emit_error "could not unmount the overlay over $dir"; return 1; }
  probe_start || { emit_error "could not start the probe's receiver; LG's store is in place"; return 1; }
  if ! store_stage "$parts" "$PROBE_BOUND"; then
    probe_stop
    emit_error "could not build the theme; LG's store is in place"
    return 1
  fi
  if ! store_mount "$dir" || ! store_mounted "$dir" || [ ! -s "$dir/webOSUserScripts/userScript.js" ]; then
    store_umount "$dir"
    probe_stop
    emit_error "could not mount the theme over $dir; LG's store is in place"
    return 1
  fi
  store_close
  log_line "probe ($parts): mounted over $dir, receiver on $PROBE_BOUND, closed the running instance: ${STORE_CLOSED:-false}"
  printf '{"ok":true,"store":"probe","dir":"%s","parts":"%s","closed":%s,"probe":%s}\n' "$(json_escape "$dir")" "$parts" "${STORE_CLOSED:-false}" "$(probe_json)"
}

verb_off() {
  dir=$(store_dir)
  was=false
  probe_stop
  if [ -n "$dir" ]; then
    store_mounted "$dir" && was=true
    store_umount "$dir" || { emit_error "could not unmount the overlay over $dir"; return 1; }
  fi
  rm -rf "$STORE_STAGE" 2>/dev/null
  STORE_CLOSED=false
  # Only when there was something to undo: a set that never used the switch loses no running store to `off`.
  [ "$was" = true ] && store_close
  log_line "off: unmounted from ${dir:-nowhere}, was mounted: $was, closed the running instance: ${STORE_CLOSED:-false}"
  printf '{"ok":true,"store":"off","wasMounted":%s,"closed":%s}\n' "$was" "${STORE_CLOSED:-false}"
}

verb_status() {
  dir=$(store_dir)
  parts=""
  [ -f "$STORE_PARTS_FILE" ] && parts=$(sed -n 1p "$STORE_PARTS_FILE" 2>/dev/null)
  printf '{"ok":true,"dir":"%s","staged":%s,"mounted":%s,"parts":"%s","running":%s,"probe":%s}\n' "$(json_escape "${dir:-}")" \
    "$( [ -s "$STORE_UPPER/webOSUserScripts/userScript.js" ] && echo true || echo false )" \
    "$( [ -n "$dir" ] && store_mounted "$dir" && echo true || echo false )" \
    "$(json_escape "$parts")" \
    "$( store_running && echo true || echo false )" \
    "$(probe_json)"
}

case "${1:-status}" in
  on) verb_on "${2:-}" ;;
  off) verb_off ;;
  probe) verb_probe "${2:-}" ;;
  stage) verb_stage "${2:-}" ;;
  status) verb_status ;;
  *) emit_error "usage: store.sh on [parts]|off|probe [parts]|stage [parts]|status"; exit 1 ;;
esac
