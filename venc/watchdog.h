/* A watchdog around every call that can block in the kernel (ioctls on /dev/venc, the tap and HMA, poll,
 * mmap): armed with a budget before the call, disarmed after. When the budget runs out the process prints
 * what it was waiting on and ends itself (exit 75), so a slot step never sits in a hung call for longer
 * than its budget. A call that is stuck uninterruptibly inside the kernel cannot be ended this way (the
 * signal stays pending; the slot script's own kill-and-check notices that case); every other hang can. */
#ifndef LUMATV_WATCHDOG_H
#define LUMATV_WATCHDOG_H

/* Installs the SIGALRM handler once. Safe to call more than once. */
void wd_install(void);

/* Arms the watchdog for ms milliseconds; what is kept by pointer, so it must outlive the call. */
void wd_arm(const char *what, unsigned ms);

void wd_disarm(void);

/* The number of arms so far (for the tests). */
unsigned long wd_count(void);

/* Budgets, in ms. The MCU's own encode wait is 100 ms per frame in record mode; a memory-to-memory frame is
 * tens of ms; the tap's plane-property call waits up to 300 ms in the driver. */
#define WD_GATE_MS 2000
#define WD_CREATE_MS 5000
#define WD_BUFINFO_MS 2000
#define WD_ENCODE_MS 3000
#define WD_PARAM_MS 2000
#define WD_DESTROY_MS 5000
#define WD_MMAP_MS 3000
#define WD_HMA_MS 3000
#define WD_TAP_CTRL_MS 3000
#define WD_TAP_BUFS_MS 5000
#define WD_TAP_STREAM_MS 5000
#define WD_TAP_STOP_MS 10000
#define WD_POLL_EXTRA_MS 1000

#endif
