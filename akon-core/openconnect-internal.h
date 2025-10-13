/* Minimal openconnect-internal.h excerpt for ssl_read/ssl_write access
 * This allows us to diagnose and potentially fix the null function pointer issue
 * Source: https://gitlab.com/openconnect/openconnect
 */

#ifndef OPENCONNECT_INTERNAL_H
#define OPENCONNECT_INTERNAL_H

#include <openconnect.h>

/* Forward declaration - we only need the function pointer types */
struct openconnect_info_internal {
    /* ... many fields we don't care about ... */

    /* These are at specific offsets - we need to find them */
    int (*ssl_read)(struct openconnect_info *vpninfo, char *buf, size_t len);
    int (*ssl_gets)(struct openconnect_info *vpninfo, char *buf, size_t len);
    int (*ssl_write)(struct openconnect_info *vpninfo, char *buf, size_t len);
};

/* Cast to access internal fields (UNSAFE - for debugging only) */
#define INTERNAL_INFO(vpn) ((struct openconnect_info_internal *)(vpn))

#endif /* OPENCONNECT_INTERNAL_H */
