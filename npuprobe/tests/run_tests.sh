#!/bin/sh
# Tests of lumatv-npu-probe against the stub delegate (stub/stub_delegate.cc) and generated models.
#   tests/run_tests.sh [host|arm]     default host; arm runs the TV build under qemu-arm-static
# Optional, skipped when the file is absent: the real detector (fetch-model.sh) and LG's sample pictures from
# a copy of the TV's /usr/share/aif/images in $NPU_TV_ROOT/usr/share/aif/images.
set -u
here=$(cd "$(dirname "$0")" && pwd)
target=${1:-host}
b=$here/../build/$target
tools=${NPU_TOOLS:-/tmp/lumatv-npu-tools}
[ -f "$tools/env.sh" ] && . "$tools/env.sh"

if [ "$target" = arm ]; then
  [ -x "${QEMU_ARM:-}" ] || { echo "qemu-arm-static not found (toolchain.py unpacks it)" >&2; exit 2; }
  run() { "$QEMU_ARM" -L "$CROSS_ROOT/usr/arm-linux-gnueabi" "$@"; }
else
  run() { "$@"; }
fi
probe=$b/lumatv-npu-probe
stub=$b/libstub-npu-delegate.so
[ -x "$probe" ] && [ -f "$stub" ] && [ -f "$b/libnot-a-delegate.so" ] || { echo "build first: build.sh $target" >&2; exit 2; }

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
run "$b/make-test-models" "$tmp" || { echo "make-test-models failed" >&2; exit 2; }
m=$tmp

pass=0
fail=0
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -n "${2:-}" ] && printf '%s\n' "$2" | sed 's/^/       /' | head -${3:-12}; }
# t NAME EXPECTED_EXIT PATTERN... -- args : runs the probe once, checks the exit code and that every PATTERN (grep -E) is in the output
t() {
  name=$1; want=$2; shift 2
  pats=""
  while [ "$1" != "--" ]; do pats="$pats
$1"; shift; done
  shift
  out=$(run "$probe" "$@" 2>&1); code=$?
  res=ok
  [ "$code" = "$want" ] || res="exit $code, wanted $want"
  oldifs=$IFS; IFS='
'
  for p in $pats; do
    [ -z "$p" ] && continue
    printf '%s\n' "$out" | grep -Eq -- "$p" || res="$res; missing: $p"
  done
  IFS=$oldifs
  if [ "$res" = ok ]; then ok "$name"; else bad "$name ($res)" "$out" 25; fi
  last=$out
}
# n NAME PATTERN : the last output must not contain PATTERN
n() { if printf '%s\n' "$last" | grep -Eq -- "$2"; then bad "$1 (found: $2)"; else ok "$1"; fi; }

T=2
J=1
common="--runs 8 --warmup 1 --threads $T"

t "info: builtin model has no NPU node" 0 'no lgnpu_custom_op node' 'ops: ADD x1, CONV_2D x1, DEPTHWISE_CONV_2D x1' -- --model "$m/float_net.tflite" --info
t "info: custom-op model reports its package" 0 '1 lgnpu_custom_op node\(s\), 8 bytes' 'custom ops: lgnpu_custom_op' -- --model "$m/npu_custom.tflite" --info
t "info: int8 net has exactly LG's op set" 0 'ops: ADD x1, CONCATENATION x1, CONV_2D x1, DEPTHWISE_CONV_2D x1, PAD x1, QUANTIZE x1, RESIZE_NEAREST_NEIGHBOR x1' 'input .*UINT8 +\[1,16,16,3\] scale=0.00392157 zp=0' 'output .*INT8 +\[1,16,16,8\]' -- --model "$m/int8_net.tflite" --info

t "cpu builtin kernels" 0 'n=8 min=' 'process CPU:' 'cpu_backend=builtin' -- --model "$m/float_net.tflite" $common --cpu-backend builtin
t "cpu xnnpack, int8 per-channel model" 0 'XNNPACK: [0-9]+ delegate kernel' 'n=8 min=' -- --model "$m/int8_net.tflite" $common
t "cpu backend none" 0 -- --model "$m/float_net.tflite" $common --cpu-backend none
n "cpu backend none runs nothing" '== CPU'

stubopts="--delegate $stub"
t "delegate takes the whole float graph" 0 'ModifyGraphWithDelegate: ok' 'took 3 node\(s\), 0 left on the CPU' 'delegated ops: ADD x1, CONV_2D x1, DEPTHWISE_CONV_2D x1' 'outputs vs CPU: identical' 'delegate_applied=1' -- \
  --model "$m/float_net.tflite" $common --cpu-backend builtin $stubopts --delegate-opt mode=claim_all --delegate-opt model="$m/float_net.tflite" --require-delegate
t "delegate takes the int8 graph, outputs identical" 0 'took 7 node\(s\)' 'delegated ops: ADD x1, CONCATENATION x1, CONV_2D x1, DEPTHWISE_CONV_2D x1, PAD x1, QUANTIZE x1, RESIZE_NEAREST_NEIGHBOR x1' 'outputs vs CPU: identical' -- \
  --model "$m/int8_net.tflite" $common --cpu-backend builtin $stubopts --delegate-opt model="$m/int8_net.tflite" --require-delegate
t "delegate with LG's options (min_freq, use_custom_alloc) and a 3 ms device" 0 'delegate_applied=1' 'outputs vs CPU: identical' 'use_custom_alloc=true' -- \
  --model "$m/float_net.tflite" $common --cpu-backend builtin $stubopts --delegate-opt model="$m/float_net.tflite" --delegate-opt min_freq=30 --delegate-opt use_custom_alloc=true --delegate-opt delay_us=3000
ms=$(printf '%s\n' "$last" | sed -n 's/.*delegate_mean_ms=\([0-9.]*\).*/\1/p')
if [ -n "$ms" ] && [ "$(echo "$ms" | awk '{print ($1 >= 3.0 && $1 < 20.0)}')" = 1 ]; then ok "delegate phase timing includes the device's 3 ms ($ms)"; else bad "delegate phase timing ($ms)"; fi

t "custom-op model: CPU skipped, delegate takes the custom node, result computed by it" 0 'CPU, no delegate: skipped' 'delegated ops: lgnpu_custom_op x1' '1 delegate kernel\(s\) took 1 node\(s\), 1 left on the CPU' 'output: 2 2 2 2 2 2 2 2' -- \
  --model "$m/npu_custom.tflite" --runs 4 --warmup 1 --fill const:0.25 --print-output $stubopts --delegate-opt mode=custom_op --require-delegate
t "custom-op model without a delegate: nothing runs, exit 4" 4 'CPU, no delegate: skipped' -- --model "$m/npu_custom.tflite" --runs 2 --fill const:0.25

t "LG-style refusal: no custom op in the graph" 3 'delegate refused the graph' 'delegate NOT APPLIED' 'delegate_applied=0' 'Nodes doesn.t have customOp node for NPU' -- \
  --model "$m/float_net.tflite" $common --cpu-backend builtin $stubopts --delegate-opt mode=custom_op --require-delegate
t "refusal without --require-delegate still exits 0 and reports the CPU path" 0 'delegate NOT APPLIED' 'outputs vs CPU: identical' -- \
  --model "$m/float_net.tflite" $common --cpu-backend builtin $stubopts --delegate-opt mode=refuse
t "unknown option key is reported" 3 'plugin said: Wrong key entered in StubDelegateOptions bogus' 'delegate not created' -- \
  --model "$m/float_net.tflite" $common --cpu-backend none $stubopts --delegate-opt bogus=1 --require-delegate
t "min_freq must be an integer" 3 'min_freq is not an integer' -- --model "$m/float_net.tflite" $common --cpu-backend none $stubopts --delegate-opt min_freq=fast --require-delegate
t "missing delegate library" 3 'dlopen failed' -- --model "$m/float_net.tflite" $common --cpu-backend none --delegate /nonexistent/libx.so --require-delegate
t "a library that is not a plugin" 3 'not an external delegate' -- --model "$m/float_net.tflite" $common --cpu-backend none --delegate "$b/libnot-a-delegate.so" --require-delegate

t "detections are decoded, sorted, thresholded and named" 0 'person +0.910 +box y 0.100..0.600 x 0.200..0.500' 'dog +0.620' -- --model "$m/ssd_const.tflite" --runs 2 --warmup 0 --cpu-backend builtin
n "detection below the threshold is hidden" 'cat +0.120'
t "threshold and top-k" 0 'person +0.910' 'cat +0.120' -- --model "$m/ssd_const.tflite" --runs 1 --warmup 0 --cpu-backend builtin --min-score 0.1 --top 3
printf 'thing-zero\n' > "$tmp/labels.txt"
t "labels file" 0 'thing-zero +0.910' 'class 17 +0.620' -- --model "$m/ssd_const.tflite" --runs 1 --warmup 0 --cpu-backend builtin --labels "$tmp/labels.txt"

t "bad usage" 2 'usage:' -- --runs 3
t "delegate option without a value" 2 'wants KEY=VALUE' -- --model "$m/float_net.tflite" --delegate-opt novalue
t "system info without a model" 0 'probe runtime: TFLite [0-9]+\.[0-9]+\.[0-9]+' 'delegate: ' -- --system-info --delegate "$stub"
t "unreadable model" 1 'cannot load model' -- --model "$tmp/nothing.tflite"

# --- optional: the real detector on LG's own sample pictures
models=${NPU_MODELS:-/tmp/lumatv-npu-models}
pics=${NPU_TV_ROOT:-/tmp/lumatv-npu-tv/root}/usr/share/aif/images
det=$models/efficientdet_lite0_320_ptq.tflite
if [ -f "$det" ] && [ -f "$pics/5people.jpg" ]; then
  runs="--runs 2 --warmup 0 --threads 4"
  [ "$target" = arm ] && runs="--runs 1 --warmup 0 --threads 2"
  t "EfficientDet-Lite0 finds the people in LG's 5people.jpg" 0 'XNNPACK: [0-9]+ delegate kernel' -- --model "$det" --image "$pics/5people.jpg" $runs --top 10 --min-score 0.4 --labels "$models/coco_labels.txt"
  people=$(printf '%s\n' "$last" | grep -c '^  person ')
  if [ "$people" -ge 4 ]; then ok "5people.jpg: $people person detections"; else bad "5people.jpg: only $people person detections" "$last"; fi
else
  echo "skip real-model test (run fetch-model.sh, and point NPU_TV_ROOT at a copy of the TV's files)"
fi

echo "$pass passed, $fail failed ($target)"
[ "$fail" = 0 ]
