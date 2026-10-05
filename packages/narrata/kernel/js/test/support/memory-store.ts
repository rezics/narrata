/** Test reference host; deliberately not exported by the product package. */
import { compareBytes, equalBytes } from "../../src/cbor";
import type { CacheStore } from "../../src/host";
import { decodeHostFlush, decodeHostLoadRequest, encodeHostLoaded } from "../../src/limits";
import {
  type KeyRecord, type Loaded, type Range, type StoreState, encodeFlushReply, hex,
} from "../../src/protocol";
import type { StoreFixture } from "./store-fixture";

interface State {
  store: StoreState;
  keys: Map<string, [Uint8Array, KeyRecord]>;
  objects: Map<string, [Uint8Array, Uint8Array]>;
}

function empty(): State {
  return { store: { id: crypto.getRandomValues(new Uint8Array(16)), revision: 0 }, keys: new Map(), objects: new Map() };
}

function page<T>(entries: Iterable<[Uint8Array, T]>, range: Range): [Uint8Array, T][] {
  return [...entries].filter(([key]) => compareBytes(key, range.lower) >= 0 &&
    (range.upper === null || compareBytes(key, range.upper) < 0))
    .sort(([a], [b]) => compareBytes(a, b)).slice(0, range.limit);
}

export async function memoryFixture(): Promise<StoreFixture> {
  let state = empty();
  let failAfter: number | undefined;
  let onCommit: (() => void) | undefined;
  const closers: (() => void)[] = [];
  const connect = (): CacheStore => {
    const id = state.store.id;
    let closed = false;
    closers.push(() => { closed = true; });
    const active = () => {
      if (closed || !equalBytes(id, state.store.id)) throw new Error("library handle was closed or replaced");
    };
    return {
      async load(message) {
        active();
        const wanted = decodeHostLoadRequest(message);
        const loaded: Loaded = {
          store: state.store,
          keys: wanted.keys.map((key) => [key, state.keys.get(hex(key))?.[1] ?? null]),
          objects: wanted.objects.map((digest) => [digest, state.objects.get(hex(digest))?.[1] ?? null]),
          keyRanges: wanted.keyRanges.map((range) => [range, page(state.keys.values(), range)]),
          objectRanges: wanted.objectRanges.map((range) => [range, page(state.objects.values(), range).map(([digest]) => digest)]),
        };
        return encodeHostLoaded(loaded);
      },
      async persist(message) {
        active();
        const flush = decodeHostFlush(message);
        if (!equalBytes(flush.store, state.store.id) || flush.batches[0]!.base !== state.store.revision) {
          return encodeFlushReply({ conflict: state.store });
        }
        const keys = new Map(state.keys);
        const objects = new Map(state.objects);
        for (const batch of flush.batches) {
          for (const [digest, bytes] of batch.putObjects) {
            if (!objects.has(hex(digest))) objects.set(hex(digest), [digest, bytes]);
          }
          for (const digest of batch.deleteObjects) objects.delete(hex(digest));
          for (const [key, value] of batch.putKeys) {
            keys.set(hex(key), [key, { value, revision: batch.revision }]);
            if (failAfter !== undefined && --failAfter === 0) {
              failAfter = undefined;
              throw new DOMException("injected quota failure after a staged write", "QuotaExceededError");
            }
          }
          for (const key of batch.deleteKeys) keys.delete(hex(key));
        }
        const revision = flush.batches[flush.batches.length - 1]!.revision;
        state = { store: { id, revision }, keys, objects };
        const committed = onCommit;
        onCommit = undefined;
        committed?.();
        return encodeFlushReply({ persisted: revision });
      },
    };
  };
  return {
    store: connect(),
    async connect() { return connect(); },
    async reopen() { for (const close of closers) close(); return connect(); },
    async clear() { for (const close of closers) close(); state = empty(); return connect(); },
    failNextPersist(afterWrites = 1) { failAfter = afterWrites; },
    observeNextCommit(committed) { onCommit = committed; },
    async dispose() { for (const close of closers) close(); state = empty(); },
  };
}
