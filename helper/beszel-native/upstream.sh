#!/bin/sh
# What moved upstream in the files that define the hub-agent protocol, between the Beszel version beszel-native speaks (`UPSTREAM`
# in src/system.rs) and <tag>: the messages and actions, the stats they carry, how the hub takes an agent, and how the agent dials
# and answers. Prints a unified diff; exits 0 when nothing in them changed, 1 when something did, 2 when they could not be fetched.
#   helper/beszel-native/upstream.sh v0.21.2
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
[ $# -eq 1 ] || { echo "usage: $0 <tag>" >&2; exit 2; }
PINNED=v$(sed -n 's/^pub const UPSTREAM: &str = "\(.*\)";$/\1/p' "$HERE/src/system.rs")
TAG=v${1#v}
[ "$PINNED" != v ] || { echo "upstream.sh: no UPSTREAM in $HERE/src/system.rs" >&2; exit 2; }
FILES="internal/common/common-ws.go internal/entities/system/system.go internal/hub/agent_connect.go agent/client.go agent/handlers.go"

WORK=$(mktemp -d "${CODY_DATA_SCRATCH:-${TMPDIR:-/tmp}}/beszel-upstream.XXXXXX")
trap 'rm -rf "$WORK"' EXIT
for side in "$PINNED" "$TAG"; do
  fetched=0
  for f in $FILES; do
    mkdir -p "$WORK/$side/$(dirname "$f")"
    status=0
    curl -fsL "https://raw.githubusercontent.com/henrygd/beszel/$side/$f" -o "$WORK/$side/$f" || status=$?
    # 22 is an HTTP error: a file missing at one tag shows in the diff as added or removed whole
    case $status in
      0) fetched=$((fetched + 1)) ;;
      22) rm -f "$WORK/$side/$f" ;;
      *) echo "upstream.sh: $side/$f could not be fetched (curl $status)" >&2; exit 2 ;;
    esac
  done
  [ "$fetched" -gt 0 ] || { echo "upstream.sh: none of the files is at $side (no such tag?)" >&2; exit 2; }
done
cd "$WORK"
diff -ruN "$PINNED" "$TAG"
