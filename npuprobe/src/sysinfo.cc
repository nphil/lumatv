#include "sysinfo.h"

#include <time.h>

#include <algorithm>
#include <chrono>
#include <cstdio>
#include <cstdlib>

namespace probe {

double CpuSeconds() {
  timespec ts;
  clock_gettime(CLOCK_PROCESS_CPUTIME_ID, &ts);
  return static_cast<double>(ts.tv_sec) + static_cast<double>(ts.tv_nsec) * 1e-9;
}

double WallSeconds() {
  timespec ts;
  clock_gettime(CLOCK_MONOTONIC, &ts);
  return static_cast<double>(ts.tv_sec) + static_cast<double>(ts.tv_nsec) * 1e-9;
}

std::string ReadSmall(const std::string& path) {
  std::FILE* f = std::fopen(path.c_str(), "r");
  if (f == nullptr) return "";
  std::string out;
  char buf[512];
  size_t n;
  while ((n = std::fread(buf, 1, sizeof buf, f)) > 0 && out.size() < 16384) out.append(buf, n);
  std::fclose(f);
  return out;
}

int FirstInt(const std::string& text) {
  size_t i = 0;
  while (i < text.size() && (text[i] < '0' || text[i] > '9')) ++i;
  if (i == text.size()) return -1;
  return std::atoi(text.c_str() + i);
}

MaliCounters ReadMaliCounters(const std::string& path) {
  MaliCounters c;
  const std::string text = ReadSmall(path);
  bool have_busy = false, have_idle = false;
  size_t pos = 0;
  while (pos < text.size()) {
    size_t nl = text.find('\n', pos);
    if (nl == std::string::npos) nl = text.size();
    const std::string line = text.substr(pos, nl - pos);
    unsigned long long v = 0;
    if (std::sscanf(line.c_str(), "busy_time: %llu", &v) == 1) {
      c.busy = v;
      have_busy = true;
    } else if (std::sscanf(line.c_str(), "idle_time: %llu", &v) == 1) {
      c.idle = v;
      have_idle = true;
    }
    pos = nl + 1;
  }
  c.ok = have_busy && have_idle;
  return c;
}

double MaliBusyPercent(const MaliCounters& before, const MaliCounters& after) {
  if (!before.ok || !after.ok || after.busy < before.busy || after.idle < before.idle) return -1;
  const double busy = static_cast<double>(after.busy - before.busy), idle = static_cast<double>(after.idle - before.idle);
  return busy + idle > 0 ? busy / (busy + idle) * 100.0 : -1;
}

NpuSampler::NpuSampler(const std::string& proc_dir, int period_ms)
    : path_(proc_dir + "/npu_utilization"), period_ms_(period_ms) {
  available_ = !ReadSmall(path_).empty();
  if (!available_) return;
  run_ = true;
  thread_ = std::thread([this] {
    while (run_) {
      int v = FirstInt(ReadSmall(path_));
      if (v >= 0) {
        std::lock_guard<std::mutex> lock(mu_);
        values_.push_back(v);
      }
      std::this_thread::sleep_for(std::chrono::milliseconds(period_ms_));
    }
  });
}

NpuSampler::~NpuSampler() { Stop(); }

NpuSampler::Result NpuSampler::Stop() {
  if (stopped_) return result_;
  stopped_ = true;
  run_ = false;
  if (thread_.joinable()) thread_.join();
  std::lock_guard<std::mutex> lock(mu_);
  result_.samples = static_cast<int>(values_.size());
  double sum = 0;
  for (int v : values_) {
    sum += v;
    result_.max = std::max(result_.max, v);
    if (v > 0) ++result_.nonzero;
  }
  result_.mean = values_.empty() ? 0 : sum / static_cast<double>(values_.size());
  return result_;
}

}  // namespace probe
