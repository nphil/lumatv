#!/bin/sh
# LumaGame (docs/lumagame/ARCHITECTURE.md) as `tools/lumatv` puts it on the set: a native app of its own, packed by game/build-ipk.sh,
# shipped by tools/build-ipk.sh as payload/game/org.nphil.lumagame.ipk inside the main package and put on the set through
# appInstallService, like the GeForce NOW app. POSIX sh and busybox ash only. Sourced by tools/lumatv; tests/test_gameapp.py sources it
# with a fake luna-send.
#
#   game_update     what boot runs: bring an installed app to the shipped build. An app that is not installed, or one that is running,
#                   is left alone. 0 when nothing is wrong with that.
#   game_install    install the shipped ipk, or update the app when the shipped build differs, closing it first; 0 when it is there
#   game_remove     remove the app through appInstallService; 0 when its directory is gone. Its data directory is not touched.
#   game_adopt_gfn  copy the GeForce NOW app's sign-in (config/, data/, prefs.json) into LumaGame's gfn engine directory
#   game_status     one JSON line: {"installed":bool,"build":"","shipped":"","current":bool,"running":bool,"data":""}
#
# The version stays at 0.1.0 while the programs change, so "the shipped build" is game/build-ipk.sh's Build-Id, not the version.
#
# Expects from the caller: APPDIR (the main app's directory), SHORT_ID and log_line. GAME_APP, GAME_IPK, GAME_DIR, GAME_LUNA, GAME_WAIT,
# GAME_DATA_BASES and GAME_GFN_LEGACY default to the set's and exist so the host test can point them elsewhere.

GAME_APP="${GAME_APP:-org.nphil.lumagame}"
GAME_IPK="${GAME_IPK:-$APPDIR/payload/game/$GAME_APP.ipk}"
GAME_DIR="${GAME_DIR:-$(dirname "$APPDIR")/$GAME_APP}"
GAME_LUNA="${GAME_LUNA:-luna-send}"
GAME_WAIT="${GAME_WAIT:-180}"
# The feeder's persistent_dir bases, in its order; each app keeps its data in <base>/<app id>.
GAME_DATA_BASES="${GAME_DATA_BASES:-/media/developer/apps/usr/palm/data /media/cryptofs/apps/usr/palm/data /media/internal /var/luna/preferences /var/palm/data /media/developer/temp}"
# Where the GeForce NOW app kept its files when the feeder found no base.
GAME_GFN_LEGACY="${GAME_GFN_LEGACY:-/var/lib/lumatv-gfn}"
GAME_GFN_APP="${GAME_GFN_APP:-org.nphil.lumatv.gfn}"

game_shipped_build() {
  ar p "$GAME_IPK" control.tar.gz 2>/dev/null | tar -xzO control 2>/dev/null | sed -n 's/^Build-Id: //p'
}

game_installed_build() {
  head -n 1 "$GAME_DIR/build-id" 2>/dev/null
}

# Whether the installed app has what SAM and the launcher need: the feeder (`main`), the UI, the runner and the runtime.
game_complete() {
  [ -x "$GAME_DIR/lumagame-main" ] && [ -x "$GAME_DIR/bin64/lumagame-ui" ] && [ -x "$GAME_DIR/run64.sh" ] &&
    [ -x "$GAME_DIR/gfx64/launch.sh" ] && [ -f "$GAME_DIR/gfx64/heapshim.so" ] && [ -f "$GAME_DIR/engines/gfn/engine.json" ]
}

game_current() {
  shipped=$(game_shipped_build)
  [ -n "$shipped" ] && [ "$(game_installed_build)" = "$shipped" ] && game_complete
}

game_running() {
  pgrep -f "$GAME_DIR/" >/dev/null 2>&1
}

# An update cannot overwrite a running executable (text busy), and the sign-in copy must not race the engine writing its own.
game_close() {
  game_running || return 0
  "$GAME_LUNA" -n 1 -f luna://com.webos.applicationManager/closeByAppId "{\"id\":\"$GAME_APP\"}" >/dev/null 2>&1
  n=0
  while game_running && [ "$n" -lt 10 ]; do sleep 1; n=$((n + 1)); done
  ! game_running
}

game_install() {
  [ -f "$GAME_IPK" ] || { log_line "game app: no ipk in the package"; return 1; }
  shipped=$(game_shipped_build)
  [ -n "$shipped" ] || { log_line "game app: the shipped ipk has no build id"; return 1; }
  game_current && return 0
  game_close
  tmp="/tmp/$SHORT_ID-game.ipk"
  cp -f "$GAME_IPK" "$tmp" 2>/dev/null || { log_line "game app: the ipk could not be copied to /tmp"; return 1; }
  out=$("$GAME_LUNA" -n 3 -f luna://com.webos.appInstallService/dev/install "{\"id\":\"$GAME_APP\",\"ipkUrl\":\"$tmp\",\"subscribe\":true}" 2>&1 | tr -d '\n')
  i=0
  while [ "$i" -lt "$GAME_WAIT" ]; do
    game_current && break
    sleep 1
    i=$((i + 1))
  done
  rm -f "$tmp"
  if game_current; then
    log_line "game app $shipped installed"
    return 0
  fi
  log_line "game app install did not complete: $(printf '%s' "$out" | cut -c1-200)"
  return 1
}

game_update() {
  [ -f "$GAME_IPK" ] || return 0
  [ -d "$GAME_DIR" ] || return 0
  game_current && return 0
  if game_running; then
    log_line "game app: running, not updated (next boot)"
    return 0
  fi
  game_install
}

game_remove() {
  [ -d "$GAME_DIR" ] || return 0
  game_close
  "$GAME_LUNA" -n 3 -f luna://com.webos.appInstallService/dev/remove "{\"id\":\"$GAME_APP\",\"subscribe\":true}" >/dev/null 2>&1 &
  lpid=$!
  w=0
  while [ -d "$GAME_DIR" ] && [ "$w" -lt 30 ]; do sleep 1; w=$((w + 1)); done
  kill "$lpid" 2>/dev/null
  if [ -d "$GAME_DIR" ]; then
    log_line "game app: still installed after 30 s"
    return 1
  fi
  log_line "game app removed"
  return 0
}

# LumaGame's data directory: the base the feeder chose, recognisable because the UI makes engines/ there on its first start. The
# feeder creates the app's directory under every base it tries, so an existing directory alone says nothing.
game_data_dir() {
  for base in $GAME_DATA_BASES; do
    [ -d "$base/$GAME_APP/engines" ] && { printf '%s\n' "$base/$GAME_APP"; return 0; }
  done
  return 1
}

# The GeForce NOW app's data directory: the first one in the feeder's order that holds a sign-in or preferences.
game_gfn_data_dir() {
  for dir in $(for base in $GAME_DATA_BASES; do printf '%s ' "$base/$GAME_GFN_APP"; done) "$GAME_GFN_LEGACY"; do
    { [ -d "$dir/config" ] || [ -f "$dir/prefs.json" ]; } && { printf '%s\n' "$dir"; return 0; }
  done
  return 1
}

# Jails are per app id, so LumaGame cannot read the old app's files; this root step copies them, keeping their modes, and gives the
# copy to the owner of LumaGame's data directory (its jail user). The originals stay where they are. An existing sign-in in LumaGame
# is never written over.
game_adopt_gfn() {
  GAME_ADOPT_WHY=
  new=$(game_data_dir) || { GAME_ADOPT_WHY="LumaGame has not been started on this TV yet; start it once, then adopt"; return 1; }
  old=$(game_gfn_data_dir) || { GAME_ADOPT_WHY="the GeForce NOW app has no sign-in or preferences to adopt"; return 1; }
  dest=$new/engines/gfn
  if [ -d "$dest/config" ] && [ -n "$(ls -A "$dest/config" 2>/dev/null)" ]; then
    GAME_ADOPT_WHY="LumaGame already has a GeForce NOW sign-in"
    return 1
  fi
  game_close || { GAME_ADOPT_WHY="LumaGame did not close"; return 1; }
  owner=$(stat -c %u:%g "$new") || { GAME_ADOPT_WHY="the owner of $new could not be read"; return 1; }
  stage=$new/engines/.adopt-gfn.$$
  rm -rf "$stage"
  mkdir -p "$stage" || { GAME_ADOPT_WHY="$stage could not be created"; return 1; }
  for name in config data prefs.json; do
    [ -e "$old/$name" ] || continue
    cp -Rp "$old/$name" "$stage/" || { rm -rf "$stage"; GAME_ADOPT_WHY="$old/$name could not be copied"; return 1; }
  done
  chown -R "$owner" "$stage" || { rm -rf "$stage"; GAME_ADOPT_WHY="the copy could not be given to $owner"; return 1; }
  mkdir -p "$dest" && chmod 700 "$dest" && chown "$owner" "$dest" || { rm -rf "$stage"; GAME_ADOPT_WHY="$dest could not be prepared"; return 1; }
  for name in config data prefs.json; do
    [ -e "$stage/$name" ] || continue
    rm -rf "${dest:?}/$name"
    mv "$stage/$name" "$dest/$name" || { rm -rf "$stage"; GAME_ADOPT_WHY="$dest/$name could not be placed"; return 1; }
  done
  rmdir "$stage"
  log_line "game app: adopted the GeForce NOW sign-in from $old into $dest"
  return 0
}

game_status() {
  inst=false
  [ -d "$GAME_DIR" ] && game_complete && inst=true
  cur=false
  game_current && cur=true
  run=false
  game_running && run=true
  printf '{"installed":%s,"build":"%s","shipped":"%s","current":%s,"running":%s,"data":"%s"}\n' \
    "$inst" "$(game_installed_build)" "$(game_shipped_build)" "$cur" "$run" "$(game_data_dir)"
}
