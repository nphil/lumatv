#!/bin/sh
# TV smoke test of lumatv-helper, run ON the set from the directory smoke-tv.py uploads to (default /tmp/lumatv-helper-smoke):
#   lumatv-helper (the static aarch64 binary) and this script.
#
#   sh smoke.sh [PORT]
#
# It starts the helper detached the way tools/lumatv will, curls the loopback API, checks what the set can and cannot do
# for it, stops it, and prints one PASS/FAIL line per check and a VERDICT. Exit 0 = PASS.
# Everything lives in volatile /tmp (data dir, lock); the production data dir and the production port (18731) are never
# touched, the default test port is 18741. The only thing it reaches outside the set is what the helper itself reaches: the
# receiver's status page (read only) and, through `openssl s_client`, the weather and news services.
#
# Phase A runs with the real screen detection (luna-send). If the panel is not on (Active Standby), the receiver must stay
# unpolled and that is what is checked; Phase B then repeats the receiver and feed checks with the screen forced on
# (`screen.source: "on"`), so the run is meaningful in any power state.
#
# Expected on the set (see docs/research/native-helper-result.md): every check PASS; screen source luna-subscribe or luna-poll;
# weather and news fetched; receiver fresh with the same volume as a direct read; idle CPU well under 1 %; RSS about 5-10 MB.
D=${LUMATV_SMOKE_DIR:-$(cd "$(dirname "$0")" && pwd)}
BIN=$D/lumatv-helper
PORT=${1:-18741}
AVR=${LUMATV_AVR:-192.168.1.93:8080}
PASS=0
FAIL=0

ok() { PASS=$((PASS + 1)); echo "PASS  $1"; }
bad() { FAIL=$((FAIL + 1)); echo "FAIL  $1  -- $2"; }
fact() { echo "FACT  $1"; }
clip() { printf '%s' "$1" | tr -d '\n' | cut -c1-${2:-240}; }

# has NAME BODY PATTERN: the compact JSON the helper writes has no spaces, so grep is enough.
has() {
  if printf '%s' "$2" | grep -q -- "$3"; then ok "$1"; else bad "$1" "no '$3' in: $(clip "$2")"; fi
}
get() { curl -s -m "${2:-6}" "http://127.0.0.1:$PORT$1"; }
post() {
  b=$2
  [ -n "$b" ] || b='{}'
  curl -s -m 6 -X POST -H 'X-Luma-Client: smoke' -H 'Content-Type: application/json' -d "$b" "http://127.0.0.1:$PORT$1"
}
field() { printf '%s' "$1" | sed -n "s/.*\"$2\":\"\{0,1\}\([^\",}]*\).*/\1/p" | head -n 1; }

power_state() {
  luna-send -n 1 -f luna://com.webos.service.tvpower/power/getPowerState '{}' 2>/dev/null </dev/null |
    sed -n 's/.*"state" *: *"\([^"]*\)".*/\1/p' | head -n 1
}

# wait_for NAME SECONDS PATH PATTERN: polls the API until the pattern shows.
wait_for() {
  i=0
  while [ "$i" -lt "$(($2 * 4))" ]; do
    body=$(get "$3" 3)
    if printf '%s' "$body" | grep -q -- "$4"; then ok "$1"; LAST=$body; return 0; fi
    sleep 0.25 2>/dev/null || sleep 1
    i=$((i + 1))
  done
  bad "$1" "no '$4' in $3 after $2 s: $(clip "$body")"
  LAST=$body
  return 1
}

cpu_ticks() { awk '{print $14 + $15}' "/proc/$1/stat" 2>/dev/null; }

echo "=== environment"
fact "machine $(uname -m), kernel $(uname -r), uid $(id -u)"
fact "power state: $(power_state)"
fact "curl: $(command -v curl || echo MISSING): $(curl --version 2>&1 | head -n 1)"
fact "openssl: $(command -v openssl || ls /usr/bin/openssl 2>/dev/null || echo MISSING): $(openssl version 2>&1 | head -n 1)"
fact "luna-send: $(command -v luna-send || echo MISSING)"
for b in /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-bundle.crt /etc/pki/tls/certs/ca-bundle.crt /etc/ssl/cert.pem; do
  [ -f "$b" ] && fact "CA bundle: $b ($(wc -c < "$b") bytes)"
done
fact "nobody: $(grep '^nobody:' /etc/passwd || echo MISSING)"
fact "MemAvailable: $(awk '/MemAvailable/ {print $2 " kB"}' /proc/meminfo)"
fact "/tmp: $(df /tmp | tail -n 1 | awk '{print $1 " " $4 " kB free"}'); mount: $(grep ' /tmp ' /proc/mounts | head -n 1 | cut -c1-120)"

# luna-send subscriptions through a pipe: does the output arrive, or does stdio hold it back? The helper subscribes with
# `-n 1000000` (the form tools/lumatv uses for subscribing calls) and polls instead when nothing arrives; `-i` is probed for the
# record. Only the probe's own luna-send is ever killed (its pid is recorded), never another program's.
for form in "-n 1000000" "-i"; do
  rm -f "$D/lsub.out" "$D/lsub.pid"
  ( luna-send $form luna://com.webos.service.tvpower/power/getPowerState '{"subscribe":true}' </dev/null 2>/dev/null & echo $! > "$D/lsub.pid"; wait ) | head -n 1 > "$D/lsub.out" &
  sleep 3
  [ -s "$D/lsub.pid" ] && kill "$(cat "$D/lsub.pid")" 2>/dev/null
  sleep 1
  if [ -s "$D/lsub.out" ]; then fact "luna-send $form through a pipe delivers: $(clip "$(cat "$D/lsub.out")" 160)"; else fact "luna-send $form through a pipe delivered nothing in 3 s"; fi
done
rm -f "$D/lsub.out" "$D/lsub.pid"

# Processes of the kinds the helper starts, by pid: what exists before the run is not the helper's to account for.
children() { ps w 2>/dev/null | grep '[o]penssl s_client\|[l]una-send -n 1000000\|[c]url .*--max-filesize' | awk '{print $1}' | sort; }
CHILDREN_BEFORE=" $(children | tr '\n' ' ')"

echo "=== the binary"
chmod +x "$BIN" 2>/dev/null
version=$("$BIN" version 2>&1)
has "the static aarch64 binary runs on the set" "$version" '"ok":true'
fact "version: $(clip "$version" 160)"
if curl -s -m 2 "http://127.0.0.1:$PORT/v1/health" >/dev/null 2>&1; then
  bad "the test port $PORT is free" "something answers on it"
  echo "VERDICT FAIL: pick another port: sh smoke.sh PORT"
  exit 1
fi

# run_phase NAME SCREEN_SOURCE DATA_DIR
run_phase() {
  name=$1
  src=$2
  data=$3
  echo "=== $name"
  rm -rf "$data"
  mkdir -p "$data"
  printf '{"listen":{"port":%s},"log":{"level":"debug"},"screen":{"source":"%s"}}\n' "$PORT" "$src" > "$data/helper.json"
  cfg=$("$BIN" check-config --data-dir "$data" 2>&1)
  has "check-config reads the config" "$cfg" '"ok":true'
  fact "check-config: $(clip "$cfg" 1400)"

  started=$("$BIN" start --data-dir "$data" 2>&1)
  has "start detaches and reports the port" "$started" '"started":true'
  pid=$(field "$started" pid)
  fact "start: $(clip "$started")"
  [ -n "$pid" ] || { bad "a pid was reported" "$started"; return 1; }
  status=$("$BIN" status --data-dir "$data" 2>&1)
  has "status says running and healthy" "$status" '"healthy":true'
  has "a second start changes nothing" "$("$BIN" start --data-dir "$data" 2>&1)" '"already":true'

  has "GET /v1/health" "$(get /v1/health)" '"ok":true'
  state=$(get /v1/state)
  has "GET /v1/state carries the api version, the helper and the topics" "$state" '"api":1.*"helper".*"topics"'
  fact "helper: rss $(awk '/VmRSS/ {print $2}' /proc/$pid/status) kB, threads $(awk '/Threads/ {print $2}' /proc/$pid/status), fds $(ls /proc/$pid/fd | wc -l), nice $(awk '{print $19}' /proc/$pid/stat)"

  # Screen.
  wait_for "the screen topic is published" 10 /v1/screen '"on":'
  scr=$LAST
  fact "screen: $(clip "$scr" 300)"
  direct=$(power_state)
  seen=$(field "$scr" state)
  if [ "$src" = luna ]; then
    if [ "$seen" = "$direct" ]; then ok "the helper's screen state equals luna-send's ($direct)"; else bad "the helper's screen state equals luna-send's" "helper '$seen', luna-send '$direct'"; fi
    if printf '%s' "$scr" | grep -q '"source":"assumed"'; then bad "the power state is read, not assumed" "$(clip "$scr")"; else ok "the power state is read, not assumed"; fi
  fi

  # The long poll.
  seq=$(field "$(get /v1/state)" seq)
  boot=$(field "$(get /v1/state)" boot)
  t0=$(date +%s)
  quiet=$(get "/v1/events?after=$seq&boot=$boot&timeout=2&topics=smoke.none" 8)
  t1=$(date +%s)
  has "a long poll with nothing to report returns empty" "$quiet" '"events":\[\]'
  if [ "$((t1 - t0))" -ge 2 ] && [ "$((t1 - t0))" -le 4 ]; then ok "the long poll waited its 2 s"; else bad "the long poll waited its 2 s" "took $((t1 - t0)) s"; fi

  # Settings, in the temporary data dir only.
  owners=$(get /v1/settings)
  has "GET /v1/settings lists the weather owner" "$owners" '"base.weather"'
  has "GET /v1/settings lists the news owner" "$owners" '"base.news"'
  change=$(post /v1/settings/base.weather '{"changes":[{"key":"units","value":{"type":"choice","value":"celsius"}}]}')
  has "a settings change is accepted" "$change" '"changed":true'
  has "a refused setting says which field" "$(post /v1/settings/base.weather '{"changes":[{"key":"zip","value":{"type":"text","value":"abc"}}]}')" '"zip"'
  post /v1/settings/base.weather '{"changes":[{"key":"units","value":{"type":"choice","value":"fahrenheit"}}]}' >/dev/null
  has "a write without the client header is refused" "$(curl -s -m 5 -X POST -H 'Content-Type: application/json' -d '{"changes":[]}' "http://127.0.0.1:$PORT/v1/settings/base.news")" 'client_header_required'
  has "a request that names another host is refused" "$(curl -s -m 5 -H 'Host: evil.example' "http://127.0.0.1:$PORT/v1/state")" 'host_not_allowed'
  has "a request from a web page (Origin) is refused" "$(curl -s -m 5 -H 'Origin: http://evil.example' "http://127.0.0.1:$PORT/v1/state")" 'origin_not_allowed'
  lan=$(ip route get 1.1.1.1 2>/dev/null | sed -n 's/.* src \([0-9.]*\).*/\1/p' | head -n 1)
  [ -n "$lan" ] || lan=$(hostname -i 2>/dev/null | awk '{print $1}')
  case "$lan" in
    '' | 127.*) fact "LAN address unknown: the LAN-closed check was skipped" ;;
    *)
      if curl -s -m 2 "http://$lan:$PORT/v1/health" >/dev/null 2>&1; then
        bad "the API is not reachable on the LAN address $lan" "it answered"
      else
        ok "the API is not reachable on the LAN address $lan"
      fi
      ;;
  esac

  # Feeds, fetched through openssl s_client on the set (a refresh fetches even with the screen off).
  post /v1/refresh >/dev/null
  if wait_for "the weather is fetched on the set" 40 /v1/weather '"state":"fresh"'; then
    fact "weather: $(field "$LAST" place) $(printf '%s' "$LAST" | sed -n 's/.*"temp":"\([^"]*\)".*/\1/p' | head -n 1)"
  else
    fact "weather status: $(clip "$LAST" 500)"
  fi
  if wait_for "the news are fetched on the set" 40 /v1/news '"state":"fresh"'; then
    fact "news: $(printf '%s' "$LAST" | grep -o '"title":"[^"]*"' | head -n 2 | tr '\n' ' ')"
  else
    fact "news status: $(clip "$LAST" 500)"
  fi
  has "the documents are saved on disk" "$(ls "$data/cache" 2>/dev/null | tr '\n' ' ')" 'news.json'
  has "the weather file is the document" "$(cat "$data/cache/weather.json" 2>/dev/null)" '"fetchedAt"'

  # The receiver.
  if [ "$src" = on ] || printf '%s' "$scr" | grep -q '"on":true'; then
    wait_for "the receiver is read while the screen is on" 15 /v1/receiver '"fresh":true' &&
      {
        rec=$LAST
        fact "receiver: $(clip "$rec" 400)"
        direct_xml=$(curl -s -m 5 "http://$AVR/goform/formMainZone_MainZoneXmlStatusLite.xml")
        direct_vol=$(printf '%s' "$direct_xml" | tr -d '\n' | sed -n 's/.*<MasterVolume><value>\([^<]*\)<.*/\1/p')
        seen_vol=$(field "$rec" volumeDb)
        if [ -n "$direct_vol" ] && [ "$seen_vol" = "$direct_vol" ]; then ok "the volume equals a direct read of the receiver ($direct_vol dB)"; else bad "the volume equals a direct read of the receiver" "helper '$seen_vol', direct '$direct_vol'"; fi
      }
    has "a nudge is accepted" "$(post /v1/receiver/nudge)" '"requested":true'
  else
    has "the receiver stays idle while the screen is off" "$(get /v1/receiver)" '"polling":false'
    has "a nudge with the screen off is accepted" "$(post /v1/receiver/nudge)" '"requested":true'
  fi

  # Idle cost: ten quiet seconds.
  c0=$(cpu_ticks "$pid")
  sleep 10
  c1=$(cpu_ticks "$pid")
  hz=$(getconf CLK_TCK 2>/dev/null || echo 100)
  used=$((c1 - c0))
  fact "idle: $used ticks of CPU in 10 s ($(awk "BEGIN {printf \"%.2f\", $used / $hz / 10 * 100}") % of one core), rss $(awk '/VmRSS/ {print $2}' /proc/$pid/status) kB"
  if [ "$used" -le "$((hz / 2))" ]; then ok "the helper is quiet when idle (at most 5 % of a core)"; else bad "the helper is quiet when idle" "$used ticks in 10 s"; fi
  rss=$(awk '/VmRSS/ {print $2}' /proc/$pid/status)
  if [ "${rss:-99999}" -lt 20000 ]; then ok "resident memory under 20 MB"; else bad "resident memory under 20 MB" "$rss kB"; fi

  stopped=$("$BIN" stop --data-dir "$data" 2>&1)
  has "stop ends it" "$stopped" '"stopped":true'
  if curl -s -m 2 "http://127.0.0.1:$PORT/v1/health" >/dev/null 2>&1; then bad "the port is closed after stop" "still answering"; else ok "the port is closed after stop"; fi
  if [ -d "/proc/$pid" ]; then bad "the process is gone after stop" "pid $pid is alive"; else ok "the process is gone after stop"; fi
  left=""
  for p in $(children); do
    case "$CHILDREN_BEFORE" in *" $p "*) ;; *) left="$left $p" ;; esac
  done
  if [ -z "$left" ]; then ok "no child process is left behind"; else bad "no child process is left behind" "new pids:$left"; fi
  rm -rf "$data"
}

run_phase "Phase A: real screen detection" luna "$D/data-a"
# Phase B only when the panel was not on in Phase A: the receiver and feeds are then checked with the screen forced on.
if [ "$(power_state)" != Active ]; then
  run_phase "Phase B: screen forced on" on "$D/data-b"
fi

echo "=== result"
echo "$PASS passed, $FAIL failed"
if [ "$FAIL" -eq 0 ]; then echo "VERDICT PASS"; exit 0; else echo "VERDICT FAIL"; exit 1; fi
