import "fake-indexeddb/auto";
import { expect, test, vi } from "vitest";
import { AuthenticatedLibrary, createOnlineSession, createReader, LibraryClosed, locateSave, openBook, retainedSave, WriteRejected, type BookView, type OnlineSession, type OnlineSessionOptions } from "@rezics/narrata";
import { decodeFlush, decodeLoaded, encodeFlush, encodeLoadRequest, IndexedDbStore, type CacheStore } from "@rezics/narrata/storage";
import { demoPack, execution, splitPack } from "./fixture.js";

const empty = encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] });
function choice(view: BookView) {
  const interaction = view.view.interaction;
  if (interaction.kind !== "choose") throw new Error("Expected a choice");
  const option = interaction.options.find(option => option.enabled);
  if (!option) throw new Error("Expected an enabled option");
  return { cursor: view.view.cursor, point: interaction.choice_point, option: option.id };
}
async function fixture() {
  const store = await IndexedDbStore.open(`online-${crypto.randomUUID()}`);
  const state = decodeLoaded(await store.load(empty)).store;
  const reference = await openBook({ pack: demoPack, execution });
  const artifact = reference.artifactId; await reference.close();
  const parts = splitPack(demoPack);
  const currentArtifact = vi.fn(async () => artifact);
  const clearContent = vi.fn();
  const create = (transport: CacheStore = store, extra: Partial<OnlineSessionOptions> = {}) => {
    const library = new AuthenticatedLibrary({ store: transport, storeId: state.id, release: () => undefined,
      clearContent, remove: async () => undefined });
    return createOnlineSession({ library, execution, currentArtifact,
      loadArtifact: async () => ({ manifest: parts.manifest, names: parts.names, fetchChunk: async id => {
        const bytes = parts.objects.get(id); if (!bytes) throw new Error("Missing artifact object"); return bytes;
      } }), ...extra });
  };
  return { store, state, create, currentArtifact, clearContent };
}
async function advance(session: OnlineSession) {
  const next = choice(await session.inspect());
  return session.choose(next.cursor, next.point, [next.option]);
}

test("an unknown uncommitted write and an initial root retain their bytes for successful retries", async () => {
  const f = await fixture(); let drop = true;
  const writes: Uint8Array[] = [];
  const session = f.create({ load: request => f.store.load(request), persist: async batch => {
    writes.push(batch.slice()); if (drop) { drop = false; throw new Error("connection lost before response"); } return f.store.persist(batch);
  } });
  try {
    await expect(session.start()).rejects.toThrow("connection lost");
    expect(session.getSnapshot()).toMatchObject({ status: "unknown", progress: null });
    await session.retrySave(); expect(writes[0]).toEqual(writes[1]);
    expect(session.getSnapshot().progress?.view.depth).toBe(0);
    drop = true; await expect(advance(session)).rejects.toThrow("connection lost");
    await session.retrySave(); expect(writes.at(-1)).toEqual(writes.at(-2));
    expect(session.getSnapshot().progress?.view.depth).toBe(1);
  } finally { await session.close(); f.store.close(); }
});

test("discovery is read-only; lost acknowledgement retries original bytes and proves a descendant's ancestry", async () => {
  const f = await fixture(); let lost = false;
  const writes: Uint8Array[] = [];
  const first = f.create({ load: request => f.store.load(request), persist: async batch => {
    writes.push(batch.slice()); const reply = await f.store.persist(batch);
    if (lost) { lost = false; throw new Error("lost acknowledgement"); } return reply;
  } });
  const second = f.create();
  try {
    expect(await locateSave(f.store, execution)).toBeNull();
    expect(decodeLoaded(await f.store.load(empty)).store.revision).toBe(0);
    await first.start(); const previous = first.getSnapshot().progress;
    lost = true; await expect(advance(first)).rejects.toThrow("lost acknowledgement");
    const unknown = first.getSnapshot();
    expect(unknown.status).toBe("unknown"); expect(unknown.progress).toBe(previous);
    expect(unknown.candidate).toMatch(/^commit:/);
    const count = writes.length; const batch = writes.at(-1);
    await expect(advance(first)).rejects.toThrow("Resolve");
    await expect(first.reload()).rejects.toThrow("Retry and verify");
    expect(writes).toHaveLength(count);
    await second.start(); await advance(second);
    expect(second.getSnapshot().progress?.view.depth).toBe(2);
    await first.retrySave();
    expect(writes.at(-1)).toEqual(batch);
    expect(first.getSnapshot()).toMatchObject({ status: "saved", candidate: null, progress: { view: { depth: 2 } } });
    expect(f.currentArtifact).toHaveBeenCalledTimes(1);
  } finally { await first.close(); await second.close(); f.store.close(); }
});

test.each(["orphan", "detached active cursor", "missing object", "read failure", "budget"])("verification %s remains unknown with original progress", async mode => {
  const f = await fixture(); let lose = false; let failRead = false;
  const writes: Uint8Array[] = [];
  const transport: CacheStore = { load: request => failRead ? Promise.reject(new Error("verification read failed")) : f.store.load(request), persist: async batch => {
    writes.push(batch.slice()); const result = await f.store.persist(batch);
    if (lose) { lose = false; throw new Error("reply lost"); } return result;
  } };
  const session = f.create(transport, mode === "budget" ? { verificationLimits: { roots: 1, commits: 1 } } : {});
  try {
    await session.start(); const before = session.getSnapshot().progress;
    lose = true; await expect(advance(session)).rejects.toThrow("reply lost");
    const candidate = session.getSnapshot().candidate;
    if (mode === "missing object") {
      if (!candidate) throw new Error("Expected pending candidate");
      const current = decodeLoaded(await f.store.load(empty)).store;
      await f.store.persist(encodeFlush({ store: current.id, batches: [{ base: current.revision, revision: current.revision + 1,
        putObjects: [], deleteObjects: [new Uint8Array(Buffer.from(candidate.slice(7), "hex"))], putKeys: [], deleteKeys: [] }] }));
    }
    if (mode === "orphan" || mode === "detached active cursor") {
      const roots = new Map<string, Uint8Array>();
      for (const write of writes) for (const batch of decodeFlush(write).batches) for (const [key] of batch.putKeys) {
        if (key[0] === 0 && key[1] === 2 && (mode === "orphan" || key[2] === 3)) roots.set(String(key), key);
      }
      const current = decodeLoaded(await f.store.load(empty)).store;
      await f.store.persist(encodeFlush({ store: current.id, batches: [{ base: current.revision, revision: current.revision + 1,
        putObjects: [], deleteObjects: [], putKeys: [], deleteKeys: [...roots.values()].sort((a, b) => Buffer.compare(a, b)) }] }));
    }
    if (mode === "read failure") failRead = true;
    if (mode === "detached active cursor") {
      if (!candidate) throw new Error("Expected retained candidate");
      expect(await retainedSave(f.store, execution, candidate)).toBe(true);
    }
    await expect(session.retrySave()).rejects.toThrow();
    expect(session.getSnapshot()).toMatchObject({ status: "unknown", candidate });
    expect(session.getSnapshot().progress).toBe(before);
    if (mode === "detached active cursor") expect(await locateSave(f.store, execution)).toBeNull();
    if (mode === "read failure") { failRead = false; await session.retrySave(); expect(session.getSnapshot().status).toBe("saved"); }
  } finally { await session.close(); f.store.close(); }
});

test("a definite rejection of a retry cannot decide an earlier unknown attempt", async () => {
  const f = await fixture(); let mode: "normal" | "lost" | "quota" = "normal";
  const session = f.create({ load: request => f.store.load(request), persist: async batch => {
    if (mode === "quota") throw new WriteRejected("quota");
    const reply = await f.store.persist(batch);
    if (mode === "lost") throw new Error("lost response"); return reply;
  } });
  try {
    await session.start(); const progress = session.getSnapshot().progress;
    mode = "lost"; await expect(advance(session)).rejects.toThrow("lost response");
    const candidate = session.getSnapshot().candidate;
    mode = "quota"; await expect(session.retrySave()).rejects.toBeInstanceOf(WriteRejected);
    expect(session.getSnapshot()).toMatchObject({ status: "unknown", candidate });
    expect(session.getSnapshot().progress).toBe(progress);
    mode = "normal"; await session.retrySave(); expect(session.getSnapshot().status).toBe("saved");
  } finally { await session.close(); f.store.close(); }
});

test("definite quota rollback and CAS conflict allow safe reload", async () => {
  const f = await fixture(); let quota = false;
  const first = f.create({ load: request => f.store.load(request), persist: batch => quota ? Promise.reject(new WriteRejected("quota")) : f.store.persist(batch) });
  const second = f.create();
  try {
    await first.start(); await second.start();
    const before = first.getSnapshot().progress;
    quota = true; await expect(advance(first)).rejects.toBeInstanceOf(WriteRejected);
    expect(first.getSnapshot()).toMatchObject({ status: "quota", progress: before });
    quota = false; await first.reload(); expect(first.getSnapshot().progress?.view.depth).toBe(0);
    await advance(second);
    await expect(advance(first)).rejects.toThrow(); expect(first.getSnapshot().status).toBe("conflict");
    await first.reload(); expect(first.getSnapshot().progress?.view.depth).toBe(1);
  } finally { await first.close(); await second.close(); f.store.close(); }
});

test("close/account switch withholds late success, rejects at call boundary and clears old content", async () => {
  const f = await fixture(); let pause = false; let release!: () => void; let entered!: () => void;
  const started = new Promise<void>(done => { entered = done; });
  const barrier = new Promise<void>(done => { release = done; });
  const old = f.create({ load: request => f.store.load(request), persist: async batch => {
    const reply = await f.store.persist(batch); if (pause) { entered(); await barrier; } return reply;
  } });
  const other = await fixture(); const fresh = other.create();
  try {
    await old.start(); pause = true;
    const operation = advance(old); await started;
    const closing = old.close();
    expect(old.getSnapshot()).toEqual({ status: "closed", progress: null, candidate: null });
    expect(f.clearContent).toHaveBeenCalledTimes(1);
    await expect(old.inspect()).rejects.toBeInstanceOf(LibraryClosed);
    await fresh.start(); const snapshot = fresh.getSnapshot();
    release(); await expect(operation).rejects.toBeInstanceOf(LibraryClosed); await closing;
    expect(fresh.getSnapshot()).toBe(snapshot);
    expect(old.getSnapshot().status).toBe("closed");
  } finally { release?.(); await old.close(); await fresh.close(); f.store.close(); other.store.close(); }
});

test("manifest identity is checked before chunks; unavailable artifacts preserve durable roots", async () => {
  const f = await fixture(); const first = f.create();
  await first.start(); const cursor = first.getSnapshot().progress?.view.cursor; await first.close();
  const fetchChunk = vi.fn(async () => new Uint8Array());
  const broken = f.create(f.store, { loadArtifact: async () => ({ manifest: new Uint8Array(), fetchChunk }) });
  try {
    await expect(broken.start()).rejects.toThrow();
    expect(broken.getSnapshot().status).toBe("unavailable"); expect(fetchChunk).not.toHaveBeenCalled();
    expect((await locateSave(f.store, execution))?.cursor).toBe(cursor);
  } finally { await broken.close(); f.store.close(); }
});

test("a valid manifest for another artifact is refused before loading its chunks or creating a cursor", async () => {
  const f = await fixture(); const parts = splitPack(demoPack);
  const fetchChunk = vi.fn(async () => parts.chunks[0]!);
  const session = f.create(f.store, { currentArtifact: async () => `artifact:${"0".repeat(64)}`,
    loadArtifact: async () => ({ manifest: parts.manifest, fetchChunk }) });
  try {
    await expect(session.start()).rejects.toThrow("differs from the saved artifact");
    expect(session.getSnapshot().status).toBe("unavailable"); expect(fetchChunk).not.toHaveBeenCalled();
    expect(await locateSave(f.store, execution)).toBeNull();
  } finally { await session.close(); f.store.close(); }
});

test("account invalidation clears the reader and discards a late content response", async () => {
  const f = await fixture();
  const library = new AuthenticatedLibrary({ store: f.store, storeId: f.state.id, release: () => undefined, clearContent: f.clearContent, remove: async () => undefined });
  const session = f.create(f.store, { library });
  await session.start();
  let release!: () => void; let entered!: () => void;
  const barrier = new Promise<void>(done => { release = done; });
  const requested = new Promise<void>(done => { entered = done; });
  const reader = createReader(session, { resolve: async request => {
    entered(); await barrier;
    return request.items.map(() => ({ status: "ok", revision: "old-account", payload: { text: "Former account content" } }));
  } }, { online: session });
  try {
    const pending = reader.start(); await requested;
    const closing = library.close();
    expect(session.getSnapshot().status).toBe("closed");
    expect(reader.getSnapshot()).toMatchObject({ screen: null, save: { status: "closed" } });
    release(); expect(await pending).toBe(false); await closing;
    expect(reader.getSnapshot()).toMatchObject({ screen: null, save: { status: "closed" } });
  } finally { release?.(); reader.dispose(); await session.close(); f.store.close(); }
});
