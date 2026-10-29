import { readFileSync, readdirSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { beforeAll, expect, test } from "vitest";
import { IDBFactory } from "fake-indexeddb";
import "fake-indexeddb/auto";
import { loadAuthoring, assessImpact, EvolutionController, defaultComparisonBudget, type Authoring, type AssessmentV1, type ImpactLoadBudget, type PlanV1 } from "@rezics/narrata/server";
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
    const contradictory = await assessImpact({evolution:c.evolution,heads:[head,{...head,artifact:`artifact:${"0".repeat(64)}`}],options:c.options,loadSave:async () => { throw new Error("contradictory heads must not load"); }});
    expect(contradictory.rejected_heads).toBe(2); expect(contradictory.examined).toBe(0);
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

test.each(["x".repeat(20),"私😺\ud800","é\udc00"])("impact jobs charge rejected returned save bytes and stop subsequent oversized loads (%j)",async body => {
  const c = await context("unseen-rule");
  try {
    const heads = ["0","1","2","3"].map(digit => ({commit:`commit:${digit.repeat(64)}`,artifact:c.previous.artifactId}));
    const budgets: ImpactLoadBudget[] = [];
    const report = await assessImpact({evolution:c.evolution,heads,options:c.options,
      loadSave:async (_head,budget) => { budgets.push(budget); return body; },
      limits:{saves:4,totalBytes:100,saveBytes:4},
    });
    expect(budgets).toEqual([{remainingBytes:100,remainingSaveBytes:4}]);
    expect(report.read_bytes).toBe(new TextEncoder().encode(body).byteLength);
    expect(report.oversized_saves).toBe(1); expect(report.skipped.limit).toBe(1);
    expect(report.limit_reasons.input_bytes).toBe(1); expect(report.evaluated_refs).toBe(0);
    expect(report.stopped).toBe("limit"); expect(report.complete).toBe(false);
    expect(JSON.stringify(report)).not.toContain(body);
  } finally { await c.close(); }
});

test.each([5,6])("impact jobs account for remaining job bytes and stop further loads (body bytes=%i)",async size => {
  const c = await context("unseen-rule");
  try {
    const heads = ["0","1","2"].map(digit => ({commit:`commit:${digit.repeat(64)}`,artifact:c.previous.artifactId}));
    const budgets: ImpactLoadBudget[] = [];
    const report = await assessImpact({evolution:c.evolution,heads,options:c.options,
      loadSave:async (_head,budget) => { budgets.push(budget); return "x".repeat(size); },
      limits:{saves:3,totalBytes:10,saveBytes:8},
    });
    expect(budgets).toEqual([{remainingBytes:10,remainingSaveBytes:8},{remainingBytes:10-size,remainingSaveBytes:8}]);
    // A host ignoring its second request's allowance can return more than the limit.
    // The report records those received bytes instead of presenting the quota as enforced.
    expect(report.read_bytes).toBe(size*2); expect(report.examined).toBe(2);
    expect(report.counts.invalid_save).toBe(size === 5 ? 2 : 1);
    expect(report.limit_reasons.job_bytes).toBe(1);
    expect(report.stopped).toBe("limit"); expect(report.complete).toBe(false);
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

test("impact jobs load authorized stored objects without writer capability and skip moved heads",async () => {
  const c = await stored();
  try {
    const exact = await c.evolution.assessSave(c.source,{target:c.previous.artifactId,migrationIds:[]});
    if (exact.kind !== "exact") throw new Error("exact source expected");
    const head = {commit:exact.source.source_commit,artifact:exact.source.artifact_id};
    const before = await c.store.export(); let reads = 0;
    const loadSave = async () => ({kind:"store" as const,execution,storage:{load:async (request: Uint8Array) => { reads++; return c.store.load(request); }}});
    const report = await assessImpact({evolution:c.evolution,heads:[head,head],options:c.options,loadSave});
    expect(report.counts.automatic).toBe(2); expect(report.distinct_counts.automatic).toBe(1); expect(reads).toBeGreaterThan(0);
    expect(report.read_bytes).toBeGreaterThan(0); expect(await c.store.export()).toEqual(before);
    const limited = await assessImpact({evolution:c.evolution,heads:[head],options:c.options,loadSave,limits:{saves:1,totalBytes:8,saveBytes:8}});
    expect(limited.skipped.limit).toBe(1); expect(limited.counts.invalid_save).toBe(0);
    const moved = await assessImpact({evolution:c.evolution,heads:[{...head,commit:`commit:${"0".repeat(64)}`}],options:c.options,loadSave});
    expect(moved.skipped.stale_scan).toBe(1); expect(moved.evaluated_refs).toBe(0);
    expect(await c.store.export()).toEqual(before);
  } finally { await c.close(); }
});

test.each(["save","job"] as const)("impact jobs charge every store reply and enforce cumulative %s bytes",async bound => {
  const c = await stored();
  try {
    const exact = await c.evolution.assessSave(c.source,{target:c.previous.artifactId,migrationIds:[]});
    if (exact.kind !== "exact") throw new Error("exact source expected");
    const head = {commit:exact.source.source_commit,artifact:exact.source.artifact_id};
    const before = await c.store.export();
    const fullSizes: number[] = [];
    const full = await assessImpact({evolution:c.evolution,heads:[head],options:c.options,loadSave:async () => ({kind:"store",execution,
      storage:{load:async request => { const reply = await c.store.load(request); fullSizes.push(reply.byteLength); return reply; }},
    })});
    expect(full.counts.automatic).toBe(1); expect(fullSizes.length).toBeGreaterThan(1);
    expect(full.read_bytes).toBe(fullSizes.reduce((sum,size) => sum+size,0));
    const limit = Math.max(...fullSizes);
    expect(full.read_bytes).toBeGreaterThan(limit);
    const received: number[] = [], budgets: ImpactLoadBudget[] = []; let saves = 0;
    const totalBytes = bound === "job" ? limit : 100_000, saveBytes = bound === "save" ? limit : 100_000;
    const limited = await assessImpact({evolution:c.evolution,heads:[head,{...head,commit:`commit:${"0".repeat(64)}`}],options:c.options,
      limits:{saves:2,totalBytes,saveBytes},loadSave:async () => {
        saves++;
        return {kind:"store",execution,storage:{load:async (request,budget) => {
          budgets.push(budget);
          const reply = await c.store.load(request); received.push(reply.byteLength); return reply;
        }}};
      },
    });
    expect(saves).toBe(1);
    // Each reply fits individually; together they must consume this save's allowance.
    expect(received.every(size => size <= limit)).toBe(true);
    expect(limited.read_bytes).toBe(received.reduce((sum,size) => sum+size,0));
    expect(limited.read_bytes).toBeGreaterThanOrEqual(limit);
    let prior = 0;
    for (const [index,budget] of budgets.entries()) {
      expect(budget).toEqual({remainingBytes:totalBytes-prior,remainingSaveBytes:saveBytes-prior});
      prior += received[index]!;
    }
    expect(limited.limit_reasons[bound === "save" ? "input_bytes" : "job_bytes"]).toBe(1);
    expect(limited.stopped).toBe("limit"); expect(limited.complete).toBe(false);
    expect(limited.evaluated_refs).toBe(0); expect(await c.store.export()).toEqual(before);
  } finally { await c.close(); }
});

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
    const attempts: Uint8Array[] = []; let failures = 2;
    const storage: CacheStore = { load:r => c.store.load(r),persist:async f => {
      attempts.push(Uint8Array.from(f));
      if (failures-- > 0) { if (committed && attempts.length === 1) await c.store.persist(f); throw new Error("connection lost"); }
      return c.store.persist(f);
    } };
    await c.evolution.connectStore(execution,storage);
    const controller = new EvolutionController(c.evolution);
    await controller.assess(c.options);
    await expect(controller.applyAutomatic()).rejects.toThrow("connection lost");
    expect(controller.state.kind).toBe("save_unknown");
    const unknown = controller.state;
    expect(c.evolution.pendingSave).toBe(true);
    await expect(c.evolution.assess(c.options)).rejects.toThrow("unknown outcome");
    await expect(controller.assess({target:c.previous.artifactId,migrationIds:[]})).rejects.toThrow("unknown outcome");
    expect(controller.state).toBe(unknown); expect(attempts).toHaveLength(1);
    await expect(controller.retrySave()).rejects.toThrow("connection lost");
    expect(controller.state).toBe(unknown); expect(c.evolution.pendingSave).toBe(true);
    await expect(controller.assess(c.options)).rejects.toThrow("unknown outcome");
    expect(controller.state).toBe(unknown); expect(attempts[1]).toEqual(attempts[0]);
    const retried = await controller.retrySave();
    expect(retried?.kind).toBe(committed ? "storage_conflict" : "migrated");
    expect(attempts).toHaveLength(3); expect(attempts[2]).toEqual(attempts[0]);
    if (committed) { await c.evolution.reload(); expect((await controller.assess(c.options)).kind).toBe("exact"); }
    // The source checkpoint always remains exactly restorable, even after cursor migration.
    const isolated = api.createEvolution();
    try { await isolated.registerPublication(c.previous); expect((await isolated.assessSave(c.source,{target:c.previous.artifactId,migrationIds:[]})).kind).toBe("exact"); } finally { await isolated.close(); }
  } finally { await c.close(); }
});
