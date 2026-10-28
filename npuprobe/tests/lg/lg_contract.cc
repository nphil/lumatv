// Runs LG's libtflite-npu-delegate.so.1 as a black box against a fake TFLite context (no TFLite library, no NPU) and
// reports what it does: the option keys it accepts, the graphs it takes or refuses, the size of the TfLiteRegistration
// it hands to ReplaceNodeSubsetsWithDelegateKernels (which pins the TFLite version it was built against), and the
// calls its kernel makes into libnnrt when it is initialised, prepared and invoked. libnnrt and PmLogLib are the
// fakes next to this file. Only exported symbols and observable behaviour are used.
//
//   lg_contract DELEGATE.so
#include <dlfcn.h>
#include <initializer_list>
#include <signal.h>
#include <ucontext.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/wait.h>
#include <unistd.h>

#include "tensorflow/lite/builtin_ops.h"
#include "tensorflow/lite/c/common.h"

// ---- what the delegate expects the host to export -------------------------------------------------------------
extern "C" {
TfLiteDelegate TfLiteDelegateCreate(void) {
  TfLiteDelegate d = {};
  d.flags = kTfLiteDelegateFlagsNone;
  return d;
}
TfLiteIntArray* TfLiteIntArrayCreate(int size) {
  TfLiteIntArray* a = static_cast<TfLiteIntArray*>(calloc(1, sizeof(int) * (1 + static_cast<size_t>(size))));
  a->size = size;
  return a;
}
void TfLiteIntArrayFree(TfLiteIntArray* a) { free(a); }
}

namespace tflite {
class Subgraph {
 public:
  TfLiteStatus SetCustomAllocationForTensor(int tensor_index, const TfLiteCustomAllocation& allocation, int64_t flags);
};
TfLiteStatus Subgraph::SetCustomAllocationForTensor(int tensor_index, const TfLiteCustomAllocation& allocation, int64_t flags) {
  printf("  host: SetCustomAllocationForTensor(tensor=%d data=%p bytes=%zu flags=%lld)\n", tensor_index, allocation.data,
         allocation.bytes, static_cast<long long>(flags));
  return kTfLiteOk;
}
}  // namespace tflite

// ---- a context with one or two nodes ---------------------------------------------------------------------------
namespace {

struct Fake {
  TfLiteTensor tensors[4];
  tflite::Subgraph subgraph;
  TfLiteRegistration regs[2];
  TfLiteNode nodes[2];
  int num_nodes = 0;
  alignas(8) unsigned char payload[8] = {0, 0, 0x40, 0x40, 0, 0, 0x80, 0x3f};  // 3.0f, 1.0f little endian
  const TfLiteDelegate* delegate = nullptr;
  // what ReplaceNodeSubsetsWithDelegateKernels received
  bool replaced = false;
  int nodes_size = -1;
  int nodes_first = -1;
  const TfLiteDelegate* delegate_seen = nullptr;
  uintptr_t reg_words[16] = {};
  int reg_words_seen = 0;
} g;

void Trap(const char* name) { printf("  host: UNEXPECTED context call %s\n", name); }

TfLiteStatus GetExecutionPlan(TfLiteContext*, TfLiteIntArray** plan) {
  *plan = TfLiteIntArrayCreate(g.num_nodes);
  for (int i = 0; i < g.num_nodes; ++i) (*plan)->data[i] = i;
  return kTfLiteOk;
}

TfLiteStatus GetNodeAndRegistration(TfLiteContext*, int node_index, TfLiteNode** node, TfLiteRegistration** reg) {
  *node = &g.nodes[node_index];
  *reg = &g.regs[node_index];
  return kTfLiteOk;
}

void ReportError(TfLiteContext*, const char* fmt, ...) { printf("  host: ReportError(%s)\n", fmt); }

// The callee for each struct size. The caller's TfLiteRegistration is copied by value into r1-r3 and the stack (AAPCS),
// so a callee that reads the argument after it with the wrong struct size sees the wrong word: the delegate's real
// size is the variant for which |nodes| comes out as a valid TfLiteIntArray and |delegate| as the delegate it was given.
template <typename Reg>
TfLiteStatus Replace(TfLiteContext*, Reg reg, const TfLiteIntArray* nodes, TfLiteDelegate* delegate) {
  g.replaced = true;
  g.delegate_seen = delegate;
  const uintptr_t* w = reinterpret_cast<const uintptr_t*>(&reg);
  g.reg_words_seen = static_cast<int>(sizeof(Reg) / sizeof(uintptr_t));
  for (int i = 0; i < g.reg_words_seen && i < 16; ++i) g.reg_words[i] = w[i];
  if (nodes != nullptr && reinterpret_cast<uintptr_t>(nodes) > 0x10000) {
    g.nodes_size = nodes->size;
    if (nodes->size > 0) g.nodes_first = nodes->data[0];
  }
  return kTfLiteOk;
}

struct Reg8 {
  void* w[8];
};
struct Reg9 {
  void* w[9];
};
struct Reg10 {
  void* w[10];
};
struct alignas(8) Reg11 {
  void* w[10];
  uint64_t inplace_operator;
};

typedef TfLiteStatus (*ReplaceFn)(TfLiteContext*, TfLiteRegistration, const TfLiteIntArray*, TfLiteDelegate*);
ReplaceFn ReplaceFor(int variant) {
  switch (variant) {
    case 8: return reinterpret_cast<ReplaceFn>(&Replace<Reg8>);
    case 9: return reinterpret_cast<ReplaceFn>(&Replace<Reg9>);
    case 10: return reinterpret_cast<ReplaceFn>(&Replace<Reg10>);
    default: return reinterpret_cast<ReplaceFn>(&Replace<Reg11>);
  }
}

void SetupContext(TfLiteContext* ctx, int variant) {
  memset(ctx, 0, sizeof *ctx);
  ctx->tensors_size = 4;
  ctx->tensors = g.tensors;
  ctx->impl_ = &g.subgraph;
  ctx->GetExecutionPlan = GetExecutionPlan;
  ctx->GetNodeAndRegistration = GetNodeAndRegistration;
  ctx->ReplaceNodeSubsetsWithDelegateKernels = ReplaceFor(variant);
  ctx->ReportError = ReportError;
  ctx->AddTensors = [](TfLiteContext*, int, int*) { Trap("AddTensors"); return kTfLiteError; };
  ctx->ResizeTensor = [](TfLiteContext*, TfLiteTensor*, TfLiteIntArray*) { Trap("ResizeTensor"); return kTfLiteError; };
}

void SetupGraph(const char* kind) {
  memset(&g.regs, 0, sizeof g.regs);
  memset(&g.nodes, 0, sizeof g.nodes);
  g.replaced = false;
  g.nodes_size = -1;
  g.nodes_first = -1;
  g.num_nodes = 0;
  auto add_builtin = [&](int builtin) {
    int i = g.num_nodes++;
    g.regs[i].builtin_code = builtin;
    g.regs[i].custom_name = nullptr;
    g.regs[i].version = 1;
  };
  auto add_custom = [&]() {
    int i = g.num_nodes++;
    g.regs[i].builtin_code = kTfLiteBuiltinCustom;
    g.regs[i].custom_name = "lgnpu_custom_op";
    g.regs[i].version = 1;
    g.nodes[i].custom_initial_data = g.payload;
    g.nodes[i].custom_initial_data_size = sizeof g.payload;
  };
  if (strcmp(kind, "builtin") == 0) { add_builtin(kTfLiteBuiltinConv2d); add_builtin(kTfLiteBuiltinAdd); }
  else if (strcmp(kind, "custom") == 0) add_custom();
  else if (strcmp(kind, "custom+add") == 0) { add_custom(); add_builtin(kTfLiteBuiltinAdd); }
  else if (strcmp(kind, "add+custom") == 0) { add_builtin(kTfLiteBuiltinAdd); add_custom(); }
  else if (strcmp(kind, "unnamed-custom") == 0) { add_custom(); g.regs[0].custom_name = nullptr; }
}

typedef TfLiteDelegate* (*CreateFn)(char**, char**, size_t, void (*)(const char*));
typedef void (*DestroyFn)(TfLiteDelegate*);

void ReportPlugin(const char* msg) { printf("  plugin report_error: %s\n", msg); }


// Where a crash happened, as module and nearest exported symbol.
void OnCrash(int sig, siginfo_t* info, void* context) {
  const ucontext_t* uc = static_cast<const ucontext_t*>(context);
  uintptr_t pc = uc->uc_mcontext.arm_pc, lr = uc->uc_mcontext.arm_lr;
  Dl_info a, b;
  bool ha = dladdr(reinterpret_cast<void*>(pc | 1), &a) != 0, hb = dladdr(reinterpret_cast<void*>(lr | 1), &b) != 0;
  printf("CRASH signal %d fault=%p pc=%s+0x%lx (%s) lr=%s+0x%lx (%s)\n", sig, info->si_addr, ha ? a.dli_fname : "?",
         ha ? static_cast<unsigned long>(pc - reinterpret_cast<uintptr_t>(a.dli_fbase)) : 0, ha && a.dli_sname ? a.dli_sname : "no symbol",
         hb ? b.dli_fname : "?", hb ? static_cast<unsigned long>(lr - reinterpret_cast<uintptr_t>(b.dli_fbase)) : 0,
         hb && b.dli_sname ? b.dli_sname : "no symbol");
  fflush(stdout);
  _exit(100 + sig);
}


// What the delegate's kernel does once a partition is claimed: init with the partition's parameters, prepare and invoke
// on one float32 [1,4,4,1] input and output, free. Calls are plain C function pointers from the registration it handed
// over; libnnrt is the fake, so its log lines show the hand-over (package bytes, tensors, priority).
void Lifecycle(TfLiteDelegate* d, TfLiteContext* ctx) {
  typedef void* (*InitFn)(TfLiteContext*, const char*, size_t);
  typedef void (*FreeFn)(TfLiteContext*, void*);
  typedef TfLiteStatus (*NodeFn)(TfLiteContext*, TfLiteNode*);
  InitFn init = reinterpret_cast<InitFn>(g.reg_words[0]);
  FreeFn free_fn = reinterpret_cast<FreeFn>(g.reg_words[1]);
  NodeFn prepare = reinterpret_cast<NodeFn>(g.reg_words[2]);
  NodeFn invoke = reinterpret_cast<NodeFn>(g.reg_words[3]);

  static int dims_storage[5] = {4, 1, 4, 4, 1};
  static float in_buf[16], out_buf[16];
  for (int i = 0; i < 2; ++i) {
    TfLiteTensor* t = &g.tensors[i];
    memset(t, 0, sizeof *t);
    t->type = kTfLiteFloat32;
    t->dims = reinterpret_cast<TfLiteIntArray*>(dims_storage);
    t->bytes = sizeof in_buf;
    t->data.raw = reinterpret_cast<char*>(i == 0 ? in_buf : out_buf);
    t->allocation_type = kTfLiteArenaRw;
    t->quantization.type = kTfLiteNoQuantization;
  }
  for (int i = 0; i < 16; ++i) in_buf[i] = 0.25f;

  TfLiteIntArray* nodes = TfLiteIntArrayCreate(1);
  nodes->data[0] = 0;
  TfLiteIntArray* ins = TfLiteIntArrayCreate(1);
  ins->data[0] = 0;
  TfLiteIntArray* outs = TfLiteIntArrayCreate(1);
  outs->data[0] = 1;
  TfLiteDelegateParams params = {d, nodes, ins, outs};

  printf("  -- init(params)\n");
  void* user = init(ctx, reinterpret_cast<const char*>(&params), 0);  // TFLite: AddNodeWithParameters passes the params with length 0
  printf("  init returned user_data=%s\n", user ? "set" : "NULL");
  TfLiteNode node = {};
  node.inputs = ins;
  node.outputs = outs;
  node.user_data = user;
  node.delegate = d;
  node.custom_initial_data = g.payload;
  node.custom_initial_data_size = sizeof g.payload;
  printf("  -- prepare(node)\n");
  TfLiteStatus st = prepare(ctx, &node);
  printf("  prepare -> %d\n", static_cast<int>(st));
  printf("  -- invoke(node)\n");
  st = invoke(ctx, &node);
  printf("  invoke -> %d; output[0]=%g (input 0.25)\n", static_cast<int>(st), static_cast<double>(out_buf[0]));
  printf("  -- free(user_data)\n");
  free_fn(ctx, user);
  free(nodes);
  free(ins);
  free(outs);
}

}  // namespace

int main(int argc, char** argv) {
  setvbuf(stdout, nullptr, _IOLBF, 0);
  struct sigaction sa = {};
  sa.sa_sigaction = OnCrash;
  sa.sa_flags = SA_SIGINFO;
  sigaction(SIGSEGV, &sa, nullptr);
  if (argc < 2) {
    fprintf(stderr, "usage: %s DELEGATE.so\n", argv[0]);
    return 2;
  }
  void* h = dlopen(argv[1], RTLD_NOW);
  if (!h) {
    printf("dlopen failed: %s\n", dlerror());
    return 1;
  }
  CreateFn create = reinterpret_cast<CreateFn>(dlsym(h, "tflite_plugin_create_delegate"));
  DestroyFn destroy = reinterpret_cast<DestroyFn>(dlsym(h, "tflite_plugin_destroy_delegate"));
  printf("loaded %s; tflite_plugin_create_delegate=%s tflite_plugin_destroy_delegate=%s\n", argv[1], create ? "yes" : "no",
         destroy ? "yes" : "no");
  if (!create || !destroy) return 1;

  auto make = [&](std::initializer_list<const char*> kv) -> TfLiteDelegate* {
    char* keys[8];
    char* values[8];
    size_t n = 0;
    for (auto it = kv.begin(); it != kv.end(); it += 2, ++n) {
      keys[n] = const_cast<char*>(it[0]);
      values[n] = const_cast<char*>(it[1]);
    }
    return create(keys, values, n, ReportPlugin);
  };

  printf("\n[options]\n");
  struct {
    const char* label;
    std::initializer_list<const char*> kv;
  } option_cases[] = {
      {"no options", {}},
      {"min_freq=30", {"min_freq", "30"}},
      {"use_custom_alloc=true", {"use_custom_alloc", "true"}},
      {"use_custom_alloc=false", {"use_custom_alloc", "false"}},
      {"min_freq=30 use_custom_alloc=true", {"min_freq", "30", "use_custom_alloc", "true"}},
      {"bogus=1", {"bogus", "1"}},
      {"min_freq=fast", {"min_freq", "fast"}},
  };
  for (auto& c : option_cases) {
    fflush(stdout);
    pid_t pid = fork();  // an option the delegate cannot parse throws out of the create call
    if (pid == 0) {
      TfLiteDelegate* d = make(c.kv);
      printf("  %-36s -> %s\n", c.label, d ? "delegate created" : "NULL");
      if (d) {
        printf("      flags=0x%llx (1 = AllowDynamicTensors) Prepare=%s CopyFromBufferHandle=%s CopyToBufferHandle=%s FreeBufferHandle=%s\n",
               static_cast<unsigned long long>(d->flags), d->Prepare ? "set" : "null", d->CopyFromBufferHandle ? "set" : "null",
               d->CopyToBufferHandle ? "set" : "null", d->FreeBufferHandle ? "set" : "null");
        destroy(d);
      }
      fflush(stdout);
      _exit(0);
    }
    int status = 0;
    waitpid(pid, &status, 0);
    if (WIFSIGNALED(status)) printf("  %-36s -> CRASH (signal %d: an exception left the create call)\n", c.label, WTERMSIG(status));
  }

  printf("\n[Prepare on different graphs; TfLiteRegistration by value taken as N words]\n");
  const char* graphs[] = {"builtin", "custom", "custom+add", "add+custom", "unnamed-custom"};
  for (const char* graph : graphs) {
    for (int variant : {8, 9, 10, 11}) {
      fflush(stdout);
      pid_t pid = fork();  // the delegate may crash on a graph it does not expect; keep going
      if (pid == 0) {
        TfLiteDelegate* d = make({});
        if (!d) _exit(3);
        TfLiteContext ctx;
        SetupContext(&ctx, variant);
        SetupGraph(graph);
        g.delegate = d;
        printf("  graph=%-14s registration=%2d words: ", graph, variant);
        fflush(stdout);
        TfLiteStatus st = d->Prepare(&ctx, d);
        printf("Prepare -> %d; Replace called=%d nodes_to_replace.size=%d first=%d delegate_ok=%d\n", static_cast<int>(st),
               g.replaced, g.nodes_size, g.nodes_first, g.delegate_seen == d);
        if (g.replaced && g.nodes_size == 1 && g.delegate_seen == d) {
          printf("      registration words:");
          for (int i = 0; i < g.reg_words_seen; ++i) printf(" %08lx", static_cast<unsigned long>(g.reg_words[i]));
          printf("\n");
        }
        if (variant == 10 && g.replaced && g.nodes_size == 1 && strcmp(graph, "custom") == 0) Lifecycle(d, &ctx);
        fflush(stdout);
        _exit(0);
      }
      int status = 0;
      waitpid(pid, &status, 0);
      if (WIFSIGNALED(status)) printf("  crashed with signal %d\n", WTERMSIG(status));
    }
  }

  printf("\n[kernel lifecycle with use_custom_alloc=true]\n");
  fflush(stdout);
  pid_t pid = fork();
  if (pid == 0) {
    TfLiteDelegate* d = make({"use_custom_alloc", "true"});
    if (!d) _exit(3);
    TfLiteContext ctx;
    SetupContext(&ctx, 10);
    SetupGraph("custom");
    g.delegate = d;
    TfLiteStatus st = d->Prepare(&ctx, d);
    printf("  Prepare -> %d; nodes_to_replace.size=%d\n", static_cast<int>(st), g.nodes_size);
    if (g.replaced && g.nodes_size == 1) Lifecycle(d, &ctx);
    fflush(stdout);
    _exit(0);
  }
  int wait_status = 0;
  waitpid(pid, &wait_status, 0);
  if (WIFSIGNALED(wait_status)) printf("  crashed with signal %d\n", WTERMSIG(wait_status));
  return 0;
}
