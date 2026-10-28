# x86-64 host build for the stub-delegate tests; compiler and sysroot come from toolchain.sh (HOST_ROOT).
set(CMAKE_C_COMPILER gcc-12)
set(CMAKE_CXX_COMPILER g++-12)
set(CMAKE_SYSROOT $ENV{HOST_ROOT})
