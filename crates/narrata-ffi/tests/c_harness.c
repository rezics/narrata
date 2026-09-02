#include "narrata.h"

typedef NarStatus (*CreateFn)(uint64_t *);
typedef NarStatus (*CallFn)(uint64_t, const uint8_t *, size_t, NarBuffer *);
typedef NarStatus (*FreeBufferFn)(uint64_t);
typedef NarStatus (*FreeEngineFn)(uint64_t);

NarStatus narrata_c_harness_roundtrip(
    CreateFn create_engine,
    CallFn call_engine,
    FreeBufferFn free_buffer,
    FreeEngineFn free_engine,
    const uint8_t *request,
    size_t request_len,
    NarBuffer *response) {
  uint64_t engine = 0;
  NarStatus status = create_engine(&engine);
  if (status != NAR_OK) return status;
  status = call_engine(engine, request, request_len, response);
  if (status != NAR_OK) {
    free_engine(engine);
    return status;
  }
  status = free_engine(engine);
  if (status != NAR_OK) {
    free_buffer(response->token);
    return status;
  }
  return NAR_OK;
}

NarStatus narrata_c_harness_trace(
    CreateFn create_engine,
    CallFn call_engine,
    FreeBufferFn free_buffer,
    FreeEngineFn free_engine,
    const uint8_t *load_request,
    size_t load_request_len,
    const uint8_t *create_request,
    size_t create_request_len,
    const uint8_t *dispatch_request,
    size_t dispatch_request_len,
    NarBuffer responses[3]) {
  uint64_t engine = 0;
  size_t completed = 0;
  NarStatus status = create_engine(&engine);
  if (status != NAR_OK) return status;

  status = call_engine(engine, load_request, load_request_len, &responses[0]);
  if (status == NAR_OK) completed = 1;
  if (status == NAR_OK) {
    status = call_engine(engine, create_request, create_request_len, &responses[1]);
    if (status == NAR_OK) completed = 2;
  }
  if (status == NAR_OK) {
    status = call_engine(engine, dispatch_request, dispatch_request_len, &responses[2]);
    if (status == NAR_OK) completed = 3;
  }
  NarStatus free_status = free_engine(engine);
  if (status == NAR_OK && free_status != NAR_OK) status = free_status;
  if (status != NAR_OK) {
    for (size_t i = 0; i < completed; ++i) free_buffer(responses[i].token);
  }
  return status;
}
