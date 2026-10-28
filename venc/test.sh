#!/bin/sh
# Host tests of the encoder path: the unit tests against the fake kernel, then the probe binary itself in
# its synth, vt and serve modes against the same fake (build.sh builds both). Nothing here touches a device.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
b=$here/build/host
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"; kill $srv 2>/dev/null || true' EXIT
fail() { echo "FAIL: $*"; exit 1; }
srv=

"$b/venc_test" || fail "unit tests"

# 1. synth: 40 frames, GOP 10 -> 4 I frames, each starting with SPS+PPS; a JSON summary with the counts
"$b/lumatv-venc" synth -w 640 -h 360 -r 400 -n 40 -g 10 -o "$tmp/s.h264" --csv "$tmp/s.csv" >"$tmp/s.out" || fail "synth exited non-zero"
tail -n 1 "$tmp/s.out" | grep -q '"frames":40,"keyframes":4' || fail "synth summary: $(tail -n 1 "$tmp/s.out")"
tail -n 1 "$tmp/s.out" | grep -q '"first_key":{"bytes":4070,"sps":1,"pps":1,"idr":1,"profile_idc":100,"level_idc":41}' || fail "synth first key"
[ "$(wc -l <"$tmp/s.csv")" = 41 ] || fail "csv rows $(wc -l <"$tmp/s.csv")"
python3 - "$tmp/s.h264" <<'EOF' || fail "synth stream content"
import re, sys
d = open(sys.argv[1], 'rb').read()
types = [d[m.start() + 3] & 0x1f for m in re.finditer(rb'\x00\x00\x01', d)]
assert types.count(7) == 4 and types.count(8) == 4 and types.count(5) == 4 and types.count(1) == 36, types
EOF

# 2. vt: the tap's frames reach the encoder by physical address; refusals and timeouts are counted, not fatal
"$b/lumatv-venc" vt -w 640 -h 368 -n 25 -g 5 -l osd -o "$tmp/v.h264" >"$tmp/v.out" || fail "vt exited non-zero"
tail -n 1 "$tmp/v.out" | grep -q '"frames":25,"keyframes":5,"refused":0' || fail "vt summary: $(tail -n 1 "$tmp/v.out")"
grep -q '^tap: osd 640x368 stride 640, 4 buffers at 0x' "$tmp/v.out" || fail "vt buffer line"
set +e
FAKE_VT_PROTECTED=1 timeout 5 "$b/lumatv-venc" vt -w 640 -h 368 -n 3 -l blended -o "$tmp/p.h264" >"$tmp/p.out"; rc=$?
set -e
[ "$rc" = 124 ] || fail "protected run should keep waiting (rc $rc)"
[ ! -s "$tmp/p.h264" ] || fail "protected content produced bytes"
set +e
"$b/lumatv-venc" vt -w 1000 -h 368 -n 3 -o "$tmp/w.h264" >"$tmp/w.out" 2>"$tmp/w.err"; rc=$?
set -e
[ "$rc" = 3 ] || fail "width 1000 should be refused at the tap (rc $rc)"
grep -q 'multiple of 16' "$tmp/w.err" || fail "width refusal message"

# 3. serve: one client gets an Annex B stream that starts with a keyframe; the active file exists while
#    the client is connected and is gone afterwards
port=$((20000 + $$ % 20000))
"$b/lumatv-venc" serve -w 640 -h 368 -l blended -g 15 -p "$port" --active-file "$tmp/active" >"$tmp/srv.out" 2>&1 &
srv=$!
sleep 0.5
python3 - "$port" "$tmp/active" "$tmp/got.h264" <<'EOF' || fail "serve client"
import socket, sys, time, os
port, active, out = int(sys.argv[1]), sys.argv[2], sys.argv[3]
s = socket.create_connection(("127.0.0.1", port), timeout=5)
buf = b""
while len(buf) < 60000:
    chunk = s.recv(65536)
    if not chunk:
        break
    buf += chunk
assert os.path.exists(active), "active file missing while connected"
line = open(active).read()
assert '"what":"screen-encode"' in line and '"location":"blended"' in line, line
s.close()
open(out, "wb").write(buf)
assert buf[:5] == b"\x00\x00\x00\x01\x67", buf[:8]
for _ in range(50):
    if not os.path.exists(active):
        break
    time.sleep(0.1)
assert not os.path.exists(active), "active file left behind"
EOF
kill -TERM $srv; wait $srv || true
srv=
grep -q '^serve: client 127.0.0.1' "$tmp/srv.out" || fail "serve did not log the client"
grep -q '"route":"serve:/dev/video60->/dev/venc->tcp"' "$tmp/srv.out" || fail "serve summary"

echo "venc tests: ok"
