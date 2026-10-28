#include "gpu_path.h"

#include <dlfcn.h>
#include <sys/stat.h>

#include <algorithm>
#include <cerrno>
#include <climits>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <fstream>
#include <map>

#include "absl/container/flat_hash_set.h"
#include "tensorflow/lite/builtin_ops.h"
#include "tensorflow/lite/c/c_api.h"
#include "tensorflow/lite/delegates/gpu/common/model_builder.h"
#include "tensorflow/lite/delegates/gpu/delegate.h"
#include "tensorflow/lite/schema/schema_generated.h"

namespace probe {

bool ParseGpuOption(const std::string& kv, GpuOptions* o, std::string* error) {
  const size_t eq = kv.find('=');
  if (eq == std::string::npos || eq == 0) {
    *error = "--gpu-opt wants KEY=VALUE, got '" + kv + "'";
    return false;
  }
  const std::string key = kv.substr(0, eq), val = kv.substr(eq + 1);
  auto one_of = [&](std::initializer_list<const char*> allowed, std::string* out) {
    for (const char* a : allowed) {
      if (val == a) {
        *out = val;
        return true;
      }
    }
    std::string list;
    for (const char* a : allowed) list += (list.empty() ? "" : ", ") + std::string(a);
    *error = key + " is one of " + list + ", got '" + val + "'";
    return false;
  };
  if (key == "precision") return one_of({"fp32", "fp16"}, &o->precision);
  if (key == "pref") return one_of({"fast", "sustained", "balanced"}, &o->preference);
  if (key == "backend") return one_of({"cl", "gl", "any"}, &o->backend);
  if (key == "quant") {
    if (val == "true" || val == "1") o->quant = true;
    else if (val == "false" || val == "0") o->quant = false;
    else {
      *error = "quant is true or false, got '" + val + "'";
      return false;
    }
    return true;
  }
  if (key == "partitions") {
    char* end = nullptr;
    const long v = std::strtol(val.c_str(), &end, 10);
    if (val.empty() || *end != '\0' || v < 1 || v > 64) {
      *error = "partitions is a whole number from 1 to 64, got '" + val + "'";
      return false;
    }
    o->partitions = static_cast<int>(v);
    return true;
  }
  if (key == "cache") {
    if (val.empty()) {
      *error = "cache wants a directory";
      return false;
    }
    o->cache_dir = val;
    return true;
  }
  *error = "unknown --gpu-opt key '" + key + "' (precision, quant, pref, partitions, backend, cache)";
  return false;
}

std::string DescribeGpuOptions(const GpuOptions& o) {
  std::string s = "precision=" + o.precision + " quant=" + (o.quant ? "true" : "false") + " pref=" + o.preference +
                  " partitions=" + std::to_string(o.partitions) + " backend=" + o.backend;
  if (!o.cache_dir.empty()) s += " cache=" + o.cache_dir;
  return s;
}

TfLiteGpuDelegateOptionsV2 ToDelegateOptions(const GpuOptions& o, const TfLiteGpuDelegateOptionsV2& defaults, const std::string& token) {
  TfLiteGpuDelegateOptionsV2 d = defaults;
  const bool fp16 = o.precision == "fp16";
  d.is_precision_loss_allowed = fp16 ? 1 : 0;
  d.inference_priority1 = fp16 ? TFLITE_GPU_INFERENCE_PRIORITY_MIN_LATENCY : TFLITE_GPU_INFERENCE_PRIORITY_MAX_PRECISION;
  d.inference_priority2 = TFLITE_GPU_INFERENCE_PRIORITY_AUTO;
  d.inference_priority3 = TFLITE_GPU_INFERENCE_PRIORITY_AUTO;
  d.inference_preference = o.preference == "fast"       ? TFLITE_GPU_INFERENCE_PREFERENCE_FAST_SINGLE_ANSWER
                           : o.preference == "balanced" ? TFLITE_GPU_INFERENCE_PREFERENCE_BALANCED
                                                        : TFLITE_GPU_INFERENCE_PREFERENCE_SUSTAINED_SPEED;
  d.max_delegated_partitions = o.partitions;
  int64_t flags = 0;
  if (o.quant) flags |= TFLITE_GPU_EXPERIMENTAL_FLAGS_ENABLE_QUANT;
  if (o.backend == "cl") flags |= TFLITE_GPU_EXPERIMENTAL_FLAGS_CL_ONLY;
  else if (o.backend == "gl") flags |= TFLITE_GPU_EXPERIMENTAL_FLAGS_GL_ONLY;
  if (!o.cache_dir.empty() && !token.empty()) {
    flags |= TFLITE_GPU_EXPERIMENTAL_FLAGS_ENABLE_SERIALIZATION;
    d.serialization_dir = o.cache_dir.c_str();
    d.model_token = token.c_str();
  }
  d.experimental_flags = flags;
  return d;
}

std::string ModelToken(const std::string& path) {
  std::ifstream f(path, std::ios::binary);
  if (!f) return "";
  uint64_t h = 1469598103934665603ull;
  char buf[1 << 16];
  while (f.read(buf, sizeof buf) || f.gcount() > 0) {
    for (std::streamsize i = 0; i < f.gcount(); ++i) h = (h ^ static_cast<unsigned char>(buf[i])) * 1099511628211ull;
  }
  char out[17];
  std::snprintf(out, sizeof out, "%016llx", static_cast<unsigned long long>(h));
  return out;
}

GpuDelegate::~GpuDelegate() {
  if (delegate_ != nullptr && delete_ != nullptr) delete_(delegate_);
  // A library given with --gpu-lib stays mapped: a device's keeps threads and OpenCL objects of its own, and the process is about to exit.
}

bool GpuDelegate::Create(const GpuOptions& o, const std::string& model_path, std::string* error) {
  using CreateFn = TfLiteDelegate* (*)(const TfLiteGpuDelegateOptionsV2*);
  using DefaultFn = TfLiteGpuDelegateOptionsV2 (*)();
  CreateFn create = nullptr;
  DefaultFn defaults = nullptr;
  if (o.lib.empty()) {
    create = &TfLiteGpuDelegateV2Create;
    defaults = &TfLiteGpuDelegateOptionsV2Default;
    delete_ = &TfLiteGpuDelegateV2Delete;
    source_ = std::string("the delegate built into the probe (TFLite ") + TfLiteVersion() + ", OpenCL backend)";
  } else {
    handle_ = dlopen(o.lib.c_str(), RTLD_NOW | RTLD_LOCAL);
    if (handle_ == nullptr) {
      const char* why = dlerror();
      *error = "dlopen failed: " + std::string(why != nullptr ? why : o.lib);
      return false;
    }
    create = reinterpret_cast<CreateFn>(dlsym(handle_, "TfLiteGpuDelegateV2Create"));
    delete_ = reinterpret_cast<void (*)(TfLiteDelegate*)>(dlsym(handle_, "TfLiteGpuDelegateV2Delete"));
    defaults = reinterpret_cast<DefaultFn>(dlsym(handle_, "TfLiteGpuDelegateOptionsV2Default"));
    if (create == nullptr || delete_ == nullptr || defaults == nullptr) {
      *error = o.lib + " is not a GPU delegate library: it lacks TfLiteGpuDelegateV2Create, TfLiteGpuDelegateV2Delete or TfLiteGpuDelegateOptionsV2Default";
      delete_ = nullptr;
      return false;
    }
    source_ = o.lib;
  }
  if (!o.cache_dir.empty()) {
    if (mkdir(o.cache_dir.c_str(), 0700) != 0 && errno != EEXIST) {
      *error = "cannot create the kernel cache directory " + o.cache_dir + ": " + std::strerror(errno);
      return false;
    }
    token_ = ModelToken(model_path);
    if (token_.empty()) {
      *error = "cannot read " + model_path + " to fingerprint it for the kernel cache";
      return false;
    }
  }
  cache_dir_ = o.cache_dir;
  GpuOptions copy = o;
  copy.cache_dir = cache_dir_;  // the struct keeps the pointer into this object's string
  options_ = ToDelegateOptions(copy, defaults(), token_);
  delegate_ = create(&options_);
  if (delegate_ == nullptr) {
    *error = "TfLiteGpuDelegateV2Create returned null";
    return false;
  }
  return true;
}

int CaptureReporter::Report(const char* format, va_list args) {
  char buf[4096];
  va_list copy;
  va_copy(copy, args);
  const int n = std::vsnprintf(buf, sizeof buf, format, copy);
  va_end(copy);
  text_.append(buf);
  if (text_.empty() || text_.back() != '\n') text_.push_back('\n');
  return n;
}

namespace {

std::string NodeName(TfLiteContext* context, int node_index) {
  TfLiteNode* node = nullptr;
  TfLiteRegistration* reg = nullptr;
  if (context->GetNodeAndRegistration(context, node_index, &node, &reg) != kTfLiteOk || reg == nullptr) return "?";
  if (reg->builtin_code == kTfLiteBuiltinCustom) return reg->custom_name != nullptr ? reg->custom_name : "custom";
  return tflite::EnumNameBuiltinOperator(static_cast<tflite::BuiltinOperator>(reg->builtin_code));
}

std::vector<int> ToVector(const TfLiteIntArray* a) {
  std::vector<int> v;
  if (a != nullptr) v.assign(a->data, a->data + a->size);
  return v;
}

// GetExecutionPlan and PreviewDelegatePartitioning work only while a delegate is being prepared: the selection runs inside the Prepare of a
// delegate that replaces nothing.
struct CoverageJob {
  const GpuOptions* options;
  bool opencl;
  const CaptureReporter* reporter;
  GpuCoverage* out;
  std::string* error;
  bool ok = false;
};

void FillCoverage(TfLiteContext* context, CoverageJob* job) {
  const GpuOptions& o = *job->options;
  GpuCoverage* out = job->out;
  TfLiteIntArray* plan = nullptr;
  if (context->GetExecutionPlan(context, &plan) != kTfLiteOk || plan == nullptr) {
    *job->error = "the context has no execution plan";
    return;
  }
  const std::vector<int> order = ToVector(plan);  // the plan array belongs to the context: copy before anything else is called
  out->node_ids = order;
  out->op_names.clear();
  for (int idx : order) out->op_names.push_back(NodeName(context, idx));

  // The same exclusion DelegatePrepare makes: without OpenCL the delegate does not take SPLIT and SPLIT_V.
  absl::flat_hash_set<TfLiteBuiltinOperator> excluded;
  if (!job->opencl) {
    excluded.insert(kTfLiteBuiltinSplit);
    excluded.insert(kTfLiteBuiltinSplitV);
  }
  const size_t mark = job->reporter->text().size();
  TfLiteIntArray* claimed = tflite::gpu::GetOpsToReplace(context, o.quant, o.partitions, &excluded);
  out->claimed = ToVector(claimed);
  TfLiteIntArrayFree(claimed);
  const size_t after_first = job->reporter->text().size();
  // Every node the delegate could take, whatever max_delegated_partitions says. Its report repeats the first call's with other counts.
  TfLiteIntArray* all = tflite::gpu::GetOpsToReplace(context, o.quant, INT_MAX, &excluded);
  out->supported = ToVector(all);
  out->partition_sizes.clear();
  if (all != nullptr && all->size > 0) {
    TfLiteDelegateParams* params = nullptr;
    int n = 0;
    if (context->PreviewDelegatePartitioning(context, all, &params, &n) == kTfLiteOk) {
      for (int i = 0; i < n; ++i) out->partition_sizes.push_back(params[i].nodes_to_replace->size);
    }
  }
  TfLiteIntArrayFree(all);
  out->reasons = job->reporter->text().substr(mark, after_first - mark);
  job->ok = true;
}

TfLiteStatus CoveragePrepare(TfLiteContext* context, TfLiteDelegate* delegate) {
  FillCoverage(context, static_cast<CoverageJob*>(delegate->data_));
  return kTfLiteOk;
}

}  // namespace

bool ComputeGpuCoverage(tflite::Interpreter& in, const GpuOptions& o, bool opencl, const CaptureReporter& reporter, GpuCoverage* out,
                        std::string* error) {
  CoverageJob job{&o, opencl, &reporter, out, error};
  TfLiteDelegate probe_delegate = TfLiteDelegateCreate();
  probe_delegate.data_ = &job;
  probe_delegate.Prepare = &CoveragePrepare;
  probe_delegate.flags = kTfLiteDelegateFlagsNone;
  if (in.ModifyGraphWithDelegate(&probe_delegate) != kTfLiteOk && error->empty()) {
    *error = "the interpreter refused the preparing delegate";
    return false;
  }
  return job.ok;
}

void PrintGpuCoverage(const GpuCoverage& c) {
  std::map<std::string, int> taken, left;
  for (size_t i = 0; i < c.op_names.size(); ++i) {
    const bool is_taken = std::find(c.claimed.begin(), c.claimed.end(), c.node_ids[i]) != c.claimed.end();
    ++(is_taken ? taken : left)[c.op_names[i]];
  }
  auto hist = [](const std::map<std::string, int>& h) {
    std::string s;
    for (const auto& kv : h) s += (s.empty() ? "" : ", ") + kv.first + " x" + std::to_string(kv.second);
    return s.empty() ? std::string("(none)") : s;
  };
  std::printf("  nodes: %zu; the GPU delegate can take %zu of them", c.op_names.size(), c.supported.size());
  if (!c.partition_sizes.empty()) {
    std::printf(" in %zu piece(s) of", c.partition_sizes.size());
    for (size_t i = 0; i < c.partition_sizes.size() && i < 12; ++i) std::printf(" %d", c.partition_sizes[i]);
    if (c.partition_sizes.size() > 12) std::printf(" ...");
  }
  std::printf("\n  with this partitions setting it takes %zu node(s) and leaves %zu on the CPU\n", c.claimed.size(),
              c.op_names.size() - c.claimed.size());
  std::printf("  on the GPU: %s\n  on the CPU: %s\n", hist(taken).c_str(), hist(left).c_str());
  if (!c.reasons.empty()) {
    std::printf("  TFLite's reasons:\n");
    size_t pos = 0;
    while (pos < c.reasons.size()) {
      size_t nl = c.reasons.find('\n', pos);
      if (nl == std::string::npos) nl = c.reasons.size();
      if (nl > pos) std::printf("    %.*s\n", static_cast<int>(nl - pos), c.reasons.c_str() + pos);
      pos = nl + 1;
    }
  }
}

}  // namespace probe
