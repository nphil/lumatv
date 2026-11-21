#!/bin/sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
mkdir -p "$here/build/arm" "$here/build/host"
# The captured library has EF_ARM_ABI_FLOAT_SOFT (0x5000200), not hard-float.
zig cc -target arm-linux-gnueabi.2.28 -O2 -s -Wall -Wextra -Werror \
    "$here/runtime_probe.c" -ldl -o "$here/build/arm/lumatv-npu-probe"
zig cc -target x86_64-linux-gnu.2.36 -O2 -Wall -Wextra -Werror \
    "$here/runtime_probe.c" -ldl -o "$here/build/host/lumatv-npu-probe"
