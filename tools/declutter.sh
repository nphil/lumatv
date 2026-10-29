#!/bin/sh
# On-device LG declutter (POSIX sh / busybox ash only; python3 for JSON).
#
#   declutter.sh on ads|lean              switch a profile on (ads is part of lean)
#   declutter.sh on --group NAME ...      switch single groups on (any group, also opt ones)
#   declutter.sh off [--group NAME ...]   restore the recorded state, all groups or the named
#   declutter.sh status                   one JSON object: profile, groups, memory
#   declutter.sh list                     groups and what each costs
#   declutter.sh boot                     boot hook entry: re-applies what is on, detached
#   declutter.sh early                    early boot step: masks the units and binds the stubs of the groups that are on, before LG starts them
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
EARLY_PENDING="$DCS/.early-pending"
EARLY_RECORD="$DCS/early.json"
# Searched before /etc/systemd/system. RUNSYS_OLD is where earlier versions put
# their masks, which have no effect on units defined in /etc; removed on contact.
RUNSYS=$ROOT/run/systemd/system.control
RUNSYS_OLD=$ROOT/run/systemd/system
INIT_LINK=$ROOT/var/lib/webosbrew/init.d/52-declutter
DRY="${DECLUTTER_DRY:-0}"
# Where is_bound looks, and the pause of the early settle watch; the host tests point the first at a file their
# stand-in `mount` writes and shorten the second.
MOUNTS="${DECLUTTER_MOUNTS:-/proc/self/mounts}"
SETTLE_STEP="${DECLUTTER_SETTLE_STEP:-15}"

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

# A JSON array of the names in $1, separated by spaces or newlines.
json_list() {
  jl=""
  for jn in $1; do
    jl="$jl${jl:+,}\"$(json_escape "$jn")\""
  done
  printf '[%s]' "$jl"
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

# Pids whose /proc/PID/exe ends with one of the given paths (newline-separated),
# or, with $2 = name, those paths themselves; one pass over /proc, a jail's
# prefixed copy of a path matches too.
exe_pids() {
  ls -l /proc/[0-9]*/exe 2>/dev/null | awk -v pats="$1" -v what="${2:-pid}" '
    BEGIN { n = split(pats, p, "\n") }
    $(NF-1) == "->" {
      exe = $NF; split($(NF-2), a, "/")
      for (i = 1; i <= n; i++)
        if (p[i] != "" && length(exe) >= length(p[i]) && substr(exe, length(exe) - length(p[i]) + 1) == p[i]) { print (what == "name" ? p[i] : a[3]); break }
    }'
}

kill_exes() {
  pids=$(exe_pids "$1")
  [ -n "$pids" ] || return 0
  # shellcheck disable=SC2046
  [ "$DRY" = 1 ] || log_line "killed a running copy, pid $(echo $pids), at uptime $(cut -d. -f1 /proc/uptime) s"
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
  grep -qF " $1 " "$MOUNTS" 2>/dev/null
}

luna_bind() {
  exe=$ROOT$1
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
  exe=$ROOT$1
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
          is_bound "$ROOT$N" && done_=$((done_ + 1))
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
  printf '{"ok":true,"profile":"%s","groups":{%s},"persist":%s,"skip":%s,"early":%s,"memAvailableKb":%s}\n' \
    "${prof:-off}" "$out" "$([ -L "$INIT_LINK" ] && echo true || echo false)" \
    "$([ -f "$SKIP" ] && echo true || echo false)" "$(early_json)" \
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
  # The record of an early pass describes the groups that were on then.
  [ "$DRY" = 1 ] || rm -f "$EARLY_RECORD"
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
  [ "$DRY" = 1 ] || rm -f "$EARLY_RECORD"
  if [ ! -s "$ACTIVE" ] && [ "$DRY" != 1 ]; then
    rm -rf "$BEFORE" "$ACTIVE" "$PROFILE_FILE" "$BACKUP" "$EARLY_RECORD" "$EARLY_PENDING"
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

# Whether the kdump shim that starts autostart.sh's worker is in place (tools/lumatv,
# install_early_hook) and the worker is not switched off by the persist flag.
early_hook_ready() {
  eh_env=$ROOT/var/systemd/system/env/kdump.env
  [ -f "$eh_env" ] && grep -qF "$BRAND_DATA_DIR/earlybin" "$eh_env" 2>/dev/null \
    && [ -x "$ROOT$BRAND_DATA_DIR/earlybin/grep" ] \
    && grep -q '"persist"[[:space:]]*:[[:space:]]*true' "$ROOT$BRAND_DATA_DIR/config.json" 2>/dev/null
}

# The boot's clock to the hundredth (the wall clock is not set for the first seconds), and the boot's
# own id: a record of an early pass is this boot's only when the two ids agree.
boot_clock() { cut -d' ' -f1 /proc/uptime; }
boot_id() { printf '%s' "${DECLUTTER_BOOT_ID:-$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)}"; }

# The early step's state for `status`: whether it will run at the next boot (the shim, the
# worker's flag, this tool's own persist link and no skip file), a marker left by an early pass
# that has not settled yet, the record of the last pass under the groups that are on now, and
# whether that record is this boot's.
early_json() {
  ej_hook=false
  [ -L "$INIT_LINK" ] && [ ! -f "$SKIP" ] && early_hook_ready && ej_hook=true
  ej_last=null ej_this=false
  if [ -s "$EARLY_RECORD" ]; then
    ej_last=$(cat "$EARLY_RECORD")
    [ "$(printf '%s' "$ej_last" | sed -n 's/.*"bootId":"\([^"]*\)".*/\1/p')" = "$(boot_id)" ] && ej_this=true
  fi
  printf '{"hook":%s,"pending":%s,"thisBoot":%s,"last":%s}' "$ej_hook" "$([ -f "$EARLY_PENDING" ] && echo true || echo false)" "$ej_this" "$ej_last"
}

# $1 ran (true|false), $2 why not, $3 masked units, $4 stubbed programs, $5 what was already running.
# startSec, stubbedSec, maskedSec and uptimeSec are the boot's clock when the pass began, when the
# last stub was in place, when the masks were loaded and when it ended.
early_write() {
  [ "$DRY" = 1 ] && return 0
  mkdir -p "$DCS" 2>/dev/null
  printf '{"ran":%s,"why":"%s","bootId":"%s","startSec":%s,"stubbedSec":%s,"maskedSec":%s,"uptimeSec":%s,"masked":%s,"stubbed":%s,"running":%s}\n' \
    "$1" "$(json_escape "$2")" "$(boot_id)" "${EV_T0:-null}" "${EV_T1:-null}" "${EV_T2:-null}" "$(boot_clock)" \
    "$(json_list "$3")" "$(json_list "$4")" "$(json_list "$5")" > "$EARLY_RECORD.tmp" 2>/dev/null && mv "$EARLY_RECORD.tmp" "$EARLY_RECORD"
}

# The early boot step, started by autostart.sh's worker about 13 to 14 s after power-on, the earliest
# the mod can act (/var is not writable before mount-readwrite at 10.7 s, and kdump's script, the
# hook, starts at 13.4 s). On the stock boot the network came up at 17.5 s, pbs started at 16.9 s,
# the content miner at 18.7 s and the targets that pull nudge, uploadd and the rest at 22 s and 30 s,
# while acr2, admanager, adoverlay-service and sportsalarm were launched at 43 to 46 s; the first
# pass of `boot` (init.d) only came at 73 s. So what needs neither luna-send nor LG's settings
# service goes down at once, in the order of how soon LG can start it: the stub over each on-demand
# program (a mount each, nothing else to wait for), then the masks of the units (one systemd
# reload), then a stop for whatever of them already runs and a kill for the programs. Settings and
# apps persist across a boot and need no early step. Nothing is touched on a boot that is not to
# be: no group on, persist off (nothing would settle the marker), the skip file, a boot that never
# completed (which `boot` consumes), or one that did not settle after the last early pass (skipped
# once). One JSON line, always exit 0; the record is for `status`.
verb_early() {
  [ -s "$ACTIVE" ] || { printf '{"ok":true,"early":false,"why":"no group is on"}\n'; exit 0; }
  EV_T0=$(boot_clock) EV_T1="" EV_T2=""
  ev_why=""
  if [ ! -L "$INIT_LINK" ]; then ev_why="persist is off"
  elif [ -f "$SKIP" ]; then ev_why="the skip file is present"
  elif [ -f "$BOOT_PENDING" ]; then ev_why="the previous boot never completed"
  elif [ -f "$EARLY_PENDING" ]; then [ "$DRY" = 1 ] || rm -f "$EARLY_PENDING"; ev_why="the previous boot did not settle after the early pass"
  elif ! refuse_denied >/dev/null; then ev_why="items.txt names a protected unit"
  elif ! take_lock; then ev_why="another run holds the lock"
  fi
  if [ -n "$ev_why" ]; then
    early_write false "$ev_why" "" "" ""
    [ "$DRY" = 1 ] || log_line "early: skipped ($ev_why) at uptime $(boot_clock) s"
    printf '{"ok":true,"early":false,"why":"%s"}\n' "$(json_escape "$ev_why")"
    exit 0
  fi
  [ -f "$STUB" ] || { mkdir -p "$DCS"; mut cp "$SRC/stub.sh" "$STUB"; mut chmod 755 "$STUB"; }
  [ "$DRY" = 1 ] || : > "$EARLY_PENDING"
  # What there is to do, from the items of the groups that are on (reads only).
  ev_units="" ev_progs="" ev_pats=""
  for g in $(cat "$ACTIVE" 2>/dev/null); do
    ev_items=$(group_items "$g")
    for u in $(printf '%s\n' "$ev_items" | awk -F'|' '$1=="unit" {print $3}'); do
      unit_exists "$u" && ev_units="$ev_units $u"
    done
    for x in $(printf '%s\n' "$ev_items" | awk -F'|' '$1=="luna" && $4=="stub" {print $3}'); do
      [ -f "$ROOT$x" ] || continue
      ev_progs="$ev_progs $x"
      ev_pats="$ev_pats$x
"
    done
  done
  # What already runs, read before anything changes: the units in one call, the programs in one pass over /proc.
  ev_stop=""
  if [ -n "$ev_units" ]; then
    # shellcheck disable=SC2086
    ev_states=$(systemctl is-active $ev_units 2>/dev/null)
    # shellcheck disable=SC2086
    set -- $ev_units
    for ev_state in $ev_states; do
      [ "$ev_state" = active ] && ev_stop="$ev_stop $1"
      shift
    done
  fi
  ev_running=""
  # shellcheck disable=SC2086
  [ -n "$ev_pats" ] && ev_running=$(exe_pids "$ev_pats" name | sort -u | tr '\n' ' ')
  # shellcheck disable=SC2086
  ev_run=$(echo $ev_stop $ev_running)
  # 1. The stubs: nothing launches as the real program from here on.
  ev_stubbed=""
  for x in $ev_progs; do
    luna_bind "$x"
    if [ "$DRY" = 1 ] || is_bound "$ROOT$x"; then ev_stubbed="$ev_stubbed $x"; fi
  done
  EV_T1=$(boot_clock)
  # 2. The masks, one reload.
  ev_masked="" ev_reload=0
  [ -n "$ev_units" ] && mut mkdir -p "$RUNSYS"
  for u in $ev_units; do
    if unit_masked "$u"; then
      ev_masked="$ev_masked $u"
    elif mut ln -sf /dev/null "$RUNSYS/$u"; then
      [ -L "$RUNSYS_OLD/$u" ] && mut rm -f "$RUNSYS_OLD/$u"
      ev_masked="$ev_masked $u"
      ev_reload=1
    fi
  done
  [ "$ev_reload" = 1 ] && mut systemctl daemon-reload >/dev/null 2>&1
  EV_T2=$(boot_clock)
  # 3. What was running: the units stop, the programs are killed.
  # shellcheck disable=SC2086
  [ -n "$ev_stop" ] && mut systemctl stop --no-block $ev_stop >/dev/null 2>&1
  [ -n "$ev_pats" ] && kill_exes "$ev_pats"
  release_lock
  early_write true "" "$ev_masked" "$ev_stubbed" "$ev_run"
  n_masked=$(echo $ev_masked | wc -w | tr -d ' ')
  n_stubbed=$(echo $ev_stubbed | wc -w | tr -d ' ')
  log_line "early: $n_masked unit(s) masked, $n_stubbed program(s) stubbed, already running: ${ev_run:-none}, stubs at $EV_T1 s, masks at $EV_T2 s, done at uptime $(boot_clock) s"
  if [ "$DRY" != 1 ]; then
    setsid "$SELF" --early-settle </dev/null >/dev/null 2>&1 &
  fi
  printf '{"ok":true,"early":true,"startSec":%s,"stubbedSec":%s,"maskedSec":%s,"uptimeSec":%s,"masked":%s,"stubbed":%s,"running":%s}\n' \
    "$EV_T0" "$EV_T1" "$EV_T2" "$(boot_clock)" "$n_masked" "$n_stubbed" "$(json_list "$ev_run")"
  exit 0
}

# Clears the marker of an early pass once the set answers (as `boot_worker` does for its own), so
# a boot that hangs after the pass is skipped once. Stops when the state directory is gone.
early_settle() {
  n=0
  while [ "$n" -lt 12 ]; do
    sleep "$SETTLE_STEP"
    [ -d "$DCS" ] || return 0
    if boot_settled; then
      rm -f "$EARLY_PENDING"
      log_line "early: settled"
      return 0
    fi
    n=$((n + 1))
  done
  log_line "early: the set did not settle; the next boot skips the early pass once"
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
      mkdir -p "$ROOT/var/lib/webosbrew/init.d" 2>/dev/null
      mut ln -sf "$SELF" "$INIT_LINK"
      [ "$DRY" = 1 ] || rm -f "$EARLY_RECORD"
      printf '{"ok":true,"persist":true}\n' ;;
    off)
      mut rm -f "$INIT_LINK"
      [ "$DRY" = 1 ] || rm -f "$EARLY_RECORD"
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
  --early-settle) early_settle ;;
  persist)  verb_persist "$@" ;;
  measure)  verb_measure "$@" ;;
  verify)   verb_verify ;;
  *)        emit_error "usage: declutter.sh on|off|status|list|boot|early|persist|measure|verify"; exit 1 ;;
esac
