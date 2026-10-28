#!/bin/sh
# TV slot for lumatv-npu-probe. Run as root on the TV, in a shell with a PTY, with LG's compositor and apps live.
# It reads LG's state, loads CPU for a few minutes, and runs LG's delegate in-process; it writes only under $W, installs
# nothing and changes no setting. Rollback: rm -rf $W (the probe holds no device after it exits).
#
# Pushed beforehand (docs/research/npu-result.md, "TV slot"):
#   $W/lumatv-npu-probe   the arm build (npuprobe/build.sh arm)
#   $W/models/            fetch-model.sh output: efficientdet_lite0_320_ptq.tflite, coco_labels.txt
#   $W/tv-probe.sh        this file
#
#   sh $W/tv-probe.sh [--runs N] [--threads N] [--no-luna] [--npu-run] [--force] [--assume-abi] [--nnapi-run]
#     --npu-run  also run any model found in step 2 that carries an lgnpu_custom_op through LG's delegate (opens /dev/npu0)
#     --force    carry on although something already holds the NPU
#     --assume-abi  run the delegate steps although step 1 could not compare the TFLite releases
#     --nnapi-run   step 6b: LG's own benchmark_model with the NNAPI delegate on our model (the one route left for a builtin-op model;
#                   it may open /dev/npu0 through libwebos_nnhal)
set -u
W=${W:-/tmp/lumatv-npu}
OUT=$W/results
PROBE=$W/lumatv-npu-probe
LGMODEL=${LGMODEL:-/usr/share/aif/model/FitTV_Detector_Yolov3_QAT_SoC.tflite}
IMG=${IMG:-/usr/share/aif/images/people.jpg}
OURS=${OURS:-$W/models/efficientdet_lite0_320_ptq.tflite}
ALT=${ALT:-$W/models/tf2_ssd_mobilenet_v2_coco17_ptq.tflite}  # optional second candidate (fetch-model.sh --all)
LABELS=${LABELS:-$W/models/coco_labels.txt}
DELEGATE=${DELEGATE:-/usr/lib/libtflite-npu-delegate.so.1}
RUNS=100
THREADS=3
LUNA=1
NPURUN=0
FORCE=0
ASSUME_ABI=0
NNAPIRUN=0
while [ $# -gt 0 ]; do
  case "$1" in
    --runs) RUNS=$2; shift ;;
    --threads) THREADS=$2; shift ;;
    --no-luna) LUNA=0 ;;
    --npu-run) NPURUN=1 ;;
    --force) FORCE=1 ;;
    --assume-abi) ASSUME_ABI=1 ;;
    --nnapi-run) NNAPIRUN=1 ;;
    *) echo "usage: sh tv-probe.sh [--runs N] [--threads N] [--no-luna] [--npu-run] [--force] [--assume-abi] [--nnapi-run]"; exit 2 ;;
  esac
  shift
done
mkdir -p "$OUT"
[ -x "$PROBE" ] || { echo "no $PROBE"; exit 2; }
SUMMARY=$OUT/summary.txt
: > "$SUMMARY"

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

step "6b. NNAPI route (only with --nnapi-run): LG's benchmark_model, our model, delegate nnapi, accelerator lgnpu"
if [ "$NNAPIRUN" = 1 ]; then
  BM=$(ls /usr/bin/benchmark_model* 2>/dev/null | head -n 1)
  if [ -n "$BM" ] && [ -f "$OURS" ]; then
    (
      i=0
      while [ $i -lt 600 ]; do echo "$(cat /proc/npu/npu_utilization 2>/dev/null)"; i=$((i + 1)); sleep 0.05 2>/dev/null || sleep 1; done
    ) > "$OUT/06b-util.txt" 2>&1 &
    SAMPLER=$!
    "$BM" --graph="$OURS" --num_runs=20 --warmup_runs=2 --num_threads="$THREADS" --use_nnapi=true --nnapi_accelerator_name=lgnpu > "$OUT/06b-nnapi.txt" 2>&1
    echo "exit $?" >> "$OUT/06b-nnapi.txt"
    kill "$SAMPLER" 2>/dev/null
    tail -25 "$OUT/06b-nnapi.txt"
    note "6b: benchmark_model nnapi: $(grep -i -E 'nnapi|delegate|average|avg' "$OUT/06b-nnapi.txt" | head -4 | tr '\n' ';'); npu utilisation samples above 0: $(grep -c -v '^0$' "$OUT/06b-util.txt")"
  else
    note "6b: no benchmark_model or no model, skipped"
  fi
else
  note "6b: skipped (--nnapi-run)"
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
