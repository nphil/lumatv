#ifndef LUMATV_LOG_STREAM_H
#define LUMATV_LOG_STREAM_H
#include "producer.h"
#include <fcntl.h>
#include <pthread.h>
#include <stdlib.h>
#include <unistd.h>

struct lumatv_log_drain { int fd; const char *tag; };
static void *lumatv_log_drain_run(void *arg) {
    struct lumatv_log_drain ctx = *(struct lumatv_log_drain *)arg;
    free(arg);
    char input[4096], line[4097];
    size_t len = 0;
    int cut = 0;
    for (;;) {
        ssize_t n = read(ctx.fd, input, sizeof input);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) break;
        for (ssize_t i = 0; i < n; i++) {
            if (input[i] == '\n') {
                line[len] = 0;
                lumatv_log(ctx.tag, 6, "%s%s", line, cut ? "..." : "");
                len = 0; cut = 0;
            } else if (len < sizeof line - 1) line[len++] = input[i];
            else cut = 1;
        }
    }
    if (len || cut) { line[len] = 0; lumatv_log(ctx.tag, 6, "%s%s", line, cut ? "..." : ""); }
    close(ctx.fd);
    return NULL;
}

/* Only the write end is nonblocking. No collector state can hold the native producer or its shutdown. */
static int lumatv_log_pipe(const char *tag) {
    int fds[2];
    if (pipe2(fds, O_CLOEXEC) != 0) return -1;
    struct lumatv_log_drain *ctx = malloc(sizeof *ctx);
    if (!ctx || fcntl(fds[1], F_SETFL, O_NONBLOCK) < 0) { free(ctx); close(fds[0]); close(fds[1]); return -1; }
    *ctx = (struct lumatv_log_drain){ fds[0], tag };
    pthread_t thread;
    if (pthread_create(&thread, NULL, lumatv_log_drain_run, ctx) != 0) { free(ctx); close(fds[0]); close(fds[1]); return -1; }
    pthread_detach(thread);
    return fds[1];
}
#endif
