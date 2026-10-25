#!/bin/sh
# Logic tests for ambient-probe on the host, against the fake libvtcapture (build.sh builds both).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
b=$here/build/host
export LD_LIBRARY_PATH=$b
p=$b/ambient-probe
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fail() { echo "FAIL: $*"; exit 1; }

# 1. dry run resolves the four functions and never takes the capture
"$p" -n | grep -q '^library: init=0x' || fail "dry run did not resolve the library"

# 2. a short run: calls, changes, finalize, csv rows, dump header
FAKE_VT_FIN_LOG=$tmp/fin "$p" -s 2 -r 30 -c "$tmp/c.csv" -d "$tmp/d.bin" -L t2 >"$tmp/o2" || fail "run exited non-zero"
grep -q 'preset 0: 16 areas' "$tmp/o2" || fail "no 16-area header"
grep -q 'finalizeHistogram rc=0' "$tmp/o2" || fail "no finalize line"
[ "$(wc -l <"$tmp/fin")" = 1 ] || fail "finalize ran $(wc -l <"$tmp/fin") times"
rows=$(wc -l <"$tmp/c.csv"); [ "$rows" -ge 40 ] && [ "$rows" -le 70 ] || fail "csv rows $rows, expected about 60"
head -c 4 "$tmp/d.bin" | grep -q VTH1 || fail "dump magic"
[ "$(wc -c <"$tmp/d.bin")" = $((16 + 16 * 1540)) ] || fail "dump size $(wc -c <"$tmp/d.bin")"
awk -F'[ =]' '/^summary/ {print}' "$tmp/o2" | grep -q 'changed=' || fail "no summary"

# 3. SIGTERM mid-run still finalizes, exactly once
rm -f "$tmp/fin"
FAKE_VT_FIN_LOG=$tmp/fin "$p" -s 30 -r 20 >"$tmp/o3" & pid=$!
sleep 1; kill -TERM $pid; wait $pid || true
[ "$(wc -l <"$tmp/fin")" = 1 ] || fail "SIGTERM: finalize ran $(wc -l <"$tmp/fin" 2>/dev/null || echo 0) times"
grep -q 'stopped by signal' "$tmp/o3" || fail "SIGTERM not reported"

# 4. denied init: exit 2, nothing to finalize
set +e; FAKE_VT_DENY=1 FAKE_VT_FIN_LOG=$tmp/fin4 "$p" -s 2 >"$tmp/o4"; rc=$?; set -e
[ $rc = 2 ] || fail "deny exit code $rc"
[ ! -e "$tmp/fin4" ] || fail "finalize called without a handle"

# 5. LG policy action stops the run early and still finalizes
rm -f "$tmp/fin"
set +e; FAKE_VT_POLICY_MS=700 FAKE_VT_FIN_LOG=$tmp/fin "$p" -s 20 -r 20 >"$tmp/o5"; rc=$?; set -e
[ $rc = 2 ] || fail "policy exit code $rc"
[ "$(wc -l <"$tmp/fin")" = 1 ] || fail "policy: finalize count"
grep -q 'stopped by LG policy action' "$tmp/o5" || fail "policy not reported"

# 6. sweep walks the four presets with 16, 16, 16 and 14 areas
"$p" -S -s 1 -r 20 >"$tmp/o6" || fail "sweep exit"
for n in "0: 16" "1: 16" "2: 16" "3: 14"; do grep -q "preset $n areas" "$tmp/o6" || fail "sweep missing preset $n"; done

# 7. EAGAIN answers are counted, not treated as data
FAKE_VT_EAGAIN_EVERY=2 "$p" -s 1 -r 30 >"$tmp/o7" || fail "eagain run"
grep -q 'rc: 0 x[0-9]* 11 x[0-9]*' "$tmp/o7" || fail "rc histogram lacks 11: $(grep '^summary' "$tmp/o7")"

echo "ambient-probe host tests: all passed"
