// A stand-in OpenCL ICD loader for host tests (--opencl-lib): the four query calls --gpu-info makes (OpenCL 1.0), answering with one
// platform and one GPU device shaped like the TV's Mali-G52. Nothing can be run on it.
//
//   STUB_OPENCL_NONE=1     clGetPlatformIDs fails with CL_PLATFORM_NOT_FOUND_KHR (-1001), as the loader does without a vendor driver
//   STUB_OPENCL_NO_FP16=1  leave cl_khr_fp16 out of the extension list
// The platform name carries OCL_ICD_FILENAMES as the process had it when the platform was asked for, so a test can see that --opencl-icd
// reached the environment before the loader was used.
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define EXPORT extern "C" __attribute__((visibility("default")))

namespace {

constexpr int32_t kSuccess = 0, kInvalidValue = -30, kPlatformNotFound = -1001;
int g_platform_token, g_device_token;  // the handles are the addresses of these

int32_t CopyOut(const void* data, size_t size, size_t capacity, void* value, size_t* size_ret) {
  if (size_ret != nullptr) *size_ret = size;
  if (value != nullptr) {
    if (capacity < size) return kInvalidValue;
    memcpy(value, data, size);
  }
  return kSuccess;
}

int32_t CopyString(const char* s, size_t capacity, void* value, size_t* size_ret) { return CopyOut(s, strlen(s) + 1, capacity, value, size_ret); }

template <typename T>
int32_t CopyValue(T v, size_t capacity, void* value, size_t* size_ret) {
  return CopyOut(&v, sizeof v, capacity, value, size_ret);
}

}  // namespace

EXPORT int32_t clGetPlatformIDs(uint32_t num_entries, void** platforms, uint32_t* num_platforms) {
  if (getenv("STUB_OPENCL_NONE") != nullptr) {
    if (num_platforms != nullptr) *num_platforms = 0;
    return kPlatformNotFound;
  }
  if (num_platforms != nullptr) *num_platforms = 1;
  if (platforms != nullptr && num_entries >= 1) platforms[0] = &g_platform_token;
  return kSuccess;
}

EXPORT int32_t clGetPlatformInfo(void* platform, uint32_t what, size_t capacity, void* value, size_t* size_ret) {
  if (platform != &g_platform_token) return -32;  // CL_INVALID_PLATFORM
  char buf[512];
  switch (what) {
    case 0x0902: {  // CL_PLATFORM_NAME
      const char* icd = getenv("OCL_ICD_FILENAMES");
      snprintf(buf, sizeof buf, "Stub ARM Platform [OCL_ICD_FILENAMES=%s]", icd != nullptr ? icd : "");
      return CopyString(buf, capacity, value, size_ret);
    }
    case 0x0903: return CopyString("Stub ARM", capacity, value, size_ret);  // vendor
    case 0x0901: return CopyString("OpenCL 2.0 stub", capacity, value, size_ret);  // version
    case 0x0904: return CopyString("cl_khr_icd", capacity, value, size_ret);  // extensions
    default: return kInvalidValue;
  }
}

EXPORT int32_t clGetDeviceIDs(void* platform, uint64_t type, uint32_t num_entries, void** devices, uint32_t* num_devices) {
  if (platform != &g_platform_token) return -32;
  if ((type & (4 | 0xFFFFFFFFull)) == 0) return -1;  // CL_DEVICE_NOT_FOUND
  if (num_devices != nullptr) *num_devices = 1;
  if (devices != nullptr && num_entries >= 1) devices[0] = &g_device_token;
  return kSuccess;
}

EXPORT int32_t clGetDeviceInfo(void* device, uint32_t what, size_t capacity, void* value, size_t* size_ret) {
  if (device != &g_device_token) return -33;  // CL_INVALID_DEVICE
  switch (what) {
    case 0x1000: return CopyValue<uint64_t>(4, capacity, value, size_ret);               // CL_DEVICE_TYPE: GPU
    case 0x1002: return CopyValue<uint32_t>(3, capacity, value, size_ret);               // MAX_COMPUTE_UNITS
    case 0x1004: return CopyValue<size_t>(384, capacity, value, size_ret);               // MAX_WORK_GROUP_SIZE
    case 0x100C: return CopyValue<uint32_t>(800, capacity, value, size_ret);             // MAX_CLOCK_FREQUENCY
    case 0x1010: return CopyValue<uint64_t>(512ull << 20, capacity, value, size_ret);    // MAX_MEM_ALLOC_SIZE
    case 0x1016: return CopyValue<uint32_t>(1, capacity, value, size_ret);               // IMAGE_SUPPORT
    case 0x101F: return CopyValue<uint64_t>(1024ull << 20, capacity, value, size_ret);   // GLOBAL_MEM_SIZE
    case 0x1023: return CopyValue<uint64_t>(32ull << 10, capacity, value, size_ret);     // LOCAL_MEM_SIZE
    case 0x1033: return CopyValue<uint64_t>(0x3f, capacity, value, size_ret);            // HALF_FP_CONFIG
    case 0x102B: return CopyString("Stub GPU (Mali-G52 shaped)", capacity, value, size_ret);
    case 0x102C: return CopyString("Stub ARM", capacity, value, size_ret);
    case 0x102D: return CopyString("stub-1.0", capacity, value, size_ret);
    case 0x102F: return CopyString("OpenCL 2.0 stub", capacity, value, size_ret);
    case 0x103D: return CopyString("OpenCL C 2.0", capacity, value, size_ret);
    case 0x1030:
      return CopyString(getenv("STUB_OPENCL_NO_FP16") != nullptr
                            ? "cl_khr_global_int32_base_atomics cl_arm_integer_dot_product_int8 cl_arm_import_memory"
                            : "cl_khr_fp16 cl_khr_priority_hints cl_arm_integer_dot_product_int8 cl_arm_import_memory",
                        capacity, value, size_ret);
    default: return kInvalidValue;
  }
}
