#!/bin/sh
# LG's second-screen gateway on port 3001 (the SSAP websocket Home Assistant's webostv integration uses) handed to LumaTV's Rust
# gateway, with LG's as the fallback for every connection. POSIX sh / busybox ash.
#
#   ssgw.sh on       bind LumaTV's copy of xinetd's entry for port 3001 over LG's, have xinetd read it, remember it
#   ssgw.sh off      forget it, take LG's entry back and have xinetd read it
#   ssgw.sh early    boot step: bind it again when it was switched on; always exit 0, bounded
#   ssgw.sh status   JSON: the entry, the marker, the programs
#
# xinetd runs LG's `/usr/bin/nsl 3001 com.webos.service.secondscreen.gateway` for each connection to 3001 (/etc/xinetd.d/ssg_https).
# LumaTV's copy differs in the `server` line only: $BIN/ssgw-accept, which hands the connection to the gateway (`ssgw`, run by the
# helper's plugin `ssgw`) over its accept socket and execs LG's nsl with the same arguments whenever the gateway is not there or does not
# take it in time. So with the gateway down every connection goes LG's way and nothing needs switching back. Port 3000 is not touched.
#
# The copy is bound (mount --bind; /etc is a read-only overlay) only when LG's entry and nsl are the ones this was made for (sha256
# pinned below), ssgw-accept is installed, and nothing is mounted on the entry yet; a mount that is not this tool's is never stacked on
# or removed. A reboot puts LG's entry back; `early` binds it again from the boot hook while `enabled` is there. `off` deletes
# `enabled` before anything else.
# The plugin runs `on` when it is switched on and `off` when it is switched off; the owner's escape and `cutover.sh revert` run `off`
# through `lumatv ssgw off`, which switches the plugin off too. A compositor cutover, an install and the gateway's crashes leave it.
set -u

APPDIR=${APPDIR:-$(cd "$(dirname "$0")/.." && pwd)}
if [ -z "${STATE:-}" ]; then
  . "$APPDIR/tools/brand.sh"
  brand_load "$APPDIR/brand.json" 2>/dev/null || { printf '{"ok":false,"error":"brand.json is missing or invalid"}\n'; exit 1; }
  STATE="$BRAND_DATA_DIR"
fi

GW=${SSGW_DIR:-$STATE/ssgw}
BIN="$GW/bin"
ACCEPT="$BIN/ssgw-accept"
COPY="$GW/ssg_https"
ENABLED="$GW/enabled"
LOCKDIR="$GW/.lock"
LOGFILE=${LOGFILE:-$STATE/log}

ENTRY=${SSGW_ENTRY:-/etc/xinetd.d/ssg_https}
# webOS 10.2.1's entry (283 bytes) and nsl; the gateway pins the same files and LG's gateway's own behaviour was read from them.
ENTRY_SHA=${SSGW_ENTRY_SHA:-01011c104d1e6874a1bd6f691bb07fd539155e802a8b4a91fb4b2acb5b85547c}
NSL=${SSGW_NSL:-/usr/bin/nsl}
NSL_SHA=${SSGW_NSL_SHA:-d2936420cdc07d4a84aa27fcc1d1cfaf1d0d5a35574b2f15e3ca6e8174eedfa6}
XINETD_PIDFILE=${SSGW_XINETD_PIDFILE:-/var/run/xinetd.pid}
XINETD_EXE=${SSGW_XINETD_EXE:-/usr/sbin/xinetd}
MOUNT=${MOUNT:-mount}
UMOUNT=${UMOUNT:-umount}
KILL=${KILL:-kill}
PROC_MOUNTS=${PROC_MOUNTS:-/proc/mounts}
PROC_ROOT=${PROC_ROOT:-/proc}
LOCK_WAIT=${SSGW_LOCK_WAIT:-30}
CONFIG=${SSGW_CONFIG:-$STATE/config.json}

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n\r\t' '   '
}

emit_error() {
  printf '{"ok":false,"error":"%s"}\n' "$(json_escape "$1")"
}

log_line() {
  mkdir -p "$(dirname "$LOGFILE")" 2>/dev/null
  printf '%s ssgw: %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$(printf '%s' "$1" | tr '\n' ' ')" >> "$LOGFILE" 2>/dev/null
  # The state log is shared by the tool scripts and kept on the TV's flash: only the last 300 lines stay.
  if [ "$(wc -l < "$LOGFILE" 2>/dev/null | tr -d ' ')" -gt 300 ] 2>/dev/null; then
    tail -n 300 "$LOGFILE" > "$LOGFILE.trim.$$" 2>/dev/null && mv "$LOGFILE.trim.$$" "$LOGFILE"
  fi
}

sha_of() {
  sha256sum "$1" 2>/dev/null | cut -d' ' -f1
}

# The program's path goes into xinetd's configuration and an awk replacement.
case "$ACCEPT" in
  ''|*[!A-Za-z0-9/._-]*) emit_error "the state directory has characters xinetd's configuration cannot carry"; exit 1 ;;
esac

take_lock() {
  mkdir -p "$GW" 2>/dev/null || return 1
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

mounts_on_entry() {
  awk -v p="$ENTRY" '$2 == p { n++ } END { print n + 0 }' "$PROC_MOUNTS" 2>/dev/null
}

# stock: LG's entry, nothing mounted. ours: exactly one mount and it shows the copy itself (same device and inode: the same bytes on
# another inode are somebody else's mount). foreign: any other mount on the entry. unknown: nothing mounted and not the entry this was
# made for.
entry_state() {
  es_n=$(mounts_on_entry)
  if [ "$es_n" -gt 0 ]; then
    if [ "$es_n" = 1 ] && [ -f "$COPY" ] && [ "$ENTRY" -ef "$COPY" ]; then
      echo ours
    else
      echo foreign
    fi
    return 0
  fi
  if [ -f "$ENTRY" ] && [ "$(sha_of "$ENTRY")" = "$ENTRY_SHA" ]; then
    echo stock
  else
    echo unknown
  fi
}

accept_ok() {
  [ -f "$ACCEPT" ] && [ -x "$ACCEPT" ]
}

# LG's entry with its one `server` line naming ssgw-accept; `server_args` and everything else as they are.
build_copy() {
  awk -v accept="$ACCEPT" '
    /^[ \t]*server[ \t]*=/ { n++; sub(/=.*/, "= " accept) }
    { print }
    END { exit n == 1 ? 0 : 1 }' "$1" > "$2"
}

# xinetd reads its configuration again on HUP; connections it already handed out keep their program. Prints told or not running.
hup_xinetd() {
  hx_pid=$(tr -d ' \n' < "$XINETD_PIDFILE" 2>/dev/null)
  case "$hx_pid" in ''|*[!0-9]*) echo "not running"; return 0 ;; esac
  if [ "$(readlink "$PROC_ROOT/$hx_pid/exe" 2>/dev/null)" = "$XINETD_EXE" ] && "$KILL" -HUP "$hx_pid" 2>/dev/null; then
    echo told
  else
    echo "not running"
  fi
}

# Takes back what a failed run laid: only a mount that is proven ours is unmounted, and the copy goes only once nothing is mounted over
# the entry. Returns 1 when something is still mounted.
undo_bind() {
  [ "$(entry_state)" = ours ] && "$UMOUNT" "$ENTRY" 2>/dev/null
  [ "$(mounts_on_entry)" = 0 ] || return 1
  rm -f "$COPY"
  return 0
}

# Binds the copy. $1 = remember: write `enabled` once it stands. Sets $sg_result (bound, already) and $sg_told, or $sg_err.
apply() {
  sg_err="" sg_result="" sg_told=""
  mkdir -p "$GW" 2>/dev/null || { sg_err="$GW cannot be created"; return 1; }
  ap_state=$(entry_state)
  case "$ap_state" in
    foreign) sg_err="another mount is on $ENTRY; nothing was changed"; return 1 ;;
    unknown) sg_err="$ENTRY is not the entry this was made for (sha256 $(sha_of "$ENTRY")); nothing was changed"; return 1 ;;
  esac
  [ "$(sha_of "$NSL")" = "$NSL_SHA" ] || { sg_err="$NSL is not the program this was made for (sha256 $(sha_of "$NSL")); nothing was changed"; return 1; }
  # Without it xinetd would have no program at all for port 3001, LG's gateway included.
  accept_ok || { sg_err="$ACCEPT is not installed; nothing was changed"; return 1; }
  if [ "$ap_state" = stock ]; then
    rm -f "$COPY.new"
    if ! build_copy "$ENTRY" "$COPY.new" || ! chmod 644 "$COPY.new" || ! mv "$COPY.new" "$COPY"; then
      rm -f "$COPY.new"
      sg_err="LG's entry could not be copied"
      return 1
    fi
    if ! "$MOUNT" --bind "$COPY" "$ENTRY" 2>/dev/null || [ "$(entry_state)" != ours ]; then
      sg_err="the copy could not be bound over $ENTRY"
      undo_bind || sg_err="$sg_err, and a mount on it could not be taken back"
      return 1
    fi
    sg_result=bound
  else
    sg_result=already
  fi
  if [ "${1:-no}" = remember ]; then
    if ! { printf 'enabled\n' > "$ENABLED.tmp" && mv "$ENABLED.tmp" "$ENABLED"; }; then
      rm -f "$ENABLED.tmp"
      sg_err="$ENABLED could not be written"
      # A bind laid by this call goes with it; one that was already there stays as it was.
      [ "$sg_result" = bound ] && { undo_bind || sg_err="$sg_err, and the bind could not be taken back"; }
      return 1
    fi
  fi
  [ "$sg_result" = bound ] && sg_told=$(hup_xinetd)
  return 0
}

# ---------------------------------------------------------------- verbs

verb_on() {
  take_lock || { emit_error "another ssgw run is in progress"; return 1; }
  if ! apply remember; then
    log_line "on refused: $sg_err"
    emit_error "$sg_err"
    return 1
  fi
  if [ "$sg_result" = bound ]; then
    log_line "on: bound, xinetd $sg_told"
    printf '{"ok":true,"ssgw":"bound","xinetd":"%s"}\n' "$sg_told"
  else
    printf '{"ok":true,"ssgw":"already"}\n'
  fi
}

# Nothing staged, nothing mounted, nobody working: there is nothing for `off` to do and it leaves no trace.
off_idle() {
  [ ! -d "$LOCKDIR" ] && [ "$(mounts_on_entry)" = 0 ] || return 1
  for oi_f in "$ENABLED" "$ENABLED.tmp" "$COPY" "$COPY.new"; do
    [ -e "$oi_f" ] && return 1
  done
  return 0
}

verb_off() {
  if off_idle; then
    printf '{"ok":true,"ssgw":"off","was":"%s"}\n' "$(entry_state)"
    return 0
  fi
  take_lock || { emit_error "another ssgw run is in progress; off was not applied"; return 1; }
  # With the lock held no `on` or boot step can write the marker back; it goes before anything that can fail, so a boot does not bind
  # the copy again whatever else happens.
  rm -f "$ENABLED" "$ENABLED.tmp"
  vo_state=$(entry_state)
  vo_told=""
  case "$vo_state" in
    foreign) log_line "off refused: a mount that is not ours is on the entry"; emit_error "another mount is on $ENTRY and was left alone; the boot marker is removed"; return 1 ;;
    ours)
      "$UMOUNT" "$ENTRY" 2>/dev/null
      if [ "$(mounts_on_entry)" != 0 ]; then
        log_line "off: the copy could not be unmounted"
        emit_error "the copy could not be unmounted; the boot marker is removed and the copy stays until the next boot"
        return 1
      fi
      vo_told=$(hup_xinetd) ;;
  esac
  rm -f "$COPY" "$COPY.new"
  log_line "off: $vo_state${vo_told:+, xinetd $vo_told}"
  printf '{"ok":true,"ssgw":"off","was":"%s"%s}\n' "$vo_state" "${vo_told:+,\"xinetd\":\"$vo_told\"}"
}

# The boot hook's step. Always exits 0: a failure is a line in the log, never a held-up boot.
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
    log_line "early: $sg_result${sg_told:+, xinetd $sg_told} at uptime $(cut -d. -f1 /proc/uptime 2>/dev/null) s"
    printf '{"ok":true,"applied":true,"ssgw":"%s"}\n' "$sg_result"
  else
    log_line "early: not applied ($sg_err)"
    printf '{"ok":true,"applied":false,"why":"%s"}\n' "$(json_escape "$sg_err")"
  fi
  return 0
}

verb_status() {
  st_enabled=false
  [ -f "$ENABLED" ] && st_enabled=true
  st_nsl=stock
  [ "$(sha_of "$NSL")" = "$NSL_SHA" ] || st_nsl=changed
  st_accept=false
  accept_ok && st_accept=true
  st_gateway=false
  [ -f "$BIN/ssgw" ] && [ -x "$BIN/ssgw" ] && st_gateway=true
  st_persist=false
  grep -q '"persist"[[:space:]]*:[[:space:]]*true' "$CONFIG" 2>/dev/null && st_persist=true
  # enabled is what a boot binds again; entry is what is bound now (ours: xinetd hands 3001 to ssgw-accept).
  printf '{"ok":true,"enabled":%s,"entry":"%s","nsl":"%s","accept":%s,"gateway":%s,"bootHook":%s}\n' \
    "$st_enabled" "$(entry_state)" "$st_nsl" "$st_accept" "$st_gateway" "$st_persist"
}

case "${1:-status}" in
  on) verb_on ;;
  off) verb_off ;;
  early) verb_early ;;
  status) verb_status ;;
  *) emit_error "usage: ssgw.sh on | off | early | status"; exit 1 ;;
esac
