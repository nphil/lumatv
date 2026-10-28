#include "opencl_info.h"

#include <dirent.h>
#include <dlfcn.h>
#include <sys/stat.h>

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <sstream>

namespace probe {

namespace {

// OpenCL 1.0 types and constants, spelled out so that no OpenCL header is needed (the values are the Khronos ones).
using cl_int = int32_t;
using cl_uint = uint32_t;
using cl_ulong = uint64_t;
using cl_platform_id = void*;
using cl_device_id = void*;

constexpr cl_int kClSuccess = 0;
constexpr cl_uint kPlatformVersion = 0x0901, kPlatformName = 0x0902, kPlatformVendor = 0x0903, kPlatformExtensions = 0x0904;
constexpr cl_ulong kDeviceTypeAll = 0xFFFFFFFFull;
constexpr cl_uint kDeviceType = 0x1000, kMaxComputeUnits = 0x1002, kMaxWorkGroupSize = 0x1004, kMaxClockFrequency = 0x100C,
                  kMaxMemAlloc = 0x1010, kImageSupport = 0x1016, kGlobalMemSize = 0x101F, kLocalMemSize = 0x1023, kName = 0x102B,
                  kVendor = 0x102C, kDriverVersion = 0x102D, kVersion = 0x102F, kExtensions = 0x1030, kHalfFpConfig = 0x1033,
                  kOpenClCVersion = 0x103D;

using GetPlatformIDs = cl_int (*)(cl_uint, cl_platform_id*, cl_uint*);
using GetPlatformInfo = cl_int (*)(cl_platform_id, cl_uint, size_t, void*, size_t*);
using GetDeviceIDs = cl_int (*)(cl_platform_id, cl_ulong, cl_uint, cl_device_id*, cl_uint*);
using GetDeviceInfo = cl_int (*)(cl_device_id, cl_uint, size_t, void*, size_t*);

struct Loaded {
  void* handle = nullptr;
  std::string name;
  std::string error;
};

Loaded& State() {
  static Loaded s;
  return s;
}

std::string ReadFirstLine(const std::string& path) {
  std::ifstream f(path);
  std::string line;
  if (f) std::getline(f, line);
  while (!line.empty() && (line.back() == '\r' || line.back() == ' ')) line.pop_back();
  return line;
}

std::string PlatformString(GetPlatformInfo fn, cl_platform_id p, cl_uint what) {
  size_t n = 0;
  if (fn(p, what, 0, nullptr, &n) != kClSuccess || n == 0) return "";
  std::string s(n, '\0');
  if (fn(p, what, n, &s[0], nullptr) != kClSuccess) return "";
  while (!s.empty() && s.back() == '\0') s.pop_back();
  return s;
}

std::string DeviceString(GetDeviceInfo fn, cl_device_id d, cl_uint what) {
  size_t n = 0;
  if (fn(d, what, 0, nullptr, &n) != kClSuccess || n == 0) return "";
  std::string s(n, '\0');
  if (fn(d, what, n, &s[0], nullptr) != kClSuccess) return "";
  while (!s.empty() && s.back() == '\0') s.pop_back();
  return s;
}

template <typename T>
T DeviceValue(GetDeviceInfo fn, cl_device_id d, cl_uint what) {
  T v = 0;
  if (fn(d, what, sizeof v, &v, nullptr) != kClSuccess) return 0;
  return v;
}

void ListIcdFiles(const std::string& dir, std::vector<IcdFile>* out) {
  DIR* d = opendir(dir.c_str());
  if (d == nullptr) return;
  std::vector<std::string> names;
  while (const dirent* e = readdir(d)) {
    const std::string n = e->d_name;
    if (n.size() > 4 && n.compare(n.size() - 4, 4, ".icd") == 0) names.push_back(n);
  }
  closedir(d);
  std::sort(names.begin(), names.end());
  for (const std::string& n : names) {
    IcdFile f;
    f.path = dir + "/" + n;
    f.content = ReadFirstLine(f.path);
    if (!f.content.empty()) {
      void* h = dlopen(f.content.c_str(), RTLD_LAZY | RTLD_LOCAL);
      f.loads = h != nullptr;
      if (h == nullptr) {
        const char* why = dlerror();
        f.load_error = why != nullptr ? why : "dlopen failed";
      } else {
        dlclose(h);
      }
    }
    out->push_back(f);
  }
}

}  // namespace

bool ClDeviceInfo::Has(const char* extension) const {
  const size_t n = std::strlen(extension);
  for (size_t pos = extensions.find(extension); pos != std::string::npos; pos = extensions.find(extension, pos + 1)) {
    const bool start = pos == 0 || extensions[pos - 1] == ' ';
    const bool end = pos + n == extensions.size() || extensions[pos + n] == ' ';
    if (start && end) return true;
  }
  return false;
}

OpenClInfo QueryOpenCl(const std::string& loader, const std::string& icd_library) {
  OpenClInfo info;
  if (!icd_library.empty()) setenv("OCL_ICD_FILENAMES", icd_library.c_str(), 1);  // read when the loader first enumerates its drivers
  if (const char* e = getenv("OCL_ICD_FILENAMES")) info.icd_filenames = e;
  const char* vendors = getenv("OCL_ICD_VENDORS");
  if (vendors == nullptr) vendors = getenv("OPENCL_VENDOR_PATH");
  info.icd_dir = vendors != nullptr ? vendors : "/etc/OpenCL/vendors";
  struct stat st;
  if (stat(info.icd_dir.c_str(), &st) == 0 && S_ISDIR(st.st_mode)) ListIcdFiles(info.icd_dir, &info.icd_files);

  Loaded& s = State();
  if (s.handle == nullptr) {
    std::vector<std::string> names;
    if (!loader.empty()) names.push_back(loader);
    else names = {"libOpenCL.so.1", "libOpenCL.so", "/usr/lib/libOpenCL.so.1", "/usr/lib/libOpenCL.so.1.2"};
    for (const std::string& n : names) {
      // RTLD_GLOBAL: TFLite's own dlopen("libOpenCL.so") may find nothing on a device without the development symlink, and then
      // looks the entry points up in the process (RTLD_DEFAULT), which finds them here.
      s.handle = dlopen(n.c_str(), RTLD_NOW | RTLD_GLOBAL);
      if (s.handle != nullptr) {
        s.name = n;
        s.error.clear();
        break;
      }
      const char* why = dlerror();
      s.error = why != nullptr ? why : "dlopen failed";
    }
  }
  if (s.handle == nullptr) {
    info.error = s.error;
    return info;
  }
  info.loader = s.name;
  auto get_platforms = reinterpret_cast<GetPlatformIDs>(dlsym(s.handle, "clGetPlatformIDs"));
  auto platform_info = reinterpret_cast<GetPlatformInfo>(dlsym(s.handle, "clGetPlatformInfo"));
  auto get_devices = reinterpret_cast<GetDeviceIDs>(dlsym(s.handle, "clGetDeviceIDs"));
  auto device_info = reinterpret_cast<GetDeviceInfo>(dlsym(s.handle, "clGetDeviceInfo"));
  if (get_platforms == nullptr || platform_info == nullptr || get_devices == nullptr || device_info == nullptr) {
    info.error = s.name + " does not export clGetPlatformIDs, clGetPlatformInfo, clGetDeviceIDs and clGetDeviceInfo";
    return info;
  }
  info.loaded = true;

  cl_uint count = 0;
  cl_int st_code = get_platforms(0, nullptr, &count);
  if (st_code != kClSuccess || count == 0) {
    info.platform_status = st_code;  // CL_PLATFORM_NOT_FOUND_KHR is -1001: the loader found no driver
    return info;
  }
  std::vector<cl_platform_id> ids(count);
  if (get_platforms(count, ids.data(), nullptr) != kClSuccess) {
    info.platform_status = -1;
    return info;
  }
  for (cl_platform_id p : ids) {
    ClPlatformInfo pi;
    pi.name = PlatformString(platform_info, p, kPlatformName);
    pi.vendor = PlatformString(platform_info, p, kPlatformVendor);
    pi.version = PlatformString(platform_info, p, kPlatformVersion);
    pi.extensions = PlatformString(platform_info, p, kPlatformExtensions);
    cl_uint nd = 0;
    cl_int rc = get_devices(p, kDeviceTypeAll, 0, nullptr, &nd);
    if (rc != kClSuccess || nd == 0) {
      pi.device_error = "clGetDeviceIDs returned " + std::to_string(rc) + (nd == 0 ? ", no device" : "");
    } else {
      std::vector<cl_device_id> devs(nd);
      if (get_devices(p, kDeviceTypeAll, nd, devs.data(), nullptr) == kClSuccess) {
        for (cl_device_id d : devs) {
          ClDeviceInfo di;
          di.name = DeviceString(device_info, d, kName);
          di.vendor = DeviceString(device_info, d, kVendor);
          di.version = DeviceString(device_info, d, kVersion);
          di.driver = DeviceString(device_info, d, kDriverVersion);
          di.c_version = DeviceString(device_info, d, kOpenClCVersion);
          di.extensions = DeviceString(device_info, d, kExtensions);
          di.type = DeviceValue<cl_ulong>(device_info, d, kDeviceType);
          di.compute_units = DeviceValue<cl_uint>(device_info, d, kMaxComputeUnits);
          di.clock_mhz = DeviceValue<cl_uint>(device_info, d, kMaxClockFrequency);
          di.image_support = DeviceValue<cl_uint>(device_info, d, kImageSupport);
          di.global_mem = DeviceValue<cl_ulong>(device_info, d, kGlobalMemSize);
          di.max_alloc = DeviceValue<cl_ulong>(device_info, d, kMaxMemAlloc);
          di.local_mem = DeviceValue<cl_ulong>(device_info, d, kLocalMemSize);
          di.half_config = DeviceValue<cl_ulong>(device_info, d, kHalfFpConfig);
          di.max_work_group = DeviceValue<size_t>(device_info, d, kMaxWorkGroupSize);
          pi.devices.push_back(di);
        }
      }
    }
    info.platforms.push_back(pi);
  }
  return info;
}

std::string DescribeFeatures(const ClDeviceInfo& d) {
  auto yn = [](bool b) { return b ? "yes" : "no"; };
  std::string s;
  s += std::string("fp16 arithmetic ") + yn(d.Has("cl_khr_fp16"));
  s += std::string(", images ") + yn(d.image_support != 0);
  s += std::string(", int8 dot product ") + yn(d.Has("cl_arm_integer_dot_product_int8") || d.Has("cl_khr_integer_dot_product"));
  s += std::string(", priority hints ") + yn(d.Has("cl_khr_priority_hints"));
  s += std::string(", Arm memory import ") + yn(d.Has("cl_arm_import_memory"));
  s += std::string(", subgroups ") + yn(d.Has("cl_khr_subgroups"));
  return s;
}

bool HasUsableDevice(const OpenClInfo& info) {
  for (const ClPlatformInfo& p : info.platforms)
    if (!p.devices.empty()) return true;
  return false;
}

void PrintOpenClInfo(const OpenClInfo& info) {
  std::printf("OpenCL: ICD loader %s\n", info.loaded ? info.loader.c_str() : ("not loaded (" + info.error + ")").c_str());
  std::printf("  ICD files in %s: %s\n", info.icd_dir.c_str(), info.icd_files.empty() ? "none" : "");
  for (const IcdFile& f : info.icd_files) {
    std::printf("    %s -> %s%s\n", f.path.c_str(), f.content.empty() ? "(empty)" : f.content.c_str(),
                f.content.empty() ? "" : f.loads ? " (that library loads)" : (" (does not load: " + f.load_error + ")").c_str());
  }
  if (!info.icd_filenames.empty()) std::printf("  OCL_ICD_FILENAMES=%s\n", info.icd_filenames.c_str());
  if (!info.loaded) return;
  if (info.platforms.empty()) {
    std::printf("  no OpenCL platform (clGetPlatformIDs %d%s): the loader found no vendor driver\n", info.platform_status,
                info.platform_status == -1001 ? ", CL_PLATFORM_NOT_FOUND_KHR" : "");
    return;
  }
  for (size_t i = 0; i < info.platforms.size(); ++i) {
    const ClPlatformInfo& p = info.platforms[i];
    std::printf("  platform [%zu] %s, %s, %s\n", i, p.name.c_str(), p.vendor.c_str(), p.version.c_str());
    if (!p.device_error.empty()) std::printf("    no devices: %s\n", p.device_error.c_str());
    for (size_t j = 0; j < p.devices.size(); ++j) {
      const ClDeviceInfo& d = p.devices[j];
      const char* type = (d.type & 4) ? "GPU" : (d.type & 2) ? "CPU" : (d.type & 8) ? "accelerator" : "other";
      std::printf("    device [%zu.%zu] %s (%s), %s, %s, driver %s\n", i, j, d.name.c_str(), type, d.vendor.c_str(), d.version.c_str(),
                  d.driver.c_str());
      std::printf("      %u compute units at up to %u MHz, global memory %llu MiB (largest object %llu MiB), local memory %llu KiB, work group %llu\n",
                  d.compute_units, d.clock_mhz, static_cast<unsigned long long>(d.global_mem >> 20),
                  static_cast<unsigned long long>(d.max_alloc >> 20), static_cast<unsigned long long>(d.local_mem >> 10),
                  static_cast<unsigned long long>(d.max_work_group));
      std::printf("      %s\n", DescribeFeatures(d).c_str());
      std::printf("      extensions: %s\n", d.extensions.c_str());
    }
  }
}

}  // namespace probe
