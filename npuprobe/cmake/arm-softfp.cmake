# 32-bit ARM, soft-float calling convention with NEON/VFP code: the TV's userland ABI (e_flags 0x5000200,
# interpreter /lib/ld-linux.so.3). GCC 11 and glibc 2.35: the TV's libraries ask for GLIBCXX_3.4.29 (GCC 11) and GLIBC_2.34,
# and a newer toolchain would pull in symbols the TV's libstdc++.so.6 and libc.so.6 do not have (GLIBCXX_3.4.30, arc4random).
# Compiler and sysroot come from toolchain.sh (CROSS_ROOT, CROSS_CC_VER).
set(CMAKE_SYSTEM_NAME Linux)
# XNNPACK selects its ARM32 kernels on "armv7", not "arm".
set(CMAKE_SYSTEM_PROCESSOR armv7)
set(CMAKE_C_COMPILER arm-linux-gnueabi-gcc-$ENV{CROSS_CC_VER})
set(CMAKE_CXX_COMPILER arm-linux-gnueabi-g++-$ENV{CROSS_CC_VER})
set(CMAKE_SYSROOT $ENV{CROSS_ROOT})
set(_arm "-march=armv7-a -mfpu=neon-vfpv4 -mfloat-abi=softfp -flax-vector-conversions")
set(CMAKE_C_FLAGS_INIT "${_arm}")
set(CMAKE_CXX_FLAGS_INIT "${_arm}")
# XNNPACK's .S kernels see CMAKE_ASM_FLAGS only: without the float ABI the assembler turns the FPU off and refuses vsdot.
set(CMAKE_ASM_FLAGS_INIT "${_arm}")
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
