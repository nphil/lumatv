// lumatv-npu-probe: time a .tflite model on the CPU and through an accelerator route, and say which nodes the accelerator
// took: a TFLite external delegate (LG's libtflite-npu-delegate), TFLite's NNAPI delegate over the device's NN HAL, or TFLite's GPU
// delegate on OpenCL (the TV's Mali-G52). Built for the TV's 32-bit ARM userland and for the build host (stub-delegate, stub-NNAPI and
// stub-OpenCL tests).
//
//   lumatv-npu-probe --model m.tflite --image person.jpg --runs 100
//       [--delegate /usr/lib/libtflite-npu-delegate.so.1 --delegate-opt min_freq=30]
//       [--nnapi --nnapi-opt accelerator=lgnpu]
//       [--gpu --gpu-opt precision=fp16]        --gpu-info: what OpenCL offers; --gpu-coverage: what the GPU delegate would take
//
// Output is plain text; the last line, SUMMARY, repeats the headline numbers as key=value pairs.
#include <dirent.h>
#include <dlfcn.h>
#include <signal.h>
#include <sys/stat.h>
#include <ucontext.h>
#include <unistd.h>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <memory>
#include <string>
#include <thread>
#include <vector>

#include "delegate_loader.h"
#include "detect.h"
#include "gpu_path.h"
#include "image.h"
#include "nnapi_path.h"
#include "opencl_info.h"
#include "sysinfo.h"
#include "tensorflow/lite/builtin_ops.h"
#include "tensorflow/lite/c/c_api.h"
#include "tensorflow/lite/delegates/xnnpack/xnnpack_delegate.h"
#include "tensorflow/lite/interpreter.h"
#include "tensorflow/lite/kernels/register.h"
#include "tensorflow/lite/model.h"
#include "tensorflow/lite/schema/schema_generated.h"

namespace {

constexpr const char* kVersion = "0.1";
constexpr const char* kLgCustomOp = "lgnpu_custom_op";  // the node LG's delegate looks for (strings of the library)

struct Args {
  std::string model;
  std::string image;
  std::string labels;
  std::string delegate;
  probe::DelegateOptions delegate_opts;
  std::string cpu_backend = "xnnpack";  // xnnpack | builtin | none
  std::string norm = "auto";            // auto | raw | unit | signed
  std::string fill = "image";           // image | pattern | const:<real value>
  std::string proc_dir = "/proc/npu";
  int runs = 100;
  int warmup = 5;
  int threads = 0;
  int top = 5;
  float min_score = 0.3f;
  bool info = false;
  bool require_delegate = false;
  bool print_output = false;
  bool system_info = false;
  bool nnapi = false;
  bool nnapi_list = false;
  probe::NnapiOptions nnapi_opts;
  bool gpu = false;
  bool gpu_info = false;
  bool gpu_coverage = false;
  probe::GpuOptions gpu_opts;
  std::string opencl_lib;  // the OpenCL ICD loader to open (default: the device's libOpenCL.so.1)
  std::string opencl_icd;  // the vendor driver the loader should use without an .icd file (OCL_ICD_FILENAMES)
  std::string mali_proc = "/sys/kernel/debug/mali0/dvfs_utilization";
};

void Usage() {
  std::fputs(
      "usage: lumatv-npu-probe --model FILE.tflite [options]\n"
      "  --image FILE            JPEG/PNG/BMP/PNM input (default: a built-in test pattern)\n"
      "  --fill image|pattern|const:V   what goes into input 0 (const: every element = real value V)\n"
      "  --norm auto|raw|unit|signed    byte -> real value: 0..255, 0..1, -1..1 (auto: raw for 8-bit inputs, unit for float)\n"
      "  --runs N --warmup N     timed and untimed inferences per phase (100, 5)\n"
      "  --threads N             CPU threads (default: online cores, at most 4)\n"
      "  --cpu-backend xnnpack|builtin|none   the no-delegate phase (xnnpack)\n"
      "  --delegate LIB.so       TFLite external delegate to try (tflite_plugin_create_delegate)\n"
      "  --delegate-opt K=V      option for the plugin, repeatable (LG's: min_freq=30, use_custom_alloc=true)\n"
      "  --require-delegate      exit 3 unless the delegate took a node and the timed runs completed, 5 if its outputs are not the CPU's\n"
      "  --nnapi                 instead of --delegate: TFLite's NNAPI delegate over the device's libneuralnetworks.so (the NN HAL route)\n"
      "  --nnapi-opt K=V         accelerator=NAME (lgnpu), sdk=N (Android level to assume), pref=low|fast|sustained, partitions=3, allow_cpu=true, allow_fp16=true\n"
      "  --nnapi-list            load libneuralnetworks.so, list its devices and exit\n"
      "  --gpu                   instead of --delegate: TFLite's GPU delegate V2 on OpenCL (the delegate built into the probe)\n"
      "  --gpu-opt K=V           precision=fp32|fp16, quant=true|false, pref=fast|sustained|balanced, partitions=N, backend=cl|gl|any, cache=DIR\n"
      "  --gpu-lib LIB.so        take the GPU delegate from a library with TfLiteGpuDelegateV2Create/Delete/OptionsV2Default instead\n"
      "  --opencl-lib LIB.so     the OpenCL ICD loader to open (default libOpenCL.so.1)\n"
      "  --opencl-icd LIB.so     vendor driver for the loader to use without an .icd file (sets OCL_ICD_FILENAMES; e.g. /usr/lib/libmali.so.0)\n"
      "  --gpu-info              list the OpenCL platforms and devices, the .icd files and the Mali counters, and exit (3: no device)\n"
      "  --gpu-coverage          which nodes of the model the GPU delegate takes (needs no GPU) and exit\n"
      "  --gpu-proc FILE         Mali busy/idle counters (/sys/kernel/debug/mali0/dvfs_utilization)\n"
      "  --labels FILE           class names, one per line (default: COCO)\n"
      "  --min-score X --top K   detection filter and count (0.3, 5)\n"
      "  --print-output          first values of every output tensor\n"
      "  --proc-dir DIR          NPU driver proc directory (/proc/npu)\n"
      "  --info                  describe the model and exit\n"
      "  --system-info           what the device itself has (TFLite runtime and version, XNNPACK, the delegate's entry points) and exit\n",
      stderr);
}

bool ParseArgs(int argc, char** argv, Args* a) {
  auto need = [&](int& i, const char* flag) -> const char* {
    if (i + 1 >= argc) {
      std::fprintf(stderr, "%s needs a value\n", flag);
      return nullptr;
    }
    return argv[++i];
  };
  for (int i = 1; i < argc; ++i) {
    std::string f = argv[i];
    const char* v = nullptr;
    if (f == "--info") {
      a->info = true;
    } else if (f == "--require-delegate") {
      a->require_delegate = true;
    } else if (f == "--system-info") {
      a->system_info = true;
    } else if (f == "--nnapi") {
      a->nnapi = true;
    } else if (f == "--nnapi-list") {
      a->nnapi_list = true;
    } else if (f == "--gpu") {
      a->gpu = true;
    } else if (f == "--gpu-info") {
      a->gpu_info = true;
    } else if (f == "--gpu-coverage") {
      a->gpu_coverage = true;
    } else if (f == "--print-output") {
      a->print_output = true;
    } else if (f == "-h" || f == "--help") {
      Usage();
      std::exit(0);
    } else if (f == "--model" || f == "--image" || f == "--labels" || f == "--delegate" || f == "--delegate-opt" ||
               f == "--cpu-backend" || f == "--norm" || f == "--fill" || f == "--proc-dir" || f == "--runs" ||
               f == "--warmup" || f == "--threads" || f == "--top" || f == "--min-score" || f == "--nnapi-opt" ||
               f == "--gpu-opt" || f == "--gpu-lib" || f == "--opencl-lib" || f == "--opencl-icd" || f == "--gpu-proc") {
      v = need(i, f.c_str());
      if (v == nullptr) return false;
      if (f == "--model") a->model = v;
      else if (f == "--image") a->image = v;
      else if (f == "--labels") a->labels = v;
      else if (f == "--delegate") a->delegate = v;
      else if (f == "--delegate-opt") {
        if (!probe::ParseOption(v, &a->delegate_opts)) {
          std::fprintf(stderr, "--delegate-opt wants KEY=VALUE, got '%s'\n", v);
          return false;
        }
      } else if (f == "--nnapi-opt") {
        std::string why;
        if (!probe::ParseNnapiOption(v, &a->nnapi_opts, &why)) {
          std::fprintf(stderr, "%s\n", why.c_str());
          return false;
        }
      } else if (f == "--gpu-opt") {
        std::string why;
        if (!probe::ParseGpuOption(v, &a->gpu_opts, &why)) {
          std::fprintf(stderr, "%s\n", why.c_str());
          return false;
        }
      } else if (f == "--gpu-lib") a->gpu_opts.lib = v;
      else if (f == "--opencl-lib") a->opencl_lib = v;
      else if (f == "--opencl-icd") a->opencl_icd = v;
      else if (f == "--gpu-proc") a->mali_proc = v;
      else if (f == "--cpu-backend") a->cpu_backend = v;
      else if (f == "--norm") a->norm = v;
      else if (f == "--fill") a->fill = v;
      else if (f == "--proc-dir") a->proc_dir = v;
      else if (f == "--runs") a->runs = std::atoi(v);
      else if (f == "--warmup") a->warmup = std::atoi(v);
      else if (f == "--threads") a->threads = std::atoi(v);
      else if (f == "--top") a->top = std::atoi(v);
      else if (f == "--min-score") a->min_score = static_cast<float>(std::atof(v));
    } else {
      std::fprintf(stderr, "unknown option %s\n", f.c_str());
      return false;
    }
  }
  if (a->model.empty() && !a->system_info && !a->nnapi_list && !a->gpu_info) {
    std::fputs("--model is required\n", stderr);
    return false;
  }
  if ((a->nnapi ? 1 : 0) + (a->gpu ? 1 : 0) + (a->delegate.empty() ? 0 : 1) > 1) {
    std::fputs("--delegate, --nnapi and --gpu are separate routes: one route per run\n", stderr);
    return false;
  }
  if (a->cpu_backend != "xnnpack" && a->cpu_backend != "builtin" && a->cpu_backend != "none") {
    std::fputs("--cpu-backend is xnnpack, builtin or none\n", stderr);
    return false;
  }
  if (a->norm != "auto" && a->norm != "raw" && a->norm != "unit" && a->norm != "signed") {
    std::fputs("--norm is auto, raw, unit or signed\n", stderr);
    return false;
  }
  if (a->fill != "image" && a->fill != "pattern" && a->fill.rfind("const:", 0) != 0) {
    std::fputs("--fill is image, pattern or const:V\n", stderr);
    return false;
  }
  if (a->runs < 1) a->runs = 1;
  if (a->warmup < 0) a->warmup = 0;
  if (a->threads <= 0) {
    unsigned hw = std::thread::hardware_concurrency();
    a->threads = static_cast<int>(std::min(4u, hw == 0 ? 1u : hw));
  }
  return true;
}

// ---------------------------------------------------------------------------------------------------------
// Model description (straight from the flatbuffer, so custom ops the runtime cannot resolve still show up)

int32_t BuiltinCode(const tflite::OperatorCode* oc) {
  return std::max<int32_t>(oc->builtin_code(), oc->deprecated_builtin_code());  // old files only fill the deprecated byte
}

std::string OpCodeName(const tflite::OperatorCode* oc) {
  int32_t code = BuiltinCode(oc);
  if (code == tflite::BuiltinOperator_CUSTOM) return oc->custom_code() ? oc->custom_code()->str() : "CUSTOM";
  return tflite::EnumNameBuiltinOperator(static_cast<tflite::BuiltinOperator>(code));
}

// First "digits.digits.digits" in |s|, "" if none.
std::string FirstVersion(const std::string& s) {
  for (size_t i = 0; i < s.size(); ++i) {
    if (s[i] < '0' || s[i] > '9') continue;
    size_t j = i;
    int dots = 0;
    while (j < s.size() && ((s[j] >= '0' && s[j] <= '9') || (s[j] == '.' && ++dots <= 2))) ++j;
    if (dots == 2 && s[j - 1] != '.') return s.substr(i, j - i);
    i = j;
  }
  return "";
}

struct ModelFacts {
  std::map<std::string, int> ops;  // name -> count, subgraph 0
  int op_count = 0;
  int npu_nodes = 0;               // lgnpu_custom_op nodes
  size_t npu_payload_bytes = 0;    // their custom options, the compiled NPU package
  std::vector<std::string> custom_ops;
  std::string converter;           // TF version that wrote the model, when the metadata says
  std::string min_runtime;
  std::string description;
};

ModelFacts ReadFacts(const tflite::Model& m) {
  ModelFacts f;
  if (m.description()) f.description = m.description()->str();
  if (m.subgraphs() && m.subgraphs()->size() > 0 && m.operator_codes()) {
    const tflite::SubGraph* sg = m.subgraphs()->Get(0);
    for (const tflite::Operator* op : *sg->operators()) {
      std::string name = OpCodeName(m.operator_codes()->Get(op->opcode_index()));
      ++f.ops[name];
      ++f.op_count;
      const bool custom = BuiltinCode(m.operator_codes()->Get(op->opcode_index())) == tflite::BuiltinOperator_CUSTOM;
      if (custom && std::find(f.custom_ops.begin(), f.custom_ops.end(), name) == f.custom_ops.end())
        f.custom_ops.push_back(name);
      if (name == kLgCustomOp) {
        ++f.npu_nodes;
        if (op->custom_options()) f.npu_payload_bytes += op->custom_options()->size();
      }
    }
  }
  if (m.metadata() && m.buffers()) {
    for (const tflite::Metadata* md : *m.metadata()) {
      if (!md->name() || md->buffer() >= m.buffers()->size()) continue;
      const tflite::Buffer* b = m.buffers()->Get(md->buffer());
      if (!b->data()) continue;
      std::string bytes(reinterpret_cast<const char*>(b->data()->data()), b->data()->size());
      std::string name = md->name()->str();
      if (name == "min_runtime_version") f.min_runtime = bytes.c_str();
      if (name == "CONVERSION_METADATA") {
        f.converter = FirstVersion(bytes.size() > 40 ? bytes.substr(40) : bytes);
      }
    }
  }
  return f;
}

std::string QuantString(const TfLiteTensor* t) {
  char buf[96];
  if (t->quantization.type == kTfLiteAffineQuantization && t->quantization.params) {
    auto* q = static_cast<const TfLiteAffineQuantization*>(t->quantization.params);
    if (q->scale && q->scale->size > 1) {
      std::snprintf(buf, sizeof buf, " per-channel x%d", q->scale->size);
      return buf;
    }
  }
  if (t->params.scale != 0) {
    std::snprintf(buf, sizeof buf, " scale=%.6g zp=%d", t->params.scale, t->params.zero_point);
    return buf;
  }
  return "";
}

void PrintIo(const tflite::Interpreter& in) {
  for (int idx : in.inputs()) {
    const TfLiteTensor* t = in.tensor(idx);
    std::printf("  input  %-28s %-8s %s%s\n", t->name ? t->name : "?", TfLiteTypeGetName(t->type),
                probe::ShapeString(t->dims).c_str(), QuantString(t).c_str());
  }
  for (int idx : in.outputs()) {
    const TfLiteTensor* t = in.tensor(idx);
    std::printf("  output %-28s %-8s %s%s\n", t->name ? t->name : "?", TfLiteTypeGetName(t->type),
                probe::ShapeString(t->dims).c_str(), QuantString(t).c_str());
  }
}

std::string HistogramString(const std::map<std::string, int>& h) {
  std::string s;
  for (const auto& kv : h) s += (s.empty() ? "" : ", ") + kv.first + " x" + std::to_string(kv.second);
  return s;
}

// ---------------------------------------------------------------------------------------------------------
// One interpreter and what happened to it

const char* StatusName(TfLiteStatus s) {
  switch (s) {
    case kTfLiteOk: return "ok";
    case kTfLiteError: return "error";
    case kTfLiteDelegateError: return "delegate error";
    case kTfLiteApplicationError: return "application error";
    case kTfLiteDelegateDataNotFound: return "delegate data not found";
    case kTfLiteDelegateDataWriteError: return "delegate data write error";
    case kTfLiteDelegateDataReadError: return "delegate data read error";
    case kTfLiteUnresolvedOps: return "unresolved ops";
    default: return "unknown status";
  }
}

struct PlanOp {
  std::string name;
  bool delegate_kernel = false;
  int inputs = 0, outputs = 0;
};

std::vector<PlanOp> Plan(const tflite::Interpreter& in) {
  std::vector<PlanOp> plan;
  for (int idx : in.execution_plan()) {
    const auto* nr = in.node_and_registration(idx);
    const TfLiteRegistration& reg = nr->second;
    PlanOp p;
    p.inputs = nr->first.inputs ? nr->first.inputs->size : 0;
    p.outputs = nr->first.outputs ? nr->first.outputs->size : 0;
    if (reg.builtin_code == kTfLiteBuiltinDelegate) {
      p.delegate_kernel = true;
      p.name = std::string("delegate kernel ") + (reg.custom_name ? reg.custom_name : "(unnamed)");
    } else if (reg.builtin_code == kTfLiteBuiltinCustom) {
      p.name = reg.custom_name ? reg.custom_name : "custom";
    } else {
      p.name = tflite::EnumNameBuiltinOperator(static_cast<tflite::BuiltinOperator>(reg.builtin_code));
    }
    plan.push_back(p);
  }
  return plan;
}

std::map<std::string, int> Histogram(const std::vector<PlanOp>& plan, bool delegate_kernels) {
  std::map<std::string, int> h;
  for (const PlanOp& p : plan)
    if (p.delegate_kernel == delegate_kernels) ++h[p.name];
  return h;
}

struct Stats {
  double min = 0, mean = 0, median = 0, p95 = 0, max = 0, stddev = 0;
};

Stats Summarize(std::vector<double> v) {
  Stats s;
  if (v.empty()) return s;
  std::sort(v.begin(), v.end());
  s.min = v.front();
  s.max = v.back();
  s.median = v[v.size() / 2];
  s.p95 = v[std::min(v.size() - 1, static_cast<size_t>(std::ceil(0.95 * v.size())) - 1)];
  double sum = 0;
  for (double x : v) sum += x;
  s.mean = sum / v.size();
  double sq = 0;
  for (double x : v) sq += (x - s.mean) * (x - s.mean);
  s.stddev = std::sqrt(sq / v.size());
  return s;
}

struct Phase {
  bool ran = false;
  bool delegate_applied = false;
  std::string note;
  std::vector<double> ms;
  double cpu_ms_per_inference = 0;
  double cpu_percent = 0;
  double init_ms = -1;       // delegate set-up: ModifyGraphWithDelegate and AllocateTensors (-1: not timed)
  double first_ms = -1;      // the first invoke, where lazy set-up lands (-1: no warm-up run)
  double mali_percent = -1;  // Mali busy share over the timed loop, from the debugfs counters (-1: not readable)
  probe::NpuSampler::Result npu;
  bool npu_sampled = false;
  std::vector<std::vector<float>> outputs;
  std::vector<std::string> output_names;
  std::vector<probe::Detection> detections;
  bool detections_ok = false;
  std::string detections_why;
  std::string verdict = "n/a";  // outputs against the CPU's: identical, close, zero, differ or n/a
};

// |image_bytes| is the picture already resized to the input size (or empty for const fills).
bool FillInput(tflite::Interpreter* in, const probe::Image& img, const Args& args, std::string* error) {
  if (in->inputs().empty()) {
    *error = "model has no input";
    return false;
  }
  TfLiteTensor* t = in->tensor(in->inputs()[0]);
  if (t->data.raw == nullptr) {
    *error = "input tensor not allocated";
    return false;
  }
  const size_t n = t->bytes / (t->type == kTfLiteFloat32 || t->type == kTfLiteInt32 ? 4 : t->type == kTfLiteInt16 ? 2 : 1);
  const int channels = t->dims->size >= 1 ? t->dims->data[t->dims->size - 1] : 1;
  const bool eight_bit = t->type == kTfLiteUInt8 || t->type == kTfLiteInt8;
  std::string norm = args.norm;
  if (norm == "auto") norm = eight_bit ? "raw" : "unit";
  const bool is_const = args.fill.rfind("const:", 0) == 0;
  const float const_value = is_const ? static_cast<float>(std::atof(args.fill.c_str() + 6)) : 0.f;

  auto real_value = [&](size_t i) -> float {
    if (is_const) return const_value;
    const size_t pixel = i / channels, c = i % channels;
    float byte;
    if (channels == 1) {
      byte = 0;
    } else {
      byte = img.rgb[pixel * 3 + c % 3];
    }
    if (channels == 1) byte = (img.rgb[pixel * 3] + img.rgb[pixel * 3 + 1] + img.rgb[pixel * 3 + 2]) / 3.f;
    if (norm == "raw") return byte;
    if (norm == "unit") return byte / 255.f;
    return (byte - 127.5f) / 127.5f;
  };

  switch (t->type) {
    case kTfLiteFloat32:
      for (size_t i = 0; i < n; ++i) t->data.f[i] = real_value(i);
      break;
    case kTfLiteUInt8:
    case kTfLiteInt8: {
      const bool is_u8 = t->type == kTfLiteUInt8;
      const float scale = t->params.scale != 0 ? t->params.scale : 1.f;
      const int zp = t->params.zero_point;
      // Raw bytes into a uint8 tensor go in unchanged: the model's own scale/zero point (LG's is 1/255, 0) is a
      // statement about how it reads them.
      const bool identity = is_u8 && norm == "raw" && !is_const;
      for (size_t i = 0; i < n; ++i) {
        float r = real_value(i);
        int q = identity ? static_cast<int>(r) : static_cast<int>(std::lround(r / scale)) + zp;
        q = std::clamp(q, is_u8 ? 0 : -128, is_u8 ? 255 : 127);
        if (is_u8) t->data.uint8[i] = static_cast<uint8_t>(q);
        else t->data.int8[i] = static_cast<int8_t>(q);
      }
      break;
    }
    default:
      *error = std::string("input type ") + TfLiteTypeGetName(t->type) + " not supported by the probe";
      return false;
  }
  // further inputs (rare for detectors): zero
  for (size_t k = 1; k < in->inputs().size(); ++k) {
    TfLiteTensor* extra = in->tensor(in->inputs()[k]);
    if (extra->data.raw) std::memset(extra->data.raw, 0, extra->bytes);
  }
  return true;
}

struct Interp {
  // The delegate must outlive the interpreter: it is declared first so it is destroyed last.
  std::unique_ptr<TfLiteDelegate, void (*)(TfLiteDelegate*)> xnn{nullptr, &TfLiteXNNPackDelegateDelete};
  std::unique_ptr<tflite::Interpreter> interp;
};

// A custom op the runtime has no kernel for (lgnpu_custom_op) gets a registration without a Prepare, so that
// ModifyGraphWithDelegate can walk the graph and hand the node to a delegate; running it on the CPU is an error.
// Without it a delegate that does not set kTfLiteDelegateFlagsAllowDynamicTensors is refused with kTfLiteUnresolvedOps.
const TfLiteRegistration* PlaceholderOp() {
  static const TfLiteRegistration reg = [] {
    TfLiteRegistration r = {};
    r.builtin_code = kTfLiteBuiltinCustom;
    r.custom_name = "placeholder";
    r.version = 1;
    r.invoke = [](TfLiteContext* context, TfLiteNode*) {
      TF_LITE_KERNEL_LOG(context, "custom op has no CPU kernel: it runs only inside its delegate");
      return kTfLiteError;
    };
    return r;
  }();
  return &reg;
}

bool Build(const tflite::FlatBufferModel& model, int threads, const std::vector<std::string>& placeholder_ops,
           Interp* out, std::string* error) {
  tflite::ops::builtin::BuiltinOpResolverWithoutDefaultDelegates resolver;  // no hidden XNNPACK: the probe picks
  for (const std::string& name : placeholder_ops)
    if (resolver.FindOp(name.c_str(), 1) == nullptr) resolver.AddCustom(name.c_str(), PlaceholderOp(), 1, 8);
  tflite::InterpreterBuilder builder(model, resolver);
  if (builder(&out->interp, threads) != kTfLiteOk || !out->interp) {
    *error = "InterpreterBuilder failed";
    return false;
  }
  return true;
}

void PrintStats(const Phase& p) {
  Stats s = Summarize(p.ms);
  std::printf("  per-inference ms (%zu):\n", p.ms.size());
  for (size_t i = 0; i < p.ms.size(); ++i) {
    if (i % 10 == 0) std::printf("   ");
    std::printf(" %.3f", p.ms[i]);
    if (i % 10 == 9 || i + 1 == p.ms.size()) std::printf("\n");
  }
  std::printf("  n=%zu min=%.3f mean=%.3f median=%.3f p95=%.3f max=%.3f stddev=%.3f ms\n", p.ms.size(), s.min, s.mean,
              s.median, s.p95, s.max, s.stddev);
  std::printf("  process CPU: %.3f ms per inference, %.0f%% of one core over the timed loop\n", p.cpu_ms_per_inference,
              p.cpu_percent);
  if (p.init_ms >= 0) std::printf("  set-up %.1f ms (delegate and tensors), first invoke %.1f ms\n", p.init_ms, p.first_ms < 0 ? 0.0 : p.first_ms);
  if (p.mali_percent >= 0) std::printf("  Mali busy: %.1f%% over the timed loop\n", p.mali_percent);
  if (p.npu_sampled) {
    std::printf("  npu_utilization: %d samples, max %d, mean %.1f, non-zero %d\n", p.npu.samples, p.npu.max,
                p.npu.mean, p.npu.nonzero);
  }
}

// Runs warm-up and timed loops on an interpreter whose tensors are allocated and whose input is filled.
bool TimedRuns(tflite::Interpreter* in, const Args& args, Phase* p, std::string* error) {
  for (int i = 0; i < args.warmup; ++i) {
    const double w0 = probe::WallSeconds();
    if (in->Invoke() != kTfLiteOk) {
      *error = "Invoke failed during warm-up";
      return false;
    }
    if (i == 0) p->first_ms = (probe::WallSeconds() - w0) * 1e3;
  }
  probe::NpuSampler sampler(args.proc_dir);
  const probe::MaliCounters mali0 = probe::ReadMaliCounters(args.mali_proc);
  const double cpu0 = probe::CpuSeconds(), wall0 = probe::WallSeconds();
  for (int i = 0; i < args.runs; ++i) {
    const double t0 = probe::WallSeconds();
    TfLiteStatus st = in->Invoke();
    const double t1 = probe::WallSeconds();
    if (st != kTfLiteOk) {
      *error = "Invoke failed at run " + std::to_string(i);
      return false;
    }
    p->ms.push_back((t1 - t0) * 1e3);
  }
  const double cpu1 = probe::CpuSeconds(), wall1 = probe::WallSeconds();
  p->mali_percent = probe::MaliBusyPercent(mali0, probe::ReadMaliCounters(args.mali_proc));
  p->npu_sampled = sampler.available();
  p->npu = sampler.Stop();
  p->cpu_ms_per_inference = (cpu1 - cpu0) * 1e3 / args.runs;
  p->cpu_percent = wall1 > wall0 ? (cpu1 - cpu0) / (wall1 - wall0) * 100.0 : 0;
  return true;
}

void CaptureOutputs(const tflite::Interpreter& in, Phase* p) {
  for (int idx : in.outputs()) {
    const TfLiteTensor* t = in.tensor(idx);
    p->outputs.push_back(probe::TensorAsFloat(*t));
    p->output_names.push_back(t->name ? t->name : "?");
  }
}

void PrintOutputs(const tflite::Interpreter& in) {
  for (int idx : in.outputs()) {
    const TfLiteTensor* t = in.tensor(idx);
    std::vector<float> v = probe::TensorAsFloat(*t);
    std::printf("  %s:", t->name ? t->name : "?");
    for (size_t i = 0; i < v.size() && i < 8; ++i) std::printf(" %.6g", v[i]);
    std::printf("%s\n", v.size() > 8 ? " ..." : "");
  }
}

void ShowDetections(const Phase& p, const Args& args, const probe::Labels& labels, const probe::Image& src) {
  if (!p.detections_ok) {
    std::printf("  no detections decoded: %s\n", p.detections_why.c_str());
    return;
  }
  int shown = 0;
  for (const probe::Detection& d : p.detections) {
    if (shown++ >= args.top) break;
    std::printf("  %-14s %.3f  box y %.3f..%.3f x %.3f..%.3f", labels.Name(d.cls).c_str(), d.score, d.ymin, d.ymax,
                d.xmin, d.xmax);
    if (src.w > 0) {
      std::printf("  (pixels x %d..%d y %d..%d)", static_cast<int>(d.xmin * src.w), static_cast<int>(d.xmax * src.w),
                  static_cast<int>(d.ymin * src.h), static_cast<int>(d.ymax * src.h));
    }
    std::printf("\n");
  }
  if (p.detections.empty()) std::printf("  no detection at or above %.2f\n", args.min_score);
  else if (static_cast<int>(p.detections.size()) > args.top)
    std::printf("  (%zu detections at or above %.2f, top %d shown)\n", p.detections.size(), args.min_score, args.top);
}

// Detector outputs are cut at a count and sorted by score, so a small change in a score moves rows and the raw tensors differ a lot while the
// detections are the same. Compares the decoded lists: every detection a list holds with a score at least 0.1 above the threshold has a partner of
// the same class in the other list (intersection over union at least 0.5), and partners' scores are within 0.15.
float BoxIou(const probe::Detection& a, const probe::Detection& b) {
  const float inter = std::max(0.f, std::min(a.ymax, b.ymax) - std::max(a.ymin, b.ymin)) * std::max(0.f, std::min(a.xmax, b.xmax) - std::max(a.xmin, b.xmin));
  const float uni = (a.ymax - a.ymin) * (a.xmax - a.xmin) + (b.ymax - b.ymin) * (b.xmax - b.xmin) - inter;
  return uni > 0 ? inter / uni : 0.f;
}

bool Covered(const std::vector<probe::Detection>& from, const std::vector<probe::Detection>& to, float strong, const char* from_name, std::string* why) {
  for (const probe::Detection& d : from) {
    if (d.score < strong) continue;
    bool found = false;
    for (const probe::Detection& e : to)
      if (e.cls == d.cls && BoxIou(d, e) >= 0.5f && std::fabs(e.score - d.score) <= 0.15f) found = true;
    if (!found) {
      char buf[160];
      std::snprintf(buf, sizeof buf, "class %d score %.2f box y %.2f..%.2f x %.2f..%.2f of the %s has no partner in the other", d.cls, d.score, d.ymin, d.ymax, d.xmin, d.xmax, from_name);
      *why = buf;
      return false;
    }
  }
  return true;
}

bool SameDetections(const std::vector<probe::Detection>& cpu, const std::vector<probe::Detection>& dlg, float min_score, std::string* why) {
  const float strong = min_score + 0.1f;
  return Covered(cpu, dlg, strong, "CPU's list", why) && Covered(dlg, cpu, strong, "delegate's list", why);
}

// Where an accelerator's outputs stand against the CPU's: "identical"; "close" (every tensor within 2% of its largest CPU value, a
// quantisation step or two; or, for a detector, the same detections); "zero" (all zero while the CPU's are not: a device that returned
// without computing); "differ". "n/a" when there is nothing to compare with. An accelerator's time means nothing if it did not compute the answer.
std::string CompareOutputs(const Phase& cpu, const Phase& dlg, bool applied, float min_score) {
  if (!cpu.ran || !dlg.ran) return "n/a";
  if (cpu.outputs.size() != dlg.outputs.size()) {
    std::printf("  outputs: different tensor counts\n");
    return "differ";
  }
  double worst = 0, worst_ratio = 0;
  std::string worst_name;
  size_t differing = 0, total = 0;
  bool cpu_zero = true, dlg_zero = true;
  for (size_t k = 0; k < cpu.outputs.size(); ++k) {
    const auto& a = cpu.outputs[k];
    const auto& b = dlg.outputs[k];
    if (a.size() != b.size()) {
      std::printf("  outputs: tensor %s changed size\n", cpu.output_names[k].c_str());
      return "differ";
    }
    double range = 0, tensor_worst = 0;
    for (size_t i = 0; i < a.size(); ++i) {
      double d = std::fabs(static_cast<double>(a[i]) - b[i]);
      ++total;
      if (d > 0) ++differing;
      if (d > worst) {
        worst = d;
        worst_name = cpu.output_names[k];
      }
      tensor_worst = std::max(tensor_worst, d);
      range = std::max(range, static_cast<double>(std::fabs(a[i])));
      if (a[i] != 0) cpu_zero = false;
      if (b[i] != 0) dlg_zero = false;
    }
    worst_ratio = std::max(worst_ratio, tensor_worst / std::max(range, 1e-9));
  }
  if (worst == 0) {
    std::printf("  outputs vs CPU: identical (%zu values)\n", total);
    return "identical";
  }
  std::string verdict = (dlg_zero && !cpu_zero) ? "zero" : worst_ratio <= 0.02 ? "close" : "differ";
  std::printf("  outputs vs CPU: max |difference| %.6g in %s, %zu of %zu values differ (%s)\n", worst, worst_name.c_str(),
              differing, total, verdict.c_str());
  if (verdict == "differ" && cpu.detections_ok && dlg.detections_ok) {
    std::string why;
    if (SameDetections(cpu.detections, dlg.detections, min_score, &why)) {
      std::printf("  detections vs CPU: the same (%zu against %zu at or above %.2f): the raw tensors differ because the rows are sorted by score\n",
                  cpu.detections.size(), dlg.detections.size(), min_score);
      verdict = "close";
    } else {
      std::printf("  detections vs CPU: %s\n", why.c_str());
    }
  }
  // With no node taken the second phase is the CPU again (builtin kernels against the first phase's XNNPACK): a difference says nothing
  // about the accelerator, and the "NOT APPLIED" line already says what the run was.
  if (verdict == "zero" && applied)
    std::printf("  WARNING: every output value is zero: the accelerator returned without computing, so its time is not a measurement of this model\n");
  else if (verdict == "differ" && applied)
    std::printf("  WARNING: the accelerator's outputs are not the CPU's: its time is not a measurement of this model\n");
  return verdict;
}


// A plugin that crashes takes the probe with it, and the TV has no debugger: say which module and function the
// faulting address is in (module offsets can be looked up in the library's symbol table offline).
void CrashHandler(int sig, siginfo_t* info, void* context) {
  uintptr_t pc = 0, lr = 0;
  const ucontext_t* uc = static_cast<const ucontext_t*>(context);
#if defined(__arm__)
  pc = uc->uc_mcontext.arm_pc;
  lr = uc->uc_mcontext.arm_lr;
#elif defined(__x86_64__)
  pc = static_cast<uintptr_t>(uc->uc_mcontext.gregs[REG_RIP]);
#endif
  auto where = [](uintptr_t addr, char* buf, size_t n) {
    Dl_info di;
    if (addr != 0 && (dladdr(reinterpret_cast<void*>(addr), &di) || dladdr(reinterpret_cast<void*>(addr | 1), &di)) && di.dli_fname) {
      std::snprintf(buf, n, "%s+0x%lx (%s)", di.dli_fname, static_cast<unsigned long>(addr - reinterpret_cast<uintptr_t>(di.dli_fbase)),
                    di.dli_sname ? di.dli_sname : "no symbol");
    } else {
      std::snprintf(buf, n, "0x%lx (unmapped)", static_cast<unsigned long>(addr));
    }
  };
  char a[512], b[512];
  where(pc, a, sizeof a);
  where(lr, b, sizeof b);
  std::fprintf(stderr, "\nCRASH: signal %d, fault address %p\n  pc  %s\n  lr  %s\n", sig, info->si_addr, a, b);
  std::fflush(stderr);
  _exit(128 + sig);
}

void InstallCrashReporter() {
  struct sigaction sa = {};
  sa.sa_sigaction = CrashHandler;
  sa.sa_flags = SA_SIGINFO;
  sigaction(SIGSEGV, &sa, nullptr);
  sigaction(SIGBUS, &sa, nullptr);
  sigaction(SIGILL, &sa, nullptr);
  sigaction(SIGABRT, &sa, nullptr);
}

}  // namespace

// ---------------------------------------------------------------------------------------------------------
// --system-info: look at what the device ships, without running anything of it

// The first TFLite runtime library the loader finds, loaded RTLD_LOCAL so its symbols stay out of this process's way.
void* OpenFirst(const char* const* names, std::string* found) {
  for (; *names != nullptr; ++names) {
    void* h = dlopen(*names, RTLD_LAZY | RTLD_LOCAL);
    if (h != nullptr) {
      *found = *names;
      return h;
    }
  }
  return nullptr;
}

int SystemInfo(const Args& args) {
  std::printf("probe runtime: TFLite %s (what the delegate must be built against to run in this process)\n", TfLiteVersion());
  static const char* const runtimes[] = {"libtensorflow-lite.so", "libtensorflowlite.so", "libtensorflow-lite.so.1",
                                         "/usr/lib/libtensorflow-lite.so", nullptr};
  std::string name;
  void* rt = OpenFirst(runtimes, &name);
  std::string device_version;
  if (rt == nullptr) {
    std::printf("device runtime: libtensorflow-lite.so not loadable (%s)\n", dlerror());
  } else {
    using VersionFn = const char* (*)();
    auto version = reinterpret_cast<VersionFn>(dlsym(rt, "TfLiteVersion"));
    device_version = version != nullptr ? version() : "";
    std::printf("device runtime: %s, TfLiteVersion() = %s\n", name.c_str(), version != nullptr ? device_version.c_str() : "(not exported)");
    static const char* const symbols[] = {"TfLiteDelegateCreate", "TfLiteIntArrayCreate", "TfLiteIntArrayFree",
                                          "_ZN6tflite8Subgraph28SetCustomAllocationForTensorEiRK22TfLiteCustomAllocationx",
                                          "_ZN6tflite11Interpreter23ModifyGraphWithDelegateEP14TfLiteDelegate",
                                          "TfLiteInterpreterCreate", nullptr};
    for (const char* const* s = symbols; *s != nullptr; ++s)
      std::printf("  %-90s %s\n", *s, dlsym(rt, *s) != nullptr ? "exported" : "NOT exported");
  }
  static const char* const xnn[] = {"libXNNPACK.so", nullptr};
  std::string xnn_name;
  void* x = OpenFirst(xnn, &xnn_name);
  std::printf("device XNNPACK: %s\n", x != nullptr ? xnn_name.c_str() : "libXNNPACK.so not loadable");

  // The other route to a TFLite model: the NNAPI delegate over a vendor HAL (libwebos_nnhal.so is one). Presence only.
  static const char* const nnapi[] = {"libneuralnetworks.so", "libneuralnetworks.so.1", nullptr};
  std::string nnapi_name;
  void* nn = OpenFirst(nnapi, &nnapi_name);
  std::printf("device NNAPI runtime: %s", nn != nullptr ? nnapi_name.c_str() : "libneuralnetworks.so not loadable");
  if (nn != nullptr) std::printf("; ANeuralNetworksModel_create %s", dlsym(nn, "ANeuralNetworksModel_create") ? "exported" : "missing");
  std::printf("\n");

  const std::string delegate = args.delegate.empty() ? "libtflite-npu-delegate.so.1" : args.delegate;
  void* d = dlopen(delegate.c_str(), RTLD_LAZY | RTLD_LOCAL);
  if (d == nullptr) {
    std::printf("delegate: %s not loadable (%s)\n", delegate.c_str(), dlerror());
  } else {
    std::printf("delegate: %s loads (lazy binding); tflite_plugin_create_delegate %s, tflite_plugin_destroy_delegate %s\n",
                delegate.c_str(), dlsym(d, "tflite_plugin_create_delegate") ? "present" : "missing",
                dlsym(d, "tflite_plugin_destroy_delegate") ? "present" : "missing");
  }
  auto major_minor = [](const std::string& v) { return v.substr(0, v.find('.', v.find('.') + 1)); };
  if (!device_version.empty()) {
    const bool match = major_minor(device_version) == major_minor(TfLiteVersion());
    std::printf("ABI: device %s, probe %s: %s\n", device_version.c_str(), TfLiteVersion(),
                match ? "same minor release, the delegate can run in this probe" : "DIFFERENT minor release: LG's delegate may misread the structures this probe passes it");
  }
  const std::string utilization = probe::ReadSmall(args.proc_dir + "/npu_utilization");
  if (utilization.empty()) {
    std::printf("npu: %s/npu_utilization is not readable here\n", args.proc_dir.c_str());
  } else {
    std::printf("npu: %s/npu_utilization = %s, npu_process_list %s\n", args.proc_dir.c_str(), utilization.c_str(),
                probe::ReadSmall(args.proc_dir + "/npu_process_list").empty() ? "is empty (nothing holds the NPU)" : "is NOT EMPTY");
  }
  return 0;
}

static void PrintNnapiInfo(const probe::NnapiInfo& n) {
  if (!n.library) {
    std::printf("  NNAPI runtime: %s\n", n.error.c_str());
    return;
  }
  std::printf("  NNAPI runtime: libneuralnetworks.so loaded, feature level %lld; TFLite derives Android level %d from the functions it exports (27 = NNAPI 1.0, 29 = 1.2, 30 = 1.3)\n",
              static_cast<long long>(n.runtime_feature_level), n.tflite_sdk_level);
  if (!n.error.empty()) std::printf("  NNAPI devices: %s\n", n.error.c_str());
  else std::printf("  NNAPI devices (%zu):%s\n", n.devices.size(), n.devices.empty() ? " none" : "");
  for (size_t i = 0; i < n.devices.size(); ++i) {
    const probe::NnapiDevice& d = n.devices[i];
    std::printf("    [%zu] %s  type=%s version=%s feature level %lld\n", i, d.name.c_str(), probe::NnapiDeviceType(d.type),
                d.version.empty() ? "?" : d.version.c_str(), static_cast<long long>(d.feature_level));
  }
}

static int NnapiList() {
  PrintNnapiInfo(probe::QueryNnapi());
  return 0;
}

// --gpu-info: what the device's OpenCL offers, the Mali counters, the delegate on hand. Read only; 3 when no device is listed.
static int GpuInfo(const Args& args) {
  const probe::OpenClInfo cl = probe::QueryOpenCl(args.opencl_lib, args.opencl_icd);
  probe::PrintOpenClInfo(cl);
  const probe::MaliCounters mali = probe::ReadMaliCounters(args.mali_proc);
  std::printf("Mali counters %s: %s\n", args.mali_proc.c_str(),
              mali.ok ? "readable (busy_time and idle_time)" : "not readable (debugfs not mounted, or not root)");
  std::printf("GPU delegate: TFLite %s GPU delegate V2 built into the probe, OpenCL backend only\n", TfLiteVersion());
  return probe::HasUsableDevice(cl) ? 0 : 3;
}

// The nodes of the model the GPU delegate takes, by the delegate's own selection code (needs no device).
static bool ReportGpuCoverage(const Args& args, const ModelFacts& facts, bool opencl) {
  probe::CaptureReporter reporter;
  auto model = tflite::FlatBufferModel::BuildFromFile(args.model.c_str(), &reporter);
  Interp it;
  std::string error;
  if (!model || !Build(*model, 1, facts.custom_ops, &it, &error)) {
    std::printf("  coverage not computed: %s\n", model ? error.c_str() : "cannot load the model");
    return false;
  }
  probe::GpuCoverage cov;
  if (!probe::ComputeGpuCoverage(*it.interp, args.gpu_opts, opencl, reporter, &cov, &error)) {
    std::printf("  coverage not computed: %s\n", error.c_str());
    return false;
  }
  probe::PrintGpuCoverage(cov);
  return true;
}

// Busy share of the Mali over |seconds| with nothing of the probe's running: what the UI and LG's services leave.
static double MaliIdleShare(const Args& args, double seconds) {
  const probe::MaliCounters a = probe::ReadMaliCounters(args.mali_proc);
  if (!a.ok) return -1;
  usleep(static_cast<useconds_t>(seconds * 1e6));
  return probe::MaliBusyPercent(a, probe::ReadMaliCounters(args.mali_proc));
}

// What the kernel cache directory holds after a run.
static void PrintKernelCache(const std::string& dir) {
  DIR* d = opendir(dir.c_str());
  if (d == nullptr) return;
  int files = 0;
  long long bytes = 0;
  while (const dirent* e = readdir(d)) {
    if (e->d_name[0] == '.') continue;
    struct stat st;
    if (stat((dir + "/" + e->d_name).c_str(), &st) == 0 && S_ISREG(st.st_mode)) {
      ++files;
      bytes += st.st_size;
    }
  }
  closedir(d);
  std::printf("  kernel cache %s: %d file(s), %lld KiB\n", dir.c_str(), files, bytes >> 10);
}


int main(int argc, char** argv) {
  InstallCrashReporter();
  Args args;
  if (!ParseArgs(argc, argv, &args)) {
    Usage();
    return 2;
  }
  std::setvbuf(stdout, nullptr, _IOLBF, 0);

  std::printf("lumatv-npu-probe %s  (TFLite runtime %s, built %s, %zu-bit)\n", kVersion, TfLiteVersion(), __DATE__,
              sizeof(void*) * 8);
  if (args.system_info) return SystemInfo(args);
  if (args.nnapi_list) return NnapiList();
  if (args.gpu_info) return GpuInfo(args);

  auto model = tflite::FlatBufferModel::BuildFromFile(args.model.c_str());
  if (!model) {
    std::fprintf(stderr, "cannot load model %s\n", args.model.c_str());
    return 1;
  }
  ModelFacts facts = ReadFacts(*model->GetModel());
  std::printf("model: %s  \"%s\", %d ops\n", args.model.c_str(), facts.description.c_str(), facts.op_count);
  if (!facts.converter.empty() || !facts.min_runtime.empty())
    std::printf("  written by TF %s, needs runtime >= %s\n", facts.converter.empty() ? "?" : facts.converter.c_str(),
                facts.min_runtime.empty() ? "?" : facts.min_runtime.c_str());
  std::printf("  ops: %s\n", HistogramString(facts.ops).c_str());
  if (facts.npu_nodes > 0)
    std::printf("  NPU: %d %s node(s), %zu bytes of compiled package: LG's delegate can take this model\n",
                facts.npu_nodes, kLgCustomOp, facts.npu_payload_bytes);
  else
    std::printf("  NPU: no %s node: LG's delegate refuses a graph without one\n", kLgCustomOp);
  if (!facts.custom_ops.empty()) {
    std::printf("  custom ops:");
    for (const auto& c : facts.custom_ops) std::printf(" %s", c.c_str());
    std::printf("\n");
  }

  Interp shape;
  std::string error;
  if (!Build(*model, args.threads, facts.custom_ops, &shape, &error)) {
    std::fprintf(stderr, "%s\n", error.c_str());
    return 1;
  }
  PrintIo(*shape.interp);
  if (args.info) return 0;
  if (args.gpu_coverage) {
    const probe::OpenClInfo cl = probe::QueryOpenCl(args.opencl_lib, args.opencl_icd);
    std::printf("GPU delegate coverage (the delegate's own selection code; an OpenCL device is %s here, no run is made)\n",
                probe::HasUsableDevice(cl) ? "present" : "absent");
    std::printf("  %s\n", probe::DescribeGpuOptions(args.gpu_opts).c_str());
    return ReportGpuCoverage(args, facts, probe::HasUsableDevice(cl)) ? 0 : 1;
  }

  // ---- input picture
  probe::Image src, fitted;
  const TfLiteTensor* in0 = shape.interp->tensor(shape.interp->inputs()[0]);
  int in_h = 1, in_w = 1;
  if (in0->dims->size == 4) {
    in_h = in0->dims->data[1];
    in_w = in0->dims->data[2];
  }
  if (args.fill == "image") {
    if (!args.image.empty()) {
      if (!probe::LoadImage(args.image, &src, &error)) {
        std::fprintf(stderr, "%s\n", error.c_str());
        return 1;
      }
      std::printf("image: %s %dx%d -> %dx%d (stretched)\n", args.image.c_str(), src.w, src.h, in_w, in_h);
    } else {
      src = probe::TestPattern(in_w, in_h);
      std::printf("image: built-in test pattern %dx%d\n", in_w, in_h);
    }
    fitted = probe::ResizeBilinear(src, in_w, in_h);
  } else if (args.fill == "pattern") {
    fitted = probe::TestPattern(in_w, in_h);
    std::printf("image: built-in test pattern %dx%d\n", in_w, in_h);
  } else {
    std::printf("input: every element %s\n", args.fill.c_str() + 6);
    fitted.w = in_w;
    fitted.h = in_h;
  }
  (void)src;

  probe::Labels labels = probe::Labels::Coco();
  if (!args.labels.empty() && !labels.LoadFile(args.labels, &error)) {
    std::fprintf(stderr, "%s\n", error.c_str());
    return 1;
  }

  const std::string npu_caps = probe::ReadSmall(args.proc_dir + "/npu_device_capability");
  if (!npu_caps.empty()) std::printf("npu driver:\n%s", npu_caps.c_str());
  std::printf("runs: %d timed + %d warm-up per phase, %d CPU thread(s)\n", args.runs, args.warmup, args.threads);

  Phase cpu, dlg;

  // ---- phase 1: no delegate
  if (args.cpu_backend != "none" && facts.npu_nodes > 0) {
    cpu.note = std::string("the model's ") + kLgCustomOp + " has no CPU kernel: it runs only inside its delegate";
    std::printf("\n== CPU, no delegate: skipped, %s\n", cpu.note.c_str());
  } else if (args.cpu_backend != "none") {
    std::printf("\n== CPU, no delegate (%s kernels)\n", args.cpu_backend == "xnnpack" ? "XNNPACK" : "builtin");
    Interp it;
    if (!Build(*model, args.threads, facts.custom_ops, &it, &error)) {
      cpu.note = error;
    } else {
      bool ok = true;
      if (args.cpu_backend == "xnnpack") {
        TfLiteXNNPackDelegateOptions xo = TfLiteXNNPackDelegateOptionsDefault();
        xo.num_threads = args.threads;
        xo.flags |= TFLITE_XNNPACK_DELEGATE_FLAG_QS8 | TFLITE_XNNPACK_DELEGATE_FLAG_QU8;
        it.xnn.reset(TfLiteXNNPackDelegateCreate(&xo));
        TfLiteStatus st = it.interp->ModifyGraphWithDelegate(it.xnn.get());
        if (st != kTfLiteOk) {
          cpu.note = std::string("XNNPACK refused the graph (") + StatusName(st) + "), running builtin kernels";
          std::printf("  %s\n", cpu.note.c_str());
        } else {
          auto plan = Plan(*it.interp);
          int kernels = 0;
          for (const PlanOp& p : plan) kernels += p.delegate_kernel;
          std::printf("  XNNPACK: %d delegate kernel(s), %zu node(s) left on builtin kernels\n", kernels, plan.size() - kernels);
        }
      } else {
        it.interp->SetNumThreads(args.threads);
      }
      if (it.interp->AllocateTensors() != kTfLiteOk) {
        cpu.note = "AllocateTensors failed (model has ops this runtime cannot run without a delegate)";
        ok = false;
      }
      if (ok && !FillInput(it.interp.get(), fitted, args, &error)) {
        cpu.note = error;
        ok = false;
      }
      if (ok && TimedRuns(it.interp.get(), args, &cpu, &error)) {
        cpu.ran = true;
        CaptureOutputs(*it.interp, &cpu);
        cpu.detections_ok = probe::DecodeDetections(*it.interp, args.min_score, &cpu.detections, &cpu.detections_why);
        PrintStats(cpu);
        if (args.print_output) PrintOutputs(*it.interp);
        std::printf("  top detections:\n");
        if (cpu.detections_ok) {
          ShowDetections(cpu, args, labels, src);
        } else {
          std::printf("  not a post-processed detector output (%s); raw outputs:\n", cpu.detections_why.c_str());
          for (const auto& l : probe::DescribeOutputs(*it.interp)) std::printf("    %s\n", l.c_str());
        }
      } else if (ok) {
        cpu.note = error;
      }
    }
    if (!cpu.ran) std::printf("  CPU phase did not run: %s\n", cpu.note.c_str());
  }

  // ---- phase 2: through an accelerator route: LG's external delegate plugin, TFLite's NNAPI delegate over the NN HAL, or TFLite's GPU delegate
  const bool route = !args.delegate.empty() || args.nnapi || args.gpu;
  const char* const key = args.nnapi ? "nnapi" : args.gpu ? "gpu" : "delegate";  // SUMMARY prefix
  const char* const label = args.nnapi ? "NNAPI delegate" : args.gpu ? "GPU delegate" : "delegate";
  double mali_idle = -1;
  if (route) {
    if (args.nnapi) {
      std::printf("\n== NNAPI delegate (%s)\n", probe::DescribeNnapiOptions(args.nnapi_opts).c_str());
    } else if (args.gpu) {
      std::printf("\n== GPU delegate (%s)\n", probe::DescribeGpuOptions(args.gpu_opts).c_str());
    } else {
      std::printf("\n== delegate %s", args.delegate.c_str());
      for (const auto& kv : args.delegate_opts) std::printf(" %s=%s", kv.first.c_str(), kv.second.c_str());
      std::printf("\n");
    }
    const std::string procs_before = probe::ReadSmall(args.proc_dir + "/npu_process_list");
    probe::ExternalDelegate plugin;  // declared before the interpreter: destroyed after it
    probe::NnapiDelegate nnapi;
    probe::GpuDelegate gpu;  // likewise
    TfLiteDelegate* dg = nullptr;
    Interp it;
    bool ok = true;
    if (args.nnapi) {
      PrintNnapiInfo(probe::QueryNnapi());
      if (!nnapi.Create(args.nnapi_opts, &error)) {
        dlg.note = "NNAPI delegate not created: " + error;
        ok = false;
      }
      dg = nnapi.get();
    } else if (args.gpu) {
      if (args.gpu_opts.lib.empty()) {
        const probe::OpenClInfo cl = probe::QueryOpenCl(args.opencl_lib, args.opencl_icd);
        probe::PrintOpenClInfo(cl);
        if (!probe::HasUsableDevice(cl)) std::printf("  no OpenCL device: the delegate cannot start; it is tried all the same so that its own message is shown\n");
        std::printf("  GPU delegate coverage (its own selection code; it does not need the device):\n");
        if (!ReportGpuCoverage(args, facts, probe::HasUsableDevice(cl))) std::printf("  (coverage unavailable, the run goes on)\n");
      }
      mali_idle = MaliIdleShare(args, 1.0);
      if (mali_idle >= 0) std::printf("  Mali busy with nothing of the probe's running: %.1f%% (1 s)\n", mali_idle);
      if (!gpu.Create(args.gpu_opts, args.model, &error)) {
        dlg.note = "GPU delegate not created: " + error;
        ok = false;
      } else {
        std::printf("  delegate: %s\n", gpu.source().c_str());
      }
      dg = gpu.get();
    } else {
      if (!plugin.Load(args.delegate, args.delegate_opts)) {
        dlg.note = "delegate not created: " + plugin.error();
        ok = false;
      }
      for (const auto& m : plugin.plugin_messages()) std::printf("  plugin said: %s\n", m.c_str());
      dg = plugin.get();
    }
    if (ok && !Build(*model, args.threads, facts.custom_ops, &it, &error)) {
      dlg.note = error;
      ok = false;
    }
    if (ok) {
      const double init0 = probe::WallSeconds();
      const std::vector<PlanOp> before = Plan(*it.interp);
      const TfLiteStatus st = it.interp->ModifyGraphWithDelegate(dg);
      if (st != kTfLiteOk) {
        dlg.note = std::string(label) + " refused the graph: ModifyGraphWithDelegate returned " + StatusName(st) + " (" +
                   std::to_string(static_cast<int>(st)) + ")";
        std::printf("  %s; the interpreter is back to its pre-delegation state\n", dlg.note.c_str());
        if (args.nnapi) std::printf("  (TFLite's reason is on stderr, above; NNAPI errno %d)\n", nnapi.Errno());
        else if (args.gpu) std::printf("  (TFLite's reason is on stderr, above: with no OpenCL driver the delegate stops while it builds its kernels)\n");
        else std::printf("  (the plugin logs its reason to PmLog, not stderr: see /var/log/messages)\n");
      } else {
        const std::vector<PlanOp> after = Plan(*it.interp);
        int kernels = 0;
        for (const PlanOp& p : after) kernels += p.delegate_kernel;
        const int left = static_cast<int>(after.size()) - kernels;
        const int taken = static_cast<int>(before.size()) - left;
        std::map<std::string, int> absorbed = Histogram(before, false);
        for (const auto& kv : Histogram(after, false)) {
          absorbed[kv.first] -= kv.second;
          if (absorbed[kv.first] <= 0) absorbed.erase(kv.first);
        }
        dlg.delegate_applied = kernels > 0;
        std::printf("  ModifyGraphWithDelegate: ok. nodes before %zu, after %zu: %d delegate kernel(s) took %d node(s), %d left on the CPU\n",
                    before.size(), after.size(), kernels, taken, left);
        if (kernels > 0) {
          std::printf("  delegated ops: %s\n", HistogramString(absorbed).c_str());
          for (const PlanOp& p : after)
            if (p.delegate_kernel) std::printf("  - %s (%d input tensor(s), %d output tensor(s))\n", p.name.c_str(), p.inputs, p.outputs);
        } else {
          dlg.note = std::string("the ") + label + " accepted the call but claimed no node";
          std::printf("  %s\n", dlg.note.c_str());
          if (args.nnapi)
            std::printf("  (NNAPI leaves a node to the CPU unless the chosen device reports its operation as supported: compare the device list above with the ops of the model)\n");
        }
      }
      if (it.interp->AllocateTensors() != kTfLiteOk) {
        dlg.note = "AllocateTensors failed after delegation";
        ok = false;
      }
      dlg.init_ms = (probe::WallSeconds() - init0) * 1e3;
      if (ok && !FillInput(it.interp.get(), fitted, args, &error)) {
        dlg.note = error;
        ok = false;
      }
      if (ok && TimedRuns(it.interp.get(), args, &dlg, &error)) {
        dlg.ran = true;
        CaptureOutputs(*it.interp, &dlg);
        dlg.detections_ok = probe::DecodeDetections(*it.interp, args.min_score, &dlg.detections, &dlg.detections_why);
        PrintStats(dlg);
        if (args.print_output) PrintOutputs(*it.interp);
        std::printf("  top detections:\n");
        if (dlg.detections_ok) ShowDetections(dlg, args, labels, src);
        else std::printf("  not a post-processed detector output (%s)\n", dlg.detections_why.c_str());
        dlg.verdict = CompareOutputs(cpu, dlg, dlg.delegate_applied, args.min_score);
        if (args.gpu && !args.gpu_opts.cache_dir.empty()) PrintKernelCache(args.gpu_opts.cache_dir);
      } else if (ok) {
        dlg.note = error;
      }
    }
    if (args.nnapi && nnapi.Errno() != 0) std::printf("  NNAPI errno after the run: %d\n", nnapi.Errno());
    const std::string procs_after = probe::ReadSmall(args.proc_dir + "/npu_process_list");
    if (!procs_before.empty() || !procs_after.empty()) {
      std::printf("  npu_process_list before the run: %s\n", procs_before.empty() ? "(empty)" : procs_before.c_str());
      std::printf("  npu_process_list after the run (%s still loaded): %s\n", label, procs_after.empty() ? "(empty)" : procs_after.c_str());
    }
    if (!dlg.ran) std::printf("  %s phase did not run to the end: %s\n", label, dlg.note.c_str());
    else if (!dlg.delegate_applied) std::printf("  %s NOT APPLIED: this run is the CPU path again (%s)\n", label, dlg.note.c_str());
  }

  // ---- summary
  std::printf("\nSUMMARY model=%s", args.model.c_str());
  if (cpu.ran) {
    Stats s = Summarize(cpu.ms);
    std::printf(" cpu_backend=%s cpu_mean_ms=%.3f cpu_median_ms=%.3f cpu_p95_ms=%.3f cpu_busy_pct=%.0f",
                args.cpu_backend.c_str(), s.mean, s.median, s.p95, cpu.cpu_percent);
    if (cpu.npu_sampled) std::printf(" cpu_npu_util_max=%d", cpu.npu.max);
    if (cpu.mali_percent >= 0) std::printf(" cpu_mali_pct=%.1f", cpu.mali_percent);
  } else {
    std::printf(" cpu_ran=0");
  }
  if (route) {
    std::printf(" %s_applied=%d", key, dlg.delegate_applied ? 1 : 0);
    if (dlg.ran) {
      Stats s = Summarize(dlg.ms);
      std::printf(" %s_mean_ms=%.3f %s_median_ms=%.3f %s_p95_ms=%.3f %s_busy_pct=%.0f", key, s.mean, key, s.median, key, s.p95, key,
                  dlg.cpu_percent);
      if (dlg.npu_sampled) std::printf(" %s_npu_util_max=%d %s_npu_util_mean=%.1f", key, dlg.npu.max, key, dlg.npu.mean);
      if (dlg.init_ms >= 0) std::printf(" %s_init_ms=%.1f", key, dlg.init_ms);
      if (dlg.first_ms >= 0) std::printf(" %s_first_ms=%.1f", key, dlg.first_ms);
      if (dlg.mali_percent >= 0) std::printf(" %s_mali_pct=%.1f", key, dlg.mali_percent);
      if (args.gpu && mali_idle >= 0 && dlg.mali_percent >= 0)
        std::printf(" mali_idle_pct=%.1f gpu_ms_per_inference=%.1f", mali_idle, std::max(0.0, (dlg.mali_percent - mali_idle) / 100.0 * s.mean));
      std::printf(" %s_outputs=%s", key, dlg.verdict.c_str());
    } else {
      std::printf(" %s_ran=0", key);
    }
  }
  std::printf("\n");

  if (args.require_delegate && (!dlg.delegate_applied || !dlg.ran)) return 3;
  if (args.require_delegate && (dlg.verdict == "zero" || dlg.verdict == "differ")) return 5;
  if (!cpu.ran && args.cpu_backend != "none" && !(dlg.ran)) return 4;
  return 0;
}
