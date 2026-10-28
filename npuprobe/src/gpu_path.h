// The GPU route: TFLite's GPU delegate V2 on its OpenCL backend (the TV's Mali-G52 through the Khronos ICD loader LG ships). The delegate is
// the one built into the probe, or one taken from a library with the three V2 entry points (--gpu-lib: the stub of the tests, or a
// libtensorflowlite_gpu_delegate.so of the device, which must then be of the probe's TFLite release).
#pragma once

#include <cstdint>
#include <cstdarg>
#include <string>
#include <vector>

#include "tensorflow/lite/core/api/error_reporter.h"
#include "tensorflow/lite/delegates/gpu/delegate_options.h"
#include "tensorflow/lite/interpreter.h"

namespace probe {

struct GpuOptions {
  std::string precision = "fp32";        // fp32: no precision loss allowed; fp16: 16-bit arithmetic where the device has it (cl_khr_fp16)
  bool quant = true;                     // run models with 8-bit tensors (the GPU computes them in float); false leaves those ops on the CPU
  std::string preference = "sustained";  // fast (FAST_SINGLE_ANSWER: short start-up), sustained (SUSTAINED_SPEED) or balanced
  int partitions = 1;                    // max_delegated_partitions: how many separate pieces of the graph go to the GPU
  std::string backend = "cl";            // cl (OpenCL only), gl, any
  std::string cache_dir;                 // serialisation directory: compiled kernels and tuning are stored there, later starts skip them
  std::string lib;                       // library with the V2 entry points; empty: the delegate built into the probe
};

// "key=value" with keys precision, quant, pref, partitions, backend, cache.
bool ParseGpuOption(const std::string& kv, GpuOptions* o, std::string* error);
std::string DescribeGpuOptions(const GpuOptions& o);

// TfLiteGpuDelegateOptionsV2 from |o|; |defaults| is what the entry point's own TfLiteGpuDelegateOptionsV2Default() returned. |token| and
// |o.cache_dir| must outlive the delegate (the struct keeps their pointers).
TfLiteGpuDelegateOptionsV2 ToDelegateOptions(const GpuOptions& o, const TfLiteGpuDelegateOptionsV2& defaults, const std::string& token);

// FNV-1a 64 of a file, as 16 hex digits: the delegate's model_token for the kernel cache. Empty if the file cannot be read.
std::string ModelToken(const std::string& path);

class GpuDelegate {
 public:
  GpuDelegate() = default;
  GpuDelegate(const GpuDelegate&) = delete;
  GpuDelegate& operator=(const GpuDelegate&) = delete;
  ~GpuDelegate();

  bool Create(const GpuOptions& o, const std::string& model_path, std::string* error);
  TfLiteDelegate* get() const { return delegate_; }
  const std::string& source() const { return source_; }
  // The struct handed to the entry point, for the report.
  const TfLiteGpuDelegateOptionsV2& options() const { return options_; }

 private:
  void* handle_ = nullptr;
  TfLiteDelegate* delegate_ = nullptr;
  void (*delete_)(TfLiteDelegate*) = nullptr;
  TfLiteGpuDelegateOptionsV2 options_{};
  std::string token_, cache_dir_, source_;
};

// Collects what TFLite reports through an interpreter's error reporter (the GPU delegate lists the operations it cannot take, with the
// reason for each, that way).
class CaptureReporter : public tflite::ErrorReporter {
 public:
  int Report(const char* format, va_list args) override;
  const std::string& text() const { return text_; }

 private:
  std::string text_;
};

struct GpuCoverage {
  std::vector<int> node_ids;           // the nodes in execution order
  std::vector<std::string> op_names;   // and their operations
  std::vector<int> claimed;            // nodes the delegate takes with the chosen max_delegated_partitions
  std::vector<int> supported;          // every node it could take
  std::vector<int> partition_sizes;    // the supported nodes cut into the pieces TFLite would hand to separate kernels, in plan order
  std::string reasons;                 // TFLite's list of the unsupported operations and why
};

// What the delegate built into the probe would take of |in|, by the delegate's own selection code (tflite::gpu::GetOpsToReplace). No OpenCL
// device is needed; |opencl| says whether one would be there (without it the delegate leaves SPLIT and SPLIT_V to the CPU).
// |reporter| must be the interpreter's error reporter. False with |error| set if the context refuses.
bool ComputeGpuCoverage(tflite::Interpreter& in, const GpuOptions& o, bool opencl, const CaptureReporter& reporter, GpuCoverage* out,
                        std::string* error);

// The same, written out: counts, the ops taken and left, the pieces, TFLite's reasons.
void PrintGpuCoverage(const GpuCoverage& c);

}  // namespace probe
