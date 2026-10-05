import { readFileSync } from "node:fs";

import { describe, expect, it } from "vitest";

import { encode } from "../src/cbor";
import {
  type Flush,
  ProtocolError,
  decodeFlush,
  decodeFlushReply,
  decodeLoadRequest,
  decodeLoaded,
  encodeFlush,
  encodeFlushReply,
  encodeLoadRequest,
  encodeLoaded,
  hex,
} from "../src/protocol";

/** The vectors the Rust codec is tested against, so both sides agree byte for byte. */
const vectors = JSON.parse(
  readFileSync(new URL("../../crates/narrata-storage-host/tests/protocol-vectors.json", import.meta.url), "utf8"),
);

const bytes = (value: string) => Uint8Array.from(value.match(/../g) ?? [], (byte) => Number.parseInt(byte, 16));
const optional = (value: string | null) => (value === null ? null : bytes(value));
const range = ([lower, upper, limit]: [string, string | null, number]) => ({ lower: bytes(lower), upper: optional(upper), limit });

describe("host messages", () => {
  it("decode and re-encode the shared vectors", () => {
    const request = decodeLoadRequest(bytes(vectors.loadRequest.hex));
    expect(request).toEqual({
      keys: vectors.loadRequest.keys.map(bytes),
      objects: vectors.loadRequest.objects.map(bytes),
      keyRanges: vectors.loadRequest.keyRanges.map(range),
      objectRanges: vectors.loadRequest.objectRanges.map(range),
    });
    expect(hex(encodeLoadRequest(request))).toBe(vectors.loadRequest.hex);

    const loaded = decodeLoaded(bytes(vectors.loaded.hex));
    expect(loaded.store).toEqual({ id: bytes(vectors.loaded.store[0]), revision: vectors.loaded.store[1] });
    expect(loaded.keys[0]).toEqual([bytes("0004616263"), { value: bytes("76"), revision: 9 }]);
    expect(loaded.objects[1]).toEqual([bytes("22".repeat(32)), null]);
    expect(loaded.keyRanges[0]![1]).toEqual([[bytes("000461"), { value: new Uint8Array(), revision: 2 }]]);
    expect(hex(encodeLoaded(loaded))).toBe(vectors.loaded.hex);

    const flush = decodeFlush(bytes(vectors.flush.hex));
    expect(flush.batches.map((batch) => [batch.base, batch.revision])).toEqual([
      [9, 10],
      [10, 11],
    ]);
    expect(flush.batches[0]!.deleteKeys).toEqual([bytes("0004616264")]);
    expect(hex(encodeFlush(flush))).toBe(vectors.flush.hex);

    expect(decodeFlushReply(bytes(vectors.persisted.hex))).toEqual({ persisted: 11 });
    expect(hex(encodeFlushReply({ persisted: 11 }))).toBe(vectors.persisted.hex);
    const conflict = { id: bytes(vectors.conflict.store[0]), revision: vectors.conflict.store[1] };
    expect(decodeFlushReply(bytes(vectors.conflict.hex))).toEqual({ conflict });
    expect(hex(encodeFlushReply({ conflict }))).toBe(vectors.conflict.hex);
  });

  it("refuse unordered or inconsistent requests and flushes", () => {
    const request = decodeLoadRequest(bytes(vectors.loadRequest.hex));
    const refusedRequests = [
      { ...request, keys: [bytes("000402"), bytes("000401")] },
      { ...request, objects: [bytes("11".repeat(31))] },
      { ...request, keyRanges: [{ lower: bytes("0005"), upper: bytes("0005"), limit: 1 }] },
      { ...request, objectRanges: [{ lower: new Uint8Array(), upper: null, limit: 0 }] },
      {
        ...request,
        // A range without an upper bound sorts before one with it, as Rust orders `Option`.
        keyRanges: [
          { lower: bytes("0004"), upper: bytes("0005"), limit: 1 },
          { lower: bytes("0004"), upper: null, limit: 1 },
        ],
      },
    ];
    for (const refused of refusedRequests) {
      expect(() => decodeLoadRequest(encodeLoadRequest(refused))).toThrow(ProtocolError);
    }

    const flush = decodeFlush(bytes(vectors.flush.hex));
    const changed = (change: (flush: Flush) => void) => {
      const copy = structuredClone(flush);
      change(copy);
      return encodeFlush(copy);
    };
    const refusedFlushes = [
      changed((flush) => (flush.batches = [])),
      changed((flush) => (flush.batches[1]!.base = 11)),
      changed((flush) => (flush.batches[1]!.revision = 12)),
      changed((flush) => (flush.batches[0]!.deleteKeys = [flush.batches[0]!.putKeys[0]![0]])),
      changed((flush) => (flush.batches[0]!.deleteKeys = [bytes("00")])),
      changed((flush) => (flush.store = bytes("07"))),
    ];
    for (const refused of refusedFlushes) expect(() => decodeFlush(refused)).toThrow(ProtocolError);
  });

  it("refuse other versions and unknown replies", () => {
    expect(() => decodeFlushReply(encode([2, 0, 11]))).toThrow(ProtocolError);
    expect(() => decodeFlushReply(encode([1, 1, 11]))).toThrow(ProtocolError);
    expect(() => decodeLoadRequest(encode([2, [], [], [], []]))).toThrow(ProtocolError);
    expect(() => decodeLoaded(encode([1, new Uint8Array(16), 1, [[bytes("0001"), [new Uint8Array(), 2]]], [], [], []]))).toThrow(
      ProtocolError,
    );
  });
});
