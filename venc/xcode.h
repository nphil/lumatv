/* `lumatv-venc xcode`: the child of the transcode engine (xcode_proto.h). One job per process: records in on stdin,
 * H.264 units out on stdout, the hardware held from the first record to the end. */
#ifndef LUMATV_XCODE_H
#define LUMATV_XCODE_H

#include "sysops.h"

/* How long the job waits on the decoder before it gives up: from the first access unit to the first picture, from the last
 * picture while access units keep arriving, and for the end-of-stream marker. Only the tests change them. */
extern struct xcode_limits {
    double first_stall_s, stall_s, drain_s;
} xcode_limits;

/* Runs one job over the two descriptors until the input ends, SIGTERM or a failure; returns the exit status
 * (XC_EXIT_*). Every device it opened is closed on the way out. For the tests; the process mode is xcode_main. */
int xcode_run(const struct sysops *sys, int in_fd, int out_fd);

/* SIGTERM and SIGINT end a job cleanly, a closed output pipe is an error return rather than a signal. */
void xcode_signals(void);

/* The process mode: no core dumps, the parent's death and SIGTERM end the job cleanly, stdin and stdout are the pipes. */
int xcode_main(void);

#endif
