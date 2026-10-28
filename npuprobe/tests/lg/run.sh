#!/bin/sh
# LG's libtflite-npu-delegate.so.1 as a black box, on the build host: loaded into the ARM probe (under qemu-arm-static)
# next to fakes for libnnrt and PmLogLib, and driven by lg_contract.cc against a fake TFLite context. The delegate is
# read from a copy of the TV's files (never committed): $NPU_TV_ROOT/usr/lib/libtflite-npu-delegate.so.1.0.0.
#   tests/lg/run.sh        (needs build.sh arm)
set -u
here=$(cd "$(dirname "$0")" && pwd)
root=${NPU_TV_ROOT:-/tmp/lumatv-npu-tv/root}
lib=$root/usr/lib/libtflite-npu-delegate.so.1.0.0
[ -f "$lib" ] || { echo "skip: no copy of LG's delegate at $lib (set NPU_TV_ROOT)"; exit 0; }
tools=${NPU_TOOLS:-/tmp/lumatv-npu-tools}
[ -f "$tools/env.sh" ] && . "$tools/env.sh"
b=$here/../../build/arm
[ -x "$b/lumatv-npu-probe" ] || { echo "build first: build.sh arm" >&2; exit 2; }
tf=$(ls -d "$here"/../../build/src/tensorflow-* 2>/dev/null | head -n 1)
[ -n "$tf" ] || tf=$(sed -n 's/^TFLITE_SRC:PATH=//p' "$b/CMakeCache.txt")

w=$(mktemp -d)
trap 'rm -rf "$w"' EXIT
cc="arm-linux-gnueabi-g++-${CROSS_CC_VER:-11} --sysroot=$CROSS_ROOT -O1 -march=armv7-a -mfpu=neon-vfpv4 -mfloat-abi=softfp"
$cc -shared -fPIC -Wl,-soname,libnnrt.so.1 -o "$w/libnnrt.so.1" "$here/fake_nnrt.cc" || exit 2
arm-linux-gnueabi-gcc-${CROSS_CC_VER:-11} --sysroot="$CROSS_ROOT" -O1 -march=armv7-a -mfpu=neon-vfpv4 -mfloat-abi=softfp \
  -shared -fPIC -Wl,-soname,libPmLogLib.so.3 -o "$w/libPmLogLib.so.3" "$here/fake_pmlog.c" || exit 2
$cc -std=c++17 -isystem "$tf" -o "$w/lg_contract" "$here/lg_contract.cc" -ldl -rdynamic || exit 2
cp "$lib" "$w/libtflite-npu-delegate.so.1"
"$b/make-test-models" "$w" 2>/dev/null || "$QEMU_ARM" -L "$CROSS_ROOT/usr/arm-linux-gnueabi" "$b/make-test-models" "$w" || exit 2

q() { "$QEMU_ARM" -L "$CROSS_ROOT/usr/arm-linux-gnueabi" -E LD_LIBRARY_PATH="$w" "$@" 2>&1; }
pass=0; fail=0
want() { # NAME TEXT PATTERN
  if printf '%s\n' "$2" | grep -Eq -- "$3"; then pass=$((pass + 1)); echo "ok   $1"; else fail=$((fail + 1)); echo "FAIL $1 (missing: $3)"; printf '%s\n' "$2" | head -20 | sed 's/^/       /'; fi
}

h=$(q "$w/lg_contract" "$w/libtflite-npu-delegate.so.1")
want "loads, both plugin entry points present" "$h" 'tflite_plugin_create_delegate=yes tflite_plugin_destroy_delegate=yes'
want "sets kTfLiteDelegateFlagsAllowDynamicTensors" "$h" 'flags=0x1 \(1 = AllowDynamicTensors\)'
want "unknown option key: reported, delegate still created" "$h" 'Wrong key entered in NpuDelegateOptions bogus'
want "min_freq that is not a number throws out of the create call" "$h" 'min_freq=fast +-> CRASH'
want "TfLiteRegistration is 10 words (TFLite 2.13): size 1, delegate echoed" "$h" 'graph=custom +registration=10 words: Prepare -> 0; Replace called=1 nodes_to_replace.size=1 first=0 delegate_ok=1'
want "8, 9 and 11 word registrations are read wrongly" "$h" 'graph=custom +registration= 9 words: Prepare -> 0; Replace called=1 nodes_to_replace.size=-1'
want "builtin-only graph refused with LG's message" "$h" "graph=builtin +registration=10 words: PMLOG level=3 \| NPUDELEGATE \| %s failed : Nodes doesn't have customOp node for NPU"
want "the custom node is found among builtin nodes, either order" "$h" 'graph=add\+custom +registration=10 words: Prepare -> 0; Replace called=1 nodes_to_replace.size=1 first=1'
want "init hands libnnrt the node's custom options as the package" "$h" 'LoadOpGraph\(package=0x[0-9a-f]+ size=8 first bytes=000040400000803f'
want "tensors are registered at the first invoke, then ExecuteOpGraph, UnLoadOpGraph on free" "$h" 'AddInputTensor\(graph=1 type=6 bytes=64 .*dims=\[1,4,4,1\] custom_alloc=0\)'
want "use_custom_alloc=true: tensors registered in prepare with custom_alloc=1" "$h" 'AddInputTensor\(graph=1 type=6 bytes=64 data=0x[0-9a-f]+ flags=4 dims=\[1,4,4,1\] custom_alloc=1\)'
want "use_custom_alloc=true: NNRT's buffers handed to TFLite, alignment check skipped" "$h" 'host: SetCustomAllocationForTensor\(tensor=0 .*flags=1\)'

o=$(q "$b/lumatv-npu-probe" --model "$w/float_net.tflite" --runs 1 --warmup 0 --cpu-backend none --delegate "$w/libtflite-npu-delegate.so.1" --require-delegate)
want "probe: builtin model refused, reported NOT APPLIED" "$o" "Nodes doesn't have customOp node for NPU"
want "probe: refusal is not a crash" "$o" 'delegate NOT APPLIED'
o=$(q "$b/lumatv-npu-probe" --model "$w/npu_only.tflite" --runs 2 --warmup 1 --fill const:0.25 --delegate "$w/libtflite-npu-delegate.so.1" --delegate-opt min_freq=30 --require-delegate)
want "probe: custom-op model taken by LG's delegate" "$o" 'delegated ops: lgnpu_custom_op x1'
want "probe: min_freq is the priority handed to PrepareOpGraph" "$o" 'PrepareOpGraph\(graph=1 priority=30\)'
o=$(q "$b/lumatv-npu-probe" --model "$w/npu_only.tflite" --runs 1 --warmup 0 --delegate "$w/libtflite-npu-delegate.so.1" --delegate-opt min_freq=fast)
want "probe: an exception from the plugin is caught" "$o" 'tflite_plugin_create_delegate threw: stoi'
o=$(q "$b/lumatv-npu-probe" --system-info --delegate "$w/libtflite-npu-delegate.so.1")
want "probe --system-info finds the entry points" "$o" 'tflite_plugin_create_delegate present, tflite_plugin_destroy_delegate present'

echo "$pass passed, $fail failed (LG delegate contract)"
[ "$fail" = 0 ]
