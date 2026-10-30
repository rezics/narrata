import "fake-indexeddb/auto";
import { expect, test } from "vitest";
import { loadAuthoring, AuthoringError, defaultMaintenanceLimits, MaintenanceLimit, type Maintenance, type MaintenanceRequest, type ObservedRoot, type PackageSource, type ProjectManifest } from "@rezics/narrata/server";
import { openBook, locateSave, type Book } from "@rezics/narrata";
import { AuthenticatedLibrary, IndexedDbStore, decodeLoaded, encodeLoaded, decodeFlush, encodeFlush, encodeLoadRequest, StoreSuperseded, WriteRejected, type CacheStore } from "@rezics/narrata/storage";
import { demoPack, execution } from "./fixture.js";

const empty = encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] });
const secondExecution = "execution:0190f2a0000070008000000000000002";
function rawHandle(m:Maintenance): object {
  const raw:unknown=Reflect.get(m,"raw");
  if(typeof raw!=="object" || raw===null)throw new Error("Missing Wasm maintenance handle");
  return raw;
}
function observeRaw(m:Maintenance,name:string): {count:number} {
  const raw=rawHandle(m),method:unknown=Reflect.get(raw,name),counter={count:0};
  if(typeof method!=="function")throw new Error(`Missing Wasm ${name} method`);
  Reflect.set(raw,name,(...args:unknown[])=>{counter.count++;return Reflect.apply(method,raw,args);});
  return counter;
}
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

test("inspection and import reject input budgets and closed handles before binary expansion", async () => {
  const f=await fixture(), api=await loadAuthoring();
  const book=await openBook({pack:demoPack,execution,storage:f.store});
  const normal=await f.create();
  try {
    const cursor=(await book.inspect()).view.cursor, artifact=book.artifactId;
    const save=await normal.exportSave({execution,artifact,cursor});
    const selection={execution:secondExecution,artifact,cursor,expected_revision:null,now:100};
    for(const override of [{bytes:0},{bytes:1},{object_reads:0},{work:1},{work:save.length*2+10}]) {
      let loads=0,writes=0;
      const m=api.createMaintenance({storage:{load:async r=>{loads++;return f.store.load(r);},persist:async r=>{writes++;return f.store.persist(r);}},storeId:f.state.id,limits:{...defaultMaintenanceLimits,...override}});
      const inspect=observeRaw(m,"inspectSave"),importing=observeRaw(m,"import");
      await m.registerPublication(f.publication);
      try {
        await expect(m.inspectSave(save)).rejects.toThrow();
        await expect(m.importSave(save,selection)).rejects.toThrow();
        expect(loads).toBe(0);expect(writes).toBe(0);
        const entersWasm=Number(override.work===save.length*2+10);
        expect(inspect.count).toBe(entersWasm);expect(importing.count).toBe(entersWasm);
      } finally {await m.close();}
    }
    class Unconvertible extends Uint8Array { override subarray(): Uint8Array<ArrayBuffer> {throw new Error("binary conversion ran");} }
    const large=new Unconvertible(8*1024*1024);
    const tiny=api.createMaintenance({storage:f.store,storeId:f.state.id,limits:{...defaultMaintenanceLimits,bytes:1}});
    const inspection=observeRaw(tiny,"inspectSave"),importing=observeRaw(tiny,"import");
    await expect(tiny.inspectSave(large)).rejects.toBeInstanceOf(MaintenanceLimit);
    await expect(tiny.importSave(large,selection)).rejects.toBeInstanceOf(MaintenanceLimit);
    await tiny.close();
    await expect(tiny.inspectSave(large)).rejects.toThrow("closed");
    await expect(tiny.importSave(large,selection)).rejects.toThrow("closed");
    expect(inspection.count).toBe(0);expect(importing.count).toBe(0);
    expect(await normal.inspectSave(save)).toEqual({artifact,cursor});
    expect((await locateSave(f.store,execution))?.cursor).toBe(cursor);
  } finally {await Promise.all([normal.close(),book.close()]);f.store.close();f.publication.free();}
});

test("positive public load and actual reply-byte budgets accumulate across cold retries", async () => {
  const f=await fixture(), api=await loadAuthoring();
  const book=await openBook({pack:demoPack,execution,storage:f.store});
  const cursor=(await book.inspect()).view.cursor;
  const sizes:number[]=[];
  const accepted=new Map<Maintenance,{count:number}>();
  const make=async (limits=defaultMaintenanceLimits)=> {
    const m=api.createMaintenance({storage:{load:async r=>{const reply=await f.store.load(r);sizes.push(reply.byteLength);return reply;},persist:()=>{throw new Error("read-only operation wrote");}},storeId:f.state.id,limits});
    // Observe the real Wasm acceptance boundary without replacing its implementation.
    const raw=rawHandle(m);
    const load:unknown=Reflect.get(raw,"load");
    if(typeof load!=="function")throw new Error("Missing Wasm load method");
    const count={count:0};accepted.set(m,count);
    Reflect.set(raw,"load",(reply:Uint8Array)=>{count.count++;return Reflect.apply(load,raw,[reply]);});
    await m.registerPublication(f.publication);return m;
  };
  const scan=(m:Maintenance)=>m.run({kind:"roots",after:null,limit:1024});
  try {
    const full=await make();await scan(full);await full.close();
    const required=sizes.length,total=sizes.reduce((a,b)=>a+b,0);
    expect(required).toBeGreaterThan(1);expect(total).toBeGreaterThan(sizes[0]!);
    const first=sizes[0]!;
    for(const limits of [{...defaultMaintenanceLimits,loads:required-1},{...defaultMaintenanceLimits,load_bytes:0},{...defaultMaintenanceLimits,load_bytes:1},{...defaultMaintenanceLimits,load_bytes:first},{...defaultMaintenanceLimits,load_bytes:total-1}]) {
      sizes.length=0;const m=await make(limits);
      try {
        await expect(scan(m)).rejects.toBeInstanceOf(MaintenanceLimit);
        expect(sizes.length).toBeLessThanOrEqual(required);
        const received=sizes.reduce((a,b)=>a+b,0);
        expect(Reflect.get(m,"readBytes")).toBe(received);
        expect(accepted.get(m)!.count).toBe(sizes.length-Number(received>limits.load_bytes));
        if(limits.load_bytes===0)expect(sizes).toHaveLength(0);
        if(limits.load_bytes===1)expect(sizes).toHaveLength(1);
        if(limits.load_bytes===first)expect(sizes).toHaveLength(1);
        expect((await locateSave(f.store,execution))?.cursor).toBe(cursor);
        if(limits.load_bytes===total-1) {
          const boundary=sizes.length;
          expect((await m.run({kind:"pins",after:null,limit:1,now:100})).kind).toBe("pins");
          expect(Reflect.get(m,"readBytes")).toBe(sizes.slice(boundary).reduce((a,b)=>a+b,0));
        }
      } finally {await m.close();}
    }
    sizes.length=0;const exact=await make({...defaultMaintenanceLimits,loads:required,load_bytes:total});
    try {expect((await scan(exact)).kind).toBe("roots");expect(sizes.length).toBe(required);expect(accepted.get(exact)!.count).toBe(required);}
    finally {await exact.close();}
  } finally {await book.close();f.store.close();f.publication.free();}
});

test("committed reply loss retries original bytes and verifies durable roots after conflict or later rejection", async () => {
  for(const laterQuota of [false,true]) {
    let lose=false, replay=false;
    const writes:Uint8Array[]=[];
    const f=await fixture(store=>({load:r=>store.load(r),persist:async r=>{
      writes.push(r.slice());
      if(replay && laterQuota) throw new WriteRejected("quota");
      const reply=await store.persist(r);
      if(lose) {lose=false;replay=true;throw new Error("committed reply lost");}
      return reply;
    }}));
    const book=await openBook({pack:demoPack,execution,storage:f.store}), m=await f.create();
    try {
      const selected=await roots(m);lose=true;
      await expect(m.run({kind:"delete_roots",roots:selected})).rejects.toThrow("committed reply lost");
      expect(await locateSave(f.store,execution)).toBeNull();
      await expect(m.retryPending()).rejects.toBeInstanceOf(laterQuota ? WriteRejected : StoreSuperseded);
      expect(writes.at(-1)).toEqual(writes.at(-2));
      expect(await roots(m)).toEqual([]);
      expect(await m.run({kind:"integrity"})).toEqual({kind:"integrity",issues:[]});
    } finally {await Promise.all([book.close(),m.close()]);f.store.close();f.publication.free();}
  }
});

test("a held mutation cannot publish into an actually deleted and recreated database", async () => {
  const name=`maintenance-recreate-${crypto.randomUUID()}`;
  let store=await IndexedDbStore.open(name);
  const state=decodeLoaded(await store.load(empty)).store;
  const book=await openBook({pack:demoPack,execution,storage:store});
  const api=await loadAuthoring(), publication=api.publicationFromPack(demoPack);
  let enter!:()=>void,release!:()=>void;
  const started=new Promise<void>(r=>{enter=r;}),wait=new Promise<void>(r=>{release=r;});
  const m=api.createMaintenance({storeId:state.id,storage:{load:r=>store.load(r),persist:async r=>{enter();await wait;return store.persist(r);}}});
  await m.registerPublication(publication);
  try {
    const selected=await roots(m), pending=m.run({kind:"delete_roots",roots:selected});
    await started;
    const closing=m.close();await book.close();store.close();
    await IndexedDbStore.delete(name);store=await IndexedDbStore.open(name);
    expect(decodeLoaded(await store.load(empty)).store.id).not.toEqual(state.id);
    release();await expect(pending).rejects.toThrow("closed");await closing;
    expect(await locateSave(store,execution)).toBeNull();
    expect(decodeLoaded(await store.load(empty)).store.revision).toBe(0);
  } finally {release();await m.close();await book.close();store.close();publication.free();}
});

test("cold import and legacy reader restore reject corrupt preexisting destination commits and states", async () => {
  const source=await fixture(),target=await fixture();
  const incoming=await openBook({pack:demoPack,execution,storage:source.store});
  const active=await openBook({pack:demoPack,execution,storage:target.store});
  const exporting=await source.create(),receiving=await target.create();
  try {
    const a=(await active.inspect()).view;
    await choose(incoming);await choose(incoming);
    const b=(await incoming.inspect()).view,cursor=b.cursor,artifact=incoming.artifactId;
    const bytes=await exporting.exportSave({execution,artifact,cursor});
    const container=Buffer.from(bytes);let at=16+Number(container.readBigUInt64BE(8));
    const count=Number(container.readBigUInt64BE(at));at+=8;const states:Uint8Array[]=[];
    for(let i=0;i<count;i++) {const id=new Uint8Array(container.subarray(at,at+32)),kind=container.readUInt16BE(at+32),size=Number(container.readBigUInt64BE(at+36));if(kind===0x0111)states.push(id);at+=44+size;}
    const absent=decodeLoaded(await target.store.load(encodeLoadRequest({keys:[],objects:states,keyRanges:[],objectRanges:[]}))).objects.find(([,v])=>v===null)?.[0];
    if(!absent)throw new Error("Expected new incoming state");
    for(const id of [new Uint8Array(Buffer.from(cursor.split(":").at(-1)!,"hex")),absent]) {
      let state=decodeLoaded(await target.store.load(empty)).store;
      await target.store.persist(encodeFlush({store:state.id,batches:[{base:state.revision,revision:state.revision+1,putObjects:[[id,new Uint8Array([1])]],deleteObjects:[],putKeys:[],deleteKeys:[]}]}));
      const selected=(await roots(receiving)).find(r=>r.key.namespace===3)!;
      await expect(receiving.importSave(bytes,{execution,artifact,cursor,expected_revision:selected.revision,now:100})).rejects.toMatchObject({code:"corrupt_save"});
      await active.reload();await expect(active.restore(Buffer.from(bytes).toString("hex"))).rejects.toThrow();
      expect((await locateSave(target.store,execution))?.cursor).toBe(a.cursor);
      const reopened=await openBook({pack:demoPack,execution,storage:target.store});
      try {expect((await reopened.inspect()).view).toMatchObject({cursor:a.cursor,shared:a.shared,frames:a.frames,interaction:a.interaction});}
      finally {await reopened.close();}
      expect(decodeLoaded(await target.store.load(encodeLoadRequest({keys:[],objects:[id],keyRanges:[],objectRanges:[]}))).objects[0]?.[1]).toEqual(new Uint8Array([1]));
      state=decodeLoaded(await target.store.load(empty)).store;
      await target.store.persist(encodeFlush({store:state.id,batches:[{base:state.revision,revision:state.revision+1,putObjects:[],deleteObjects:[id],putKeys:[],deleteKeys:[]}]}));
    }
  } finally {await Promise.all([incoming.close(),active.close(),exporting.close(),receiving.close()]);source.store.close();target.store.close();source.publication.free();target.publication.free();}
});

test("real Wasm import discards an aggregate flush crossing 10000 operations and preserves the old journey", async () => {
  const api=await loadAuthoring(),ref=(key:string)=>({provider:"loop",key});
  const manifest:ProjectManifest={format_version:2,packages:{loop:"loop.json"},product:{id:"loop",entry:{package:"loop",graph:"main"},endings:{done:{}}}};
  const source:PackageSource={id:"test.loop",version:"1.0.0",exports:["main"],graphs:{main:{entry:"start",outcomes:["done"],nodes:{
    start:{type_id:"narrata.passage",data:{body:{unit:ref("body")},choice_points:[{key:"again",options:[{key:"again",label:ref("again"),outcome:{kind:"branch",target:"start"}},{key:"finish",label:ref("finish"),outcome:{kind:"branch",target:"end"}}]}]}},
    end:{type_id:"narrata.return",data:{outcome:"done"}},
  }}}};
  let entropy=1;const minter=api.createMinter({now:()=>1_750_000_000_000,random:b=>{b.fill(entropy++);}});
  const drafts=api.createDrafts();
  let publication;
  try {for(const record of minter.fill(manifest,[["loop",source]]).records)drafts.add(record.bytes);drafts.addOutline({format_version:1,provider:"loop",units:{body:{blocks:["p1"]}}});publication=drafts.publish().publication;}
  finally {minter.free();drafts.free();}
  if(!publication)throw new Error("Expected loop publication");
  const pack=publication.pack(),book=await openBook({pack,execution});
  const first=(await book.inspect()).view;
  if(first.interaction.kind!=="choose")throw new Error("Expected loop choice");
  let cursor=first.cursor,below="",belowCursor="";
  try {
    for(let n=1;n<=2500;n++) {cursor=await book.choose(cursor,first.interaction.choice_point,[first.interaction.options[0]!.id]);if(n===2497){below=await book.exportSave();belowCursor=cursor;}}
    const above=await book.exportSave();
    for(const accepted of [true,false]) {
      const store=await IndexedDbStore.open(`maintenance-aggregate-${crypto.randomUUID()}`);
      const old=await openBook({pack,execution,storage:store});
      const state=decodeLoaded(await store.load(empty)).store;
      // A trusted host may prefetch a complete small library snapshot. Its empty tails
      // prove absence without thousands of point misses, so this reaches the flush guard.
      const range={lower:new Uint8Array(),upper:null,limit:256};
      const inventory=decodeLoaded(await store.load(encodeLoadRequest({keys:[],objects:[],keyRanges:[range],objectRanges:[range]})));
      const ids=inventory.objectRanges.flatMap(([,ids])=>ids);
      const objects=decodeLoaded(await store.load(encodeLoadRequest({keys:[],objects:ids,keyRanges:[],objectRanges:[]}))).objects;
      const reply=encodeLoaded({...inventory,objects});expect(reply.byteLength).toBeLessThan(4096);
      const flushes:Uint8Array[]=[];
      const m=api.createMaintenance({storeId:state.id,storage:{load:async()=>reply,persist:async b=>{flushes.push(b.slice());return store.persist(b);}},limits:{...defaultMaintenanceLimits,bytes:256*1024*1024,work:10_000_000}});
      await m.registerPublication(publication);
      const raw=rawHandle(m),importer:unknown=Reflect.get(raw,"import"),pending:unknown=Reflect.get(raw,"unconfirmed");
      if(typeof importer!=="function" || typeof pending!=="function")throw new Error("Missing real Wasm import boundary");
      let queuedOps=0;
      Reflect.set(raw,"import",(...args:unknown[])=>{
        try {return Reflect.apply(importer,raw,args);}
        catch(error) {const bytes:unknown=Reflect.apply(pending,raw,[]);if(bytes instanceof Uint8Array)queuedOps=decodeFlush(bytes).batches.reduce((sum,b)=>sum+b.putObjects.length+b.putKeys.length+b.deleteKeys.length+b.deleteObjects.length,0);throw error;}
      });
      try {
        const observed=(await roots(m)).find(r=>r.key.namespace===3)!;
        const importing=m.importSave(accepted?below:above,{execution,artifact:book.artifactId,cursor:accepted?belowCursor:cursor,expected_revision:observed.revision,now:100});
        if(accepted) {await importing;expect(flushes).toHaveLength(1);const flush=decodeFlush(flushes[0]!);expect(flush.batches.reduce((sum,b)=>sum+b.putObjects.length+b.putKeys.length+b.deleteKeys.length+b.deleteObjects.length,0)).toBe(9996);}
        else {await expect(importing).rejects.toMatchObject({code:"limit"});expect(queuedOps).toBeGreaterThan(9900);expect(flushes).toHaveLength(0);expect(Reflect.apply(pending,raw,[])).toBeUndefined();}
        const expected=accepted?belowCursor:first.cursor;
        expect((await locateSave(store,execution))?.cursor).toBe(expected);
        const reopened=await openBook({pack,execution,storage:store});
        try {expect(await reopened.choose(expected,first.interaction.choice_point,[first.interaction.options[0]!.id])).not.toBe(expected);}
        finally {await reopened.close();}
      } finally {await Promise.all([old.close(),m.close()]);store.close();}
    }
  } finally {await book.close();publication.free();}
},30_000);
