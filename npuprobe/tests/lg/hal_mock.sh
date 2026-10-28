#!/bin/sh
# LG's NN HAL tools as black boxes, on the build host: what libwebos_nnhal.so (the HAL "lgnpu", package nnhal 2.0.1-1072) does when
# its own test, lgnpu_online_nnhal_test, runs under qemu-arm-static, and what its symbols and strings say. Result: it is a stand-in.
# Its execute call sleeps one millisecond and reports that as the time on the device; nothing compiles or runs a model, and no device
# is opened. Read from a copy of the TV's files (never committed): $NPU_TV_ROOT/usr/{bin,lib}.
#   tests/lg/hal_mock.sh
set -u
root=${NPU_TV_ROOT:-/tmp/lumatv-npu-tv/root}
hal=$root/usr/lib/libwebos_nnhal.so
test_bin=$root/usr/bin/lgnpu_online_nnhal_test
[ -f "$hal" ] && [ -f "$test_bin" ] || { echo "skip: no copy of LG's NN HAL files under $root (set NPU_TV_ROOT)"; exit 0; }
tools=${NPU_TOOLS:-/tmp/lumatv-npu-tools}
[ -f "$tools/env.sh" ] && . "$tools/env.sh"
[ -x "${QEMU_ARM:-}" ] || { echo "qemu-arm-static not found (toolchain.py unpacks it)" >&2; exit 2; }

pass=0; fail=0
want() { # NAME TEXT PATTERN
  if printf '%s\n' "$2" | grep -Eq -- "$3"; then pass=$((pass + 1)); echo "ok   $1"; else fail=$((fail + 1)); echo "FAIL $1 (missing: $3)"; printf '%s\n' "$2" | head -12 | sed 's/^/       /'; fi
}
unwant() { # NAME TEXT PATTERN
  if printf '%s\n' "$2" | grep -Eq -- "$3"; then fail=$((fail + 1)); echo "FAIL $1 (found: $3)"; printf '%s\n' "$2" | grep -E -- "$3" | head -5 | sed 's/^/       /'; else pass=$((pass + 1)); echo "ok   $1"; fi
}

# The build the findings are about. A different file means the analysis has to be redone, not that this test is wrong.
sum=$(sha256sum "$hal" | cut -d' ' -f1)
if [ "$sum" = 963204be235cdcdf9361e61a7505280d9b000ee27d59e8627bc88de46c03d230 ]; then pass=$((pass + 1)); echo "ok   libwebos_nnhal.so is the analysed build"; else fail=$((fail + 1)); echo "FAIL libwebos_nnhal.so is another build ($sum): reread it"; fi

# --- symbols: what the library can reach
syms=$(arm-linux-gnueabi-readelf --dyn-syms -W "$hal" 2>&1)
needed=$(arm-linux-gnueabi-readelf -d "$hal" 2>&1 | grep NEEDED)
want "exports the HAL's device list" "$syms" 'android2nn3hal10getDevicesEv'
want "exports the HAL 1.2 device calls" "$syms" 'NpuNnHal16prepareModel_1_2'
want "exports execute" "$syms" 'NpuPreparedModel20executeSynchronously'
want "links only the C and C++ runtimes" "$needed" 'libstdc\+\+\.so\.6'
unwant "no NPU or runtime library is linked" "$needed" 'libnpu|libnnrt|libwebos_offline|libneuralnetworks'
undef=$(printf '%s\n' "$syms" | awk '$7 == "UND" {print $8}')
unwant "imports nothing that opens a device, maps memory, loads a library or talks to a socket" "$undef" '^(open|openat|open64|ioctl|mmap|mmap2|dlopen|dlsym|socket|connect)(@|$)'
want "imports sleep and a clock: the time it reports is made" "$undef" '^usleep(@|$)'
want "imports a monotonic clock" "$undef" 'steady_clock3now'

# --- the strings of the other tools
ro() { strings -a "$1"; }
want "the offline HAL's test reads a package file" "$(ro "$root/usr/bin/lgnpu_nn_hal_test")" '\./dne_custom_op\.lgnpu'
want "run_lgnpu takes a package file, its tensors and a count" "$(ro "$root/usr/bin/run_lgnpu")" 'Usage: %s \[-i input tensor file\] \[-p \.lgnpu file\] \[-o output file prefix\] \[-a affinity\] \[-c run count\] -j \[tensor meta json\]'
want "nn-proxy is the AI picture service" "$(ro "$root/usr/bin/nn-proxy")" 'AI 8K Upscaling is Updated'

# --- the test itself, run with a system-call trace
w=$(mktemp -d)
trap 'rm -rf "$w"' EXIT
sysroot=$CROSS_ROOT/usr/arm-linux-gnueabi
"$QEMU_ARM" -strace -L "$sysroot" -E LD_LIBRARY_PATH="$root/usr/lib:$sysroot/lib" "$test_bin" > "$w/out.txt" 2> "$w/strace.txt"
code=$?
out=$(cat "$w/out.txt")
want "LG's own online NN HAL test passes (exit $code)" "$out" '\[  PASSED  \] 1 test'
want "one device, one operation reported supported" "$out" 'count: 1supported size: 1'
tr=$(cat "$w/strace.txt")
sleeps=$(printf '%s\n' "$tr" | grep -c 'clock_nanosleep(')
if [ "$sleeps" = 1 ]; then pass=$((pass + 1)); echo "ok   the execute call is one sleep"; else fail=$((fail + 1)); echo "FAIL expected one sleep, saw $sleeps"; fi
want "the sleep is 1 ms (the time the HAL reports on the device)" "$tr" 'clock_nanosleep\(CLOCK_REALTIME,0,\{tv_sec = 0,tv_nsec = 1000000\}'
unwant "the HAL opens nothing under /dev" "$tr" 'open(at)?\(.*"/dev/'
unwant "the HAL makes no ioctl but the terminal query on stdout" "$tr" 'ioctl\((3|4|5|6|7|8|9|[1-9][0-9]),'
unwant "no socket, no connect" "$tr" ' (socket|connect)\('

echo "$pass passed, $fail failed (LG NN HAL as a black box)"
[ "$fail" = 0 ]
