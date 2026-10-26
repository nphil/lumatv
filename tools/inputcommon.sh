#!/bin/sh
# LG's input banners (com.webos.app.inputcommon) in the Luma look: a patched copy of its QML, laid over LG's directory by an
# overlay mount. POSIX sh / busybox ash. Nothing is written into the root file system: the copy and the overlay's upper layer
# live under $STATE, the only thing that touches LG's directory is a mount over it, and `off` (or a reboot) puts LG's back.
#
#   inputcommon.sh on       unmount, stage a fresh patched copy, mount it over LG's directory, set LG's dump of the app aside,
#                           end the input apps that are running but not on screen (they come back cold)
#   inputcommon.sh off      unmount, put the dump back, end the idle input apps (they come back from it)
#   inputcommon.sh stage    only the staging (the upper layer under $STATE/inputcommon/upper), no mount: what the host tests run
#   inputcommon.sh status   JSON: where LG's QML is, whether the copy is staged and mounted, where LG's dump is
#
# Each verb prints one JSON object. A stage that fails (tools/inputcommon_patch.py refusing an anchor LG changed, no fonts,
# no QML directory) leaves LG's tree in force and the previous stage as it was: nothing is mounted from a half-patched copy.
#
# Why the dump: every app whose main is /usr/bin/com.webos.app.inputcommon (HDMI 1-4, Live TV, AV, component, USB-C, DisplayPort:
# fifteen in the census) is served by an instance that LG's snapshot boot launched once with {"snapshot":true} and dumped with
# CRIU. The instances in the boot snapshot keep running, and each launch after a close is a CRIU restore from the dump, with
# the dumped pid (2513 on this set, every time) and the QML the app had loaded when it was dumped: a mount changes nothing for
# it. docs/research/lg-restyles-result.md, "Cold start", has the evidence and LG's own scripts.
# The restore reads CRIU's working directory $CRIU_TMP (criu.service: CRIU_IMAGE_DIR=/tmp/checkpoint, tmpfs, back from the boot
# snapshot at every cold power-on). So the dump's directory is renamed there to a dot-name nothing of LG's lists, and put back by
# `off`; the copy in /mnt/lg/cmn_data/checkpoint (LG's persisted second copy, which the working copy's page files are symlinks
# into) is never touched, and neither is a marker written there: the `invalid_img` touched there by the first version of this
# tool did nothing on the set (round 7, slots 6 and 6b).
# Running instances hold the old QML in memory, so the ones that are not on screen are ended through SAM (closeByAppId); the
# preload manager loads them again, cold. The input on screen is left: it keeps LG's banners until it is next launched.
#
# [INFERENCE] where the QML is: the census copy of the tree has the same layout as the QML of LG's other native apps
# (/usr/palm/applications/<id>/qml); INPUTCOMMON_QML overrides. Verified on the set in round 7 (273 files, same GlobalStyle.qml).
set -u

APPDIR=${APPDIR:-$(cd "$(dirname "$0")/.." && pwd)}
if [ -z "${STATE:-}" ]; then
  . "$APPDIR/tools/brand.sh"
  brand_load "$APPDIR/brand.json" 2>/dev/null || { printf '{"ok":false,"error":"brand.json is missing or invalid"}\n'; exit 1; }
  STATE="$BRAND_DATA_DIR"
fi

mkdir -p "$STATE" 2>/dev/null
# tools/voice.sh runs this script for LG's Search app with these set (the same overlay and cold start, another tree); the defaults are
# the input banners'.
IC_NAME=${IC_NAME:-inputcommon}
IC_ID=${IC_ID:-com.webos.app.inputcommon}
IC_ROOT_FILE=${IC_ROOT_FILE:-RootWindow.qml}
IC_STYLE_FILE=${IC_STYLE_FILE:-UserInterfaceLayer/GlobalStyle.qml}
IC_STAGE="$STATE/$IC_NAME"
IC_UPPER="$IC_STAGE/upper"
IC_WORK="$IC_STAGE/work"
IC_PATCH_TOOL=${IC_PATCH_TOOL:-$APPDIR/tools/inputcommon_patch.py}
TOKENS_TOOL="$APPDIR/tools/tokens.py"
THEME_SRC=${THEME_SRC:-$APPDIR/payload/theme}
THEME_DIR=${THEME_DIR:-$STATE/theme}
FONTS_DIR=${FONTS_DIR:-$STATE/fonts}
LOGFILE=${LOGFILE:-$STATE/log}
# CRIU's working directory (criu.service: CRIU_IMAGE_DIR). LG's persisted second copy (/mnt/lg/cmn_data/checkpoint) is not used.
CRIU_TMP=${CRIU_TMP:-/tmp/checkpoint}
# criu.sh makes CRIU_ENABLE when the kernel is booted with criu_enabled and not in cold mode; CRIU_RESTORING exists while a
# restore runs (criu_copy_img.sh waits up to five seconds for it to go before it touches the working directory).
CRIU_ENABLE=${CRIU_ENABLE:-/tmp/criu_enable}
CRIU_RESTORING=${CRIU_RESTORING:-/tmp/criu_restoring}
RESTORE_WAIT=${RESTORE_WAIT:-5}
IC_ASIDE=${IC_ASIDE:-.lumatv-aside.}
# Which names in CRIU's working directory are this app's dump (globs; the voice app must not match voiceweb or voiceagent).
IC_DUMP_GLOBS=${IC_DUMP_GLOBS:-*inputcommon*}
LUNA_SEND=${LUNA_SEND:-luna-send}
SAM_TIMEOUT=${SAM_TIMEOUT:-10}
SAM_URI=luna://com.webos.applicationManager
# The apps whose main is /usr/bin/com.webos.app.inputcommon (listApps.json of the census), the ones an instance can be running for.
IC_APPS=${IC_APPS:-"com.webos.app.inputcommon com.webos.app.livetv com.webos.app.livetvopapp com.webos.app.hdmi1 com.webos.app.hdmi2 com.webos.app.hdmi3 com.webos.app.hdmi4 com.webos.app.dp1 com.webos.app.dp2 com.webos.app.externalinput.av1 com.webos.app.externalinput.av2 com.webos.app.externalinput.component com.webos.app.externalinput.scart com.webos.app.usbc1 com.webos.app.usbc2"}
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
  printf '%s %s: %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$IC_NAME" "$(printf '%s' "$1" | tr '\n' ' ')" >> "$LOGFILE" 2>/dev/null
}

# LG's QML directory: the override, else the first candidate that holds the root window.
ic_qml() {
  if [ -n "${IC_QML_OVERRIDE-${INPUTCOMMON_QML:-}}" ]; then
    over=${IC_QML_OVERRIDE-${INPUTCOMMON_QML:-}}
    [ -f "$over/$IC_ROOT_FILE" ] && printf '%s\n' "$over"
    return
  fi
  for d in $IC_CANDIDATES; do
    if [ -f "$d/$IC_ROOT_FILE" ]; then printf '%s\n' "$d"; return; fi
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
  $MOUNT -t overlay "$IC_NAME" -o "lowerdir=$1,upperdir=$IC_UPPER,workdir=$IC_WORK" "$1" >>"$LOGFILE" 2>&1
}

# ---- LG's CRIU dump of the app, and the instances that hold its QML
# Names of the dump directories of inputcommon in CRIU's working directory. `*` does not match the dot-named copies set aside below.
ic_dumps() {
  # Matched with `case` on each name, so that the patterns are never expanded against the current directory.
  for d in "$CRIU_TMP"/*; do
    [ -d "$d" ] || continue
    n=${d##*/}
    set -f
    for pat in $IC_DUMP_GLOBS; do
      case "$n" in $pat) printf '%s\n' "$n"; break ;; esac
    done
    set +f
  done
  return 0
}

# The dumps this tool set aside, by the dumps' own names.
ic_asides() {
  for d in "$CRIU_TMP"/"$IC_ASIDE"*; do
    [ -d "$d" ] || continue
    n=${d##*/}
    printf '%s\n' "${n#"$IC_ASIDE"}"
  done
  return 0
}

ic_wait_restore() {
  w=0
  while [ -e "$CRIU_RESTORING" ] && [ "$w" -lt "$RESTORE_WAIT" ]; do sleep 1; w=$((w+1)); done
  [ -e "$CRIU_RESTORING" ] && log_line "a CRIU restore was still running after $RESTORE_WAIT s; going on"
  return 0
}

# Sets LG's dump aside, so that the next launch finds none and starts cold. A rename inside one directory: atomic, nothing is
# copied, and the page files in it stay the symlinks into LG's persisted copy that they are. A dump that is already aside leaves
# what stands at the path now to be a dump made with the overlay mounted, which holds the patched app and goes. Prints how many.
ic_checkpoint_aside() {
  ic_wait_restore
  c=0
  for name in $(ic_dumps); do
    if [ -e "$CRIU_TMP/$IC_ASIDE$name" ]; then
      rm -rf "$CRIU_TMP/$name"
    elif ! mv "$CRIU_TMP/$name" "$CRIU_TMP/$IC_ASIDE$name" 2>>"$LOGFILE"; then
      log_line "could not set the dump $name aside"
      continue
    fi
    c=$((c+1))
  done
  printf '%s\n' "$c"
}

# Puts LG's dump back where its restore looks; a dump made meanwhile held the patched app and is removed first. Prints how many.
# `name` is never empty here: a copy with no name prints an empty line, which the word splitting of $(ic_asides) drops.
ic_checkpoint_back() {
  ic_wait_restore
  c=0
  for name in $(ic_asides); do
    rm -rf "$CRIU_TMP/$name"
    if mv "$CRIU_TMP/$IC_ASIDE$name" "$CRIU_TMP/$name" 2>>"$LOGFILE"; then c=$((c+1)); else log_line "could not put the dump $name back"; fi
  done
  printf '%s\n' "$c"
}

# aside (held by this tool), present (LG's, in its place) or none (no dump: CRIU is not in use, or nothing was dumped).
ic_checkpoint_state() {
  if [ -n "$(ic_asides)" ]; then echo aside
  elif [ -n "$(ic_dumps)" ]; then echo present
  else echo none
  fi
}

# One call to SAM, its answer on one line. `timeout`: luna-send waits for ever on a service that does not answer.
ic_sam() {
  timeout "$SAM_TIMEOUT" $LUNA_SEND -n 1 "$SAM_URI/$1" "$2" 2>/dev/null | tr -d ' \r\n'
}

# Ends the input apps' instances that are running but not on screen: they hold LG's QML, and the preload manager loads them
# again as soon as one closes. The one on screen stays (closing it would leave the input) and keeps its banners until it is next
# launched. Sets IC_CLOSED (ids, one space before each) and IC_ONSCREEN.
ic_close_idle() {
  IC_CLOSED=""; IC_ONSCREEN=""
  running=$(ic_sam running '{}')
  front=$(ic_sam getForegroundAppInfo '{}' | sed -n 's/.*"appId":"\([^"]*\)".*/\1/p')
  for app in $IC_APPS; do
    case "$running" in *"\"id\":\"$app\""*) ;; *) continue ;; esac
    if [ "$app" = "$front" ]; then IC_ONSCREEN=$app; continue; fi
    case "$(ic_sam closeByAppId "{\"id\":\"$app\"}")" in
      *'"returnValue":true'*) IC_CLOSED="$IC_CLOSED $app" ;;
      *) log_line "SAM did not close $app" ;;
    esac
  done
  return 0
}

# ["a","b"] from the words given.
ic_json_list() {
  out=""
  for w in "$@"; do out="$out${out:+,}\"$w\""; done
  printf '[%s]' "$out"
}

verb_stage() {
  qml=$(ic_qml)
  [ -n "$qml" ] || { emit_error "LG's $IC_NAME QML was not found (set ${IC_OVERRIDE_NAME:-INPUTCOMMON_QML})"; return 1; }
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
  [ -n "$qml" ] || { emit_error "LG's $IC_NAME QML was not found (set ${IC_OVERRIDE_NAME:-INPUTCOMMON_QML})"; return 1; }
  ic_umount "$qml" || { emit_error "could not unmount the overlay over $qml"; return 1; }
  if ! ic_stage "$qml"; then
    emit_error "could not stage the patched copy; LG's banners are in place"
    return 1
  fi
  if ! ic_mount "$qml"; then
    emit_error "could not mount the patched copy; LG's banners are in place"
    return 1
  fi
  aside=$(ic_checkpoint_aside)
  ic_close_idle
  log_line "on: mounted over $qml, dumps set aside $aside, ended:${IC_CLOSED:- none}, on screen: ${IC_ONSCREEN:-none}"
  printf '{"ok":true,"%s":"on","checkpointAside":%s,"closed":%s,"onScreen":"%s"}\n' "$IC_NAME" "$aside" "$(ic_json_list $IC_CLOSED)" "$IC_ONSCREEN"
}

verb_off() {
  qml=$(ic_qml)
  [ -n "$qml" ] || { emit_error "LG's $IC_NAME QML was not found (set ${IC_OVERRIDE_NAME:-INPUTCOMMON_QML})"; return 1; }
  was=0; ic_mounted "$qml" && was=1
  ic_umount "$qml" || { emit_error "could not unmount the overlay over $qml"; return 1; }
  back=$(ic_checkpoint_back)
  IC_CLOSED=""; IC_ONSCREEN=""
  # Only when there was something to undo: a set that never used the switch loses no running app to `off`.
  if [ "$was" = 1 ] || [ "$back" != 0 ]; then ic_close_idle; fi
  log_line "off: unmounted from $qml, dumps put back $back, ended:${IC_CLOSED:- none}, on screen: ${IC_ONSCREEN:-none}"
  printf '{"ok":true,"%s":"off","checkpointBack":%s,"closed":%s,"onScreen":"%s"}\n' "$IC_NAME" "$back" "$(ic_json_list $IC_CLOSED)" "$IC_ONSCREEN"
}

verb_status() {
  qml=$(ic_qml)
  printf '{"ok":true,"qml":"%s","staged":%s,"mounted":%s,"checkpoint":"%s","criuEnabled":%s}\n' "$(json_escape "${qml:-}")" \
    "$( [ -f "$IC_UPPER/$IC_STYLE_FILE" ] && echo true || echo false )" \
    "$( [ -n "$qml" ] && ic_mounted "$qml" && echo true || echo false )" \
    "$(ic_checkpoint_state)" \
    "$( [ -e "$CRIU_ENABLE" ] && echo true || echo false )"
}

case "${1:-status}" in
  on) verb_on ;;
  off) verb_off ;;
  stage) verb_stage ;;
  status) verb_status ;;
  *) emit_error "usage: $IC_NAME.sh on|off|stage|status"; exit 1 ;;
esac
