// A stand-in for the Android NN runtime (libneuralnetworks.so) with devices that behave like an NN HAL, for the probe's NNAPI tests.
// It records the model TFLite's NNAPI delegate builds, says which operations a device supports, and "executes":
// ADD on float32 is computed for real (so data crossing the CPU/NNAPI border can be checked); every other supported operation is
// accepted and not computed, which is what LG's libwebos_nnhal does (its executeSynchronously sleeps 1 ms and reports that).
// Built twice: as it is, and with STUB_NNAPI_12_ONLY, which leaves out the 1.3 device feature-level call an Android 10 runtime lacks.
//
// Configuration, environment variables read at the first call:
//   STUB_NNAPI_DEVICES   name:type,...  type accelerator|cpu|gpu|other        (lgnpu-stub:accelerator,nnapi-reference:cpu)
//   STUB_NNAPI_OPS       all | none | ADD,CONV_2D,...  operations the accelerator devices support   (all); cpu devices support all
//   STUB_NNAPI_DELAY_US  time one execution takes                               (1000)
//   STUB_NNAPI_FAIL      compile | compute   make that step return an error     (none)
//   STUB_NNAPI_LOG=1     one line per model, compilation and execution on stderr
#include <fcntl.h>
#include <sys/mman.h>
#include <unistd.h>

#include <algorithm>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <map>
#include <memory>
#include <set>
#include <string>
#include <vector>

#include "tensorflow/lite/nnapi/NeuralNetworksTypes.h"

#define EXPORT extern "C" __attribute__((visibility("default")))

namespace {

struct Device {
  std::string name;
  std::string version = "stub-1.0";
  int32_t type = ANEURALNETWORKS_DEVICE_ACCELERATOR;
};

struct Config {
  std::vector<Device> devices;
  bool all_ops = true;
  std::set<int32_t> ops;
  int delay_us = 1000;
  std::string fail;
  bool log = false;
};

const std::map<std::string, int32_t>& OpNames() {
  static const std::map<std::string, int32_t> names = {
      {"ADD", ANEURALNETWORKS_ADD},
      {"CONCATENATION", ANEURALNETWORKS_CONCATENATION},
      {"CONV_2D", ANEURALNETWORKS_CONV_2D},
      {"DEPTHWISE_CONV_2D", ANEURALNETWORKS_DEPTHWISE_CONV_2D},
      {"PAD", ANEURALNETWORKS_PAD},
      {"QUANTIZE", ANEURALNETWORKS_QUANTIZE},
      {"RESIZE_NEAREST_NEIGHBOR", ANEURALNETWORKS_RESIZE_NEAREST_NEIGHBOR},
  };
  return names;
}

std::vector<std::string> Split(const std::string& s, char sep) {
  std::vector<std::string> out;
  size_t i = 0;
  while (i <= s.size()) {
    size_t j = s.find(sep, i);
    if (j == std::string::npos) j = s.size();
    if (j > i) out.push_back(s.substr(i, j - i));
    i = j + 1;
  }
  return out;
}

const Config& Cfg() {
  static const Config cfg = [] {
    Config c;
    const char* d = std::getenv("STUB_NNAPI_DEVICES");
    for (const std::string& item : Split(d != nullptr ? d : "lgnpu-stub:accelerator,nnapi-reference:cpu", ',')) {
      Device dev;
      const size_t colon = item.find(':');
      dev.name = item.substr(0, colon);
      const std::string type = colon == std::string::npos ? "accelerator" : item.substr(colon + 1);
      dev.type = type == "cpu" ? ANEURALNETWORKS_DEVICE_CPU : type == "gpu" ? ANEURALNETWORKS_DEVICE_GPU
                 : type == "other" ? ANEURALNETWORKS_DEVICE_OTHER : ANEURALNETWORKS_DEVICE_ACCELERATOR;
      c.devices.push_back(dev);
    }
    const char* ops = std::getenv("STUB_NNAPI_OPS");
    if (ops != nullptr && std::string(ops) != "all") {
      c.all_ops = false;
      for (const std::string& n : Split(ops, ',')) {
        auto it = OpNames().find(n);
        if (it != OpNames().end()) c.ops.insert(it->second);
        else if (n != "none") std::fprintf(stderr, "stub nnapi: unknown operation %s in STUB_NNAPI_OPS\n", n.c_str());
      }
    }
    if (const char* v = std::getenv("STUB_NNAPI_DELAY_US")) c.delay_us = std::atoi(v);
    if (const char* v = std::getenv("STUB_NNAPI_FAIL")) c.fail = v;
    if (const char* v = std::getenv("STUB_NNAPI_LOG")) c.log = v[0] == '1';
    return c;
  }();
  return cfg;
}

#define LOG(...) do { if (Cfg().log) { std::fprintf(stderr, "[stub nnapi] " __VA_ARGS__); std::fputc('\n', stderr); } } while (0)

// Devices have stable addresses: the runtime hands out pointers to them.
std::vector<std::unique_ptr<Device>>& Devices() {
  static std::vector<std::unique_ptr<Device>> devices = [] {
    std::vector<std::unique_ptr<Device>> v;
    for (const Device& d : Cfg().devices) v.push_back(std::make_unique<Device>(d));
    return v;
  }();
  return devices;
}

bool Supports(const Device& d, int32_t op) {
  if (d.type != ANEURALNETWORKS_DEVICE_ACCELERATOR && d.type != ANEURALNETWORKS_DEVICE_GPU) return true;
  return Cfg().all_ops || Cfg().ops.count(op) > 0;
}

}  // namespace

struct ANeuralNetworksMemory {
  int fd = -1;
  size_t size = 0, offset = 0;
  int prot = 0;
  uint8_t* map = nullptr;
  size_t map_len = 0, map_skew = 0;

  const uint8_t* Data() {
    if (map == nullptr) {
      const size_t page = static_cast<size_t>(sysconf(_SC_PAGESIZE));
      map_skew = offset % page;
      map_len = size + map_skew;
      void* p = mmap(nullptr, map_len, PROT_READ | PROT_WRITE, MAP_SHARED, fd, static_cast<off_t>(offset - map_skew));
      if (p == MAP_FAILED) p = mmap(nullptr, map_len, PROT_READ, MAP_SHARED, fd, static_cast<off_t>(offset - map_skew));
      if (p == MAP_FAILED) return nullptr;
      map = static_cast<uint8_t*>(p);
    }
    return map + map_skew;
  }
  ~ANeuralNetworksMemory() {
    if (map != nullptr) munmap(map, map_len);
    if (fd >= 0) close(fd);
  }
};

namespace {
struct Operand {
  int32_t type = 0;
  std::vector<uint32_t> dims;
  std::vector<uint8_t> value;  // a constant set inline
  ANeuralNetworksMemory* from = nullptr;  // or in a memory object
  size_t from_offset = 0, from_length = 0;
  bool constant = false;
};
struct Operation {
  int32_t type = 0;
  std::vector<uint32_t> in, out;
};
}  // namespace

struct ANeuralNetworksModel {
  std::vector<Operand> operands;
  std::vector<Operation> ops;
  std::vector<uint32_t> inputs, outputs;
  bool finished = false;
};
struct ANeuralNetworksCompilation {
  ANeuralNetworksModel* model = nullptr;
  std::vector<const Device*> devices;
  bool finished = false;
};
namespace {
struct Binding {
  void* buffer = nullptr;
  ANeuralNetworksMemory* memory = nullptr;
  size_t offset = 0, length = 0;
};
}  // namespace
struct ANeuralNetworksExecution {
  ANeuralNetworksCompilation* compilation = nullptr;
  std::map<uint32_t, Binding> in, out;
};
struct ANeuralNetworksEvent {};

namespace {

size_t Elements(const Operand& o) {
  size_t n = 1;
  for (uint32_t d : o.dims) n *= d;
  return n;
}

uint8_t* Resolve(const Binding& b) {
  if (b.buffer != nullptr) return static_cast<uint8_t*>(b.buffer);
  if (b.memory == nullptr) return nullptr;
  return const_cast<uint8_t*>(b.memory->Data()) + b.offset;
}

const uint8_t* ConstData(const Operand& o) {
  if (!o.value.empty()) return o.value.data();
  if (o.from != nullptr) {
    const uint8_t* base = o.from->Data();
    return base == nullptr ? nullptr : base + o.from_offset;
  }
  return nullptr;
}

// Runs what the stub can compute: float32 ADD. Everything else a device claims is accepted and left alone.
bool Compute(ANeuralNetworksExecution* e) {
  ANeuralNetworksModel* m = e->compilation->model;
  std::map<uint32_t, std::vector<uint8_t>> scratch;  // intermediate tensors
  auto data = [&](uint32_t idx, bool want_write) -> uint8_t* {
    for (size_t i = 0; i < m->inputs.size(); ++i)
      if (m->inputs[i] == idx) {
        auto it = e->in.find(static_cast<uint32_t>(i));
        return it == e->in.end() ? nullptr : Resolve(it->second);
      }
    for (size_t i = 0; i < m->outputs.size(); ++i)
      if (m->outputs[i] == idx) {
        auto it = e->out.find(static_cast<uint32_t>(i));
        return it == e->out.end() ? nullptr : Resolve(it->second);
      }
    const Operand& o = m->operands[idx];
    if (o.constant && !want_write) return const_cast<uint8_t*>(ConstData(o));
    std::vector<uint8_t>& s = scratch[idx];
    if (s.empty()) s.assign(Elements(o) * 4, 0);
    return s.data();
  };
  for (const Operation& op : m->ops) {
    if (op.type != ANEURALNETWORKS_ADD) {
      LOG("execution: operation %d accepted, not computed", op.type);
      continue;
    }
    const Operand& a = m->operands[op.in[0]];
    const Operand& b = m->operands[op.in[1]];
    const Operand& o = m->operands[op.out[0]];
    if (a.type != ANEURALNETWORKS_TENSOR_FLOAT32 || Elements(a) != Elements(b) || Elements(a) != Elements(o)) {
      LOG("execution: ADD of other shapes or types accepted, not computed");
      continue;
    }
    int32_t activation = 0;
    const uint8_t* act = ConstData(m->operands[op.in[2]]);
    if (act != nullptr) std::memcpy(&activation, act, sizeof(activation));
    const float* pa = reinterpret_cast<const float*>(data(op.in[0], false));
    const float* pb = reinterpret_cast<const float*>(data(op.in[1], false));
    float* po = reinterpret_cast<float*>(data(op.out[0], true));
    if (pa == nullptr || pb == nullptr || po == nullptr) return false;
    for (size_t i = 0, n = Elements(a); i < n; ++i) {
      float v = pa[i] + pb[i];
      if (activation == ANEURALNETWORKS_FUSED_RELU) v = std::max(v, 0.f);
      else if (activation == ANEURALNETWORKS_FUSED_RELU6) v = std::min(std::max(v, 0.f), 6.f);
      else if (activation == ANEURALNETWORKS_FUSED_RELU1) v = std::min(std::max(v, -1.f), 1.f);
      po[i] = v;
    }
  }
  if (Cfg().delay_us > 0) usleep(static_cast<useconds_t>(Cfg().delay_us));
  return true;
}

}  // namespace

// ---- NNAPI 1.0 / 1.1 ----------------------------------------------------------------------------------------------------------

EXPORT int ANeuralNetworksMemory_createFromFd(size_t size, int protect, int fd, size_t offset, ANeuralNetworksMemory** memory) {
  if (memory == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  auto* m = new ANeuralNetworksMemory;
  m->fd = dup(fd);
  m->size = size;
  m->offset = offset;
  m->prot = protect;
  *memory = m;
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT void ANeuralNetworksMemory_free(ANeuralNetworksMemory* memory) { delete memory; }

EXPORT int ANeuralNetworksModel_create(ANeuralNetworksModel** model) {
  if (model == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  *model = new ANeuralNetworksModel;
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT void ANeuralNetworksModel_free(ANeuralNetworksModel* model) { delete model; }
EXPORT int ANeuralNetworksModel_finish(ANeuralNetworksModel* model) {
  if (model == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  model->finished = true;
  LOG("model finished: %zu operands, %zu operations, %zu inputs, %zu outputs", model->operands.size(), model->ops.size(),
      model->inputs.size(), model->outputs.size());
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksModel_addOperand(ANeuralNetworksModel* model, const ANeuralNetworksOperandType* type) {
  if (model == nullptr || type == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (model->finished) return ANEURALNETWORKS_BAD_STATE;
  Operand o;
  o.type = type->type;
  o.dims.assign(type->dimensions, type->dimensions + type->dimensionCount);
  model->operands.push_back(std::move(o));
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksModel_setOperandValue(ANeuralNetworksModel* model, int32_t index, const void* buffer, size_t length) {
  if (model == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (index < 0 || static_cast<size_t>(index) >= model->operands.size()) return ANEURALNETWORKS_BAD_DATA;
  Operand& o = model->operands[index];
  o.constant = true;
  if (buffer != nullptr) o.value.assign(static_cast<const uint8_t*>(buffer), static_cast<const uint8_t*>(buffer) + length);  // null: an optional operand left out
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksModel_setOperandValueFromMemory(ANeuralNetworksModel* model, int32_t index, const ANeuralNetworksMemory* memory,
                                                          size_t offset, size_t length) {
  if (model == nullptr || memory == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (index < 0 || static_cast<size_t>(index) >= model->operands.size()) return ANEURALNETWORKS_BAD_DATA;
  Operand& o = model->operands[index];
  o.constant = true;
  o.from = const_cast<ANeuralNetworksMemory*>(memory);
  o.from_offset = offset;
  o.from_length = length;
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksModel_setOperandSymmPerChannelQuantParams(ANeuralNetworksModel* model, int32_t index,
                                                                    const ANeuralNetworksSymmPerChannelQuantParams* params) {
  if (model == nullptr || params == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  return index >= 0 && static_cast<size_t>(index) < model->operands.size() ? ANEURALNETWORKS_NO_ERROR : ANEURALNETWORKS_BAD_DATA;
}
EXPORT int ANeuralNetworksModel_addOperation(ANeuralNetworksModel* model, ANeuralNetworksOperationType type, uint32_t inputCount,
                                             const uint32_t* inputs, uint32_t outputCount, const uint32_t* outputs) {
  if (model == nullptr || inputs == nullptr || outputs == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  Operation op;
  op.type = type;
  op.in.assign(inputs, inputs + inputCount);
  op.out.assign(outputs, outputs + outputCount);
  for (uint32_t i : op.in) if (i >= model->operands.size()) return ANEURALNETWORKS_BAD_DATA;
  for (uint32_t i : op.out) if (i >= model->operands.size()) return ANEURALNETWORKS_BAD_DATA;
  model->ops.push_back(std::move(op));
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksModel_identifyInputsAndOutputs(ANeuralNetworksModel* model, uint32_t inputCount, const uint32_t* inputs,
                                                         uint32_t outputCount, const uint32_t* outputs) {
  if (model == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  model->inputs.assign(inputs, inputs + inputCount);
  model->outputs.assign(outputs, outputs + outputCount);
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksModel_relaxComputationFloat32toFloat16(ANeuralNetworksModel* model, bool) {
  return model == nullptr ? ANEURALNETWORKS_UNEXPECTED_NULL : ANEURALNETWORKS_NO_ERROR;
}

EXPORT int ANeuralNetworksCompilation_create(ANeuralNetworksModel* model, ANeuralNetworksCompilation** compilation) {
  if (model == nullptr || compilation == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (!model->finished) return ANEURALNETWORKS_BAD_STATE;
  auto* c = new ANeuralNetworksCompilation;
  c->model = model;
  for (auto& d : Devices()) c->devices.push_back(d.get());
  *compilation = c;
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT void ANeuralNetworksCompilation_free(ANeuralNetworksCompilation* compilation) { delete compilation; }
EXPORT int ANeuralNetworksCompilation_setPreference(ANeuralNetworksCompilation* compilation, int32_t) {
  return compilation == nullptr ? ANEURALNETWORKS_UNEXPECTED_NULL : ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksCompilation_finish(ANeuralNetworksCompilation* compilation) {
  if (compilation == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (Cfg().fail == "compile") {
    LOG("compilation fails (STUB_NNAPI_FAIL=compile)");
    return ANEURALNETWORKS_OP_FAILED;
  }
  compilation->finished = true;
  std::string names;
  for (const Device* d : compilation->devices) names += (names.empty() ? "" : ",") + d->name;
  LOG("compilation finished for %s: %zu operations", names.c_str(), compilation->model->ops.size());
  return ANEURALNETWORKS_NO_ERROR;
}

EXPORT int ANeuralNetworksExecution_create(ANeuralNetworksCompilation* compilation, ANeuralNetworksExecution** execution) {
  if (compilation == nullptr || execution == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (!compilation->finished) return ANEURALNETWORKS_BAD_STATE;
  auto* e = new ANeuralNetworksExecution;
  e->compilation = compilation;
  *execution = e;
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT void ANeuralNetworksExecution_free(ANeuralNetworksExecution* execution) { delete execution; }
EXPORT int ANeuralNetworksExecution_setInput(ANeuralNetworksExecution* e, int32_t index, const ANeuralNetworksOperandType*, const void* buffer,
                                             size_t length) {
  if (e == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  e->in[static_cast<uint32_t>(index)] = Binding{const_cast<void*>(buffer), nullptr, 0, length};
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksExecution_setInputFromMemory(ANeuralNetworksExecution* e, int32_t index, const ANeuralNetworksOperandType*,
                                                       const ANeuralNetworksMemory* memory, size_t offset, size_t length) {
  if (e == nullptr || memory == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  e->in[static_cast<uint32_t>(index)] = Binding{nullptr, const_cast<ANeuralNetworksMemory*>(memory), offset, length};
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksExecution_setOutput(ANeuralNetworksExecution* e, int32_t index, const ANeuralNetworksOperandType*, void* buffer,
                                              size_t length) {
  if (e == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  e->out[static_cast<uint32_t>(index)] = Binding{buffer, nullptr, 0, length};
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksExecution_setOutputFromMemory(ANeuralNetworksExecution* e, int32_t index, const ANeuralNetworksOperandType*,
                                                        const ANeuralNetworksMemory* memory, size_t offset, size_t length) {
  if (e == nullptr || memory == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  e->out[static_cast<uint32_t>(index)] = Binding{nullptr, const_cast<ANeuralNetworksMemory*>(memory), offset, length};
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksExecution_startCompute(ANeuralNetworksExecution* e, ANeuralNetworksEvent** event) {
  if (e == nullptr || event == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (Cfg().fail == "compute") return ANEURALNETWORKS_OP_FAILED;
  if (!Compute(e)) return ANEURALNETWORKS_BAD_DATA;
  *event = new ANeuralNetworksEvent;
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksEvent_wait(ANeuralNetworksEvent* event) { return event == nullptr ? ANEURALNETWORKS_UNEXPECTED_NULL : ANEURALNETWORKS_NO_ERROR; }
EXPORT void ANeuralNetworksEvent_free(ANeuralNetworksEvent* event) { delete event; }

// ---- NNAPI 1.2: devices and device-targeted compilation -----------------------------------------------------------------------------

EXPORT int ANeuralNetworks_getDeviceCount(uint32_t* numDevices) {
  if (numDevices == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  *numDevices = static_cast<uint32_t>(Devices().size());
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworks_getDevice(uint32_t devIndex, ANeuralNetworksDevice** device) {
  if (device == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (devIndex >= Devices().size()) return ANEURALNETWORKS_BAD_DATA;
  *device = reinterpret_cast<ANeuralNetworksDevice*>(Devices()[devIndex].get());
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksDevice_getName(const ANeuralNetworksDevice* device, const char** name) {
  if (device == nullptr || name == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  *name = reinterpret_cast<const Device*>(device)->name.c_str();
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksDevice_getVersion(const ANeuralNetworksDevice* device, const char** version) {
  if (device == nullptr || version == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  *version = reinterpret_cast<const Device*>(device)->version.c_str();
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksDevice_getType(const ANeuralNetworksDevice* device, int32_t* type) {
  if (device == nullptr || type == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  *type = reinterpret_cast<const Device*>(device)->type;
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksModel_getSupportedOperationsForDevices(const ANeuralNetworksModel* model, const ANeuralNetworksDevice* const* devices,
                                                                 uint32_t numDevices, bool* supportedOps) {
  if (model == nullptr || devices == nullptr || supportedOps == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (!model->finished) return ANEURALNETWORKS_BAD_STATE;
  size_t supported = 0;
  for (size_t i = 0; i < model->ops.size(); ++i) {
    supportedOps[i] = false;
    for (uint32_t d = 0; d < numDevices; ++d)
      if (Supports(*reinterpret_cast<const Device*>(devices[d]), model->ops[i].type)) supportedOps[i] = true;
    supported += supportedOps[i];
  }
  LOG("supported operations: %zu of %zu", supported, model->ops.size());
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksCompilation_createForDevices(ANeuralNetworksModel* model, const ANeuralNetworksDevice* const* devices,
                                                       uint32_t numDevices, ANeuralNetworksCompilation** compilation) {
  if (model == nullptr || devices == nullptr || compilation == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (!model->finished || numDevices == 0) return ANEURALNETWORKS_BAD_STATE;
  auto* c = new ANeuralNetworksCompilation;
  c->model = model;
  for (uint32_t d = 0; d < numDevices; ++d) c->devices.push_back(reinterpret_cast<const Device*>(devices[d]));
  *compilation = c;
  return ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksCompilation_setCaching(ANeuralNetworksCompilation* compilation, const char*, const uint8_t*) {
  return compilation == nullptr ? ANEURALNETWORKS_UNEXPECTED_NULL : ANEURALNETWORKS_NO_ERROR;
}
EXPORT int ANeuralNetworksExecution_compute(ANeuralNetworksExecution* e) {
  if (e == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  if (Cfg().fail == "compute") {
    LOG("execution fails (STUB_NNAPI_FAIL=compute)");
    return ANEURALNETWORKS_OP_FAILED;
  }
  return Compute(e) ? ANEURALNETWORKS_NO_ERROR : ANEURALNETWORKS_BAD_DATA;
}

// ---- NNAPI 1.3 ---------------------------------------------------------------------------------------------------------------------

#ifndef STUB_NNAPI_12_ONLY
EXPORT int ANeuralNetworksDevice_getFeatureLevel(const ANeuralNetworksDevice* device, int64_t* featureLevel) {
  if (device == nullptr || featureLevel == nullptr) return ANEURALNETWORKS_UNEXPECTED_NULL;
  *featureLevel = 29;
  return ANEURALNETWORKS_NO_ERROR;
}
#endif
