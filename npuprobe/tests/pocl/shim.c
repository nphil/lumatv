/* Test-only: makes the CPU device of a CPU OpenCL implementation (PoCL) look like a GPU to TFLite's GPU delegate, which asks
 * clGetDeviceIDs for CL_DEVICE_TYPE_GPU and checks CL_DEVICE_TYPE. Preloaded (LD_PRELOAD), so the delegate's dlsym finds these first;
 * the real calls are the next definitions in the search order (the ICD loader). */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <stddef.h>
#include <stdint.h>

#define CL_DEVICE_TYPE_GPU 4ull
#define CL_DEVICE_TYPE_ALL 0xFFFFFFFFull
#define CL_DEVICE_TYPE 0x1000

int32_t clGetDeviceIDs(void* platform, uint64_t type, uint32_t n, void** devices, uint32_t* num) {
  static int32_t (*real)(void*, uint64_t, uint32_t, void**, uint32_t*);
  if (!real) real = dlsym(RTLD_NEXT, "clGetDeviceIDs");
  if (type & CL_DEVICE_TYPE_GPU) type = CL_DEVICE_TYPE_ALL;
  return real(platform, type, n, devices, num);
}

int32_t clGetDeviceInfo(void* device, uint32_t what, size_t capacity, void* value, size_t* size_ret) {
  static int32_t (*real)(void*, uint32_t, size_t, void*, size_t*);
  if (!real) real = dlsym(RTLD_NEXT, "clGetDeviceInfo");
  int32_t r = real(device, what, capacity, value, size_ret);
  if (what == CL_DEVICE_TYPE && r == 0 && value != NULL && capacity >= sizeof(uint64_t)) *(uint64_t*)value = CL_DEVICE_TYPE_GPU;
  return r;
}
