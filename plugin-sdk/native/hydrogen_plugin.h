#ifndef HYDROGEN_PLUGIN_V1_H
#define HYDROGEN_PLUGIN_V1_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif

/* Native code has full process privileges. The host serializes calls per instance.
 * No request, callback, context or response pointer may be retained after call returns.
 * Respond exactly once, synchronously, with at most 256 KiB of UTF-8 JSON.
 * Request: {"method":"...","params":...}
 * Response: {"ok":true,"value":...} or {"ok":false,"error":{"code":"...","message":"..."}}
 * plugin.describe must return {"id":"<manifest id>","abiVersion":1}.
 * destroy must finish all owned work and release callbacks before returning.
 * Exceptions/panics must never escape these function pointers. */
typedef void (*hydrogen_respond)(void *context, const uint8_t *bytes, size_t length);
typedef struct hydrogen_plugin_api_v1 {
    uint32_t abi_version;
    uint32_t struct_size;
    void *(*create)(void);
    int32_t (*call)(void *instance, const uint8_t *request, size_t length,
                    hydrogen_respond respond, void *context);
    void (*destroy)(void *instance);
} hydrogen_plugin_api_v1;

/* Optional extension: callbacks are synchronous and valid only during this invocation.
 * Service request: {"provider":"id","service":"id","method":"method","params":...}.
 * Uses the same result envelope and size bound. Provider must be a declared dependency.
 * No callback/context pointer may be retained or called from another thread. */
typedef int32_t (*hydrogen_service_call)(void *context, const uint8_t *request, size_t length,
                                      hydrogen_respond respond, void *reply_context);
int32_t hydrogen_plugin_services_v1(void *instance, const uint8_t *request, size_t length,
                                   hydrogen_respond respond, void *reply_context,
                                   hydrogen_service_call service, void *service_context);

/* Optional large-message capability. Same ABI signature, with an explicit 64 MiB
 * request/response limit for this invocation and its nested service callbacks.
 * Hosts without this capability keep using the original 256 KiB entry points. */
int32_t hydrogen_plugin_large_services_v1(void *instance, const uint8_t *request, size_t length,
                                         hydrogen_respond respond, void *reply_context,
                                         hydrogen_service_call service, void *service_context);

/* Export this symbol with the C calling convention and default visibility. */
const hydrogen_plugin_api_v1 *hydrogen_plugin_v1(void);
#ifdef __cplusplus
}
#endif
#endif
