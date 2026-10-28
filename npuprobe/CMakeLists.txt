cmake_minimum_required(VERSION 3.16)
project(lumatv_npu_probe C CXX)

# TensorFlow Lite v2.13.x: LG's libtflite-npu-delegate.so.1 hands ReplaceNodeSubsetsWithDelegateKernels a TfLiteRegistration
# of ten fields by value (tests/lg/run.sh shows it), the layout of 2.13; 2.9 to 2.12 have eight or nine, 2.14 and later
# eleven, and with a different size the call reads its next argument from the wrong place and crashes.
set(TFLITE_SRC "" CACHE PATH "TensorFlow v2.13.1 source tree (tensorflow/lite inside)")
set(STB_DIR "" CACHE PATH "directory holding stb_image.h (third_party/fetch.sh)")
if(NOT TFLITE_SRC OR NOT STB_DIR)
  message(FATAL_ERROR "set -DTFLITE_SRC=<tensorflow-2.13.1> and -DSTB_DIR=<dir>; build.sh does both")
endif()

set(TFLITE_ENABLE_XNNPACK ON CACHE BOOL "")
set(TFLITE_ENABLE_NNAPI ON CACHE BOOL "" FORCE)  # built on Linux by patch_tflite.py; the device's libneuralnetworks.so is dlopen'ed
set(TFLITE_ENABLE_GPU OFF CACHE BOOL "")
set(TFLITE_ENABLE_EXTERNAL_DELEGATE ON CACHE BOOL "")
add_subdirectory("${TFLITE_SRC}/tensorflow/lite" "${CMAKE_CURRENT_BINARY_DIR}/tensorflow-lite" EXCLUDE_FROM_ALL)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
add_compile_options(-Wall -Wextra)

# Delegates are shared libraries that leave TfLiteDelegateCreate, TfLiteIntArray* and
# tflite::Subgraph::SetCustomAllocationForTensor undefined (libtflite-npu-delegate.so has no DT_NEEDED on the
# runtime); the probe exports its static TFLite so they bind to it, as they bind to libtensorflow-lite.so on the TV.
add_executable(lumatv-npu-probe src/probe.cc src/delegate_loader.cc src/detect.cc src/image.cc src/nnapi_path.cc src/sysinfo.cc)
target_include_directories(lumatv-npu-probe PRIVATE src)
target_include_directories(lumatv-npu-probe SYSTEM PRIVATE "${TFLITE_SRC}" "${STB_DIR}")
target_link_libraries(lumatv-npu-probe PRIVATE tensorflow-lite ${CMAKE_DL_LIBS} pthread)
target_link_options(lumatv-npu-probe PRIVATE -rdynamic)
if(CMAKE_CROSSCOMPILING)
  # The TV's libstdc++.so.6 stops at GLIBCXX_3.4.29 (GCC 11: what its libraries need); the cross sysroot's runtime is
  # newer and would stamp the binary with GLIBCXX_3.4.30 (std::condition_variable::wait). The GCC 11 static library
  # has no such version, and --exclude-libs keeps its symbols out of the export table, so a delegate loaded later binds
  # to the TV's own libstdc++.so.6.
  target_link_options(lumatv-npu-probe PRIVATE -static-libstdc++ -Wl,--exclude-libs,libstdc++.a:libsupc++.a)
endif()

# Test fixtures: a stand-in delegate with LG's plugin surface, and the small models it is exercised on.
add_library(stub-npu-delegate SHARED stub/stub_delegate.cc)
target_include_directories(stub-npu-delegate SYSTEM PRIVATE "${TFLITE_SRC}" $<TARGET_PROPERTY:flatbuffers,INTERFACE_INCLUDE_DIRECTORIES>)
target_compile_options(stub-npu-delegate PRIVATE -fvisibility=hidden)
# TFLite's own symbols come from the executable; nothing may be linked in here.
target_link_options(stub-npu-delegate PRIVATE -Wl,--allow-shlib-undefined)

# Stand-ins for the Android NN runtime the NNAPI route loads (libneuralnetworks.so), one per directory so that LD_LIBRARY_PATH picks it:
# nnapi-stub/ is a 1.3 runtime, nnapi-stub-12/ an Android 10 one without ANeuralNetworksDevice_getFeatureLevel.
foreach(variant "" "-12")
  add_library(stub-neuralnetworks${variant} SHARED stub/stub_nnapi.cc)
  target_include_directories(stub-neuralnetworks${variant} SYSTEM PRIVATE "${TFLITE_SRC}")
  target_compile_options(stub-neuralnetworks${variant} PRIVATE -fvisibility=hidden)
  set_target_properties(stub-neuralnetworks${variant} PROPERTIES OUTPUT_NAME neuralnetworks
                        LIBRARY_OUTPUT_DIRECTORY "${CMAKE_CURRENT_BINARY_DIR}/nnapi-stub${variant}")
endforeach()
target_compile_definitions(stub-neuralnetworks-12 PRIVATE STUB_NNAPI_12_ONLY)

# A shared library without the plugin entry points, for the loader's error path.
add_library(not-a-delegate SHARED stub/not_a_delegate.c)

add_executable(make-test-models tests/make_models.cc)
target_include_directories(make-test-models SYSTEM PRIVATE "${TFLITE_SRC}")
target_link_libraries(make-test-models PRIVATE tensorflow-lite)
