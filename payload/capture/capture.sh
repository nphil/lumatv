#!/bin/sh
# The ambient light's colour reader (lumatv-capture, ambient/capture/service.c) on the set: staged, registered with the bus,
# started, stopped, removed. One implementation for tools/lumatv (apply and revert) and for the light-sync probe slot
# (ambient/capture/tv_probe_slot.py).
#
#   capture.sh install BIN ROLE STATE   stage BIN as STATE/bin/lumatv-capture, write its LS2 role from the template ROLE, rescan
#   capture.sh start   STATE SOCKET     start it (idempotent); log STATE/capture.log, pid STATE/capture.pid
#   capture.sh stop    STATE
#   capture.sh remove  STATE            stop it, delete the role and the staged binary, rescan
#   capture.sh status  STATE            one JSON line
#
# Why a role: LG's library registers a bus client of its own once its permission gate has passed, and ls-hubd refuses an
# executable it has no role file for (`LSHUB_NO_ROLE_FILE {"EXE": ...}`, the light-sync probe slot of 2026-10-25,
# docs/research/ambient-result.md). The role names only what the library registers (`com.webos.service.capture.client`) and the
# two services it calls (`com.webos.service.capture`, `...capturepermission`, strings of libvtcapture.so.1.0.0) and the bus's own
# signal service; nothing is offered to anyone (`inbound` empty). It is matched by executable path, so the template carries
# @EXE@ and the role is written for the path the binary is staged at.
#
# Where: rooted installs write their LS2 files under /var/luna-service2-dev, not the read-only /usr/share/luna-service2
# (docs/research/home-launcher-replacements.md: PlainHome's autostart.sh writes client-permissions.d there). LUMATV_LS2_DIR
# overrides it (the tests do). ls-hubd is asked to rescan with ls-control (scan-volatile-dirs, scan-services); where that is not
# there the role is read at the next start of the hub. The developer flag /var/luna/preferences/vt_devmode is not touched here.
# NOT VERIFIED ON THE TV: the directory, the role's `type`, and that the library's registration passes with exactly these names.

LS2="${LUMATV_LS2_DIR:-/var/luna-service2-dev}"
ROLE="$LS2/roles.d/lumatv-capture.json"

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
  mkdir -p "$LS2/roles.d" || { echo "capture: cannot write $LS2/roles.d" >&2; return 1; }
  sed "s#@EXE@#$exe#g" "$tpl" > "$ROLE.new" || { rm -f "$ROLE.new"; return 1; }
  if cmp -s "$ROLE.new" "$ROLE" 2>/dev/null; then # unchanged: the hub is not asked to rescan at every boot
    rm -f "$ROLE.new"
  else
    mv -f "$ROLE.new" "$ROLE" || return 1
    rescan
  fi
  return 0
}

start_cmd() {
  state=$1 socket=$2
  exe="$state/bin/lumatv-capture"
  [ -x "$exe" ] || { echo "capture: not installed" >&2; return 1; }
  [ -n "$(running_pid "$state")" ] && return 0
  [ "$(wc -c < "$state/capture.log" 2>/dev/null || echo 0)" -gt 262144 ] && mv -f "$state/capture.log" "$state/capture.log.1"
  if command -v setsid >/dev/null 2>&1; then
    setsid nohup "$exe" -s "$socket" >> "$state/capture.log" 2>&1 < /dev/null &
  else
    nohup "$exe" -s "$socket" >> "$state/capture.log" 2>&1 < /dev/null &
  fi
  echo $! > "$state/capture.pid"
  return 0
}

remove_cmd() {
  state=$1
  stop_cmd "$state"
  had=0
  [ -e "$ROLE" ] && had=1
  rm -f "$ROLE" "$ROLE.new" "$state/bin/lumatv-capture" "$state/bin/lumatv-capture.new"
  [ "$had" = 1 ] && rescan
  return 0
}

status_cmd() {
  state=$1
  inst=false role=false run=false pid=0
  [ -x "$state/bin/lumatv-capture" ] && inst=true
  [ -e "$ROLE" ] && role=true
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
