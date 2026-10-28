#!/bin/sh
# Builds lumatv-npu-probe, the stub delegate, the stub NNAPI runtime and the test-model generator.
#   build.sh [arm|host|all] [-j N]
# arm  -> build/arm/  : 32-bit ARM soft-float (the TV; also runs under qemu-arm-static)
# host -> build/host/ : x86-64, for the stub-delegate tests
# First run unpacks the compilers (toolchain.py), fetches TensorFlow Lite 2.13.1 and stb_image.h, and builds TFLite with its
# dependencies for each target (about ten minutes each on 8 cores).
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=$here/build
what=all
jobs=$(( $(nproc) / 2 ))
while [ $# -gt 0 ]; do
  case "$1" in
    arm|host|all) what=$1 ;;
    -j) shift; jobs=$1 ;;
    *) echo "usage: build.sh [arm|host|all] [-j N]" >&2; exit 2 ;;
  esac
  shift
done
[ "$jobs" -ge 1 ] || jobs=1

tools=${NPU_TOOLS:-/tmp/lumatv-npu-tools}
[ -f "$tools/env.sh" ] || python3 "$here/toolchain.py"
. "$tools/env.sh"

# TensorFlow Lite 2.13.x, the layout LG's delegate was built against (CMakeLists.txt); TFLITE_VERSION and TFLITE_SHA256 pick another
tf=${TFLITE_VERSION:-2.13.1}
case "$tf" in
  2.13.1) tf_sha256=89c07aebd4f41fbe0d08cc88aef00305542134f2f16d3b62918dc3c1182f33e2 ;;
  2.9.3) tf_sha256=59d09bd00eef6f07477eea2f50778582edd4b7b2850a396f1fd0c646b357a573 ;;
  *) tf_sha256=${TFLITE_SHA256:?set TFLITE_SHA256 for TensorFlow $tf} ;;
esac
src=$out/src/tensorflow-$tf
if [ ! -f "$src/tensorflow/lite/CMakeLists.txt" ]; then
  mkdir -p "$out/src"
  curl -fsSL -m 600 -o "$out/src/tf-$tf.tar.gz" "https://github.com/tensorflow/tensorflow/archive/refs/tags/v$tf.tar.gz"
  [ "$(sha256sum "$out/src/tf-$tf.tar.gz" | cut -d' ' -f1)" = "$tf_sha256" ] || { echo "TensorFlow $tf: checksum mismatch" >&2; exit 1; }
  tar xzf "$out/src/tf-$tf.tar.gz" -C "$out/src"
fi
python3 "$here/patch_tflite.py" "$src" >/dev/null
stb=$("$here/third_party/fetch.sh" "$out/third_party")

build() {
  target=$1
  case "$target" in
    arm) toolchain=$here/cmake/arm-softfp.cmake ;;
    host) toolchain=$here/cmake/host.cmake ;;
  esac
  cmake -S "$here" -B "$out/$target" -G Ninja -DCMAKE_BUILD_TYPE=Release -DCMAKE_TOOLCHAIN_FILE="$toolchain" \
    -DTFLITE_SRC="$src" -DSTB_DIR="$stb" -DCMAKE_POLICY_VERSION_MINIMUM=3.5
  ninja -C "$out/$target" -j "$jobs" lumatv-npu-probe stub-npu-delegate stub-neuralnetworks stub-neuralnetworks-12 not-a-delegate make-test-models
  echo "built $out/$target/lumatv-npu-probe"
}

case "$what" in
  arm) build arm ;;
  host) build host ;;
  all) build host; build arm ;;
esac
if [ -f "$out/arm/lumatv-npu-probe" ]; then
  arm-linux-gnueabi-readelf -h "$out/arm/lumatv-npu-probe" | grep -E 'Class|Machine|Flags'
  arm-linux-gnueabi-readelf -d "$out/arm/lumatv-npu-probe" | grep NEEDED
fi
