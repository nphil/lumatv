// Stand-in for LG's libPmLogLib.so.3. The delegate logs its reasons through _PmLogMsgKV, whose exact signature is
// not published with the library; this prints every argument word that points at readable text, which includes the
// message id and the format string. It never dereferences a word it has not checked with write(2).
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <unistd.h>

void* PmLogGetContext(const char* name) {
  (void)name;
  static unsigned char context[256];
  return context;
}

static int readable(const void* p) {
  if ((uintptr_t)p < 0x10000) return 0;
  int fds[2];
  if (pipe(fds) != 0) return 0;
  ssize_t n = write(fds[1], p, 1);
  close(fds[0]);
  close(fds[1]);
  return n == 1;
}

static void text(const void* p) {
  if (!readable(p)) return;
  const unsigned char* s = (const unsigned char*)p;
  char out[240];
  int k = 0;
  while (k < 239 && s[k] >= 0x20 && s[k] < 0x7f) {
    out[k] = (char)s[k];
    ++k;
  }
  out[k] = 0;
  if (k > 3) fprintf(stderr, " | %s", out);
}

int _PmLogMsgKV(void* context, int level, unsigned a, const char* b, ...) {
  (void)context;
  fprintf(stderr, "PMLOG level=%d", level);
  text((const void*)(uintptr_t)a);
  text(b);
  va_list ap;
  va_start(ap, b);
  for (int i = 0; i < 8; ++i) text(va_arg(ap, const void*));
  va_end(ap);
  fprintf(stderr, "\n");
  return 0;
}
