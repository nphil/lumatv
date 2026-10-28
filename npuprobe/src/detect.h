// Reading detector outputs. Only the TFLite_Detection_PostProcess layout is decoded (boxes [1,N,4], classes [1,N],
// scores [1,N], count [1], in any order); models with raw heads (LG's YOLOv3 needs its own anchors) get a summary.
#pragma once

#include <string>
#include <vector>

#include "tensorflow/lite/interpreter.h"

namespace probe {

struct Detection {
  int cls = 0;
  float score = 0;
  float ymin = 0, xmin = 0, ymax = 0, xmax = 0;  // normalised to the model input
};

class Labels {
 public:
  static Labels Coco();  // 90 ids, 0 = person, "n/a" in the gaps (google-coral/test_data coco_labels.txt)
  bool LoadFile(const std::string& path, std::string* error);
  std::string Name(int cls) const;

 private:
  std::vector<std::string> names_;
};

// All values of |t| as float, dequantised with its scale and zero point when it is integer typed.
std::vector<float> TensorAsFloat(const TfLiteTensor& t);

// True when the outputs have the post-process layout; |out| is sorted by score, best first, and holds only
// detections at or above |min_score|. On false |why| says what did not fit.
bool DecodeDetections(const tflite::Interpreter& interp, float min_score, std::vector<Detection>* out,
                      std::string* why);

// One line per output tensor: name, type, shape, value range.
std::vector<std::string> DescribeOutputs(const tflite::Interpreter& interp);

std::string ShapeString(const TfLiteIntArray* dims);

}  // namespace probe
