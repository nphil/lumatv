#include "watchdog.h"

#include <signal.h>
#include <stdio.h>
#include <string.h>
#include <sys/time.h>
#include <unistd.h>

static volatile const char *wd_what;
static volatile unsigned wd_ms;
static unsigned long wd_arms;
static int wd_installed;

static void wd_fire(int sig)
{
    (void)sig;
    char line[192];
    const char *w = (const char *)wd_what;
    int n = snprintf(line, sizeof(line), "watchdog: %s did not return within %u ms; ending the process\n", w ? w : "a call", (unsigned)wd_ms);
    if (n > 0) {
        ssize_t r = write(2, line, (size_t)n);
        (void)r;
    }
    _exit(75);
}

void wd_install(void)
{
    if (wd_installed)
        return;
    struct sigaction sa;
    memset(&sa, 0, sizeof(sa));
    sa.sa_handler = wd_fire;
    sigemptyset(&sa.sa_mask);
    sigaction(SIGALRM, &sa, NULL);
    wd_installed = 1;
}

void wd_arm(const char *what, unsigned ms)
{
    wd_install();
    wd_what = what;
    wd_ms = ms;
    wd_arms++;
    struct itimerval it = { { 0, 0 }, { ms / 1000, (ms % 1000) * 1000 } };
    setitimer(ITIMER_REAL, &it, NULL);
}

void wd_disarm(void)
{
    struct itimerval it = { { 0, 0 }, { 0, 0 } };
    setitimer(ITIMER_REAL, &it, NULL);
    wd_what = NULL;
}

unsigned long wd_count(void)
{
    return wd_arms;
}
