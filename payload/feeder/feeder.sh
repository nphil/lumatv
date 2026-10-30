#!/bin/sh
# The camera feeder (lumatv-gfn-feeder, gfn/feeder/) on the set: staged where the helper starts it for a camera's live picture (the pop-up's and the
# full view's) and registered with the bus. One implementation for tools/lumatv (install, apply, revert).
#
#   feeder.sh install BIN ROLE STATE   stage BIN as STATE/bin/lumatv-gfn-feeder, write its LS2 role from the template ROLE and the manifest that
#                                      names it, rescan the hub when either changed
#   feeder.sh update  BIN STATE        stage a changed BIN over an installed feeder (the role names the path, which does not change)
#   feeder.sh remove  STATE            delete the manifest, the role and the staged binary, rescan
#   feeder.sh status  STATE            one JSON line
#
# The helper starts the feeder itself, for a stream and the seconds after it, so there is nothing to supervise here.
#
# Why a role: as for the video-plane player (payload/vplane/vplane.sh), LG's media library registers bus clients of its own once it is started by a
# process that is no app, and ls-hubd refuses an executable it has no role file for (`LSRegisterPubPriv FAILED`, measured on the set when the feeder
# was started from a root shell without one). The template names what the player's does: the same library registers the same clients. A denial is in
# /var/log/messages, and the template is widened for the name it shows, not before. LUMATV_LS2_DIR overrides the directory.

LS2="${LUMATV_LS2_DIR:-/var/luna-service2-dev}"
ROLE="$LS2/roles.d/lumatv-feeder.json"
MANIFEST="$LS2/manifests.d/org.nphil.lumatv.feeder.json"
NAME=lumatv-gfn-feeder

rescan() {
  command -v ls-control >/dev/null 2>&1 || { echo "feeder: no ls-control; the hub reads the role when it next starts" >&2; return 0; }
  ls-control scan-volatile-dirs >/dev/null 2>&1 || echo "feeder: ls-control scan-volatile-dirs failed" >&2
  ls-control scan-services >/dev/null 2>&1 || true
}

stage() { # stage BIN STATE: the binary where the helper starts it; a running one keeps its own file, the new one takes the name
  bin=$1 exe="$2/bin/$NAME"
  cmp -s "$bin" "$exe" 2>/dev/null && return 0
  mkdir -p "$2/bin" && cp -f "$bin" "$exe.new" && chmod 755 "$exe.new" && mv -f "$exe.new" "$exe" \
    || { rm -f "$exe.new"; echo "feeder: could not stage the binary" >&2; return 1; }
}

install_cmd() {
  bin=$1 tpl=$2 state=$3
  [ -x "$bin" ] && [ -f "$tpl" ] || { echo "feeder: no binary or role template in the package" >&2; return 1; }
  stage "$bin" "$state" || return 1
  mkdir -p "$LS2/roles.d" "$LS2/manifests.d" || { echo "feeder: cannot write under $LS2" >&2; return 1; }
  sed "s#@EXE@#$state/bin/$NAME#g" "$tpl" > "$ROLE.new" || { rm -f "$ROLE.new"; return 1; }
  printf '{"id":"org.nphil.lumatv.feeder","version":"1.0.0","roleFiles":["%s"]}\n' "$ROLE" > "$MANIFEST.new" \
    || { rm -f "$ROLE.new" "$MANIFEST.new"; return 1; }
  if cmp -s "$ROLE.new" "$ROLE" 2>/dev/null && cmp -s "$MANIFEST.new" "$MANIFEST" 2>/dev/null; then # unchanged: no rescan at every boot
    rm -f "$ROLE.new" "$MANIFEST.new"
  else
    mv -f "$ROLE.new" "$ROLE" && mv -f "$MANIFEST.new" "$MANIFEST" || return 1
    rescan
  fi
  return 0
}

update_cmd() {
  bin=$1 state=$2
  [ -e "$ROLE" ] && [ -e "$MANIFEST" ] && [ -x "$state/bin/$NAME" ] || return 0 # not installed: nothing to bring up to date
  [ -x "$bin" ] || { echo "feeder: no binary in the package" >&2; return 1; }
  stage "$bin" "$state"
}

remove_cmd() {
  state=$1
  had=0
  [ -e "$ROLE" ] || [ -e "$MANIFEST" ] && had=1
  rm -f "$MANIFEST" "$MANIFEST.new" "$ROLE" "$ROLE.new" "$state/bin/$NAME" "$state/bin/$NAME.new"
  [ "$had" = 1 ] && rescan
  return 0
}

status_cmd() {
  state=$1
  inst=false role=false
  [ -x "$state/bin/$NAME" ] && inst=true
  [ -e "$ROLE" ] && [ -e "$MANIFEST" ] && role=true
  printf '{"installed":%s,"role":%s}\n' "$inst" "$role"
}

case "$1" in
  install) shift; install_cmd "$@" ;;
  update) shift; update_cmd "$@" ;;
  remove) shift; remove_cmd "$@" ;;
  status) shift; status_cmd "$@" ;;
  *) echo "usage: feeder.sh install BIN ROLE STATE | update BIN STATE | remove STATE | status STATE" >&2; exit 2 ;;
esac
