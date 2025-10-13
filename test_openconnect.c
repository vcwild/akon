#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <openconnect.h>

static int process_auth_form_cb(void *privdata, struct oc_auth_form *form) {
    printf("Auth form callback called\n");

    // Fill username and password fields
    struct oc_form_opt *opt;
    for (opt = form->opts; opt; opt = opt->next) {
        if (opt->name) {
            printf("Field: %s\n", opt->name);
            if (strstr(opt->name, "user") || strstr(opt->name, "name")) {
                opt->_value = strdup("vicwil");
            } else if (strstr(opt->name, "pass") || strstr(opt->name, "secret")) {
                opt->_value = strdup("test123");
            }
        }
    }
    return 0;
}

int main() {
    printf("Initializing OpenConnect...\n");

    // Initialize SSL
    if (openconnect_init_ssl() != 0) {
        fprintf(stderr, "Failed to init SSL\n");
        return 1;
    }

    printf("Creating vpninfo...\n");
    struct openconnect_info *vpn = openconnect_vpninfo_new(
        NULL,  // useragent
        NULL,  // validate_peer_cert
        NULL,  // write_new_config
        process_auth_form_cb,  // process_auth_form
        NULL,  // progress
        NULL   // privdata
    );

    if (!vpn) {
        fprintf(stderr, "Failed to create vpninfo\n");
        return 1;
    }

    printf("Setting protocol to f5...\n");
    if (openconnect_set_protocol(vpn, "f5") != 0) {
        fprintf(stderr, "Failed to set protocol\n");
        return 1;
    }

    printf("Parsing URL...\n");
    if (openconnect_parse_url(vpn, "https://access.etraveligroup.com") != 0) {
        fprintf(stderr, "Failed to parse URL\n");
        return 1;
    }

    printf("Disabling DTLS...\n");
    openconnect_disable_dtls(vpn);

    printf("Obtaining cookie (this will make HTTPS requests)...\n");
    int ret = openconnect_obtain_cookie(vpn);
    printf("openconnect_obtain_cookie returned: %d\n", ret);

    if (ret != 0) {
        fprintf(stderr, "Authentication failed\n");
    } else {
        printf("Authentication successful!\n");
    }

    openconnect_vpninfo_free(vpn);
    return ret;
}
