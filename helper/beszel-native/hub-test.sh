#!/bin/sh
# Runs beszel-native against a real Beszel hub of [version] (default: the version it speaks, `UPSTREAM` in src/system.rs) on this
# machine. Passes when the hub has the system up and a stats record with CPU, memory, disk, the temperature of a fake SoC sensor
# (laid out as the helper lays out the TV's) and the load of a fake GPU sample (as the helper writes it).
#   helper/beszel-native/hub-test.sh [version]
#   HUB_TEST_AGENT=official      runs Beszel's own agent of that version instead, the payload's nvtop handing it the GPU sample
#   HUB_TEST_MEASURE=<seconds>   then keeps the hub's live view open that long (a request a second) and prints the agent's CPU
#                                time and memory
# The hub and Beszel's agent (hash-checked against the release's checksums) are cached in $HOME/.cache/beszel-hub; the hub's data
# and the agent's live in a scratch directory under $CODY_DATA_SCRATCH (else $TMPDIR, else /tmp), removed at the end.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
UPSTREAM=$(sed -n 's/^pub const UPSTREAM: &str = "\(.*\)";$/\1/p' "$HERE/src/system.rs")
VERSION=${1:-$UPSTREAM}
VERSION=${VERSION#v}
AGENT=${HUB_TEST_AGENT:-native}
MEASURE=${HUB_TEST_MEASURE:-0}
CACHE=$HOME/.cache/beszel-hub/$VERSION

say() { printf '%s\n' "$*" >&2; }
die() { say "hub-test.sh: $*"; exit 1; }
for tool in curl jq tar sha256sum awk; do
  command -v "$tool" >/dev/null 2>&1 || die "$tool is not on PATH"
done
case $(uname -m) in
  x86_64) ARCH=amd64 ;;
  aarch64) ARCH=arm64 ;;
  *) die "no Beszel release for $(uname -m)" ;;
esac
case $AGENT in native | official) ;; *) die "HUB_TEST_AGENT is native or official, not $AGENT" ;; esac

# The release's <asset> in the cache, checked against the release's checksums, and unpacked.
fetch() {
  asset=$1
  base=https://github.com/henrygd/beszel/releases/download/v$VERSION
  mkdir -p "$CACHE"
  if [ ! -s "$CACHE/checksums.txt" ]; then
    curl -fsSL "$base/beszel_${VERSION}_checksums.txt" -o "$CACHE/checksums.part" || die "no release v$VERSION"
    mv "$CACHE/checksums.part" "$CACHE/checksums.txt"
  fi
  want=$(awk -v a="$asset" '$2 == a { print $1 }' "$CACHE/checksums.txt")
  [ -n "$want" ] || die "$asset is not in the checksums of v$VERSION"
  if [ "$(sha256sum "$CACHE/$asset" 2>/dev/null | cut -d' ' -f1)" != "$want" ]; then
    curl -fsSL "$base/$asset" -o "$CACHE/$asset.part"
    [ "$(sha256sum "$CACHE/$asset.part" | cut -d' ' -f1)" = "$want" ] || die "$asset does not match its checksum"
    mv "$CACHE/$asset.part" "$CACHE/$asset"
  fi
  tar -xzf "$CACHE/$asset" -C "$CACHE"
}

fetch "beszel_linux_$ARCH.tar.gz"
if [ "$AGENT" = native ]; then
  "$REPO/helper/build.sh" host
  PROGRAM=$REPO/helper/target/release/beszel-native
else
  fetch "beszel-agent_linux_$ARCH.tar.gz"
  PROGRAM=$CACHE/beszel-agent
fi

WORK=$(mktemp -d "${CODY_DATA_SCRATCH:-${TMPDIR:-/tmp}}/beszel-hub-test.XXXXXX")
HUB_PID='' AGENT_PID='' LIVE_PID=''
cleanup() {
  for pid in $LIVE_PID $AGENT_PID $HUB_PID; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
  rm -rf "$WORK"
}
trap cleanup EXIT
trap 'exit 1' INT TERM
# Fails with the end of both logs and the last stats record.
fail() {
  for log in hub.log agent.log stats.json; do
    [ -s "$WORK/$log" ] && { say "--- $log"; tail -n 20 "$WORK/$log" >&2; }
  done
  die "$*"
}
# Runs the command after <what> and <seconds> every 0.2 s until it succeeds, failing when <seconds> are up.
wait_for() {
  what=$1 tries=$(($2 * 5))
  shift 2
  until "$@"; do
    tries=$((tries - 1))
    [ "$tries" -gt 0 ] || fail "$what: not within the time"
    sleep 0.2
  done
}

PORT=$((20000 + $$ % 40000))
API=http://127.0.0.1:$PORT/api
"$CACHE/beszel" serve --http "127.0.0.1:$PORT" --dir "$WORK/hub" > "$WORK/hub.log" 2>&1 &
HUB_PID=$!
healthy() {
  kill -0 "$HUB_PID" 2>/dev/null || fail "the hub ended"
  curl -fs "$API/health" >/dev/null
}
wait_for "the hub" 30 healthy

EMAIL=hub-test@example.com
PASSWORD=hub-test-password
JSON='Content-Type: application/json'
curl -fsS -H "$JSON" -d "{\"email\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" "$API/beszel/create-user" >/dev/null
AUTH=$(curl -fsS -H "$JSON" -d "{\"identity\":\"$EMAIL\",\"password\":\"$PASSWORD\"}" "$API/collections/users/auth-with-password")
BEARER=$(printf '%s' "$AUTH" | jq -r .token)
USER_ID=$(printf '%s' "$AUTH" | jq -r .record.id)
get() { curl -fsS -H "Authorization: $BEARER" "$API/$1"; }
post() { curl -fsS -H "Authorization: $BEARER" -H "$JSON" -d "$2" "$API/$1"; }
KEY=$(get beszel/getkey | jq -r .key)
# the hub dials host:port only for an agent that does not dial it: a port nothing listens on
SYSTEM=$(post collections/systems/records \
  "{\"name\":\"hub-test\",\"host\":\"127.0.0.1\",\"port\":\"$((PORT + 1))\",\"users\":[\"$USER_ID\"]}" | jq -r .id)
TOKEN=$(cat /proc/sys/kernel/random/uuid)
post collections/fingerprints/records "{\"system\":\"$SYSTEM\",\"token\":\"$TOKEN\"}" >/dev/null

hwmon=$WORK/sys/class/hwmon/hwmon0
mkdir -p "$hwmon" "$WORK/agent"
echo soc > "$hwmon/name"
echo 44000 > "$hwmon/temp1_input"
printf '%s\n' '[{"device_name":"Mali-G52 MC3","gpu_util":"37%","mem_used":"41943040","mem_total":"2147483648"}]' > "$WORK/gpu"
set -- env -i PATH="$REPO/payload/beszel:/usr/bin:/bin" HOME="$WORK/agent" DATA_DIR="$WORK/agent" HUB_URL="http://127.0.0.1:$PORT" \
  KEY="$KEY" TOKEN="$TOKEN" DISABLE_SSH=true SKIP_SYSTEMD=true LOG_LEVEL=info SYS_SENSORS="$WORK/sys"
if [ "$AGENT" = native ]; then
  set -- "$@" GPU_SAMPLE_FILE="$WORK/gpu"
else
  set -- "$@" GPU_COLLECTOR=nvtop LUMATV_GPU_SAMPLE="$WORK/gpu"
fi
"$@" "$PROGRAM" > "$WORK/agent.log" 2>&1 &
AGENT_PID=$!

up() {
  kill -0 "$AGENT_PID" 2>/dev/null || fail "the agent ended"
  [ "$(get "collections/systems/records/$SYSTEM" | jq -r .status)" = up ]
}
wait_for "the system up" 60 up
# Beszel's agent may answer the first request before nvtop's first sample: a later record has it
stats_complete() {
  get "collections/system_stats/records?filter=system%3D%22$SYSTEM%22&sort=-created&perPage=1" | jq -c '.items[0].stats // empty' \
    > "$WORK/stats.json"
  jq -e 'has("cpu") and .m > 0 and .mp > 0 and .d > 0 and .t.soc == 44
    and (.g | to_entries[0].value | .n == "Mali-G52 MC3" and .u == 37)' "$WORK/stats.json" >/dev/null
}
wait_for "a stats record with CPU, memory, disk, temperature and GPU" 150 stats_complete
say "hub v$VERSION, $AGENT agent: up; stats $(cat "$WORK/stats.json")"

if [ "$MEASURE" -gt 0 ]; then
  # the live view is PocketBase's event stream: its first event names the client, which then subscribes to the system's metrics
  curl -fsN -H "Authorization: $BEARER" "$API/realtime" > "$WORK/live" &
  LIVE_PID=$!
  CLIENT=''
  connected() {
    CLIENT=$(sed -n 's/^data:{"clientId":"\([^"]*\)".*/\1/p' "$WORK/live")
    [ -n "$CLIENT" ]
  }
  wait_for "the live view" 10 connected
  topic=rt_metrics?options=$(printf '{"query":{"system":"%s"}}' "$SYSTEM" | jq -sRr @uri)
  post realtime "{\"clientId\":\"$CLIENT\",\"subscriptions\":[\"$topic\"]}" >/dev/null
  # CPU ticks (its own and its reaped children's) and resident kB of the agent and what it started
  usage() {
    ticks=0 kb=0
    for pid in $AGENT_PID $(cat "/proc/$AGENT_PID"/task/*/children 2>/dev/null); do
      ticks=$((ticks + $(awk '{ print $14 + $15 + $16 + $17 }' "/proc/$pid/stat" 2>/dev/null || echo 0)))
      kb=$((kb + $(awk '/^VmRSS:/ { print $2 }' "/proc/$pid/status" 2>/dev/null || echo 0)))
    done
    echo "$ticks $kb"
  }
  set -- $(usage)
  before=$1
  sleep "$MEASURE"
  set -- $(usage)
  kill -0 "$AGENT_PID" 2>/dev/null || fail "the agent ended"
  peak=$(awk '/^VmHWM:/ { print $2 }' "/proc/$AGENT_PID/status")
  updates=$(grep -c '^event:rt_metrics' "$WORK/live" || true)
  say "$(awk -v t=$(($1 - before)) -v hz="$(getconf CLK_TCK)" -v s="$MEASURE" -v kb="$2" -v peak="$peak" -v n="$updates" -v a="$AGENT" \
    'BEGIN { printf "%s agent, %d s of live view (%d updates): CPU %.2f s (%.3f%% of a core); resident %d kB (agent peak %d kB)", a, s, n, t / hz, 100 * t / hz / s, kb, peak }')"
fi
