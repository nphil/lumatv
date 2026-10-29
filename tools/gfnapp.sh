#!/bin/sh
# The GeForce NOW app (docs/research/gfn-app-result.md) as `tools/lumatv` puts it on the set: a native app of its own, packed by
# gfn/build-ipk.sh, shipped by tools/build-ipk.sh as payload/gfn/<appId>.gfn.ipk inside the main package and put on the set through
# appInstallService, the way the frame and card apps are. POSIX sh and busybox ash only. Sourced by tools/lumatv; tests/test_gfnapp.py
# sources it with a fake luna-send.
#
#   gfn_update    what `lumatv apply` runs: bring an installed app to the shipped build. An app that is not installed is left alone (a
#                 new app on the set is `lumatv gfn install`, the owner's own command), and so is one that is streaming (an update
#                 cannot overwrite a running executable; the next apply does it). 0 when nothing is wrong with that.
#   gfn_install   install the shipped ipk, or update the app when the shipped build differs, closing it first; 0 when the shipped build is there
#   gfn_remove    remove the app through appInstallService, and 0 when its directory is gone. The sign-in, under /var/lib/lumatv-gfn, is not touched.
#   gfn_status    one JSON line: {"installed":bool,"build":"","shipped":"","current":bool,"running":bool}
#
# The app's version stays at 0.1.0 while its programs change, so "the shipped build" is gfn/build-ipk.sh's Build-Id (control file of
# the ipk, `build-id` in the app's directory), not the version. An app installed before that file existed has no build id and counts as stale.
#
# Expects from the caller: APPDIR (the main app's directory), APP_ID, SHORT_ID and log_line. GFN_APP, GFN_IPK, GFN_DIR, GFN_LUNA and
# GFN_WAIT (seconds to wait for an install) default to the set's and exist so the host test can point them elsewhere.

GFN_APP="${GFN_APP:-$APP_ID.gfn}"
GFN_IPK="${GFN_IPK:-$APPDIR/payload/gfn/$GFN_APP.ipk}"
GFN_DIR="${GFN_DIR:-$(dirname "$APPDIR")/$GFN_APP}"
GFN_LUNA="${GFN_LUNA:-luna-send}"
GFN_WAIT="${GFN_WAIT:-180}"

gfn_shipped_build() {
  ar p "$GFN_IPK" control.tar.gz 2>/dev/null | tar -xzO control 2>/dev/null | sed -n 's/^Build-Id: //p'
}

gfn_installed_build() {
  head -n 1 "$GFN_DIR/build-id" 2>/dev/null
}

# Whether the installed app has what SAM and the launcher need: the feeder (`main`), the UI, the core, the engine and the runtime.
gfn_complete() {
  [ -x "$GFN_DIR/lumatv-gfn-feeder" ] && [ -x "$GFN_DIR/bin64/lumatv-gfn-app" ] && [ -x "$GFN_DIR/bin64/opennow-core" ] &&
    [ -x "$GFN_DIR/bin64/opennow-streamer" ] && [ -x "$GFN_DIR/gfx64/launch.sh" ] && [ -f "$GFN_DIR/gfx64/heapshim.so" ]
}

gfn_current() {
  shipped=$(gfn_shipped_build)
  [ -n "$shipped" ] && [ "$(gfn_installed_build)" = "$shipped" ] && gfn_complete
}

gfn_running() {
  pgrep -f "$GFN_DIR/" >/dev/null 2>&1
}

# Asks SAM to close the app first when it runs: an update cannot overwrite a running executable (text busy).
gfn_close() {
  gfn_running || return 0
  "$GFN_LUNA" -n 1 -f luna://com.webos.applicationManager/closeByAppId "{\"id\":\"$GFN_APP\"}" >/dev/null 2>&1
  n=0
  while gfn_running && [ "$n" -lt 10 ]; do sleep 1; n=$((n + 1)); done
}

gfn_install() {
  [ -f "$GFN_IPK" ] || { log_line "gfn app: no ipk in the package"; return 1; }
  shipped=$(gfn_shipped_build)
  [ -n "$shipped" ] || { log_line "gfn app: the shipped ipk has no build id"; return 1; }
  gfn_current && return 0
  gfn_close
  tmp="/tmp/$SHORT_ID-gfn.ipk"
  cp -f "$GFN_IPK" "$tmp" 2>/dev/null || { log_line "gfn app: the ipk could not be copied to /tmp"; return 1; }
  out=$("$GFN_LUNA" -n 3 -f luna://com.webos.appInstallService/dev/install "{\"id\":\"$GFN_APP\",\"ipkUrl\":\"$tmp\",\"subscribe\":true}" 2>&1 | tr -d '\n')
  i=0
  while [ "$i" -lt "$GFN_WAIT" ]; do
    gfn_current && break
    sleep 1
    i=$((i + 1))
  done
  rm -f "$tmp"
  if gfn_current; then
    log_line "gfn app $shipped installed"
    return 0
  fi
  log_line "gfn app install did not complete: $(printf '%s' "$out" | cut -c1-200)"
  return 1
}

gfn_update() {
  [ -f "$GFN_IPK" ] || return 0
  [ -d "$GFN_DIR" ] || return 0
  gfn_current && return 0
  if gfn_running; then
    log_line "gfn app: running, not updated (next apply)"
    return 0
  fi
  gfn_install
}

gfn_remove() {
  [ -d "$GFN_DIR" ] || return 0
  gfn_close
  "$GFN_LUNA" -n 3 -f luna://com.webos.appInstallService/dev/remove "{\"id\":\"$GFN_APP\",\"subscribe\":true}" >/dev/null 2>&1 &
  lpid=$!
  w=0
  while [ -d "$GFN_DIR" ] && [ "$w" -lt 30 ]; do sleep 1; w=$((w + 1)); done
  kill "$lpid" 2>/dev/null
  if [ -d "$GFN_DIR" ]; then
    log_line "gfn app: still installed after 30 s"
    return 1
  fi
  log_line "gfn app removed"
  return 0
}

gfn_status() {
  inst=false
  [ -d "$GFN_DIR" ] && gfn_complete && inst=true
  cur=false
  gfn_current && cur=true
  run=false
  gfn_running && run=true
  printf '{"installed":%s,"build":"%s","shipped":"%s","current":%s,"running":%s}\n' "$inst" "$(gfn_installed_build)" "$(gfn_shipped_build)" "$cur" "$run"
}
