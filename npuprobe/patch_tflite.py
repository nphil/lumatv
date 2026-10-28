#!/usr/bin/env python3
"""Lets TensorFlow Lite's CMake build its NNAPI delegate on Linux. Idempotent.

The tree gates NNAPI on CMAKE_SYSTEM_NAME == Android, which swaps in nnapi_delegate_disabled.cc. webosose's
0007-enable-nnapi-delegate.patch (meta-webosose, tensorflow-lite recipe) drops the same three lines for LG's build.
usage: patch_tflite.py <tensorflow source root>
"""
import sys

path = sys.argv[1].rstrip("/") + "/tensorflow/lite/CMakeLists.txt"
gate = '''if(NOT "${CMAKE_SYSTEM_NAME}" STREQUAL "Android")
  set(_TFLITE_ENABLE_NNAPI OFF)
endif()
'''
text = open(path).read()
if gate in text:
    open(path, "w").write(text.replace(gate, "# NNAPI is built on Linux too (patch_tflite.py).\n"))
    print("patched", path)
elif "patch_tflite.py" in text:
    print("already patched", path)
else:
    sys.exit("unexpected tensorflow/lite/CMakeLists.txt: the NNAPI gate is not there")
