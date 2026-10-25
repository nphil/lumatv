#!/bin/sh
# A stand-in for the card app, for the host test of lumatv-card.c: what the launcher starts when LUMATV_CARD_CHILD names this.
# It writes its arguments and every line the launcher sends on descriptor 3 to $FAKE_APP_LOG, says it is ready, and ends on a
# `close` line (or, with FAKE_APP_IGNORE_CLOSE, never does) or after FAKE_APP_EXIT seconds with that status.
echo "argv: $*" >> "$FAKE_APP_LOG"
echo "env: XDG_RUNTIME_DIR=$XDG_RUNTIME_DIR WAYLAND_DISPLAY=$WAYLAND_DISPLAY" >> "$FAKE_APP_LOG"
echo '{"kind":"ready"}' >&3
echo '{"kind":"state","state":"fullscreen"}' >&3
if [ -n "$FAKE_APP_EXIT" ]; then
  ( while IFS= read -r line <&3; do echo "got: $line" >> "$FAKE_APP_LOG"; done ) &
  sleep "${FAKE_APP_EXIT%%:*}"; exit "${FAKE_APP_EXIT##*:}"
fi
trap '' TERM
[ -z "$FAKE_APP_HONOUR_TERM" ] || trap 'echo "got TERM" >> "$FAKE_APP_LOG"; exit 0' TERM
while :; do
  if IFS= read -r line <&3; then
    echo "got: $line" >> "$FAKE_APP_LOG"
    case "$line" in *'"kind":"close"'*) [ -n "$FAKE_APP_IGNORE_CLOSE" ] || exit 0 ;; esac
  else
    echo "channel closed" >> "$FAKE_APP_LOG"; exit 0
  fi
done
