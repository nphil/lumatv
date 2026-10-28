#!/bin/sh
# TV slot for lumatv-npu-probe. Run as root on the TV, in a shell with a PTY, with LG's compositor and apps live.
# It reads LG's state, loads CPU for a few minutes, and runs LG's delegate in-process; it writes only under $W, installs
# nothing and changes no setting. Rollback: rm -rf $W (the probe holds no device after it exits).
#
# Pushed beforehand (docs/research/npu-result.md, "TV slot"):
#   $W/lumatv-npu-probe   the arm build (npuprobe/build.sh arm)
#   $W/models/            fetch-model.sh output: efficientdet_lite0_320_ptq.tflite, coco_labels.txt; with --all and --gpu also
#                         tf2_ssd_mobilenet_v2_coco17_ptq.tflite, mp_efficientdet_lite0_float32.tflite, mp_ssd_mobilenet_v2_float32.tflite
#   $W/gpu-ui-impact.py   for --gpu-run (python3 is on the set)
#   $W/tv-probe.sh        this file
#
#   sh $W/tv-probe.sh [--runs N] [--threads N] [--no-luna] [--npu-run] [--force] [--assume-abi] [--nnapi-run] [--gpu-run]
#     --npu-run  also run any model found in step 2 that carries an lgnpu_custom_op through LG's delegate (opens /dev/npu0)
#     --force    carry on although something already holds the NPU
#     --assume-abi  run the delegate steps although step 1 could not compare the TFLite releases
#     --nnapi-run   step 6b: our model through the NNAPI delegate (accelerator lgnpu) over the device's NN HAL, with the probe and then
#                   with LG's benchmark_model (the one route a builtin-op model could still take; it opens /dev/npu0 only if the
#                   HAL behind it is a real one: the build analysed offline, step 6a, is a stand-in that sleeps 1 ms)
#     --gpu-run     step 6d: the models through TFLite's GPU delegate on OpenCL (the Mali is busy for a few seconds at a time, one
#                   process of ours; LG's compositor keeps running) and the UI's page-flip cadence while it runs. Steps 6c (what OpenCL
#                   and the Mali offer, what the delegate would take of each model) and 6e (LG's AI picture services, files and luna
#                   reads) always run and only read.
set -u
W=${W:-/tmp/lumatv-npu}
OUT=$W/results
PROBE=$W/lumatv-npu-probe
LGMODEL=${LGMODEL:-/usr/share/aif/model/FitTV_Detector_Yolov3_QAT_SoC.tflite}
IMG=${IMG:-/usr/share/aif/images/people.jpg}
OURS=${OURS:-$W/models/efficientdet_lite0_320_ptq.tflite}
ALT=${ALT:-$W/models/tf2_ssd_mobilenet_v2_coco17_ptq.tflite}  # optional second candidate (fetch-model.sh --all)
OURS_F32=${OURS_F32:-$W/models/mp_efficientdet_lite0_float32.tflite}  # optional, for the GPU (fetch-model.sh --gpu)
ALT_F32=${ALT_F32:-$W/models/mp_ssd_mobilenet_v2_float32.tflite}
LABELS=${LABELS:-$W/models/coco_labels.txt}
DELEGATE=${DELEGATE:-/usr/lib/libtflite-npu-delegate.so.1}
LIBDIR=${LIBDIR:-/usr/lib}
NNHAL_TEST=${NNHAL_TEST:-/usr/bin/lgnpu_online_nnhal_test}   # LG's gtest of the online NN HAL
OFFLINE_TEST=${OFFLINE_TEST:-/usr/bin/lgnpu_nn_hal_test}     # and of the offline HAL
RUN_LGNPU=${RUN_LGNPU:-/usr/bin/run_lgnpu}
RUNS=100
THREADS=3
LUNA=1
NPURUN=0
FORCE=0
ASSUME_ABI=0
NNAPIRUN=0
GPURUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --runs) RUNS=$2; shift ;;
    --threads) THREADS=$2; shift ;;
    --no-luna) LUNA=0 ;;
    --npu-run) NPURUN=1 ;;
    --force) FORCE=1 ;;
    --assume-abi) ASSUME_ABI=1 ;;
    --nnapi-run) NNAPIRUN=1 ;;
    --gpu-run) GPURUN=1 ;;
    *) echo "usage: sh tv-probe.sh [--runs N] [--threads N] [--no-luna] [--npu-run] [--force] [--assume-abi] [--nnapi-run] [--gpu-run]"; exit 2 ;;
  esac
  shift
done
mkdir -p "$OUT"
[ -x "$PROBE" ] || { echo "no $PROBE"; exit 2; }
SUMMARY=$OUT/summary.txt
: > "$SUMMARY"
# libwebos_nnhal.so as analysed offline (docs/research/npu-result.md): an NN HAL 1.2 device "lgnpu" whose execute call sleeps 1 ms.
ANALYSED_NNHAL=963204be235cdcdf9361e61a7505280d9b000ee27d59e8627bc88de46c03d230

step() { echo; echo "=== $1"; }
note() { echo "$*" | tee -a "$SUMMARY"; }
npu_state() {
  for f in npu_device_capability npu_utilization npu_process_list; do
    echo "-- /proc/npu/$f"
    cat "/proc/npu/$f" 2>&1
  done
}
holders() {  # processes with /dev/npu0 open
  for fd in /proc/[0-9]*/fd/*; do
    t=$(readlink "$fd" 2>/dev/null) || continue
    case "$t" in */dev/npu0) p=${fd#/proc/}; p=${p%%/*}; echo "$p $(cat /proc/$p/comm 2>/dev/null)" ;; esac
  done
}

step "0. before: who holds the NPU, CPU state"
{
  date
  uname -a
  cat /proc/loadavg
  echo "online cpus: $(cat /sys/devices/system/cpu/online 2>/dev/null)"
  for c in /sys/devices/system/cpu/cpu[0-9]*; do
    [ -r "$c/cpufreq/scaling_cur_freq" ] && echo "$(basename "$c") $(cat "$c/cpufreq/scaling_cur_freq") kHz $(cat "$c/cpufreq/scaling_governor" 2>/dev/null)"
  done
  ls -l /dev/npu0
  npu_state
  echo "-- processes holding /dev/npu0:"
  holders
} > "$OUT/00-before.txt" 2>&1
cat "$OUT/00-before.txt"
AIPQ=$(grep -i aipq /proc/npu/npu_device_capability 2>/dev/null)
note "0: ${AIPQ:-aipq compatibility: not readable}  (false: the NPU is not meant to be shared with AI picture (ai_pq); only steps 6 and 6b ever open it, and only after this check)"
if [ -n "$(cat /proc/npu/npu_process_list 2>/dev/null)" ] || [ -n "$(holders)" ]; then
  note "WARNING: something holds the NPU already (see 00-before.txt)"
  [ "$FORCE" = 1 ] || { echo "stopping; --force to go on"; exit 3; }
fi

step "1. what the device has (TFLite runtime version, XNNPACK, the delegate's entry points)"
{
  ls -l /usr/lib/libtensorflow-lite* /usr/lib/libXNNPACK* /usr/lib/libtflite-npu-delegate* /usr/lib/libnnrt* /usr/lib/libnpu* /usr/lib/libneuralnetworks* /usr/lib/libwebos_nnhal* 2>&1
  ls -l /usr/bin/benchmark_model* /usr/sbin/ai-inference-manager 2>&1
  "$PROBE" --system-info --delegate "$DELEGATE" 2>&1
} > "$OUT/01-system.txt" 2>&1
cat "$OUT/01-system.txt"
ABI=$ASSUME_ABI
grep -q "same minor release" "$OUT/01-system.txt" && ABI=1
note "1: $(grep -E '^ABI:' "$OUT/01-system.txt" || echo 'ABI: not determined')"

step "2. models on the TV that carry lgnpu_custom_op (an NPU package), read only"
: > "$OUT/02-npu-models.txt"
for d in /usr/share /usr/lib /mnt/lg /media /var /opt; do
  [ -d "$d" ] || continue
  find "$d" -xdev -type f -name '*.tflite' 2>/dev/null | while read -r f; do
    if grep -q lgnpu_custom_op "$f" 2>/dev/null; then echo "NPU $f"; else echo "cpu $f"; fi
  done
done >> "$OUT/02-npu-models.txt"
cat "$OUT/02-npu-models.txt"
NPUMODELS=$(grep '^NPU ' "$OUT/02-npu-models.txt" | cut -d' ' -f2-)
note "2: $(grep -c '^NPU ' "$OUT/02-npu-models.txt") of $(grep -c . "$OUT/02-npu-models.txt") .tflite files carry lgnpu_custom_op"

if [ "$LUNA" = 1 ]; then
  step "3. ai-inference-manager, the query group only (getDeviceCapability, getDeviceStatus, getResourceList)"
  {
    for m in getDeviceCapability getDeviceStatus getResourceList; do
      echo "-- $m"
      if command -v timeout >/dev/null 2>&1; then
        timeout 10 luna-send -n 1 -f "luna://com.webos.service.aiinferencemanager/$m" '{}' 2>&1
      else
        luna-send -n 1 -f "luna://com.webos.service.aiinferencemanager/$m" '{}' 2>&1
      fi
    done
  } > "$OUT/03-luna.txt" 2>&1
  cat "$OUT/03-luna.txt"
fi

step "4. CPU, no delegate: LG's detector and ours, $RUNS runs"
for backend in xnnpack builtin; do
  for t in 1 "$THREADS"; do
    for m in "$LGMODEL" "$OURS" "$ALT"; do
      [ -f "$m" ] || continue
      tag=$(basename "$m" .tflite)-$backend-t$t
      "$PROBE" --model "$m" --image "$IMG" --labels "$LABELS" --runs "$RUNS" --warmup 5 --threads "$t" --cpu-backend "$backend" > "$OUT/04-$tag.txt" 2>&1
      echo "$tag: $(grep '^SUMMARY' "$OUT/04-$tag.txt" | cut -d' ' -f3-)" | tee -a "$SUMMARY"
    done
  done
done
echo "-- detections, ours:"
grep -A6 "top detections" "$OUT/04-efficientdet_lite0_320_ptq-xnnpack-t$THREADS.txt" 2>/dev/null | head -8

step "5. LG's delegate on both models, $RUNS runs each (expected: refused; the timed runs are then the CPU path with builtin kernels, nothing opened)"
if [ "$ABI" = 1 ]; then
  for m in "$LGMODEL" "$OURS"; do
    [ -f "$m" ] || continue
    tag=$(basename "$m" .tflite)
    "$PROBE" --model "$m" --image "$IMG" --labels "$LABELS" --runs "$RUNS" --warmup 1 --threads "$THREADS" --cpu-backend none --delegate "$DELEGATE" --delegate-opt min_freq=30 > "$OUT/05-$tag.txt" 2>&1
    echo "exit $?" >> "$OUT/05-$tag.txt"
    echo "$tag: $(grep -E 'delegate (NOT APPLIED|refused)|ModifyGraphWithDelegate|CRASH|exit ' "$OUT/05-$tag.txt" | tr '\n' ';') $(grep '^SUMMARY' "$OUT/05-$tag.txt" | cut -d' ' -f3-)" | tee -a "$SUMMARY"
  done
  echo "-- LG's own log lines about it (PmLog, /var/log/messages):"
  grep -a NPUDELEGATE /var/log/messages 2>/dev/null | tail -5
else
  note "5: skipped, the probe's TFLite is not the device's minor release (step 1)"
fi

step "6. an NPU model through the delegate"
if [ -n "$NPUMODELS" ] && [ "$NPURUN" = 1 ] && [ "$ABI" = 1 ]; then
  for m in $NPUMODELS; do
    tag=$(basename "$m" .tflite)
    echo "-- $m"
    "$PROBE" --model "$m" --fill const:0 --runs "$RUNS" --warmup 5 --threads "$THREADS" --delegate "$DELEGATE" --delegate-opt min_freq=30 --require-delegate > "$OUT/06-$tag.txt" 2>&1
    echo "exit $?" >> "$OUT/06-$tag.txt"
    echo "$tag: $(grep '^SUMMARY' "$OUT/06-$tag.txt" | cut -d' ' -f3-)" | tee -a "$SUMMARY"
    cat /proc/npu/npu_process_list
  done
elif [ -n "$NPUMODELS" ]; then
  note "6: NPU models exist ($(echo "$NPUMODELS" | wc -l)); rerun with --npu-run to open /dev/npu0 with them"
else
  note "6: no model with an NPU package on this TV, so nothing to run on the NPU"
fi

step "6a. LG's NN HAL tools (read only): the online HAL's own test, run_lgnpu's usage"
HALSUM=$(sha256sum "$LIBDIR/libwebos_nnhal.so" 2>/dev/null | cut -d' ' -f1)
{
  echo "-- hashes; libwebos_nnhal.so $ANALYSED_NNHAL is the build analysed offline"
  ( cd "$LIBDIR" && sha256sum libwebos_nnhal.so libwebos_offline_nnhal.so.1.0.0 libnnrt.so.1.0.0 libnpu.so.1.0.0 libtflite-npu-delegate.so.1.0.0 2>&1 )
  echo "-- NNAPI runtime files: $(ls "$LIBDIR"/libneuralnetworks* 2>&1 | tr '\n' ' ')"
  if [ "$HALSUM" = "$ANALYSED_NNHAL" ]; then
    echo "-- lgnpu_online_nnhal_test, the analysed build (it opens no device)"
    ( cd "$W" && "$NNHAL_TEST" ) 2>&1
    echo "exit $?"
  else
    echo "-- libwebos_nnhal.so is not the analysed build: lgnpu_online_nnhal_test NOT run; reread the library (tests/lg/hal_mock.sh) first"
  fi
  echo "-- run_lgnpu -h (its documented arguments; a run needs a .lgnpu package, and none is shipped)"
  "$RUN_LGNPU" -h 2>&1 | head -n 5
  echo "-- npu_process_list after: $(cat /proc/npu/npu_process_list 2>&1)"
} > "$OUT/06a-hal-tools.txt" 2>&1
cat "$OUT/06a-hal-tools.txt"
if [ "$HALSUM" = "$ANALYSED_NNHAL" ]; then HALNOTE="is the analysed stand-in"; else HALNOTE="DIFFERS from the analysed build"; fi
note "6a: libwebos_nnhal.so $HALNOTE; $(grep -E 'PASSED|FAILED|NOT run' "$OUT/06a-hal-tools.txt" | head -n 1)"
if [ "$NPURUN" = 1 ]; then
  mkdir -p "$W/offline" && ( cd "$W/offline" && "$OFFLINE_TEST" ) > "$OUT/06a-offline-hal-test.txt" 2>&1
  echo "exit $?" >> "$OUT/06a-offline-hal-test.txt"
  note "6a: offline HAL test (needs ./dne_custom_op.lgnpu, which is not shipped): $(grep -E 'PASSED|FAILED|fail to open|^exit' "$OUT/06a-offline-hal-test.txt" | head -n 3 | tr '\n' ';')"
fi

step "6b. NNAPI route to the HAL (only with --nnapi-run): our model through the NNAPI delegate, accelerator lgnpu"
if [ "$NNAPIRUN" = 1 ]; then
  echo "${AIPQ:-aipq compatibility: not readable}: false means the NPU is not shared with AI picture. If AI picture is on and the picture changes while this runs, stop it (kill -9 the probe); turning AI picture back on is a settings action for a person."
  "$PROBE" --nnapi-list > "$OUT/06b-nnapi-list.txt" 2>&1
  cat "$OUT/06b-nnapi-list.txt"
  note "6b runtime and devices: $(grep -E 'NNAPI runtime|NNAPI devices|\[[0-9]+\]' "$OUT/06b-nnapi-list.txt" | cut -c1-110 | tr '\n' ';')"
  if [ -f "$OURS" ]; then
    for parts in 3 16; do
      tag=ours-partitions$parts
      "$PROBE" --model "$OURS" --image "$IMG" --labels "$LABELS" --runs "$RUNS" --warmup 3 --threads "$THREADS" --cpu-backend xnnpack --nnapi --nnapi-opt accelerator=lgnpu --nnapi-opt partitions=$parts > "$OUT/06b-$tag.txt" 2>&1
      echo "exit $?" >> "$OUT/06b-$tag.txt"
      echo "$tag: $(grep -E 'delegated ops|delegate kernel\(s\) took|NOT APPLIED|refused|not created|WARNING|^exit ' "$OUT/06b-$tag.txt" | cut -c1-200 | tr '\n' ';') $(grep '^SUMMARY' "$OUT/06b-$tag.txt" | cut -d' ' -f3-)" | tee -a "$SUMMARY"
    done
  else
    note "6b: no $OURS, probe run skipped"
  fi
  BM=$(ls /usr/bin/benchmark_model* 2>/dev/null | head -n 1)
  if [ -n "$BM" ] && [ -f "$OURS" ]; then
    (
      i=0
      while [ $i -lt 600 ]; do echo "$(cat /proc/npu/npu_utilization 2>/dev/null)"; i=$((i + 1)); sleep 0.05 2>/dev/null || sleep 1; done
    ) > "$OUT/06b-util.txt" 2>&1 &
    SAMPLER=$!
    "$BM" --graph="$OURS" --num_runs=20 --warmup_runs=2 --num_threads="$THREADS" --use_nnapi=true --nnapi_accelerator_name=lgnpu > "$OUT/06b-benchmark-model.txt" 2>&1
    echo "exit $?" >> "$OUT/06b-benchmark-model.txt"
    kill "$SAMPLER" 2>/dev/null
    tail -n 25 "$OUT/06b-benchmark-model.txt"
    note "6b: LG's benchmark_model, nnapi lgnpu: $(grep -i -E 'nnapi|delegate|average|avg' "$OUT/06b-benchmark-model.txt" | head -n 4 | tr '\n' ';'); npu utilisation samples above 0: $(grep -c -v '^0$' "$OUT/06b-util.txt")"
  else
    note "6b: no benchmark_model or no model, LG's tool skipped"
  fi
else
  note "6b: skipped (--nnapi-run)"
fi

step "6c. GPU, read only: the OpenCL loader and driver, what the Mali offers, what the GPU delegate would take of our models"
{
  echo "-- files"
  ls -l /usr/lib/libOpenCL* /usr/lib/libOpenCLDriverStub* /usr/lib/libmali* /usr/lib/libtensorflowlite_gpu_delegate* /etc/OpenCL /etc/OpenCL/vendors 2>&1
  for f in /etc/OpenCL/vendors/*.icd; do [ -f "$f" ] && echo "$f: $(cat "$f")"; done
  echo "-- OpenCL names inside the driver and loader binaries (lines containing each)"
  for l in /usr/lib/libmali.so.0 /usr/lib/libmali.so /usr/lib/libOpenCL.so.1; do
    [ -f "$l" ] || continue
    echo "$l: clGetPlatformIDs $(grep -c clGetPlatformIDs "$l"), clIcdGetPlatformIDsKHR $(grep -c clIcdGetPlatformIDsKHR "$l"), clEnqueueNDRangeKernel $(grep -c clEnqueueNDRangeKernel "$l"), cl_khr_fp16 $(grep -c cl_khr_fp16 "$l"), cl_khr_priority_hints $(grep -c cl_khr_priority_hints "$l")"
  done
  echo "-- the Mali kernel driver's own files"
  for f in $(find /sys/devices /sys/class/misc -maxdepth 6 \( -name gpuinfo -o -name power_policy -o -name core_mask -o -name pm_poweroff -o -name js_scheduling_period -o -name dvfs_period -o -name js_timeouts \) 2>/dev/null | head -n 20); do
    echo "$f: $(head -c 300 "$f" 2>&1 | tr '\n' ' ')"
  done
  echo "-- the probe's view of OpenCL (--gpu-info; exit 3 means no device)"
  "$PROBE" --gpu-info 2>&1
  echo "exit $?"
} > "$OUT/06c-gpu.txt" 2>&1
GPU_ICD=""
if ! grep -q 'platform \[0\]' "$OUT/06c-gpu.txt"; then
  for l in /usr/lib/libmali.so.0 /usr/lib/libmali.so; do
    [ -f "$l" ] || continue
    echo "-- no platform through the loader's own search: the Mali library named as the driver (OCL_ICD_FILENAMES=$l, this process only)" >> "$OUT/06c-gpu.txt"
    "$PROBE" --gpu-info --opencl-icd "$l" >> "$OUT/06c-gpu.txt" 2>&1
    if tail -n 20 "$OUT/06c-gpu.txt" | grep -q 'platform \[0\]'; then GPU_ICD=$l; break; fi
  done
fi
cat "$OUT/06c-gpu.txt"
note "6c: $(grep -E '^ *platform \[|^ *device \[' "$OUT/06c-gpu.txt" | cut -c1-120 | tr '\n' ';') ${GPU_ICD:+ICD needed: $GPU_ICD;} $(grep -E 'fp16 arithmetic' "$OUT/06c-gpu.txt" | head -n 1 | sed 's/^ *//')"
for m in "$OURS" "$OURS_F32" "$ALT" "$ALT_F32"; do
  [ -f "$m" ] || continue
  tag=$(basename "$m" .tflite)
  "$PROBE" --model "$m" --gpu-coverage ${GPU_ICD:+--opencl-icd "$GPU_ICD"} > "$OUT/06c-coverage-$tag.txt" 2>&1
  note "6c: $tag, GPU delegate: $(grep -E 'nodes:|on the CPU:' "$OUT/06c-coverage-$tag.txt" | sed 's/^ *//' | tr '\n' ';')"
done

step "6d. GPU runs (only with --gpu-run): the models through TFLite's GPU delegate on OpenCL, then the UI's flip cadence while one runs"
if [ "$GPURUN" = 1 ]; then
  tmo() { s=$1; shift; if command -v timeout >/dev/null 2>&1; then timeout "$s" "$@"; else "$@"; fi; }
  rm -rf "$W/gpucache"
  gpu_one() {  # TAG MODEL gpu-opts...: a CPU phase (XNNPACK) and a GPU phase in one probe run
    tag=$1; m=$2; shift 2
    [ -f "$m" ] || return 0
    tmo 900 "$PROBE" --model "$m" --image "$IMG" --labels "$LABELS" --runs "$RUNS" --warmup 5 --threads "$THREADS" --cpu-backend xnnpack --gpu ${GPU_ICD:+--opencl-icd "$GPU_ICD"} "$@" > "$OUT/06d-$tag.txt" 2>&1
    echo "exit $?" >> "$OUT/06d-$tag.txt"
    note "$tag: $(grep -E 'delegate kernel\(s\) took|NOT APPLIED|refused|not created|WARNING|^exit ' "$OUT/06d-$tag.txt" | cut -c1-150 | tr '\n' ';') $(grep '^SUMMARY' "$OUT/06d-$tag.txt" | cut -d' ' -f3-)"
  }
  # ours (int8): the cache cold, then warm, then 32-bit arithmetic, then the short-start preference
  gpu_one ours-int8-fp16-cold "$OURS" --gpu-opt precision=fp16 --gpu-opt cache="$W/gpucache"
  gpu_one ours-int8-fp16-warm "$OURS" --gpu-opt precision=fp16 --gpu-opt cache="$W/gpucache"
  gpu_one ours-int8-fp32 "$OURS" --gpu-opt precision=fp32
  gpu_one ours-int8-fp16-fast "$OURS" --gpu-opt precision=fp16 --gpu-opt pref=fast
  # the float32 networks (no QUANTIZE/DEQUANTIZE in the graph)
  gpu_one lite0-f32-fp16-cold "$OURS_F32" --gpu-opt precision=fp16 --gpu-opt cache="$W/gpucache"
  gpu_one lite0-f32-fp16-warm "$OURS_F32" --gpu-opt precision=fp16 --gpu-opt cache="$W/gpucache"
  gpu_one lite0-f32-fp32 "$OURS_F32" --gpu-opt precision=fp32
  # SSD-MobileNetV2
  gpu_one ssd-int8-fp16 "$ALT" --gpu-opt precision=fp16 --gpu-opt cache="$W/gpucache"
  gpu_one ssd-f32-fp16 "$ALT_F32" --gpu-opt precision=fp16 --gpu-opt cache="$W/gpucache"
  gpu_one ssd-f32-fp32 "$ALT_F32" --gpu-opt precision=fp32
  # What a GPU loop does to the UI's page flips at Home (nothing animating), against the same loop on the CPU
  if [ -f "$W/gpu-ui-impact.py" ] && command -v python3 >/dev/null 2>&1 && [ -f "$OURS" ]; then
    python3 "$W/gpu-ui-impact.py" --before 6 --after 4 --log "$OUT/06d-ui-gpu-probe.txt" -- "$PROBE" --model "$OURS" --image "$IMG" --runs 300 --warmup 5 --cpu-backend none --gpu ${GPU_ICD:+--opencl-icd "$GPU_ICD"} --gpu-opt precision=fp16 --gpu-opt cache="$W/gpucache" > "$OUT/06d-ui-gpu.txt" 2>&1
    note "6d: Home's flip cadence around a 300-run GPU loop: $(grep '^UIIMPACT' "$OUT/06d-ui-gpu.txt" | cut -d' ' -f2-)"
    python3 "$W/gpu-ui-impact.py" --before 6 --after 4 --log "$OUT/06d-ui-cpu-probe.txt" -- "$PROBE" --model "$OURS" --image "$IMG" --runs 300 --warmup 5 --threads "$THREADS" --cpu-backend xnnpack > "$OUT/06d-ui-cpu.txt" 2>&1
    note "6d: Home's flip cadence around a 300-run CPU loop (XNNPACK, $THREADS threads): $(grep '^UIIMPACT' "$OUT/06d-ui-cpu.txt" | cut -d' ' -f2-)"
  else
    note "6d: no gpu-ui-impact.py, python3 or model: the flip cadence was not measured"
  fi
else
  note "6d: skipped (--gpu-run)"
fi

step "6e. LG's own AI picture services, read only: who runs them, the files they leave, what nn-proxy answers on luna"
{
  echo "-- processes with LG's AI libraries mapped"
  for m in /proc/[0-9]*/maps; do
    p=${m#/proc/}; p=${p%%/*}
    libs=$(grep -o -E 'libaibspLib\.so|libaibsp\.so|libnpu\.so|libnnrt\.so|libwebos_[a-z_]*nnhal\.so|libtflite-npu-delegate\.so|libedgeai-vision\.so' "$m" 2>/dev/null | sort -u | tr '\n' ' ')
    [ -n "$libs" ] && echo "$p $(cat /proc/$p/comm 2>/dev/null): $(tr '\0' ' ' < /proc/$p/cmdline 2>/dev/null | cut -c1-100): $libs"
  done
  echo "-- device nodes and who holds them open"
  ls -l /dev/lne* /dev/npu0 /dev/mali0 2>&1
  for fd in /proc/[0-9]*/fd/*; do
    t=$(readlink "$fd" 2>/dev/null) || continue
    case "$t" in */dev/lne*|*/dev/npu0|*/dev/mali0) p=${fd#/proc/}; p=${p%%/*}; echo "$t $p $(cat /proc/$p/comm 2>/dev/null)" ;; esac
  done | sort -u
  grep -i -E 'lne|npu|mali' /proc/devices 2>&1
  echo "-- /var/aibsp (the work directory of libaibsp and libaibspLib)"
  ls -l /var/aibsp /var/aibsp/image 2>&1 | head -n 40
  for f in /var/aibsp/ai_scene_iir_params.txt /var/aibsp/ai_tone_params.txt /var/aibsp/genre_coef.txt /var/aibsp/scale_zero_point.txt; do
    [ -f "$f" ] && { echo "-- $f"; head -c 600 "$f"; echo; }
  done
  [ -e /var/aibsp/skip ] && echo "/var/aibsp/skip exists: LG's AI initialisation is skipped"
  echo "-- EdgeAI's extension registry (the solutions it offers: FACE, TEXT, BODY ...)"
  for f in /mnt/lg/cmn_data/edgeai/edgeai_extension_registry.json; do [ -f "$f" ] && { head -c 3000 "$f"; echo; }; done
  echo "-- LG's tools' own usage text"
  /usr/bin/airunner --help 2>&1 | head -n 30
} > "$OUT/06e-lg-ai.txt" 2>&1
cat "$OUT/06e-lg-ai.txt"
note "6e: AI library users: $(grep -E '^[0-9]+ ' "$OUT/06e-lg-ai.txt" | cut -c1-100 | tr '\n' ';')"
if [ "$LUNA" = 1 ]; then
  # nn-proxy (com.webos.service.nnproxy) registers /picture/get, /picture/set and /picture/debug; only get is called, and only for the data
  # commands (the strings of the binary name the payload keys type, command, option, selected, subscribe and the values GENRE, SR_SQM, SR_LEARN).
  lcall() { echo "-- luna-send $1 $2"; if command -v timeout >/dev/null 2>&1; then timeout 10 luna-send -n 1 -f "$1" "$2" 2>&1; else luna-send -n 1 -f "$1" "$2" 2>&1; fi; }
  {
    lcall luna://com.webos.service.nnproxy/picture/get '{"type":"GENRE","command":"data"}'
    lcall luna://com.webos.service.nnproxy/picture/get '{"type":"SR_SQM","command":"data"}'
    lcall luna://com.webos.service.nnproxy/picture/get '{"type":"SR_LEARN","command":"data"}'
    lcall luna://com.webos.service.nnproxy/picture/get '{"type":"GENRE"}'
    echo "-- the GenreData subscription for 8 s (what plays now decides what comes)"
    if command -v timeout >/dev/null 2>&1; then timeout 8 luna-send -i -f luna://com.webos.service.nnproxy/picture/get '{"type":"GENRE","command":"data","subscribe":true}' 2>&1; fi
  } > "$OUT/06e-nnproxy.txt" 2>&1
  cat "$OUT/06e-nnproxy.txt"
  note "6e: nn-proxy: $(grep -E 'returnValue|errorText|errorCode|data[1-4]|selected' "$OUT/06e-nnproxy.txt" | cut -c1-110 | head -n 8 | tr '\n' ';')"
else
  note "6e: luna reads skipped (--no-luna)"
fi

step "7. after"
{
  date
  npu_state
  echo "-- processes holding /dev/npu0:"
  holders
  cat /proc/loadavg
  dmesg 2>/dev/null | grep -i -E 'npu|dne' | tail -n 20
} > "$OUT/07-after.txt" 2>&1
cat "$OUT/07-after.txt"
echo
echo "results in $OUT (summary.txt first); copy them off, then: rm -rf $W"
