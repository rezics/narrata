#ifndef NARRATA_H
#define NARRATA_H

#include <stddef.h>
#include <stdint.h>

#if defined(_WIN32)
#define NAR_API __declspec(dllimport)
#else
#define NAR_API
#endif

#ifdef __cplusplus
extern "C" {
#endif

typedef int32_t NarStatus;

enum {
  NAR_OK = 0,
  NAR_INVALID_ARGUMENT = 1,
  NAR_INVALID_HANDLE = 2,
  NAR_PROTOCOL_ERROR = 3,
  NAR_PANIC = 4,
  NAR_INTERNAL = 5
};

typedef struct NarBuffer {
  const uint8_t *data;
  size_t len;
  uint64_t token;
} NarBuffer;

NAR_API uint32_t nar_abi_version(void);
NAR_API NarStatus nar_engine_create(uint64_t *out_handle);
NAR_API NarStatus nar_engine_free(uint64_t handle);
NAR_API NarStatus nar_engine_call(
    uint64_t handle,
    const uint8_t *input,
    size_t input_len,
    NarBuffer *out_buffer);
NAR_API NarStatus nar_buffer_free(uint64_t token);

#ifdef __cplusplus
}
#endif

#endif
