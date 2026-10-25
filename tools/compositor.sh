#!/bin/sh
# LumaTV's own compositor, as `tools/lumatv` drives it: the mount that makes it the live one, the
# commands that switch it, and the questions the rest of the tool asks about it. POSIX sh and busybox
# ash only, and clean under `set -u`. Sourced by tools/lumatv and by the on-set cutover script
# (compositor/tools/tv-cutover.sh); tests/test_compositor.py sources it with fake binaries.
#
# How it goes live (docs/research/compositor-c2-result.md, "Enable path"; docs/research/cutover-result.md):
# `lumatv-sm`, the 32-bit supervisor, is bind-mounted over /usr/bin/surface-manager, so
# `surface-manager-daemon.service` runs it where it ran LG's compositor; the supervisor decides again
# at every start and execs LG's binary itself when anything says no. The bundle is put in $COMP by
# `compositor/tools/tv-cutover.sh install`, which also records what the checks below compare with:
#   lg-binary        size and mtime of LG's binary (the supervisor cannot see through its own mount)
#   fingerprint      the supervisor's stat-and-release hash of LG's libraries
#   manifest         SHA-256 of LG's binary and libraries as installed (LG's binary is hidden once mounted)
#   verified.list    the builds the gate passed on, from compositor/firmware/verified.list
#   bundle.manifest  size (and SHA-256 for bin/) of every file of the bundle
#   supervisor-api   2: the supervisor in bin/ answers --would-run and --home-held
# and the flag `enabled`: empty or "persistent" holds across boots, "session <boot id>" only in the boot
# it was made in (a gate or trial cut short never becomes a boot-time enable).
#
# A persistent enable needs a build on verified.list: LumaTV starts on LG's compositor until the contract
# checks have passed again on the firmware (PLAN, C4). Nothing here ever sets the flag by itself: only
# comp_switch on (the user's command) and the cutover script's enable and trial do.
#
# Expects from the caller: COMP (the state directory, <dataDir>/compositor), log_line, and, for
# comp_switch, restart_surface, recover_home and take_mutation_lock. The COMP_* variables below default to
# the set's paths and exist so the host test can point them elsewhere.

COMP_LG="${COMP_LG:-/usr/bin/surface-manager}"
COMP_MOUNTINFO="${COMP_MOUNTINFO:-/proc/self/mountinfo}"
# A recovery of LG's Home (tv-recover-home.sh) holds this directory while it runs.
COMP_LOCK="${COMP_LOCK:-/tmp/lumatv-recover.lock}"
COMP_RELEASE="${COMP_RELEASE:-/etc/starfish-release}"
COMP_BOOT_ID="${COMP_BOOT_ID:-/proc/sys/kernel/random/boot_id}"
# What the firmware check hashes besides LG's compositor binary and the release file: the GPU driver and the
# Wayland and luna libraries the compositor is built against (the supervisor's fingerprint covers the first
# five), and the client libraries whose protocol tables the compositor re-implements (protocol/lg-tables.txt).
COMP_FW_LIBS="${COMP_FW_LIBS:-/usr/lib/libmali.so.0.46.0
/usr/lib/libWebOSCoreCompositor.so.0.0.1
/usr/lib/libwayland-webos-server.so.1.0.0
/usr/lib/libwayland-server.so.0
/usr/lib/libluna-service2.so.3
/usr/lib/libwayland-webos-client.so.1.0.0
/usr/lib/libgm-wayland.so.1.0.0
/usr/lib/liblsm-connector.so.1.0.0}"
# The reason the last refusal gave, for the log and the status object.
COMP_WHY=""
# What the early hook decided: one word, see comp_boot_decide.
COMP_WORD=""

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

# This boot's id: what a session flag carries.
comp_boot_id() {
  head -n 1 "$COMP_BOOT_ID" 2>/dev/null
}

# The flag: off, persistent, session (made in this boot) or stale (a session of an earlier boot).
comp_flag() {
  f_kind=""; f_id=""
  if [ ! -f "$COMP/enabled" ]; then echo off; return 0; fi
  read -r f_kind f_id < "$COMP/enabled" 2>/dev/null || true
  if [ "$f_kind" = session ]; then
    if [ -n "$f_id" ] && [ "$f_id" = "$(comp_boot_id)" ]; then echo session; else echo stale; fi
    return 0
  fi
  echo persistent
}

comp_enabled() {
  [ -x "$COMP/bin/lumatv-sm" ] || return 1
  case "$(comp_flag)" in persistent|session) return 0 ;; esac
  return 1
}

# Whether LG's binary on disk is the one the bundle was installed against. Meaningless, and not
# asked, while the mount is in place.
comp_lg_unchanged() {
  [ -f "$COMP/lg-binary" ] && [ "$(stat -c '%s %Y' "$COMP_LG" 2>/dev/null)" = "$(cat "$COMP/lg-binary" 2>/dev/null)" ]
}

# ---------------------------------------------------------------- the firmware check
# The contract: LumaTV's compositor was built against one firmware and gated on it. A persistent enable is
# allowed only on a build listed in $COMP/verified.list, by the SHA-256 of LG's compositor binary and the
# libraries below, not by name or size: an update that keeps the release string (or one this tool has never
# seen) is "unknown" and LG's compositor stays. The list is written by `cutover.sh firmware record` after a full
# gate pass and committed as compositor/firmware/verified.list; the supervisor checks the same build by its
# fingerprint (lumatv-sm decide()).

# The SHA-256 of a file as 64 hex digits, nothing when it cannot be read or no tool is there.
comp_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" 2>/dev/null | cut -d ' ' -f 1
  elif command -v python3 >/dev/null 2>&1; then
    python3 -c 'import hashlib,sys
h=hashlib.sha256()
f=open(sys.argv[1],"rb")
for b in iter(lambda: f.read(1048576), b""): h.update(b)
print(h.hexdigest())' "$1" 2>/dev/null
  fi
}

# The files the firmware check covers, one path per line: the release file, LG's compositor, the libraries.
comp_fw_files() {
  printf '%s\n' "$COMP_RELEASE" "$COMP_LG"
  printf '%s\n' "$COMP_FW_LIBS"
}

# The release file's first line: "Rockhopper release 10.2.1-5202 (ponytail-paparoa)" on the set.
comp_release_line() {
  head -n 1 "$COMP_RELEASE" 2>/dev/null | tr -d '\r'
}

# What is on the set now: the release line and `file <sha256|-> <size|-> <path>` per covered file. LG's binary
# is hidden by the mount while LumaTV is live, so its line is the one install recorded in $COMP/manifest.
comp_fw_manifest() {
  printf 'release %s\n' "$(comp_release_line)"
  comp_fw_files | while IFS= read -r m_path; do
    [ -n "$m_path" ] || continue
    if [ "$m_path" = "$COMP_LG" ] && comp_mounted; then
      m_line=$(grep "^file .* $m_path\$" "$COMP/manifest" 2>/dev/null | head -n 1)
      printf '%s\n' "${m_line:-file - - $m_path}"
    elif [ -f "$m_path" ]; then
      m_sum=$(comp_sha256 "$m_path"); m_size=$(stat -c %s "$m_path" 2>/dev/null)
      printf 'file %s %s %s\n' "${m_sum:--}" "${m_size:--}" "$m_path"
    else
      printf 'file - - %s\n' "$m_path"
    fi
  done
}

# Size and mtime of every covered file, and the release line: what a verified result is cached against, so the
# boot that follows an unchanged firmware reads ten stat results instead of hashing 60 MB.
comp_fw_sig() {
  printf 'release %s\n' "$(comp_release_line)"
  comp_fw_files | while IFS= read -r s_path; do
    [ -n "$s_path" ] || continue
    if [ "$s_path" = "$COMP_LG" ] && comp_mounted; then s_stat=$(cat "$COMP/lg-binary" 2>/dev/null)
    else s_stat=$(stat -c '%s %Y' "$s_path" 2>/dev/null); fi
    printf '%s %s\n' "$s_path" "${s_stat:-missing}"
  done
}

# verified, unverified or unknown, from the cached verdicts alone (never a hash): cheap enough for `status`. The cache is
# the last check's, kept only while the files' sizes and times are what that check saw; a changed file is `unknown`
# until the next check says.
comp_fw_state() {
  c_now=$(comp_fw_sig)
  if [ -f "$COMP/fw-verified" ] && [ "$(sed '1d' "$COMP/fw-verified" 2>/dev/null)" = "$c_now" ]; then echo verified; return 0; fi
  if [ ! -s "$COMP/verified.list" ]; then echo unverified; return 0; fi
  if [ -f "$COMP/fw-unverified" ] && [ "$(sed '1d' "$COMP/fw-unverified" 2>/dev/null)" = "$c_now" ]; then echo unverified; return 0; fi
  echo unknown
}

# Compares a manifest (stdin, then a line "@@LIST@@", then verified.list) and prints "verified <build>" or
# "unverified: <why>", the why naming the closest entry's first difference.
comp_fw_compare() {
  awk '
    $0 == "@@LIST@@" { part = 1; next }
    part == 0 {
      if ($1 == "release") { line = $0; sub(/^release[[:space:]]*/, "", line); mrel = line }
      else if ($1 == "file") { mh[$4] = $2; mp[++nm] = $4 }
      next
    }
    /^[[:space:]]*#/ || /^[[:space:]]*$/ { next }
    $1 == "build" { nb++; bname[nb] = $2; inb = 1; next }
    $1 == "release" && inb { line = $0; sub(/^release[[:space:]]*/, "", line); brel[nb] = line; next }
    $1 == "file" && inb { bh[nb "|" $4] = $2; bc[nb]++; next }
    $1 == "end" { inb = 0; next }
    END {
      best = 0; bestd = 1000000
      for (b = 1; b <= nb; b++) {
        d = 0; why = ""
        if (brel[b] != mrel) { d++; why = "the release is " (mrel == "" ? "unreadable" : mrel) ", not " brel[b] }
        for (i = 1; i <= nm; i++) {
          p = mp[i]
          if (!((b "|" p) in bh)) { d++; if (why == "") why = p " is not in the entry" }
          else if (mh[p] == "-" || bh[b "|" p] != mh[p]) { d++; if (why == "") why = (mh[p] == "-" ? p " cannot be read" : p " is not the verified file") }
        }
        if (bc[b] != nm) { d++; if (why == "") why = "the entry covers other files" }
        if (d == 0) { print "verified " bname[b]; exit 0 }
        if (d < bestd) { bestd = d; best = b; bwhy = why }
      }
      if (nb == 0) print "unverified: verified.list has no entry"
      else print "unverified: " bwhy " (closest entry " bname[best] ")"
    }'
}

# 0 and "verified <build>" when this firmware is a build the gate passed on; 5 and "unverified: <why>" when not.
# `force` ignores the cached result. A verified result is cached against comp_fw_sig.
comp_fw_check() {
  if [ ! -s "$COMP/verified.list" ]; then echo "unverified: no verified.list is installed"; return 5; fi
  c_sig=$(comp_fw_sig)
  if [ "${1:-}" != force ] && [ -f "$COMP/fw-verified" ] && [ "$(sed '1d' "$COMP/fw-verified" 2>/dev/null)" = "$c_sig" ]; then
    echo "verified $(head -n 1 "$COMP/fw-verified")"
    return 0
  fi
  c_out=$({ comp_fw_manifest; echo '@@LIST@@'; cat "$COMP/verified.list"; } | comp_fw_compare)
  case "$c_out" in
    verified\ *)
      { printf '%s\n' "${c_out#verified }"; printf '%s\n' "$c_sig"; } > "$COMP/fw-verified" 2>/dev/null
      rm -f "$COMP/fw-unverified" 2>/dev/null
      echo "$c_out"
      return 0 ;;
  esac
  c_out=${c_out:-unverified: the comparison gave no answer}
  rm -f "$COMP/fw-verified" 2>/dev/null
  { printf '%s\n' "$c_out"; printf '%s\n' "$c_sig"; } > "$COMP/fw-unverified" 2>/dev/null
  echo "$c_out"
  return 5
}

# The verified.list entry for this set: what `cutover.sh firmware record` prints once the gate has passed, to be
# committed as compositor/firmware/verified.list. Read-only.
comp_fw_block() {
  b_rel=$(comp_release_line)
  b_build=$(printf '%s\n' "$b_rel" | sed -n 's/.*release \([^ ]*\).*/\1/p')
  b_fp=$("$COMP/bin/lumatv-sm" --print-fingerprint 2>/dev/null)
  printf 'build %s %s\n' "${b_build:-unknown}" "$(date +%Y-%m-%d 2>/dev/null)"
  printf 'release %s\n' "$b_rel"
  printf 'fingerprint %s\n' "${b_fp:-unknown}"
  comp_fw_manifest | grep '^file '
  printf 'end\n'
}

# ---------------------------------------------------------------- the bundle and the supervisor
# Every file of the bundle is the size install recorded, and bin/ is the content it recorded too (`full`): a copy
# cut short by a power cut, or an older install that never recorded one, is refused before anything is mounted.
comp_bundle_ok() {
  COMP_WHY=""
  if [ ! -s "$COMP/bundle.manifest" ]; then COMP_WHY="no bundle.manifest: the bundle predates the checks, install it again"; return 1; fi
  while read -r b_size b_sum b_path; do
    [ -n "$b_path" ] || continue
    if [ "$(stat -c %s "$COMP/$b_path" 2>/dev/null)" != "$b_size" ]; then COMP_WHY="$b_path is not the installed size"; return 1; fi
    if [ "${1:-}" = full ] && [ "$b_sum" != "-" ] && [ "$(comp_sha256 "$COMP/$b_path")" != "$b_sum" ]; then COMP_WHY="$b_path differs from the installed copy"; return 1; fi
  done < "$COMP/bundle.manifest"
  return 0
}

# The supervisor's own decision (flag, escape, bundle, firmware fingerprint and verified list, failures), asked of
# the binary before anything is mounted: sets COMP_WORD to run, or its refusal (off, session, escaped, bundle,
# firmware, failures), or broken when it cannot answer (a bundle from before the question existed, a missing
# library, the wrong ABI). An old binary would take --would-run for a start and exec LG's compositor from the hook,
# so the question is asked only of a bundle whose install wrote supervisor-api.
comp_sm_says() {
  COMP_WORD=broken; COMP_WHY=""
  if [ "$(cat "$COMP/supervisor-api" 2>/dev/null)" != 2 ]; then COMP_WHY="the supervisor is from an older install (no supervisor-api)"; return 0; fi
  s_out=$(if command -v timeout >/dev/null 2>&1; then timeout 10 "$COMP/bin/lumatv-sm" --state "$COMP" --would-run 2>&1; else "$COMP/bin/lumatv-sm" --state "$COMP" --would-run 2>&1; fi); s_rc=$?
  case "$s_rc" in
    0) COMP_WORD=run ;;
    3) COMP_WORD=$(printf '%s\n' "$s_out" | sed -n 's/^no \([a-z]*\):.*/\1/p' | head -n 1)
       COMP_WHY=$(printf '%s\n' "$s_out" | sed -n 's/^no [a-z]*: *//p' | head -n 1)
       [ -n "$COMP_WORD" ] || { COMP_WORD=broken; COMP_WHY="the supervisor refused without a reason"; } ;;
    *) COMP_WORD=broken; COMP_WHY="the supervisor does not run here (exit $s_rc): $(printf '%s' "$s_out" | head -n 1 | cut -c1-100)" ;;
  esac
  return 0
}

# Whether a remote's Home key is down now and stays down: the escape, at boot.
comp_home_held() {
  [ -x "$COMP/bin/lumatv-sm" ] && [ "$(cat "$COMP/supervisor-api" 2>/dev/null)" = 2 ] || return 1
  "$COMP/bin/lumatv-sm" --state "$COMP" --home-held 700 >/dev/null 2>&1
}

# ---------------------------------------------------------------- boot
# What the early hook decides, as COMP_WORD (and COMP_WHY), the first refusal winning:
#   already   the mount is in place            off       not enabled, no bundle, or a session of an earlier boot
#   failsafe  the previous boot never completed
#   firmware  LG's binary changed, or this firmware is not a build the gate passed on (persistent enable only)
#   bundle    the installed bundle is damaged or older than the checks
#   escaped, failures, session   the supervisor's own refusals        broken   the supervisor cannot run here
#   home      Home is held down (the escape at boot)                  ok       everything allows the mount
# `force` makes the firmware check hash the files instead of trusting the cached result.
comp_boot_decide() {
  COMP_WHY=""
  if comp_mounted; then COMP_WORD=already; return 0; fi
  d_flag=$(comp_flag)
  case "$d_flag" in
    persistent|session) ;;
    stale) COMP_WORD=off; COMP_WHY="the flag is a session of an earlier boot"; return 0 ;;
    *) COMP_WORD=off; return 0 ;;
  esac
  if [ ! -x "$COMP/bin/lumatv-sm" ]; then COMP_WORD=off; COMP_WHY="no bundle"; return 0; fi
  if [ -f "${BOOT_PENDING:-/nonexistent}" ]; then COMP_WORD=failsafe; COMP_WHY="the previous boot never completed"; return 0; fi
  if ! comp_lg_unchanged; then COMP_WORD=firmware; COMP_WHY="LG's compositor binary is not the one the bundle was installed against"; return 0; fi
  if [ "$d_flag" = persistent ]; then
    if ! d_fw=$(comp_fw_check "${1:-}"); then COMP_WORD=firmware; COMP_WHY="$d_fw"; return 0; fi
  fi
  if ! comp_bundle_ok; then COMP_WORD=bundle; return 0; fi
  comp_sm_says
  if [ "$COMP_WORD" != run ]; then return 0; fi
  if comp_home_held; then COMP_WORD=home; COMP_WHY="Home is held down"; return 0; fi
  COMP_WORD=ok
  return 0
}

# At boot, before the restart that picks up the compositor's env file: put the mount in place when everything
# says LumaTV's compositor is wanted. Prints one word for the log (comp_boot_decide's, or mounted when it mounted,
# or error when the mount did not take); the reason goes to the log through log_line when there is one. With
# `check` nothing is changed and `would-mount` stands for mounted. Never fails the boot: the exit status is 0.
# A stale session flag is removed; Home held down is recorded as an escape, as the compositor's own hold would.
comp_boot_mount() {
  comp_boot_decide
  if [ -n "$COMP_WHY" ]; then log_line "compositor-boot: $COMP_WORD: $COMP_WHY"; fi
  if [ "$COMP_WORD" != ok ]; then
    if [ "${1:-}" != check ]; then
      if [ "$COMP_WORD" = home ]; then echo "hold Home at boot" > "$COMP/escaped" 2>/dev/null; fi
      if [ "$COMP_WORD" = off ] && [ "$(comp_flag)" = stale ]; then rm -f "$COMP/enabled" 2>/dev/null; fi
    fi
    echo "$COMP_WORD"
    return 0
  fi
  if [ "${1:-}" = check ]; then echo would-mount; return 0; fi
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

# on | off. `on` is the persistent enable and needs the bundle from `install`, a firmware build on the verified
# list and an intact bundle; it clears the failure count and the recorded escape, mounts, restarts the unit and lets
# the supervisor take over. `off` is the way back: flag removed, mount removed, LG's compositor restarted, Home
# recovered.
comp_switch() {
  case "${1:-}" in
    on)
      [ -x "$COMP/bin/lumatv-sm" ] || { echo "no bundle in $COMP (run compositor/tools/tv-cutover.sh install first)"; return 2; }
      comp_recovering && { echo "a recovery of LG's Home is running; try again when it is done"; return 3; }
      comp_lg_unchanged || comp_mounted || { echo "LG's compositor binary is not the one the bundle was installed against (firmware changed); install again"; return 4; }
      if ! w_fw=$(comp_fw_check force); then echo "$w_fw; LumaTV's compositor waits for LG's firmware to pass the gate (compositor/tools/tv_gate.py) and be listed in compositor/firmware/verified.list"; return 5; fi
      comp_bundle_ok full || { echo "the bundle is not the installed one ($COMP_WHY); install again"; return 6; }
      echo persistent > "$COMP/enabled"
      echo 0 > "$COMP/failures"
      rm -f "$COMP/escaped" "$COMP/last-fallback" "$COMP/running"
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
    *) echo "compositor: on, off, status or check"; return 2 ;;
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
  unc=false; [ -f "$COMP/running" ] && unc=true
  fl=$(cat "$COMP/failures" 2>/dev/null); [ -n "$fl" ] || fl=0
  case "$fl" in *[!0-9]*) fl=0 ;; esac
  lf=$(cat "$COMP/last-fallback" 2>/dev/null | tr -d '"\\' | tr '\n' ' ' | cut -c1-160)
  sl=$(tail -n 1 "$COMP/sm.log" 2>/dev/null | tr -d '"\\' | cut -c1-200)
  printf '{"bundle":%s,"enabled":%s,"flag":"%s","mounted":%s,"live":%s,"lgBinary":"%s","firmware":"%s","escaped":%s,"unsettled":%s,"failures":%s,"lastFallback":"%s","smLog":"%s"}' \
    "$bd" "$en" "$(comp_flag)" "$mt" "$lv" "$fw" "$(comp_fw_state)" "$esc" "$unc" "$fl" "$lf" "$sl"
}

# The early hook's decision with nothing changed and the firmware hashed for real: what `lumatv compositor check`
# prints, and what a TV session reads to see the boot path's answer without a reboot.
comp_check_json() {
  comp_boot_decide force
  k_why=$(printf '%s' "$COMP_WHY" | tr -d '"\\' | tr '\n' ' ' | cut -c1-200)
  k_build=$(comp_release_line | sed -n 's/.*release \([^ ]*\).*/\1/p')
  k_would=false; [ "$COMP_WORD" = ok ] && k_would=true
  printf '{"decision":"%s","why":"%s","flag":"%s","build":"%s","wouldMount":%s}' "$COMP_WORD" "$k_why" "$(comp_flag)" "${k_build:-unknown}" "$k_would"
}
