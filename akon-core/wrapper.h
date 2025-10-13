/* Wrapper header for libopenconnect FFI bindings */

#include <openconnect.h>

/* C shim for progress callback with variadic args */
void progress_shim(void *privdata, int level, const char *fmt, ...);
