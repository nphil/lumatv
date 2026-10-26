#!/bin/sh
# LG's on-screen keyboard keys (maliit, `MaliitServer`) in the Luma look: a patched copy of the keyboard's QML, laid over LG's
# directory by an overlay mount, and the keyboard's server restarted so that it reads it. POSIX sh / busybox ash. Nothing is written into
# the root file system: the copy and the overlay's upper layer live under $STATE, the only thing that touches LG's directory is a mount
# over it, and `off` (or a reboot) puts LG's back.
#
#   maliit.sh on       unmount, stage a fresh patched copy, mount it over LG's directory, restart maliit-server, check that it came back
#   maliit.sh off      unmount, restart maliit-server: it reads LG's QML again
#   maliit.sh stage    only the staging (the upper layer under $STATE/maliit/upper), no mount: what the host tests run
#   maliit.sh status   JSON: where LG's QML is, whether the copy is staged and mounted, what the unit is doing
#
# Each verb prints one JSON object. A stage that fails (tools/maliit_patch.py refusing an anchor LG changed, no fonts, no QML directory)
# leaves LG's tree in force and the previous stage as it was: nothing is mounted from a half-patched copy, nothing is restarted.
#
# How the server takes the QML up: MaliitServer loads the plugins' QML once, when it starts (the keyboard's main.qml runs as long as the
# server does: its own log lines, `root.foregroundAppId <app>`, come at every change of the foreground app, in /var/log/messages, with
# no keyboard on screen), and the files are read from disk (view-*/main.qml import `../keyboard-common`, so they are not a compiled
# resource; only the pictures, `qrc:///images-2k/`, are). A mount changes nothing for a server that is already running, so it is
# restarted the way LG restarts it itself: lsm-ready.service stops and starts maliit-server.service every time the compositor comes up
# (the census copy of the unit). `systemctl try-restart` of that one unit and nothing else: the server keeps no state the apps need (a
# text field asks again), no app is closed, and neither SAM nor any other unit is touched. A unit that is not running is left not
# running (under LumaTV's own compositor the census found it inactive): the server reads the mounted QML when it next starts.
#
# A server that does not come back, or that logs a QML file it could not load (a type that is unavailable, a syntax error), takes the
# mount away again and is brought up on LG's QML: a keyboard that cannot open is the one fault of this patch that leaves a TV without a way
# to type. [INFERENCE] where the QML is: /usr/lib/maliit/plugins/qml (copied from the set in round 11, 85 files); MALIIT_QML overrides.
set -u

APPDIR=${APPDIR:-$(cd "$(dirname "$0")/.." && pwd)}
if [ -z "${STATE:-}" ]; then
  . "$APPDIR/tools/brand.sh"
  brand_load "$APPDIR/brand.json" 2>/dev/null || { printf '{"ok":false,"error":"brand.json is missing or invalid"}\n'; exit 1; }
  STATE="$BRAND_DATA_DIR"
fi

mkdir -p "$STATE" 2>/dev/null
MT_NAME=maliit
MT_STAGE="$STATE/$MT_NAME"
MT_UPPER="$MT_STAGE/upper"
MT_WORK="$MT_STAGE/work"
PATCH_TOOL=${MALIIT_PATCH_TOOL:-$APPDIR/tools/maliit_patch.py}
TOKENS_TOOL="$APPDIR/tools/tokens.py"
THEME_SRC=${THEME_SRC:-$APPDIR/payload/theme}
THEME_DIR=${THEME_DIR:-$STATE/theme}
FONTS_DIR=${FONTS_DIR:-$STATE/fonts}
LOGFILE=${LOGFILE:-$STATE/log}
PROC_MOUNTS=${PROC_MOUNTS:-/proc/mounts}
MOUNT=${MOUNT:-mount}
UMOUNT=${UMOUNT:-umount}
SYSTEMCTL=${SYSTEMCTL:-systemctl}
MT_UNIT=${MALIIT_UNIT:-maliit-server.service}
# The system log MaliitServer writes to (PmLog), read from the byte where the restart began; seconds the server is watched for after it,
# and the length of a second (a test sets 0).
MT_MESSAGES=${MALIIT_MESSAGES:-/var/log/messages}
MT_SETTLE=${MALIIT_SETTLE:-5}
MT_STEP=${MALIIT_STEP:-1}
# What the QML engine says of a file it could not load; LG's own warnings about values (`Unable to assign [undefined] to int`, a handler's
# ReferenceError) are in the stock keyboard's log too and are not these.
MT_LOAD_ERRORS='plugins/qml/.*(Type [A-Za-z0-9_]+ unavailable|is not a type|is not installed|File not found|Syntax error|Unexpected token|Invalid alias reference|Invalid property|Cannot override|Duplicate)'
MT_CANDIDATES="/usr/lib/maliit/plugins/qml"
MT_ROOT_FILE=view-global/main.qml
MT_STYLE_FILE=keyboard-common/style/Style.qml

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n\r\t' '   '
}

emit_error() {
  printf '{"ok":false,"error":"%s"}\n' "$(json_escape "$1")"
}

log_line() {
  mkdir -p "$STATE" 2>/dev/null
  printf '%s %s: %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$MT_NAME" "$(printf '%s' "$1" | tr '\n' ' ')" >> "$LOGFILE" 2>/dev/null
}

# LG's QML directory: the override, else the first candidate that holds the global plugin's root item.
mt_qml() {
  if [ -n "${MALIIT_QML:-}" ]; then
    [ -f "$MALIIT_QML/$MT_ROOT_FILE" ] && printf '%s\n' "$MALIIT_QML"
    return
  fi
  for d in $MT_CANDIDATES; do
    if [ -f "$d/$MT_ROOT_FILE" ]; then printf '%s\n' "$d"; return; fi
  done
}

mt_mounted() {
  grep -q " $1 overlay " "$PROC_MOUNTS" 2>/dev/null
}

mt_umount() {
  # Unmounts every layer of ours over $1 (a remount stacks, so the loop is not for show).
  n=0
  while mt_mounted "$1" && [ "$n" -lt 8 ]; do
    $UMOUNT "$1" >/dev/null 2>&1 || break
    n=$((n+1))
  done
  mt_mounted "$1" && return 1
  return 0
}

# A fresh copy of LG's tree, patched; the upper layer is just the files the patch rewrote. The old upper layer stays until the new one is
# whole. Must run with nothing mounted over $1, or the copy would be of our own files (and the patch refuses it).
mt_stage() {
  qml="$1"
  pal=$(python3 "$TOKENS_TOOL" "$THEME_SRC/theme.json" "$THEME_DIR/theme.json" qml < /dev/null 2>>"$LOGFILE") || { log_line "no palette"; return 1; }
  rm -rf "$MT_STAGE/full.new" "$MT_STAGE/upper.new" 2>/dev/null
  mkdir -p "$MT_STAGE" 2>/dev/null || return 1
  cp -a "$qml" "$MT_STAGE/full.new" 2>/dev/null || { log_line "could not copy $qml"; rm -rf "$MT_STAGE/full.new"; return 1; }
  files=$(python3 "$PATCH_TOOL" "$MT_STAGE/full.new" "$pal" "$FONTS_DIR" < /dev/null 2>>"$LOGFILE") || {
    log_line "the patch refused LG's tree: $(tail -n 1 "$LOGFILE" 2>/dev/null)"
    rm -rf "$MT_STAGE/full.new"
    return 1
  }
  [ -n "$files" ] || { rm -rf "$MT_STAGE/full.new"; return 1; }
  mkdir -p "$MT_STAGE/upper.new" 2>/dev/null || return 1
  ok=1
  for rel in $files; do
    mkdir -p "$MT_STAGE/upper.new/$(dirname "$rel")" 2>/dev/null
    cp -f "$MT_STAGE/full.new/$rel" "$MT_STAGE/upper.new/$rel" 2>/dev/null || ok=0
  done
  rm -rf "$MT_STAGE/full.new"
  [ "$ok" = 1 ] || { rm -rf "$MT_STAGE/upper.new"; return 1; }
  rm -rf "$MT_STAGE/upper.old" 2>/dev/null
  [ -d "$MT_UPPER" ] && mv "$MT_UPPER" "$MT_STAGE/upper.old" 2>/dev/null
  mv "$MT_STAGE/upper.new" "$MT_UPPER" 2>/dev/null || return 1
  rm -rf "$MT_STAGE/upper.old" 2>/dev/null
  return 0
}

mt_mount() {
  # overlayfs wants an empty work directory on the upper layer's file system.
  rm -rf "$MT_WORK" 2>/dev/null
  mkdir -p "$MT_WORK" 2>/dev/null || return 1
  $MOUNT -t overlay "$MT_NAME" -o "lowerdir=$1,upperdir=$MT_UPPER,workdir=$MT_WORK" "$1" >>"$LOGFILE" 2>&1
}

# ---- the server
# What systemd says of the unit (active, inactive, failed, activating, ...); empty when there is no systemctl.
mt_unit_state() {
  $SYSTEMCTL is-active "$MT_UNIT" 2>/dev/null | tr -d ' \r\n'
  return 0
}

mt_log_size() {
  wc -c < "$MT_MESSAGES" 2>/dev/null | tr -d ' ' || true
}

mt_wait() {
  [ "$MT_STEP" = 0 ] || sleep "$MT_STEP"
}

# Restarts the unit when it is running. Sets MT_RESTART (restarted, inactive) and MT_WHY (what went wrong, when something did). A
# restart that leaves the unit not active, or a QML file that the server logged as not loaded, is a failure (return 1).
mt_restart() {
  MT_RESTART=inactive; MT_WHY=""; MT_WAS_ACTIVE=0
  [ "$(mt_unit_state)" = active ] || return 0
  MT_WAS_ACTIVE=1
  before=$(mt_log_size)
  if ! $SYSTEMCTL try-restart "$MT_UNIT" >>"$LOGFILE" 2>&1; then
    MT_WHY="systemctl try-restart $MT_UNIT failed"
    return 1
  fi
  MT_RESTART=restarted
  s=0
  while [ "$s" -le "$MT_SETTLE" ]; do
    mt_wait
    s=$((s+1))
    if [ "$(mt_unit_state)" != active ]; then
      MT_WHY="$MT_UNIT is $(mt_unit_state) $s s after the restart"
      return 1
    fi
  done
  if [ -n "$before" ] && [ -r "$MT_MESSAGES" ]; then
    # (NUL bytes dropped and the C locale: a log with a stray byte is still text to grep, which would otherwise say "binary file matches")
    bad=$(tail -c "+$((before + 1))" "$MT_MESSAGES" 2>/dev/null | tr -d '\000' | LC_ALL=C grep 'MaliitServer' | LC_ALL=C grep -E "$MT_LOAD_ERRORS" | head -n 2 | cut -c1-300)
    if [ -n "$bad" ]; then
      MT_WHY="MaliitServer could not load a QML file: $bad"
      return 1
    fi
  fi
  return 0
}

# The restart of `off`: nothing to check, LG's QML is what it reads.
mt_restart_plain() {
  [ "$(mt_unit_state)" = active ] || { MT_RESTART=inactive; return 0; }
  $SYSTEMCTL try-restart "$MT_UNIT" >>"$LOGFILE" 2>&1 && MT_RESTART=restarted || MT_RESTART=failed
  return 0
}

# After a rollback: the server is running on LG's QML again. A unit that our restart left failed (or that is restarting itself) is brought
# up with `restart`, which a `try-restart` would not do; one that is running is restarted as before. Only when the unit was running before.
mt_recover() {
  [ "$MT_WAS_ACTIVE" = 1 ] || return 0
  if [ "$(mt_unit_state)" = active ]; then
    $SYSTEMCTL try-restart "$MT_UNIT" >>"$LOGFILE" 2>&1
  else
    $SYSTEMCTL reset-failed "$MT_UNIT" >>"$LOGFILE" 2>&1
    $SYSTEMCTL restart "$MT_UNIT" >>"$LOGFILE" 2>&1
  fi
  return 0
}

verb_stage() {
  qml=$(mt_qml)
  [ -n "$qml" ] || { emit_error "LG's maliit QML was not found (set MALIIT_QML)"; return 1; }
  mt_mounted "$qml" && { emit_error "an overlay is mounted over $qml: unmount it first, a stage is a copy of LG's tree"; return 1; }
  if mt_stage "$qml"; then
    printf '{"ok":true,"staged":true,"qml":"%s"}\n' "$(json_escape "$qml")"
  else
    emit_error "could not stage the patched copy; LG's keys are in place"
    return 1
  fi
}

verb_on() {
  qml=$(mt_qml)
  [ -n "$qml" ] || { emit_error "LG's maliit QML was not found (set MALIIT_QML)"; return 1; }
  mt_umount "$qml" || { emit_error "could not unmount the overlay over $qml"; return 1; }
  if ! mt_stage "$qml"; then
    emit_error "could not stage the patched copy; LG's keys are in place"
    return 1
  fi
  if ! mt_mount "$qml"; then
    emit_error "could not mount the patched copy; LG's keys are in place"
    return 1
  fi
  if ! mt_restart; then
    why=$MT_WHY
    mt_umount "$qml"
    mt_recover
    log_line "on: rolled back, $why"
    emit_error "$why; the patched keys are unmounted and the server is brought up on LG's"
    return 1
  fi
  log_line "on: mounted over $qml, server $MT_RESTART"
  printf '{"ok":true,"keys":"on","restarted":%s,"unit":"%s"}\n' "$([ "$MT_RESTART" = restarted ] && echo true || echo false)" "$(mt_unit_state)"
}

verb_off() {
  qml=$(mt_qml)
  [ -n "$qml" ] || { emit_error "LG's maliit QML was not found (set MALIIT_QML)"; return 1; }
  was=0; mt_mounted "$qml" && was=1
  mt_umount "$qml" || { emit_error "could not unmount the overlay over $qml"; return 1; }
  MT_RESTART=inactive
  # Only when there was something to undo: a set that never used the switch loses nothing to `off`.
  [ "$was" = 1 ] && mt_restart_plain
  log_line "off: unmounted from $qml, server $MT_RESTART"
  printf '{"ok":true,"keys":"off","restarted":%s,"unit":"%s"}\n' "$([ "$MT_RESTART" = restarted ] && echo true || echo false)" "$(mt_unit_state)"
}

verb_status() {
  qml=$(mt_qml)
  printf '{"ok":true,"qml":"%s","staged":%s,"mounted":%s,"unit":"%s"}\n' "$(json_escape "${qml:-}")" \
    "$( [ -f "$MT_UPPER/$MT_STYLE_FILE" ] && echo true || echo false )" \
    "$( [ -n "$qml" ] && mt_mounted "$qml" && echo true || echo false )" \
    "$(mt_unit_state)"
}

case "${1:-status}" in
  on) verb_on ;;
  off) verb_off ;;
  stage) verb_stage ;;
  status) verb_status ;;
  *) emit_error "usage: maliit.sh on|off|stage|status"; exit 1 ;;
esac
