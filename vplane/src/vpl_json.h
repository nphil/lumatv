/* The one JSON object a request file holds: top-level keys with string, number or boolean values, nothing else is looked at.
 *
 * QML writes the file with JSON.stringify, so the text is well formed; this reads what it needs and skips what it does not know
 * (nested values included) without allocating. A key that is absent, or has another kind of value, gives the caller's default.
 */
#ifndef VPL_JSON_H
#define VPL_JSON_H

#include <stddef.h>
#include <stdint.h>

/* 1 when `json` is one object (leading and trailing space allowed), 0 otherwise. */
int vj_valid(const char *json, size_t len);

/* Integer, number, boolean, string for a top-level key. A number given as 3.0 or 3e2 reads as an integer too (JSON has one number kind). */
int64_t vj_int(const char *json, size_t len, const char *key, int64_t def);
double vj_num(const char *json, size_t len, const char *key, double def);
int vj_bool(const char *json, size_t len, const char *key, int def);
/* Copies the string with its escapes decoded (\" \\ \/ \n \t \uXXXX as UTF-8) into `out` (NUL ended, cut to `cap`); returns its length, -1 when the key is absent or is not a string. */
int vj_str(const char *json, size_t len, const char *key, char *out, size_t cap);

#endif
