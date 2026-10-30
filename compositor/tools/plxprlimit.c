// plxprlimit PID: lets PID dump a core (RLIMIT_CORE unlimited) and keeps the dump small (coredump_filter 1: anonymous private memory only).
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <sys/resource.h>

int main(int argc, char **argv) {
    if (argc != 2) { fprintf(stderr, "usage: plxprlimit PID\n"); return 2; }
    int pid = atoi(argv[1]);
    struct rlimit r = { RLIM_INFINITY, RLIM_INFINITY };
    if (prlimit(pid, RLIMIT_CORE, &r, NULL) != 0) { perror("prlimit"); return 1; }
    char path[64];
    snprintf(path, sizeof path, "/proc/%d/coredump_filter", pid);
    FILE *f = fopen(path, "w");
    if (f) { fputs("1\n", f); fclose(f); }
    printf("pid %d: core unlimited, filter 1\n", pid);
    return 0;
}
