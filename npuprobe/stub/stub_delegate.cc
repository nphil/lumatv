// A TFLite external delegate for host tests. It has LG's plugin surface (libtflite-npu-delegate.so.1: the two
// tflite_plugin_* entry points, the option keys min_freq and use_custom_alloc, the custom-op-only claim rule, the
// "Nodes doesn't have customOp node for NPU" refusal) and leaves TFLite's C structures and
// tflite::Subgraph::SetCustomAllocationForTensor undefined, as LG's does, so loading it checks that the host exports
// them. No NPU: the work is done by the CPU.
//
// Options:
//   mode=claim_all   (default) take every node; needs model=FILE and runs the graph in a nested interpreter
//   mode=custom_op   take only lgnpu_custom_op nodes; payload = two floats (scale, bias), y = x*scale + bias
//   mode=refuse      Prepare fails
//   model=FILE       for claim_all
//   min_freq=N       integer, as LG's (parsed with strtol)
//   use_custom_alloc=true   give the graph's input and output tensors 64-byte aligned buffers through
//                    tflite::Subgraph::SetCustomAllocationForTensor, as LG's CustomOpKernel does
//   delay_us=N       sleep N microseconds per Invoke, to look like a slower or faster device
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include <cerrno>
#include <cstdio>
#include <memory>
#include <string>
#include <vector>

#include "tensorflow/lite/builtin_ops.h"
#include "tensorflow/lite/c/common.h"
#include "tensorflow/lite/core/subgraph.h"
#include "tensorflow/lite/interpreter.h"
#include "tensorflow/lite/kernels/register.h"
#include "tensorflow/lite/model.h"

#define EXPORT __attribute__((visibility("default")))

namespace {

enum class Mode { kClaimAll, kCustomOp, kRefuse };

struct Options {
  Mode mode = Mode::kClaimAll;
  std::string model;
  long min_freq = 0;
  bool custom_alloc = false;
  long delay_us = 0;
};

struct StubDelegate {
  TfLiteDelegate base;  // must stay first: the runtime hands back &base
  Options options;
};

struct Kernel {
  Options options;
  std::vector<int> inputs, outputs;
  std::unique_ptr<tflite::FlatBufferModel> model;
  std::unique_ptr<tflite::Interpreter> inner;
  float scale = 1, bias = 0;
  std::vector<void*> buffers;
  ~Kernel() {
    for (void* p : buffers) free(p);
  }
};

void* Init(TfLiteContext* context, const char* buffer, size_t) {
  const auto* params = reinterpret_cast<const TfLiteDelegateParams*>(buffer);
  const auto* delegate = reinterpret_cast<const StubDelegate*>(params->delegate);
  auto k = std::make_unique<Kernel>();
  k->options = delegate->options;
  for (int i = 0; i < params->input_tensors->size; ++i) k->inputs.push_back(params->input_tensors->data[i]);
  for (int i = 0; i < params->output_tensors->size; ++i) k->outputs.push_back(params->output_tensors->data[i]);

  if (k->options.mode == Mode::kCustomOp) {
    if (params->nodes_to_replace->size != 1) {
      TF_LITE_KERNEL_LOG(context, "StubDelegate: custom_op mode takes one node per partition, got %d",
                         params->nodes_to_replace->size);
      return nullptr;
    }
    TfLiteNode* node = nullptr;
    TfLiteRegistration* reg = nullptr;
    context->GetNodeAndRegistration(context, params->nodes_to_replace->data[0], &node, &reg);
    if (node->custom_initial_data == nullptr) {
      TF_LITE_KERNEL_LOG(context, "StubDelegate Init: custom_init data is not set.");
      return nullptr;
    }
    if (node->custom_initial_data_size >= 8) {
      const float* p = static_cast<const float*>(node->custom_initial_data);
      k->scale = p[0];
      k->bias = p[1];
    }
  } else {
    tflite::ops::builtin::BuiltinOpResolverWithoutDefaultDelegates resolver;
    k->model = tflite::FlatBufferModel::BuildFromFile(k->options.model.c_str());
    if (!k->model) {
      TF_LITE_KERNEL_LOG(context, "StubDelegate: cannot load model=%s", k->options.model.c_str());
      return nullptr;
    }
    tflite::InterpreterBuilder(*k->model, resolver)(&k->inner);
    if (!k->inner || k->inner->AllocateTensors() != kTfLiteOk) {
      TF_LITE_KERNEL_LOG(context, "StubDelegate: nested interpreter failed");
      return nullptr;
    }
  }
  return k.release();
}

void Free(TfLiteContext*, void* data) { delete static_cast<Kernel*>(data); }

TfLiteStatus KernelPrepare(TfLiteContext* context, TfLiteNode* node) {
  auto* k = static_cast<Kernel*>(node->user_data);
  if (k == nullptr) return kTfLiteError;
  if (k->options.custom_alloc) {
    auto* subgraph = static_cast<tflite::Subgraph*>(context->impl_);
    std::vector<int> io = k->inputs;
    io.insert(io.end(), k->outputs.begin(), k->outputs.end());
    for (int idx : io) {
      TfLiteTensor* t = &context->tensors[idx];
      if (t->allocation_type == kTfLiteMmapRo) continue;
      void* p = nullptr;
      size_t bytes = (t->bytes + 63) & ~static_cast<size_t>(63);
      if (posix_memalign(&p, 64, bytes ? bytes : 64) != 0) return kTfLiteError;
      k->buffers.push_back(p);
      TfLiteCustomAllocation alloc = {p, bytes};
      if (subgraph->SetCustomAllocationForTensor(idx, alloc) != kTfLiteOk) return kTfLiteError;
    }
  }
  return kTfLiteOk;
}

TfLiteStatus Invoke(TfLiteContext* context, TfLiteNode* node) {
  auto* k = static_cast<Kernel*>(node->user_data);
  if (k->options.delay_us > 0) usleep(static_cast<useconds_t>(k->options.delay_us));
  if (k->options.mode == Mode::kCustomOp) {
    if (k->inputs.size() != 1 || k->outputs.size() != 1) return kTfLiteError;
    const TfLiteTensor* in = &context->tensors[k->inputs[0]];
    TfLiteTensor* out = &context->tensors[k->outputs[0]];
    if (in->type != kTfLiteFloat32 || out->type != kTfLiteFloat32 || in->bytes != out->bytes) return kTfLiteError;
    for (size_t i = 0; i < in->bytes / sizeof(float); ++i) out->data.f[i] = in->data.f[i] * k->scale + k->bias;
    return kTfLiteOk;
  }
  // The partition is the whole graph and the nested interpreter was built from the same file, so tensor indices agree.
  for (int idx : k->inputs) {
    const TfLiteTensor* src = &context->tensors[idx];
    TfLiteTensor* dst = k->inner->tensor(idx);
    if (src->allocation_type == kTfLiteMmapRo) continue;  // weights: the nested interpreter has its own copy
    if (dst->bytes != src->bytes) return kTfLiteError;
    memcpy(dst->data.raw, src->data.raw, src->bytes);
  }
  if (k->inner->Invoke() != kTfLiteOk) return kTfLiteError;
  for (int idx : k->outputs) {
    const TfLiteTensor* src = k->inner->tensor(idx);
    TfLiteTensor* dst = &context->tensors[idx];
    if (dst->bytes != src->bytes) return kTfLiteError;
    memcpy(dst->data.raw, src->data.raw, src->bytes);
  }
  return kTfLiteOk;
}

TfLiteStatus DelegatePrepare(TfLiteContext* context, TfLiteDelegate* delegate) {
  const auto* self = reinterpret_cast<const StubDelegate*>(delegate);
  if (self->options.mode == Mode::kRefuse) return kTfLiteDelegateError;

  TfLiteIntArray* plan = nullptr;
  if (context->GetExecutionPlan(context, &plan) != kTfLiteOk) return kTfLiteError;
  std::vector<int> claim;
  for (int i = 0; i < plan->size; ++i) {
    const int node_index = plan->data[i];
    TfLiteNode* node = nullptr;
    TfLiteRegistration* reg = nullptr;
    if (context->GetNodeAndRegistration(context, node_index, &node, &reg) != kTfLiteOk) return kTfLiteError;
    if (self->options.mode == Mode::kCustomOp) {
      if (reg->builtin_code == kTfLiteBuiltinCustom && reg->custom_name != nullptr &&
          strcmp(reg->custom_name, "lgnpu_custom_op") == 0)
        claim.push_back(node_index);
    } else {
      claim.push_back(node_index);
    }
  }
  if (claim.empty()) {
    fprintf(stderr, "StubDelegate: PrepareImpl failed : Nodes doesn't have customOp node for NPU\n");
    return kTfLiteDelegateError;
  }
  TfLiteRegistration registration = {};
  registration.custom_name = "StubNpuDelegate";
  registration.version = 1;
  registration.init = Init;
  registration.free = Free;
  registration.prepare = KernelPrepare;
  registration.invoke = Invoke;
  TfLiteIntArray* nodes = TfLiteIntArrayCreate(static_cast<int>(claim.size()));
  for (size_t i = 0; i < claim.size(); ++i) nodes->data[i] = claim[i];
  TfLiteStatus st = context->ReplaceNodeSubsetsWithDelegateKernels(context, registration, nodes, delegate);
  TfLiteIntArrayFree(nodes);
  return st;
}

bool ParseLong(const char* s, long* out) {
  char* end = nullptr;
  errno = 0;
  long v = strtol(s, &end, 10);
  if (errno != 0 || end == s || *end != '\0') return false;
  *out = v;
  return true;
}

}  // namespace

extern "C" {

EXPORT TfLiteDelegate* tflite_plugin_create_delegate(char** keys, char** values, size_t n,
                                                     void (*report_error)(const char*)) {
  auto fail = [&](const std::string& why) -> TfLiteDelegate* {
    if (report_error != nullptr) report_error(why.c_str());
    return nullptr;
  };
  Options opt;
  for (size_t i = 0; i < n; ++i) {
    const std::string key = keys[i], value = values[i];
    if (key == "mode") {
      if (value == "claim_all") opt.mode = Mode::kClaimAll;
      else if (value == "custom_op") opt.mode = Mode::kCustomOp;
      else if (value == "refuse") opt.mode = Mode::kRefuse;
      else return fail("StubDelegateOptions: unknown mode " + value);
    } else if (key == "model") {
      opt.model = value;
    } else if (key == "min_freq") {
      if (!ParseLong(value.c_str(), &opt.min_freq)) return fail("StubDelegateOptions: min_freq is not an integer: " + value);
    } else if (key == "delay_us") {
      if (!ParseLong(value.c_str(), &opt.delay_us)) return fail("StubDelegateOptions: delay_us is not an integer: " + value);
    } else if (key == "use_custom_alloc") {
      opt.custom_alloc = value == "true";
    } else {
      return fail("Wrong key entered in StubDelegateOptions " + key);
    }
  }
  if (opt.mode == Mode::kClaimAll && opt.model.empty()) return fail("StubDelegateOptions: mode=claim_all needs model=FILE");
  auto* d = new StubDelegate;
  d->base = TfLiteDelegateCreate();
  d->base.data_ = d;
  d->base.Prepare = &DelegatePrepare;
  d->base.flags = kTfLiteDelegateFlagsNone;
  d->options = opt;
  return &d->base;
}

EXPORT void tflite_plugin_destroy_delegate(TfLiteDelegate* delegate) { delete reinterpret_cast<StubDelegate*>(delegate); }

}  // extern "C"
