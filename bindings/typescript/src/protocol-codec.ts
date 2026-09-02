import type { ProtocolCodec } from "./engine.js";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";
import {
  RequestSchema,
  ResponseSchema,
  type Request,
  type Response,
} from "./generated/narrata_pb.js";

/** Canonical Protobuf transport codec generated from the protocol shared with Rust and C#. */
export class NarrataProtocolCodec implements ProtocolCodec<Request, Response> {
  readonly #sliceWork: number;

  constructor(sliceWork = 0) {
    if (!Number.isSafeInteger(sliceWork) || sliceWork < 0) {
      throw new RangeError("Slice work must be a non-negative safe integer.");
    }
    this.#sliceWork = sliceWork;
  }

  encode(request: Request): Uint8Array {
    return toBinary(RequestSchema, request);
  }

  decode(response: Uint8Array): Response {
    return fromBinary(ResponseSchema, response);
  }

  isSliceYielded(response: Response): boolean {
    return response.body.case === "sliceYielded";
  }

  continueAfter(response: Response): Request {
    if (response.body.case !== "sliceYielded") {
      throw new TypeError("The response did not yield a protocol slice.");
    }
    if (response.requestId >= BigInt(Number.MAX_SAFE_INTEGER)) {
      throw new RangeError("The protocol request ID cannot be incremented safely.");
    }
    return create(RequestSchema, {
      protocolVersion: response.protocolVersion,
      requestId: response.requestId + 1n,
      body: {
        case: "continueSlice",
        value: {
          session: response.body.value.session,
          sliceWork: BigInt(this.#sliceWork),
        },
      },
    });
  }
}
