#!/bin/sh
# Runs lumatv-npu-probe's --gpu route against a real OpenCL implementation on the build host: PoCL (the portable CPU OpenCL), unpacked from
# Debian 12's packages under $POCL_DIR, nothing installed. TFLite's GPU delegate asks for a GPU-type device and PoCL has a CPU one, so
# shim.c (preloaded) answers the two device queries as if the device were a GPU. The times are those of a CPU: this checks that the
# delegate's options, kernel cache, coverage and outputs work end to end, not how fast the Mali is.
#   tests/pocl/run.sh [host-probe args...]      e.g. --model /tmp/lumatv-npu-models/efficientdet_lite0_320_ptq.tflite --runs 5
# Not part of tests/run_tests.sh: it needs ~60 MB of downloads and minutes of kernel compilation.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tools=${NPU_TOOLS:-/tmp/lumatv-npu-tools}
. "$tools/env.sh"
dir=${POCL_DIR:-/tmp/lumatv-npu-pocl}
probe=${PROBE:-$here/../../build/host/lumatv-npu-probe}
mirror=http://deb.debian.org/debian
if [ ! -d "$dir/root/etc/OpenCL/vendors" ]; then
  mkdir -p "$dir/debs" "$dir/root"
  curl -fsSL -m 120 -o "$dir/Packages.xz" "$mirror/dists/bookworm/main/binary-amd64/Packages.xz"
  for p in pocl-opencl-icd libpocl2 libpocl2-common libclang-cpp15 libclang1-15 libclang-common-15-dev libhwloc15 clang-15 llvm-15-linker-tools; do
    f=$(python3 -c 'import lzma, sys; sys.stdout.write(lzma.open(sys.argv[1], "rt").read())' "$dir/Packages.xz" | awk -v p="$p" '$1 == "Package:" { hit = ($2 == p) } hit && $1 == "Filename:" { print $2; exit }')
    [ -n "$f" ] || { echo "no $p in the package index" >&2; exit 1; }
    [ -f "$dir/debs/$(basename "$f")" ] || curl -fsSL -m 300 -o "$dir/debs/$(basename "$f")" "$mirror/$f"
    dpkg-deb -x "$dir/debs/$(basename "$f")" "$dir/root"
  done
fi
# the device-type shim
[ -f "$dir/libshim.so" ] || gcc-12 --sysroot="$HOST_ROOT" -shared -fPIC -o "$dir/libshim.so" "$here/shim.c" -ldl
# PoCL links each compiled kernel with `ld`: one that finds the C library and libgcc in the toolchain's root
mkdir -p "$dir/bin"
cat > "$dir/bin/ld" <<EOF
#!/bin/sh
exec $HOST_ROOT/usr/bin/ld --sysroot=$HOST_ROOT -L$HOST_ROOT/usr/lib/gcc/x86_64-linux-gnu/12 -L$HOST_ROOT/usr/lib/x86_64-linux-gnu -L$HOST_ROOT/lib/x86_64-linux-gnu "\$@"
EOF
chmod +x "$dir/bin/ld"
export OCL_ICD_VENDORS=$dir/root/etc/OpenCL/vendors
export LD_LIBRARY_PATH=$dir/root/usr/lib/x86_64-linux-gnu:$HOST_ROOT/usr/lib/x86_64-linux-gnu
export PATH=$dir/bin:$PATH
exec env LD_PRELOAD="$dir/libshim.so" "$probe" "$@" --gpu
