#!/bin/sh
# LG's input banners (com.webos.app.inputcommon) in the Luma look: a patched copy of its QML, laid over LG's directory by an
# overlay mount. POSIX sh / busybox ash. Nothing is written into the root file system: the copy and the overlay's upper layer
# live under $STATE, the only thing that touches LG's directory is a mount over it, and `off` (or a reboot) puts LG's back.
#
#   inputcommon.sh on       unmount, stage a fresh patched copy, mount it over LG's directory, drop the process snapshot
#   inputcommon.sh off      unmount, drop the process snapshot
#   inputcommon.sh stage    only the staging (the upper layer under $STATE/inputcommon/upper), no mount: what the host tests run
#   inputcommon.sh status   JSON: where LG's QML is, whether the copy is staged and whether it is mounted
#
# Each verb prints one JSON object. A stage that fails (tools/inputcommon_patch.py refusing an anchor LG changed, no fonts,
# no QML directory) leaves LG's tree in force and the previous stage as it was: nothing is mounted from a half-patched copy.
#
# Why the snapshot: inputcommon is restored from a CRIU checkpoint (/mnt/lg/cmn_data/checkpoint holds only it, PLAN.md
# platform findings), and a restored process holds the QML it was dumped with, mounts or not. Touching `invalid_img` in its
# checkpoint directory makes LG take a fresh start. A running inputcommon is not touched: an input that is already up keeps
# its banners until it is next launched, since closing the app of the input on screen would leave it.
#
# [INFERENCE] where the QML is: the census copy of the tree has the same layout as the QML of LG's other native apps
# (/usr/palm/applications/<id>/qml); INPUTCOMMON_QML overrides. Not verified on the TV (docs/research/lg-restyles-result.md).
set -u

APPDIR=${APPDIR:-$(cd "$(dirname "$0")/.." && pwd)}
if [ -z "${STATE:-}" ]; then
  . "$APPDIR/tools/brand.sh"
  brand_load "$APPDIR/brand.json" 2>/dev/null || { printf '{"ok":false,"error":"brand.json is missing or invalid"}\n'; exit 1; }
  STATE="$BRAND_DATA_DIR"
fi

mkdir -p "$STATE" 2>/dev/null
IC_ID=com.webos.app.inputcommon
IC_STAGE="$STATE/inputcommon"
IC_UPPER="$IC_STAGE/upper"
IC_WORK="$IC_STAGE/work"
IC_PATCH_TOOL="$APPDIR/tools/inputcommon_patch.py"
TOKENS_TOOL="$APPDIR/tools/tokens.py"
THEME_SRC=${THEME_SRC:-$APPDIR/payload/theme}
THEME_DIR=${THEME_DIR:-$STATE/theme}
FONTS_DIR=${FONTS_DIR:-$STATE/fonts}
LOGFILE=${LOGFILE:-$STATE/log}
CHECKPOINTS=${CHECKPOINTS:-/mnt/lg/cmn_data/checkpoint}
PROC_MOUNTS=${PROC_MOUNTS:-/proc/mounts}
MOUNT=${MOUNT:-mount}
UMOUNT=${UMOUNT:-umount}
IC_CANDIDATES="/usr/palm/applications/$IC_ID/qml /usr/share/$IC_ID/qml"

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n\r\t' '   '
}

emit_error() {
  printf '{"ok":false,"error":"%s"}\n' "$(json_escape "$1")"
}

log_line() {
  mkdir -p "$STATE" 2>/dev/null
  printf '%s inputcommon: %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$(printf '%s' "$1" | tr '\n' ' ')" >> "$LOGFILE" 2>/dev/null
}

# LG's QML directory: the override, else the first candidate that holds the root window.
ic_qml() {
  if [ -n "${INPUTCOMMON_QML:-}" ]; then
    [ -f "$INPUTCOMMON_QML/RootWindow.qml" ] && printf '%s\n' "$INPUTCOMMON_QML"
    return
  fi
  for d in $IC_CANDIDATES; do
    if [ -f "$d/RootWindow.qml" ]; then printf '%s\n' "$d"; return; fi
  done
}

ic_mounted() {
  grep -q " $1 overlay " "$PROC_MOUNTS" 2>/dev/null
}

ic_umount() {
  # Unmounts every layer of ours over $1 (a remount stacks, so the loop is not for show).
  n=0
  while ic_mounted "$1" && [ "$n" -lt 8 ]; do
    $UMOUNT "$1" >/dev/null 2>&1 || break
    n=$((n+1))
  done
  ic_mounted "$1" && return 1
  return 0
}

# A fresh copy of LG's tree, patched; the upper layer is just the files the patch rewrote. The old upper layer stays until
# the new one is whole. Must run with nothing mounted over $1, or the copy would be of our own files (and the patch refuses it).
ic_stage() {
  qml="$1"
  pal=$(python3 "$TOKENS_TOOL" "$THEME_SRC/theme.json" "$THEME_DIR/theme.json" qml < /dev/null 2>>"$LOGFILE") || { log_line "no palette"; return 1; }
  rm -rf "$IC_STAGE/full.new" "$IC_STAGE/upper.new" 2>/dev/null
  mkdir -p "$IC_STAGE" 2>/dev/null || return 1
  cp -a "$qml" "$IC_STAGE/full.new" 2>/dev/null || { log_line "could not copy $qml"; rm -rf "$IC_STAGE/full.new"; return 1; }
  files=$(python3 "$IC_PATCH_TOOL" "$IC_STAGE/full.new" "$pal" "$FONTS_DIR" < /dev/null 2>>"$LOGFILE") || {
    log_line "the patch refused LG's tree: $(tail -n 1 "$LOGFILE" 2>/dev/null)"
    rm -rf "$IC_STAGE/full.new"
    return 1
  }
  [ -n "$files" ] || { rm -rf "$IC_STAGE/full.new"; return 1; }
  mkdir -p "$IC_STAGE/upper.new" 2>/dev/null || return 1
  ok=1
  for rel in $files; do
    mkdir -p "$IC_STAGE/upper.new/$(dirname "$rel")" 2>/dev/null
    cp -f "$IC_STAGE/full.new/$rel" "$IC_STAGE/upper.new/$rel" 2>/dev/null || ok=0
  done
  rm -rf "$IC_STAGE/full.new"
  [ "$ok" = 1 ] || { rm -rf "$IC_STAGE/upper.new"; return 1; }
  rm -rf "$IC_STAGE/upper.old" 2>/dev/null
  [ -d "$IC_UPPER" ] && mv "$IC_UPPER" "$IC_STAGE/upper.old" 2>/dev/null
  mv "$IC_STAGE/upper.new" "$IC_UPPER" 2>/dev/null || return 1
  rm -rf "$IC_STAGE/upper.old" 2>/dev/null
  return 0
}

ic_mount() {
  # overlayfs wants an empty work directory on the upper layer's file system.
  rm -rf "$IC_WORK" 2>/dev/null
  mkdir -p "$IC_WORK" 2>/dev/null || return 1
  $MOUNT -t overlay inputcommon -o "lowerdir=$1,upperdir=$IC_UPPER,workdir=$IC_WORK" "$1" >>"$LOGFILE" 2>&1
}

# The snapshot LG restores the process from, dropped so the next start reads the QML that is mounted now (or not).
ic_snapshot() {
  n=0
  for d in "$CHECKPOINTS"/*inputcommon*; do
    [ -d "$d" ] || continue
    : > "$d/invalid_img" 2>/dev/null && n=$((n+1))
  done
  printf '%s\n' "$n"
}

verb_stage() {
  qml=$(ic_qml)
  [ -n "$qml" ] || { emit_error "LG's inputcommon QML was not found (set INPUTCOMMON_QML)"; return 1; }
  ic_mounted "$qml" && { emit_error "an overlay is mounted over $qml: unmount it first, a stage is a copy of LG's tree"; return 1; }
  if ic_stage "$qml"; then
    printf '{"ok":true,"staged":true,"qml":"%s"}\n' "$(json_escape "$qml")"
  else
    emit_error "could not stage the patched copy; LG's banners are in place"
    return 1
  fi
}

verb_on() {
  qml=$(ic_qml)
  [ -n "$qml" ] || { emit_error "LG's inputcommon QML was not found (set INPUTCOMMON_QML)"; return 1; }
  ic_umount "$qml" || { emit_error "could not unmount the overlay over $qml"; return 1; }
  if ! ic_stage "$qml"; then
    emit_error "could not stage the patched copy; LG's banners are in place"
    return 1
  fi
  if ! ic_mount "$qml"; then
    emit_error "could not mount the patched copy; LG's banners are in place"
    return 1
  fi
  dropped=$(ic_snapshot)
  log_line "on: mounted over $qml, snapshots dropped $dropped"
  printf '{"ok":true,"inputcommon":"on","snapshotsDropped":%s}\n' "$dropped"
}

verb_off() {
  qml=$(ic_qml)
  [ -n "$qml" ] || { emit_error "LG's inputcommon QML was not found (set INPUTCOMMON_QML)"; return 1; }
  ic_umount "$qml" || { emit_error "could not unmount the overlay over $qml"; return 1; }
  dropped=$(ic_snapshot)
  log_line "off: unmounted from $qml, snapshots dropped $dropped"
  printf '{"ok":true,"inputcommon":"off","snapshotsDropped":%s}\n' "$dropped"
}

verb_status() {
  qml=$(ic_qml)
  printf '{"ok":true,"qml":"%s","staged":%s,"mounted":%s}\n' "$(json_escape "${qml:-}")" \
    "$( [ -f "$IC_UPPER/UserInterfaceLayer/GlobalStyle.qml" ] && echo true || echo false )" \
    "$( [ -n "$qml" ] && ic_mounted "$qml" && echo true || echo false )"
}

case "${1:-status}" in
  on) verb_on ;;
  off) verb_off ;;
  stage) verb_stage ;;
  status) verb_status ;;
  *) emit_error "usage: inputcommon.sh on|off|stage|status"; exit 1 ;;
esac
