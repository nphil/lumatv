#include "image.h"

#include <algorithm>
#include <cmath>

#define STB_IMAGE_IMPLEMENTATION
#define STBI_ONLY_JPEG
#define STBI_ONLY_PNG
#define STBI_ONLY_BMP
#define STBI_ONLY_PNM
#define STBI_NO_STDIO_WRITE
#include "stb_image.h"

namespace probe {

bool LoadImage(const std::string& path, Image* out, std::string* error) {
  int w = 0, h = 0, n = 0;
  unsigned char* data = stbi_load(path.c_str(), &w, &h, &n, 3);
  if (data == nullptr) {
    const char* why = stbi_failure_reason();
    *error = "cannot load " + path + ": " + (why != nullptr ? why : "unknown");
    return false;
  }
  out->w = w;
  out->h = h;
  out->rgb.assign(data, data + static_cast<size_t>(w) * h * 3);
  stbi_image_free(data);
  return true;
}

Image TestPattern(int w, int h) {
  Image img;
  img.w = w;
  img.h = h;
  img.rgb.resize(static_cast<size_t>(w) * h * 3);
  const float cx = w * 0.5f, cy = h * 0.5f, r = std::min(w, h) * 0.25f;
  for (int y = 0; y < h; ++y) {
    for (int x = 0; x < w; ++x) {
      uint8_t* p = &img.rgb[(static_cast<size_t>(y) * w + x) * 3];
      float dx = x - cx, dy = y - cy;
      bool disc = dx * dx + dy * dy < r * r;
      p[0] = disc ? 250 : static_cast<uint8_t>(255 * x / std::max(1, w - 1));
      p[1] = disc ? 240 : static_cast<uint8_t>(255 * y / std::max(1, h - 1));
      p[2] = disc ? 230 : static_cast<uint8_t>(128 + 127 * std::sin(0.05f * (x + y)));
    }
  }
  return img;
}

Image ResizeBilinear(const Image& in, int w, int h) {
  Image out;
  out.w = w;
  out.h = h;
  out.rgb.resize(static_cast<size_t>(w) * h * 3);
  const float sx = static_cast<float>(in.w) / w, sy = static_cast<float>(in.h) / h;
  for (int y = 0; y < h; ++y) {
    float fy = std::clamp((y + 0.5f) * sy - 0.5f, 0.0f, static_cast<float>(in.h - 1));
    int y0 = static_cast<int>(fy), y1 = std::min(y0 + 1, in.h - 1);
    float wy = fy - y0;
    for (int x = 0; x < w; ++x) {
      float fx = std::clamp((x + 0.5f) * sx - 0.5f, 0.0f, static_cast<float>(in.w - 1));
      int x0 = static_cast<int>(fx), x1 = std::min(x0 + 1, in.w - 1);
      float wx = fx - x0;
      for (int c = 0; c < 3; ++c) {
        float a = in.rgb[(static_cast<size_t>(y0) * in.w + x0) * 3 + c];
        float b = in.rgb[(static_cast<size_t>(y0) * in.w + x1) * 3 + c];
        float d = in.rgb[(static_cast<size_t>(y1) * in.w + x0) * 3 + c];
        float e = in.rgb[(static_cast<size_t>(y1) * in.w + x1) * 3 + c];
        float v = (a * (1 - wx) + b * wx) * (1 - wy) + (d * (1 - wx) + e * wx) * wy;
        out.rgb[(static_cast<size_t>(y) * w + x) * 3 + c] = static_cast<uint8_t>(std::lround(v));
      }
    }
  }
  return out;
}

}  // namespace probe
