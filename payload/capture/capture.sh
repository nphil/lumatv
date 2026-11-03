#!/bin/sh
# The ambient light's colour reader (lumatv-capture, ambient/capture/service.c) on the set: staged, registered with the bus,
# started, stopped, removed. One implementation for tools/lumatv (apply and revert) and for the light-sync probe slot
# (ambient/capture/tv_probe_slot.py).
#
#   capture.sh install BIN ROLE STATE   stage BIN as STATE/bin/lumatv-capture, write its LS2 role from the template ROLE and the
#                                       manifest that names it, rescan
#   capture.sh start   STATE SOCKET     start it (idempotent); diagnostics go to the RAM collector, pid STATE/capture.pid
#   capture.sh stop    STATE
#   capture.sh remove  STATE            stop it, delete the manifest, the role and the staged binary, rescan
#   capture.sh status  STATE            one JSON line
#
# Why a role: LG's library registers bus clients of its own once its permission gate has passed, and ls-hubd refuses an
# executable it has no role file for (`LSHUB_NO_ROLE_FILE {"EXE": ...}`, the light-sync probe slots of 2026-10-25,
# docs/research/ambient-result.md). Seen on the set (16:21): the first name it registers is a resource-manager client,
# `com.webos.rm.client.<session>` (uMediaServer's connector, which talks to `com.webos.media`); LG's own role for
# `/usr/bin/vtCaptureTestSuite` names `com.webos.service.capture.client*` and `com.webos.rm.client.*` with outbound
# `com.webos.media`, and the template names the same two and the two capture services the library calls. Nothing is offered
# to anyone but the resource manager's own replies (`inbound`). The groups those names call with are LG's own
# (`compat.perm.json` gives `com.webos.rm.client.*` `private`, `com.webos.service.capture.perm.json` gives the capture client
# `all`), so no client-permissions file is written. The role is matched by executable path: the template carries @EXE@ and the
# role is written for the path the binary is staged at.
#
# Where: rooted installs write their LS2 files under /var/luna-service2-dev, not the read-only /usr/share/luna-service2.
# On webOS 10 the hub reads a volatile role only through a manifest in `manifests.d` (`ManifestsVolatileDirectories` in
# /etc/luna-service2/ls-hubd.conf; `roles.d` alone is not scanned: the slot of 16:21 wrote the role there and the hub still
# said no role file), so install writes `manifests.d/org.nphil.lumatv.capture.json` naming the role, in the flat `roles.d`
# format of the dev-installed apps' files (`exeName`, `type`, `allowedNames`, `permissions`). LUMATV_LS2_DIR overrides the
# directory (the tests do). ls-hubd is asked to rescan with ls-control (scan-volatile-dirs, scan-services); where that is not
# there the role is read at the next start of the hub. The developer flag /var/luna/preferences/vt_devmode is not touched here.

LS2="${LUMATV_LS2_DIR:-/var/luna-service2-dev}"
ROLE="$LS2/roles.d/lumatv-capture.json"
MANIFEST="$LS2/manifests.d/org.nphil.lumatv.capture.json"

rescan() {
  command -v ls-control >/dev/null 2>&1 || { echo "capture: no ls-control; the hub reads the role when it next starts" >&2; return 0; }
  ls-control scan-volatile-dirs >/dev/null 2>&1 || echo "capture: ls-control scan-volatile-dirs failed" >&2
  ls-control scan-services >/dev/null 2>&1 || true
}

running_pid() { # the pid of a lumatv-capture of this STATE that is alive, or nothing
  pid=$(cat "$1/capture.pid" 2>/dev/null)
  [ -n "$pid" ] && [ "$(readlink "/proc/$pid/exe" 2>/dev/null)" = "$1/bin/lumatv-capture" ] && printf '%s' "$pid"
}

stop_cmd() {
  pid=$(running_pid "$1")
  if [ -n "$pid" ]; then
    kill "$pid" 2>/dev/null
    n=0
    while [ "$n" -lt 30 ] && [ -e "/proc/$pid" ]; do sleep 0.1; n=$((n + 1)); done
    [ -e "/proc/$pid" ] && kill -9 "$pid" 2>/dev/null
  fi
  rm -f "$1/capture.pid"
  return 0
}

install_cmd() {
  bin=$1 tpl=$2 state=$3
  [ -x "$bin" ] && [ -f "$tpl" ] || { echo "capture: no binary or no role template in the package" >&2; return 1; }
  exe="$state/bin/lumatv-capture"
  if ! cmp -s "$bin" "$exe" 2>/dev/null; then # an update cannot overwrite a running executable (text busy): stop, swap
    stop_cmd "$state"
    mkdir -p "$state/bin" && cp -f "$bin" "$exe.new" && chmod 755 "$exe.new" && mv -f "$exe.new" "$exe" \
      || { rm -f "$exe.new"; echo "capture: could not stage the binary" >&2; return 1; }
  fi
  mkdir -p "$LS2/roles.d" "$LS2/manifests.d" || { echo "capture: cannot write under $LS2" >&2; return 1; }
  sed "s#@EXE@#$exe#g" "$tpl" > "$ROLE.new" || { rm -f "$ROLE.new"; return 1; }
  printf '{"id":"org.nphil.lumatv.capture","version":"1.0.0","roleFiles":["%s"]}\n' "$ROLE" > "$MANIFEST.new" \
    || { rm -f "$ROLE.new" "$MANIFEST.new"; return 1; }
  if cmp -s "$ROLE.new" "$ROLE" 2>/dev/null && cmp -s "$MANIFEST.new" "$MANIFEST" 2>/dev/null; then # unchanged: no rescan at every boot
    rm -f "$ROLE.new" "$MANIFEST.new"
  else
    mv -f "$ROLE.new" "$ROLE" && mv -f "$MANIFEST.new" "$MANIFEST" || return 1
    rescan
  fi
  return 0
}

start_cmd() {
  state=$1 socket=$2
  exe="$state/bin/lumatv-capture"
  [ -x "$exe" ] || { echo "capture: not installed" >&2; return 1; }
  [ -n "$(running_pid "$state")" ] && return 0
  # 8>&- 9>&-: the daemon must not inherit lumatv's mutation lock (flock on fd 9), or it holds the lock for as long as it
  # runs and every later apply or restart waits 180 s and fails (seen 2026-10-26: the reader of the 00:18 apply held it).
  if command -v setsid >/dev/null 2>&1; then
    setsid nohup "$exe" -s "$socket" > /dev/null 2>&1 < /dev/null 8>&- 9>&- &
  else
    nohup "$exe" -s "$socket" > /dev/null 2>&1 < /dev/null 8>&- 9>&- &
  fi
  echo $! > "$state/capture.pid"
  return 0
}

remove_cmd() {
  state=$1
  stop_cmd "$state"
  had=0
  [ -e "$ROLE" ] || [ -e "$MANIFEST" ] && had=1
  rm -f "$MANIFEST" "$MANIFEST.new" "$ROLE" "$ROLE.new" "$state/bin/lumatv-capture" "$state/bin/lumatv-capture.new"
  [ "$had" = 1 ] && rescan
  return 0
}

status_cmd() {
  state=$1
  inst=false role=false run=false pid=0
  [ -x "$state/bin/lumatv-capture" ] && inst=true
  [ -e "$ROLE" ] && [ -e "$MANIFEST" ] && role=true
  p=$(running_pid "$state")
  [ -n "$p" ] && { run=true; pid=$p; }
  printf '{"installed":%s,"role":%s,"running":%s,"pid":%s}\n' "$inst" "$role" "$run" "$pid"
}

case "$1" in
  install) shift; install_cmd "$@" ;;
  start) shift; start_cmd "$@" ;;
  stop) shift; stop_cmd "$@" ;;
  remove) shift; remove_cmd "$@" ;;
  status) shift; status_cmd "$@" ;;
  *) echo "usage: capture.sh install BIN ROLE STATE | start STATE SOCKET | stop STATE | remove STATE | status STATE" >&2; exit 2 ;;
esac
