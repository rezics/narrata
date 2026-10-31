import "fake-indexeddb/auto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import { ArtifactUnavailable, createReader, openBook, readerContent, type ArtifactSource, type BookView } from "@rezics/narrata/reader";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Reader, type ContentSlotProps } from "@rezics/narrata/react";
import { IndexedDbStore } from "@rezics/narrata/storage";
import { execution, splitPack } from "./fixture.js";

const directory = resolve(import.meta.dirname, "../../../../fixtures/compat/nodes-evolution-v1/republished-after-migration");
const bytes = (name: string) => new Uint8Array(readFileSync(resolve(directory, name)));
const packs = [bytes("ancestor.narpack"), bytes("source.narpack"), bytes("target.narpack")];
const checkpoint = Buffer.from(bytes("target.checkpoint")).toString("hex");
function choice(view: BookView) {
  const interaction = view.view.interaction;
  if (interaction.kind !== "choose") throw new Error("Expected a choice");
  const selected = interaction.options.filter(option => option.enabled).slice(0, Math.max(1, interaction.min));
  return { expected: view.view.cursor, point: interaction.choice_point, options: selected.map(option => option.id) };
}
async function registry() {
  const artifacts = new Map<string, { pack: Uint8Array; source: ArtifactSource; parts: ReturnType<typeof splitPack> }>();
  for (const pack of packs) {
    const book = await openBook({ pack, execution });
    const parts = splitPack(pack);
    artifacts.set(book.artifactId, { pack, parts, source: { manifest: parts.manifest, names: parts.names, fetchChunk: async id => {
      const object = parts.objects.get(id); if (!object) throw new Error("No object in this artifact"); return object;
    } } });
    await book.close();
  }
  return artifacts;
}
async function full(storage?: IndexedDbStore) {
  return openBook({ pack: packs[2]!, historyPacks: packs.slice(0, 2), execution, ...(storage ? { storage } : {}) });
}

test("full-pack checkout switches aliases, analysis, pack, lookahead and reload to each commit's exact program", async () => {
  const artifacts = await registry();
  const store = await IndexedDbStore.open(`history-context-${crypto.randomUUID()}`);
  const book = await full(store);
  const reference = await full();
  try {
    await book.restore(checkpoint);
    await reference.restore(checkpoint);
    const history = (await book.inspect()).view.history;
    expect(history.length).toBeGreaterThan(2);
    const seen = new Set<string>();
    for (const row of history) {
      await book.checkout(row.id);
      await reference.checkout(row.id);
      const actual = await book.inspect();
      seen.add(actual.view.artifact_id);
      const artifact = artifacts.get(actual.view.artifact_id); expect(artifact).toBeDefined();
      expect(book.artifactId).toBe(actual.view.artifact_id);
      expect(book.pack()).toEqual(artifact!.pack);
      const standalone = await openBook({ pack: artifact!.pack, execution });
      try {
        expect(actual.graphs).toEqual((await standalone.inspect()).graphs);
        expect(actual.diagnostics).toEqual((await standalone.inspect()).diagnostics);
        expect(actual).toEqual(await reference.inspect());
        expect(await book.nextContentUnits()).toEqual(await reference.nextContentUnits());
        await book.reload(); expect(await book.inspect()).toEqual(actual);
        if (actual.view.interaction.kind === "choose") {
          const next = choice(actual);
          await book.choose(next.expected, next.point, next.options);
          await reference.choose(next.expected, next.point, next.options);
          expect(await book.inspect()).toEqual(await reference.inspect());
          expect(await book.exportSave()).toBe(await reference.exportSave());
        }
      } finally { await standalone.close(); }
    }
    expect(seen.size).toBe(3);
  } finally { await reference.close(); await book.close(); store.close(); }
});

test.each(["native", "legacy"])("incoming %s A-root restores on B through exact lazy dispatch and preserves B on failed loads", async format => {
  const artifacts = await registry();
  const a = [...artifacts.values()][0]!, b = [...artifacts.values()][2]!;
  const original = resolve(directory, "../old-save-continue");
  const save = format === "legacy" ? readFileSync(resolve(original, "legacy.json"), "utf8")
    : readFileSync(resolve(original, "source.checkpoint")).toString("hex");
  const store = await IndexedDbStore.open(`incoming-history-${crypto.randomUUID()}`);
  const requests: string[] = [], chunks: string[] = [];
  let unavailable = true;
  const book = await openBook({ pack: b.pack, execution, storage: store, loadArtifact: async artifact => {
    requests.push(artifact);
    if (unavailable) throw new Error("Exact A offline");
    expect(artifact).toBe([...artifacts.keys()][0]);
    return { ...a.source, fetchChunk: async id => { chunks.push(id); return a.source.fetchChunk(id); } };
  } });
  const reference = await openBook({ pack: b.pack, historyPacks: [a.pack], execution });
  try {
    const before = await book.inspect(), savedB = await book.exportSave();
    await expect(book.restore(save)).rejects.toBeInstanceOf(ArtifactUnavailable);
    expect(await book.inspect()).toEqual(before); expect(await book.exportSave()).toBe(savedB);
    unavailable = false;
    await book.restore(save); await reference.restore(save);
    const actual = await book.inspect(), expected = await reference.inspect();
    expect(requests).toEqual([[...artifacts.keys()][0], [...artifacts.keys()][0]]);
    expect(book.artifactId).toBe([...artifacts.keys()][0]);
    expect(actual.view).toEqual(expected.view); expect(actual.page).toEqual(expected.page);
    expect(actual.graphs).toEqual([]); expect(actual.diagnostics).toEqual([]);
    expect(actual.view.frames.every(frame => frame.key !== undefined)).toBe(true);
    expect(await book.nextContentUnits()).toEqual(await reference.nextContentUnits());
    expect(chunks.length).toBeGreaterThan(0);
    expect(chunks.every(id => a.parts.objects.has(id))).toBe(true);
    expect(chunks).not.toContain([...a.parts.objects.keys()].at(-1));
    await book.reload(); expect((await book.inspect()).view).toEqual(actual.view);
    const next = choice(actual);
    await book.choose(next.expected, next.point, next.options); await reference.choose(next.expected, next.point, next.options);
    expect((await book.inspect()).view).toEqual((await reference.inspect()).view);
    expect(await book.exportSave()).toBe(await reference.exportSave());
  } finally { await book.close(); await reference.close(); store.close(); }
});

test("historical slots resolve identical references in their own artifact context after checkout", async () => {
  const book = await full();
  await book.restore(checkpoint);
  const reader = createReader(book, { resolve: async request => request.items.map(() => ({ status: "ok", revision: request.context.realization ?? "missing",
    payload: { text: request.context.realization ?? "missing" } })) }, { context: { viewer: "reader" }, contextForArtifact: (artifact, context) => ({ ...context, realization: artifact }) });
  try {
    expect(await reader.start()).toBe(true);
    const screen = reader.getSnapshot().screen!;
    const identities = new Set(Object.values(screen.artifacts!)); expect(identities.size).toBe(3);
    for (const row of screen.book.view.history) if (row.title) {
      const artifact = screen.artifacts![row.id]!;
      expect(readerContent(screen, { content: row.title }, artifact)).toEqual({ status: "ok", revision: artifact, payload: { text: artifact } });
    }
    const slot = (props: ContentSlotProps) => createElement("span", { "data-context": props.context.realization }, props.resolution.status === "ok" && "text" in props.resolution.payload ? props.resolution.payload.text : "unavailable");
    const html = renderToStaticMarkup(createElement(Reader, { controller: reader, slots: { body: slot, option: slot, reference: slot } }));
    for (const artifact of identities) expect(html).toContain(`data-context="${artifact}"`);
    for (const row of screen.book.view.history) {
      expect(await reader.checkout(row.id)).toBe(true);
      const current = reader.getSnapshot().screen!;
      expect(current.context.realization).toBe(current.book.view.artifact_id);
    }
  } finally { reader.dispose(); await book.close(); }
});

test("lazy historical manifests retry missing artifacts/chunks and preserve all timeline identities and aliases", async () => {
  const artifacts = await registry();
  const store = await IndexedDbStore.open(`lazy-history-${crypto.randomUUID()}`);
  const seed = await full(store); await seed.restore(checkpoint); const expected = await seed.inspect(); await seed.close();
  const target = artifacts.get(expected.view.artifact_id)!;
  const manifests: string[] = [], chunks: { artifact: string; object: string }[] = [];
  let missing: "artifact" | "chunk" | "none" = "artifact";
  const book = await openBook({ manifest: target.source.manifest, names: target.parts.names, execution, storage: store,
    fetchChunk: target.source.fetchChunk, loadArtifact: async artifact => {
      manifests.push(artifact);
      if (missing === "artifact") throw new Error("Historic publication offline");
      const source = artifacts.get(artifact)?.source; if (!source) throw new Error("Unknown artifact");
      return { ...source, fetchChunk: async object => {
        chunks.push({ artifact, object });
        if (missing === "chunk") throw new Error("Historic chunk offline");
        return source.fetchChunk(object);
      } };
    } });
  try {
    await expect(book.inspect()).rejects.toBeInstanceOf(ArtifactUnavailable);
    expect(await book.exportSave()).toBe(checkpoint);
    missing = "chunk";
    await expect(book.inspect()).rejects.toBeInstanceOf(ArtifactUnavailable);
    expect(await book.exportSave()).toBe(checkpoint);
    missing = "none";
    const actual = await book.inspect();
    expect(actual.view).toEqual(expected.view); expect(actual.page).toEqual(expected.page);
    expect(actual.graphs).toEqual([]); expect(actual.diagnostics).toEqual([]);
    expect(actual.view.history.every(row => row.key !== undefined)).toBe(true);
    expect(new Set(manifests).size).toBe(2);
    for (const request of chunks) expect(artifacts.get(request.artifact)!.parts.objects.has(request.object)).toBe(true);
    // Registration and rendering require no artifact-wide tombstone/full-pack scan.
    for (const artifact of artifacts.values()) expect(chunks.some(request => request.object === [...artifact.parts.objects.keys()].at(-1))).toBe(false);
    const reference = await full();
    await reference.restore(checkpoint);
    try { for (const row of expected.view.history) {
      await book.checkout(row.id);
      await reference.checkout(row.id);
      const before = await book.inspect();
        const native = await reference.inspect();
        expect(before.view).toEqual(native.view); expect(before.page).toEqual(native.page);
        expect(await book.nextContentUnits()).toEqual(await reference.nextContentUnits());
        if (before.view.interaction.kind === "choose") {
          const next = choice(before); await book.choose(next.expected, next.point, next.options);
          await reference.choose(next.expected, next.point, next.options);
          const after = await book.inspect(), wanted = await reference.inspect();
          expect(after.view).toEqual(wanted.view); expect(after.page).toEqual(wanted.page);
          expect(await book.exportSave()).toBe(await reference.exportSave());
          await book.reload(); expect((await book.inspect()).view).toEqual(after.view);
        }
    } } finally { await reference.close(); }
    await expect(book.restore("broken save")).rejects.not.toBeInstanceOf(ArtifactUnavailable);
  } finally { await book.close(); store.close(); }
});
