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
# The NNAPI route loads libneuralnetworks.so by name: NN_LIB is the directory the stub runtime is picked from, NN_ENV its switches
# (stub/stub_nnapi.cc), both applied inside the subshell that runs the probe. GPU_ENV is the same for the switches of the GPU stubs
# (stub/stub_gpu_delegate.cc, stub/stub_opencl.cc) and any other variable a test needs; values hold no spaces.
NN_LIB=
NN_ENV=
GPU_ENV=
nnenv() {
  [ -n "$NN_LIB" ] && export LD_LIBRARY_PATH="$NN_LIB${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
  for kv in $NN_ENV $GPU_ENV; do export "$kv"; done
}
gpu_stub=$b/libstub-gpu-delegate.so
cl_stub=$b/opencl-stub/libOpenCL.so.1
[ -x "$probe" ] && [ -f "$stub" ] && [ -f "$b/libnot-a-delegate.so" ] && [ -f "$gpu_stub" ] && [ -f "$cl_stub" ] || { echo "build first: build.sh $target" >&2; exit 2; }

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
  out=$(nnenv; run "$probe" "$@" 2>&1); code=$?
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

# --- the NNAPI route: TFLite's NNAPI delegate over a runtime that is stub/stub_nnapi.cc, with devices lgnpu-stub (accelerator) and
# nnapi-reference (cpu). It computes ADD and accepts every other operation it claims without computing it, as LG's libwebos_nnhal does.
nnc="--runs 8 --warmup 1 --threads $T --cpu-backend builtin --nnapi"
NN_LIB=$b/nnapi-stub
NN_ENV="STUB_NNAPI_OPS=ADD"
t "nnapi: the device takes the ADD and computes it, outputs identical" 0 'NNAPI devices \(2\):' '\[0\] lgnpu-stub +type=accelerator' '\[1\] nnapi-reference +type=cpu' \
  '1 delegate kernel\(s\) took 1 node\(s\), 2 left on the CPU' 'delegated ops: ADD x1$' 'outputs vs CPU: identical' 'nnapi_applied=1' 'nnapi_outputs=identical' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=lgnpu-stub --require-delegate
NN_ENV="STUB_NNAPI_OPS=ADD STUB_NNAPI_DELAY_US=3000"
t "nnapi: a 3 ms device" 0 'nnapi_applied=1' 'outputs vs CPU: identical' -- --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=lgnpu-stub
ms=$(printf '%s\n' "$last" | sed -n 's/.*nnapi_mean_ms=\([0-9.]*\).*/\1/p')
if [ -n "$ms" ] && [ "$(echo "$ms" | awk '{print ($1 >= 3.0 && $1 < 20.0)}')" = 1 ]; then ok "nnapi: the timed phase includes the device's 3 ms ($ms)"; else bad "nnapi: timing ($ms)"; fi
NN_ENV=
t "nnapi: a device that claims every op and computes nothing is caught, exit 5" 5 'delegated ops: ADD x1, CONV_2D x1, DEPTHWISE_CONV_2D x1' '1 delegate kernel\(s\) took 3 node\(s\), 0 left on the CPU' \
  'WARNING: every output value is zero' 'nnapi_applied=1' 'nnapi_outputs=zero' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=lgnpu-stub --require-delegate
t "nnapi: the same run without --require-delegate reports it and exits 0" 0 'WARNING: every output value is zero' 'nnapi_outputs=zero' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=lgnpu-stub
t "nnapi: LG's op set, which ops TFLite hands an NNAPI 1.2 device" 5 'took 5 node\(s\), 2 left on the CPU' 'delegated ops: ADD x1, CONCATENATION x1, CONV_2D x1, DEPTHWISE_CONV_2D x1, PAD x1$' -- \
  --model "$m/int8_net.tflite" $nnc --nnapi-opt accelerator=lgnpu-stub --require-delegate
NN_ENV="STUB_NNAPI_OPS=none"
t "nnapi: a device that supports nothing takes no node, exit 3" 3 'claimed no node' 'NNAPI delegate NOT APPLIED' 'nnapi_applied=0' -- \
  --model "$m/float_net.tflite" $nnc --require-delegate
t "nnapi: allow_cpu=true adds NNAPI's CPU reference, which claims everything" 5 'delegated ops: ADD x1, CONV_2D x1, DEPTHWISE_CONV_2D x1' 'allow_cpu' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt allow_cpu=true --require-delegate
NN_ENV="STUB_NNAPI_DEVICES=lgnpu:accelerator STUB_NNAPI_OPS=ADD"
t "nnapi: LG's device name, a single device" 0 'NNAPI devices \(1\):' '\[0\] lgnpu +type=accelerator' 'delegated ops: ADD x1$' 'outputs vs CPU: identical' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=lgnpu --require-delegate
t "nnapi: --nnapi-list lists the devices and exits" 0 'libneuralnetworks.so loaded, feature level 29' 'TFLite derives Android level 29' '\[0\] lgnpu +type=accelerator version=stub-1.0 feature level 29' -- --nnapi-list
NN_ENV="STUB_NNAPI_OPS=ADD"
t "nnapi: an unknown accelerator is refused with TFLite's list" 3 'Could not find the specified NNAPI accelerator: nope. Must be one of: \{lgnpu-stub,nnapi-reference\}' 'NNAPI delegate refused the graph' 'nnapi_applied=0' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=nope --require-delegate
NN_ENV="STUB_NNAPI_OPS=ADD STUB_NNAPI_FAIL=compile"
t "nnapi: a compilation the device fails is refused and the CPU path runs, exit 3" 3 'ANEURALNETWORKS_OP_FAILED' 'NNAPI delegate refused the graph' 'NNAPI errno 5' 'outputs vs CPU: identical' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=lgnpu-stub --require-delegate
NN_ENV="STUB_NNAPI_OPS=ADD STUB_NNAPI_FAIL=compute"
t "nnapi: an execution the device fails ends the phase, exit 3" 3 'ANEURALNETWORKS_OP_FAILED' 'NNAPI delegate phase did not run to the end: Invoke failed during warm-up' 'nnapi_ran=0' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=lgnpu-stub --require-delegate
NN_LIB=$b/nnapi-stub-12
NN_ENV="STUB_NNAPI_OPS=ADD"
t "nnapi: a runtime without the 1.3 feature-level call still selects its device" 0 'delegated ops: ADD x1$' 'outputs vs CPU: identical' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt accelerator=lgnpu-stub --require-delegate
t "nnapi: sdk=28 has no device selection: the accelerator name and the device's op list are not used" 5 'sdk=28' '1 delegate kernel\(s\) took 3 node\(s\)' 'nnapi_outputs=zero' -- \
  --model "$m/float_net.tflite" $nnc --nnapi-opt sdk=28 --nnapi-opt accelerator=lgnpu-stub --require-delegate
NN_LIB=
NN_ENV=
t "nnapi: no libneuralnetworks.so, exit 3" 3 'libneuralnetworks.so is not loadable' 'nnapi_applied=0' -- --model "$m/float_net.tflite" $nnc --require-delegate
t "nnapi: --nnapi-list without a runtime" 0 'libneuralnetworks.so is not loadable' -- --nnapi-list

# --- the GPU route. --gpu-lib takes the stub's three entry points (stub/stub_gpu_delegate.cc: it claims the ops of STUB_GPU_OPS, the
# largest `partitions` pieces of them, and computes by running the whole model in a nested interpreter); the built-in TFLite delegate and
# --gpu-coverage need no GPU and run the real selection code; --opencl-lib takes the stand-in ICD loader (stub/stub_opencl.cc).
gc="--runs 8 --warmup 1 --threads $T --cpu-backend builtin --gpu --gpu-lib $gpu_stub"
gc_builtin="--runs 8 --warmup 1 --threads $T --cpu-backend builtin --gpu"
: > "$tmp/gpu.log"
# gl NAME TEXT : the option line the stub delegate logged last must contain TEXT
gl() { if tail -n 1 "$tmp/gpu.log" | grep -Fq -- "$2"; then ok "$1"; else bad "$1 (logged: $(tail -n 1 "$tmp/gpu.log"))"; fi; }
GPU_ENV="STUB_GPU_MODEL=$m/float_net.tflite STUB_GPU_LOG=$tmp/gpu.log"
t "gpu: the stub takes the whole float graph, outputs identical" 0 'delegate: .*libstub-gpu-delegate.so' '1 delegate kernel\(s\) took 3 node\(s\), 0 left on the CPU' \
  'delegated ops: ADD x1, CONV_2D x1, DEPTHWISE_CONV_2D x1' 'set-up [0-9.]+ ms \(delegate and tensors\), first invoke' 'outputs vs CPU: identical' \
  'gpu_applied=1' 'gpu_init_ms=' 'gpu_first_ms=' 'gpu_outputs=identical' -- --model "$m/float_net.tflite" $gc --require-delegate
gl "gpu: default options reach the delegate (fp32, sustained, one partition, OpenCL only, quantised ops on)" 'precision_loss=0 pref=1 p1=1 p2=0 p3=0 flags=3 partitions=1 dir=(none) token=(none)'
t "gpu: fp16, fast start, three partitions, quant off, any backend" 0 'GPU delegate \(precision=fp16 quant=false pref=fast partitions=3 backend=any\)' 'gpu_applied=1' -- \
  --model "$m/float_net.tflite" $gc --gpu-opt precision=fp16 --gpu-opt pref=fast --gpu-opt partitions=3 --gpu-opt quant=false --gpu-opt backend=any
gl "gpu: those options reach the delegate (precision loss on, MIN_LATENCY, FAST_SINGLE_ANSWER, no flags, 3 partitions)" 'precision_loss=1 pref=0 p1=2 p2=0 p3=0 flags=0 partitions=3'
t "gpu: balanced preference" 0 'gpu_applied=1' -- --model "$m/float_net.tflite" $gc --gpu-opt pref=balanced
gl "gpu: balanced is BALANCED (2)" 'pref=2'
t "gpu: a kernel cache directory is created, fingerprinted per model and reported" 0 'kernel cache .*: 0 file\(s\), 0 KiB' 'gpu_applied=1' -- --model "$m/float_net.tflite" $gc --gpu-opt cache="$tmp/kcache"
gl "gpu: the cache reaches the delegate (serialisation on, directory, 16-digit token)" "flags=11 partitions=1 dir=$tmp/kcache token="
if tail -n 1 "$tmp/gpu.log" | grep -Eq 'token=[0-9a-f]{16}$' && [ -d "$tmp/kcache" ]; then ok "gpu: the token is 16 hex digits and the directory exists"; else bad "gpu: cache token or directory"; fi
GPU_ENV="STUB_GPU_MODEL=$m/float_net.tflite STUB_GPU_LOG=$tmp/gpu.log STUB_GPU_OPS=ADD"
t "gpu: the support list can be narrowed" 0 '1 delegate kernel\(s\) took 1 node\(s\), 2 left on the CPU' 'delegated ops: ADD x1$' 'outputs vs CPU: identical' -- --model "$m/float_net.tflite" $gc --require-delegate
GPU_ENV="STUB_GPU_MODEL=$m/float_net.tflite STUB_GPU_LOG=$tmp/gpu.log STUB_GPU_OPS=CONV_2D,ADD"
t "gpu: two pieces, one partition: the first of the largest takes one node" 0 '1 delegate kernel\(s\) took 1 node\(s\), 2 left on the CPU' 'outputs vs CPU: identical' -- --model "$m/float_net.tflite" $gc
t "gpu: two pieces, partitions=2: two kernels take two nodes" 0 '2 delegate kernel\(s\) took 2 node\(s\), 1 left on the CPU' 'delegated ops: ADD x1, CONV_2D x1' 'outputs vs CPU: identical' -- \
  --model "$m/float_net.tflite" $gc --gpu-opt partitions=2
GPU_ENV="STUB_GPU_MODEL=$m/float_net.tflite STUB_GPU_LOG=$tmp/gpu.log STUB_GPU_COMPUTE=none"
t "gpu: a delegate that takes the nodes and computes nothing is caught, exit 5" 5 'WARNING: every output value is zero' 'gpu_applied=1' 'gpu_outputs=zero' -- --model "$m/float_net.tflite" $gc --require-delegate
# a detector: rows sorted by score come back in another order with slightly other scores and boxes (what a GPU's arithmetic does); the raw tensors differ, the detections are the same
GPU_ENV="STUB_GPU_MODEL=$m/det_b.tflite STUB_GPU_LOG=$tmp/gpu.log"
t "gpu: the same detections in another order and with other scores are close, not different" 0 '1 delegate kernel\(s\) took 4 node\(s\), 0 left on the CPU' 'outputs vs CPU: max \|difference\| .*\(differ\)' \
  'detections vs CPU: the same \(2 against 2 at or above 0.30\)' 'gpu_outputs=close' -- --model "$m/det_a.tflite" $gc --require-delegate
GPU_ENV="STUB_GPU_MODEL=$m/det_c.tflite STUB_GPU_LOG=$tmp/gpu.log"
t "gpu: a detector that loses a detection is caught, exit 5" 5 'detections vs CPU: class 0 score 0.80 .* of the CPU.s list has no partner in the other' 'WARNING: the accelerator.s outputs are not the CPU.s' \
  'gpu_outputs=differ' -- --model "$m/det_a.tflite" $gc --require-delegate
GPU_ENV="STUB_GPU_MODEL=$m/float_net.tflite STUB_GPU_LOG=$tmp/gpu.log STUB_GPU_OPS=none"
t "gpu: a delegate that supports nothing takes no node, exit 3" 3 'claimed no node' 'GPU delegate NOT APPLIED' 'gpu_applied=0' -- --model "$m/float_net.tflite" $gc --require-delegate
GPU_ENV="STUB_GPU_MODEL=$m/float_net.tflite STUB_GPU_LOG=$tmp/gpu.log STUB_GPU_FAIL=prepare"
t "gpu: a delegate that cannot start refuses the graph, the CPU path runs, exit 3" 3 'delegate is not initialized' 'GPU delegate refused the graph' 'outputs vs CPU: identical' 'gpu_applied=0' -- \
  --model "$m/float_net.tflite" $gc --require-delegate
GPU_ENV="STUB_GPU_MODEL=$m/float_net.tflite STUB_GPU_LOG=$tmp/gpu.log STUB_GPU_FAIL=create"
t "gpu: a delegate that cannot be created, exit 3" 3 'GPU delegate not created: TfLiteGpuDelegateV2Create returned null' 'gpu_ran=0' -- --model "$m/float_net.tflite" $gc --require-delegate
GPU_ENV="STUB_GPU_MODEL=$m/float_net.tflite STUB_GPU_LOG=$tmp/gpu.log STUB_GPU_DELAY_US=3000"
t "gpu: a 3 ms device" 0 'gpu_applied=1' 'outputs vs CPU: identical' -- --model "$m/float_net.tflite" $gc
ms=$(printf '%s\n' "$last" | sed -n 's/.*gpu_mean_ms=\([0-9.]*\).*/\1/p')
if [ -n "$ms" ] && [ "$(echo "$ms" | awk '{print ($1 >= 3.0 && $1 < 20.0)}')" = 1 ]; then ok "gpu: the timed phase includes the device's 3 ms ($ms)"; else bad "gpu: timing ($ms)"; fi
# the Mali counters (LG's debugfs file) as a file a background loop keeps moving: 30 busy to 70 idle in every step
mali=$tmp/dvfs_utilization
printf 'busy_time: 0\nidle_time: 0\n' > "$mali"
( i=0; while [ $i -lt 400 ]; do i=$((i + 1)); printf 'busy_time: %d\nidle_time: %d\n' $((i * 30)) $((i * 70)) > "$mali.new"; mv "$mali.new" "$mali"; sleep 0.02; done ) &
writer=$!
t "gpu: the Mali counters give the busy share while the probe waits and while it runs, and the cost per inference" 0 'Mali busy with nothing of the probe.s running: 30.0%' \
  'Mali busy: 30.0% over the timed loop' 'gpu_mali_pct=30.0' 'mali_idle_pct=30.0' 'gpu_ms_per_inference=0.0' -- --model "$m/float_net.tflite" $gc --runs 30 --gpu-proc "$mali"
kill "$writer" 2>/dev/null; wait "$writer" 2>/dev/null
t "gpu: counters that are not there are skipped without noise" 0 'gpu_applied=1' -- --model "$m/float_net.tflite" $gc --gpu-proc "$tmp/no-such-file"
n "gpu: no Mali line without counters" 'Mali busy'
GPU_ENV=
t "gpu-lib: a missing library" 3 'GPU delegate not created: dlopen failed' 'gpu_ran=0' -- --model "$m/float_net.tflite" --runs 2 --cpu-backend none --gpu --gpu-lib /nonexistent/libgpu.so --require-delegate
t "gpu-lib: a library that is not a GPU delegate" 3 'is not a GPU delegate library' -- --model "$m/float_net.tflite" --runs 2 --cpu-backend none --gpu --gpu-lib "$b/libnot-a-delegate.so" --require-delegate
# the delegate built into the probe, on a machine whose ICD loader has no driver (an empty vendor directory): TFLite's own refusal, and the CPU path runs
mkdir "$tmp/noicd"
GPU_ENV="OCL_ICD_VENDORS=$tmp/noicd"
t "gpu: no OpenCL driver: the built-in delegate refuses the graph, exit 3" 3 'no OpenCL device' 'GPU delegate coverage' 'the GPU delegate can take 3 of them' 'GPU delegate refused the graph' \
  'gpu_applied=0' 'outputs vs CPU: identical' -- --model "$m/float_net.tflite" $gc_builtin --require-delegate
GPU_ENV=

# the coverage report: the delegate's own selection code on the generated models, no device
t "gpu-coverage: the whole float graph is taken" 0 'GPU delegate coverage' 'nodes: 3; the GPU delegate can take 3 of them in 1 piece\(s\) of 3' 'on the GPU: ADD x1, CONV_2D x1, DEPTHWISE_CONV_2D x1' 'on the CPU: \(none\)' -- \
  --model "$m/float_net.tflite" --gpu-coverage
n "gpu-coverage runs nothing" '== '
t "gpu-coverage: LG's op set is taken whole with quantised ops on" 0 'nodes: 7; the GPU delegate can take 7 of them in 1 piece\(s\) of 7' 'on the CPU: \(none\)' -- --model "$m/int8_net.tflite" --gpu-coverage
t "gpu-coverage: quant=false leaves the quantised graph to the CPU and says why" 0 'can take 0 of them' 'QUANTIZE: Operation is not supported' 'No operations will run on the GPU, and all 7 operations will run on the CPU' -- \
  --model "$m/int8_net.tflite" --gpu-coverage --gpu-opt quant=false
t "gpu-coverage: a model without nodes" 0 'nodes: 0; the GPU delegate can take 0 of them' -- --model "$m/ssd_const.tflite" --gpu-coverage

# --gpu-info through the stand-in ICD loader
t "gpu-info: the stand-in loader's platform and device" 0 'OpenCL: ICD loader .*libOpenCL.so.1' 'platform \[0\] Stub ARM Platform' 'device \[0.0\] Stub GPU \(Mali-G52 shaped\) \(GPU\)' \
  '3 compute units at up to 800 MHz, global memory 1024 MiB' 'fp16 arithmetic yes, images yes, int8 dot product yes, priority hints yes' 'Mali counters .*: not readable' -- --gpu-info --opencl-lib "$cl_stub"
GPU_ENV="STUB_OPENCL_NO_FP16=1"
t "gpu-info: a device without cl_khr_fp16" 0 'fp16 arithmetic no' -- --gpu-info --opencl-lib "$cl_stub"
GPU_ENV="STUB_OPENCL_NONE=1"
t "gpu-info: a loader that finds no platform, exit 3" 3 'no OpenCL platform \(clGetPlatformIDs -1001, CL_PLATFORM_NOT_FOUND_KHR\)' -- --gpu-info --opencl-lib "$cl_stub"
GPU_ENV=
t "gpu-info: --opencl-icd reaches the environment before the loader is used" 0 'OCL_ICD_FILENAMES=/usr/lib/libmali.so.0' 'Stub ARM Platform \[OCL_ICD_FILENAMES=/usr/lib/libmali.so.0\]' -- \
  --gpu-info --opencl-lib "$cl_stub" --opencl-icd /usr/lib/libmali.so.0
mkdir "$tmp/icd"
printf 'libm.so.6\n' > "$tmp/icd/a.icd"
printf '/nonexistent/libmali.so\n' > "$tmp/icd/b.icd"
GPU_ENV="OCL_ICD_VENDORS=$tmp/icd"
t "gpu-info: the .icd files are listed with what they point at and whether that loads" 0 'a.icd -> libm.so.6 \(that library loads\)' 'b.icd -> /nonexistent/libmali.so \(does not load' -- --gpu-info --opencl-lib "$cl_stub"
GPU_ENV=
t "gpu-info: no loader at all, exit 3" 3 'ICD loader not loaded' -- --gpu-info --opencl-lib /nonexistent/libOpenCL.so.1
t "gpu-info needs no model" 0 'GPU delegate: TFLite [0-9.]+ GPU delegate V2 built into the probe' -- --gpu-info --opencl-lib "$cl_stub"

t "detections are decoded, sorted, thresholded and named" 0 'person +0.910 +box y 0.100..0.600 x 0.200..0.500' 'dog +0.620' -- --model "$m/ssd_const.tflite" --runs 2 --warmup 0 --cpu-backend builtin
n "detection below the threshold is hidden" 'cat +0.120'
t "threshold and top-k" 0 'person +0.910' 'cat +0.120' -- --model "$m/ssd_const.tflite" --runs 1 --warmup 0 --cpu-backend builtin --min-score 0.1 --top 3
printf 'thing-zero\n' > "$tmp/labels.txt"
t "labels file" 0 'thing-zero +0.910' 'class 17 +0.620' -- --model "$m/ssd_const.tflite" --runs 1 --warmup 0 --cpu-backend builtin --labels "$tmp/labels.txt"

t "bad usage" 2 'usage:' -- --runs 3
t "delegate option without a value" 2 'wants KEY=VALUE' -- --model "$m/float_net.tflite" --delegate-opt novalue
t "system info without a model" 0 'probe runtime: TFLite [0-9]+\.[0-9]+\.[0-9]+' 'delegate: ' -- --system-info --delegate "$stub"
t "nnapi and a delegate plugin are separate routes" 2 'separate routes' -- --model "$m/float_net.tflite" --nnapi --delegate "$stub"
t "gpu and nnapi are separate routes" 2 'separate routes' -- --model "$m/float_net.tflite" --gpu --nnapi
t "gpu and a delegate plugin are separate routes" 2 'separate routes' -- --model "$m/float_net.tflite" --gpu --delegate "$stub"
t "gpu option without a value" 2 'wants KEY=VALUE' -- --model "$m/float_net.tflite" --gpu-opt novalue
t "gpu option with a bad choice" 2 'precision is one of fp32, fp16' -- --model "$m/float_net.tflite" --gpu-opt precision=fp8
t "gpu option partitions out of range" 2 'partitions is a whole number from 1 to 64' -- --model "$m/float_net.tflite" --gpu-opt partitions=0
t "gpu option quant is a boolean" 2 'quant is true or false' -- --model "$m/float_net.tflite" --gpu-opt quant=maybe
t "gpu option with an unknown key" 2 'unknown --gpu-opt key' -- --model "$m/float_net.tflite" --gpu-opt bogus=1
t "nnapi option without a value" 2 'wants KEY=VALUE' -- --model "$m/float_net.tflite" --nnapi-opt novalue
t "nnapi option with a bad number" 2 'sdk is not a whole number' -- --model "$m/float_net.tflite" --nnapi-opt sdk=x
t "nnapi option with an unknown key" 2 'unknown --nnapi-opt key' -- --model "$m/float_net.tflite" --nnapi-opt bogus=1
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
  # what TFLite 2.13.1's GPU delegate takes of the detector, by its own selection code (no GPU): everything but the post-processing op
  t "EfficientDet-Lite0 int8: the GPU delegate takes 266 of 267 nodes in one piece" 0 'nodes: 267; the GPU delegate can take 266 of them in 1 piece\(s\) of 266' 'on the CPU: TFLite_Detection_PostProcess x1$' \
    'CUSTOM TFLite_Detection_PostProcess' -- --model "$det" --gpu-coverage
  ssd=$models/tf2_ssd_mobilenet_v2_coco17_ptq.tflite
  if [ -f "$ssd" ]; then
    t "SSD-MobileNetV2 int8: the GPU delegate takes 101 of 102 nodes in one piece" 0 'nodes: 102; the GPU delegate can take 101 of them in 1 piece\(s\) of 101' 'on the CPU: TFLite_Detection_PostProcess x1$' -- --model "$ssd" --gpu-coverage
  fi
  for f32 in mp_efficientdet_lite0_float32:263 mp_ssd_mobilenet_v2_float32:199; do
    f=$models/${f32%%:*}.tflite
    [ -f "$f" ] && t "${f32%%:*}: all ${f32##*:} nodes go to the GPU" 0 "nodes: ${f32##*:}; the GPU delegate can take ${f32##*:} of them in 1 piece" 'on the CPU: \(none\)' -- --model "$f" --gpu-coverage
  done
else
  echo "skip real-model test (run fetch-model.sh, and point NPU_TV_ROOT at a copy of the TV's files)"
fi

# --- gpu-ui-impact.py (the page-flip cadence sampler of the TV slot) against fake counter files
if [ "$target" = host ]; then
  if command -v python3 >/dev/null 2>&1; then
    uiout=$(python3 "$here/test_ui_impact.py" 2>&1) && ok "gpu-ui-impact.py: 6 checks on fake counters" || bad "gpu-ui-impact.py" "$uiout"
  else
    echo "skip gpu-ui-impact.py tests (no python3)"
  fi
fi

echo "$pass passed, $fail failed ($target)"
[ "$fail" = 0 ]
