/** Host resource bounds from ADR 0021, independent of checks performed by the cache. */
import type { DecodeLimits } from "./cbor";
import {
  type Flush, type Loaded, type LoadRequest, ProtocolError,
  decodeFlush, decodeLoadRequest, encodeLoaded,
} from "./protocol";

export const HOST_LIMITS = Object.freeze({
  keyBytes: 1_024,
  valueBytes: 16 * 1024 * 1024,
  operations: 10_000,
  batchBytes: 64 * 1024 * 1024,
  readItems: 1_024,
  messageBytes: 128 * 1024 * 1024,
});

const allocationLimits: DecodeLimits = {
  maxMessageBytes: HOST_LIMITS.messageBytes,
  maxStringBytes: HOST_LIMITS.valueBytes,
  maxArrayItems: HOST_LIMITS.operations,
  // 10,000 empty batches plus 10,000 key puts need at most 100,004 CBOR nodes.
  maxNodes: 110_000,
};

function bounded(actual: number, maximum: number, what: string): void {
  if (actual > maximum) throw new ProtocolError(`${what} exceeds host limit`);
}

function key(bytes: Uint8Array): void {
  bounded(bytes.length, HOST_LIMITS.keyBytes + 2, "storage key");
}

export function decodeHostLoadRequest(message: Uint8Array): LoadRequest {
  const wanted = decodeLoadRequest(message, allocationLimits);
  for (const bytes of wanted.keys) key(bytes);
  let items = wanted.keys.length + wanted.objects.length;
  for (const range of [...wanted.keyRanges, ...wanted.objectRanges]) {
    key(range.lower);
    if (range.upper !== null) key(range.upper);
    bounded(range.limit, HOST_LIMITS.readItems, "scan page");
    items += range.limit;
  }
  bounded(items, HOST_LIMITS.readItems, "load items");
  return wanted;
}

export function decodeHostFlush(message: Uint8Array): Flush {
  const flush = decodeFlush(message, allocationLimits);
  let totalOps = 0;
  let totalBytes = 0;
  for (const batch of flush.batches) {
    const ops = batch.putObjects.length + batch.deleteObjects.length + batch.putKeys.length + batch.deleteKeys.length;
    bounded(ops, HOST_LIMITS.operations, "batch operations");
    let bytes = 0;
    for (const [, value] of batch.putObjects) {
      bounded(value.length, HOST_LIMITS.valueBytes, "object bytes");
      bytes += value.length;
    }
    for (const [storedKey, value] of batch.putKeys) {
      key(storedKey);
      bounded(value.length, HOST_LIMITS.valueBytes, "key value bytes");
      // Batch::validate counts the raw key, excluding its two-byte space prefix.
      bytes += storedKey.length - 2 + value.length;
    }
    for (const storedKey of batch.deleteKeys) {
      key(storedKey);
      bytes += storedKey.length - 2;
    }
    bounded(bytes, HOST_LIMITS.batchBytes, "batch bytes");
    totalOps += ops;
    totalBytes += bytes;
  }
  bounded(totalOps, HOST_LIMITS.operations, "flush operations");
  bounded(totalBytes, HOST_LIMITS.batchBytes, "flush bytes");
  return flush;
}

/** Check values before encoding/copying the reply; never truncate a requested page. */
export function encodeHostLoaded(loaded: Loaded): Uint8Array {
  let bytes = 0;
  const value = (value: Uint8Array) => {
    bounded(value.length, HOST_LIMITS.valueBytes, "loaded value bytes");
    bytes += value.length;
  };
  for (const [, record] of loaded.keys) if (record !== null) value(record.value);
  for (const [, object] of loaded.objects) if (object !== null) value(object);
  for (const [, entries] of loaded.keyRanges) for (const [, record] of entries) value(record.value);
  bounded(bytes, HOST_LIMITS.batchBytes, "loaded value bytes");
  const message = encodeLoaded(loaded);
  bounded(message.length, HOST_LIMITS.messageBytes, "loaded message bytes");
  return message;
}
