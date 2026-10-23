#ifndef PAWS_SING_PROVIDER_COMPILER_H
#define PAWS_SING_PROVIDER_COMPILER_H
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
int paws_set_provider_compiler(void *callback);
void *paws_compile_provider(uint32_t kind, uint32_t behavior, uint32_t format,
                           const uint8_t *bytes, size_t length, size_t *output_length);
#endif
