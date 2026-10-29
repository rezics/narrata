import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test } from "vitest";
import { AuthoringError, decodeRecord, loadAuthoring, type Authoring, type DraftRecord, type Entropy, type PackageSource, type ProjectManifest, type Publication } from "@rezics/narrata/server";
import { initialize, LocalContent, openBook, type Book } from "@rezics/narrata";
import { shortStoryOutline, shortStorySource } from "@rezics/narrata/testing";
import legacyInit, { publishGraph } from "../dist/wasm/narrata_nodes_wasm.js";
import { execution } from "./fixture.js";

const productPath = (path: string) => resolve(import.meta.dirname, "../../../../products/gamebook-demo", path);
const product = (path: string) => new Uint8Array(readFileSync(productPath(path)));
const text = (bytes: Uint8Array) => new TextDecoder().decode(bytes);

function demoSource(): { manifest: ProjectManifest; packages: [string, PackageSource][] } {
  const manifest = JSON.parse(text(product("project.json"))) as ProjectManifest;
  return { manifest, packages: Object.entries(manifest.packages).map(([alias, path]) => [alias, JSON.parse(text(product(path))) as PackageSource]) };
}
async function demoOutline() {
  await initialize();
  const content = new LocalContent();
  try {
    content.add(text(product("content/zh-Hans.json")));
    return JSON.parse(content.outline("local", ["zh-Hans"]) ?? "null");
  } finally { content.free(); }
}
function counter(start = 1): Entropy {
  let next = start;
  return { now: () => 1_750_000_000_000, random: bytes => { bytes.fill(next++ & 255); } };
}
async function choose(book: Book, key: string): Promise<void> {
  const view = (await book.inspect()).view;
  if (view.interaction.kind !== "choose") throw new Error("Expected a choice");
  const option = view.interaction.options.find(option => option.key === key);
  if (!option) throw new Error(`Missing option ${key}`);
  await book.choose(view.cursor, view.interaction.choice_point, [option.id]);
}
/** A host's immutable object store, keyed by the description's hex IDs. */
function store(publication: Publication): Map<string, Uint8Array> {
  return new Map([...publication.objects()].map(([reference, bytes]) => [reference.object_id, bytes]));
}
/** Opens a stored publication's manifest; aliases (option keys) come from its optional name table. */
function openStored(objects: ReadonlyMap<string, Uint8Array>, publication: { artifact: string; names?: string | null | undefined }, requests: string[] = []): Promise<Book> {
  const manifest = objects.get(publication.artifact.slice("artifact:".length));
  const names = publication.names ? objects.get(publication.names) : undefined;
  if (!manifest || !names) throw new Error("Manifest or names not stored");
  return openBook({ manifest, names, execution, fetchChunk: async id => {
    requests.push(id);
    const bytes = objects.get(id.slice("object:".length));
    if (!bytes) throw new Error(`Missing object ${id}`);
    return bytes;
  } });
}

const authoring: Promise<Authoring> = loadAuthoring();

test("record-by-record publication equals the CLI's publication, pack, lock and legacy publishGraph", async () => {
  const api = await authoring;
  const source = demoSource();
  const split = api.splitSource(source.manifest, source.packages);
  expect(split.diagnostics).toEqual([]);
  const drafts = api.createDrafts();
  const outline = await demoOutline();
  try {
    for (const record of split.records) drafts.add(record.bytes);
    drafts.addOutline(outline);
    const { report, publication, records } = drafts.publish();
    expect(report.status).toBe("switchable");
    expect(records.map(record => record.key)).toEqual(split.records.map(record => record.key));
    if (!publication) throw new Error("Publication expected");
    try {
      expect(publication.descriptionBytes).toEqual(product("story.publication.json"));
      for (const [reference, bytes] of publication.objects()) expect(bytes).toEqual(product(`story.publication/${reference.object_id}.${reference.encoding}`));
      expect(publication.pack()).toEqual(product("story.narpack"));
      expect(report.lock).toEqual(JSON.parse(text(product("project.lock.json"))));
      await legacyInit({ module_or_path: readFileSync(resolve(import.meta.dirname, "../dist/wasm/narrata_nodes_wasm_bg.wasm")) });
      const legacy = JSON.parse(publishGraph(product("story.narpack"), JSON.stringify([outline]))) as {
        index: { object_id: string; bytes: number[] }; tiles: { object_id: string; bytes: number[] }[]; labels: { object_id: string; bytes: number[] }[]; summary_json: string;
      };
      for (const object of [legacy.index, ...legacy.tiles, ...legacy.labels]) expect(publication.object(object.object_id)).toEqual(Uint8Array.from(object.bytes));
      expect(JSON.parse(legacy.summary_json)).toEqual(JSON.parse(text(publication.object(publication.description.summary) ?? new Uint8Array())));
      const roles = new Set(publication.description.objects.filter(reference => ["graph_index", "graph_tile", "labels"].includes(reference.role)).map(reference => reference.object_id));
      expect(roles).toEqual(new Set([legacy.index, ...legacy.tiles, ...legacy.labels].map(object => object.object_id)));
    } finally { publication.free(); }
  } finally { drafts.free(); }
});

test("a short story from public types mints, previews, publishes A then B and keeps old journeys on A", async () => {
  const api = await authoring;
  const source = shortStorySource();
  const minter = api.createMinter(counter());
  const filled = minter.fill(source.manifest, source.packages);
  expect(filled.diagnostics).toEqual([]);
  expect(filled.minted).toHaveLength(5 + 2 + 4);
  expect(filled.records.every(record => record.indexes)).toBe(true);
  // Filling again keeps every existing ID and needs no entropy.
  const assembled = api.createDrafts();
  for (const record of filled.records) assembled.add(record.bytes);
  const exported = assembled.assemble().source;
  if (!exported) throw new Error("Assembled source expected");
  const again = api.createMinter({ now: () => { throw new Error("unused"); }, random: () => { throw new Error("unused"); } })
    .fill(exported.manifest, exported.packages);
  expect(again.minted).toEqual([]);
  expect(again.records.map(record => record.bytes)).toEqual(filled.records.map(record => record.bytes));
  assembled.free();

  const drafts = api.createDrafts();
  for (const record of filled.records) drafts.add(record.bytes);
  drafts.addOutline(shortStoryOutline());
  const checked = drafts.check();
  expect(checked.diagnostics).toEqual([]);
  expect(checked.records.flatMap(record => record.indexes?.options ?? []).map(row => row.target_status).sort())
    .toEqual(["control", "control", "passage", "passage"]);

  // Immediate preview through the reader's own module, chunk by chunk.
  const preview = drafts.preview();
  expect(preview.compiled).toBe(true);
  const manifest = preview.manifest();
  if (!manifest) throw new Error("Preview manifest expected");
  const previewBook = await openBook({ manifest, names: preview.names() ?? new Uint8Array(), execution, fetchChunk: async id => preview.object(id) ?? new Uint8Array() });
  await choose(previewBook, "forest");
  expect((await previewBook.inspect()).view.interaction.kind).toBe("choose");
  await previewBook.close();
  preview.free();

  const first = drafts.publish();
  expect(first.report.status).toBe("switchable");
  expect(first.report.compared).toBe(false);
  const a = first.publication;
  if (!a || !first.report.lock) throw new Error("Publication A expected");
  const objectsA = store(a);
  const descriptionA = a.descriptionBytes, idA = a.publicationId, artifactA = a.artifactId, lockA = first.report.lock;
  const openA = { artifact: artifactA, names: a.description.names };
  a.free();
  drafts.free();

  const requests: string[] = [];
  const journey = await openStored(objectsA, openA, requests);
  expect(requests).toHaveLength(1);
  await choose(journey, "forest");
  const saved = await journey.exportSave();
  const cursor = (await journey.inspect()).view.cursor;
  await journey.close();

  // Edit: remove the forest's "wander" option. The previous publication is re-read from its
  // description and single objects, not from a pack.
  const edited = filled.records.map(record => {
    const draft = decodeRecord(record.bytes);
    if (draft.kind === "choice_point" && draft.payload.key === "return") draft.payload.options = draft.payload.options.filter(option => option.key !== "wander");
    return draft;
  });
  const previous = await api.readPublication(descriptionA, idA, async reference => {
    const bytes = objectsA.get(reference.object_id);
    if (!bytes) throw new Error("missing");
    return bytes;
  });
  const next = api.createDrafts();
  for (const record of edited) next.add(record);
  next.addOutline(shortStoryOutline());
  const locked = next.publish({ previous, lock: lockA });
  expect(locked.publication).toBeUndefined();
  expect(locked.report.diagnostics.map(d => d.code)).toContain("tombstones_needed");
  const second = next.publish({ previous });
  next.free();
  const b = second.publication;
  if (!b || !second.report.lock) throw new Error("Publication B expected");
  expect(second.report.compared).toBe(true);
  expect(second.report.previous_artifact_id).toBe(artifactA);
  expect(b.artifactId).not.toBe(artifactA);
  expect(second.report.tombstones).toHaveLength(1);
  expect(second.records.some(record => decodeRecord(record.bytes).kind === "tombstone")).toBe(true);

  // Retrying B with its persisted records and lock is locked, idempotent and identical.
  const retry = api.createDrafts();
  for (const record of second.records) retry.add(record.bytes);
  retry.addOutline(shortStoryOutline());
  const repeated = retry.publish({ previous: b, lock: second.report.lock });
  expect(repeated.report.publication_id).toBe(b.publicationId);
  repeated.publication?.free();
  const stale = retry.publish({ previous: b, lock: lockA });
  expect(stale.report.diagnostics.map(d => d.code)).toContain("lock_mismatch");
  retry.free();

  const objectsB = store(b);
  const openB = { artifact: b.artifactId, names: b.description.names };
  b.free();
  previous.free();
  const resumed = await openStored(objectsA, openA);
  await resumed.restore(saved);
  expect((await resumed.inspect()).view.cursor).toBe(cursor);
  await resumed.close();
  const fresh = await openStored(objectsB, openB);
  await expect(fresh.restore(saved)).rejects.toThrow();
  await choose(fresh, "forest");
  const view = (await fresh.inspect()).view;
  if (view.interaction.kind !== "choose") throw new Error("Expected a choice");
  expect(view.interaction.options.map(option => option.key)).toEqual(["home"]);
  await fresh.close();
});

test("bad records are located by input position, record, node, choice point and option", async () => {
  const api = await authoring;
  const lone = api.validateRecord(new TextEncoder().encode('{"format_version":1,"kind":"project","payload":{},"extra":1}'));
  expect(lone.record).toBeUndefined();
  expect(lone.diagnostics.some(d => d.severity === "error")).toBe(true);
  const source = shortStorySource();
  const filled = api.createMinter(counter(40)).fill(source.manifest, source.packages);
  const drafts = api.createDrafts();
  let point: DraftRecord | undefined;
  for (const record of filled.records) {
    const draft = decodeRecord(record.bytes);
    if (draft.kind === "choice_point" && draft.payload.key === "path") {
      const option = draft.payload.options[1];
      if (option) option.outcome = { kind: "branch", target: "nowhere" };
      point = draft;
      drafts.add(draft);
    } else drafts.add(record.bytes);
  }
  drafts.add(new TextEncoder().encode("{\"format_version\":1,"));
  const checked = drafts.check();
  drafts.free();
  expect(checked.records).toEqual([]);
  if (point?.kind !== "choice_point") throw new Error("Choice point expected");
  const target = checked.diagnostics.find(d => d.option === point.payload.options[1]?.id);
  expect(target).toMatchObject({ severity: "error", node: point.node, choice_point: point.payload.id, record: point.payload.id });
  expect(target?.pointer.startsWith("/payload/options/1")).toBe(true);
  expect(checked.diagnostics.some(d => d.source_path === `records[${filled.records.length}]` && d.severity === "error")).toBe(true);
  expect(checked.diagnostics.some(d => d.code === "checks_skipped")).toBe(true);
});

test("objects are supplied and read one at a time with checked identity", async () => {
  const api = await authoring;
  const description = product("story.publication.json");
  const reference = JSON.parse(text(description)).objects[0] as { object_id: string; encoding: string };
  const packed = api.publicationFromPack(product("story.narpack"));
  const publicationId = packed.publicationId;
  packed.free();
  expect(() => api.openPublication(description, "0".repeat(64))).toThrow(AuthoringError);
  const loader = api.openPublication(description, publicationId);
  expect(() => loader.supply(reference.object_id, new Uint8Array(1))).toThrow(expect.objectContaining({ code: "object_length" }));
  expect(() => loader.supply("f".repeat(64), new Uint8Array(1))).toThrow(expect.objectContaining({ code: "reference" }));
  const incomplete = api.openPublication(description, publicationId);
  expect(() => incomplete.finish()).toThrow(expect.objectContaining({ code: "missing_object" }));
  for (const item of loader.references) loader.supply(item.object_id, product(`story.publication/${item.object_id}.${item.encoding}`));
  expect(loader.missing()).toEqual([]);
  const publication = loader.finish();
  expect(() => loader.missing()).toThrow("freed");
  const chunk = publication.description.objects.find(item => item.role === "chunk");
  if (!chunk) throw new Error("Chunk expected");
  expect(publication.object(`object:${chunk.object_id}`)).toEqual(publication.object(chunk.object_id));
  expect(() => publication.object("object:nothex")).toThrow(expect.objectContaining({ code: "identifier" }));
  publication.free();
  expect(() => publication.object(chunk.object_id)).toThrow("freed");
});

test("minting uses injected entropy, avoids occupied IDs and defaults to Web Crypto", async () => {
  const api = await authoring;
  const fixed = api.createMinter({ now: () => 1_750_000_000_000, random: bytes => bytes.fill(9) });
  const [node] = fixed.mint({ nodes: 1 }).nodes;
  expect(node).toMatch(/^node:[0-9a-f]{12}7[0-9a-f]{3}[89ab][0-9a-f]{15}$/);
  expect(() => fixed.mint({ nodes: 1 })).toThrow(expect.objectContaining({ code: "random" }));
  const other = api.createMinter({ now: () => 1_750_000_000_000, random: bytes => bytes.fill(9) });
  if (!node) throw new Error("Node expected");
  other.occupy([node]);
  expect(() => other.mint({ options: 1 })).toThrow(expect.objectContaining({ code: "random" }));
  expect(() => api.createMinter({ now: () => Number.NaN, random: () => undefined }).mint({ nodes: 1 })).toThrow(expect.objectContaining({ code: "clock" }));
  const host = api.createMinter().mint({ nodes: 1, choicePoints: 1, options: 2 });
  expect(new Set([...host.nodes, ...host.choice_points, ...host.options]).size).toBe(4);
  fixed.free(); other.free();
});
