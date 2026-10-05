/**
 * The IndexedDB side of the kernel's storage cache (ADR 0017): it answers load requests from one
 * read transaction and persists flushes in one read-write transaction that first checks the
 * store's id and revision. It stores keys and values as opaque bytes; only the revision counter
 * and the store id mean something to it.
 */

import { type Cbor, compareBytes, decode, encode, equalBytes } from "./cbor";
import { decodeHostFlush, decodeHostLoadRequest, encodeHostLoaded } from "./limits";
import {
  DIGEST_BYTES,
  type KeyRecord,
  type Loaded,
  type Range,
  STORE_ID_BYTES,
  type StoreState,
  encodeFlushReply,
} from "./protocol";

export const DATABASE_VERSION = 1;
const OBJECTS = "objects";
const KEYS = "keys";
const META = "meta";
const STORE = "store";
const EXPORT_LABEL = "narrata-store-export";
const EXPORT_VERSION = 1;

/** A database that is not in the layout this adapter writes. */
export class StoreFormatError extends Error {
  override name = "StoreFormatError";
}

/** Lookups served so far: requested keys and objects, and entries returned by ranges. */
export interface HostReads {
  loads: number;
  keys: number;
  objects: number;
  keyEntries: number;
  objectEntries: number;
}

function request<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

function done(transaction: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    transaction.oncomplete = () => resolve();
    transaction.onabort = () => reject(transaction.error ?? new Error("IndexedDB transaction aborted"));
  });
}

/** Decoded byte strings are fresh copies, so they are backed by an `ArrayBuffer` as IndexedDB keys must be. */
function idbKey(bytes: Uint8Array): IDBValidKey {
  return bytes as Uint8Array<ArrayBuffer>;
}

function keyRange(range: Range): IDBKeyRange | null {
  // The empty key is the least binary key, and the stores hold nothing else, so an empty lower
  // bound is left out; some implementations refuse empty keys.
  if (range.lower.length === 0) {
    return range.upper === null ? null : IDBKeyRange.upperBound(idbKey(range.upper), true);
  }
  const lower = idbKey(range.lower);
  return range.upper === null ? IDBKeyRange.lowerBound(lower) : IDBKeyRange.bound(lower, idbKey(range.upper), false, true);
}

/** IndexedDB takes counts up to 2^32 - 1; a range never needs more. */
function count(limit: number): number {
  return Math.min(limit, 0xffff_ffff);
}

function binary(key: IDBValidKey): Uint8Array {
  if (key instanceof ArrayBuffer) return new Uint8Array(key);
  if (ArrayBuffer.isView(key)) return new Uint8Array(key.buffer, key.byteOffset, key.byteLength).slice();
  throw new StoreFormatError("IndexedDB key is not binary");
}

function revision(value: unknown): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) {
    throw new StoreFormatError("IndexedDB revision is not a safe unsigned integer");
  }
  return value;
}

function storeState(value: unknown): StoreState {
  const meta = value as Partial<StoreState> | undefined;
  if (!(meta?.id instanceof Uint8Array) || meta.id.length !== STORE_ID_BYTES) {
    throw new StoreFormatError("IndexedDB store has no store id");
  }
  return { id: meta.id, revision: revision(meta.revision) };
}

function keyRecord(value: unknown): KeyRecord {
  const record = value as Partial<KeyRecord> | undefined;
  if (!(record?.value instanceof Uint8Array)) throw new StoreFormatError("IndexedDB key record has no value");
  return { value: record.value, revision: revision(record.revision) };
}

function objectBytes(value: unknown): Uint8Array {
  if (!(value instanceof Uint8Array)) throw new StoreFormatError("IndexedDB object is not bytes");
  return value;
}

function newStoreId(): Uint8Array {
  return crypto.getRandomValues(new Uint8Array(STORE_ID_BYTES));
}

export class IndexedDbStore {
  readonly reads: HostReads = { loads: 0, keys: 0, objects: 0, keyEntries: 0, objectEntries: 0 };

  private constructor(private readonly db: IDBDatabase) {}

  /** Opens the database `name`, creating it empty under a new store id. */
  static async open(name: string, factory: IDBFactory = indexedDB): Promise<IndexedDbStore> {
    const opening = factory.open(name, DATABASE_VERSION);
    opening.onupgradeneeded = () => {
      const db = opening.result;
      db.createObjectStore(OBJECTS);
      db.createObjectStore(KEYS);
      db.createObjectStore(META).put({ id: newStoreId(), revision: 0 }, STORE);
    };
    let db: IDBDatabase;
    try {
      db = await request(opening);
    } catch (error) {
      if (error instanceof DOMException && error.name === "VersionError") {
        throw new StoreFormatError(`IndexedDB database ${name} has a newer layout than version ${DATABASE_VERSION}`);
      }
      throw error;
    }
    // Another tab deleting or upgrading the database must not wait for this one.
    db.onversionchange = () => db.close();
    return new IndexedDbStore(db);
  }

  static async delete(name: string, factory: IDBFactory = indexedDB): Promise<void> {
    await request(factory.deleteDatabase(name));
  }

  close(): void {
    this.db.close();
  }

  /** Answers an encoded load request from one read transaction. */
  async load(message: Uint8Array): Promise<Uint8Array> {
    const wanted = decodeHostLoadRequest(message);
    const transaction = this.db.transaction([META, KEYS, OBJECTS], "readonly");
    const finished = done(transaction);
    const keys = transaction.objectStore(KEYS);
    const objects = transaction.objectStore(OBJECTS);
    const meta = transaction.objectStore(META).get(STORE);
    const keyReads = wanted.keys.map((key) => keys.get(idbKey(key)));
    const objectReads = wanted.objects.map((digest) => objects.get(idbKey(digest)));
    const keyRangeReads = wanted.keyRanges.map((range) => ({
      keys: keys.getAllKeys(keyRange(range), count(range.limit)),
      values: keys.getAll(keyRange(range), count(range.limit)),
    }));
    const objectRangeReads = wanted.objectRanges.map((range) => objects.getAllKeys(keyRange(range), count(range.limit)));
    await finished;

    const loaded: Loaded = {
      store: storeState(meta.result),
      keys: wanted.keys.map((key, index) => {
        const value = keyReads[index]!.result;
        return [key, value === undefined ? null : keyRecord(value)];
      }),
      objects: wanted.objects.map((digest, index) => {
        const value = objectReads[index]!.result;
        return [digest, value === undefined ? null : objectBytes(value)];
      }),
      keyRanges: wanted.keyRanges.map((range, index) => {
        const read = keyRangeReads[index]!;
        const values = read.values.result;
        return [range, read.keys.result.map((key, entry) => [binary(key), keyRecord(values[entry])])];
      }),
      objectRanges: wanted.objectRanges.map((range, index) => [range, objectRangeReads[index]!.result.map(binary)]),
    };
    this.reads.loads += 1;
    this.reads.keys += wanted.keys.length;
    this.reads.objects += wanted.objects.length;
    this.reads.keyEntries += loaded.keyRanges.reduce((sum, [, entries]) => sum + entries.length, 0);
    this.reads.objectEntries += loaded.objectRanges.reduce((sum, [, digests]) => sum + digests.length, 0);
    return encodeHostLoaded(loaded);
  }

  /**
   * Persists an encoded flush in one read-write transaction, or writes nothing when the store
   * is not where its first batch expects it, and answers with an encoded reply. Resolves only
   * after the transaction has completed.
   */
  async persist(message: Uint8Array): Promise<Uint8Array> {
    const flush = decodeHostFlush(message);
    const first = flush.batches[0]!;
    const last = flush.batches[flush.batches.length - 1]!;
    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction([META, KEYS, OBJECTS], "readwrite", { durability: "strict" });
      let reply: Uint8Array | undefined;
      let refused: unknown;
      const metaStore = transaction.objectStore(META);
      const meta = metaStore.get(STORE);
      meta.onsuccess = () => {
        try {
          const state = storeState(meta.result);
          if (!equalBytes(state.id, flush.store) || state.revision !== first.base) {
            reply = encodeFlushReply({ conflict: state });
            return;
          }
          const objects = transaction.objectStore(OBJECTS);
          const keys = transaction.objectStore(KEYS);
          for (const batch of flush.batches) {
            for (const [digest, bytes] of batch.putObjects) {
              const added = objects.add(bytes, idbKey(digest));
              added.onerror = (event) => {
                if (added.error?.name === "ConstraintError") {
                  // Only an already-present digest is ignored. Other failures abort the flush.
                  event.preventDefault();
                  event.stopPropagation();
                }
              };
            }
            for (const digest of batch.deleteObjects) objects.delete(idbKey(digest));
            for (const [key, value] of batch.putKeys) keys.put({ value, revision: batch.revision }, idbKey(key));
            for (const key of batch.deleteKeys) keys.delete(idbKey(key));
          }
          metaStore.put({ id: state.id, revision: last.revision }, STORE);
          reply = encodeFlushReply({ persisted: last.revision });
        } catch (error) {
          refused = error;
          transaction.abort();
        }
      };
      transaction.oncomplete = () => (reply ? resolve(reply) : reject(new Error("flush transaction ended without a reply")));
      transaction.onabort = () => reject(refused ?? transaction.error ?? new Error("flush transaction aborted"));
    });
  }

  /**
   * The whole store as canonical CBOR:
   * `["narrata-store-export", 1, revision, [[digest, bytes]], [[key, value, revision]]]`.
   */
  async export(): Promise<Uint8Array> {
    const transaction = this.db.transaction([META, KEYS, OBJECTS], "readonly");
    const finished = done(transaction);
    const meta = transaction.objectStore(META).get(STORE);
    const objectKeys = transaction.objectStore(OBJECTS).getAllKeys();
    const objectValues = transaction.objectStore(OBJECTS).getAll();
    const keyKeys = transaction.objectStore(KEYS).getAllKeys();
    const keyValues = transaction.objectStore(KEYS).getAll();
    await finished;
    const objects = objectKeys.result.map((key, index): Cbor => [binary(key), objectBytes(objectValues.result[index])]);
    const keys = keyKeys.result.map((key, index): Cbor => {
      const record = keyRecord(keyValues.result[index]);
      return [binary(key), record.value, record.revision];
    });
    return encode([EXPORT_LABEL, EXPORT_VERSION, storeState(meta.result).revision, objects, keys]);
  }

  /**
   * Writes an export into this store, which must be empty, under a new store id and a revision
   * above both the export's and the store's, so no cache of an earlier incarnation matches it.
   */
  async import(message: Uint8Array): Promise<void> {
    const exported = decodeExport(message);
    return new Promise((resolve, reject) => {
      const transaction = this.db.transaction([META, KEYS, OBJECTS], "readwrite", { durability: "strict" });
      let refused: Error | undefined;
      const objects = transaction.objectStore(OBJECTS);
      const keys = transaction.objectStore(KEYS);
      const metaStore = transaction.objectStore(META);
      const objectCount = objects.count();
      const keyCount = keys.count();
      const meta = metaStore.get(STORE);
      meta.onsuccess = () => {
        if (objectCount.result !== 0 || keyCount.result !== 0) {
          refused = new StoreFormatError("import needs an empty store");
          transaction.abort();
          return;
        }
        const current = storeState(meta.result);
        if (Math.max(current.revision, exported.revision) === Number.MAX_SAFE_INTEGER) {
          refused = new StoreFormatError("import would exceed the safe revision limit");
          transaction.abort();
          return;
        }
        for (const [digest, bytes] of exported.objects) objects.put(bytes, idbKey(digest));
        for (const [key, value, revision] of exported.keys) keys.put({ value, revision }, idbKey(key));
        metaStore.put({ id: newStoreId(), revision: Math.max(current.revision, exported.revision) + 1 }, STORE);
      };
      transaction.oncomplete = () => resolve();
      transaction.onabort = () => reject(refused ?? transaction.error ?? new Error("import transaction aborted"));
    });
  }

  /**
   * Asks the browser to keep the store through storage pressure. Without it the store is
   * evictable, and Safari deletes it after seven days without interaction; export it to keep it.
   */
  static async requestPersistence(storage: StorageManager | undefined = globalThis.navigator?.storage): Promise<boolean> {
    if (storage?.persist === undefined) return false;
    if (await storage.persisted?.()) return true;
    return storage.persist();
  }
}

interface Export {
  revision: number;
  objects: [Uint8Array, Uint8Array][];
  keys: [Uint8Array, Uint8Array, number][];
}

function decodeExport(message: Uint8Array): Export {
  const fail = (reason: string): never => {
    throw new StoreFormatError(`not a store export: ${reason}`);
  };
  const value = decode(message);
  if (!Array.isArray(value) || value.length !== 5) fail("expected a five-item array");
  const [label, version, revision, objects, keys] = value as readonly Cbor[];
  if (label !== EXPORT_LABEL) fail("missing label");
  if (version !== EXPORT_VERSION) fail(`unsupported version ${String(version)}`);
  if (typeof revision !== "number") fail("revision");
  const exported: Export = { revision: revision as number, objects: [], keys: [] };
  const entries = (list: Cbor | undefined, length: number): readonly (readonly Cbor[])[] => {
    if (!Array.isArray(list)) fail("expected a list");
    return (list as readonly Cbor[]).map((entry) => {
      if (!Array.isArray(entry) || entry.length !== length) fail("entry shape");
      return entry as readonly Cbor[];
    });
  };
  for (const [digest, bytes] of entries(objects, 2)) {
    if (!(digest instanceof Uint8Array) || digest.length !== DIGEST_BYTES || !(bytes instanceof Uint8Array)) fail("object entry");
    exported.objects.push([digest as Uint8Array, bytes as Uint8Array]);
  }
  for (const [key, value, keyRevision] of entries(keys, 3)) {
    if (!(key instanceof Uint8Array) || key.length < 2 || !(value instanceof Uint8Array)) fail("key entry");
    if (typeof keyRevision !== "number" || keyRevision === 0 || keyRevision > exported.revision) fail("key revision");
    exported.keys.push([key as Uint8Array, value as Uint8Array, keyRevision as number]);
  }
  for (const [list, what] of [
    [exported.objects.map(([digest]) => digest), "objects"],
    [exported.keys.map(([key]) => key), "keys"],
  ] as const) {
    for (let index = 1; index < list.length; index++) {
      if (compareBytes(list[index - 1]!, list[index]!) >= 0) fail(`${what} out of order`);
    }
  }
  return exported;
}
