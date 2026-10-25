#!/bin/sh
# Host-side tests: the allocation policy against mock devices, the interposer's fd bookkeeping under LD_PRELOAD,
# and the launcher's fallback logic with a fake loader. Native zig build; no TV, no GPU.
set -eu
HERE=$(cd "$(dirname "$0")" && pwd)
H=$HERE/build/host
mkdir -p "$H"
zig cc -O1 -pthread -Wall -Wextra -o "$H/test_heapcore" "$HERE/test_heapcore.c"
"$H/test_heapcore"
zig cc -O2 -Wall -Wextra -shared -fPIC -o "$H/heapshim.so" "$HERE/heapshim.c" -ldl
zig cc -O1 -Wall -o "$H/test_shim" "$HERE/test_shim.c"
LD_PRELOAD=$H/heapshim.so "$H/test_shim" 2>/dev/null
"$HERE/test_launch.sh"
