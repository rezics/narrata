import "fake-indexeddb/auto";

import { describe, expect, test } from "vitest";
import { create, fromBinary, toBinary } from "@bufbuild/protobuf";

import { NarrataEngine } from "../src/engine.js";
import { NarrataIndexedDbStore } from "../src/indexeddb-store.js";
import { NarrataProtocolCodec } from "../src/protocol-codec.js";
import {
  RequestSchema,
  ResponseSchema,
} from "../src/generated/narrata_pb.js";

describe("shared protocol", () => {
  test("generated DTOs match the Rust EngineCreate wire vector", () => {
    const request = create(RequestSchema, {
      protocolVersion: 1,
      requestId: 7n,
      body: {
        case: "engineCreate",
        value: { maxMessageBytes: 0n, maxSliceWork: 0n },
      },
    });
    expect(hex(toBinary(RequestSchema, request))).toBe("080110075200");
    const response = fromBinary(
      ResponseSchema,
      fromHex("08011007520708011080808008"),
    );
    expect(response.requestId).toBe(7n);
    expect(response.body.case).toBe("engineCreated");
    if (response.body.case !== "engineCreated") throw new Error("expected engine response");
    expect(response.body.value.abiVersion).toBe(1);
    expect(response.body.value.maxMessageBytes).toBe(16_777_216n);
  });

  test("the TypeScript async iterator continues only the versioned pull protocol", async () => {
    let calls = 0;
    const wasm = {
      call(bytes: Uint8Array): Uint8Array {
        const request = fromBinary(RequestSchema, bytes);
        expect(request.protocolVersion).toBe(1);
        calls += 1;
        if (request.body.case === "dispatch") {
          return toBinary(ResponseSchema, create(ResponseSchema, {
            protocolVersion: 1,
            requestId: request.requestId,
            body: {
              case: "sliceYielded",
              value: {
                session: request.body.value.session,
                instructionCount: 1n,
                callCount: 0n,
                logicalAllocUnits: 0n,
                microstepCount: 0n,
                internalEventCount: 0n,
              },
            },
          }));
        }
        expect(request.body.case).toBe("continueSlice");
        if (request.body.case !== "continueSlice") throw new Error("expected continuation");
        expect(request.body.value.session).toBe(7n);
        return toBinary(ResponseSchema, create(ResponseSchema, {
          protocolVersion: 1,
          requestId: request.requestId,
          body: {
            case: "committed",
            value: {
              session: 7n,
              commitId: new Uint8Array(32),
              receiptId: new Uint8Array(32),
              snapshot: new Uint8Array([1]),
              stateDigest: new Uint8Array(32),
              reused: false,
              result: undefined,
            },
          },
        }));
      },
    };
    const engine = new NarrataEngine(wasm, new NarrataProtocolCodec(10));
    const iterator = engine.pull(create(RequestSchema, {
      protocolVersion: 1,
      requestId: 10n,
      body: {
        case: "dispatch",
        value: { session: 7n, input: undefined, sliceWork: 10n },
      },
    }));
    const yielded = await iterator.next();
    expect(yielded.done).toBe(false);
    expect(yielded.value.body.case).toBe("sliceYielded");
    const committed = await iterator.next();
    expect(committed.done).toBe(true);
    expect(committed.value.body.case).toBe("committed");
    if (committed.value.body.case !== "committed") throw new Error("expected commit");
    expect(committed.value.body.value.commitId).toHaveLength(32);
    expect(calls).toBe(2);
  });
});

function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}

function fromHex(value: string): Uint8Array {
  if (value.length % 2 !== 0) throw new TypeError("hex input must have an even length");
  return Uint8Array.from(value.match(/.{2}/g) ?? [], (byte) => Number.parseInt(byte, 16));
}

describe("IndexedDB store", () => {
  test("publishes immutable objects and CAS refs atomically", async () => {
    const store = await NarrataIndexedDbStore.open(`narrata-${crypto.randomUUID()}`);
    const first = { id: new Uint8Array([1]), bytes: new Uint8Array([10]) };
    await store.commit(
      [first],
      [{ key: "active/test/cursor", expectedRevision: null, nextObjectId: first.id }],
    );
    expect(await store.getObject(first.id)).toEqual(first.bytes);
    expect((await store.readRef("active/test/cursor"))?.revision).toBe(1);

    const unpublished = { id: new Uint8Array([2]), bytes: new Uint8Array([20]) };
    await expect(
      store.commit(
        [unpublished],
        [{ key: "active/test/cursor", expectedRevision: null, nextObjectId: unpublished.id }],
      ),
    ).rejects.toThrow("changed before this save could be published");
    expect(await store.getObject(unpublished.id)).toBeNull();
    expect((await store.readRef("active/test/cursor"))?.objectId).toEqual(first.id);
    store.close();
  });
});
