#!/bin/sh
# LG's Content Store (com.webos.app.discovery) in the Lucent look: one script, WAM's user script for the app, laid into LG's app
# directory by an overlay mount. POSIX sh / busybox ash. Nothing is written into the root file system or into LG's directory: the
# script and the overlay's upper layer live under $STATE, the only thing that touches LG's directory is a mount over it, and `off`
# (or a reboot) puts LG's back exactly.
#
#   store.sh on [parts]    unmount, build the script, mount it over LG's directory, end the store's running instance (it comes back
#                          on the next launch with the theme); parts are look, motion and lazy, commas or spaces ("" = look,motion)
#   store.sh off           unmount, delete what was staged, end the running instance (it comes back as LG's)
#   store.sh stage [parts] only the staging (the upper layer under $STATE/store/upper), no mount: what the host tests run
#   store.sh status        JSON: where LG's directory is, whether the script is staged and mounted, the parts, whether the store runs
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
# pages included (tools/store_theme.py builds it, docs/research/store-result.md says why). Presentation only.
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

# The script, built into a new upper layer that replaces the old one only when it is whole.
store_stage() {
  parts="$1"
  rm -rf "$STORE_STAGE/upper.new" 2>/dev/null
  mkdir -p "$STORE_STAGE/upper.new/webOSUserScripts" 2>/dev/null || return 1
  STORE_BUILD=$(python3 "$STORE_BUILD_TOOL" build "$STORE_SRC" "$THEME_SRC/theme.json" "$THEME_DIR/theme.json" \
    "$STORE_STAGE/upper.new/webOSUserScripts/userScript.js" --parts "$parts" < /dev/null 2>>"$LOGFILE") || {
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

verb_stage() {
  parts=$(store_parts "${1:-}") || { emit_error "unknown part: ${1:-} (parts: look, motion, lazy)"; return 1; }
  if store_stage "$parts"; then
    printf '{"ok":true,"staged":true,"parts":"%s","build":%s}\n' "$parts" "${STORE_BUILD:-null}"
  else
    emit_error "could not build the theme; LG's store is in place"
    return 1
  fi
}

verb_on() {
  parts=$(store_parts "${1:-}") || { emit_error "unknown part: ${1:-} (parts: look, motion, lazy)"; return 1; }
  dir=$(store_dir)
  [ -n "$dir" ] || { emit_error "LG's $STORE_ID directory was not found"; return 1; }
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

verb_off() {
  dir=$(store_dir)
  was=false
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
  printf '{"ok":true,"dir":"%s","staged":%s,"mounted":%s,"parts":"%s","running":%s}\n' "$(json_escape "${dir:-}")" \
    "$( [ -s "$STORE_UPPER/webOSUserScripts/userScript.js" ] && echo true || echo false )" \
    "$( [ -n "$dir" ] && store_mounted "$dir" && echo true || echo false )" \
    "$(json_escape "$parts")" \
    "$( store_running && echo true || echo false )"
}

case "${1:-status}" in
  on) verb_on "${2:-}" ;;
  off) verb_off ;;
  stage) verb_stage "${2:-}" ;;
  status) verb_status ;;
  *) emit_error "usage: store.sh on [parts]|off|stage [parts]|status"; exit 1 ;;
esac
