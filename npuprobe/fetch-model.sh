#!/bin/sh
# Downloads the person/animal detector the probe is meant to run, and a second one for comparison. Not committed:
# the files go under /tmp (or DIR) and are checked against pinned SHA-256 sums.
#   fetch-model.sh [DIR]            efficientdet_lite0_320_ptq.tflite + coco_labels.txt
#   fetch-model.sh [DIR] --all      and tf2_ssd_mobilenet_v2_coco17_ptq.tflite
#
# Source: github.com/google-coral/test_data at the commit below (Apache-2.0, the repository's licence). The
# EfficientDet-Lite0 file is the TF Model Maker network, full-integer post-training quantised: uint8 input, QUANTIZE,
# int8 activations with per-channel int8 weights, TFLite_Detection_PostProcess last. COCO classes, ids 0-89 as in coco_labels.txt.
set -eu
dir=${1:-/tmp/lumatv-npu-models}
all=0
[ "${2:-}" = "--all" ] && all=1
commit=104342d2d3480b3e66203073dac24f4e2dbb4c41

get() {  # name sha256
  f=$dir/$1
  if [ -f "$f" ] && [ "$(sha256sum "$f" | cut -d' ' -f1)" = "$2" ]; then return 0; fi
  mkdir -p "$dir"
  curl -fsSL -m 300 -o "$f.part" "https://github.com/google-coral/test_data/raw/$commit/$1"
  [ "$(sha256sum "$f.part" | cut -d' ' -f1)" = "$2" ] || { echo "$1: checksum mismatch" >&2; rm -f "$f.part"; exit 1; }
  mv "$f.part" "$f"
}

get efficientdet_lite0_320_ptq.tflite 33a3b622c7cac0762f96089353cd61495f3e993968d133af7871bfc2d5396704
get coco_labels.txt dc183f003fc753c4c43fae6fdf7f387559449573f13fa32e517fb7453fd380f1
if [ "$all" = 1 ]; then
  get tf2_ssd_mobilenet_v2_coco17_ptq.tflite 3438ee45f33153b1fe6c49e18bcd869ce7c94ef01c485d4eb027eaf8fc0db8ac
fi
echo "$dir"
