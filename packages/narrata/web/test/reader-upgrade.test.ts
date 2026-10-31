import "fake-indexeddb/auto";
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test, vi } from "vitest";
import { AuthenticatedLibrary, createOnlineSession, openBook, WriteRejected } from "@rezics/narrata/reader";
import { decodeLoaded, encodeLoadRequest, IndexedDbStore, type CacheStore } from "@rezics/narrata/storage";
import { loadAuthoring } from "@rezics/narrata/server";
import { execution, splitPack } from "./fixture.js";

async function setup(name: string) {
  const root = resolve(import.meta.dirname, "../../../../fixtures/compat/nodes-evolution-v1", name);
  const read = (file: string) => new Uint8Array(readFileSync(resolve(root, file)));
  const api = await loadAuthoring();
  const previous = api.publicationFromPack(read("source.narpack"));
  const candidate = api.publicationFromPack(read("target.narpack"));
  const declarations = existsSync(resolve(root, "declarations.json")) ? readFileSync(resolve(root, "declarations.json"), "utf8") : undefined;
  const descriptor = api.planEvolution(previous, candidate, declarations);
  const target = descriptor.report.kind === "exact" ? candidate : candidate.withEvolution(previous, descriptor);
  const evolution = api.createEvolution();
  const migrationIds = descriptor.report.kind === "exact" ? [] : [await evolution.registerEvolution(previous, target)];
  if (!migrationIds.length) { await evolution.registerPublication(previous); await evolution.registerPublication(target); }
  const store = await IndexedDbStore.open(`reader-update-${crypto.randomUUID()}`);
  const seed = await openBook({ pack: previous.pack(), execution, storage: store });
  let save = Buffer.from(read("source.checkpoint")).toString("hex");
  if (name === "shallow-history") {
    // The frozen shallow vector intentionally lacks readable page context. Build a
    // readable shallow ending via the checked exporter instead of bypassing imports.
    const complete = readFileSync(resolve(root, "../target-rejoins/source.checkpoint"));
    await evolution.restoreSave(complete.toString("hex"));
    save = await evolution.exportShallow(1);
  }
  await seed.restore(save); await seed.close();
  let fault: "none" | "lost" | "quota" | "uncommitted" = "none";
  let unavailableArtifact = false;
  let pause: Promise<void> | undefined;
  const writes: Uint8Array[] = [];
  const transport: CacheStore = { load: request => store.load(request), persist: async bytes => {
    writes.push(bytes.slice()); const mode = fault; fault = "none";
    if (mode === "quota") throw new WriteRejected("quota");
    if (mode === "uncommitted") throw new Error("reply lost");
    const reply = await store.persist(bytes);
    await pause;
    if (mode === "lost") throw new Error("reply lost");
    return reply;
  } };
  const state = decodeLoaded(await store.load(encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] }))).store;
  const library = new AuthenticatedLibrary({ store: transport, storeId: state.id, release: () => undefined, clearContent: () => undefined, remove: async () => undefined });
  await evolution.connectStore(execution, library);
  const artifacts = new Map([previous, target].map(publication => [publication.artifactId, splitPack(publication.pack())]));
  const currentArtifact = vi.fn(async () => target.artifactId);
  const create = () => createOnlineSession({ library, execution, currentArtifact, loadArtifact: async artifact => {
    if (unavailableArtifact) throw new Error("Publication offline");
    const parts = artifacts.get(artifact); if (!parts) throw new Error("Missing exact artifact");
    return { manifest: parts.manifest, names: parts.names, fetchChunk: async id => {
      const bytes = parts.objects.get(id); if (!bytes) throw new Error("Wrong artifact loader"); return bytes;
    } };
  } });
  const session = create(); await session.start();
  return { session, store, library, evolution, previous, target, currentArtifact, create, writes, setFault: (value: typeof fault) => { fault = value; },
    setUnavailable: (value: boolean) => { unavailableArtifact = value; }, setPause: (value: Promise<void>) => { pause = value; },
    options: { target: target.artifactId, migrationIds },
    async close() { await evolution.close(); await session.close(); store.close(); previous.free(); target.free(); if (candidate !== target) candidate.free(); descriptor.free(); } };
}

test.each(["unseen-rule", "target-rejoins", "deleted-current", "shallow-history"])("reader reviews and confirms %s before durable application, historical checkout and refresh", async name => {
  const f = await setup(name);
  try {
    const before = f.session.getSnapshot().progress!;
    const writes = f.writes.length;
    const assessment = await f.session.assessUpdate(f.evolution, f.options);
    expect(f.writes).toHaveLength(writes); expect(f.session.getSnapshot().progress).toBe(before);
    expect(["automatic", "needs_confirmation"]).toContain(assessment.kind);
    await f.session.continueOld(); await expect(f.session.applyUpdate(true)).rejects.toThrow("Review");
    expect((await f.session.inspect()).view).toEqual(before.view);
    await f.session.assessUpdate(f.evolution, f.options);
    const applied = await f.session.applyUpdate(true); expect(applied.kind).toBe("migrated");
    const updated = f.session.getSnapshot(); expect(updated.status).toBe("saved");
    expect(updated.progress?.view.artifact_id).toBe(f.target.artifactId);
    await f.session.reload(); expect(f.session.getSnapshot().progress).toEqual(updated.progress);
    const old = updated.progress!.view.history.find(row => row.id === before.view.cursor)!;
    expect(old).toBeDefined(); await f.session.checkout(old.id);
    expect(f.session.getSnapshot().progress?.view.artifact_id).toBe(f.previous.artifactId);
    expect((await f.session.inspect()).page).toEqual(before.page);
    const interaction = (await f.session.inspect()).view.interaction;
    if (interaction.kind === "choose") {
      const option = interaction.options.find(option => option.enabled)!;
      await f.session.choose(old.id, interaction.choice_point, [option.id]);
      expect(f.session.getSnapshot().progress?.view.artifact_id).toBe(f.previous.artifactId);
    }
    expect(f.currentArtifact).not.toHaveBeenCalled();
  } finally { await f.close(); }
});

test("an acknowledged migration whose artifact is offline stays retryable without losing the old screen", async () => {
  const f = await setup("unseen-rule");
  try {
    const before = f.session.getSnapshot().progress;
    await f.session.assessUpdate(f.evolution, f.options); f.setFault("lost");
    await expect(f.session.applyUpdate()).rejects.toThrow("reply lost");
    f.setUnavailable(true); await expect(f.session.retrySave()).rejects.toThrow("Publication offline");
    expect(f.session.getSnapshot()).toMatchObject({ status: "unknown", progress: before });
    f.setUnavailable(false); await f.session.retrySave();
    expect(f.session.getSnapshot()).toMatchObject({ status: "saved", progress: { view: { artifact_id: f.target.artifactId } } });
  } finally { await f.close(); }
});

test("closing while migration persistence replies are delayed never publishes its old-account result", async () => {
  const f = await setup("unseen-rule");
  let release!: () => void;
  const pause = new Promise<void>(done => { release = done; });
  try {
    await f.session.assessUpdate(f.evolution, f.options); f.setPause(pause);
    const applying = f.session.applyUpdate();
    const rejected = expect(applying).rejects.toThrow("closed");
    await vi.waitFor(() => expect(f.writes).toHaveLength(1));
    const closing = f.session.close();
    expect(f.session.getSnapshot()).toEqual({ status: "closed", progress: null, candidate: null });
    release(); await rejected; await closing;
    await expect(f.session.retrySave()).rejects.toThrow("closed");
    expect(f.writes).toHaveLength(1);
  } finally { release(); await f.close(); }
});

test.each(["content-alias-exact", "no-declarations"])("reader preserves the saved version for %s without inventing an application", async name => {
  const f = await setup(name);
  try {
    const before = f.session.getSnapshot().progress;
    const result = await f.session.assessUpdate(f.evolution, f.options);
    expect(result.kind).toBe(name === "content-alias-exact" ? "exact" : "old_version_only");
    await expect(f.session.applyUpdate(true)).rejects.toThrow("Review");
    await f.session.continueOld(); expect(f.session.getSnapshot().progress).toBe(before); expect(f.writes).toHaveLength(0);
  } finally { await f.close(); }
});

test("a missing evaluator object remains retryable and a competing cursor invalidates the reviewed plan", async () => {
  const f = await setup("unseen-rule");
  const api = await loadAuthoring(), absent = api.createEvolution();
  try {
    await absent.connectStore(execution, f.library);
    expect((await f.session.assessUpdate(absent, f.options)).kind).toBe("needs_objects");
    const before = f.session.getSnapshot().progress;
    await f.session.assessUpdate(f.evolution, f.options);
    const rival = await openBook({ pack: f.previous.pack(), execution, storage: f.store });
    try {
      const view = (await rival.inspect()).view;
      if (view.interaction.kind !== "choose") throw new Error("Expected source choice");
      await rival.choose(view.cursor, view.interaction.choice_point, [view.interaction.options.find(option => option.enabled)!.id]);
    } finally { await rival.close(); }
    expect((await f.session.applyUpdate()).kind).toBe("conflict");
    expect(f.session.getSnapshot()).toMatchObject({ status: "conflict", progress: before });
    await f.session.reload(); expect(f.session.getSnapshot().progress?.view.cursor).not.toBe(before?.view.cursor);
  } finally { await absent.close(); await f.close(); }
});

test.each(["lost", "uncommitted"] as const)("reader keeps the old screen after an update's %s response and verifies its original write", async mode => {
  const f = await setup("unseen-rule");
  try {
    const before = f.session.getSnapshot().progress;
    await f.session.assessUpdate(f.evolution, f.options); f.setFault(mode);
    await expect(f.session.applyUpdate()).rejects.toThrow("reply lost");
    expect(f.session.getSnapshot()).toMatchObject({ status: "unknown", progress: before });
    await expect(f.session.reload()).rejects.toThrow("Retry and verify");
    await expect(f.session.assessUpdate(f.evolution, f.options)).rejects.toThrow();
    await f.session.retrySave();
    expect(f.writes.at(-1)).toEqual(f.writes.at(-2));
    expect(f.session.getSnapshot()).toMatchObject({ status: "saved", progress: { view: { artifact_id: f.target.artifactId } } });
  } finally { await f.close(); }
});

test("reader rejects stale confirmations after a choice, target or budget change and preserves old progress on quota", async () => {
  const f = await setup("target-rejoins");
  try {
    const assessment = await f.session.assessUpdate(f.evolution, f.options); expect(assessment.kind).toBe("needs_confirmation");
    await expect(f.session.applyUpdate()).rejects.toThrow("explicitly confirm");
    await f.session.assessUpdate(f.evolution, f.options);
    await f.session.assessUpdate(f.evolution, { target: f.previous.artifactId, migrationIds: [] });
    await expect(f.session.applyUpdate(true)).rejects.toThrow("Review");
    await f.session.assessUpdate(f.evolution, { ...f.options, budget: { transitions: 0, automatic_steps: 1, object_bytes: 1 } });
    await f.session.assessUpdate(f.evolution, f.options);
    const before = f.session.getSnapshot().progress;
    f.setFault("quota"); await expect(f.session.applyUpdate(true)).rejects.toBeInstanceOf(WriteRejected);
    expect(f.session.getSnapshot()).toMatchObject({ status: "quota", progress: before });
    await f.session.reload(); expect(f.session.getSnapshot().progress).toEqual(before);
    const earlier = before!.view.history.find(row => !row.finished)!;
    await f.session.checkout(earlier.id);
    await f.session.assessUpdate(f.evolution, f.options);
    const view = (await f.session.inspect()).view;
    if (view.interaction.kind !== "choose") throw new Error("Expected a choice");
    await f.session.choose(view.cursor, view.interaction.choice_point, [view.interaction.options.find(option => option.enabled)!.id]);
    await expect(f.session.applyUpdate(true)).rejects.toThrow("Review");
  } finally { await f.close(); }
});
