#include "provider_compiler.h"
#include <stdatomic.h>

typedef void *(*paws_provider_compiler)(uint32_t, uint32_t, uint32_t,
                                      const uint8_t *, size_t, size_t *);
static _Atomic(uintptr_t) paws_provider_compiler_address;

int paws_set_provider_compiler(void *callback) {
    if (callback == NULL) return 0;
    uintptr_t expected = 0;
    uintptr_t address = (uintptr_t)callback;
    if (atomic_compare_exchange_strong(&paws_provider_compiler_address, &expected, address)) return 1;
    return expected == address;
}
void *paws_compile_provider(uint32_t kind, uint32_t behavior, uint32_t format,
                           const uint8_t *bytes, size_t length, size_t *output_length) {
    if (output_length == NULL) return NULL;
    *output_length = 0;
    uintptr_t address = atomic_load(&paws_provider_compiler_address);
    if (address == 0) return NULL;
    return ((paws_provider_compiler)address)(kind, behavior, format, bytes, length, output_length);
}
