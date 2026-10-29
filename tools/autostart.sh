#!/bin/sh
# Boot hook.
#
# Symlinked to /var/lib/webosbrew/init.d/50-<shortId> by `<shortId> persist on`
# and run by Homebrew Channel's startup.sh via run-parts during boot.
#
# Two hard rules, because this runs on the boot path:
#
#   1. It must ALWAYS exit 0. A non-zero exit from an init.d hook can trip
#      Homebrew Channel's failsafe mode, which disables every root
#      customization on the next boot and shows the user a crash toast. A
#      cosmetic mod is never worth that, so every failure here is logged and
#      swallowed.
#   2. It must never block boot indefinitely. Every wait is bounded and the
#      whole run is capped.
#
# The hook is also a no-op unless the user explicitly asked for persistence,
# so removing the symlink is not the only off switch.

# Resolve the real app dir even though we are invoked through a symlink
# in /var/lib/webosbrew/init.d.
self="$0"
if [ -L "$self" ]; then
  target=$(readlink "$self" 2>/dev/null)
  case "$target" in
    /*) self="$target" ;;
    *)  self="$(dirname "$self")/$target" ;;
  esac
fi
APPDIR=$(cd "$(dirname "$self")/.." 2>/dev/null && pwd)

# Names come from brand.json. Without them there is nowhere to log to and
# nothing to run, and a boot hook never fails the boot.
[ -f "$APPDIR/tools/brand.sh" ] || exit 0
. "$APPDIR/tools/brand.sh"
brand_load "$APPDIR/brand.json" 2>/dev/null || exit 0

STATE="$BRAND_DATA_DIR"
LOG="$STATE/log"
# Per-boot, so it lives in tmpfs: the pid of whichever hook is applying.
WORKER_MARK="/tmp/.$BRAND_SHORT_ID-worker"
CLI="$APPDIR/tools/$BRAND_SHORT_ID"

log() {
  mkdir -p "$STATE" 2>/dev/null
  # The uptime is the boot's monotonic clock: the wall clock is not set for the first seconds, and the log's
  # own stamps say nothing about how far into the boot a step was.
  read -r up_s _ < /proc/uptime 2>/dev/null
  echo "$(date '+%Y-%m-%d %H:%M:%S') boot: [up ${up_s:-?} s] $1" >> "$LOG" 2>/dev/null
}

# Never let anything below abort the boot path.
trap 'log "hook aborted unexpectedly"; exit 0' HUP INT TERM

if [ ! -x "$CLI" ]; then
  log "cli not found at $CLI (app uninstalled?), nothing to do"
  exit 0
fi

# Honour the opt-in flag as well as the symlink's presence.
if ! grep -q '"persist"[[:space:]]*:[[:space:]]*true' "$STATE/config.json" 2>/dev/null; then
  log "persist disabled in config, skipping"
  exit 0
fi

# --worker: the detached half, run by whichever hook got here first.
#
# homestub-boot first, when the Home stub is switched on: LG's Home must not
# start (bootd opens it about 14 s into a power-on, the hook runs at 12-14 s),
# so the stub goes over its directory before anything else is done. Then
# boot-bind, so the binds are live within a second; then boot-restart, which
# restarts the compositor because its first start at boot+2.6s cannot
# see our EnvironmentFile - nothing writable is mounted on /var until
# mount-readwrite.service at ~13s; then the full apply, which restarts it only
# if boot-restart did not. The early hook (a PATH shim on
# kdump.service, 14.2s) gets here about 19s before Homebrew Channel's
# run-parts does, which is the whole difference between a boot that shows
# stock Home and one that does not.
if [ "$1" = "--worker" ]; then
  # Claimed before any work, not after: the late hook fires while the early
  # apply is still running, and a marker written only on success let it start
  # a second apply that then took the "previous boot never completed"
  # failsafe path, because boot-ok had not been written yet either.
  echo $$ > "$WORKER_MARK" 2>/dev/null
  # Tells apply it is on the boot path: never force the panel on.
  LUMA_BOOT=1; export LUMA_BOOT
  # The logo check's frames (tools/bootcap.sh): `echo <seconds> > $STATE/bootcap` before a reboot.
  # One boot only, and detached like everything else here.
  if [ -f "$STATE/bootcap" ] && [ -x "$APPDIR/tools/bootcap.sh" ]; then
    bc_secs=$(cat "$STATE/bootcap" 2>/dev/null)
    rm -f "$STATE/bootcap"
    setsid "$APPDIR/tools/bootcap.sh" "$bc_secs" "/tmp/$BRAND_SHORT_ID-bootcap" </dev/null >/dev/null 2>&1 &
    log "boot capture of ${bc_secs}s started"
  fi
  hs=$("$CLI" homestub-boot 2>&1)
  log "homestub-boot: $(echo "$hs" | tr -d '\n' | cut -c1-200)"
  # LG's ad, content-recognition and upload pieces start from about +17 s and the targets that pull the rest at +22 s and +30 s
  # (the first pass of declutter's init.d hook came at +73 s on the set): its masks and stubs go down here, before the heavier
  # steps below. Bounded; a boot with no group on only prints that.
  if [ -x "$APPDIR/tools/declutter.sh" ]; then
    de=$(timeout 20 "$APPDIR/tools/declutter.sh" early </dev/null 2>&1)
    log "declutter early: $(echo "$de" | tr -d '\n' | cut -c1-300)"
  fi
  # LumaTV's own compositor, when it is enabled: the supervisor goes over LG's binary here, before the
  # restart that apply makes, so that restart starts it. A set that never enabled it answers "off".
  cb=$("$CLI" compositor-boot 2>&1)
  log "compositor-boot: $(echo "$cb" | tr -d '\n' | cut -c1-200)"
  bb=$("$CLI" boot-bind 2>&1)
  log "boot-bind: $(echo "$bb" | tr -d '\n' | cut -c1-200)"
  # The compositor's restart, as soon as boot-bind has the set in place and before apply's preparation; the panel
  # shows the last input until it, and apply used to reach it at ~55 s. When it found a shader that does not compile it
  # has reverted the set to stock, and apply must not put it back this boot.
  br=$("$CLI" boot-restart 2>&1)
  log "boot-restart: $(echo "$br" | tr -d '\n' | cut -c1-200)"
  case "$br" in
    *'auto-reverted to stock'*) exit 0 ;;
  esac
  out=$("$CLI" apply 2>&1)
  case "$out" in
    *'"ok":true'*)
      log "apply ok"
      "$CLI" boot-ok >/dev/null 2>&1
      ;;
    *)
      log "apply failed: $(echo "$out" | tr -d '\n' | cut -c1-300)"
      # Let run-parts still have its go: the net is the whole point of it.
      rm -f "$WORKER_MARK" 2>/dev/null
      ;;
  esac
  # What this boot's log says about the logo path, kept in $STATE (/var/log/messages rotates; the window's own record,
  # $STATE/logo-events, is written by BootLogo.qml and MasterBrand.qml).
  [ -x "$APPDIR/tools/bootcap.sh" ] && "$APPDIR/tools/bootcap.sh" events "$STATE/boot-events" </dev/null >/dev/null 2>&1
  exit 0
fi

# A worker is already running or has already succeeded this boot, so
# run-parts has nothing to do. A stale pid means that worker died without
# finishing, and then the net should still run.
if [ -f "$WORKER_MARK" ]; then
  wpid=$(cat "$WORKER_MARK" 2>/dev/null)
  if [ -n "$wpid" ] && { [ -d "/proc/$wpid" ] || [ ! -e "$STATE/.boot-pending" ]; }; then
    log "early hook already handled this boot, nothing to do"
    exit 0
  fi
  log "early worker $wpid is gone and the boot never completed, applying"
fi

# Everything below runs DETACHED, and that is the whole point.
#
# Homebrew Channel's startup.sh arms a failsafe flag, runs these hooks with
# run-parts, then clears the flag ~10s later. A hook that blocks holds that
# window open for as long as it runs: if the set powers off before the flag is
# cleared, the next boot comes up in failsafe mode with every root
# customization disabled. Applying inline took ~45s, so an overnight power-off
# landed inside the window and did exactly that.
#
# So: return to run-parts immediately and do the work in a detached child.
# startup.sh closes the lock fd for children (`run-parts ... 200>&-`), so the
# child cannot hold Homebrew Channel's startup lock either.
setsid "$APPDIR/tools/autostart.sh" --worker </dev/null >/dev/null 2>&1 &
exit 0
