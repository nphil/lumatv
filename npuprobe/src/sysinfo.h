// Process CPU time and the kernel NPU driver's /proc counters (read only).
#pragma once

#include <atomic>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace probe {

double CpuSeconds();   // CPU time of this process (all threads)
double WallSeconds();  // monotonic clock

// Whole file as text, "" when it cannot be read. For small /proc files only.
std::string ReadSmall(const std::string& path);

// First integer in |text|, or -1.
int FirstInt(const std::string& text);

// Polls <proc_dir>/npu_utilization (an integer, the driver's busy percentage) from a thread until Stop().
class NpuSampler {
 public:
  struct Result {
    int samples = 0;
    int max = 0;
    int nonzero = 0;
    double mean = 0;
  };

  explicit NpuSampler(const std::string& proc_dir, int period_ms = 5);
  NpuSampler(const NpuSampler&) = delete;
  NpuSampler& operator=(const NpuSampler&) = delete;
  ~NpuSampler();

  bool available() const { return available_; }
  Result Stop();  // joins the thread; the same result on later calls

 private:
  std::string path_;
  int period_ms_;
  bool available_ = false;
  std::atomic<bool> run_{false};
  std::thread thread_;
  std::mutex mu_;
  std::vector<int> values_;
  bool stopped_ = false;
  Result result_;
};

}  // namespace probe
