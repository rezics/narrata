import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { loadAuthoring, openLibraryMaintenance, PostgresStore, LibraryQuotaExceeded, type Maintenance, type Publication, type ObservedRoot } from "../../../web/dist/server/index.js";
import { openBook, locateSave, type Book } from "../../../web/dist/index.js";
import { StoreSuperseded, WriteRejected, decodeLoaded, encodeLoadRequest, type CacheStore } from "../../../web/dist/storage.js";
import { ddl, generousQuota, pgliteDatabase, postgresDatabase, sqlFixture, type TestDatabase } from "./server-fixture.js";

const pack = new Uint8Array(readFileSync(new URL("../../../../../products/gamebook-demo/story.narpack", import.meta.url)));
const execution = "execution:0190f2a0000070008000000000000001";
const second = "execution:0190f2a0000070008000000000000002";
const third = "execution:0190f2a0000070008000000000000003";
const url = process.env.NARRATA_POSTGRES_URL;
async function roots(m: Maintenance): Promise<ObservedRoot[]> {
  const result = await m.run({kind:"roots",after:null,limit:1024});
  if (result.kind !== "roots" || result.more) throw new Error("Unexpected root result");
  return result.items;
}
async function choose(book: Book): Promise<void> {
  const view = (await book.inspect()).view;
  if (view.interaction.kind !== "choose") throw new Error("Expected choice");
  await book.choose(view.cursor,view.interaction.choice_point,[view.interaction.options.find(o=>o.enabled)!.id]);
}
for (const real of [false,true]) describe.skipIf(real && !url)(`public maintenance: ${real ? "PostgreSQL" : "PGlite"}`, () => {
  let db: TestDatabase, publication: Publication;
  beforeAll(async () => {
    db = real ? await postgresDatabase(url!) : await pgliteDatabase();await db.exec(ddl);
    publication = (await loadAuthoring()).publicationFromPack(pack);
  });
  afterAll(async () => {publication?.free();await db?.close();});
  test("wrong-kind stored references refuse maintenance without reclaiming any dependencies", async () => {
    const f=await sqlFixture(db),store=new PostgresStore(db,f.id,generousQuota),book=await openBook({pack,execution,storage:store});
    const m=await openLibraryMaintenance(store);await m.registerPublication(publication);
    try {
      const cursor=(await book.inspect()).view.cursor,id=new Uint8Array(Buffer.from(cursor.split(":").at(-1)!,"hex"));
      const stored=decodeLoaded(await store.load(encodeLoadRequest({keys:[],objects:[id],keyRanges:[],objectRanges:[]}))).objects[0]?.[1];
      if(!stored)throw new Error("Missing current commit");
      const bad=Buffer.from(stored),state=bad.indexOf(Buffer.from([3,0x58,32]),56);
      if(state<56)throw new Error("Missing commit state field");
      bad.set(id,state+3);bad.set(createHash("sha256").update(bad.subarray(56)).digest(),24);
      const digest=createHash("sha256").update("narrata-object\0").update(bad.subarray(10,14)).update(bad.subarray(56)).digest();
      await db.query("INSERT INTO narrata_objects (library_id,digest,bytes) VALUES ($1,$2,$3)",[f.id,digest,bad]);
      const before=await store.usage(),integrity=await m.run({kind:"integrity"});
      expect(integrity.kind).toBe("integrity");if(integrity.kind!=="integrity")throw new Error("Expected integrity result");
      expect(integrity.issues.some(i=>i.object===`object:${digest.toString("hex")}`)).toBe(true);
      await expect(m.run({kind:"collect",now:1000,grace_seconds:0,dry_run:false})).rejects.toMatchObject({code:"corrupt_save"});
      expect(await store.usage()).toEqual(before);
      const reopened=await openBook({pack,execution,storage:store});try{expect((await reopened.inspect()).view.cursor).toBe(cursor);await choose(reopened);}finally{await reopened.close();}
    } finally {await Promise.all([book.close(),m.close()]);await f.dispose();}
  });

  test("a changed authorized quota after unknown import distinguishes committed loss from an uncommitted attempt", async () => {
    for(const committed of [false,true]) {
      const f=await sqlFixture(db),store=new PostgresStore(db,f.id,generousQuota),book=await openBook({pack,execution,storage:store});
      const strict=new PostgresStore(db,f.id,{bytes:0,objects:0,keys:0});
      let attempted=false;const writes:Uint8Array[]=[];
      const m=(await loadAuthoring()).createMaintenance({storeId:f.id,storage:{load:r=>store.load(r),persist:async r=>{
        writes.push(r.slice());
        if(!attempted){attempted=true;if(committed)await store.persist(r);throw new Error("unknown import response");}
        try{return await strict.persist(r);}catch(error){if(error instanceof LibraryQuotaExceeded)throw new WriteRejected("quota");throw error;}
      }}});await m.registerPublication(publication);
      try {
        await choose(book);const cursor=(await book.inspect()).view.cursor,artifact=book.artifactId,save=await book.exportSave();
        await expect(m.importSave(save,{execution:second,artifact,cursor,expected_revision:null,now:100})).rejects.toThrow("unknown import response");
        await expect(m.retryPending()).rejects.toBeInstanceOf(committed?StoreSuperseded:WriteRejected);
        expect(writes[1]).toEqual(writes[0]);
        expect((await locateSave(store,execution))?.cursor).toBe(cursor);
        expect((await locateSave(store,second))?.cursor??null).toBe(committed?cursor:null);
        const reopened=await openBook({pack,execution,storage:store});try{expect((await reopened.inspect()).view.cursor).toBe(cursor);}finally{await reopened.close();}
      } finally {await Promise.all([book.close(),m.close()]);await f.dispose();}
    }
  });
  test("corrupt destination commit and state copies reject import before moving the healthy active root", async () => {
    const f=await sqlFixture(db),store=new PostgresStore(db,f.id,generousQuota);
    const active=await openBook({pack,execution,storage:store}),incoming=await openBook({pack,execution:second});
    const m=await openLibraryMaintenance(store);await m.registerPublication(publication);
    try {
      const a=(await active.inspect()).view;await choose(incoming);await choose(incoming);
      const b=(await incoming.inspect()).view, save=await incoming.exportSave();
      const bytes=Buffer.from(save,"hex");
      let at=16+Number(bytes.readBigUInt64BE(8));const count=Number(bytes.readBigUInt64BE(at));at+=8;
      const states:Uint8Array[]=[];
      for(let i=0;i<count;i++) {const id=new Uint8Array(bytes.subarray(at,at+32)),kind=bytes.readUInt16BE(at+32),size=Number(bytes.readBigUInt64BE(at+36));if(kind===0x0111)states.push(id);at+=44+size;}
      const absent=decodeLoaded(await store.load(encodeLoadRequest({keys:[],objects:states,keyRanges:[],objectRanges:[]}))).objects.find(([,v])=>v===null)?.[0];
      if(!absent)throw new Error("Expected a new incoming state");
      const root=new Uint8Array(Buffer.from(b.cursor.split(":").at(-1)!,"hex"));
      for(const id of [root,absent]) {
        await db.query("INSERT INTO narrata_objects (library_id,digest,bytes) VALUES ($1,$2,$3)",[f.id,id,new Uint8Array([1])]);
        const selected=(await roots(m)).find(r=>r.key.namespace===3)!;
        const before=await store.usage();
        await expect(m.importSave(save,{execution,artifact:incoming.artifactId,cursor:b.cursor,expected_revision:selected.revision,now:100})).rejects.toMatchObject({code:"corrupt_save"});
        await active.reload();
        await expect(active.restore(save)).rejects.toThrow();
        expect(await store.usage()).toEqual(before);
        const reopened=await openBook({pack,execution,storage:store});
        try {expect((await reopened.inspect()).view).toMatchObject({cursor:a.cursor,shared:a.shared,frames:a.frames,interaction:a.interaction});}
        finally {await reopened.close();}
        await db.query("DELETE FROM narrata_objects WHERE library_id=$1 AND digest=$2",[f.id,id]);
      }
    } finally {await Promise.all([active.close(),incoming.close(),m.close()]);await f.dispose();}
  });

  test("a held maintenance mutation cannot revive an actually removed SQL library", async () => {
    const f=await sqlFixture(db),store=new PostgresStore(db,f.id,generousQuota);
    const book=await openBook({pack,execution,storage:store});
    let enter!:()=>void,release!:()=>void;
    const started=new Promise<void>(r=>{enter=r;}),waiting=new Promise<void>(r=>{release=r;});
    const m=(await loadAuthoring()).createMaintenance({storeId:f.id,storage:{load:r=>store.load(r),persist:async r=>{enter();await waiting;return store.persist(r);}}});
    await m.registerPublication(publication);
    try {
      const pending=m.run({kind:"delete_roots",roots:await roots(m)});
      await started;const closing=m.close();await book.close();
      await f.clear();release();await expect(pending).rejects.toThrow();await closing;
      const replacement=new PostgresStore(db,f.id,generousQuota);
      expect(await locateSave(replacement,execution)).toBeNull();
      expect((await replacement.usage()).used).toEqual({bytes:"0",objects:"0",keys:"0"});
    } finally {release();await Promise.all([book.close(),m.close()]);await f.dispose();}
  });
  test("cold import/shallow restore/usage, quota rollback, selected root cleanup and retry", async () => {
    const f = await sqlFixture(db), target = await sqlFixture(db);
    const store = new PostgresStore(db,f.id,generousQuota), destination = new PostgresStore(db,target.id,generousQuota);
    const book = await openBook({pack,execution,storage:store});
    const source = await openLibraryMaintenance(store), receiving = await openLibraryMaintenance(destination);
    await source.registerPublication(publication);await receiving.registerPublication(publication);
    try {
      await choose(book); await choose(book);
      const cursor = (await book.inspect()).view.cursor, artifact = book.artifactId;
      const bytes = await source.exportSave({execution,artifact,cursor,ancestors:0});
      expect(await source.inspectSave(bytes)).toEqual({artifact,cursor});
      const tooSmall = new PostgresStore(db,target.id,{bytes:0,objects:0,keys:0});
      const rejected = await openLibraryMaintenance(tooSmall);
      await rejected.registerPublication(publication);
      try { await expect(rejected.importSave(bytes,{execution,artifact,cursor,expected_revision:null,now:100})).rejects.toMatchObject({reason:"quota"}); }
      finally {await rejected.close();}
      expect((await destination.usage()).used).toEqual({bytes:"0",objects:"0",keys:"0"});
      await receiving.importSave(bytes,{execution,artifact,cursor,expected_revision:null,now:100});
      const reopened = await openBook({pack,execution,storage:destination});
      try {expect((await reopened.inspect()).view.cursor).toBe(cursor);await choose(reopened);}
      finally {await reopened.close();}
      const usage = await destination.usage();
      expect(BigInt(usage.used.bytes)).toBeGreaterThan(0n);expect(usage.quota).toEqual(generousQuota);
      await receiving.run({kind:"delete_roots",roots:await roots(receiving)});
      const afterDeletion = await destination.usage();
      expect(BigInt(afterDeletion.used.keys)).toBeLessThan(BigInt(usage.used.keys));
      const dry = await receiving.run({kind:"collect",now:1000,grace_seconds:10,dry_run:true});
      expect(dry.kind).toBe("collected");expect(await destination.usage()).toEqual(afterDeletion);
      await receiving.run({kind:"collect",now:1000,grace_seconds:10,dry_run:false});
      expect(BigInt((await destination.usage()).used.objects)).toBe(0n);
      await receiving.importSave(bytes,{execution,artifact,cursor,expected_revision:null,now:1001});
      expect((await locateSave(destination,execution))?.cursor).toBe(cursor);
      const before = await destination.usage();
      target.failNextPersist();
      // The fixture fault adapter rolls back all batches before any root is published.
      const faulty = (await loadAuthoring()).createMaintenance({storage:target.store,storeId:target.id});
      await faulty.registerPublication(publication);
      try {await expect(faulty.importSave(bytes,{execution:second,artifact,cursor,expected_revision:null,now:1001})).rejects.toThrow();}
      finally {await faulty.close();}
      expect((await destination.usage()).used).toEqual(before.used);
      expect((await locateSave(destination,execution))?.cursor).toBe(cursor);
    } finally {await Promise.all([book.close(),source.close(),receiving.close()]);await f.dispose();await target.dispose();}
  });

  test.skipIf(!real)("GC loses its atomic flush to a separate connection importing a previously unrooted head", async () => {
    const f = await sqlFixture(db);
    const store = new PostgresStore(db,f.id,generousQuota);
    const book = await openBook({pack,execution,storage:store});
    const authoring = await loadAuthoring();
    let beforePersist: (()=>Promise<void>) | undefined;
    const transport: CacheStore = {load:r=>store.load(r),persist:async bytes=>{const hook=beforePersist;beforePersist=undefined;await hook?.();return store.persist(bytes);}};
    const collector = authoring.createMaintenance({storage:transport,storeId:f.id}), importer = await openLibraryMaintenance(store);
    await collector.registerPublication(publication);await importer.registerPublication(publication);
    try {
      await choose(book);const cursor=(await book.inspect()).view.cursor,artifact=book.artifactId;
      const bytes=await collector.exportSave({execution,artifact,cursor});
      await collector.run({kind:"delete_roots",roots:await roots(collector)});
      beforePersist=async()=>{await importer.importSave(bytes,{execution:second,artifact,cursor,expected_revision:null,now:100});};
      await expect(collector.run({kind:"collect",now:1000,grace_seconds:0,dry_run:false})).rejects.toBeInstanceOf(StoreSuperseded);
      expect((await locateSave(store,second))?.cursor).toBe(cursor);
      expect(await collector.run({kind:"integrity"})).toEqual({kind:"integrity",issues:[]});
      const reopened=await openBook({pack,execution:second,storage:store});try{await choose(reopened);}finally{await reopened.close();}
    } finally {await Promise.all([book.close(),collector.close(),importer.close()]);await f.dispose();}
  });

  test.skipIf(!real)("GC loses to a separate connection deleting selected roots, then a fresh sweep may reclaim them", async () => {
    const f=await sqlFixture(db),store=new PostgresStore(db,f.id,generousQuota);
    const firstBook=await openBook({pack,execution,storage:store}),other=await openBook({pack,execution:second,storage:store});
    let hook:(()=>Promise<void>)|undefined;
    const collector=(await loadAuthoring()).createMaintenance({storeId:f.id,storage:{load:r=>store.load(r),persist:async r=>{const before=hook;hook=undefined;await before?.();return store.persist(r);}}});
    const deleter=await openLibraryMaintenance(store);await collector.registerPublication(publication);await deleter.registerPublication(publication);
    try {
      // Creating the other execution moved the library revision before this deliberate race.
      await firstBook.reload();await choose(firstBook);await collector.run({kind:"delete_roots",roots:(await roots(collector)).filter(r=>r.key.owner===execution.slice(10))});
      let afterDeletion:Awaited<ReturnType<PostgresStore["usage"]>>|undefined;
      hook=async()=>{await deleter.run({kind:"delete_roots",roots:await roots(deleter)});afterDeletion=await store.usage();};
      await expect(collector.run({kind:"collect",now:1000,grace_seconds:0,dry_run:false})).rejects.toBeInstanceOf(StoreSuperseded);
      expect(afterDeletion).toBeDefined();expect(await store.usage()).toEqual(afterDeletion);
      expect(await roots(collector)).toEqual([]);
      await collector.run({kind:"collect",now:1000,grace_seconds:0,dry_run:false});
      expect((await store.usage()).used.objects).toBe("0");
    } finally {await Promise.all([firstBook.close(),other.close(),collector.close(),deleter.close()]);await f.dispose();}
  });

  test("one unchanged quota preserves old progress, then permits retry after selected-root cleanup", async () => {
    const f = await sqlFixture(db);
    const store = new PostgresStore(db,f.id,generousQuota);
    const book = await openBook({pack,execution,storage:store});
    const exporting = await openLibraryMaintenance(store);
    await exporting.registerPublication(publication);
    let limited: Maintenance | undefined;
    try {
      await choose(book);
      const cursor = (await book.inspect()).view.cursor, artifact = book.artifactId;
      const bytes = await exporting.exportSave({execution,artifact,cursor});
      // Seed the checked bundle manifest as well as ordinary session objects. Subsequent
      // imports then add only their selected active root, rather than first-import indexes.
      await exporting.importSave(bytes,{execution:second,artifact,cursor,expected_revision:null,now:100});
      const used = await store.usage();
      const constrained = new PostgresStore(db,f.id,{...generousQuota,keys:Number(used.used.keys)});
      limited = await openLibraryMaintenance(constrained);
      await limited.registerPublication(publication);
      const selection = {execution:third,artifact,cursor,expected_revision:null,now:100};
      // Objects already exist. The additional active root alone exceeds this fixed quota.
      await expect(limited.importSave(bytes,selection)).rejects.toMatchObject({reason:"quota"});
      expect(await locateSave(constrained,third)).toBeNull();
      const old = await openBook({pack,execution,storage:constrained});
      try {expect((await old.inspect()).view.cursor).toBe(cursor);}
      finally {await old.close();}
      await book.close();
      await limited.run({kind:"delete_roots",roots:await roots(limited)});
      const beforeDryRun = await constrained.usage();
      await limited.run({kind:"collect",now:1000,grace_seconds:0,dry_run:true});
      expect(await constrained.usage()).toEqual(beforeDryRun);
      await limited.run({kind:"collect",now:1000,grace_seconds:0,dry_run:false});
      await limited.importSave(bytes,{...selection,now:1001});
      expect(await locateSave(constrained,execution)).toBeNull();
      const restored = await openBook({pack,execution:third,storage:constrained});
      try {
        expect((await restored.inspect()).view.cursor).toBe(cursor);
        expect(await restored.exportSave()).toBe(Buffer.from(bytes).toString("hex"));
      } finally {await restored.close();}
      expect((await constrained.usage()).quota.keys).toBe(Number(used.used.keys));
    } finally {await Promise.all([book.close(),exporting.close(),limited?.close()]);await f.dispose();}
  });
});
