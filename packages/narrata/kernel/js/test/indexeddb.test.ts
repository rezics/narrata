import "fake-indexeddb/auto";

import { IDBFactory } from "fake-indexeddb";
import { describe, expect, it } from "vitest";

import { encode } from "../src/cbor";
import { IndexedDbStore, StoreFormatError } from "../src/indexeddb";
import {
  type LoadRequest,
  type Persist,
  type StoreState,
  decodeFlushReply,
  decodeLoaded,
  encodeFlush,
  encodeLoadRequest,
} from "../src/protocol";

const key = (space: number, ...rest: number[]) => new Uint8Array([space >> 8, space & 0xff, ...rest]);
const digest = (fill: number) => new Uint8Array(32).fill(fill);
const empty: LoadRequest = { keys: [], objects: [], keyRanges: [], objectRanges: [] };

async function fresh(name = "store"): Promise<[IndexedDbStore, IDBFactory]> {
  const factory = new IDBFactory();
  return [await IndexedDbStore.open(name, factory), factory];
}

async function load(store: IndexedDbStore, request: Partial<LoadRequest> = {}) {
  return decodeLoaded(await store.load(encodeLoadRequest({ ...empty, ...request })));
}

function batch(base: number, change: Partial<Persist>): Persist {
  return { base, revision: base + 1, putObjects: [], deleteObjects: [], putKeys: [], deleteKeys: [], ...change };
}

async function persist(store: IndexedDbStore, state: StoreState, ...batches: Persist[]) {
  return decodeFlushReply(await store.persist(encodeFlush({ store: state.id, batches })));
}

describe("IndexedDbStore", () => {
  it("starts empty at revision 0 under a random store id", async () => {
    const [first] = await fresh();
    const [second] = await fresh();
    const one = (await load(first, { keys: [key(1, 1)], objects: [digest(1)] })).store;
    const other = (await load(second)).store;
    expect(one.revision).toBe(0);
    expect(one.id).toHaveLength(16);
    expect(one.id).not.toEqual(other.id);
    expect(await load(first, { keys: [key(1, 1)] })).toMatchObject({ keys: [[key(1, 1), null]] });
  });

  it("persists consecutive batches in one transaction and answers keys, objects and ranges", async () => {
    const [store] = await fresh();
    const { store: state } = await load(store);
    const reply = await persist(
      store,
      state,
      batch(0, {
        putObjects: [
          [digest(1), new Uint8Array([1])],
          [digest(2), new Uint8Array([2])],
        ],
        putKeys: [
          [key(4, 1), new Uint8Array([10])],
          [key(4, 2), new Uint8Array([20])],
          [key(4, 3), new Uint8Array([30])],
          [key(5), new Uint8Array([50])],
        ],
      }),
      batch(1, { deleteKeys: [key(4, 2)], deleteObjects: [digest(2)], putKeys: [[key(4, 3), new Uint8Array([31])]] }),
    );
    expect(reply).toEqual({ persisted: 2 });

    const loaded = await load(store, {
      keys: [key(4, 1), key(4, 2)],
      objects: [digest(1), digest(2)],
      keyRanges: [
        { lower: key(4), upper: key(5), limit: 1 },
        { lower: key(4), upper: key(5), limit: 5 },
        { lower: key(5), upper: null, limit: 5 },
      ],
      objectRanges: [{ lower: new Uint8Array(), upper: null, limit: 5 }],
    });
    expect(loaded.store).toEqual({ id: state.id, revision: 2 });
    expect(loaded.keys).toEqual([
      [key(4, 1), { value: new Uint8Array([10]), revision: 1 }],
      [key(4, 2), null],
    ]);
    expect(loaded.objects).toEqual([
      [digest(1), new Uint8Array([1])],
      [digest(2), null],
    ]);
    expect(loaded.keyRanges.map(([, entries]) => entries.map(([key, record]) => [Array.from(key), record.revision]))).toEqual([
      [[[0, 4, 1], 1]],
      [
        [[0, 4, 1], 1],
        [[0, 4, 3], 2],
      ],
      [[[0, 5], 1]],
    ]);
    expect(loaded.objectRanges[0]![1]).toEqual([digest(1)]);
    expect(store.reads).toEqual({ loads: 2, keys: 2, objects: 2, keyEntries: 4, objectEntries: 1 });
  });

  it("writes nothing when the store is not where the first batch expects it", async () => {
    const [store] = await fresh();
    const { store: state } = await load(store);
    expect(await persist(store, state, batch(0, { putKeys: [[key(1), new Uint8Array()]] }))).toEqual({ persisted: 1 });
    const current = { ...state, revision: 1 };
    const stale = batch(0, { putKeys: [[key(2), new Uint8Array()]] });
    expect(await persist(store, state, stale)).toEqual({ conflict: current });
    const elsewhere = { id: new Uint8Array(16), revision: 1 };
    expect(await persist(store, elsewhere, batch(1, { putKeys: [[key(2), new Uint8Array()]] }))).toEqual({ conflict: current });
    expect((await load(store, { keys: [key(2)] })).keys).toEqual([[key(2), null]]);
  });

  it("restores an export into an empty store under a new id and a later revision", async () => {
    const [store, factory] = await fresh("saves");
    const { store: state } = await load(store);
    await persist(
      store,
      state,
      batch(0, { putObjects: [[digest(3), new Uint8Array([3])]], putKeys: [[key(1, 1), new Uint8Array([1])]] }),
      batch(1, { putKeys: [[key(1, 2), new Uint8Array([2])]] }),
    );
    const exported = await store.export();
    store.close();

    await IndexedDbStore.delete("saves", factory);
    const restored = await IndexedDbStore.open("saves", factory);
    await restored.import(exported);
    const loaded = await load(restored, {
      keys: [key(1, 1), key(1, 2)],
      objects: [digest(3)],
    });
    expect(loaded.store.revision).toBe(3);
    expect(loaded.store.id).not.toEqual(state.id);
    expect(loaded.keys.map(([, record]) => record?.revision)).toEqual([1, 2]);
    expect(loaded.objects).toEqual([[digest(3), new Uint8Array([3])]]);
    expect(await restored.export()).not.toEqual(exported);

    await expect(restored.import(exported)).rejects.toThrow(StoreFormatError);
    await expect(restored.import(new Uint8Array([0x80]))).rejects.toThrow(StoreFormatError);
  });

  it("refuses a database with a newer layout", async () => {
    const factory = new IDBFactory();
    await new Promise<void>((resolve, reject) => {
      const opening = factory.open("newer", 2);
      opening.onsuccess = () => {
        opening.result.close();
        resolve();
      };
      opening.onerror = () => reject(opening.error);
    });
    await expect(IndexedDbStore.open("newer", factory)).rejects.toThrow(StoreFormatError);
  });

  it("refuses an import that would overflow the library revision without writing", async () => {
    const [store] = await fresh();
    const before = await load(store);
    const message = encode(["narrata-store-export", 1, Number.MAX_SAFE_INTEGER,
      [[digest(1), new Uint8Array([1])]], [[key(1), new Uint8Array([1]), 1]]]);
    await expect(store.import(message)).rejects.toThrow("safe revision limit");
    expect(await load(store)).toEqual(before);
    expect((await load(store, { keys: [key(1)], objects: [digest(1)] })).keys[0]![1]).toBeNull();
    expect((await load(store, { objects: [digest(1)] })).objects[0]![1]).toBeNull();
  });

  it("asks for persistent storage only when it is not granted yet", async () => {
    const asked: string[] = [];
    const manager = (persisted: boolean) =>
      ({
        persisted: async () => (asked.push("persisted"), persisted),
        persist: async () => (asked.push("persist"), true),
      }) as unknown as StorageManager;
    expect(await IndexedDbStore.requestPersistence(manager(true))).toBe(true);
    expect(asked).toEqual(["persisted"]);
    expect(await IndexedDbStore.requestPersistence(manager(false))).toBe(true);
    expect(asked).toEqual(["persisted", "persisted", "persist"]);
    expect(await IndexedDbStore.requestPersistence(undefined)).toBe(false);
  });
});
