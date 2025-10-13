/* C shim for OpenConnect progress callback with variadic args
 * This forwards progress messages to stdout/stderr based on level
 */
#include <stdarg.h>
#include <stdio.h>

/* Progress levels from openconnect.h */
#define PRG_ERR     0
#define PRG_INFO    1
#define PRG_DEBUG   2
#define PRG_TRACE   3

/* Progress callback that forwards to stdout/stderr like OpenConnect CLI does */
void progress_shim(void *privdata, int level, const char *fmt, ...) {
    va_list args;
    FILE *out;

    (void)privdata;  /* unused */

    /* Use stderr for errors, stdout for everything else */
    out = (level == PRG_ERR) ? stderr : stdout;

    va_start(args, fmt);
    vfprintf(out, fmt, args);
    va_end(args);

    fflush(out);
}
