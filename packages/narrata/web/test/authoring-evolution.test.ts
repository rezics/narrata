import { readFileSync, readdirSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { beforeAll, expect, test } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import "fake-indexeddb/auto";
import { loadAuthoring, assessImpact, EvolutionController, defaultComparisonBudget, type Authoring, type AssessmentV1, type PlanV1 } from "@rezics/narrata/server";
import { openBook } from "@rezics/narrata/reader";
import { IndexedDbStore, type CacheStore } from "@rezics/narrata/storage";
import { evolutionJourney, type EvolutionVector } from "./authoring-evolution-journey.js";
import { execution } from "./fixture.js";

export const corpusRoot = resolve(import.meta.dirname,"../../../../fixtures/compat/nodes-evolution-v1");
export function vector(name: string): EvolutionVector {
  const root = resolve(corpusRoot,name);
  const bytes = (name: string) => new Uint8Array(readFileSync(resolve(root,name)));
  const optional = (name: string) => existsSync(resolve(root,name));
  return {
    name, source: [...bytes("source.narpack")], target: [...bytes("target.narpack")], save: Buffer.from(bytes("source.checkpoint")).toString("hex"),
    expected: JSON.parse(readFileSync(resolve(root,"assessment.json"),"utf8")) as AssessmentV1,
    ...(optional("declarations.json") ? { declarations: readFileSync(resolve(root,"declarations.json"),"utf8") } : {}),
    ...(optional("descriptor.cbor") ? { descriptor: [...bytes("descriptor.cbor")] } : {}),
    ...(optional("apply.json") ? { applied: JSON.parse(readFileSync(resolve(root,"apply.json"),"utf8")) } : {}),
    ...(optional("target.checkpoint") ? { targetSave: Buffer.from(bytes("target.checkpoint")).toString("hex") } : {}),
    ...(optional("ancestor.narpack") ? { ancestor: [...bytes("ancestor.narpack")] } : {}),
  };
}
let api: Authoring;
beforeAll(async () => { api = await loadAuthoring(); });
for (const name of readdirSync(corpusRoot).filter(name => existsSync(resolve(corpusRoot,name,"target.narpack")))) {
  test(`public evolution corpus: ${name}`,async () => { await evolutionJourney(api,vector(name)); });
}
function plan(result: AssessmentV1): PlanV1 {
  if (result.kind !== "automatic" && result.kind !== "needs_confirmation") throw new Error(`No plan: ${result.kind}`);
  return result.plan;
}
async function context(name: string) {
  const v = vector(name);
  const previous = api.publicationFromPack(Uint8Array.from(v.source));
  const candidate = api.publicationFromPack(Uint8Array.from(v.target));
  const descriptor = api.planEvolution(previous,candidate,v.declarations);
  const target = candidate.withEvolution(previous,descriptor);
  descriptor.free();
  const evolution = api.createEvolution();
  const id = await evolution.registerEvolution(previous,target);
  return { v,previous,target,evolution,options: { target: target.artifactId,migrationIds: [id] }, async close() { await evolution.close(); previous.free(); target.free(); } };
}

test("confirmations bind target, budget, report and source; legacy saves still restore exactly",async () => {
  const c = await context("target-rejoins");
  try {
    await c.evolution.restoreSave(c.v.save);
    const controller = new EvolutionController(c.evolution);
    const a = await controller.assess(c.options);
    expect(a.kind).toBe("needs_confirmation");
    await expect(controller.applyAutomatic()).rejects.toThrow("explicitly confirm");
    controller.continueOld();
    expect(controller.state.kind).toBe("continued_old");
    const p = plan(await controller.assess(c.options));
    expect((await c.evolution.apply(p,{...defaultComparisonBudget,transitions: 0})).kind).toBe("plan_changed");
    await c.evolution.assess({target: c.previous.artifactId,migrationIds: []});
    expect((await c.evolution.apply(p)).kind).toBe("plan_changed");
    await controller.assess(c.options);
    expect((await controller.confirm()).kind).toBe("migrated");
    expect(controller.state.kind).toBe("applied_snapshot");
    const old = readFileSync(resolve(corpusRoot,"old-save-continue/legacy.json"),"utf8");
    expect((await c.evolution.assessSave(old,{target:c.previous.artifactId,migrationIds:[]})).kind).toBe("exact");
  } finally { await c.close(); }
});

test("explicit multi-hop paths and subsequent migrations preserve every committed segment",async () => {
  const v = vector("republished-after-migration");
  const ancestor = api.publicationFromPack(Uint8Array.from(v.ancestor ?? []));
  let middle = api.publicationFromPack(Uint8Array.from(v.source));
  let target = api.publicationFromPack(Uint8Array.from(v.target));
  const first = api.planEvolution(ancestor,middle), second = api.planEvolution(middle,target);
  middle = middle.withEvolution(ancestor,first); target = target.withEvolution(middle,second);
  const e = api.createEvolution();
  try {
    const a = await e.registerEvolution(ancestor,middle), b = await e.registerEvolution(middle,target);
    const original = readFileSync(resolve(corpusRoot,"no-declarations/source.checkpoint")).toString("hex");
    await e.restoreSave(original);
    const p = plan(await e.assess({target:target.artifactId,migrationIds:[a,b]}));
    expect(p.migration_ids).toEqual([a,b]);
    expect((await e.apply(p)).kind).toBe("migrated");
    const migrated = await e.exportSave();
    await e.restoreSave(migrated);
    expect((await e.assess({target:target.artifactId,migrationIds:[]})).kind).toBe("exact");
  } finally { await e.close(); first.free(); second.free(); ancestor.free(); middle.free(); target.free(); }
});

test("impact jobs bound authorization enumeration, reject lying heads and expose only aggregate counts",async () => {
  const c = await context("unseen-rule");
  try {
    const source = c.v.expected.kind === "automatic" ? c.v.expected.plan.source_commit : "";
    const head = {commit:source,artifact:c.previous.artifactId};
    const heads = [head,{...head,commit:`commit:${"0".repeat(64)}`},head];
    let pulls = 0, loads = 0;
    function* authorized() { for (const head of heads) { pulls++; yield head; } }
    const report = await assessImpact({ evolution:c.evolution,heads:authorized(),options:c.options,loadSave:async () => { loads++; return c.v.save; },limits:{saves:2,totalBytes:100_000,saveBytes:50_000,scanHeads:2} });
    expect(pulls).toBe(2); expect(loads).toBe(2);
    expect(report.counts.automatic).toBe(1); expect(report.rejected_heads).toBe(1); expect(report.stopped).toBe("limit");
    expect(JSON.stringify(report)).not.toContain(source);
    const missing = await assessImpact({evolution:c.evolution,heads:[head],options:c.options,loadSave:async () => undefined});
    expect(missing.counts.needs_objects).toBe(1);
    expect(missing.complete).toBe(true);
    const absent = api.createEvolution();
    try { expect((await absent.assessSave(c.v.save,c.options)).kind).toBe("needs_objects"); } finally { await absent.close(); }
    expect((await c.evolution.assessSave("malformed",c.options)).kind).toBe("invalid_save");
  } finally { await c.close(); }
});

test("impact sampling is deterministic, counts duplicate Refs once per commit, and marks stale scans and limits",async () => {
  const c = await context("unseen-rule");
  try {
    const source = plan(c.v.expected).source_commit;
    const head = {commit:source,artifact:c.previous.artifactId};
    let loads = 0;
    const both = await assessImpact({evolution:c.evolution,heads:[head,head],options:c.options,loadSave:async () => { loads++; return c.v.save; }});
    expect(loads).toBe(1); expect(both.counts.automatic).toBe(2); expect(both.distinct_counts.automatic).toBe(1);
    const others = [head,...["0","1","2"].map(digit => ({...head,commit:`commit:${digit.repeat(64)}`}))];
    const selected: string[][] = [];
    for (const heads of [others,[...others].reverse()]) {
      const seen: string[] = []; selected.push(seen);
      const r = await assessImpact({evolution:c.evolution,heads,options:c.options,loadSave:async h => { seen.push(h.commit); return undefined; },limits:{saves:2,totalBytes:100_000,saveBytes:50_000}});
      expect(r.sample.distinct_population).toBe(4); expect(r.sample.selected).toBe(2); expect(r.complete).toBe(false);
    }
    expect(selected[0]).toEqual(selected[1]);
    const stale = await assessImpact({evolution:c.evolution,heads:[head],options:c.options,loadSave:async () => ({kind:"stale_scan"}),scan:{revision_start:"1",revision_end:"2"}});
    expect(stale.skipped.stale_scan).toBe(1); expect(stale.evaluated_refs).toBe(0); expect(stale.complete).toBe(false);
    const oversized = await assessImpact({evolution:c.evolution,heads:[head],options:c.options,loadSave:async () => c.v.save,limits:{saves:1,totalBytes:16,saveBytes:8}});
    expect(oversized.skipped.limit).toBe(1); expect(oversized.counts.invalid_save).toBe(0);
    const limitedPath = await assessImpact({evolution:c.evolution,heads:[head],options:{...c.options,budget:{...defaultComparisonBudget,transitions:0}},loadSave:async () => c.v.save});
    expect(limitedPath.confirmation.unverified).toBe(1); expect(limitedPath.unverified_reasons.limit).toBe(1);
    const abort = new AbortController(); abort.abort();
    const cancelled = await assessImpact({evolution:c.evolution,heads:[head],options:c.options,loadSave:async () => { throw new Error("must not load"); },signal:abort.signal});
    expect(cancelled.stopped).toBe("cancelled"); expect(cancelled.scan.enumerated).toBe(0);
  } finally { await c.close(); }
});

async function stored() {
  const c = await context("content-reference");
  const factory = new IDBFactory();
  const name = `evolution-${crypto.randomUUID()}`;
  const store = await IndexedDbStore.open(name,factory);
  const book = await openBook({pack:Uint8Array.from(c.v.source),execution,storage:store});
  const source = await book.exportSave(); await book.close();
  return { ...c, store, source, async close() { await c.close(); store.close(); await IndexedDbStore.delete(name,factory); } };
}

test("the controller reports upgraded only after persistence and stale cursors return conflicts",async () => {
  const c = await stored();
  try {
    let writes = 0;
    const storage: CacheStore = { load:r => c.store.load(r),persist:async f => { writes++; return c.store.persist(f); } };
    await c.evolution.connectStore(execution,storage);
    const controller = new EvolutionController(c.evolution);
    await controller.assess(c.options); expect(writes).toBe(0);
    const rival = await openBook({pack:Uint8Array.from(c.v.source),execution,storage:c.store});
    const view = (await rival.inspect()).view;
    if (view.interaction.kind !== "choose") throw new Error("choice expected");
    await rival.choose(view.cursor,view.interaction.choice_point,[view.interaction.options[0]!.id]);
    await rival.close();
    expect((await controller.applyAutomatic()).kind).toBe("conflict");
    await controller.assess(c.options);
    expect((await controller.applyAutomatic()).kind).toBe("migrated");
    expect(controller.state.kind).toBe("upgraded"); expect(writes).toBe(1);
    const reopened = await openBook({pack:c.target.pack(),execution,storage:c.store});
    expect(reopened.artifactId).toBe(c.target.artifactId); await reopened.close();
  } finally { await c.close(); }
});

test.each([false,true])("unknown writes retain old progress and recover without rerunning migration (committed=%s)",async committed => {
  const c = await stored();
  try {
    const attempts: Uint8Array[] = []; let fail = true;
    const storage: CacheStore = { load:r => c.store.load(r),persist:async f => {
      attempts.push(Uint8Array.from(f));
      if (fail) { fail = false; if (committed) await c.store.persist(f); throw new Error("connection lost"); }
      return c.store.persist(f);
    } };
    await c.evolution.connectStore(execution,storage);
    const controller = new EvolutionController(c.evolution);
    await controller.assess(c.options);
    await expect(controller.applyAutomatic()).rejects.toThrow("connection lost");
    expect(controller.state.kind).toBe("save_unknown");
    await expect(c.evolution.assess(c.options)).rejects.toThrow("unknown outcome");
    const retried = await controller.retrySave();
    expect(retried?.kind).toBe(committed ? "storage_conflict" : "migrated");
    expect(attempts[1]).toEqual(attempts[0]);
    if (committed) { await c.evolution.reload(); expect((await controller.assess(c.options)).kind).toBe("exact"); }
    // The source checkpoint always remains exactly restorable, even after cursor migration.
    const isolated = api.createEvolution();
    try { await isolated.registerPublication(c.previous); expect((await isolated.assessSave(c.source,{target:c.previous.artifactId,migrationIds:[]})).kind).toBe("exact"); } finally { await isolated.close(); }
  } finally { await c.close(); }
});
