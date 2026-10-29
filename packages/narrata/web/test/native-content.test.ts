import { expect, test } from "vitest";
import { createNativeContentAdapter, nativeContentOutline, type NativeContentResult, type NativeDocument } from "../src/content/native.js";
import { nativeDocumentSample, type SampleNativeValue } from "../src/testing/native-document.js";
import type { ResolveContext, ResolveItem, Resolution } from "../src/content/index.js";

const context: ResolveContext = { languages: ["en"], realization: "edition", viewer: "reader" };
const item: ResolveItem = { content: { unit: { provider: "native", key: "chapter" }, first: "arrival", last: "decision" }, args: { count: { type: "int", value: "2" } } };
const request = (items = [item], ctx = context) => ({ context: ctx, items });
function first(results: Resolution[]): Resolution {
  const result = results[0];
  if (!result) throw new Error("Missing resolution");
  return result;
}

test("native slices and standard fallbacks use the same ordered revision and inclusive endpoints", async () => {
  const document = nativeDocumentSample();
  const adapter = createNativeContentAdapter<SampleNativeValue>({ resolve: async () => [{ status: "document", document }] });
  const resolution = first(await adapter.resolve(request()));
  expect(resolution).toEqual({ status: "ok", revision: "document:1", payload: { blocks: [
    { id: "arrival", text: "A letter waits beside the mountain map." },
    { id: "map", text: "Map showing the mountain pass and the riverside route." },
  ] } });
  const slice = adapter.lookup(item, context, resolution);
  expect(slice?.blocks.map(block => block.id)).toEqual(["arrival", "map"]);
  expect(slice?.blocks[0]?.value).toMatchObject({ kind: "paragraph", runs: [
    { text: "A letter waits beside the " }, { text: "mountain map", strong: true }, { text: "." },
  ] });
  const outline = nativeContentOutline("native", { chapter: document });
  expect(outline).toEqual({ format_version: 1, provider: "native", units: { chapter: {
    blocks: ["arrival", "map", "decision", "departure"], markers: { decision: `choice-point:${"1".repeat(32)}` },
  } } });
  expect(JSON.stringify(outline)).not.toContain("mountain");
  expect(JSON.stringify(outline)).not.toContain("document:1");
});

test("inserting a native block inside a segment includes it without changing either endpoint", async () => {
  const original = nativeDocumentSample();
  const inserted = { id: "inserted", text: "Read the route notes.", value: { kind: "paragraph" as const, runs: [{ text: "Read the route notes." }] } };
  const document: NativeDocument<SampleNativeValue> = { revision: "document:2", blocks: [original.blocks[0]!, inserted, ...original.blocks.slice(1)] };
  const adapter = createNativeContentAdapter({ resolve: async () => [{ status: "document", document }] });
  const resolution = first(await adapter.resolve(request()));
  expect(adapter.lookup(item, context, resolution)?.blocks.map(block => block.id)).toEqual(["arrival", "inserted", "map"]);
  expect(resolution.status === "ok" && "blocks" in resolution.payload && resolution.payload.blocks.map(block => block.id)).toEqual(["arrival", "inserted", "map"]);
});

test("missing or reversed endpoints, duplicate anchors and invalid markers cannot produce native slices", async () => {
  for (const content of [
    { unit: { provider: "native", key: "chapter" }, first: "missing" },
    { unit: { provider: "native", key: "chapter" }, first: "departure", last: "arrival" },
  ]) {
    const adapter = createNativeContentAdapter({ resolve: async () => [{ status: "document", document: nativeDocumentSample() }] });
    const bad: ResolveItem = { content };
    const resolution = first(await adapter.resolve(request([bad])));
    expect(resolution.status).toBe("incompatible");
    expect(adapter.lookup(bad, context, resolution)).toBeUndefined();
  }
  const original = nativeDocumentSample();
  for (const document of [
    { revision: "bad", blocks: [...original.blocks, original.blocks[0]!] },
    { revision: "bad", blocks: [{ id: "marker", choicePoint: "not-a-choice-point" }] },
    { revision: "bad", blocks: [{ id: "bad\nanchor", text: "text", value: {} }] },
  ]) {
    const adapter = createNativeContentAdapter({ resolve: async () => [{ status: "document", document }] });
    const resolution = first(await adapter.resolve(request()));
    expect(resolution.status).toBe("incompatible");
    expect(adapter.lookup(item, context, resolution)).toBeUndefined();
    expect(() => nativeContentOutline("native", { chapter: document })).toThrow();
  }
});

test("native snapshots are copied, revision-bound and partitioned by every context value, bounds and arguments", async () => {
  let document = nativeDocumentSample();
  const adapter = createNativeContentAdapter({ resolve: async request => {
    request.context.viewer = "mutated by host";
    return [{ status: "document", document }];
  } });
  const revisionA = first(await adapter.resolve(request()));
  const nativeA = adapter.lookup(item, context, revisionA);
  expect(nativeA?.revision).toBe("document:1");
  const paragraph = document.blocks[0];
  if (!paragraph || paragraph.choicePoint !== undefined || paragraph.value.kind !== "paragraph") throw new Error("No paragraph");
  // Even an object mutation under a host revision cannot mutate a previously cached snapshot.
  Object.assign(paragraph.value.runs[0]!, { text: "Changed host object" });
  document = { ...document, revision: "document:2" };
  const revisionB = first(await adapter.resolve(request()));
  expect(adapter.lookup(item, context, revisionA)).toEqual(nativeA);
  expect(adapter.lookup(item, context, revisionB)?.blocks[0]?.text).toBe("A letter waits beside the mountain map.");
  if (nativeA?.blocks[0]?.value.kind === "paragraph") Object.assign(nativeA.blocks[0].value.runs[0]!, { text: "Changed lookup result" });
  expect(adapter.lookup(item, context, revisionA)?.blocks[0]?.value).not.toEqual(nativeA?.blocks[0]?.value);
  for (const other of [
    { ...context, languages: ["fr"] }, { ...context, realization: "other" }, { ...context, viewer: "another-reader" },
  ]) expect(adapter.lookup(item, other, revisionA)).toBeUndefined();
  expect(adapter.lookup({ ...item, args: { count: { type: "int", value: "3" } } }, context, revisionA)).toBeUndefined();
  expect(adapter.lookup({ content: { unit: { provider: "native", key: "chapter" }, first: "arrival", last: "map" } }, context, revisionA)).toBeUndefined();
  expect(adapter.lookup(item, context, { status: "unavailable" })).toBeUndefined();
  expect(adapter.lookup(item, context, { status: "incompatible" })).toBeUndefined();
  adapter.clear();
  expect(adapter.lookup(item, context, revisionA)).toBeUndefined();
});

test("cache budgets evict native slices while standard fallbacks stay usable", async () => {
  const adapter = createNativeContentAdapter({ maxCacheEntries: 1, maxCacheBlocks: 2,
    resolve: async request => request.items.map(() => ({ status: "document", document: nativeDocumentSample() })) });
  const one = first(await adapter.resolve(request()));
  const other: ResolveItem = { content: { unit: { provider: "native", key: "other" }, first: "departure" } };
  const two = first(await adapter.resolve(request([other])));
  expect(adapter.lookup(item, context, one)).toBeUndefined();
  expect(adapter.lookup(other, context, two)?.blocks.map(block => block.id)).toEqual(["departure"]);
  const whole: ResolveItem = { content: { unit: { provider: "native", key: "chapter" } } };
  const tooLarge = first(await adapter.resolve(request([whole])));
  expect(tooLarge.status === "ok" && "blocks" in tooLarge.payload && tooLarge.payload.blocks).toHaveLength(3);
  expect(adapter.lookup(whole, context, tooLarge)).toBeUndefined();
  const blockBound = createNativeContentAdapter({ maxCacheEntries: 10, maxCacheBlocks: 2,
    resolve: async request => request.items.map(() => ({ status: "document", document: nativeDocumentSample() })) });
  const old = first(await blockBound.resolve(request()));
  await blockBound.resolve(request([other]));
  expect(blockBound.lookup(item, context, old)).toBeUndefined();
});

test("clear invalidates in-flight cache writes; callback errors and cardinality errors remain retryable", async () => {
  let release: (value: NativeContentResult<SampleNativeValue>[]) => void = () => { throw new Error("Not waiting"); };
  const adapter = createNativeContentAdapter<SampleNativeValue>({ resolve: () => new Promise(resolve => { release = resolve; }) });
  const pending = adapter.resolve(request());
  adapter.clear();
  release([{ status: "document", document: nativeDocumentSample() }]);
  const result = first(await pending);
  expect(result.status).toBe("ok");
  expect(adapter.lookup(item, context, result)).toBeUndefined();
  await expect(createNativeContentAdapter({ resolve: async () => [] }).resolve(request())).rejects.toThrow("result count");
  await expect(createNativeContentAdapter({ resolve: async () => { throw new Error("offline"); } }).resolve(request())).rejects.toThrow("offline");
  for (const limit of [0, -1, 1.5, Infinity]) expect(() => createNativeContentAdapter({ maxCacheBlocks: limit, resolve: async () => [] })).toThrow(RangeError);
});
