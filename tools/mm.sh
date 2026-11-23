#!/bin/sh
# LG's memory manager (memchute, com.webos.memorymanager) handed to LumaTV's, with LG's as the fallback. POSIX sh / busybox ash.
#
#   mm.sh on          switch on: clear a give-up, bind the shim over LG's binary, have systemd start it, remember it
#   mm.sh on --auto   the helper's retry after a fallback: as `on`, but only while switched on and not given up this boot
#   mm.sh off         forget it, have the shim hand back to LG's in place, take LG's binary back
#   mm.sh early       boot step: as `on --auto`; always exit 0
#   mm.sh status      JSON: the marker, the binary, who serves
#
# The shim ($MM/bin/lumatv-mm, helper/mm/lumatv-mm.c) is bound (mount --bind; /usr is read-only) over /usr/sbin/memchute, so systemd's
# memchute.service and ls-hubd's role for that path apply to it unchanged. It is bound only over the binary this was made for (sha256
# pinned below), only when it runs here and would serve (`--would-run`), and kept only when the bus would give it the name (`--probe`,
# run before LG's is restarted). Whenever it cannot serve it unmounts itself and execs LG's in place, so the unit and its pid stay.
# A reboot puts LG's binary back; `early` binds it again from the boot hook while `enabled` is there. `off` deletes `enabled` first.
# An install that brings a new shim while one is bound stages it as lumatv-mm.next (the mount holds the old file) and runs `on --auto`,
# which binds the new one.
set -u

APPDIR=${APPDIR:-$(cd "$(dirname "$0")/.." && pwd)}
if [ -z "${STATE:-}" ]; then
  . "$APPDIR/tools/brand.sh"
  brand_load "$APPDIR/brand.json" 2>/dev/null || { printf '{"ok":false,"error":"brand.json is missing or invalid"}\n'; exit 1; }
  STATE="$BRAND_DATA_DIR"
fi

MM=${MM_DIR:-$STATE/mm}
SHIM="$MM/bin/lumatv-mm"
ENABLED="$MM/enabled"
GAVE_UP="$MM/gave-up"
FAILURES="$MM/failures"
LOCKDIR="$MM/.lock"
LOGFILE=${LOGFILE:-$STATE/log}

BIN=${MM_BIN:-/usr/sbin/memchute}
# webOS 10.2.1's memchute; the shim answers the calls and signals read from this one.
BIN_SHA=${MM_BIN_SHA:-446af3fef4da17b8e98cfeab09170eeceda1b14c2f04da0aacdd347919ef2912}
UNIT=${MM_UNIT:-memchute.service}
SYSTEMCTL=${SYSTEMCTL:-systemctl}
MOUNT=${MOUNT:-mount}
UMOUNT=${UMOUNT:-umount}
KILL=${KILL:-kill}
PROC_MOUNTS=${PROC_MOUNTS:-/proc/mounts}
PROC_ROOT=${PROC_ROOT:-/proc}
BOOT_ID=${BOOT_ID:-/proc/sys/kernel/random/boot_id}
LOCK_WAIT=${MM_LOCK_WAIT:-30}
OFF_WAIT=${MM_OFF_WAIT:-5}

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n\r\t' '   '
}

emit_error() {
  printf '{"ok":false,"error":"%s"}\n' "$(json_escape "$1")"
}

log_line() {
  mkdir -p "$(dirname "$LOGFILE")" 2>/dev/null
  printf '%s mm: %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$(printf '%s' "$1" | tr '\n' ' ')" >> "$LOGFILE" 2>/dev/null
  # The state log is shared by the tool scripts and kept on the TV's flash: only the last 300 lines stay.
  if [ "$(wc -l < "$LOGFILE" 2>/dev/null | tr -d ' ')" -gt 300 ] 2>/dev/null; then
    tail -n 300 "$LOGFILE" > "$LOGFILE.trim.$$" 2>/dev/null && mv "$LOGFILE.trim.$$" "$LOGFILE"
  fi
}

sha_of() {
  sha256sum "$1" 2>/dev/null | cut -d' ' -f1
}

take_lock() {
  mkdir -p "$MM" 2>/dev/null || return 1
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

mounts_on_bin() {
  awk -v p="$BIN" '$2 == p { n++ } END { print n + 0 }' "$PROC_MOUNTS" 2>/dev/null
}

# stock: LG's binary, nothing mounted. ours: exactly one mount and it shows the shim itself. foreign: any other mount on the binary.
# unknown: nothing mounted and not the binary this was made for.
bin_state() {
  bs_n=$(mounts_on_bin)
  if [ "$bs_n" -gt 0 ]; then
    if [ "$bs_n" = 1 ] && [ -f "$SHIM" ] && [ "$BIN" -ef "$SHIM" ]; then echo ours; else echo foreign; fi
    return 0
  fi
  if [ -f "$BIN" ] && [ "$(sha_of "$BIN")" = "$BIN_SHA" ]; then echo stock; else echo unknown; fi
}

gave_up_now() {
  gu_mark=$(head -n 1 "$GAVE_UP" 2>/dev/null)
  [ -n "$gu_mark" ] && [ "$gu_mark" = "$(head -n 1 "$BOOT_ID" 2>/dev/null)" ]
}

unit_pid() {
  "$SYSTEMCTL" show -p MainPID "$UNIT" 2>/dev/null | sed -n 's/^MainPID=//p'
}

# Who answers com.webos.memorymanager: lumatv (the unit's process is the shim), lg, or none.
serving() {
  sv_pid=$(unit_pid)
  case "$sv_pid" in ''|0|*[!0-9]*) echo none; return 0 ;; esac
  if [ -f "$SHIM" ] && [ "$PROC_ROOT/$sv_pid/exe" -ef "$SHIM" ]; then echo lumatv; else echo lg; fi
}

# Binds the shim and has systemd start it. Sets $mm_result (bound, restarted, already) or $mm_err; the caller holds the lock.
apply() {
  mm_err="" mm_result=""
  ap_state=$(bin_state)
  case "$ap_state" in
    foreign) mm_err="another mount is on $BIN; nothing was changed"; return 1 ;;
    unknown) mm_err="$BIN is not the memory manager this was made for (sha256 $(sha_of "$BIN")); nothing was changed"; return 1 ;;
    ours)
      if [ -f "$SHIM.next" ]; then
        # A shim an install staged while this one was bound: bound again below, on the new file.
        "$UMOUNT" "$BIN" 2>/dev/null
        [ "$(mounts_on_bin)" = 0 ] || { mm_err="the old shim could not be unmounted"; return 1; }
      else
        [ "$(serving)" = lumatv ] && { mm_result=already; return 0; }
        # Still bound after a fallback (the shim's unmount stayed in the unit's own mount namespace): a restart runs it again.
        "$SYSTEMCTL" restart "$UNIT" 2>/dev/null || { mm_err="$UNIT did not restart"; return 1; }
        mm_result=restarted
        return 0
      fi ;;
  esac
  if [ -f "$SHIM.next" ]; then
    mv -f "$SHIM.next" "$SHIM" 2>/dev/null || { mm_err="the new shim could not be put in place"; return 1; }
  fi
  [ -f "$SHIM" ] && [ -x "$SHIM" ] || { mm_err="$SHIM is not installed; nothing was changed"; return 1; }
  ap_would=$("$SHIM" --state "$MM" --would-run 2>&1)
  case $? in
    0) ;;
    3) mm_err="the shim would not serve (${ap_would#no: }); nothing was changed"; return 1 ;;
    *) mm_err="the shim cannot run here ($(printf '%s' "$ap_would" | head -n 1)); nothing was changed"; return 1 ;;
  esac
  if ! "$MOUNT" --bind "$SHIM" "$BIN" 2>/dev/null || [ "$(bin_state)" != ours ]; then
    [ "$(bin_state)" = ours ] && "$UMOUNT" "$BIN" 2>/dev/null
    mm_err="the shim could not be bound over $BIN"
    return 1
  fi
  # LG's still serves here: the hub answers the shim's registration with "already exists" when its role allows the name.
  ap_probe=$("$BIN" --state "$MM" --probe 2>&1)
  ap_code=$?
  if [ "$ap_code" != 0 ]; then
    "$UMOUNT" "$BIN" 2>/dev/null
    mm_err="the bus would not give the shim the name (probe exit $ap_code: $(printf '%s' "$ap_probe" | head -n 1)); LG's binary is back"
    [ "$(mounts_on_bin)" = 0 ] || mm_err="$mm_err, but the mount could not be taken back"
    return 1
  fi
  if ! "$SYSTEMCTL" restart "$UNIT" 2>/dev/null; then
    "$UMOUNT" "$BIN" 2>/dev/null
    "$SYSTEMCTL" restart "$UNIT" 2>/dev/null
    mm_err="$UNIT did not restart; LG's binary is back"
    return 1
  fi
  mm_result=bound
  return 0
}

write_enabled() {
  { printf 'enabled\n' > "$ENABLED.tmp" && mv "$ENABLED.tmp" "$ENABLED"; } 2>/dev/null && return 0
  rm -f "$ENABLED.tmp"
  return 1
}

# ---------------------------------------------------------------- verbs

verb_on() {
  take_lock || { emit_error "another mm run is in progress"; return 1; }
  if [ "${1:-}" = --auto ]; then
    [ -f "$ENABLED" ] || { printf '{"ok":true,"mm":"off"}\n'; return 0; }
    if gave_up_now; then
      printf '{"ok":true,"mm":"gave up","why":"three failures this boot"}\n'
      return 0
    fi
    vo_new=false
  else
    # Switching on by hand is the "try again": the failures behind a give-up are forgotten.
    rm -f "$GAVE_UP" "$FAILURES"
    vo_new=true
    [ -f "$ENABLED" ] && vo_new=false
    write_enabled || { emit_error "$ENABLED could not be written"; return 1; }
  fi
  if ! apply; then
    [ "$vo_new" = true ] && rm -f "$ENABLED"
    log_line "on refused: $mm_err"
    emit_error "$mm_err"
    return 1
  fi
  [ "$mm_result" != already ] && log_line "on: $mm_result, $UNIT restarted"
  printf '{"ok":true,"mm":"%s"}\n' "$mm_result"
}

verb_off() {
  if [ ! -d "$LOCKDIR" ] && [ "$(mounts_on_bin)" = 0 ] && [ ! -e "$ENABLED" ] && [ ! -e "$ENABLED.tmp" ]; then
    printf '{"ok":true,"mm":"off","was":"%s"}\n' "$(bin_state)"
    return 0
  fi
  take_lock || { emit_error "another mm run is in progress; off was not applied"; return 1; }
  # Before anything that can fail, so neither a boot nor the helper binds it again.
  rm -f "$ENABLED" "$ENABLED.tmp"
  vf_state=$(bin_state)
  vf_how=""
  case "$vf_state" in
    foreign) log_line "off refused: a mount that is not ours is on $BIN"; emit_error "another mount is on $BIN and was left alone; the switch is off"; return 1 ;;
    ours)
      if [ "$(serving)" = lumatv ]; then
        # The shim unmounts itself and execs LG's in place: the name moves without the unit stopping.
        "$KILL" -USR1 "$(unit_pid)" 2>/dev/null
        vf_i=0
        while [ "$(mounts_on_bin)" != 0 ] && [ "$vf_i" -lt "$OFF_WAIT" ]; do
          sleep 1
          vf_i=$((vf_i + 1))
        done
        vf_how=handed
      fi
      if [ "$(mounts_on_bin)" != 0 ]; then
        "$UMOUNT" -l "$BIN" 2>/dev/null
        if [ "$(mounts_on_bin)" != 0 ]; then
          log_line "off: the shim could not be unmounted"
          emit_error "the shim could not be unmounted; the switch is off and LG's binary is back at the next boot"
          return 1
        fi
        "$SYSTEMCTL" restart "$UNIT" 2>/dev/null
        vf_how=restarted
      fi ;;
  esac
  log_line "off: $vf_state${vf_how:+, $vf_how}"
  printf '{"ok":true,"mm":"off","was":"%s"%s}\n' "$vf_state" "${vf_how:+,\"lg\":\"$vf_how\"}"
}

# The boot hook's step. Always exits 0: a failure is a line in the log, never a held-up boot.
verb_early() {
  if [ ! -f "$ENABLED" ]; then
    printf '{"ok":true,"applied":false,"why":"not switched on"}\n'
    return 0
  fi
  if gave_up_now; then
    printf '{"ok":true,"applied":false,"why":"gave up this boot"}\n'
    return 0
  fi
  LOCK_WAIT=10
  if ! take_lock; then
    log_line "early: another run holds the lock"
    printf '{"ok":true,"applied":false,"why":"another run is in progress"}\n'
    return 0
  fi
  if apply; then
    log_line "early: $mm_result at uptime $(cut -d. -f1 /proc/uptime 2>/dev/null) s"
    printf '{"ok":true,"applied":true,"mm":"%s"}\n' "$mm_result"
  else
    log_line "early: not applied ($mm_err)"
    printf '{"ok":true,"applied":false,"why":"%s"}\n' "$(json_escape "$mm_err")"
  fi
  return 0
}

verb_status() {
  st_enabled=false
  [ -f "$ENABLED" ] && st_enabled=true
  st_gave=false
  gave_up_now && st_gave=true
  st_shim=false
  [ -f "$SHIM" ] && [ -x "$SHIM" ] && st_shim=true
  printf '{"ok":true,"enabled":%s,"bin":"%s","serving":"%s","shim":%s,"gaveUp":%s}\n' \
    "$st_enabled" "$(bin_state)" "$(serving)" "$st_shim" "$st_gave"
}

case "${1:-status}" in
  on) verb_on "${2:-}" ;;
  off) verb_off ;;
  early) verb_early ;;
  status) verb_status ;;
  *) emit_error "usage: mm.sh on [--auto] | off | early | status"; exit 1 ;;
esac
