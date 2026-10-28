#!/bin/sh
# The video-plane player (lumatv-vplane, vplane/) on the set: staged with its clips, registered with the bus, started under a small
# supervisor, stopped, removed. One implementation for tools/lumatv (apply and revert) and for the TV steps in
# docs/research/videoplane-saver-result.md.
#
#   vplane.sh install BIN ROLE CLIPS STATE   stage BIN as STATE/bin/lumatv-vplane and the clips CLIPS as STATE/vplane/clips, write its LS2 role from
#                                            the template ROLE and the manifest that names it, rescan
#   vplane.sh start   STATE MAILBOX [APPID]  start it under the supervisor (idempotent); log STATE/vplane.log, pids STATE/vplane.pid (the
#                                            supervisor) and STATE/vplane.child (the player)
#   vplane.sh stop    STATE
#   vplane.sh remove  STATE                  stop it, delete the manifest, the role, the staged binary and the clips, rescan
#   vplane.sh status  STATE                  one JSON line
#
# Idle, the player has no window, no decoder and no bus client: it waits on its mailbox and writes a status line a minute. It is asked for a
# picture through MAILBOX (req.json) by the saver, so this script never starts a picture. Extra options for the player (a window type to try,
# `-w card`; `-e after`) are read from STATE/vplane/args if the file is there: one line, taken as it stands, at the next start.
#
# Why a role: LG's media library registers bus clients of its own once its permission gate has passed, and ls-hubd refuses an executable it
# has no role file for (docs/research/ambient-result.md: `LSHUB_NO_ROLE_FILE`, then `LSRegisterPubPriv FAILED` for the first name, the
# resource manager's client `com.webos.rm.client.<session>`). LG's client permissions already give `com.webos.rm.client.*` and
# `com.webos.media.client.*` the `private` group (docs/research/arch/sm-perm-01.txt), so no client-permissions file is written; the template
# names those two with `com.webos.media` as the one name they call and answer, like the colour reader's (payload/capture/). Whether the decoder
# library registers more names than these is the TV's to say: a denial is in /var/log/messages, and the template is widened for the name it
# shows, not before. Where the role goes, and why a manifest names it, is as for the colour reader: LUMATV_LS2_DIR overrides the directory.
#
# The supervisor starts the player again after a crash or after the player's own exit when a teardown hangs (status 3); it does not after a
# clean exit (0) or a refusal to start (2: no library, no clips; the reason is in the log), and it gives up after five restarts in five
# minutes. LUMATV_VPLANE_RESTART_SLEEP (seconds, default 2) is the pause between them.

LS2="${LUMATV_LS2_DIR:-/var/luna-service2-dev}"
ROLE="$LS2/roles.d/lumatv-vplane.json"
MANIFEST="$LS2/manifests.d/org.nphil.lumatv.vplane.json"
case "$0" in /*) SELF="$0" ;; *) SELF="$(pwd)/$0" ;; esac

rescan() {
  command -v ls-control >/dev/null 2>&1 || { echo "vplane: no ls-control; the hub reads the role when it next starts" >&2; return 0; }
  ls-control scan-volatile-dirs >/dev/null 2>&1 || echo "vplane: ls-control scan-volatile-dirs failed" >&2
  ls-control scan-services >/dev/null 2>&1 || true
}

alive_exe() { # alive_exe PIDFILE EXE: the pid in the file when that process is EXE
  pid=$(cat "$1" 2>/dev/null)
  [ -n "$pid" ] && [ "$(readlink "/proc/$pid/exe" 2>/dev/null)" = "$2" ] && printf '%s' "$pid"
}

supervisor_pid() { # the pid of the supervisor of this STATE, or nothing
  pid=$(cat "$1/vplane.pid" 2>/dev/null)
  [ -n "$pid" ] && tr '\0' ' ' < "/proc/$pid/cmdline" 2>/dev/null | grep -qF "supervise $1 " && printf '%s' "$pid"
}

player_pid() { alive_exe "$1/vplane.child" "$1/bin/lumatv-vplane"; }

same_dir() { # same_dir A B: the same file names with the same bytes
  [ "$(ls "$1" 2>/dev/null)" = "$(ls "$2" 2>/dev/null)" ] || return 1
  for f in "$1"/*; do cmp -s "$f" "$2/${f##*/}" || return 1; done
  return 0
}

wait_gone() { # wait_gone PID: up to three seconds for it to go, then no more patience
  n=0
  while [ "$n" -lt 30 ] && [ -e "/proc/$1" ]; do sleep 0.1; n=$((n + 1)); done
  [ -e "/proc/$1" ] && kill -9 "$1" 2>/dev/null
  return 0
}

stop_cmd() {
  pid=$(supervisor_pid "$1")
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait_gone "$pid"
  pid=$(player_pid "$1")                         # one the supervisor did not get to
  [ -n "$pid" ] && kill "$pid" 2>/dev/null && wait_gone "$pid"
  rm -f "$1/vplane.pid" "$1/vplane.child"
  return 0
}

install_cmd() {
  bin=$1 tpl=$2 clips=$3 state=$4
  [ -x "$bin" ] && [ -f "$tpl" ] && [ -d "$clips" ] || { echo "vplane: no binary, role template or clips in the package" >&2; return 1; }
  exe="$state/bin/lumatv-vplane"
  if ! cmp -s "$bin" "$exe" 2>/dev/null; then # an update cannot overwrite a running executable (text busy): stop, swap
    stop_cmd "$state"
    mkdir -p "$state/bin" && cp -f "$bin" "$exe.new" && chmod 755 "$exe.new" && mv -f "$exe.new" "$exe" \
      || { rm -f "$exe.new"; echo "vplane: could not stage the binary" >&2; return 1; }
  fi
  # the clips are the package's, whole: a print that is no longer in it is no longer here
  dst="$state/vplane/clips"
  rm -rf "$dst.new"
  mkdir -p "$dst.new" && cp -f "$clips"/* "$dst.new"/ 2>/dev/null && [ -n "$(ls "$dst.new")" ] \
    || { rm -rf "$dst.new"; echo "vplane: could not stage the clips" >&2; return 1; }
  if ! same_dir "$dst.new" "$dst"; then
    rm -rf "$dst.old"
    [ -d "$dst" ] && mv "$dst" "$dst.old"
    mv "$dst.new" "$dst" && rm -rf "$dst.old" || { echo "vplane: could not stage the clips" >&2; return 1; }
  else
    rm -rf "$dst.new"
  fi
  mkdir -p "$LS2/roles.d" "$LS2/manifests.d" || { echo "vplane: cannot write under $LS2" >&2; return 1; }
  sed "s#@EXE@#$exe#g" "$tpl" > "$ROLE.new" || { rm -f "$ROLE.new"; return 1; }
  printf '{"id":"org.nphil.lumatv.vplane","version":"1.0.0","roleFiles":["%s"]}\n' "$ROLE" > "$MANIFEST.new" \
    || { rm -f "$ROLE.new" "$MANIFEST.new"; return 1; }
  if cmp -s "$ROLE.new" "$ROLE" 2>/dev/null && cmp -s "$MANIFEST.new" "$MANIFEST" 2>/dev/null; then # unchanged: no rescan at every boot
    rm -f "$ROLE.new" "$MANIFEST.new"
  else
    mv -f "$ROLE.new" "$ROLE" && mv -f "$MANIFEST.new" "$MANIFEST" || return 1
    rescan
  fi
  return 0
}

supervise_cmd() { # runs in the background: the player, again after a crash
  state=$1 mailbox=$2 appid=$3
  echo $$ > "$state/vplane.pid"
  exe="$state/bin/lumatv-vplane"
  log="$state/vplane.log"
  extra=$(cat "$state/vplane/args" 2>/dev/null)
  pause="${LUMATV_VPLANE_RESTART_SLEEP:-2}"
  child=
  trap 'if [ -n "$child" ]; then kill "$child" 2>/dev/null; wait "$child" 2>/dev/null; fi; rm -f "$state/vplane.child"; exit 0' TERM INT
  fails=0
  window=$(date +%s)
  # LG's AV connector opens its own Wayland connection to register the window id and reads XDG_RUNTIME_DIR for it (nothing else
  # tells it the socket): without it `avc.lsm registerId` fails and the pipeline reports "fail to create AVProxy" (set, 2026-10-25 23:38).
  export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/tmp/xdg}" WAYLAND_DISPLAY="${WAYLAND_DISPLAY:-wayland-0}"
  while :; do
    # shellcheck disable=SC2086
    "$exe" -d "$mailbox" -c "$state/vplane/clips" ${appid:+-a "$appid"} $extra >> "$log" 2>&1 < /dev/null &
    child=$!
    echo "$child" > "$state/vplane.child"
    wait "$child"
    rc=$?
    child=
    rm -f "$state/vplane.child"
    case "$rc" in 0|2) break ;; esac
    now=$(date +%s)
    [ $((now - window)) -gt 300 ] && { fails=0; window=$now; }
    fails=$((fails + 1))
    if [ "$fails" -gt 5 ]; then echo "vplane: the player ended five times (last status $rc) in five minutes; not started again" >> "$log"; break; fi
    echo "vplane: the player ended (status $rc); starting it again" >> "$log"
    sleep "$pause"
  done
  rm -f "$state/vplane.pid"
}

start_cmd() {
  state=$1 mailbox=$2 appid=$3
  [ -x "$state/bin/lumatv-vplane" ] && [ -d "$state/vplane/clips" ] || { echo "vplane: not installed" >&2; return 1; }
  [ -n "$mailbox" ] || { echo "vplane: no mailbox directory" >&2; return 1; }
  [ -n "$(supervisor_pid "$state")" ] && return 0
  [ "$(wc -c < "$state/vplane.log" 2>/dev/null || echo 0)" -gt 262144 ] && mv -f "$state/vplane.log" "$state/vplane.log.1"
  # 8>&- 9>&-: not lumatv's mutation lock (flock on fd 9) for the supervisor's whole life; see capture.sh's start.
  if command -v setsid >/dev/null 2>&1; then
    setsid nohup sh "$SELF" supervise "$state" "$mailbox" "$appid" >/dev/null 2>&1 < /dev/null 8>&- 9>&- &
  else
    nohup sh "$SELF" supervise "$state" "$mailbox" "$appid" >/dev/null 2>&1 < /dev/null 8>&- 9>&- &
  fi
  echo $! > "$state/vplane.pid"
  return 0
}

remove_cmd() {
  state=$1
  stop_cmd "$state"
  had=0
  [ -e "$ROLE" ] || [ -e "$MANIFEST" ] && had=1
  rm -f "$MANIFEST" "$MANIFEST.new" "$ROLE" "$ROLE.new" "$state/bin/lumatv-vplane" "$state/bin/lumatv-vplane.new"
  rm -rf "$state/vplane/clips" "$state/vplane/clips.new" "$state/vplane/clips.old"
  [ "$had" = 1 ] && rescan
  return 0
}

status_cmd() {
  state=$1
  inst=false role=false run=false pid=0 clips=0 walls=0
  [ -x "$state/bin/lumatv-vplane" ] && inst=true
  [ -e "$ROLE" ] && [ -e "$MANIFEST" ] && role=true
  if [ -d "$state/vplane/clips" ]; then # the prints of the art saver, and the moving wallpaper's clips (wall-*.vpc) apart
    clips=$(ls "$state/vplane/clips" 2>/dev/null | grep '\.vpc$' | grep -vc '^wall-')
    walls=$(ls "$state/vplane/clips" 2>/dev/null | grep -c '^wall-.*\.vpc$')
  fi
  p=$(supervisor_pid "$state")
  c=$(player_pid "$state")
  [ -n "$p" ] && [ -n "$c" ] && { run=true; pid=$c; }
  printf '{"installed":%s,"role":%s,"clips":%s,"walls":%s,"running":%s,"pid":%s}\n' "$inst" "$role" "$clips" "$walls" "$run" "$pid"
}

case "$1" in
  install) shift; install_cmd "$@" ;;
  start) shift; start_cmd "$@" ;;
  supervise) shift; supervise_cmd "$@" ;;
  stop) shift; stop_cmd "$@" ;;
  remove) shift; remove_cmd "$@" ;;
  status) shift; status_cmd "$@" ;;
  *) echo "usage: vplane.sh install BIN ROLE CLIPS STATE | start STATE MAILBOX [APPID] | stop STATE | remove STATE | status STATE" >&2; exit 2 ;;
esac
