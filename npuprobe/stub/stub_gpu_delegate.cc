// A stand-in for TFLite's GPU delegate V2 library, for host tests (--gpu-lib): the three entry points of
// tensorflow/lite/delegates/gpu/delegate.h, the real options struct, and the delegate's way of working (it takes the nodes of the operations
// it supports, the largest max_delegated_partitions pieces of them, and replaces them with kernels). No GPU: the kernels compute on the CPU
// by running the whole model in a nested interpreter, which gives the same outputs as the CPU phase, so a test can tell "took the nodes"
// from "computed nothing" (STUB_GPU_COMPUTE=none).
//
// There is no key/value plugin interface, so the switches are environment variables:
//   STUB_GPU_OPS=A,B,...|none   builtin operations the stub supports (default: the usual convolution-network set)
//   STUB_GPU_MODEL=FILE         the model the nested interpreter runs; without it the kernels compute nothing
//   STUB_GPU_COMPUTE=none       zero the outputs: a device that took the nodes and returned without computing
//   STUB_GPU_FAIL=create|prepare   TfLiteGpuDelegateV2Create returns null / the delegate refuses the graph the way the real one does
//                               without an OpenCL driver
//   STUB_GPU_DELAY_US=N         sleep per Invoke
//   STUB_GPU_LOG=FILE           one line appended per create, with the option struct as received
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include <algorithm>
#include <memory>
#include <set>
#include <string>
#include <vector>

#include "tensorflow/lite/builtin_ops.h"
#include "tensorflow/lite/c/common.h"
#include "tensorflow/lite/delegates/gpu/delegate_options.h"
#include "tensorflow/lite/interpreter.h"
#include "tensorflow/lite/kernels/register.h"
#include "tensorflow/lite/model.h"
#include "tensorflow/lite/schema/schema_generated.h"

#define EXPORT __attribute__((visibility("default")))

namespace {

struct StubGpu {
  TfLiteDelegate base;  // first: the runtime hands back &base
  TfLiteGpuDelegateOptionsV2 options;
};

struct Kernel {
  std::vector<int> outputs;
  long delay_us = 0;
  bool compute = true;
  std::unique_ptr<tflite::FlatBufferModel> model;
  std::unique_ptr<tflite::Interpreter> inner;
};

const char* Env(const char* name) {
  const char* v = getenv(name);
  return v != nullptr && *v != '\0' ? v : nullptr;
}

std::set<std::string> SupportedOps() {
  const char* v = Env("STUB_GPU_OPS");
  std::string list = v != nullptr ? v
                                  : "CONV_2D,DEPTHWISE_CONV_2D,ADD,MUL,CONCATENATION,RESHAPE,LOGISTIC,AVERAGE_POOL_2D,MAX_POOL_2D,PAD,"
                                    "RESIZE_NEAREST_NEIGHBOR,SOFTMAX,QUANTIZE,DEQUANTIZE,STRIDED_SLICE,TRANSPOSE";
  std::set<std::string> ops;
  if (list == "none") return ops;
  size_t pos = 0;
  while (pos <= list.size()) {
    size_t comma = list.find(',', pos);
    if (comma == std::string::npos) comma = list.size();
    if (comma > pos) ops.insert(list.substr(pos, comma - pos));
    pos = comma + 1;
  }
  return ops;
}

void* Init(TfLiteContext* context, const char* buffer, size_t) {
  const auto* params = reinterpret_cast<const TfLiteDelegateParams*>(buffer);
  auto k = std::make_unique<Kernel>();
  for (int i = 0; i < params->output_tensors->size; ++i) k->outputs.push_back(params->output_tensors->data[i]);
  if (const char* d = Env("STUB_GPU_DELAY_US")) k->delay_us = strtol(d, nullptr, 10);
  const char* compute = Env("STUB_GPU_COMPUTE");
  k->compute = compute == nullptr || strcmp(compute, "none") != 0;
  const char* file = Env("STUB_GPU_MODEL");
  if (k->compute && file != nullptr) {
    tflite::ops::builtin::BuiltinOpResolverWithoutDefaultDelegates resolver;
    k->model = tflite::FlatBufferModel::BuildFromFile(file);
    if (!k->model) {
      TF_LITE_KERNEL_LOG(context, "StubGpuDelegate: cannot load STUB_GPU_MODEL=%s", file);
      return nullptr;
    }
    tflite::InterpreterBuilder(*k->model, resolver)(&k->inner);
    if (!k->inner || k->inner->AllocateTensors() != kTfLiteOk) {
      TF_LITE_KERNEL_LOG(context, "StubGpuDelegate: nested interpreter failed");
      return nullptr;
    }
  }
  return k.release();
}

void Free(TfLiteContext*, void* data) { delete static_cast<Kernel*>(data); }

TfLiteStatus KernelPrepare(TfLiteContext*, TfLiteNode*) { return kTfLiteOk; }

TfLiteStatus Invoke(TfLiteContext* context, TfLiteNode* node) {
  auto* k = static_cast<Kernel*>(node->user_data);
  if (k->delay_us > 0) usleep(static_cast<useconds_t>(k->delay_us));
  if (!k->inner) {
    // STUB_GPU_COMPUTE=none (or no model): the outputs come back zeroed, as from a device that returned without computing
    for (int idx : k->outputs) memset(context->tensors[idx].data.raw, 0, context->tensors[idx].bytes);
    return kTfLiteOk;
  }
  // The nested interpreter runs the whole model from the model's own inputs (same tensor indices as the outer graph), then the tensors
  // this kernel produces are copied out.
  for (int idx : k->inner->inputs()) {
    const TfLiteTensor* src = &context->tensors[idx];
    TfLiteTensor* dst = k->inner->tensor(idx);
    if (src->data.raw == nullptr || dst->bytes != src->bytes) return kTfLiteError;
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

std::string OpName(const TfLiteRegistration* reg) {
  if (reg->builtin_code == kTfLiteBuiltinCustom) return reg->custom_name != nullptr ? reg->custom_name : "custom";
  return tflite::EnumNameBuiltinOperator(static_cast<tflite::BuiltinOperator>(reg->builtin_code));
}

TfLiteStatus DelegatePrepare(TfLiteContext* context, TfLiteDelegate* delegate) {
  const auto* self = reinterpret_cast<const StubGpu*>(delegate);
  const char* fail = Env("STUB_GPU_FAIL");
  if (fail != nullptr && strcmp(fail, "prepare") == 0) {
    TF_LITE_KERNEL_LOG(context, "TfLiteGpuDelegate Prepare: delegate is not initialized");
    return kTfLiteDelegateError;
  }
  const std::set<std::string> supported = SupportedOps();
  TfLiteIntArray* plan = nullptr;
  if (context->GetExecutionPlan(context, &plan) != kTfLiteOk) return kTfLiteError;
  std::vector<int> candidates;
  for (int i = 0; i < plan->size; ++i) {
    TfLiteNode* node = nullptr;
    TfLiteRegistration* reg = nullptr;
    if (context->GetNodeAndRegistration(context, plan->data[i], &node, &reg) != kTfLiteOk) return kTfLiteError;
    if (supported.count(OpName(reg)) != 0) candidates.push_back(plan->data[i]);
  }
  std::vector<int> claim;
  if (!candidates.empty()) {
    TfLiteIntArray* all = TfLiteIntArrayCreate(static_cast<int>(candidates.size()));
    for (size_t i = 0; i < candidates.size(); ++i) all->data[i] = candidates[i];
    TfLiteDelegateParams* parts = nullptr;
    int n = 0;
    TfLiteStatus st = context->PreviewDelegatePartitioning(context, all, &parts, &n);
    TfLiteIntArrayFree(all);
    if (st != kTfLiteOk) return st;
    // The first max_delegated_partitions largest pieces, as GetNodesOfFirstNLargestPartitions does.
    std::vector<int> order(n);
    for (int i = 0; i < n; ++i) order[i] = i;
    std::stable_sort(order.begin(), order.end(), [&](int a, int b) { return parts[a].nodes_to_replace->size > parts[b].nodes_to_replace->size; });
    const int take = std::min(n, std::max(1, self->options.max_delegated_partitions));
    for (int i = 0; i < take; ++i)
      for (int j = 0; j < parts[order[i]].nodes_to_replace->size; ++j) claim.push_back(parts[order[i]].nodes_to_replace->data[j]);
  }
  if (claim.empty()) return kTfLiteOk;  // nothing the stub supports: the real delegate also leaves the graph alone
  TfLiteRegistration registration = {};
  registration.custom_name = "StubGpuDelegate";
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

}  // namespace

extern "C" {

// Upstream's defaults (tensorflow/lite/delegates/gpu/delegate_options.cc).
EXPORT TfLiteGpuDelegateOptionsV2 TfLiteGpuDelegateOptionsV2Default() {
  TfLiteGpuDelegateOptionsV2 options = {};
  options.is_precision_loss_allowed = 0;
  options.inference_preference = TFLITE_GPU_INFERENCE_PREFERENCE_FAST_SINGLE_ANSWER;
  options.inference_priority1 = TFLITE_GPU_INFERENCE_PRIORITY_MAX_PRECISION;
  options.inference_priority2 = TFLITE_GPU_INFERENCE_PRIORITY_AUTO;
  options.inference_priority3 = TFLITE_GPU_INFERENCE_PRIORITY_AUTO;
  options.experimental_flags = TFLITE_GPU_EXPERIMENTAL_FLAGS_ENABLE_QUANT;
  options.max_delegated_partitions = 1;
  options.serialization_dir = nullptr;
  options.model_token = nullptr;
  return options;
}

EXPORT TfLiteDelegate* TfLiteGpuDelegateV2Create(const TfLiteGpuDelegateOptionsV2* options) {
  const char* fail = Env("STUB_GPU_FAIL");
  if (fail != nullptr && strcmp(fail, "create") == 0) return nullptr;
  auto* d = new StubGpu;
  d->options = options != nullptr ? *options : TfLiteGpuDelegateOptionsV2Default();
  if (d->options.max_delegated_partitions <= 0) d->options.max_delegated_partitions = 1;
  if (const char* log = Env("STUB_GPU_LOG")) {
    if (FILE* f = fopen(log, "a")) {
      fprintf(f, "precision_loss=%d pref=%d p1=%d p2=%d p3=%d flags=%lld partitions=%d dir=%s token=%s\n", d->options.is_precision_loss_allowed,
              d->options.inference_preference, d->options.inference_priority1, d->options.inference_priority2, d->options.inference_priority3,
              static_cast<long long>(d->options.experimental_flags), d->options.max_delegated_partitions,
              d->options.serialization_dir != nullptr ? d->options.serialization_dir : "(none)",
              d->options.model_token != nullptr ? d->options.model_token : "(none)");
      fclose(f);
    }
  }
  d->base = TfLiteDelegateCreate();
  d->base.data_ = d;
  d->base.Prepare = &DelegatePrepare;
  d->base.flags = kTfLiteDelegateFlagsNone;
  return &d->base;
}

EXPORT void TfLiteGpuDelegateV2Delete(TfLiteDelegate* delegate) { delete reinterpret_cast<StubGpu*>(delegate); }

}  // extern "C"
