// What OpenCL the device offers, read through its own ICD loader (LG ships the Khronos loader, /usr/lib/libOpenCL.so.1.2): the loader is
// opened with dlopen and the four query calls of OpenCL 1.0 are made through dlsym, so nothing of OpenCL is linked in and nothing is
// built or run on the GPU. The loader stays mapped RTLD_GLOBAL: TFLite's GPU delegate then finds the OpenCL entry points in the process
// even when no libOpenCL.so (the development name it tries) exists.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace probe {

struct ClDeviceInfo {
  std::string name, vendor, version, driver, c_version, extensions;
  uint64_t type = 0;  // CL_DEVICE_TYPE_*
  uint32_t compute_units = 0, clock_mhz = 0, image_support = 0;
  uint64_t global_mem = 0, max_alloc = 0, local_mem = 0, half_config = 0;
  uint64_t max_work_group = 0;
  bool Has(const char* extension) const;  // whole-word match in the extension list
};

struct ClPlatformInfo {
  std::string name, vendor, version, extensions;
  std::vector<ClDeviceInfo> devices;
  std::string device_error;
};

struct IcdFile {
  std::string path, content;  // content: the library name the file points at
  bool loads = false;         // that library can be opened (RTLD_LAZY, then closed)
  std::string load_error;
};

struct OpenClInfo {
  std::string loader;          // what was opened: the name tried, and the path it came from when known
  bool loaded = false;
  std::string error;           // why not: the dlerror of the last candidate, or the missing entry point
  std::string icd_filenames;   // OCL_ICD_FILENAMES as the loader will see it
  std::string icd_dir;         // the directory the loader reads .icd files from (OCL_ICD_VENDORS, or /etc/OpenCL/vendors)
  std::vector<IcdFile> icd_files;
  int platform_status = 0;     // clGetPlatformIDs result when it was not CL_SUCCESS
  std::vector<ClPlatformInfo> platforms;
};

// |loader| a path or name (empty: libOpenCL.so.1, libOpenCL.so, /usr/lib/libOpenCL.so.1); |icd_library| goes to OCL_ICD_FILENAMES (the
// loader then uses that driver library directly, without an .icd file; empty leaves the environment alone). Safe to call more than
// once: later calls reuse the opened loader.
OpenClInfo QueryOpenCl(const std::string& loader, const std::string& icd_library);

// "yes"/"no" for the features TFLite's GPU delegate and Arm's compute stack care about, one line.
std::string DescribeFeatures(const ClDeviceInfo& d);

// The report --gpu-info prints (also at the head of a --gpu run).
void PrintOpenClInfo(const OpenClInfo& info);

// True when a GPU or other accelerator device was found.
bool HasUsableDevice(const OpenClInfo& info);

}  // namespace probe
