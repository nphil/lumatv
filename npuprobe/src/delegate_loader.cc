#include "delegate_loader.h"

#include <dlfcn.h>

#include <cstring>
#include <exception>
#include <mutex>

namespace probe {
namespace {

using CreateFn = TfLiteDelegate* (*)(char**, char**, size_t, void (*)(const char*));
using DestroyFn = void (*)(TfLiteDelegate*);

// The plugin ABI has no user-data pointer on the error callback, so the messages go through one process-wide sink.
std::mutex g_mu;
std::vector<std::string>* g_sink = nullptr;

void ReportError(const char* message) {
  std::lock_guard<std::mutex> lock(g_mu);
  if (g_sink != nullptr) g_sink->push_back(message != nullptr ? message : "(null)");
}

}  // namespace

bool ParseOption(const std::string& item, DelegateOptions* out) {
  size_t eq = item.find('=');
  if (eq == std::string::npos || eq == 0) return false;
  out->emplace_back(item.substr(0, eq), item.substr(eq + 1));
  return true;
}

bool ExternalDelegate::Load(const std::string& path, const DelegateOptions& options) {
  // RTLD_NOW: a plugin that cannot bind its runtime symbols (TfLiteDelegateCreate, tflite::Subgraph::..., libnnrt)
  // should fail here with the symbol named, not later inside the first inference.
  handle_ = dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL);
  if (handle_ == nullptr) {
    const char* why = dlerror();
    error_ = std::string("dlopen failed: ") + (why != nullptr ? why : "unknown error");
    return false;
  }
  auto create = reinterpret_cast<CreateFn>(dlsym(handle_, "tflite_plugin_create_delegate"));
  destroy_ = reinterpret_cast<DestroyFn>(dlsym(handle_, "tflite_plugin_destroy_delegate"));
  if (create == nullptr || destroy_ == nullptr) {
    error_ = "not an external delegate: tflite_plugin_create_delegate/tflite_plugin_destroy_delegate missing";
    destroy_ = nullptr;
    return false;
  }

  // The plugin ABI takes writable char**; the strings are not modified by any plugin seen so far, but copy anyway.
  std::vector<std::string> keys, values;
  for (const auto& kv : options) {
    keys.push_back(kv.first);
    values.push_back(kv.second);
  }
  std::vector<char*> key_ptrs, value_ptrs;
  for (auto& k : keys) key_ptrs.push_back(k.data());
  for (auto& v : values) value_ptrs.push_back(v.data());

  {
    std::lock_guard<std::mutex> lock(g_mu);
    g_sink = &messages_;
  }
  // LG's plugin parses min_freq with std::stoi and lets the exception out of the create call.
  try {
    delegate_ = create(key_ptrs.data(), value_ptrs.data(), key_ptrs.size(), &ReportError);
  } catch (...) {
    std::string what = "unknown exception";
    try {
      throw;
    } catch (const std::exception& e) {
      what = e.what();
    } catch (...) {
    }
    delegate_ = nullptr;
    error_ = "tflite_plugin_create_delegate threw: " + what;
  }
  {
    std::lock_guard<std::mutex> lock(g_mu);
    g_sink = nullptr;
  }
  if (!error_.empty()) return false;
  if (delegate_ == nullptr) {
    error_ = "tflite_plugin_create_delegate returned null";
    if (!messages_.empty()) error_ += ": " + messages_.front();
    return false;
  }
  return true;
}

ExternalDelegate::~ExternalDelegate() {
  if (delegate_ != nullptr && destroy_ != nullptr) destroy_(delegate_);
}

}  // namespace probe
