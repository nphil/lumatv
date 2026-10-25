#!/bin/sh
# The card app (docs/research/cardapp-result.md) as `tools/lumatv` installs it: a native app of its own, packed by tools/build-ipk.sh
# into payload/card/<appId>.card.ipk inside the main package and put on the set through appInstallService, the way the frame app is.
# POSIX sh and busybox ash only. Sourced by tools/lumatv; tests/test_cardapp.py sources it with a fake luna-send.
#
#   card_install   install the shipped ipk, or update the app when the shipped version differs; 0 when the shipped version is there
#   card_remove    remove the app through appInstallService, and 0 when its directory is gone
#   card_status    one JSON line: {"installed":bool,"version":"","shipped":"","running":bool}
#
# Expects from the caller: APPDIR (the main app's directory), APP_ID, SHORT_ID and log_line. CARD_APP, CARD_IPK, CARD_DIR, CARD_LUNA and
# CARD_WAIT (seconds to wait for an install) default to the set's and exist so the host test can point them elsewhere.
#
# The install is `appInstallService/dev/install`, nothing else: no role file is written and nothing of LG's is restarted. Whether that
# install lets the app's launcher register with SAM is the TV step list's first question (docs/research/cardapp-result.md).

CARD_APP="${CARD_APP:-$APP_ID.card}"
CARD_IPK="${CARD_IPK:-$APPDIR/payload/card/$CARD_APP.ipk}"
CARD_DIR="${CARD_DIR:-$(dirname "$APPDIR")/$CARD_APP}"
CARD_LUNA="${CARD_LUNA:-luna-send}"
CARD_WAIT="${CARD_WAIT:-90}"

card_shipped_version() {
  ar p "$CARD_IPK" control.tar.gz 2>/dev/null | tar -xzO control 2>/dev/null | sed -n 's/^Version: //p'
}

card_installed_version() {
  sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' "$CARD_DIR/appinfo.json" 2>/dev/null | head -n 1
}

# Whether the installed app has what SAM and the launcher need: the launcher (`main`), the 64-bit app and the runtime.
card_complete() {
  [ -x "$CARD_DIR/bin/lumatv-card" ] && [ -x "$CARD_DIR/bin64/lumatv-card-app" ] && [ -x "$CARD_DIR/gfx64/launch.sh" ] && [ -f "$CARD_DIR/gfx64/heapshim.so" ]
}

card_running() {
  pgrep -f "$CARD_DIR/bin" >/dev/null 2>&1
}

# Asks SAM to close the app first when it runs: an update cannot overwrite a running executable (text busy).
card_close() {
  card_running || return 0
  "$CARD_LUNA" -n 1 -f luna://com.webos.applicationManager/closeByAppId "{\"id\":\"$CARD_APP\"}" >/dev/null 2>&1
  n=0
  while card_running && [ "$n" -lt 10 ]; do sleep 1; n=$((n + 1)); done
}

card_install() {
  [ -f "$CARD_IPK" ] || return 0
  shipped=$(card_shipped_version)
  [ -n "$shipped" ] || { log_line "card app: the shipped ipk has no version"; return 1; }
  if [ "$(card_installed_version)" = "$shipped" ] && card_complete; then return 0; fi
  card_close
  tmp="/tmp/$SHORT_ID-card.ipk"
  cp -f "$CARD_IPK" "$tmp" 2>/dev/null || { log_line "card app: the ipk could not be copied to /tmp"; return 1; }
  out=$("$CARD_LUNA" -n 3 -f luna://com.webos.appInstallService/dev/install "{\"id\":\"$CARD_APP\",\"ipkUrl\":\"$tmp\",\"subscribe\":true}" 2>&1 | tr -d '\n')
  i=0
  while [ "$i" -lt "$CARD_WAIT" ]; do
    [ "$(card_installed_version)" = "$shipped" ] && card_complete && break
    sleep 1
    i=$((i + 1))
  done
  rm -f "$tmp"
  if [ "$(card_installed_version)" = "$shipped" ] && card_complete; then
    log_line "card app $shipped installed"
    return 0
  fi
  log_line "card app install did not complete: $(printf '%s' "$out" | cut -c1-200)"
  return 1
}

card_remove() {
  [ -d "$CARD_DIR" ] || return 0
  card_close
  "$CARD_LUNA" -n 3 -f luna://com.webos.appInstallService/dev/remove "{\"id\":\"$CARD_APP\",\"subscribe\":true}" >/dev/null 2>&1 &
  lpid=$!
  w=0
  while [ -d "$CARD_DIR" ] && [ "$w" -lt 30 ]; do sleep 1; w=$((w + 1)); done
  kill "$lpid" 2>/dev/null
  if [ -d "$CARD_DIR" ]; then
    log_line "card app: still installed after 30 s"
    return 1
  fi
  log_line "card app removed"
  return 0
}

card_status() {
  inst=false
  [ -d "$CARD_DIR" ] && card_complete && inst=true
  run=false
  card_running && run=true
  printf '{"installed":%s,"version":"%s","shipped":"%s","running":%s}\n' "$inst" "$(card_installed_version)" "$(card_shipped_version)" "$run"
}
