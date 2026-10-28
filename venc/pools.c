#include "pools.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

unsigned pools_parse(struct pools *ps, const char *text)
{
    ps->n = 0;
    const char *line = text;
    while (line && *line && ps->n < POOLS_MAX) {
        const char *nl = strchr(line, '\n');
        size_t len = nl ? (size_t)(nl - line) : strlen(line);
        char buf[128];
        if (len < sizeof(buf)) {
            memcpy(buf, line, len);
            buf[len] = 0;
            char *end = NULL;
            uint64_t base = strtoull(buf, &end, 16);
            if (end && end != buf && (*end == '\t' || *end == ' ')) {
                char *end2 = NULL;
                uint64_t size = strtoull(end + 1, &end2, 16);
                if (end2 && end2 != end + 1 && (*end2 == '\t' || *end2 == ' ') && size) {
                    const char *name = end2 + 1;
                    while (*name == ' ' || *name == '\t')
                        name++;
                    struct pool *p = &ps->p[ps->n++];
                    p->base = base;
                    p->size = size;
                    snprintf(p->name, sizeof(p->name), "%s", name);
                }
            }
        }
        line = nl ? nl + 1 : NULL;
    }
    return ps->n;
}

unsigned pools_read(struct pools *ps, const char *path)
{
    ps->n = 0;
    FILE *f = fopen(path ? path : "/proc/hma/status", "r");
    if (!f)
        return 0;
    char text[8192];
    size_t got = fread(text, 1, sizeof(text) - 1, f);
    fclose(f);
    text[got] = 0;
    return pools_parse(ps, text);
}

const struct pool *pools_find(const struct pools *ps, uint64_t paddr, uint64_t len)
{
    for (unsigned i = 0; i < ps->n; i++)
        if (paddr >= ps->p[i].base && paddr + len <= ps->p[i].base + ps->p[i].size)
            return &ps->p[i];
    return NULL;
}
