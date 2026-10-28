// Loader for TFLite "external delegate" plugins: shared libraries exporting
//   TfLiteDelegate* tflite_plugin_create_delegate(char** keys, char** values, size_t n, void (*report_error)(const char*));
//   void tflite_plugin_destroy_delegate(TfLiteDelegate*);
// (tensorflow/lite/delegates/external/external_delegate.h). LG's /usr/lib/libtflite-npu-delegate.so.1 exports both.
#pragma once

#include <string>
#include <utility>
#include <vector>

#include "tensorflow/lite/c/common.h"

namespace probe {

using DelegateOptions = std::vector<std::pair<std::string, std::string>>;

class ExternalDelegate {
 public:
  ExternalDelegate() = default;
  ExternalDelegate(const ExternalDelegate&) = delete;
  ExternalDelegate& operator=(const ExternalDelegate&) = delete;
  // Destroys the delegate; the library stays mapped (LG's keeps static state, and the process is about to exit).
  ~ExternalDelegate();

  // dlopen(path) and tflite_plugin_create_delegate(options). False with error() filled in on any failure.
  bool Load(const std::string& path, const DelegateOptions& options);

  TfLiteDelegate* get() const { return delegate_; }
  const std::string& error() const { return error_; }
  // Text the plugin passed to its report_error callback while it was created.
  const std::vector<std::string>& plugin_messages() const { return messages_; }

 private:
  void* handle_ = nullptr;
  TfLiteDelegate* delegate_ = nullptr;
  void (*destroy_)(TfLiteDelegate*) = nullptr;
  std::string error_;
  std::vector<std::string> messages_;
};

// "k=v,k2=v2" or one "k=v" into |out|; false if an item has no '='.
bool ParseOption(const std::string& item, DelegateOptions* out);

}  // namespace probe
