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
# A durable transaction directory blocks boot after interrupted promotion.
maintenance_clear() {
  [ ! -e "$STATE/maintenance" ] && [ ! -L "$STATE/maintenance" ]
}
maintenance_clear || exit 0
# Per-boot, so it lives in tmpfs: the pid of whichever hook is applying.
WORKER_MARK="/tmp/.$BRAND_SHORT_ID-worker"
CLI="$APPDIR/tools/$BRAND_SHORT_ID"

log() {
  # Early boot can precede the collector. Drop those records rather than hold up boot.
  (
    read -r up_s _ < /proc/uptime 2>/dev/null
    python3 "$APPDIR/tools/logrelay.py" send lumatv-tool "$(date '+%Y-%m-%d %H:%M:%S') boot: [up ${up_s:-?} s] $1"
  ) </dev/null >/dev/null 2>&1 &
}

# Never let anything below abort the boot path.
trap 'log "hook aborted unexpectedly"; exit 0' HUP INT TERM

if [ ! -x "$CLI" ]; then
  log "cli not found at $CLI (app uninstalled?), nothing to do"
  exit 0
fi

# Native compositor enable is independent of the legacy persist preference.
if [ ! -f "$STATE/compositor/enabled" ] && ! grep -q '"persist"[[:space:]]*:[[:space:]]*true' "$STATE/config.json" 2>/dev/null; then
  log "startup disabled, skipping"
  exit 0
fi

# --worker: the detached half, run by whichever hook got here first.
if [ "$1" = "--worker" ]; then
  maintenance_clear || exit 0
  # Claim before work so the second boot hook cannot start a competing worker.
  echo $$ > "$WORKER_MARK" 2>/dev/null
  # Boot preparation must not force the panel on.
  LUMA_BOOT=1; export LUMA_BOOT
  LUMATV_WAKE_TOKEN=$(cat /proc/sys/kernel/random/uuid 2>/dev/null | tr -d '-')
  export LUMATV_WAKE_TOKEN
  # LG's system-update block (the Homebrew Channel's flag and its four /etc/hosts names) goes first of all, in parallel with the
  # rest: the channel's own startup.sh binds it at +27 s, the network is up at +22.7 s and the update check runs then, which raised
  # LG's "update now?" alert on the cold boot of 2026-10-29. Only while the channel's flag is set; it does nothing otherwise.
  if [ -x "$APPDIR/tools/declutter.sh" ]; then
    ( maintenance_clear || exit 0; ub=$(timeout 20 "$APPDIR/tools/declutter.sh" updateblock early </dev/null 2>&1); log "update block early: $(echo "$ub" | tr -d '\n' | cut -c1-300)" ) &
  fi
  # LG's ad, recognition and upload pieces start from +11.8 s (the push client), +16.9 s (pbs) and +18.7 s (the content miner), and
  # the network comes up at about +17.5 s: declutter's early step goes first and in parallel with everything below, so that nothing
  # waits for the Home stub or the compositor. Bounded; it only reports that no group is on when none is.
  if [ -x "$APPDIR/tools/declutter.sh" ]; then
    ( maintenance_clear || exit 0; de=$(timeout 20 "$APPDIR/tools/declutter.sh" early </dev/null 2>&1); log "declutter early: $(echo "$de" | tr -d '\n' | cut -c1-300)" ) &
  fi
  # LG's Luna binding for the unified JS server (tools/sysbus.sh): lays the repaired launcher again when it was switched on, since a boot
  # puts LG's script back. Before the Home Assistant plugin's first call can start the server. Bounded, never restarts anything.
  if [ -x "$APPDIR/tools/sysbus.sh" ]; then
    ( maintenance_clear || exit 0; sb=$(timeout 20 "$APPDIR/tools/sysbus.sh" early </dev/null 2>&1); log "sysbus early: $(echo "$sb" | tr -d '\n' | cut -c1-300)" ) &
  fi
  # The logo check's frames (tools/bootcap.sh): `echo <seconds> > $STATE/bootcap` before a reboot.
  # One boot only, and detached like everything else here.
  if [ -f "$STATE/bootcap" ] && [ -x "$APPDIR/tools/bootcap.sh" ]; then
    maintenance_clear || exit 0
    bc_secs=$(cat "$STATE/bootcap" 2>/dev/null)
    rm -f "$STATE/bootcap"
    setsid "$APPDIR/tools/bootcap.sh" "$bc_secs" "/tmp/$BRAND_SHORT_ID-bootcap" </dev/null >/dev/null 2>&1 &
    log "boot capture of ${bc_secs}s started"
  fi
  # A standby activation that fails (the set was switched on while the worker ran, or a lock or a recovery was in the way)
  # gets one more pass: compositor-boot decides again and sees the panel as it is by then.
  boot_pass=0
  while [ "$boot_pass" -lt 2 ]; do
    boot_pass=$((boot_pass + 1))
    boot_retry=no
    maintenance_clear || exit 0
    cb=$("$CLI" compositor-boot 2>&1); cb_rc=$?
    log "compositor-boot ($cb_rc): $(echo "$cb" | tr -d '\n' | cut -c1-200)"
    case "$cb_rc:$cb" in
      0:*'"compositor":"mounted"'*|0:*'"compositor":"already"'*)
        case "$cb" in
          *'"standby":true'*)
            ca=$(timeout 240 "$CLI" compositor-boot activate standby </dev/null 2>&1); ca_rc=$?
            log "compositor standby activation ($ca_rc): $(echo "$ca" | tr -d '\n' | cut -c1-200)"
            if [ "$ca_rc" -eq 0 ]; then echo done > "$WORKER_MARK" 2>/dev/null; else rm -f "$WORKER_MARK" 2>/dev/null; boot_retry=yes; fi
            ;;
          *)
            wc=$(timeout 240 "$CLI" wake boot-check prepare "$LUMATV_WAKE_TOKEN" </dev/null 2>&1); wc_rc=$?
            log "wake prepare ($wc_rc): $(echo "$wc" | tr -d '\n' | cut -c1-200)"
            # The wake policy only chooses the app to land on, so a failed preparation (it rolls itself back) must not hold
            # the compositor back: it is activated with no landing. Its `systemctl daemon-reload` is cut off at 6 s and ran
            # longer in the cold boot of 2026-11-03 20:38, which left the set on LG's compositor with LumaTV's mounted.
            ca=$(timeout 240 "$CLI" compositor-boot activate </dev/null 2>&1); ca_rc=$?
            log "compositor activation ($ca_rc): $(echo "$ca" | tr -d '\n' | cut -c1-200)"
            if [ "$ca_rc" -ne 0 ]; then
              wc_rc=$ca_rc
            elif [ "$wc_rc" -eq 0 ]; then
              wc=$(timeout 45 "$CLI" wake boot-check finalize "$LUMATV_WAKE_TOKEN" </dev/null 2>&1); wc_rc=$?
              log "wake finalize ($wc_rc): $(echo "$wc" | tr -d '\n' | cut -c1-200)"
            else
              log "wake preparation failed; compositor activated with no landing"
              wc_rc=0
            fi
            if [ "$wc_rc" -eq 0 ]; then echo done > "$WORKER_MARK" 2>/dev/null; else rm -f "$WORKER_MARK" 2>/dev/null; fi ;;
        esac ;;
      0:*'"compositor":"error"'*) rm -f "$WORKER_MARK" 2>/dev/null ;;
      0:*) echo done > "$WORKER_MARK" 2>/dev/null ;;
      *) rm -f "$WORKER_MARK" 2>/dev/null ;;
    esac
    [ "$boot_retry" = yes ] || break
  done
  # What this boot's log says about the logo path, kept in $STATE (/var/log/messages rotates; the window's own record,
  # $STATE/logo-events, is written by BootLogo.qml and MasterBrand.qml).
  maintenance_clear || exit 0
  [ -x "$APPDIR/tools/bootcap.sh" ] && "$APPDIR/tools/bootcap.sh" events "$STATE/boot-events" </dev/null >/dev/null 2>&1
  exit 0
fi

# A worker is already running or has already succeeded this boot.
if [ -f "$WORKER_MARK" ]; then
  wpid=$(cat "$WORKER_MARK" 2>/dev/null)
  if [ "$wpid" = done ] || { [ -n "$wpid" ] && [ -d "/proc/$wpid" ]; }; then
    log "early hook already handled this boot, nothing to do"
    exit 0
  fi
  log "early worker $wpid did not finish; retrying"
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
maintenance_clear || exit 0
setsid "$APPDIR/tools/autostart.sh" --worker </dev/null >/dev/null 2>&1 &
exit 0
