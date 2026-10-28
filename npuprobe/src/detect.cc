#include "detect.h"

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <fstream>

namespace probe {
namespace {

const char* const kCoco[90] = {
    "person", "bicycle", "car", "motorcycle", "airplane", "bus", "train", "truck", "boat", "traffic light",
    "fire hydrant", "n/a", "stop sign", "parking meter", "bench", "bird", "cat", "dog", "horse", "sheep",
    "cow", "elephant", "bear", "zebra", "giraffe", "n/a", "backpack", "umbrella", "n/a", "n/a",
    "handbag", "tie", "suitcase", "frisbee", "skis", "snowboard", "sports ball", "kite", "baseball bat",
    "baseball glove", "skateboard", "surfboard", "tennis racket", "bottle", "n/a", "wine glass", "cup", "fork",
    "knife", "spoon", "bowl", "banana", "apple", "sandwich", "orange", "broccoli", "carrot", "hot dog",
    "pizza", "donut", "cake", "chair", "couch", "potted plant", "bed", "n/a", "dining table", "n/a",
    "n/a", "toilet", "n/a", "tv", "laptop", "mouse", "remote", "keyboard", "cell phone", "microwave",
    "oven", "toaster", "sink", "refrigerator", "n/a", "book", "clock", "vase", "scissors", "teddy bear",
    "hair drier", "toothbrush"};

size_t Elements(const TfLiteTensor& t) {
  size_t n = 1;
  for (int i = 0; i < t.dims->size; ++i) n *= static_cast<size_t>(t.dims->data[i]);
  return t.dims->size == 0 ? 0 : n;
}

bool Integral(const std::vector<float>& v) {
  for (float x : v)
    if (std::fabs(x - std::round(x)) > 1e-4f) return false;
  return true;
}

float MaxOf(const std::vector<float>& v) {
  float m = -1e30f;
  for (float x : v) m = std::max(m, x);
  return m;
}

}  // namespace

Labels Labels::Coco() {
  Labels l;
  for (const char* n : kCoco) l.names_.emplace_back(n);
  return l;
}

bool Labels::LoadFile(const std::string& path, std::string* error) {
  std::ifstream in(path);
  if (!in) {
    *error = "cannot read labels " + path;
    return false;
  }
  std::vector<std::string> names;
  std::string line;
  while (std::getline(in, line)) {
    while (!line.empty() && (line.back() == '\r' || line.back() == ' ')) line.pop_back();
    names.push_back(line);
  }
  names_ = std::move(names);
  return true;
}

std::string Labels::Name(int cls) const {
  if (cls >= 0 && static_cast<size_t>(cls) < names_.size()) return names_[cls];
  return "class " + std::to_string(cls);
}

std::string ShapeString(const TfLiteIntArray* dims) {
  std::string s = "[";
  for (int i = 0; i < dims->size; ++i) s += (i ? "," : "") + std::to_string(dims->data[i]);
  return s + "]";
}

std::vector<float> TensorAsFloat(const TfLiteTensor& t) {
  const size_t n = Elements(t);
  std::vector<float> v(n);
  if (t.data.raw == nullptr) return {};
  const float scale = t.params.scale;
  const int zp = t.params.zero_point;
  switch (t.type) {
    case kTfLiteFloat32:
      std::copy(t.data.f, t.data.f + n, v.begin());
      break;
    case kTfLiteUInt8:
      for (size_t i = 0; i < n; ++i) v[i] = scale != 0 ? (static_cast<int>(t.data.uint8[i]) - zp) * scale : t.data.uint8[i];
      break;
    case kTfLiteInt8:
      for (size_t i = 0; i < n; ++i) v[i] = scale != 0 ? (static_cast<int>(t.data.int8[i]) - zp) * scale : t.data.int8[i];
      break;
    case kTfLiteInt16:
      for (size_t i = 0; i < n; ++i) v[i] = scale != 0 ? (static_cast<int>(t.data.i16[i]) - zp) * scale : t.data.i16[i];
      break;
    case kTfLiteInt32:
      for (size_t i = 0; i < n; ++i) v[i] = static_cast<float>(t.data.i32[i]);
      break;
    default:
      return {};
  }
  return v;
}

bool DecodeDetections(const tflite::Interpreter& interp, float min_score, std::vector<Detection>* out,
                      std::string* why) {
  out->clear();
  int boxes = -1, count = -1;
  std::vector<int> pair;  // [1,N] tensors: classes and scores in some order
  for (int idx : interp.outputs()) {
    const TfLiteTensor* t = interp.tensor(idx);
    const TfLiteIntArray* d = t->dims;
    if (d->size == 3 && d->data[0] == 1 && d->data[2] == 4 && boxes < 0) {
      boxes = idx;
    } else if (d->size == 2 && d->data[0] == 1 && d->data[1] > 1) {
      pair.push_back(idx);
    } else if (Elements(*t) == 1 && count < 0) {
      count = idx;
    }
  }
  if (boxes < 0 || pair.size() != 2) {
    *why = "outputs are not boxes [1,N,4] + classes [1,N] + scores [1,N]";
    return false;
  }
  const int n = interp.tensor(boxes)->dims->data[1];
  if (interp.tensor(pair[0])->dims->data[1] != n || interp.tensor(pair[1])->dims->data[1] != n) {
    *why = "box, class and score counts differ";
    return false;
  }

  std::vector<float> box = TensorAsFloat(*interp.tensor(boxes));
  std::vector<float> a = TensorAsFloat(*interp.tensor(pair[0]));
  std::vector<float> b = TensorAsFloat(*interp.tensor(pair[1]));
  if (box.size() != static_cast<size_t>(n) * 4 || a.size() != static_cast<size_t>(n) || b.size() != a.size()) {
    *why = "an output tensor has no data (not run yet?)";
    return false;
  }
  // Classes are whole numbers; scores are fractions. When both look whole (all-zero classes, all-one scores) the
  // post-process op's order wins: classes before scores.
  const std::string name0 = interp.tensor(pair[0])->name ? interp.tensor(pair[0])->name : "";
  const std::string name1 = interp.tensor(pair[1])->name ? interp.tensor(pair[1])->name : "";
  bool a_is_classes;
  if (name0.find("PostProcess:1") != std::string::npos || name1.find("PostProcess:2") != std::string::npos) {
    a_is_classes = true;
  } else if (name0.find("PostProcess:2") != std::string::npos || name1.find("PostProcess:1") != std::string::npos) {
    a_is_classes = false;
  } else if (Integral(a) != Integral(b)) {
    a_is_classes = Integral(a);
  } else {
    a_is_classes = MaxOf(a) >= MaxOf(b);
  }
  const std::vector<float>& cls = a_is_classes ? a : b;
  const std::vector<float>& score = a_is_classes ? b : a;

  int limit = n;
  if (count >= 0) {
    std::vector<float> c = TensorAsFloat(*interp.tensor(count));
    if (!c.empty()) limit = std::min(n, std::max(0, static_cast<int>(std::lround(c[0]))));
  }
  for (int i = 0; i < limit; ++i) {
    if (score[i] < min_score) continue;
    Detection d;
    d.cls = static_cast<int>(std::lround(cls[i]));
    d.score = score[i];
    d.ymin = box[i * 4 + 0];
    d.xmin = box[i * 4 + 1];
    d.ymax = box[i * 4 + 2];
    d.xmax = box[i * 4 + 3];
    out->push_back(d);
  }
  std::stable_sort(out->begin(), out->end(), [](const Detection& x, const Detection& y) { return x.score > y.score; });
  return true;
}

std::vector<std::string> DescribeOutputs(const tflite::Interpreter& interp) {
  std::vector<std::string> lines;
  for (int idx : interp.outputs()) {
    const TfLiteTensor* t = interp.tensor(idx);
    std::vector<float> v = TensorAsFloat(*t);
    float lo = 0, hi = 0;
    if (!v.empty()) {
      lo = *std::min_element(v.begin(), v.end());
      hi = *std::max_element(v.begin(), v.end());
    }
    char buf[256];
    std::snprintf(buf, sizeof buf, "%s %s %s range [%.4g, %.4g]", t->name ? t->name : "?", TfLiteTypeGetName(t->type),
                  ShapeString(t->dims).c_str(), lo, hi);
    lines.emplace_back(buf);
  }
  return lines;
}

}  // namespace probe
