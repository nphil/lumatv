/* Symbol discovery only: deliberately never calls lgnpu_init/prepare/invoke.
 * dlopen can run vendor constructors, so this is not the first read-only probe.
 */
#define _POSIX_C_SOURCE 200809L
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(int argc, char **argv)
{
    static const char *const symbols[] = {
        "lgnpu_init", "lgnpu_prepare", "lgnpu_invoke", "lgnpu_free",
        "lgnpu_set_affinity", "lgnpu_set_period"
    };
    if (argc == 1 || (argc == 2 && strcmp(argv[1], "--help") == 0)) {
        puts("usage: LUMATV_NPU_EXPERIMENT=1 lumatv-npu-probe --symbols [absolute-libnpu-path]");
        puts("default: disabled; no library loaded and no device opened");
        return 0;
    }
    if ((argc != 2 && argc != 3) || strcmp(argv[1], "--symbols") != 0) {
        fputs("invalid arguments\n", stderr);
        return 2;
    }
    const char *enabled = getenv("LUMATV_NPU_EXPERIMENT");
    if (!enabled || strcmp(enabled, "1") != 0) {
        fputs("disabled: explicit experiment switch required\n", stderr);
        return 2;
    }
    const char *path = argc == 3 ? argv[2] : "/usr/lib/libnpu.so.1.0.0";
    if (path[0] != '/') {
        fputs("absolute library path required\n", stderr);
        return 2;
    }
    void *lib = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    if (!lib) {
        fprintf(stderr, "dlopen: %s\n", dlerror());
        return 1;
    }
    int missing = 0;
    for (size_t i = 0; i < sizeof(symbols) / sizeof(symbols[0]); ++i) {
        dlerror();
        void *address = dlsym(lib, symbols[i]);
        const char *error = dlerror();
        const int found = address && !error;
        printf("%s=%s\n", symbols[i], found ? "present" : "missing");
        if (!found) missing = 1;
    }
    puts("hardware_execution_verified=false");
    if (dlclose(lib) != 0) {
        fputs("dlclose failed\n", stderr);
        return 1;
    }
    return missing;
}
