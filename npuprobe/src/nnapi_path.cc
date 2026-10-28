#include "nnapi_path.h"

#include <cstdlib>

namespace probe {

NnapiInfo QueryNnapi() {
  NnapiInfo info;
  const NnApi* nn = NnApiImplementation();
  info.library = nn->nnapi_exists;
  info.tflite_sdk_level = nn->android_sdk_version;
  info.runtime_feature_level = nn->nnapi_runtime_feature_level;
  if (!info.library) {
    info.error = "libneuralnetworks.so is not loadable";
    return info;
  }
  if (nn->ANeuralNetworks_getDeviceCount == nullptr || nn->ANeuralNetworks_getDevice == nullptr) {
    info.error = "the runtime has no device list (older than NNAPI 1.2)";
    return info;
  }
  uint32_t count = 0;
  if (nn->ANeuralNetworks_getDeviceCount(&count) != 0) {
    info.error = "ANeuralNetworks_getDeviceCount failed";
    return info;
  }
  for (uint32_t i = 0; i < count; ++i) {
    ANeuralNetworksDevice* device = nullptr;
    if (nn->ANeuralNetworks_getDevice(i, &device) != 0 || device == nullptr) {
      info.error = "ANeuralNetworks_getDevice(" + std::to_string(i) + ") failed";
      return info;
    }
    NnapiDevice d;
    const char* s = nullptr;
    if (nn->ANeuralNetworksDevice_getName != nullptr && nn->ANeuralNetworksDevice_getName(device, &s) == 0 && s != nullptr) d.name = s;
    s = nullptr;
    if (nn->ANeuralNetworksDevice_getVersion != nullptr && nn->ANeuralNetworksDevice_getVersion(device, &s) == 0 && s != nullptr) d.version = s;
    int32_t type = 0;
    if (nn->ANeuralNetworksDevice_getType != nullptr && nn->ANeuralNetworksDevice_getType(device, &type) == 0) d.type = type;
    int64_t level = -1;
    if (nn->ANeuralNetworksDevice_getFeatureLevel != nullptr && nn->ANeuralNetworksDevice_getFeatureLevel(device, &level) == 0)
      d.feature_level = level;
    info.devices.push_back(d);
  }
  return info;
}

const char* NnapiDeviceType(int type) {
  switch (type) {
    case ANEURALNETWORKS_DEVICE_OTHER: return "other";
    case ANEURALNETWORKS_DEVICE_CPU: return "cpu";
    case ANEURALNETWORKS_DEVICE_GPU: return "gpu";
    case ANEURALNETWORKS_DEVICE_ACCELERATOR: return "accelerator";
    default: return "unknown";
  }
}

bool ParseNnapiOption(const std::string& kv, NnapiOptions* o, std::string* error) {
  const size_t eq = kv.find('=');
  if (eq == std::string::npos || eq == 0) {
    *error = "--nnapi-opt wants KEY=VALUE, got '" + kv + "'";
    return false;
  }
  const std::string key = kv.substr(0, eq), val = kv.substr(eq + 1);
  auto number = [&](int* out) {
    char* end = nullptr;
    const long v = std::strtol(val.c_str(), &end, 10);
    if (val.empty() || *end != '\0' || v < 0 || v > 100000) {
      *error = key + " is not a whole number: '" + val + "'";
      return false;
    }
    *out = static_cast<int>(v);
    return true;
  };
  auto flag = [&](bool* out) {
    if (val == "true" || val == "1") *out = true;
    else if (val == "false" || val == "0") *out = false;
    else {
      *error = key + " is true or false, got '" + val + "'";
      return false;
    }
    return true;
  };
  if (key == "accelerator") o->accelerator = val;
  else if (key == "sdk") return number(&o->sdk_level);
  else if (key == "partitions") return number(&o->partitions);
  else if (key == "allow_cpu") return flag(&o->allow_cpu);
  else if (key == "allow_fp16") return flag(&o->allow_fp16);
  else if (key == "pref") {
    if (val != "low" && val != "fast" && val != "sustained" && !val.empty()) {
      *error = "pref is low, fast or sustained, got '" + val + "'";
      return false;
    }
    o->preference = val;
  } else {
    *error = "unknown --nnapi-opt key '" + key + "' (accelerator, sdk, pref, partitions, allow_cpu, allow_fp16)";
    return false;
  }
  return true;
}

std::string DescribeNnapiOptions(const NnapiOptions& o) {
  std::string s = "accelerator=" + (o.accelerator.empty() ? std::string("(any)") : o.accelerator) + " sdk=" + (o.sdk_level == 0 ? std::string("auto") : std::to_string(o.sdk_level)) +
                  " pref=" + (o.preference.empty() ? std::string("default") : o.preference) + " partitions=" + std::to_string(o.partitions);
  if (o.allow_cpu) s += " allow_cpu";
  if (o.allow_fp16) s += " allow_fp16";
  return s;
}

namespace {
int64_t g_assumed_feature_level = 0;

// What the delegate asks a device that cannot say (a runtime older than NNAPI 1.3).
int AssumedFeatureLevel(const ANeuralNetworksDevice*, int64_t* level) {
  *level = g_assumed_feature_level;
  return 0;
}
}  // namespace

bool NnapiDelegate::Create(const NnapiOptions& o, std::string* error) {
  const NnApi* loaded = NnApiImplementation();
  if (!loaded->nnapi_exists) {
    *error = "libneuralnetworks.so is not loadable: there is no NNAPI runtime to delegate to";
    return false;
  }
  // Off Android TFLite works out android_sdk_version from the functions the runtime exports. StatefulNnApiDelegate takes the
  // entry-point table as an argument, so a copy can state another level (--nnapi-opt sdk=N), e.g. to stay below NNAPI 1.2's device selection.
  nnapi_ = *loaded;
  if (o.sdk_level > 0) {
    nnapi_.android_sdk_version = o.sdk_level;
    if (nnapi_.nnapi_runtime_feature_level < o.sdk_level) nnapi_.nnapi_runtime_feature_level = o.sdk_level;
  }
  const int level = nnapi_.android_sdk_version;
  if (level >= 29 && (nnapi_.ANeuralNetworks_getDeviceCount == nullptr || nnapi_.ANeuralNetworks_getDevice == nullptr ||
                      nnapi_.ANeuralNetworksDevice_getName == nullptr || nnapi_.ANeuralNetworksCompilation_createForDevices == nullptr ||
                      nnapi_.ANeuralNetworksModel_getSupportedOperationsForDevices == nullptr)) {
    *error = "the runtime lacks NNAPI 1.2 device functions, so TFLite cannot target a device at level " + std::to_string(level) +
             " (--nnapi-opt sdk=28 uses the runtime without device selection)";
    return false;
  }
  // TFLite asks every selected device for its feature level without checking that the runtime has the call (NNAPI 1.3).
  g_assumed_feature_level = level;
  if (nnapi_.ANeuralNetworksDevice_getFeatureLevel == nullptr) nnapi_.ANeuralNetworksDevice_getFeatureLevel = &AssumedFeatureLevel;

  tflite::StatefulNnApiDelegate::Options opt;
  if (!o.accelerator.empty()) opt.accelerator_name = o.accelerator.c_str();  // the delegate copies it
  opt.disallow_nnapi_cpu = !o.allow_cpu;
  opt.max_number_delegated_partitions = o.partitions;
  opt.allow_fp16 = o.allow_fp16;
  if (o.preference == "low") opt.execution_preference = tflite::StatefulNnApiDelegate::Options::kLowPower;
  else if (o.preference == "fast") opt.execution_preference = tflite::StatefulNnApiDelegate::Options::kFastSingleAnswer;
  else if (o.preference == "sustained") opt.execution_preference = tflite::StatefulNnApiDelegate::Options::kSustainedSpeed;
  delegate_ = std::make_unique<tflite::StatefulNnApiDelegate>(&nnapi_, opt);
  return true;
}

}  // namespace probe
