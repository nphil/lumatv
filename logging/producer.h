#ifndef LUMATV_LOG_PRODUCER_H
#define LUMATV_LOG_PRODUCER_H

#include <errno.h>
#include <stdarg.h>
#include <stdio.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/un.h>
#include <stdatomic.h>
#include <unistd.h>

static _Atomic int lumatv_log_fd = -1;

static inline void lumatv_log_reset(void) {
    int fd = atomic_exchange(&lumatv_log_fd, -1);
    if (fd >= 0) close(fd);
}

/* A missing/full collector never falls back to stderr or a retained file. */
static void lumatv_logv(const char *tag, unsigned severity, const char *fmt, va_list ap) {
    int saved_errno = errno;
    int fd = atomic_load_explicit(&lumatv_log_fd, memory_order_relaxed);
    if (fd < 0) {
        int fresh = socket(AF_UNIX, SOCK_DGRAM | SOCK_NONBLOCK | SOCK_CLOEXEC, 0);
        if (fresh >= 0) {
            int expected = -1;
            if (!atomic_compare_exchange_strong(&lumatv_log_fd, &expected, fresh)) { close(fresh); fd = expected; }
            else fd = fresh;
        }
    }
    if (fd >= 0) {
        char record[4097];
        int header = snprintf(record, sizeof record, "<%u>%s: ", 128 + (severity & 7), tag);
        if (header > 0 && header < 100) {
            int wrote = vsnprintf(record + header, sizeof record - (size_t)header, fmt, ap);
            size_t n = wrote < 0 ? (size_t)header : (size_t)header + (size_t)wrote;
            if (n > 4096) { n = 4096; memcpy(record + n - 3, "...", 3); }
            /* Validate UTF-8 in place, including a multibyte character cut by snprintf. */
            for (size_t i = (size_t)header; i < n;) {
                unsigned char c = (unsigned char)record[i];
                if (c < 128) { if (c < 32 || c == 127) record[i] = ' '; i++; continue; }
                size_t len = c >= 0xc2 && c <= 0xdf ? 2 : c >= 0xe0 && c <= 0xef ? 3 : c >= 0xf0 && c <= 0xf4 ? 4 : 0;
                int valid = len && i + len <= n;
                for (size_t j = 1; valid && j < len; j++) valid = ((unsigned char)record[i+j] & 0xc0) == 0x80;
                if (valid && len >= 3) {
                    unsigned char b = (unsigned char)record[i+1];
                    if ((c == 0xe0 && b < 0xa0) || (c == 0xed && b >= 0xa0) || (c == 0xf0 && b < 0x90) || (c == 0xf4 && b >= 0x90)) valid = 0;
                }
                if (valid) i += len;
                else record[i++] = '?';
            }
            struct sockaddr_un address = { .sun_family = AF_UNIX };
            memcpy(address.sun_path, "/tmp/lumatv-log.sock", sizeof "/tmp/lumatv-log.sock");
            (void)sendto(fd, record, n, MSG_DONTWAIT | MSG_NOSIGNAL, (struct sockaddr *)&address, sizeof address);
        }
    }
    errno = saved_errno;
}

static void lumatv_log(const char *tag, unsigned severity, const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    lumatv_logv(tag, severity, fmt, ap);
    va_end(ap);
}

#endif
