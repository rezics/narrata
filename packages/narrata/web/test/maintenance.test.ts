import "fake-indexeddb/auto";
import { expect, test } from "vitest";
import { loadAuthoring, AuthoringError, type Maintenance, type MaintenanceRequest, type ObservedRoot } from "@rezics/narrata/server";
import { openBook, locateSave, type Book } from "@rezics/narrata";
import { AuthenticatedLibrary, IndexedDbStore, decodeLoaded, encodeLoaded, encodeFlush, encodeLoadRequest, StoreSuperseded, WriteRejected, type CacheStore } from "@rezics/narrata/storage";
import { demoPack, execution } from "./fixture.js";

const empty = encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] });
const secondExecution = "execution:0190f2a0000070008000000000000002";
async function fixture(transport?: (store: IndexedDbStore) => CacheStore) {
  const store = await IndexedDbStore.open(`maintenance-${crypto.randomUUID()}`);
  const state = decodeLoaded(await store.load(empty)).store;
  const authoring = await loadAuthoring();
  const publication = authoring.publicationFromPack(demoPack);
  const create = async (storage: CacheStore = transport?.(store) ?? store) => {
    const maintenance = authoring.createMaintenance({ storage, storeId: state.id });
    await maintenance.registerPublication(publication);
    return maintenance;
  };
  return { store, state, create, publication };
}
async function roots(maintenance: Maintenance): Promise<ObservedRoot[]> {
  const items: ObservedRoot[] = [];
  let request: MaintenanceRequest = { kind:"roots",after:null,limit:1 };
  for (;;) {
    const result = await maintenance.run(request);
    if (result.kind !== "roots") throw new Error("Unexpected result");
    items.push(...result.items);
    if (!result.more) return items;
    request = { kind:"roots",after:result.items.at(-1)!.key,limit:1 };
  }
}
async function choose(book: Book): Promise<void> {
  const view = (await book.inspect()).view;
  if (view.interaction.kind !== "choose") return;
  const option = view.interaction.options.find(o => o.enabled)!;
  await book.choose(view.cursor,view.interaction.choice_point,[option.id]);
}

test("cold CacheStore export/import, shallow page restore, reopen and continue use public entries", async () => {
  const source = await fixture(), target = await fixture();
  const book = await openBook({ pack:demoPack,execution,storage:source.store });
  const a = await source.create(), b = await target.create();
  try {
    await choose(book); await choose(book);
    const before = (await book.inspect()).view;
    const selection = {execution,artifact:book.artifactId,cursor:before.cursor};
    for (const ancestors of [undefined,0,1]) {
      const bytes = await a.exportSave({...selection,...(ancestors === undefined ? {} : {ancestors})});
      // Reader restore validates the same bytes before replacing its persistent cursor.
      const snapshot = await openBook({pack:demoPack,execution:secondExecution});
      try {
        await snapshot.restore(Buffer.from(bytes).toString("hex"));
        expect((await snapshot.inspect()).view).toMatchObject({cursor:before.cursor,shared:before.shared,frames:before.frames,interaction:before.interaction});
      } finally { await snapshot.close(); }
      const existing = (await rootsIfInitialized(b)).find(r => r.key.namespace === 3);
      await b.importSave(bytes,{...selection,expected_revision:existing?.revision ?? null,now:100});
      const reopened = await openBook({pack:demoPack,execution,storage:target.store});
      try {
        expect((await reopened.inspect()).view).toMatchObject({cursor:before.cursor,shared:before.shared,frames:before.frames,interaction:before.interaction});
        await choose(reopened);
      } finally { await reopened.close(); }
    }
  } finally { await Promise.all([book.close(),a.close(),b.close()]); source.store.close();target.store.close();source.publication.free();target.publication.free(); }
});
async function rootsIfInitialized(maintenance: Maintenance): Promise<ObservedRoot[]> {
  try { return await roots(maintenance); }
  catch (error) { if (error instanceof AuthoringError && error.code === "corrupt_save") return []; throw error; }
}

test("root CAS deletion, complete dry-run and GC preserve other executions and expose expiry", async () => {
  const f = await fixture();
  const first = await openBook({pack:demoPack,execution,storage:f.store});
  await choose(first);
  const other = await openBook({pack:demoPack,execution:secondExecution,storage:f.store});
  const otherCursor = (await other.inspect()).view.cursor;
  const m = await f.create();
  try {
    const observed = await roots(m);
    expect(observed.some(r => r.key.namespace === 1)).toBe(true);
    const selected = observed.filter(r => r.key.owner === execution.slice(10));
    await first.reload();
    await choose(first);
    await expect(m.run({kind:"delete_roots",roots:selected})).rejects.toMatchObject({code:"stale_input"});
    expect(await locateSave(f.store,execution)).not.toBeNull();
    const refreshed = (await roots(m)).filter(r => r.key.owner === execution.slice(10));
    await m.run({kind:"delete_roots",roots:refreshed});
    const revision = decodeLoaded(await f.store.load(empty)).store.revision;
    const dry = await m.run({kind:"collect",now:1000,grace_seconds:10,dry_run:true});
    expect(dry.kind).toBe("collected");
    expect(decodeLoaded(await f.store.load(empty)).store.revision).toBe(revision);
    const actual = await m.run({kind:"collect",now:1000,grace_seconds:10,dry_run:false});
    expect(actual.kind).toBe("collected");
    expect(await locateSave(f.store,execution)).toBeNull();
    expect((await locateSave(f.store,secondExecution))?.cursor).toBe(otherCursor);
    expect(await m.run({kind:"integrity"})).toEqual({kind:"integrity",issues:[]});
    expect(await m.run({kind:"pins",after:null,limit:1,now:1000})).toEqual({kind:"pins",items:[],more:false});
  } finally { await Promise.all([first.close(),other.close(),m.close()]);f.store.close();f.publication.free(); }
});

test("import corruption, budgets, exact identity and known quota rejection leave durable progress reopenable", async () => {
  let reject = false;
  const f = await fixture(store => ({load:r=>store.load(r),persist:b=>reject ? Promise.reject(new WriteRejected("quota")) : store.persist(b)}));
  const book = await openBook({pack:demoPack,execution,storage:f.store});
  const m = await f.create();
  try {
    const old = (await book.inspect()).view.cursor;
    await choose(book);
    const cursor = (await book.inspect()).view.cursor;
    const bytes = await m.exportSave({execution,artifact:book.artifactId,cursor});
    const corrupt = bytes.slice(); corrupt[corrupt.length-1]! ^= 1;
    await expect(m.importSave(corrupt,{execution,artifact:book.artifactId,cursor,expected_revision:null,now:100})).rejects.toThrow();
    expect((await locateSave(f.store,execution))?.cursor).toBe(cursor);
    const selection = {execution:secondExecution,artifact:book.artifactId,cursor,expected_revision:null,now:100};
    reject = true;
    await expect(m.importSave(bytes,selection)).rejects.toBeInstanceOf(WriteRejected);
    expect(await locateSave(f.store,secondExecution)).toBeNull();
    reject = false;
    await m.importSave(bytes,selection);
    expect((await locateSave(f.store,secondExecution))?.cursor).toBe(cursor);
    expect(cursor).not.toBe(old);
    await expect(m.exportSave({execution,artifact:book.artifactId,cursor:old})).rejects.toMatchObject({code:"stale_input"});
    const authoring = await loadAuthoring();
    const limited = authoring.createMaintenance({storage:f.store,storeId:f.state.id,limits:{object_reads:0,bytes:1024,pages:1,work:1024,page_size:1,loads:4,load_bytes:4096}});
    try { await expect(limited.run({kind:"collect",now:1000,grace_seconds:0,dry_run:false})).rejects.toMatchObject({code:"limit"}); }
    finally { await limited.close(); }
    expect((await locateSave(f.store,execution))?.cursor).toBe(cursor);
  } finally { await book.close();await m.close();f.store.close();f.publication.free(); }
});

test("unknown writes retain original bytes; close and recreated library reject late replies", async () => {
  let lost = false;
  const writes: Uint8Array[] = [];
  const f = await fixture(store => ({load:r=>store.load(r),persist:async b => {writes.push(b.slice()); if (lost) {lost=false;throw new Error("lost");} return store.persist(b);}}));
  const book = await openBook({pack:demoPack,execution,storage:f.store});
  const m = await f.create();
  try {
    const selected = await roots(m); lost = true;
    await expect(m.run({kind:"delete_roots",roots:selected})).rejects.toThrow("lost");
    await expect(m.run({kind:"integrity"})).rejects.toThrow("unknown outcome");
    expect(await m.retryPending()).toBe(true);
    expect(writes.at(-1)).toEqual(writes.at(-2));
    expect(await roots(m)).toEqual([]);
    let resolve!: () => void;
    const wait = new Promise<void>(r => {resolve=r;});
    const late = await f.create({load:async r=>{await wait;return f.store.load(r);},persist:b=>f.store.persist(b)});
    const result = late.run({kind:"integrity"});
    await Promise.resolve(); const closing = late.close();resolve();
    await expect(result).rejects.toThrow("closed");await closing;
    const replacement = await fixture();
    const stale = await f.create(replacement.store);
    try { await expect(stale.run({kind:"integrity"})).rejects.toBeInstanceOf(WriteRejected); }
    finally {await stale.close();replacement.store.close();replacement.publication.free();}
  } finally {await m.close();await book.close();f.store.close();f.publication.free();}
});

test("account generation closes maintenance and rejects an already loaded late read", async () => {
  const f = await fixture();
  const book = await openBook({pack:demoPack,execution,storage:f.store});
  let release!: () => void;
  let entered!: () => void;
  const waiting = new Promise<void>(r => { release = r; });
  const started = new Promise<void>(r => { entered = r; });
  const library = new AuthenticatedLibrary({
    storeId:f.state.id,
    store:{load:async request => {const reply = await f.store.load(request);entered();await waiting;return reply;},persist:b=>f.store.persist(b)},
    release:()=>{},remove:async()=>{},clearContent:()=>{},
  });
  const m = await f.create(library);
  try {
    const reading = m.run({kind:"integrity"});
    await started;
    const closing = library.close();
    release();
    await expect(reading).rejects.toThrow("closed");
    await closing;
    await expect(m.inspectSave("invalid")).rejects.toThrow("closed");
    expect(await locateSave(f.store,execution)).not.toBeNull();
  } finally {release();await m.close();await book.close();f.store.close();f.publication.free();}
});

test("missing objects and unregistered kinds remain distinct and never authorize a sweep", async () => {
  const f = await fixture();
  const book = await openBook({pack:demoPack,execution,storage:f.store});
  const m = await f.create();
  const missing = await f.create({
    load:async request => {
      const loaded = decodeLoaded(await f.store.load(request));
      loaded.objects = loaded.objects.map(([id])=>[id,null]);
      return encodeLoaded(loaded);
    }, persist:bytes=>f.store.persist(bytes),
  });
  try {
    const cursor = (await book.inspect()).view.cursor;
    await expect(missing.exportSave({execution,artifact:book.artifactId,cursor})).rejects.toMatchObject({code:"needs_object"});
    const [reference,bytes] = [...f.publication.objects()][0]!;
    const state = decodeLoaded(await f.store.load(empty)).store;
    await f.store.persist(encodeFlush({store:state.id,batches:[{
      base:state.revision,revision:state.revision+1,
      putObjects:[[new Uint8Array(Buffer.from(reference.object_id.replace(/^object:/,""),"hex")),bytes]],deleteObjects:[],putKeys:[],deleteKeys:[],
    }]}));
    const revision = decodeLoaded(await f.store.load(empty)).store.revision;
    await expect(m.run({kind:"collect",now:1000,grace_seconds:0,dry_run:false})).rejects.toMatchObject({code:"unregistered_kind"});
    expect(decodeLoaded(await f.store.load(empty)).store.revision).toBe(revision);
    expect((await locateSave(f.store,execution))?.cursor).toBe(cursor);
  } finally {await Promise.all([m.close(),missing.close(),book.close()]);f.store.close();f.publication.free();}
});
