#!/bin/sh
# On-device LG declutter (POSIX sh / busybox ash only; python3 for JSON).
#
#   declutter.sh on ads|lean              switch a profile on (ads is part of lean)
#   declutter.sh on --group NAME ...      switch single groups on (any group, also opt ones)
#   declutter.sh off [--group NAME ...]   restore the recorded state, all groups or the named
#   declutter.sh status                   one JSON object: profile, groups, memory
#   declutter.sh list                     groups and what each costs
#   declutter.sh boot                     boot hook entry: re-applies what is on, detached
#   declutter.sh early                    early boot hook entry: masks the units of the groups that are on
#   declutter.sh persist on|off           run `boot` from Homebrew Channel's init.d
#   declutter.sh measure [seconds]        memory, resident sizes and CPU over an interval
#   declutter.sh verify                   power state, foreground app, sound output, key services
#
# What it does, and only this:
#   unit     stop the unit and mask it with a symlink in /run/systemd/system.control
#            (tmpfs: gone at reboot, which is why `boot` exists). Not
#            /run/systemd/system: systemd searches /etc/systemd/system before it, so
#            a mask there is ignored for the units LG defines in /etc, and a killed
#            nudge was restarted by Restart=on-failure.
#   luna     kill an on-demand ls-hubd service and bind a do-nothing script over
#            its executable; jailed ones are only killed
#   setting  hold a value in LG's settings service
#   app      remove a server-pushed stub app, take its entries out of the
#            customization lists SAM syncs from, and mark it deleted; its copy
#            goes to $STATE/declutter/backup unless the item says nobackup
#            (then `off` cannot bring it back: the store can)
#   sync     mark every id of LG's country lists that is not installed as
#            deleted, so the wake-time customization sync adds nothing
# Nothing under /usr, /etc or the rootfs is written. SAM, WebAppMgr, the
# compositor and tvpowerd are never touched: names on the deny list below make
# the tool refuse to start. The state before the first change is kept in
# $STATE/declutter/before and `off` puts exactly that back.
#
# DECLUTTER_DRY=1 prints every change instead of making it.
# DECLUTTER_ROOT=DIR prefixes every path the tool reads or writes on the set
# (the host tests run it against a made-up set; empty on the TV).

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

ROOT="${DECLUTTER_ROOT:-}"
SRC="$APPDIR/payload/declutter"
ITEMS="$SRC/items.txt"
GROUPS_FILE="$SRC/groups.txt"
PY="$SRC/dc.py"
DCS="$ROOT$BRAND_DATA_DIR/declutter"
BEFORE="$DCS/before"
ACTIVE="$DCS/active"
PROFILE_FILE="$DCS/profile"
STUB="$DCS/stub"
BACKUP="$DCS/backup"
LOGFILE="$DCS/log"
LOCKDIR="$DCS/.lock"
BOOT_PENDING="$DCS/.boot-pending"
SKIP="$DCS/skip"
# Searched before /etc/systemd/system. RUNSYS_OLD is where earlier versions put
# their masks, which have no effect on units defined in /etc; removed on contact.
RUNSYS=$ROOT/run/systemd/system.control
RUNSYS_OLD=$ROOT/run/systemd/system
INIT_LINK=$ROOT/var/lib/webosbrew/init.d/52-declutter
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
  [ -e "$ROOT/etc/systemd/system/$1" ] || [ -e "$ROOT/lib/systemd/system/$1" ] || [ -e "$ROOT/usr/lib/systemd/system/$1" ]
}

unit_state() {
  systemctl is-active "$1" 2>/dev/null | head -n1
}

unit_masked() {
  [ -L "$RUNSYS/$1" ] && [ "$(readlink "$RUNSYS/$1")" = /dev/null ]
}

# Units stopped and masked in this run; one reload (only when a mask is new)
# and one stop at the end.
UNITS_NEW=""
NEED_RELOAD=0

unit_on() {
  u=$1
  if ! unit_exists "$u"; then
    log_line "unit $u: not present, skipped"
    return 0
  fi
  if ! unit_masked "$u"; then
    record unit "$u" "$(unit_state "$u")"
    mut mkdir -p "$RUNSYS"
    mut ln -sf /dev/null "$RUNSYS/$u"
    [ -L "$RUNSYS_OLD/$u" ] && mut rm -f "$RUNSYS_OLD/$u"
    UNITS_NEW="$UNITS_NEW $u"
    NEED_RELOAD=1
  elif [ "$(unit_state "$u")" = active ]; then
    UNITS_NEW="$UNITS_NEW $u"
  fi
}

units_flush() {
  [ -n "$UNITS_NEW" ] || return 0
  [ "$NEED_RELOAD" = 1 ] && mut systemctl daemon-reload >/dev/null 2>&1
  NEED_RELOAD=0
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
  if [ -n "${left:-}" ]; then
    # nudge and a few others ignore SIGTERM and would take 90 s to die.
    log_line "units still active after 15 s, killed:$left"
    # shellcheck disable=SC2086
    mut systemctl kill -s KILL $left >/dev/null 2>&1
  fi
  UNITS_NEW=""
}

unit_off() {
  u=$1
  prev=$(recorded unit "$u")
  if unit_masked "$u" || [ -L "$RUNSYS_OLD/$u" ]; then
    mut rm -f "$RUNSYS/$u" "$RUNSYS_OLD/$u"
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

# Pids whose /proc/PID/exe ends with one of the given paths (newline-separated);
# one pass over /proc, a jail's prefixed copy of a path matches too.
exe_pids() {
  ls -l /proc/[0-9]*/exe 2>/dev/null | awk -v pats="$1" '
    BEGIN { n = split(pats, p, "\n") }
    $(NF-1) == "->" {
      exe = $NF; split($(NF-2), a, "/")
      for (i = 1; i <= n; i++)
        if (p[i] != "" && length(exe) >= length(p[i]) && substr(exe, length(exe) - length(p[i]) + 1) == p[i]) { print a[3]; break }
    }'
}

kill_exes() {
  pids=$(exe_pids "$1")
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

luna_bind() {
  exe=$1
  if [ ! -f "$exe" ]; then
    log_line "luna $exe: no such file, not stubbed"
    return 0
  fi
  if ! is_bound "$exe"; then
    record luna "$exe" unbound
    mut mount --bind "$STUB" "$exe" || log_line "luna $exe: bind failed"
  fi
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
# The sync that pushed them: at every boot and every wake SAM waits for network,
# the install service and sdx_auth, homelaunchpoints asks sdx for the server's
# country app list (RequestServerInfo), SAM writes it to the customization lists
# and has the install service add every id that is missing and not on the
# deleted list. sdx stays (the store needs it), so the sync runs at every wake;
# the immutable lists and the deleted list are what keep it from adding apps.
# The `app` items remove what the sync (or the app updater) already brought and
# what it would bring back; a `sync` item marks every id of the lists that is
# not installed as deleted, so the sync has nothing left to add.

APP_ROOT=$ROOT/media/cryptofs/apps
CUSTOM_DIR=$ROOT/var/palm/customization/launchpoints
DELETED_LIST=$ROOT/var/preferences/com.webos.applicationManager/deletedSystemAppList.json

app_ids() {
  group_items "$1" | awk -F'|' '$1=="app" {print $3}'
}

# app items with arg nobackup are removed without a copy kept
app_nobackup() {
  group_items "$1" | awk -F'|' -v id="$2" '$1=="app" && $3==id && $4=="nobackup" {f=1} END {exit !f}'
}

group_syncs() {
  group_items "$1" | grep -q "^sync|"
}

# A group that edits the customization lists: app items, a sync item, or both.
group_has_apps() {
  [ -n "$(app_ids "$1")" ] || group_syncs "$1"
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

# The list edits of one group, on files that are writable: its ids out of every
# customization list and onto the deleted list; with a sync item, every id of the
# lists that is not installed onto the deleted list too. Its variables are its
# own (eg, eids): apps_off calls it for other groups while it still needs g and ids.
apps_edit_lists() {
  eg=$1
  eids=$(app_ids "$eg")
  if [ -n "$eids" ]; then
    # shellcheck disable=SC2086
    for f in $(custom_files); do
      if [ "$DRY" = 1 ]; then printf '+ strip %s from %s\n' "$eids" "$f" >&2; else py strip "$f" $eids >/dev/null; fi
    done
    if [ -f "$DELETED_LIST" ]; then
      if [ "$DRY" = 1 ]; then printf '+ deleted-list add %s\n' "$eids" >&2; else py deleted "$DELETED_LIST" $eids >/dev/null; fi
    fi
  fi
  if group_syncs "$eg" && [ -f "$DELETED_LIST" ]; then
    if [ "$DRY" = 1 ]; then printf '+ deleted-list add every id of the lists that is not installed\n' >&2
    else py deleted-missing "$DELETED_LIST" "$APP_ROOT/usr/palm/applications" $(custom_files) >/dev/null; fi
  fi
}

apps_on() {
  g=$1
  group_has_apps "$g" || return 0
  ids=$(app_ids "$g")
  mkdir -p "$BACKUP" 2>/dev/null
  # shellcheck disable=SC2086
  for f in $(custom_files) "$DELETED_LIST"; do
    [ -f "$f" ] || continue
    backup_file "$f"
    mut chattr -i "$f" 2>/dev/null
  done
  for id in $ids; do
    if app_installed "$id" && ! grep -q "^app|$id|" "$BEFORE" 2>/dev/null; then
      if app_nobackup "$g" "$id"; then
        # the owner's choice for a stub: no copy kept, `off` cannot bring it back (the store can)
        record app "$id" removed-no-backup
        continue
      fi
      # only what the app has: a factory stub has no packages dir, and a missing member must not cost the archive
      parts=""
      for p in "usr/palm/applications/$id" "usr/palm/packages/$id" "var/lib/opkg/info/$id.control" "var/lib/opkg/info/$id.list"; do
        [ -e "$APP_ROOT/$p" ] && parts="$parts ${APP_ROOT#"$ROOT"/}/$p"
      done
      # shellcheck disable=SC2086
      tar czf "$BACKUP/app-$id.tgz" -C "${ROOT:-/}" $parts 2>/dev/null
      record app "$id" installed
    fi
  done
  apps_edit_lists "$g"
  # shellcheck disable=SC2086
  for f in $(custom_files); do mut chattr +i "$f" 2>/dev/null; done
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

# The lists go back to what they were before the first group touched them, then
# every other group that is still on edits them again, so switching one group
# off does not undo another's.
apps_off() {
  g=$1
  group_has_apps "$g" || return 0
  ids=$(app_ids "$g")
  grep -h "^file|" "$BEFORE" 2>/dev/null | while IFS='|' read -r k f b; do
    [ -f "$b" ] || continue
    mut chattr -i "$f" 2>/dev/null
    mut cp -p "$b" "$f"
  done
  gone=""
  for id in $ids; do
    if [ ! -f "$BACKUP/app-$id.tgz" ]; then
      [ "$(recorded app "$id")" = removed-no-backup ] && gone="$gone $id"
      continue
    fi
    app_installed "$id" && continue
    mut tar xzf "$BACKUP/app-$id.tgz" -C "${ROOT:-/}"
  done
  for o in $(cat "$ACTIVE" 2>/dev/null); do
    [ "$o" = "$g" ] && continue
    group_has_apps "$o" || continue
    apps_edit_lists "$o"
    # shellcheck disable=SC2086
    for f in $(custom_files); do mut chattr +i "$f" 2>/dev/null; done
  done
  [ -n "$ids" ] && log_line "apps restored from backup; SAM sees them at the next start (reboot)"
  [ -n "$gone" ] && log_line "removed without a copy, not restored (the store can install them):$gone"
  return 0
}

# ---------------------------------------------------------------- groups

# One item line -> K (kind), N (name), A (arg).
split_item() {
  K=${1%%|*}
  rest=${1#*|}; rest=${rest#*|}
  N=${rest%%|*}; rest=${rest#*|}
  A=${rest%%|*}
}

# Phase one of switching a group on: queue its units, hold its settings.
group_prepare() {
  g=$1
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
}

# Phase two, after the units were stopped: the on-demand services and the apps.
group_finish() {
  g=$1
  items=$(group_items "$g")
  pats=""
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    split_item "$line"
    [ "$K" = luna ] && pats="$pats$N
"
  done <<EOF
$items
EOF
  if [ -n "$pats" ]; then
    kill_exes "$pats"
    while IFS= read -r line; do
      [ -n "$line" ] || continue
      split_item "$line"
      [ "$K" = luna ] && [ "$A" = stub ] && luna_bind "$N"
    done <<EOF
$items
EOF
    kill_exes "$pats"
  fi
  apps_on "$g"
  mark_active "$g"
  log_line "group $g on"
}

# Several groups with one daemon-reload and one stop for all their units.
groups_on() {
  for g in "$@"; do
    group_known "$g" || { emit_error "unknown group: $g"; return 1; }
  done
  UNITS_NEW=""
  NEED_RELOAD=0
  for g in "$@"; do
    group_prepare "$g"
  done
  units_flush
  for g in "$@"; do
    group_finish "$g"
  done
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
  apps_off "$g"
  unmark_active "$g"
  log_line "group $g off"
  return 0
}

# ---------------------------------------------------------------- verbs

exe_running() {
  [ -n "$(exe_pids "$1")" ]
}

state_json() {
  # How many items of a group are in their target state: done/total.
  g=$1
  total=0
  done_=0
  apps=""
  group_has_apps "$g" && apps=$(py apps 2>/dev/null)
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
      app) printf '%s\n' "$apps" | grep -qxF "$N" || app_installed "$N" || done_=$((done_ + 1)) ;;
      sync) [ "$(py deleted-missing --check "$DELETED_LIST" "$APP_ROOT/usr/palm/applications" $(custom_files) 2>/dev/null)" = 0 ] && done_=$((done_ + 1)) ;;
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
  # shellcheck disable=SC2086
  groups_on $groups || return 1
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
    rm -rf "$BEFORE" "$ACTIVE" "$PROFILE_FILE" "$BACKUP"
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
  # shellcheck disable=SC2046
  groups_on $(cat "$ACTIVE" 2>/dev/null) >/dev/null 2>&1
  release_lock
}

boot_settled() {
  # The TV answers and the services we must not disturb are up.
  [ "$(systemctl is-active sam.service 2>/dev/null)" = active ] && \
    [ "$(systemctl is-active webapp-mgr.service 2>/dev/null)" = active ] && \
    timeout 10 luna-send -n 1 luna://com.webos.service.tvpower/power/getPowerState '{}' 2>/dev/null | grep -q '"returnValue"[^,]*true'
}

boot_worker() {
  # Re-applies every group that is on. Units that LG starts after this point
  # are masked before they start; the ones that came up first are stopped, and
  # two more passes later in the boot catch what a dependency or a retry started.
  # The marker covers only the first pass: if the set answers 15 s after it,
  # whatever this tool did has not broken the boot, and the marker goes.
  boot_pass
  log_line "boot: first pass done"
  n=0
  while [ "$n" -lt 12 ]; do
    sleep 15
    if boot_settled; then
      rm -f "$BOOT_PENDING"
      log_line "boot: settled"
      break
    fi
    n=$((n + 1))
  done
  [ -f "$BOOT_PENDING" ] && log_line "boot: the set did not settle; the next boot skips declutter once"
  for wait_s in 5 25; do
    sleep "$wait_s"
    boot_pass
  done
  log_line "boot: later passes done"
  return 0
}

# Called from the early boot hook (about 14 s after power-on), before the targets
# that pull nudge, user-context-manager, uploadd and the rest of the late units
# (webos-rbd.target at about +22 s, webos-bd.target at about +30 s): only lays the
# masks down and reloads systemd once, so those units never start. It stops
# nothing and needs neither luna-send nor the settings service; the passes of
# `boot` do the rest. Silent and fast; honours the skip file and the marker of a
# boot that never completed (which `boot` itself consumes).
verb_early() {
  [ -s "$ACTIVE" ] || exit 0
  [ -f "$SKIP" ] && exit 0
  [ -f "$BOOT_PENDING" ] && exit 0
  refuse_denied >/dev/null || exit 0
  take_lock || exit 0
  n=0
  for g in $(cat "$ACTIVE" 2>/dev/null); do
    for u in $(group_items "$g" | awk -F'|' '$1=="unit" {print $3}'); do
      unit_exists "$u" || continue
      unit_masked "$u" && continue
      mkdir -p "$RUNSYS"
      ln -sf /dev/null "$RUNSYS/$u" && n=$((n + 1))
    done
  done
  [ "$n" -gt 0 ] && systemctl daemon-reload >/dev/null 2>&1
  log_line "early: $n unit(s) masked at uptime $(cut -d. -f1 /proc/uptime) s"
  release_lock
  exit 0
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
  early)    verb_early ;;
  --boot-worker) boot_worker ;;
  persist)  verb_persist "$@" ;;
  measure)  verb_measure "$@" ;;
  verify)   verb_verify ;;
  *)        emit_error "usage: declutter.sh on|off|status|list|boot|early|persist|measure|verify"; exit 1 ;;
esac
