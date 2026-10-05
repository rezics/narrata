import "fake-indexeddb/auto";

import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { IDBFactory } from "fake-indexeddb";
import { describe, expect, it } from "vitest";

import { decode, encode } from "../src/cbor";
import { IndexedDbStore } from "../src/indexeddb";
import {
  decodeFlush, decodeFlushReply, decodeLoaded, decodeLoadRequest,
  encodeFlush, encodeFlushReply, encodeLoaded, encodeLoadRequest,
} from "../src/protocol";

const corpus = new URL("../../../../../fixtures/compat/browser-storage-v1/", import.meta.url);
const manifest = JSON.parse(readFileSync(new URL("manifest.json", corpus), "utf8"));
const artifact = (file: string) => {
  const bytes = readFileSync(new URL(file, corpus));
  expect(createHash("sha256").update(bytes).digest("hex")).toBe(manifest.artifacts[file].sha256);
  return new Uint8Array(bytes);
};

describe("frozen browser storage v1", () => {
  it("reads and re-encodes all Rust/TypeScript protocol messages", () => {
    for (const [file, roundTrip] of [
      ["loadRequest.cbor", (bytes: Uint8Array) => encodeLoadRequest(decodeLoadRequest(bytes))],
      ["loaded.cbor", (bytes: Uint8Array) => encodeLoaded(decodeLoaded(bytes))],
      ["flush.cbor", (bytes: Uint8Array) => encodeFlush(decodeFlush(bytes))],
      ["persisted.cbor", (bytes: Uint8Array) => encodeFlushReply(decodeFlushReply(bytes))],
      ["conflict.cbor", (bytes: Uint8Array) => encodeFlushReply(decodeFlushReply(bytes))],
    ] as const) {
      const bytes = artifact(file);
      expect(roundTrip(bytes)).toEqual(bytes);
      expect(() => roundTrip(new Uint8Array([...bytes, 0]))).toThrow();
    }
  });

  it("imports the frozen export into IndexedDB layout v1 and preserves it across reopen", async () => {
    const factory = new IDBFactory();
    let store = await IndexedDbStore.open("frozen", factory);
    const empty = { keys: [], objects: [], keyRanges: [], objectRanges: [] };
    const before = decodeLoaded(await store.load(encodeLoadRequest(empty))).store;
    const bytes = artifact("store-export.cbor");
    await store.import(bytes);
    store.close();
    store = await IndexedDbStore.open("frozen", factory);
    try {
      const after = decodeLoaded(await store.load(encodeLoadRequest(empty))).store;
      expect(after.id).not.toEqual(before.id);
      expect(after.revision).toBe(10);
      const exported = decode(await store.export());
      const original = decode(bytes);
      if (!Array.isArray(original)) throw new Error("fixture must be an export array");
      expect(exported).toEqual(["narrata-store-export", 1, 10, ...original.slice(3)]);
      await expect(store.import(bytes)).rejects.toThrow("empty store");
      const reopened = await new Promise<IDBDatabase>((resolve, reject) => {
        const request = factory.open("frozen", 1);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
      });
      expect(reopened.version).toBe(1);
      expect(Array.from(reopened.objectStoreNames)).toEqual(["keys", "meta", "objects"]);
      reopened.close();
    } finally {
      store.close();
    }
  });

  it("refuses malformed frozen exports before changing an empty database", async () => {
    const store = await IndexedDbStore.open("refused", new IDBFactory());
    const original = decode(artifact("store-export.cbor"));
    expect(Array.isArray(original)).toBe(true);
    if (!Array.isArray(original)) throw new Error("fixture must be an export array");
    if (!Array.isArray(original[3]) || !Array.isArray(original[4])) throw new Error("fixture entries must be arrays");
    try {
      for (const changed of [
        [original[0], 2, ...original.slice(2)],
        [original[0], 1, 8, ...original.slice(3)],
        [original[0], 1, 9, [...original[3]].reverse(), original[4]],
        [original[0], 1, 9, original[3], [...original[4]].reverse()],
      ]) await expect(store.import(encode(changed))).rejects.toThrow();
      await expect(store.import(new Uint8Array([...artifact("store-export.cbor"), 0]))).rejects.toThrow();
      expect(decode(await store.export())).toEqual(["narrata-store-export", 1, 0, [], []]);
    } finally { store.close(); }
  });
});
