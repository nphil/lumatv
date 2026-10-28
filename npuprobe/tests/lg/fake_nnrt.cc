// Stand-in for LG's libnnrt.so.1 (webos::nnrt::Nnrt), enough for libtflite-npu-delegate.so.1 to load and run on a host
// without /dev/npu0. Every call is logged to stderr as "NNRT <function>(args)", which shows what the delegate hands the
// runtime. The signatures are the mangled names the delegate imports (symbols only); the return values are guesses:
// 0 / a valid graph id for success.
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <vector>

namespace webos {
namespace nnrt {

enum class TensorType : int {};
struct Tensor {
  unsigned char opaque[256];
};

class Nnrt {
 public:
  static Nnrt& getInstance();
  int LoadOpGraph(const char* package, unsigned size);
  int PrepareOpGraph(int graph, int priority);
  int ExecuteOpGraph(int graph);
  int UnLoadOpGraph(int graph);
  int AddInputTensor(int graph, TensorType type, unsigned bytes, char* data, int flags, const std::vector<int>& dims, bool custom_alloc);
  int AddOutputTensor(int graph, TensorType type, unsigned bytes, char* data, int flags, const std::vector<int>& dims, bool custom_alloc);
  Tensor* getInputTensor(int graph, int index);
  Tensor* getOutputTensor(int graph, int index);
};

namespace util {
void* getNNRTPmLogContext();
}

static void Hex(const char* p, unsigned n) {
  for (unsigned i = 0; i < n && i < 16; ++i) std::fprintf(stderr, "%02x", static_cast<unsigned char>(p[i]));
}

Nnrt& Nnrt::getInstance() {
  static Nnrt instance;
  std::fprintf(stderr, "NNRT getInstance()\n");
  return instance;
}
int Nnrt::LoadOpGraph(const char* package, unsigned size) {
  std::fprintf(stderr, "NNRT LoadOpGraph(package=%p size=%u first bytes=", static_cast<const void*>(package), size);
  if (package != nullptr) Hex(package, size);
  std::fprintf(stderr, ")\n");
  return 1;
}
int Nnrt::PrepareOpGraph(int graph, int priority) {
  std::fprintf(stderr, "NNRT PrepareOpGraph(graph=%d priority=%d)\n", graph, priority);
  return 0;
}
int Nnrt::ExecuteOpGraph(int graph) {
  std::fprintf(stderr, "NNRT ExecuteOpGraph(graph=%d)\n", graph);
  return 0;
}
int Nnrt::UnLoadOpGraph(int graph) {
  std::fprintf(stderr, "NNRT UnLoadOpGraph(graph=%d)\n", graph);
  return 0;
}
static void LogTensor(const char* what, int graph, TensorType type, unsigned bytes, char* data, int flags, const std::vector<int>& dims, bool custom) {
  std::fprintf(stderr, "NNRT %s(graph=%d type=%d bytes=%u data=%p flags=%d dims=[", what, graph, static_cast<int>(type), bytes,
               static_cast<void*>(data), flags);
  for (size_t i = 0; i < dims.size(); ++i) std::fprintf(stderr, "%s%d", i ? "," : "", dims[i]);
  std::fprintf(stderr, "] custom_alloc=%d)\n", custom ? 1 : 0);
}
int Nnrt::AddInputTensor(int graph, TensorType type, unsigned bytes, char* data, int flags, const std::vector<int>& dims, bool custom) {
  LogTensor("AddInputTensor", graph, type, bytes, data, flags, dims, custom);
  return 0;
}
int Nnrt::AddOutputTensor(int graph, TensorType type, unsigned bytes, char* data, int flags, const std::vector<int>& dims, bool custom) {
  LogTensor("AddOutputTensor", graph, type, bytes, data, flags, dims, custom);
  return 0;
}
Tensor* Nnrt::getInputTensor(int graph, int index) {
  std::fprintf(stderr, "NNRT getInputTensor(graph=%d index=%d)\n", graph, index);
  static Tensor t;
  return &t;
}
Tensor* Nnrt::getOutputTensor(int graph, int index) {
  std::fprintf(stderr, "NNRT getOutputTensor(graph=%d index=%d)\n", graph, index);
  static Tensor t;
  return &t;
}

namespace util {
// PmLog macros read the context object before they call _PmLogMsgKV, so it cannot be null. A zeroed object reads as
// "nothing enabled": the delegate's log lines are then skipped, its return values are unchanged.
void* getNNRTPmLogContext() {
  static unsigned char context[256];
  return context;
}
}  // namespace util

}  // namespace nnrt
}  // namespace webos
