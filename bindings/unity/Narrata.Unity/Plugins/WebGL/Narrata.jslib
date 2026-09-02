mergeInto(LibraryManager.library, {
  NarrataWebBridge_Call: function(request, requestLength, responseOut, responseLengthOut) {
    if (!globalThis.narrataWasmEngine) return 1;
    const input = HEAPU8.slice(request, request + requestLength);
    let output;
    try {
      output = globalThis.narrataWasmEngine.call(input);
    } catch (_) {
      return 2;
    }
    const pointer = _malloc(output.length);
    HEAPU8.set(output, pointer);
    setValue(responseOut, pointer, '*');
    setValue(responseLengthOut, output.length, 'i32');
    return 0;
  },

  NarrataWebBridge_Free: function(response) {
    _free(response);
  }
});
