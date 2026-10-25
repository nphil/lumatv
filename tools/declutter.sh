#!/bin/sh
# On-device LG declutter (POSIX sh / busybox ash only; python3 for JSON).
#
#   declutter.sh on ads|lean              switch a profile on (ads is part of lean)
#   declutter.sh on --group NAME ...      switch single groups on (any group, also opt ones)
#   declutter.sh off [--group NAME ...]   restore the recorded state, all groups or the named
#   declutter.sh status                   one JSON object: profile, groups, memory
#   declutter.sh list                     groups and what each costs
#   declutter.sh boot                     boot hook entry: re-applies what is on, detached
#   declutter.sh persist on|off           run `boot` from Homebrew Channel's init.d
#   declutter.sh measure [seconds]        memory, resident sizes and CPU over an interval
#   declutter.sh verify                   power state, foreground app, sound output, key services
#
# What it does, and only this:
#   unit     stop the unit and mask it with a symlink in /run/systemd/system
#            (tmpfs: gone at reboot, which is why `boot` exists)
#   luna     kill an on-demand ls-hubd service and bind a do-nothing script over
#            its executable; jailed ones are only killed
#   setting  hold a value in LG's settings service
#   app      remove a server-pushed stub app, take its entries out of the
#            customization lists SAM syncs from, and mark it deleted
# Nothing under /usr, /etc or the rootfs is written. SAM, WebAppMgr, the
# compositor and tvpowerd are never touched: names on the deny list below make
# the tool refuse to start. The state before the first change is kept in
# $STATE/declutter/before and `off` puts exactly that back.
#
# DECLUTTER_DRY=1 prints every change instead of making it.

set -u

SELF=$0
if [ -L "$SELF" ]; then
  t=$(readlink "$SELF" 2>/dev/null)
  case "$t" in /*) SELF=$t ;; *) SELF="$(dirname "$SELF")/$t" ;; esac
fi
APPDIR=$(cd "$(dirname "$SELF")/.." && pwd)
if [ ! -f "$APPDIR/tools/brand.sh" ]; then
  printf '{"ok":false,"error":"tools/brand.sh is missing"}\n'
  exit 1
fi
. "$APPDIR/tools/brand.sh"
brand_load "$APPDIR/brand.json" 2>/dev/null || {
  printf '{"ok":false,"error":"brand.json is missing or invalid"}\n'
  exit 1
}

SRC="$APPDIR/payload/declutter"
ITEMS="$SRC/items.txt"
GROUPS_FILE="$SRC/groups.txt"
PY="$SRC/dc.py"
DCS="$BRAND_DATA_DIR/declutter"
BEFORE="$DCS/before"
ACTIVE="$DCS/active"
PROFILE_FILE="$DCS/profile"
STUB="$DCS/stub"
BACKUP="$DCS/backup"
LOGFILE="$DCS/log"
LOCKDIR="$DCS/.lock"
BOOT_PENDING="$DCS/.boot-pending"
SKIP="$DCS/skip"
RUNSYS=/run/systemd/system
INIT_LINK=/var/lib/webosbrew/init.d/52-declutter
DRY="${DECLUTTER_DRY:-0}"

# Units this tool must never stop, whatever items.txt says.
DENY="sam.service webapp-mgr.service surface-manager.service surface-manager-daemon.service
tvpowerd.service ls-hubd.service configd.service dbus.service activitymanager.service
settings-service.service appinstalld.service pkginstalld.service luna-sys-service.service
bootd.service sleepd.service audiod.service audiooutputd.service videooutputd.service
eim.service arccontroller.service extinput-integration.service lginput2.service
panelcontroller.service pqcontroller.service eplmanager.service micomservice.service
fancontroller.service emmcd.service connman.service wpa-supplicant.service
webos-connman-adapter.service xinetd.service tvservicemock.service umediaserver.service
tvrm.service kadpd.service alwaysready.service homelaunchpoints.service
physical-device-manager.service webos-bluetooth-service.service bthidmanager.service
db8.service db8-maindb.service db8-accountdb.service db8-tempdb.service db8-mediadb.service
memchute.service preload-manager.service luna-packagemanager.service securitymanager.service
second-screen-gateway.service asc.service avahi-daemon.service mdnsd.service
ssdp-discovery-lgtv.service pacrunner.service certupdmgr.service certupdmgr.timer"

# ---------------------------------------------------------------- utilities

json_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' | tr '\n\r\t' '   '
}

emit_error() {
  printf '{"ok":false,"error":"%s"}\n' "$(json_escape "$1")"
}

log_line() {
  mkdir -p "$DCS" 2>/dev/null
  printf '%s %s\n' "$(date '+%Y-%m-%d %H:%M:%S' 2>/dev/null)" "$(printf '%s' "$1" | tr '\n' ' ')" >> "$LOGFILE" 2>/dev/null
  if [ -f "$LOGFILE" ] && [ "$(wc -l < "$LOGFILE" 2>/dev/null | tr -d ' ')" -gt 300 ] 2>/dev/null; then
    tail -n 300 "$LOGFILE" > "$LOGFILE.trim.$$" 2>/dev/null && mv "$LOGFILE.trim.$$" "$LOGFILE"
  fi
}

# A change. In a dry run it is only printed.
mut() {
  if [ "$DRY" = 1 ]; then
    printf '+ %s\n' "$*" >&2
    return 0
  fi
  "$@"
}

take_lock() {
  mkdir -p "$DCS" 2>/dev/null
  i=0
  while ! mkdir "$LOCKDIR" 2>/dev/null; do
    # A lock older than ten minutes belongs to a run that died.
    if [ -d "$LOCKDIR" ] && [ -n "$(find "$LOCKDIR" -maxdepth 0 -mmin +10 2>/dev/null)" ]; then
      rmdir "$LOCKDIR" 2>/dev/null
      continue
    fi
    i=$((i + 1))
    [ "$i" -gt 60 ] && return 1
    sleep 1
  done
  trap 'rmdir "$LOCKDIR" 2>/dev/null' EXIT
  trap 'exit 1' HUP INT TERM
  return 0
}

release_lock() {
  rmdir "$LOCKDIR" 2>/dev/null
  trap - EXIT
}

py() {
  python3 "$PY" "$@"
}

# before-state: one line per item, written once, never overwritten
record() {
  [ "$DRY" = 1 ] && return 0
  mkdir -p "$DCS" 2>/dev/null
  grep -qxF "$1|$2|$3" "$BEFORE" 2>/dev/null && return 0
  grep -q "^$1|$2|" "$BEFORE" 2>/dev/null && return 0
  printf '%s|%s|%s\n' "$1" "$2" "$3" >> "$BEFORE"
}

recorded() {
  sed -n "s/^$1|$2|\\(.*\\)\$/\\1/p" "$BEFORE" 2>/dev/null | head -n1
}

group_items() {
  grep -h "^[a-z]*|$1|" "$ITEMS" 2>/dev/null
}

group_known() {
  grep -q "^$1|" "$GROUPS_FILE" 2>/dev/null
}

groups_of_profile() {
  case "$1" in
    ads)  awk -F'|' '!/^#/ && NF && $2=="ads" {print $1}' "$GROUPS_FILE" ;;
    lean) awk -F'|' '!/^#/ && NF && ($2=="ads" || $2=="lean") {print $1}' "$GROUPS_FILE" ;;
  esac
}

group_active() {
  grep -qxF "$1" "$ACTIVE" 2>/dev/null
}

mark_active() {
  [ "$DRY" = 1 ] && return 0
  group_active "$1" || printf '%s\n' "$1" >> "$ACTIVE"
}

unmark_active() {
  [ "$DRY" = 1 ] && return 0
  [ -f "$ACTIVE" ] || return 0
  grep -vxF "$1" "$ACTIVE" > "$ACTIVE.tmp" 2>/dev/null
  mv "$ACTIVE.tmp" "$ACTIVE"
  [ -s "$ACTIVE" ] || rm -f "$ACTIVE" "$PROFILE_FILE"
}

refuse_denied() {
  while IFS='|' read -r kind group name arg ev; do
    [ "$kind" = unit ] || continue
    for d in $DENY; do
      [ "$d" = "$name" ] && { printf '%s\n' "$name"; return 1; }
    done
  done < "$ITEMS"
  return 0
}

# ---------------------------------------------------------------- units

unit_exists() {
  [ -e "/etc/systemd/system/$1" ] || [ -e "/lib/systemd/system/$1" ] || [ -e "/usr/lib/systemd/system/$1" ]
}

unit_state() {
  systemctl is-active "$1" 2>/dev/null | head -n1
}

unit_masked() {
  [ -L "$RUNSYS/$1" ] && [ "$(readlink "$RUNSYS/$1")" = /dev/null ]
}

# Units stopped and masked in this run; one reload and one stop at the end.
UNITS_NEW=""

unit_on() {
  u=$1
  if ! unit_exists "$u"; then
    log_line "unit $u: not present, skipped"
    return 0
  fi
  if ! unit_masked "$u"; then
    record unit "$u" "$(unit_state "$u")"
    mut ln -sf /dev/null "$RUNSYS/$u"
    UNITS_NEW="$UNITS_NEW $u"
  elif [ "$(unit_state "$u")" = active ]; then
    UNITS_NEW="$UNITS_NEW $u"
  fi
}

units_flush() {
  [ -n "$UNITS_NEW" ] || return 0
  mut systemctl daemon-reload >/dev/null 2>&1
  # shellcheck disable=SC2086
  mut systemctl stop --no-block $UNITS_NEW >/dev/null 2>&1
  [ "$DRY" = 1 ] && { UNITS_NEW=""; return 0; }
  n=0
  while [ "$n" -lt 15 ]; do
    left=""
    for u in $UNITS_NEW; do
      case "$(unit_state "$u")" in active|deactivating) left="$left $u" ;; esac
    done
    [ -z "$left" ] && break
    n=$((n + 1))
    sleep 1
  done
  [ -n "${left:-}" ] && log_line "units still active after 15 s:$left"
  UNITS_NEW=""
}

unit_off() {
  u=$1
  prev=$(recorded unit "$u")
  if unit_masked "$u"; then
    mut rm -f "$RUNSYS/$u"
    UNITS_NEW="$UNITS_NEW $u"
    UNITS_START="$UNITS_START $u:$prev"
  fi
}

UNITS_START=""

units_restore() {
  [ -n "$UNITS_NEW" ] || return 0
  mut systemctl daemon-reload >/dev/null 2>&1
  for e in $UNITS_START; do
    u=${e%%:*}
    [ "${e#*:}" = active ] && mut systemctl start --no-block "$u" >/dev/null 2>&1
  done
  UNITS_NEW=""
  UNITS_START=""
}

# ---------------------------------------------------------------- luna services

kill_exe() {
  # $1 = executable path; matches a jail's prefixed copy of it too.
  want=$1
  pids=""
  for d in /proc/[0-9]*; do
    e=$(readlink "$d/exe" 2>/dev/null) || continue
    case "$e" in *"$want") pids="$pids ${d#/proc/}" ;; esac
  done
  [ -n "$pids" ] || return 0
  # shellcheck disable=SC2086
  mut kill $pids 2>/dev/null
  [ "$DRY" = 1 ] && return 0
  sleep 1
  for p in $pids; do
    [ -d "/proc/$p" ] && kill -9 "$p" 2>/dev/null
  done
  return 0
}

is_bound() {
  grep -qF " $1 " /proc/self/mounts 2>/dev/null
}

luna_on() {
  exe=$1 mode=$2
  kill_exe "$exe"
  [ "$mode" = stub ] || return 0
  if [ ! -f "$exe" ]; then
    log_line "luna $exe: no such file, not stubbed"
    return 0
  fi
  if ! is_bound "$exe"; then
    record luna "$exe" unbound
    mut mount --bind "$STUB" "$exe" || log_line "luna $exe: bind failed"
  fi
  kill_exe "$exe"
}

luna_off() {
  exe=$1
  while is_bound "$exe"; do
    mut umount "$exe" 2>/dev/null || break
    [ "$DRY" = 1 ] && break
  done
}

# ---------------------------------------------------------------- settings

setting_on() {
  ck=$1 want=$2
  cur=$(py get "$ck")
  if [ "$cur" = "__missing__" ] || [ -z "$cur" ]; then
    log_line "setting $ck: not readable, skipped"
    return 0
  fi
  record setting "$ck" "$cur"
  if [ "$cur" != "$want" ]; then
    r=$(mut py set "$ck" "$want")
    [ "$DRY" = 1 ] || case "$r" in ok) : ;; *) log_line "setting $ck: $r" ;; esac
  fi
}

setting_off() {
  ck=$1
  prev=$(recorded setting "$ck")
  [ -n "$prev" ] || return 0
  cur=$(py get "$ck")
  [ "$cur" = "$prev" ] && return 0
  r=$(mut py set "$ck" "$prev")
  [ "$DRY" = 1 ] || case "$r" in ok) : ;; *) log_line "setting $ck restore: $r" ;; esac
}

# ---------------------------------------------------------------- apps
# The sync that pushed them: at every boot SAM waits for network, the install
# service and sdx_auth, then asks contentminer for the server's country app list
# (RequestServerInfo), writes it to the customization lists and has the install
# service add every id that is missing and not on the deleted list. sdx and
# contentminer are in the push and ads groups; this group removes what the sync
# already brought and what it would bring back.

APP_ROOT=/media/cryptofs/apps
CUSTOM_DIR=/var/palm/customization/launchpoints
DELETED_LIST=/var/preferences/com.webos.applicationManager/deletedSystemAppList.json

app_ids() {
  group_items "$1" | awk -F'|' '$1=="app" {print $3}'
}

app_installed() {
  [ -d "$APP_ROOT/usr/palm/applications/$1" ]
}

custom_files() {
  for f in "$CUSTOM_DIR"/*/applist.json; do
    [ -f "$f" ] && printf '%s\n' "$f"
  done
}

backup_file() {
  # $1 = file; copied once, remembered as file|path|backup
  grep -q "^file|$1|" "$BEFORE" 2>/dev/null && return 0
  key=$(printf '%s' "$1" | tr '/' '_')
  mkdir -p "$BACKUP" 2>/dev/null
  mut cp -p "$1" "$BACKUP/$key" || return 1
  record file "$1" "$BACKUP/$key"
}

apps_on() {
  g=$1
  ids=$(app_ids "$g")
  [ -n "$ids" ] || return 0
  mkdir -p "$BACKUP" 2>/dev/null
  # shellcheck disable=SC2086
  for f in $(custom_files) "$DELETED_LIST"; do
    [ -f "$f" ] || continue
    backup_file "$f"
    mut chattr -i "$f" 2>/dev/null
  done
  for id in $ids; do
    if app_installed "$id" && ! grep -q "^app|$id|" "$BEFORE" 2>/dev/null; then
      tar czf "$BACKUP/app-$id.tgz" -C / \
        "${APP_ROOT#/}/usr/palm/applications/$id" \
        "${APP_ROOT#/}/usr/palm/packages/$id" \
        "${APP_ROOT#/}/var/lib/opkg/info/$id.control" \
        "${APP_ROOT#/}/var/lib/opkg/info/$id.list" 2>/dev/null
      record app "$id" installed
    fi
  done
  # shellcheck disable=SC2086
  for f in $(custom_files); do
    [ "$DRY" = 1 ] && { printf '+ strip %s from %s\n' "$ids" "$f" >&2; continue; }
    py strip "$f" $ids >/dev/null
    chattr +i "$f" 2>/dev/null
  done
  if [ -f "$DELETED_LIST" ]; then
    if [ "$DRY" = 1 ]; then
      printf '+ deleted-list add %s\n' "$ids" >&2
    else
      py deleted "$DELETED_LIST" $ids >/dev/null
    fi
  fi
  for id in $ids; do
    app_installed "$id" || continue
    mut luna-send -n 1 luna://com.webos.appInstallService/remove "{\"id\":\"$id\",\"subscribe\":false}" >/dev/null 2>&1
  done
  [ "$DRY" = 1 ] && return 0
  n=0
  while [ "$n" -lt 20 ]; do
    left=""
    for id in $ids; do app_installed "$id" && left="$left $id"; done
    [ -z "$left" ] && break
    n=$((n + 1))
    sleep 1
  done
  [ -n "${left:-}" ] && log_line "apps still installed after 20 s:$left"
  return 0
}

apps_off() {
  g=$1
  ids=$(app_ids "$g")
  [ -n "$ids" ] || return 0
  grep -h "^file|" "$BEFORE" 2>/dev/null | while IFS='|' read -r k f b; do
    [ -f "$b" ] || continue
    mut chattr -i "$f" 2>/dev/null
    mut cp -p "$b" "$f"
  done
  for id in $ids; do
    [ -f "$BACKUP/app-$id.tgz" ] || continue
    app_installed "$id" && continue
    mut tar xzf "$BACKUP/app-$id.tgz" -C /
  done
  log_line "apps restored from backup; SAM sees them at the next start (reboot)"
}

# ---------------------------------------------------------------- groups

# One item line -> K (kind), N (name), A (arg).
split_item() {
  K=${1%%|*}
  rest=${1#*|}; rest=${rest#*|}
  N=${rest%%|*}; rest=${rest#*|}
  A=${rest%%|*}
}

group_on() {
  g=$1
  group_known "$g" || { emit_error "unknown group: $g"; return 1; }
  UNITS_NEW=""
  items=$(group_items "$g")
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    split_item "$line"
    case "$K" in
      setting) setting_on "$N" "$A" ;;
      unit) unit_on "$N" ;;
    esac
  done <<EOF
$items
EOF
  units_flush
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    split_item "$line"
    [ "$K" = luna ] && luna_on "$N" "$A"
  done <<EOF
$items
EOF
  case "$g" in apps) apps_on "$g" ;; esac
  mark_active "$g"
  log_line "group $g on"
  return 0
}

group_off() {
  g=$1
  group_known "$g" || { emit_error "unknown group: $g"; return 1; }
  UNITS_NEW=""
  UNITS_START=""
  items=$(group_items "$g")
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    split_item "$line"
    case "$K" in
      luna) luna_off "$N" ;;
      setting) setting_off "$N" ;;
      unit) unit_off "$N" ;;
    esac
  done <<EOF
$items
EOF
  units_restore
  case "$g" in apps) apps_off "$g" ;; esac
  unmark_active "$g"
  log_line "group $g off"
  return 0
}

# ---------------------------------------------------------------- verbs

exe_running() {
  for d in /proc/[0-9]*; do
    e=$(readlink "$d/exe" 2>/dev/null) || continue
    case "$e" in *"$1") return 0 ;; esac
  done
  return 1
}

state_json() {
  # How many items of a group are in their target state: done/total.
  g=$1
  total=0
  done_=0
  apps=$(py apps 2>/dev/null)
  items=$(group_items "$g")
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    split_item "$line"
    total=$((total + 1))
    case "$K" in
      unit) unit_masked "$N" && [ "$(unit_state "$N")" != active ] && done_=$((done_ + 1)) ;;
      luna)
        if [ "$A" = stub ]; then
          is_bound "$N" && done_=$((done_ + 1))
        else
          exe_running "$N" || done_=$((done_ + 1))
        fi ;;
      setting) [ "$(py get "$N")" = "$A" ] && done_=$((done_ + 1)) ;;
      app) printf '%s\n' "$apps" | grep -qxF "$N" || done_=$((done_ + 1)) ;;
    esac
  done <<EOF
$items
EOF
  printf '%s/%s' "$done_" "$total"
}

verb_status() {
  [ -f "$ITEMS" ] || { emit_error "items.txt is missing"; return 1; }
  out=""
  for g in $(awk -F'|' '!/^#/ && NF {print $1}' "$GROUPS_FILE"); do
    on=false; group_active "$g" && on=true
    out="$out${out:+,}\"$g\":{\"on\":$on,\"inTarget\":\"$(state_json "$g")\"}"
  done
  prof=$(cat "$PROFILE_FILE" 2>/dev/null)
  printf '{"ok":true,"profile":"%s","groups":{%s},"persist":%s,"skip":%s,"memAvailableKb":%s}\n' \
    "${prof:-off}" "$out" "$([ -L "$INIT_LINK" ] && echo true || echo false)" \
    "$([ -f "$SKIP" ] && echo true || echo false)" \
    "$(sed -n 's/^MemAvailable: *\([0-9]*\).*/\1/p' /proc/meminfo)"
}

verb_list() {
  awk -F'|' '!/^#/ && NF {printf "%-13s %-5s %s | %s\n", $1, $2, $3, $4}' "$GROUPS_FILE"
}

verb_on() {
  [ -f "$ITEMS" ] || { emit_error "items.txt is missing"; return 1; }
  bad=$(refuse_denied) || { emit_error "items.txt names a protected unit: $bad"; return 1; }
  [ -f "$STUB" ] || { mkdir -p "$DCS"; mut cp "$SRC/stub.sh" "$STUB"; mut chmod 755 "$STUB"; }
  groups="" profile=""
  case "${1:-}" in
    ads|lean) profile=$1; groups=$(groups_of_profile "$1"); shift ;;
  esac
  while [ $# -gt 0 ]; do
    case "$1" in
      --group) shift; groups="$groups $1" ;;
      *) emit_error "usage: on ads|lean [--group NAME]..."; return 1 ;;
    esac
    shift
  done
  [ -n "$groups" ] || { emit_error "usage: on ads|lean | on --group NAME"; return 1; }
  take_lock || { emit_error "another run holds the lock"; return 1; }
  for g in $groups; do
    group_on "$g" || return 1
  done
  if [ -n "$profile" ]; then
    [ "$DRY" = 1 ] || printf '%s\n' "$profile" > "$PROFILE_FILE"
  elif [ ! -f "$PROFILE_FILE" ] && [ "$DRY" != 1 ]; then
    printf 'custom\n' > "$PROFILE_FILE"
  fi
  printf '{"ok":true,"on":"%s"}\n' "$(json_escape "$(echo $groups)")"
}

verb_off() {
  groups=""
  while [ $# -gt 0 ]; do
    case "$1" in
      --group) shift; groups="$groups $1" ;;
      *) emit_error "usage: off [--group NAME]..."; return 1 ;;
    esac
    shift
  done
  [ -n "$groups" ] || groups=$(cat "$ACTIVE" 2>/dev/null)
  take_lock || { emit_error "another run holds the lock"; return 1; }
  for g in $groups; do
    group_off "$g" || return 1
  done
  if [ ! -s "$ACTIVE" ] && [ "$DRY" != 1 ]; then
    rm -f "$BEFORE" "$ACTIVE" "$PROFILE_FILE"
  fi
  printf '{"ok":true,"off":"%s"}\n' "$(json_escape "$(echo $groups)")"
}

verb_measure() {
  python3 "$PY" measure "${1:-30}"
}

verb_verify() {
  ps=$(timeout 10 luna-send -n 1 luna://com.webos.service.tvpower/power/getPowerState '{}' 2>/dev/null | sed -n 's/.*"state"[^"]*"\([^"]*\)".*/\1/p' | head -n1)
  fg=$(timeout 10 luna-send -n 1 luna://com.webos.applicationManager/getForegroundAppInfo '{}' 2>/dev/null | sed -n 's/.*"appId"[^"]*"\([^"]*\)".*/\1/p' | head -n1)
  so=$(timeout 10 luna-send -n 1 luna://com.webos.audio/getSoundOut '{}' 2>/dev/null | sed -n 's/.*"soundOutput"[^"]*"\([^"]*\)".*/\1/p' | head -n1)
  sam=$(systemctl is-active sam.service 2>/dev/null | head -n1)
  wam=$(systemctl is-active webapp-mgr.service 2>/dev/null | head -n1)
  printf '{"ok":true,"power":"%s","foreground":"%s","soundOutput":"%s","sam":"%s","webappMgr":"%s","uptime":%s}\n' \
    "$ps" "$fg" "$so" "$sam" "$wam" "$(cut -d. -f1 /proc/uptime)"
}

# ---------------------------------------------------------------- boot

boot_pass() {
  take_lock || return 1
  for g in $(cat "$ACTIVE" 2>/dev/null); do
    group_on "$g" >/dev/null 2>&1
  done
  release_lock
}

boot_worker() {
  # Re-applies every group that is on. Units that LG starts after this point
  # are masked before they start; the ones that came up first are stopped, and
  # the passes later in the boot catch what a dependency or a retry started.
  boot_pass
  log_line "boot: first pass done"
  for wait_s in 20 40; do
    sleep "$wait_s"
    boot_pass
  done
  # Boot counts as complete when the TV answers and the services we must not
  # disturb are up; otherwise the marker stays and the next boot skips us once.
  n=0
  while [ "$n" -lt 12 ]; do
    if [ "$(systemctl is-active sam.service 2>/dev/null)" = active ] && \
       [ "$(systemctl is-active webapp-mgr.service 2>/dev/null)" = active ] && \
       timeout 10 luna-send -n 1 luna://com.webos.service.tvpower/power/getPowerState '{}' 2>/dev/null | grep -q '"returnValue"[^,]*true'; then
      rm -f "$BOOT_PENDING"
      log_line "boot: complete"
      return 0
    fi
    n=$((n + 1))
    sleep 10
  done
  log_line "boot: the set did not settle; the next boot skips declutter once"
  return 0
}

verb_boot() {
  [ -s "$ACTIVE" ] || exit 0
  [ -f "$SKIP" ] && { log_line "boot: skip file present, nothing done"; exit 0; }
  if [ -f "$BOOT_PENDING" ]; then
    rm -f "$BOOT_PENDING"
    log_line "boot: the previous boot never completed, skipping this one"
    exit 0
  fi
  [ -f "$STUB" ] || { mkdir -p "$DCS"; cp "$SRC/stub.sh" "$STUB"; chmod 755 "$STUB"; }
  : > "$BOOT_PENDING"
  setsid "$SELF" --boot-worker </dev/null >/dev/null 2>&1 &
  exit 0
}

verb_persist() {
  case "${1:-}" in
    on)
      mkdir -p /var/lib/webosbrew/init.d 2>/dev/null
      mut ln -sf "$SELF" "$INIT_LINK"
      printf '{"ok":true,"persist":true}\n' ;;
    off)
      mut rm -f "$INIT_LINK"
      printf '{"ok":true,"persist":false}\n' ;;
    *) emit_error "usage: persist on|off"; return 1 ;;
  esac
}

# An init.d hook is run with no arguments through the link.
if [ $# -eq 0 ] && [ "$0" != "$SELF" ]; then
  set -- boot
fi

VERB="${1:-status}"
[ $# -gt 0 ] && shift

case "$VERB" in
  on)       verb_on "$@" ;;
  off)      verb_off "$@" ;;
  status)   verb_status ;;
  list)     verb_list ;;
  boot)     verb_boot ;;
  --boot-worker) boot_worker ;;
  persist)  verb_persist "$@" ;;
  measure)  verb_measure "$@" ;;
  verify)   verb_verify ;;
  *)        emit_error "usage: declutter.sh on|off|status|list|boot|persist|measure|verify"; exit 1 ;;
esac
