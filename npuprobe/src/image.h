// Image input for the probe: JPEG/PNG/BMP/PNM through stb_image, plus a deterministic test pattern.
#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace probe {

struct Image {
  int w = 0;
  int h = 0;
  std::vector<uint8_t> rgb;  // w*h*3, row-major
};

bool LoadImage(const std::string& path, Image* out, std::string* error);

// Gradient with a bright disc; the same bytes on every host.
Image TestPattern(int w, int h);

// Bilinear, pixel centres aligned (the mapping TFLite's resize_bilinear uses with half_pixel_centers).
Image ResizeBilinear(const Image& in, int w, int h);

}  // namespace probe
