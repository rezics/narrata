import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { readFileSync } from "node:fs";
import { loadAuthoring, openLibraryMaintenance, PostgresStore, type Maintenance, type Publication, type ObservedRoot } from "../../../web/dist/server/index.js";
import { openBook, locateSave, type Book } from "../../../web/dist/index.js";
import { StoreSuperseded, type CacheStore } from "../../../web/dist/storage.js";
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
