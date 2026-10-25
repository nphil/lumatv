#!/bin/sh
# LumaTV's own compositor, as `tools/lumatv` drives it: the mount that makes it the live one, the
# commands that switch it, and the questions the rest of the tool asks about it. POSIX sh and busybox
# ash only. Sourced by tools/lumatv; tests/test_compositor.py sources it with fake binaries.
#
# How it goes live (docs/research/compositor-c2-result.md, "Enable path"): `lumatv-sm`, the 32-bit
# supervisor, is bind-mounted over /usr/bin/surface-manager, so `surface-manager-daemon.service` runs
# it where it ran LG's compositor; the supervisor decides again at every start (enabled flag, firmware
# fingerprint, no recorded escape, fewer than two failures) and execs LG's binary itself when any of
# them says no. The bundle is put in $COMP by `compositor/tools/tv-cutover.sh install`, which also records
# the size and mtime of LG's binary in $COMP/lg-binary: the one firmware check the supervisor cannot make
# for itself, because once the mount is in place it cannot see through it.
#
# Expects from the caller: COMP (the state directory, <dataDir>/compositor), log_line, and, for
# comp_switch, restart_surface, recover_home and take_mutation_lock. COMP_LG, COMP_MOUNTINFO and
# COMP_LOCK default to the set's paths and exist so the host test can point them elsewhere.

COMP_LG="${COMP_LG:-/usr/bin/surface-manager}"
COMP_MOUNTINFO="${COMP_MOUNTINFO:-/proc/self/mountinfo}"
# A recovery of LG's Home (tv-recover-home.sh) holds this directory while it runs.
COMP_LOCK="${COMP_LOCK:-/tmp/lumatv-recover.lock}"

# Whether the supervisor is mounted over LG's binary. The mount table is read, not `mount`'s output,
# so one grep covers it and a path that merely contains the name does not count.
comp_mounted() {
  grep -qF " $COMP_LG " "$COMP_MOUNTINFO" 2>/dev/null
}

# Whether LumaTV's compositor is the one drawing: its process is up and the mount is in place. The
# process that carries LG's name is then the supervisor, so nothing about it (its import path, its
# environment) says anything about the compositor, and this is the only test the guards use.
comp_live() {
  # The flag is a builtin test: the Home stub's guard asks once a second, and a set that never
  # enabled LumaTV's compositor must cost it no process.
  [ -f "$COMP/enabled" ] || return 1
  comp_mounted && pgrep -f 'bin/lumatv-compositor' >/dev/null 2>&1
}

# A recovery is in progress: nothing may restart the compositor's unit meanwhile.
comp_recovering() {
  [ -e "$COMP_LOCK" ]
}

comp_enabled() {
  [ -f "$COMP/enabled" ] && [ -x "$COMP/bin/lumatv-sm" ]
}

# Whether LG's binary on disk is the one the bundle was installed against. Meaningless, and not
# asked, while the mount is in place.
comp_lg_unchanged() {
  [ -f "$COMP/lg-binary" ] && [ "$(stat -c '%s %Y' "$COMP_LG" 2>/dev/null)" = "$(cat "$COMP/lg-binary" 2>/dev/null)" ]
}

# At boot, before the restart that picks up the compositor's env file: put the mount in place when
# everything says LumaTV's compositor is wanted. Prints one word for the log: mounted, already,
# off (not enabled or no bundle), firmware (LG's binary changed, so LG's compositor stays),
# failsafe (the previous boot never completed), or error. Never fails the boot: the exit status is 0.
comp_boot_mount() {
  if comp_mounted; then echo already; return 0; fi
  if ! comp_enabled; then echo off; return 0; fi
  if [ -f "${BOOT_PENDING:-/nonexistent}" ]; then echo failsafe; return 0; fi
  if ! comp_lg_unchanged; then echo firmware; return 0; fi
  if mount --bind "$COMP/bin/lumatv-sm" "$COMP_LG" 2>/dev/null && comp_mounted; then echo mounted; else echo error; fi
  return 0
}

# Unmounts the supervisor and forgets the flag, so the next start is LG's. Used by revert; the
# restart that follows is the caller's.
comp_release() {
  rm -f "$COMP/enabled" 2>/dev/null
  if comp_mounted; then
    umount "$COMP_LG" 2>/dev/null || umount -l "$COMP_LG" 2>/dev/null
  fi
  return 0
}

# on | off. `on` needs the bundle from `install`; it clears the failure count and the recorded escape,
# mounts, restarts the unit and lets the supervisor take over. `off` is the way back: flag removed,
# mount removed, LG's compositor restarted, Home recovered.
comp_switch() {
  case "$1" in
    on)
      [ -x "$COMP/bin/lumatv-sm" ] || { echo "no bundle in $COMP (run compositor/tools/tv-cutover.sh install first)"; return 2; }
      comp_recovering && { echo "a recovery of LG's Home is running; try again when it is done"; return 3; }
      comp_lg_unchanged || comp_mounted || { echo "LG's compositor binary is not the one the bundle was installed against (firmware changed); install again"; return 4; }
      : > "$COMP/enabled"
      echo 0 > "$COMP/failures"
      rm -f "$COMP/escaped" "$COMP/last-fallback"
      comp_mounted || mount --bind "$COMP/bin/lumatv-sm" "$COMP_LG" || { echo "mount failed"; return 1; }
      restart_surface || { echo "the compositor's unit did not come back; run 'off'"; return 1; }
      recover_home
      ;;
    off)
      rm -f "$COMP/enabled" 2>/dev/null
      if comp_mounted; then
        umount "$COMP_LG" 2>/dev/null || umount -l "$COMP_LG" 2>/dev/null
        restart_surface || return 1
      fi
      recover_home
      ;;
    *) echo "compositor: on, off or status"; return 2 ;;
  esac
  return 0
}

# One JSON object: what is configured, what is live, what the supervisor last said.
comp_json() {
  en=false; comp_enabled && en=true
  mt=false; comp_mounted && mt=true
  lv=false; comp_live && lv=true
  bd=false; [ -x "$COMP/bin/lumatv-sm" ] && bd=true
  fw=unknown
  if [ "$mt" = true ]; then fw=mounted; elif comp_lg_unchanged; then fw=same; elif [ -f "$COMP/lg-binary" ]; then fw=changed; fi
  esc=false; [ -f "$COMP/escaped" ] && esc=true
  fl=$(cat "$COMP/failures" 2>/dev/null); [ -n "$fl" ] || fl=0
  case "$fl" in *[!0-9]*) fl=0 ;; esac
  lf=$(cat "$COMP/last-fallback" 2>/dev/null | tr -d '"\\' | tr '\n' ' ' | cut -c1-160)
  sl=$(tail -n 1 "$COMP/sm.log" 2>/dev/null | tr -d '"\\' | cut -c1-200)
  printf '{"bundle":%s,"enabled":%s,"mounted":%s,"live":%s,"lgBinary":"%s","escaped":%s,"failures":%s,"lastFallback":"%s","smLog":"%s"}' \
    "$bd" "$en" "$mt" "$lv" "$fw" "$esc" "$fl" "$lf" "$sl"
}
