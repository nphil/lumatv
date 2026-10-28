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

# 0. the small steps, in the slot's order
"$b/lumatv-venc" gate >"$tmp/g.out" || fail "gate"
grep -q '"step":"gate","ok":true' "$tmp/g.out" || fail "gate summary"
"$b/lumatv-venc" create -w 320 -h 240 >"$tmp/c.out" || fail "create"
grep -q '^venc: instance 0; es 0x80100000+0x270000' "$tmp/c.out" || fail "create buffer info: $(cat "$tmp/c.out")"
"$b/lumatv-venc" map -w 320 -h 240 >"$tmp/m.out" || fail "map"
grep -q '^map: ES ring mapped at' "$tmp/m.out" || fail "map line"
"$b/lumatv-venc" hma -w 320 -h 240 >"$tmp/h.out" || fail "hma"
grep -q 'counter 0x1234 (expected 0x1234)' "$tmp/h.out" || fail "hma readback"
"$b/lumatv-venc" tapopen -l osd -w 640 -h 368 --nbuf 2 >"$tmp/t.out" || fail "tapopen"
grep -q '2 buffers queued, no streaming' "$tmp/t.out" || fail "tapopen line"
grep -q '"step":"tapopen","ok":true' "$tmp/t.out" || fail "tapopen summary"
set +e
FAKE_VENC_DENY_MAGIC=1 "$b/lumatv-venc" gate >"$tmp/gd.out" 2>&1; rc=$?
set -e
[ "$rc" = 2 ] || fail "denied gate should exit 2 (rc $rc)"
grep -q 'KADP gate' "$tmp/gd.out" || fail "denied gate message"
set +e
FAKE_VENC_HANG=1 "$b/lumatv-venc" synth -w 320 -h 240 -n 2 -o "$tmp/hang.h264" >"$tmp/hang.out" 2>&1; rc=$?
set -e
[ "$rc" = 75 ] || fail "a hung encode should end with 75 (rc $rc)"
grep -q 'watchdog: LX_VENC_IOWR_ENC_ENCODE did not return within 3000 ms' "$tmp/hang.out" || fail "watchdog message"
"$b/lumatv-venc" caps >"$tmp/caps.out" || fail "caps"
grep -q '^pools parsed:' "$tmp/caps.out" || fail "caps pools line"

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
grep -q '^venc: instance 0; es 0x80100000+0x270000' "$tmp/v.out" || fail "vt buffer info after create"
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

# 4. feed: the compositor's recorder protocol (feed_proto.h). The program says it is ready, takes NV12 frames on stdin, answers each with an
#    access unit on stdout (the frame's pts back, a keyframe every GOP, SPS first), ends cleanly when stdin ends, and keeps stdout to itself
python3 - "$b/lumatv-venc" "$tmp/feed.err" <<'EOF' || fail "feed round trip"
import struct, subprocess, sys
exe, errpath = sys.argv[1], sys.argv[2]
W, H, N, GOP = 640, 360, 12, 5
err = open(errpath, "wb")
p = subprocess.Popen([exe, "feed", "-w", str(W), "-h", str(H), "-r", "30", "-b", "800", "-g", str(GOP)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=err)
def take(n):
    b = b""
    while len(b) < n:
        c = p.stdout.read(n - len(b))
        assert c, "the program closed its output early"
        b += c
    return b
ready = take(24)
assert ready[:4] == b"LRDY" and struct.unpack("<IIIII", ready[4:]) == (1, W, H, 0, 0), ready
frame = bytes((i * 7) % 256 for i in range(W * H * 3 // 2))
keys = []
for i in range(N):
    pts = 1000000 + i * 33333
    p.stdin.write(b"LFRM" + struct.pack("<IQII", 1 if i == 0 else 0, pts, len(frame), 0) + frame)
    p.stdin.flush()
    head = take(24)
    magic, flags, got_pts, size, hw_ms = head[:4], *struct.unpack("<IQII", head[4:])
    assert magic == b"LAUN" and got_pts == pts and size > 0, (magic, flags, got_pts, size)
    data = take(size)
    assert data[:3] == b"\x00\x00\x00" or data[:3] == b"\x00\x00\x01", data[:8]
    keys.append(flags & 1)
    if flags & 1:
        assert data[:5] == b"\x00\x00\x00\x01\x67", "a keyframe starts with its SPS"
assert keys == [1 if i % GOP == 0 else 0 for i in range(N)], keys
p.stdin.close()
assert p.wait(timeout=10) == 0, "feed ends cleanly when its input does"
assert p.stdout.read() == b"", "nothing but the protocol on stdout"
err.close()
log = open(errpath).read()
assert '"route":"feed:pipe->/dev/venc->pipe"' in log and '"frames":12,"keyframes":3' in log, log
EOF

# 5. feed refuses a frame that is not the size it was started for, and ends with the encoder given back
python3 - "$b/lumatv-venc" <<'EOF' || fail "feed refusal"
import struct, subprocess, sys
p = subprocess.Popen([sys.argv[1], "feed", "-w", "640", "-h", "360"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
assert p.stdout.read(24)[:4] == b"LRDY"
p.stdin.write(b"LFRM" + struct.pack("<IQII", 0, 0, 1234, 0) + b"x" * 1234)
p.stdin.flush()
assert p.wait(timeout=10) == 6, "a wrong-sized frame ends the session with the protocol error"
EOF
echo "venc tests: ok"
