/** The same protocol assertions run in Vitest, Chromium and future server host fixtures. */
import { type Cbor, encode, equalBytes } from "../../src/cbor";
import { type CacheHandle, StorageHost } from "../../src/host";
import { HOST_LIMITS } from "../../src/limits";
import {
  type LoadRequest, type Persist, type StoreState,
  decodeFlushReply, decodeLoaded, encodeFlush, encodeLoaded, encodeLoadRequest,
} from "../../src/protocol";
import type { CacheStore } from "../../src/host";
import type { StoreFactory, StoreFixture } from "./store-fixture";

export const key = (...bytes: number[]) => new Uint8Array([0, 1, ...bytes]);
export const digest = (byte: number) => new Uint8Array(32).fill(byte);
const value = (...bytes: number[]) => new Uint8Array(bytes);
const empty: LoadRequest = { keys: [], objects: [], keyRanges: [], objectRanges: [] };
const all = { lower: value(), upper: null, limit: 1024 };
export const batch = (base: number, changes: Partial<Persist> = {}): Persist => ({
  base, revision: base + 1, putObjects: [], deleteObjects: [], putKeys: [], deleteKeys: [], ...changes,
});
export const load = async (store: CacheStore, wanted: Partial<LoadRequest> = {}) =>
  decodeLoaded(await store.load(encodeLoadRequest({ ...empty, ...wanted })));
const flush = (state: StoreState, ...batches: Persist[]) => encodeFlush({ store: state.id, batches });
const persist = async (store: CacheStore, state: StoreState, ...batches: Persist[]) =>
  decodeFlushReply(await store.persist(flush(state, ...batches)));

function assert(ok: boolean, message: string): asserts ok { if (!ok) throw new Error(message); }
function same(actual: Cbor, expected: Cbor) { assert(equalBytes(encode(actual), encode(expected)), "protocol values differ"); }
async function rejected(call: () => Promise<unknown>) {
  let error: unknown;
  try { await call(); } catch (caught) { error = caught; }
  assert(error !== undefined, "expected the host to reject the whole request");
}
const snapshot = async (store: CacheStore) => store.load(encodeLoadRequest({
  ...empty, keyRanges: [{ ...all, limit: 512 }], objectRanges: [{ ...all, limit: 512 }],
}));

interface ConformanceCase { name: string; run(fixture: StoreFixture, factory: StoreFactory): Promise<void> }
export const conformanceCases: readonly ConformanceCase[] = [
  { name: "missing entries are null and reopen retains state", async run(f) {
    const initial = await load(f.store, { keys: [key()], objects: [digest(0)] });
    same(initial.store.revision, 0);
    same(initial.keys.map(([k, r]) => [k, r === null ? null : [r.value, r.revision]]), [[key(), null]]);
    same(initial.objects, [[digest(0), null]]);
    await persist(f.store, initial.store, batch(0, { putKeys: [[key(), value(7)]], putObjects: [[digest(0), value(8)]] }));
    const before = await snapshot(f.store);
    f.store = await f.reopen();
    same(await snapshot(f.store), before);
    const loaded = await load(f.store, { keys: [key()], objects: [digest(0)] });
    same(loaded.keys[0]![1]!.value, value(7));
    same(loaded.objects[0]![1], value(8));
    same(loaded.store.id, initial.store.id);
    same(loaded.store.revision, 1);
  } },
  { name: "unsigned byte ordering, prefixes, pagination and empty intervals", async run(f) {
    const state = (await load(f.store)).store;
    const keys = [key(), key(0), key(0, 0), key(0, 255), key(127), key(128), key(255), value(0, 2)];
    const digests = [0, 127, 128, 255].map(digest);
    await persist(f.store, state, batch(0, { putKeys: keys.map((k) => [k, value(1)]), putObjects: digests.map((d) => [d, value(2)]) }));
    const first = await load(f.store, { keyRanges: [{ ...all, limit: 3 }], objectRanges: [{ ...all, limit: 2 }] });
    same(first.keyRanges[0]![1].map(([k]) => k), keys.slice(0, 3));
    same(first.objectRanges[0]![1], digests.slice(0, 2));
    const next = await load(f.store, {
      keyRanges: [{ lower: key(0, 0, 0), upper: null, limit: 20 }],
      objectRanges: [{ lower: value(...digest(127), 0), upper: null, limit: 20 }],
    });
    same(next.keyRanges[0]![1].map(([k]) => k), keys.slice(3));
    same(next.objectRanges[0]![1], digests.slice(2));
    const absent = await load(f.store, { keyRanges: [{ lower: key(1), upper: key(2), limit: 1 }],
      objectRanges: [{ lower: digest(1), upper: digest(2), limit: 1 }] });
    same(absent.keyRanges[0]![1].length, 0); same(absent.objectRanges[0]![1], []);
  } },
  { name: "object puts keep the first bytes, including within a flush", async run(f) {
    const state = (await load(f.store)).store;
    await persist(f.store, state,
      batch(0, { putObjects: [[digest(1), value(1)]] }),
      batch(1, { putObjects: [[digest(1), value(2)]] }));
    await persist(f.store, state, batch(2, { putObjects: [[digest(1), value(3)]] }));
    same((await load(f.store, { objects: [digest(1)] })).objects, [[digest(1), value(1)]]);
    await persist(f.store, state, batch(3, { deleteObjects: [digest(1)] }), batch(4, { putObjects: [[digest(1), value(4)]] }));
    same((await load(f.store, { objects: [digest(1)] })).objects, [[digest(1), value(4)]]);
  } },
  { name: "key and library revisions advance across delete and recreate without ABA", async run(f) {
    const state = (await load(f.store)).store;
    await persist(f.store, state, batch(0, { putKeys: [[key(), value(1)]] }), batch(1, { deleteKeys: [key()] }));
    assert((await load(f.store, { keys: [key()] })).keys[0]![1] === null, "deleted key remains present");
    await persist(f.store, state, batch(2, { putKeys: [[key(), value(2)]] }), batch(3, { putObjects: [[digest(1), value(9)]] }));
    const current = await load(f.store, { keys: [key()] });
    same(current.store.revision, 4); same(current.keys[0]![1]!.revision, 3);
    const stale = flush(current.store, batch(4, { putKeys: [[key(), value(99)]] }));
    const old = f.store;
    f.store = await f.clear();
    const fresh = (await load(f.store)).store;
    assert(!equalBytes(fresh.id, state.id), "recreated library reused its id");
    same(fresh.revision, 0);
    await rejected(() => old.load(encodeLoadRequest(empty)));
    await rejected(() => old.persist(stale));
    await persist(f.store, fresh, batch(0), batch(1), batch(2), batch(3));
    const reply = decodeFlushReply(await f.store.persist(stale));
    assert("conflict" in reply, "old id accepted at the same revision");
    assert((await load(f.store, { keys: [key()] })).keys[0]![1] === null, "stale write resurrected a key");
  } },
  { name: "two writers from the same base have exactly one winner", async run(f) {
    const state = (await load(f.store)).store;
    const other = await f.connect();
    const replies = await Promise.all([f.store, other].map((store, index) =>
      persist(store, state, batch(0, { putKeys: [[key(), value(index + 1)]] }))));
    assert(replies.filter((r) => "persisted" in r).length === 1, "CAS allowed two winners");
    assert(replies.filter((r) => "conflict" in r).length === 1, "CAS did not report conflict");
    const winner = replies.findIndex((r) => "persisted" in r) + 1;
    same((await load(other, { keys: [key()] })).keys[0]![1]!.value, value(winner));
  } },
  { name: "multi-batch persist rolls back a staged transaction fault", async run(f) {
    const state = (await load(f.store)).store;
    const before = await snapshot(f.store);
    f.failNextPersist(2);
    const batches = [batch(0, { putObjects: [[digest(1), value(1)]], putKeys: [[key(1), value(1)]] }),
      batch(1, { putKeys: [[key(2), value(2)]] })];
    await rejected(() => f.store.persist(flush(state, ...batches)));
    same(await snapshot(f.store), before);
    const reply = await persist(f.store, state, ...batches);
    assert("persisted" in reply && reply.persisted === 2, "retry failed after rollback");
  } },
  { name: "confirmation observes committed state and failures never confirm", async run(f) {
    const state = (await load(f.store)).store;
    const message = flush(state, batch(0, { putKeys: [[key(), value(1)]] }));
    let confirmed = false;
    let committed = false;
    f.observeNextCommit(() => { committed = true; });
    let observed: Promise<void> = Promise.resolve();
    const cache: CacheHandle = {
      takeRequest: () => undefined, load: () => true, unconfirmed: () => message,
      confirm(reply) {
        assert(committed, "persistence was confirmed before the storage commit event");
        const result = decodeFlushReply(reply);
        assert("persisted" in result, "confirmation received a conflict");
        confirmed = true;
        observed = load(f.store, { keys: [key()] }).then((loaded) => {
          same(loaded.store.revision, 1); same(loaded.keys[0]![1]!.value, value(1));
        });
        return true;
      },
    };
    same(await new StorageHost(f.store, cache).run(() => 42), 42);
    assert(confirmed, "result published without confirmation"); await observed;
    confirmed = false;
    committed = false;
    f.observeNextCommit(() => { committed = true; });
    cache.unconfirmed = () => flush(state, batch(1, { putKeys: [[key(), value(2)]] }));
    f.failNextPersist();
    await rejected(() => new StorageHost(f.store, cache).run(() => 43));
    assert(!confirmed, "failed persistence was confirmed");
    assert(!committed, "failed transaction committed");
    same((await load(f.store)).store.revision, 1);
  } },
  { name: "same initial state and messages produce identical bytes", async run(f, factory) {
    const second = await factory();
    try {
      const a = (await load(f.store)).store; const b = (await load(second.store)).store;
      const changes = [batch(0, { putObjects: [[digest(1), value(9)]], putKeys: [[key(0), value(8)]] }),
        batch(1, { putKeys: [[key(128), value(7)]] })];
      same(await f.store.persist(flush(a, ...changes)), await second.store.persist(flush(b, ...changes)));
      const wanted = { keyRanges: [{ ...all, limit: 512 }], objectRanges: [{ ...all, limit: 512 }] };
      const one = await load(f.store, wanted); const two = await load(second.store, wanted);
      two.store.id = one.store.id;
      // Library ids are fresh lifecycle identities; all state and protocol effects are equal.
      same(encodeLoaded(one), encodeLoaded(two));
    } finally { await second.dispose(); }
  } },
  { name: "malformed CBOR, lists, digests and numbers reject the entire message", async run(f) {
    const state = (await load(f.store)).store;
    const before = await snapshot(f.store);
    const valid = batch(0, { putKeys: [[key(), value(1)]] });
    const invalid = [
      value(0x9f, 0xff), value(0x81), value(0x80, 0),
      encode([1, state.id, []]),
      flush(state, batch(0, { putObjects: [[value(1), value()]] })),
      flush(state, batch(0, { putKeys: [[key(2), value()], [key(1), value()]] })),
      flush(state, batch(0, { deleteKeys: [key(), key()] })),
      flush(state, batch(0, { putKeys: [[key(), value()]], deleteKeys: [key()] })),
      flush(state, batch(0, { putObjects: [[digest(1), value()]], deleteObjects: [digest(1)] })),
      flush(state, valid, batch(2)),
      flush(state, { ...valid, revision: 0 }),
      flush(state, { ...valid, base: Number.MAX_SAFE_INTEGER, revision: Number.MAX_SAFE_INTEGER }),
      flush(state, batch(0, { putKeys: [[value(1), value()]] })),
      // Canonical CBOR unsigned integer beyond JavaScript's safe range.
      value(0x83, 1, 0x50, ...state.id, 0x81, 0x86, 0x1b, 0, 0x20, 0, 0, 0, 0, 0, 0, 1, 0x80, 0x80, 0x80, 0x80),
      value(0x83, 1, 0x50, ...state.id, 0x81, 0x86, 0x20, 1, 0x80, 0x80, 0x80, 0x80),
      value(0x83, 1, 0x50, ...state.id, 0x81, 0x86, 0xf9, 0x3c, 0, 1, 0x80, 0x80, 0x80, 0x80),
    ];
    for (const message of invalid) {
      await rejected(() => f.store.persist(message));
      same(await snapshot(f.store), before);
    }
    const loads = [value(0x80), encodeLoadRequest({ ...empty, objects: [value()] }),
      encodeLoadRequest({ ...empty, keys: [key(), key()] }),
      encodeLoadRequest({ ...empty, keyRanges: [{ ...all, limit: 0 }] }),
      encodeLoadRequest({ ...empty, keyRanges: [{ lower: key(), upper: key(), limit: 1 }] })];
    for (const message of loads) await rejected(() => f.store.load(message));
  } },
  { name: "key lengths, per-value bounds and load budgets reject before writes", async run(f) {
    const state = (await load(f.store)).store;
    const before = await snapshot(f.store);
    const tooLong = new Uint8Array(HOST_LIMITS.keyBytes + 3);
    const invalid = [batch(0, { putKeys: [[tooLong, value()]] }), batch(0, { deleteKeys: [tooLong] }),
      batch(0, { putObjects: [[digest(1), new Uint8Array(HOST_LIMITS.valueBytes + 1)]] }),
      batch(0, { putKeys: [[key(), new Uint8Array(HOST_LIMITS.valueBytes + 1)]] })];
    for (const change of invalid) {
      await rejected(() => f.store.persist(flush(state, batch(0), { ...change, base: 1, revision: 2 })));
      same(await snapshot(f.store), before);
    }
    for (const wanted of [
      { keys: [tooLong] }, { keyRanges: [{ ...all, lower: tooLong }] },
      { keyRanges: [{ ...all, upper: tooLong }] }, { objectRanges: [{ ...all, limit: 1025 }] },
      { keys: [key()], keyRanges: [all] },
    ]) await rejected(() => f.store.load(encodeLoadRequest({ ...empty, ...wanted })));
    const maxKey = new Uint8Array(HOST_LIMITS.keyBytes + 2);
    await persist(f.store, state, batch(0, { putKeys: [[maxKey, value(1)]] }));
    same((await load(f.store, { keys: [maxKey] })).keys[0]![1]!.value, value(1));
    await load(f.store, { keyRanges: [all] });
  } },
  { name: "operation budgets cover all batches and count object deletions", async run(f) {
    const state = (await load(f.store)).store;
    const deletes = Array.from({ length: HOST_LIMITS.operations }, (_, n) => {
      const bytes = new Uint8Array(32); bytes[30] = n >> 8; bytes[31] = n & 255; return bytes;
    });
    const before = await snapshot(f.store);
    await rejected(() => f.store.persist(flush(state,
      batch(0, { deleteObjects: deletes }), batch(1, { deleteKeys: [key()] }))));
    same(await snapshot(f.store), before);
    await rejected(() => f.store.persist(flush(state, batch(0, { deleteObjects: [...deletes, digest(255)] }))));
    same(await snapshot(f.store), before);
    assert("persisted" in await persist(f.store, state, batch(0, { deleteObjects: deletes })), "exact operation budget rejected");
  } },
  { name: "byte budgets exclude digest and space framing and reject excess replies", async run(f) {
    const state = (await load(f.store)).store;
    const bytes = new Uint8Array(HOST_LIMITS.valueBytes);
    const puts: [Uint8Array, Uint8Array][] = [1, 2, 3, 4].map((space) => [value(0, space), bytes]);
    const before = await snapshot(f.store);
    await rejected(() => f.store.persist(flush(state,
      batch(0, { putKeys: puts }), batch(1, { putObjects: [[digest(1), value(1)]] }))));
    same(await snapshot(f.store), before);
    await rejected(() => f.store.persist(flush(state, batch(0, { putKeys: puts, putObjects: [[digest(1), value(1)]] }))));
    same(await snapshot(f.store), before);
    assert("persisted" in await persist(f.store, state, batch(0, { putKeys: puts })), "exact byte budget rejected");
    const wanted = { keys: puts.map(([key]) => key), keyRanges: [{ ...all, limit: 1 }] };
    await rejected(() => f.store.load(encodeLoadRequest({ ...empty, ...wanted })));
    const loaded = await load(f.store, { keys: puts.map(([key]) => key) });
    same(loaded.keys.length, 4);
    assert(loaded.keys.every(([, record]) => record?.value.length === bytes.length), "exact reply budget truncated");
  } },
  { name: "framing and allocation bounds reject oversized or deeply nested CBOR", async run(f) {
    const messages = [new Uint8Array(HOST_LIMITS.messageBytes + 1),
      value(0x9a, 0, 0, 0x27, 0x11), // array claims 10,001 entries before allocation
      value(0x5a, 1, 0, 0, 1), // byte string claims 16 MiB + 1 before allocation
      new Uint8Array(64).fill(0x81)];
    for (const message of messages) {
      await rejected(() => f.store.load(message));
      await rejected(() => f.store.persist(message));
    }
    same((await load(f.store)).store.revision, 0);
  } },
];

/** Each case owns a fresh library and always releases handles, including after failure. */
export async function runConformanceCase(name: string, factory: StoreFactory): Promise<void> {
  const test = conformanceCases.find((test) => test.name === name);
  assert(test !== undefined, `unknown conformance case: ${name}`);
  const fixture = await factory();
  try { await test.run(fixture, factory); } finally { await fixture.dispose(); }
}
