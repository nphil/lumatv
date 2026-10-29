#!/bin/sh
# The off-screen proof of direct scan-out's route A (docs/research/scanout-research.md, "TV check list" a): which layout a driver-served client's
# buffers have, and whether the display driver (lg1k) would import them. Executed ON THE SET as /tmp/lumatv/scanout-proof.sh (busybox sh).
#
# Nothing of the live display is touched: a private runtime directory and socket (/tmp/scanout-proof), the GL renderer with no DRM master, no shell,
# no input, no control of LG's compositor. The server's probe (--scanout-probe) opens /dev/dri/card0 without becoming master and only PRIME-imports
# each buffer's dma-buf and closes the handle: no framebuffer, no mode set, no atomic commit, no read of anyone's pixels. The client is the 256x256
# LG-driver one (x2cli, LG's own 32-bit libmali, six frames), so the set's own allocation and layout are what is recorded.
#
#   scanout-proof.sh run [VARIANT...]   variants (default: stock afbc0):
#                                         stock   the client as LG's apps run it
#                                         afbc0   the same with WAYLAND_AFBC=0 (the client library's string; asks for a linear layout)
#                                         big     stock at 1920x1080: 3 buffers of 8-9 MB, a CMA cost the 256x256 ones do not have
#   scanout-proof.sh clean              kill anything of ours and remove /tmp/scanout-proof
# Per variant, under /tmp/scanout-proof/out/<variant>/: server.log, client.log, scanout.json (`ctl scanout`), verdict.txt; the verdict is printed.
# Read the verdict: "layouts" is what the client really allocated (not what was asked for), "imported" is lg1k's answer for each buffer
# (a refusal with EINVAL is a buffer of scattered pages), "alloc_device" is the path the driver told the client to allocate from, and
# "x2 check" is the six-frame GPU-import pixel check (every drawn frame's centre pixel against the pattern) that must stay "bad 0".
D=/tmp/lumatv
LIB=$D/lib
LD=$LIB/ld-linux-aarch64.so.1
BLOB=$LIB/libmali-bifrost-g52-g24p0-wayland-gbm.so
MALI32=/usr/lib/libmali.so.0
P=/tmp/scanout-proof
SOCK=wayland-scanout-proof
BIN="$LD --library-path $LIB $D/bin/lumatv-compositor"

# Only what runs from the private directory: LG's compositor and the live one are not matched.
kill_ours() {
    pkill -f "lumatv-compositor --runtime-dir $P" 2>/dev/null
    pkill -f "x2cli $MALI32 --frames 6" 2>/dev/null
    sleep 0.3
}

cma() { grep -E 'CmaFree|MemAvailable' /proc/meminfo | tr -s ' ' | tr '\n' ' '; echo; }

# variant NAME SIZE [ENV=VALUE...]
variant() {
    name=$1; size=$2; shift 2
    out=$P/out/$name
    rm -rf "$out"; mkdir -p "$out"
    rm -f $P/$SOCK $P/$SOCK.lock $P/lumatv-control
    echo "== $name: client $size $*"
    echo "   before: $(cma)"
    ( LUMATV_LOG=debug $BIN --runtime-dir $P --socket $SOCK --renderer gl --driver $BLOB --buffers driver --shell off \
        --scanout-probe --check x2 --seconds 40 --log-protocol > "$out/server.log" 2>&1 & )
    n=0
    while [ ! -S $P/$SOCK ] && [ $n -lt 50 ]; do sleep 0.1; n=$((n+1)); done
    if [ ! -S $P/$SOCK ]; then echo "$name: the server did not open its socket"; sed -n 1,20p "$out/server.log"; kill_ours; return 1; fi
    sleep 0.5
    env XDG_RUNTIME_DIR=$P WAYLAND_DISPLAY=$SOCK "$@" timeout 20 $D/bin/x2cli $MALI32 --frames 6 --size "$size" > "$out/client.log" 2>&1
    echo "   client exit $?"
    sleep 1
    echo "   during: $(cma)"
    $BIN ctl --runtime-dir $P scanout > "$out/scanout.json" 2>&1
    $BIN ctl --runtime-dir $P quit > /dev/null 2>&1
    n=0
    while pgrep -f "lumatv-compositor --runtime-dir $P" > /dev/null && [ $n -lt 20 ]; do sleep 0.5; n=$((n+1)); done
    kill_ours
    verdict "$name" "$out"
}

verdict() {
    name=$1; out=$2
    {
        echo "variant $name"
        grep -E 'scanout-probe: buffer|scanout-probe summary' "$out/server.log" | sed 's/^\[srv *[0-9.]*\] *info *//' | uniq
        echo "alloc_device: $(sed -n 's/.*"allocDevices":\[\([^]]*\)\].*/\1/p' "$out/scanout.json" | head -1)"
        echo "tap: $(sed -n 's/.*"tap":{\([^}]*\)}.*/\1/p' "$out/scanout.json" | head -1)"
        echo "x2 check: $(sed -n 's/.*\(check ok [0-9]* bad [0-9]*\).*/\1/p' "$out/server.log" | tail -1)"
        grep -E ' (warn|error) ' "$out/server.log" | grep -v 'ld.so.preload' | head -5
    } > "$out/verdict.txt"
    cat "$out/verdict.txt"
    echo "   after: $(cma)"
}

case "${1:-help}" in
    run)
        shift
        [ $# -gt 0 ] || set -- stock afbc0
        mkdir -p $P/out
        kill_ours
        echo "scanout proof $(date -u +%Y-%m-%dT%H:%M:%SZ); LG's compositor and the screen are not touched"
        echo "   $(cma)"
        for v in "$@"; do
            case "$v" in
                stock) variant stock 256x256 ;;
                afbc0) variant afbc0 256x256 WAYLAND_AFBC=0 ;;
                big) variant big 1920x1080 ;;
                *) echo "unknown variant $v (stock afbc0 big)" ;;
            esac
        done
        ;;
    clean)
        kill_ours
        rm -rf $P
        echo "left running: $(pgrep -fl 'scanout-proof|x2cli' || echo none)"
        ;;
    *) sed -n 2,21p "$0" ;;
esac
