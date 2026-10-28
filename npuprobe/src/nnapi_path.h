// The NNAPI route to an accelerator: TFLite's NNAPI delegate over the device's libneuralnetworks.so (the Android NN runtime,
// webosose's "webos-nnruntime"), whose drivers are NN HALs such as LG's libwebos_nnhal.so (device "lgnpu").
#pragma once

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

#include "tensorflow/lite/delegates/nnapi/nnapi_delegate.h"
#include "tensorflow/lite/nnapi/nnapi_implementation.h"

namespace probe {

struct NnapiDevice {
  std::string name, version;
  int type = 0;  // ANEURALNETWORKS_DEVICE_*
  int64_t feature_level = -1;
};

struct NnapiInfo {
  bool library = false;  // libneuralnetworks.so was loaded by TFLite's loader
  int tflite_sdk_level = 0;  // android_sdk_version as TFLite derives it off Android from the functions the runtime exports (27 = NNAPI 1.0 ... 31)
  int64_t runtime_feature_level = 0;
  std::vector<NnapiDevice> devices;
  std::string error;
};

// Loads libneuralnetworks.so (first call per process, through TFLite's own loader) and lists its devices.
NnapiInfo QueryNnapi();
const char* NnapiDeviceType(int type);

struct NnapiOptions {
  std::string accelerator;  // device name; empty lets NNAPI choose
  int sdk_level = 0;        // the Android level TFLite is told it runs on; 0 keeps the one it derives from the runtime's exports
  std::string preference;   // "", low, fast, sustained
  int partitions = 3;       // max_number_delegated_partitions
  bool allow_cpu = false;   // keep NNAPI's CPU reference ("nnapi-reference") among the candidates
  bool allow_fp16 = false;
};

// "key=value" with keys accelerator, sdk, pref, partitions, allow_cpu, allow_fp16.
bool ParseNnapiOption(const std::string& kv, NnapiOptions* o, std::string* error);
std::string DescribeNnapiOptions(const NnapiOptions& o);

class NnapiDelegate {
 public:
  bool Create(const NnapiOptions& o, std::string* error);
  TfLiteDelegate* get() { return delegate_.get(); }
  int Errno() const { return delegate_ ? delegate_->GetNnApiErrno() : 0; }

 private:
  NnApi nnapi_{};  // a copy of TFLite's loaded entry points with the SDK level set; must outlive the delegate
  std::unique_ptr<tflite::StatefulNnApiDelegate> delegate_;
};

}  // namespace probe
