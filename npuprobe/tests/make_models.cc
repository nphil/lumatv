// Writes the small .tflite files the probe tests use, with TFLite's own schema (object API):
//   float_net.tflite   CONV_2D, DEPTHWISE_CONV_2D, ADD on float32
//   int8_net.tflite    the op set of LG's FitTV_Detector_Yolov3_QAT_SoC.tflite (QUANTIZE, PAD, CONV_2D,
//                      DEPTHWISE_CONV_2D, ADD, RESIZE_NEAREST_NEIGHBOR, CONCATENATION), uint8 input, per-channel int8
//   npu_custom.tflite  an lgnpu_custom_op node followed by a builtin ADD
//   npu_only.tflite    the lgnpu_custom_op node alone: the whole network is the package, the shape of an NPU model
//   ssd_const.tflite   constant TFLite_Detection_PostProcess-style outputs, for the detection decoder
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <memory>
#include <string>
#include <vector>

#include "tensorflow/lite/schema/schema_generated.h"

using namespace tflite;

namespace {

struct Rng {
  uint32_t s = 12345;
  uint32_t Next() { return s = s * 1664525u + 1013904223u; }
  float Uniform() { return static_cast<float>(Next() >> 8) / 16777216.f; }  // [0,1)
  int Int8() { return static_cast<int>(Next() >> 24) - 128; }
};

struct Builder {
  ModelT model;
  SubGraphT* sg = nullptr;

  explicit Builder(const char* description) {
    model.version = 3;
    model.description = description;
    model.buffers.emplace_back(new BufferT);  // buffer 0: the empty one every non-constant tensor points at
    model.subgraphs.emplace_back(new SubGraphT);
    sg = model.subgraphs[0].get();
    sg->name = "main";
  }

  int Buffer(const void* data, size_t n) {
    auto b = std::make_unique<BufferT>();
    const uint8_t* p = static_cast<const uint8_t*>(data);
    b->data.assign(p, p + n);
    model.buffers.push_back(std::move(b));
    return static_cast<int>(model.buffers.size()) - 1;
  }

  int Tensor(const std::string& name, TensorType type, std::vector<int32_t> shape, int buffer = 0,
             std::vector<float> scales = {}, std::vector<int64_t> zps = {}, int qdim = 0) {
    auto t = std::make_unique<TensorT>();
    t->name = name;
    t->type = type;
    t->shape = shape;
    t->buffer = buffer;
    if (!scales.empty()) {
      t->quantization = std::make_unique<QuantizationParametersT>();
      t->quantization->scale = scales;
      t->quantization->zero_point = zps.empty() ? std::vector<int64_t>(scales.size(), 0) : zps;
      t->quantization->quantized_dimension = qdim;
    }
    sg->tensors.push_back(std::move(t));
    return static_cast<int>(sg->tensors.size()) - 1;
  }

  int Code(BuiltinOperator op, const char* custom = nullptr, int version = 1) {
    for (size_t i = 0; i < model.operator_codes.size(); ++i) {
      const auto& c = model.operator_codes[i];
      if (c->builtin_code == op && (custom ? c->custom_code == custom : c->custom_code.empty())) return static_cast<int>(i);
    }
    auto c = std::make_unique<OperatorCodeT>();
    c->builtin_code = op;
    c->deprecated_builtin_code = static_cast<int8_t>(op < 127 ? op : 127);
    if (custom) c->custom_code = custom;
    c->version = version;
    model.operator_codes.push_back(std::move(c));
    return static_cast<int>(model.operator_codes.size()) - 1;
  }

  OperatorT* Op(int code, std::vector<int32_t> in, std::vector<int32_t> out) {
    auto o = std::make_unique<OperatorT>();
    o->opcode_index = code;
    o->inputs = in;
    o->outputs = out;
    sg->operators.push_back(std::move(o));
    return sg->operators.back().get();
  }

  void Io(std::vector<int32_t> in, std::vector<int32_t> out) {
    sg->inputs = in;
    sg->outputs = out;
  }

  bool Write(const std::string& path) {
    flatbuffers::FlatBufferBuilder fbb;
    fbb.Finish(Model::Pack(fbb, &model), ModelIdentifier());
    std::FILE* f = std::fopen(path.c_str(), "wb");
    if (!f) return false;
    bool ok = std::fwrite(fbb.GetBufferPointer(), 1, fbb.GetSize(), f) == fbb.GetSize();
    std::fclose(f);
    return ok;
  }
};

template <typename T>
int ConstBuffer(Builder& b, const std::vector<T>& v) {
  return b.Buffer(v.data(), v.size() * sizeof(T));
}

std::vector<float> RandomFloats(Rng& r, size_t n, float lo, float hi) {
  std::vector<float> v(n);
  for (float& x : v) x = lo + (hi - lo) * r.Uniform();
  return v;
}

bool FloatNet(const std::string& dir) {
  Builder b("float net");
  Rng r;
  int in = b.Tensor("input", TensorType_FLOAT32, {1, 8, 8, 3});
  int w1 = b.Tensor("conv_w", TensorType_FLOAT32, {4, 3, 3, 3}, ConstBuffer(b, RandomFloats(r, 4 * 3 * 3 * 3, -0.5f, 0.5f)));
  int b1 = b.Tensor("conv_b", TensorType_FLOAT32, {4}, ConstBuffer(b, RandomFloats(r, 4, -0.1f, 0.1f)));
  int t1 = b.Tensor("conv_out", TensorType_FLOAT32, {1, 8, 8, 4});
  int wd = b.Tensor("dw_w", TensorType_FLOAT32, {1, 3, 3, 4}, ConstBuffer(b, RandomFloats(r, 3 * 3 * 4, -0.5f, 0.5f)));
  int bd = b.Tensor("dw_b", TensorType_FLOAT32, {4}, ConstBuffer(b, RandomFloats(r, 4, -0.1f, 0.1f)));
  int t2 = b.Tensor("dw_out", TensorType_FLOAT32, {1, 8, 8, 4});
  int t3 = b.Tensor("sum", TensorType_FLOAT32, {1, 8, 8, 4});

  OperatorT* conv = b.Op(b.Code(BuiltinOperator_CONV_2D), {in, w1, b1}, {t1});
  Conv2DOptionsT co;
  co.padding = Padding_SAME;
  co.stride_w = co.stride_h = 1;
  co.fused_activation_function = ActivationFunctionType_RELU;
  conv->builtin_options.Set(std::move(co));
  OperatorT* dw = b.Op(b.Code(BuiltinOperator_DEPTHWISE_CONV_2D), {t1, wd, bd}, {t2});
  DepthwiseConv2DOptionsT dwo;
  dwo.padding = Padding_SAME;
  dwo.stride_w = dwo.stride_h = 1;
  dwo.depth_multiplier = 1;
  dwo.fused_activation_function = ActivationFunctionType_RELU6;
  dw->builtin_options.Set(std::move(dwo));
  OperatorT* add = b.Op(b.Code(BuiltinOperator_ADD), {t1, t2}, {t3});
  AddOptionsT ao;
  add->builtin_options.Set(std::move(ao));
  b.Io({in}, {t3});
  return b.Write(dir + "/float_net.tflite");
}

bool Int8Net(const std::string& dir) {
  Builder b("int8 net, LG's op set");
  Rng r;
  const float in_scale = 1.f / 255.f;
  int in = b.Tensor("input", TensorType_UINT8, {1, 16, 16, 3}, 0, {in_scale}, {0});
  int q = b.Tensor("quantized", TensorType_INT8, {1, 16, 16, 3}, 0, {in_scale}, {-128});
  std::vector<int32_t> pads = {0, 0, 1, 1, 1, 1, 0, 0};
  int pad_amt = b.Tensor("pad_amount", TensorType_INT32, {4, 2}, ConstBuffer(b, pads));
  int padded = b.Tensor("padded", TensorType_INT8, {1, 18, 18, 3}, 0, {in_scale}, {-128});

  std::vector<float> ws1 = {0.020f, 0.025f, 0.030f, 0.035f};
  std::vector<int8_t> w1(4 * 3 * 3 * 3);
  for (auto& x : w1) x = static_cast<int8_t>(r.Int8() / 2);
  std::vector<int32_t> bias1(4);
  std::vector<float> bs1(4);
  for (int i = 0; i < 4; ++i) {
    bs1[i] = in_scale * ws1[i];
    bias1[i] = r.Int8();
  }
  int cw = b.Tensor("conv_w", TensorType_INT8, {4, 3, 3, 3}, ConstBuffer(b, w1), ws1, {0, 0, 0, 0}, 0);
  int cb = b.Tensor("conv_b", TensorType_INT32, {4}, ConstBuffer(b, bias1), bs1, {0, 0, 0, 0}, 0);
  int c = b.Tensor("conv_out", TensorType_INT8, {1, 8, 8, 4}, 0, {0.05f}, {-10});

  std::vector<float> ws2 = {0.010f, 0.012f, 0.014f, 0.016f};
  std::vector<int8_t> w2(1 * 3 * 3 * 4);
  for (auto& x : w2) x = static_cast<int8_t>(r.Int8() / 2);
  std::vector<int32_t> bias2(4);
  std::vector<float> bs2(4);
  for (int i = 0; i < 4; ++i) {
    bs2[i] = 0.05f * ws2[i];
    bias2[i] = r.Int8();
  }
  int dw_w = b.Tensor("dw_w", TensorType_INT8, {1, 3, 3, 4}, ConstBuffer(b, w2), ws2, {0, 0, 0, 0}, 3);
  int dw_b = b.Tensor("dw_b", TensorType_INT32, {4}, ConstBuffer(b, bias2), bs2, {0, 0, 0, 0}, 0);
  int d = b.Tensor("dw_out", TensorType_INT8, {1, 8, 8, 4}, 0, {0.05f}, {-10});
  int e = b.Tensor("sum", TensorType_INT8, {1, 8, 8, 4}, 0, {0.1f}, {0});
  std::vector<int32_t> size = {16, 16};
  int sz = b.Tensor("size", TensorType_INT32, {2}, ConstBuffer(b, size));
  int f = b.Tensor("upsampled", TensorType_INT8, {1, 16, 16, 4}, 0, {0.1f}, {0});
  int g = b.Tensor("out", TensorType_INT8, {1, 16, 16, 8}, 0, {0.1f}, {0});

  b.Op(b.Code(BuiltinOperator_QUANTIZE), {in}, {q})->builtin_options.Set(QuantizeOptionsT());
  b.Op(b.Code(BuiltinOperator_PAD, nullptr, 2), {q, pad_amt}, {padded})->builtin_options.Set(PadOptionsT());
  OperatorT* conv = b.Op(b.Code(BuiltinOperator_CONV_2D, nullptr, 3), {padded, cw, cb}, {c});
  Conv2DOptionsT co;
  co.padding = Padding_VALID;
  co.stride_w = co.stride_h = 2;
  conv->builtin_options.Set(std::move(co));
  OperatorT* dw = b.Op(b.Code(BuiltinOperator_DEPTHWISE_CONV_2D, nullptr, 3), {c, dw_w, dw_b}, {d});
  DepthwiseConv2DOptionsT dwo;
  dwo.padding = Padding_SAME;
  dwo.stride_w = dwo.stride_h = 1;
  dwo.depth_multiplier = 1;
  dw->builtin_options.Set(std::move(dwo));
  b.Op(b.Code(BuiltinOperator_ADD, nullptr, 2), {c, d}, {e})->builtin_options.Set(AddOptionsT());
  OperatorT* rs = b.Op(b.Code(BuiltinOperator_RESIZE_NEAREST_NEIGHBOR, nullptr, 2), {e, sz}, {f});
  ResizeNearestNeighborOptionsT rso;
  rso.half_pixel_centers = true;
  rs->builtin_options.Set(std::move(rso));
  OperatorT* cat = b.Op(b.Code(BuiltinOperator_CONCATENATION, nullptr, 2), {f, f}, {g});
  ConcatenationOptionsT cato;
  cato.axis = 3;
  cat->builtin_options.Set(std::move(cato));
  b.Io({in}, {g});
  return b.Write(dir + "/int8_net.tflite");
}

bool NpuCustom(const std::string& dir) {
  Builder b("lgnpu_custom_op between builtin ops");
  int in = b.Tensor("input", TensorType_FLOAT32, {1, 4, 4, 1});
  int m = b.Tensor("npu_out", TensorType_FLOAT32, {1, 4, 4, 1});
  float half = 0.5f;
  int c = b.Tensor("half", TensorType_FLOAT32, {1}, b.Buffer(&half, sizeof half));
  int out = b.Tensor("output", TensorType_FLOAT32, {1, 4, 4, 1});
  OperatorT* npu = b.Op(b.Code(BuiltinOperator_CUSTOM, "lgnpu_custom_op"), {in}, {m});
  // The stub's "package": scale and bias as two little-endian floats. LG's is a compiled flatbuffer (libnpu's npu_package).
  float payload[2] = {2.0f, 1.0f};
  npu->custom_options.assign(reinterpret_cast<uint8_t*>(payload), reinterpret_cast<uint8_t*>(payload) + sizeof payload);
  b.Op(b.Code(BuiltinOperator_ADD), {m, c}, {out})->builtin_options.Set(AddOptionsT());
  b.Io({in}, {out});
  return b.Write(dir + "/npu_custom.tflite");
}

bool NpuOnly(const std::string& dir) {
  Builder b("lgnpu_custom_op alone");
  int in = b.Tensor("input", TensorType_FLOAT32, {1, 4, 4, 1});
  int out = b.Tensor("output", TensorType_FLOAT32, {1, 4, 4, 1});
  OperatorT* npu = b.Op(b.Code(BuiltinOperator_CUSTOM, "lgnpu_custom_op"), {in}, {out});
  float payload[2] = {2.0f, 1.0f};
  npu->custom_options.assign(reinterpret_cast<uint8_t*>(payload), reinterpret_cast<uint8_t*>(payload) + sizeof payload);
  b.Io({in}, {out});
  return b.Write(dir + "/npu_only.tflite");
}

bool SsdConst(const std::string& dir) {
  Builder b("constant detections");
  int in = b.Tensor("input", TensorType_FLOAT32, {1, 2, 2, 3});
  // person 0.91, dog 0.62, cat 0.12 (below the default 0.3 threshold); class ids as in the 90-id COCO map
  std::vector<float> boxes = {0.10f, 0.20f, 0.60f, 0.50f, 0.30f, 0.40f, 0.90f, 0.95f, 0.00f, 0.00f, 0.10f, 0.10f};
  std::vector<float> classes = {0, 17, 16};
  std::vector<float> scores = {0.91f, 0.62f, 0.12f};
  std::vector<float> count = {3};
  int ob = b.Tensor("TFLite_Detection_PostProcess", TensorType_FLOAT32, {1, 3, 4}, ConstBuffer(b, boxes));
  int oc = b.Tensor("TFLite_Detection_PostProcess:1", TensorType_FLOAT32, {1, 3}, ConstBuffer(b, classes));
  int os = b.Tensor("TFLite_Detection_PostProcess:2", TensorType_FLOAT32, {1, 3}, ConstBuffer(b, scores));
  int on = b.Tensor("TFLite_Detection_PostProcess:3", TensorType_FLOAT32, {1}, ConstBuffer(b, count));
  b.Io({in}, {ob, oc, os, on});
  return b.Write(dir + "/ssd_const.tflite");
}

}  // namespace

int main(int argc, char** argv) {
  if (argc != 2) {
    std::fprintf(stderr, "usage: %s OUT_DIR\n", argv[0]);
    return 2;
  }
  const std::string dir = argv[1];
  bool ok = FloatNet(dir) && Int8Net(dir) && NpuCustom(dir) && NpuOnly(dir) && SsdConst(dir);
  if (!ok) std::fprintf(stderr, "write failed in %s\n", dir.c_str());
  return ok ? 0 : 1;
}
