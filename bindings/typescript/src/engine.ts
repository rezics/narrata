export interface WasmProtocolEngine {
  call(request: Uint8Array): Uint8Array;
}

export interface ProtocolCodec<TRequest, TResponse> {
  encode(request: TRequest): Uint8Array;
  decode(response: Uint8Array): TResponse;
  isSliceYielded(response: TResponse): boolean;
  continueAfter(response: TResponse): TRequest;
}

/**
 * Copies bytes at the Wasm boundary and exposes the pull protocol as Promises or an async stream.
 * The codec should be generated from `narrata.proto`; runtime domain objects are not accepted here.
 */
export class NarrataEngine<TRequest, TResponse> {
  readonly #wasm: WasmProtocolEngine;
  readonly #codec: ProtocolCodec<TRequest, TResponse>;

  constructor(wasm: WasmProtocolEngine, codec: ProtocolCodec<TRequest, TResponse>) {
    this.#wasm = wasm;
    this.#codec = codec;
  }

  async call(request: TRequest): Promise<TResponse> {
    const encoded = this.#codec.encode(request).slice();
    const response = this.#wasm.call(encoded).slice();
    return this.#codec.decode(response);
  }

  async *pull(request: TRequest): AsyncGenerator<TResponse, TResponse> {
    let current = request;
    for (;;) {
      const response = await this.call(current);
      if (!this.#codec.isSliceYielded(response)) return response;
      yield response;
      current = this.#codec.continueAfter(response);
    }
  }
}
