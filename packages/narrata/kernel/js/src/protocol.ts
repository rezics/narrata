/**
 * Messages between the kernel's storage cache and its host (ADR 0017); the Rust side is
 * `narrata-storage-host/src/protocol.rs`. Every message is a canonical CBOR array whose first
 * item is the protocol version, and every list is strictly ascending, so decoding checks a
 * message completely. Keys are storage keys, `space (u16, big-endian) ‖ key`.
 */

import { type Cbor, type DecodeLimits, compareBytes, decode, encode } from "./cbor";

export const PROTOCOL_VERSION = 1;

export class ProtocolError extends Error {
  override name = "ProtocolError";
}

/** Which store a host holds, and the revision of the last batch it persisted (0: none). */
export interface StoreState {
  id: Uint8Array;
  revision: number;
}

/** Keys or digests from `lower` up to but excluding `upper` (`null`: to the end), at most `limit`. */
export interface Range {
  lower: Uint8Array;
  upper: Uint8Array | null;
  limit: number;
}

export interface LoadRequest {
  keys: Uint8Array[];
  objects: Uint8Array[];
  keyRanges: Range[];
  objectRanges: Range[];
}

export interface KeyRecord {
  value: Uint8Array;
  revision: number;
}

export interface Loaded {
  store: StoreState;
  keys: [Uint8Array, KeyRecord | null][];
  objects: [Uint8Array, Uint8Array | null][];
  keyRanges: [Range, [Uint8Array, KeyRecord][]][];
  objectRanges: [Range, Uint8Array[]][];
}

/** One batch's effect; it moves the store from `base` to `revision = base + 1`. */
export interface Persist {
  base: number;
  revision: number;
  putObjects: [Uint8Array, Uint8Array][];
  deleteObjects: Uint8Array[];
  putKeys: [Uint8Array, Uint8Array][];
  deleteKeys: Uint8Array[];
}

export interface Flush {
  store: Uint8Array;
  batches: Persist[];
}

export type FlushReply = { persisted: number } | { conflict: StoreState };

export const STORE_ID_BYTES = 16;
export const DIGEST_BYTES = 32;

function fail(message: string): never {
  throw new ProtocolError(message);
}

function list(value: Cbor, length?: number): readonly Cbor[] {
  if (!Array.isArray(value)) fail("expected an array");
  const items = value as readonly Cbor[];
  if (length !== undefined && items.length !== length) fail("message array length");
  return items;
}

function bytes(value: Cbor, length?: number): Uint8Array {
  if (!(value instanceof Uint8Array)) fail("expected a byte string");
  if (length !== undefined && value.length !== length) fail(`expected ${length} bytes`);
  return value;
}

function uint(value: Cbor): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0) fail("expected a safe unsigned integer");
  return value;
}

function nullable<T>(value: Cbor, read: (value: Cbor) => T): T | null {
  return value === null ? null : read(value);
}

function header(value: Cbor, length: number): readonly Cbor[] {
  const items = list(value, length);
  if (items[0] !== PROTOCOL_VERSION) fail("unsupported storage host protocol version");
  return items;
}

function ascending<T>(items: readonly T[], compare: (left: T, right: T) => number, what: string): void {
  for (let index = 1; index < items.length; index++) {
    if (compare(items[index - 1]!, items[index]!) >= 0) fail(`${what} are out of order`);
  }
}

/** Orders ranges as the Rust side does: lower bound, then upper bound with null first, then limit. */
function compareRanges(left: Range, right: Range): number {
  const lower = compareBytes(left.lower, right.lower);
  if (lower !== 0) return lower;
  if (left.upper === null || right.upper === null) {
    const upper = Number(left.upper !== null) - Number(right.upper !== null);
    if (upper !== 0) return upper;
  } else {
    const upper = compareBytes(left.upper, right.upper);
    if (upper !== 0) return upper;
  }
  return left.limit - right.limit;
}

function storageKey(value: Cbor): Uint8Array {
  const key = bytes(value);
  if (key.length < 2) fail("storage key shorter than its key space");
  return key;
}

function digest(value: Cbor): Uint8Array {
  return bytes(value, DIGEST_BYTES);
}

function range(value: Cbor): Range {
  const [lower, upper, limit] = list(value, 3);
  const decoded = { lower: bytes(lower!), upper: nullable(upper!, (upper) => bytes(upper)), limit: uint(limit!) };
  if (decoded.limit === 0) fail("range limit must be positive");
  if (decoded.upper !== null && compareBytes(decoded.upper, decoded.lower) <= 0) {
    fail("range upper bound must follow its lower bound");
  }
  return decoded;
}

function rangeCbor(range: Range): Cbor {
  return [range.lower, range.upper, range.limit];
}

function contains(range: Range, key: Uint8Array): boolean {
  return compareBytes(key, range.lower) >= 0 && (range.upper === null || compareBytes(key, range.upper) < 0);
}

function checkEntries(range: Range, keys: Uint8Array[]): void {
  if (keys.some((key) => !contains(range, key))) fail("range entry outside its range");
  ascending(keys, compareBytes, "range entries");
  if (keys.length > range.limit) fail("range answered with more entries than its limit");
}

function record(value: Cbor, revision: Cbor, store: number): KeyRecord {
  const decoded = { value: bytes(value), revision: uint(revision) };
  if (decoded.revision === 0) fail("key revision is zero");
  if (decoded.revision > store) fail("key revision passes the store revision");
  return decoded;
}

export function encodeLoadRequest(request: LoadRequest): Uint8Array {
  return encode([
    PROTOCOL_VERSION,
    request.keys,
    request.objects,
    request.keyRanges.map(rangeCbor),
    request.objectRanges.map(rangeCbor),
  ]);
}

export function decodeLoadRequest(message: Uint8Array, limits?: DecodeLimits): LoadRequest {
  const [, keys, objects, keyRanges, objectRanges] = header(decode(message, limits), 5);
  const request: LoadRequest = {
    keys: list(keys!).map((key) => bytes(key)),
    objects: list(objects!).map(digest),
    keyRanges: list(keyRanges!).map(range),
    objectRanges: list(objectRanges!).map(range),
  };
  ascending(request.keys, compareBytes, "requested keys");
  ascending(request.objects, compareBytes, "requested objects");
  ascending(request.keyRanges, compareRanges, "requested ranges");
  ascending(request.objectRanges, compareRanges, "requested ranges");
  return request;
}

export function encodeLoaded(loaded: Loaded): Uint8Array {
  return encode([
    PROTOCOL_VERSION,
    loaded.store.id,
    loaded.store.revision,
    loaded.keys.map(([key, value]) => [key, value && [value.value, value.revision]]),
    loaded.objects.map(([digest, bytes]) => [digest, bytes]),
    loaded.keyRanges.map(([range, entries]) => [
      rangeCbor(range),
      entries.map(([key, value]) => [key, value.value, value.revision]),
    ]),
    loaded.objectRanges.map(([range, digests]) => [rangeCbor(range), digests]),
  ]);
}

export function decodeLoaded(message: Uint8Array): Loaded {
  const [, id, revision, keys, objects, keyRanges, objectRanges] = header(decode(message), 7);
  const store = { id: bytes(id!, STORE_ID_BYTES), revision: uint(revision!) };
  const loaded: Loaded = {
    store,
    keys: list(keys!).map((entry) => {
      const [key, value] = list(entry, 2);
      return [
        bytes(key!),
        nullable(value!, (value) => {
          const [bytes, revision] = list(value, 2);
          return record(bytes!, revision!, store.revision);
        }),
      ];
    }),
    objects: list(objects!).map((entry) => {
      const [key, value] = list(entry, 2);
      return [digest(key!), nullable(value!, (value) => bytes(value))];
    }),
    keyRanges: list(keyRanges!).map((entry) => {
      const [bounds, entries] = list(entry, 2);
      const decoded = range(bounds!);
      const records = list(entries!).map((entry): [Uint8Array, KeyRecord] => {
        const [key, value, revision] = list(entry, 3);
        return [bytes(key!), record(value!, revision!, store.revision)];
      });
      checkEntries(decoded, records.map(([key]) => key));
      return [decoded, records];
    }),
    objectRanges: list(objectRanges!).map((entry) => {
      const [bounds, digests] = list(entry, 2);
      const decoded = range(bounds!);
      const listed = list(digests!).map(digest);
      checkEntries(decoded, listed);
      return [decoded, listed];
    }),
  };
  ascending(loaded.keys.map(([key]) => key), compareBytes, "loaded keys");
  ascending(loaded.objects.map(([digest]) => digest), compareBytes, "loaded objects");
  ascending(loaded.keyRanges.map(([range]) => range), compareRanges, "loaded key ranges");
  ascending(loaded.objectRanges.map(([range]) => range), compareRanges, "loaded object ranges");
  return loaded;
}

export function encodeFlush(flush: Flush): Uint8Array {
  return encode([
    PROTOCOL_VERSION,
    flush.store,
    flush.batches.map((batch) => [
      batch.base,
      batch.revision,
      batch.putObjects,
      batch.deleteObjects,
      batch.putKeys,
      batch.deleteKeys,
    ]),
  ]);
}

export function decodeFlush(message: Uint8Array, limits?: DecodeLimits): Flush {
  const [, store, batches] = header(decode(message, limits), 3);
  const flush: Flush = {
    store: bytes(store!, STORE_ID_BYTES),
    batches: list(batches!).map((batch): Persist => {
      const [base, revision, putObjects, deleteObjects, putKeys, deleteKeys] = list(batch, 6);
      return {
        base: uint(base!),
        revision: uint(revision!),
        putObjects: list(putObjects!).map((entry) => {
          const [key, value] = list(entry, 2);
          return [digest(key!), bytes(value!)];
        }),
        deleteObjects: list(deleteObjects!).map(digest),
        putKeys: list(putKeys!).map((entry) => {
          const [key, value] = list(entry, 2);
          return [storageKey(key!), bytes(value!)];
        }),
        deleteKeys: list(deleteKeys!).map(storageKey),
      };
    }),
  };
  if (flush.batches.length === 0) fail("a flush carries at least one batch");
  let previous: number | undefined;
  for (const batch of flush.batches) {
    if (batch.base === Number.MAX_SAFE_INTEGER || batch.revision !== batch.base + 1) fail("a batch moves the store revision by one");
    if (previous !== undefined && previous !== batch.base) fail("flushed batches are not consecutive");
    previous = batch.revision;
    const putObjects = batch.putObjects.map(([digest]) => digest);
    ascending(putObjects, compareBytes, "put objects");
    ascending(batch.deleteObjects, compareBytes, "deleted objects");
    disjoint(putObjects, batch.deleteObjects, "object put and deleted in one batch");
    const putKeys = batch.putKeys.map(([key]) => key);
    ascending(putKeys, compareBytes, "put keys");
    ascending(batch.deleteKeys, compareBytes, "deleted keys");
    disjoint(putKeys, batch.deleteKeys, "key put and deleted in one batch");
  }
  return flush;
}

function disjoint(left: Uint8Array[], right: Uint8Array[], message: string): void {
  // Merge the checked sorted lists without allocating key strings.
  let one = 0;
  let two = 0;
  while (one < left.length && two < right.length) {
    const order = compareBytes(left[one]!, right[two]!);
    if (order === 0) fail(message);
    if (order < 0) one++;
    else two++;
  }
}

export function hex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

export function encodeFlushReply(reply: FlushReply): Uint8Array {
  return "persisted" in reply
    ? encode([PROTOCOL_VERSION, 0, reply.persisted])
    : encode([PROTOCOL_VERSION, 1, reply.conflict.id, reply.conflict.revision]);
}

export function decodeFlushReply(message: Uint8Array): FlushReply {
  const items = list(decode(message));
  if (items[0] !== PROTOCOL_VERSION) fail("unsupported storage host protocol version");
  if (items.length === 3 && items[1] === 0) return { persisted: uint(items[2]!) };
  if (items.length === 4 && items[1] === 1) {
    return { conflict: { id: bytes(items[2]!, STORE_ID_BYTES), revision: uint(items[3]!) } };
  }
  fail("unknown flush reply");
}
