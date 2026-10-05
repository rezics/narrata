import { expect, test } from "vitest";
import { collectScreenContent, contentItemKey, contentRequests, resolveContent, type ResolveItem, type Resolution } from "../src/content/index.js";
import { MockRezicsContent, mountainLetterData, mountainLetterKeys } from "../src/testing/index.js";
import type { BookView } from "../src/generated/book-view.js";
import type { MockRezicsData } from "../src/testing/index.js";

function key(name: string): string {
  const value = mountainLetterKeys[name];
  if (!value) throw new Error(`Missing fixture key ${name}`);
  return value;
}
function ref(name: string) { return { provider: "rezics", key: key(name) }; }
function body(name: string, first?: string, last?: string): ResolveItem {
  return { content: { unit: ref(name), ...(first ? { first } : {}), ...(last ? { last } : {}) } };
}
function document(data: MockRezicsData, name: string, realization = "en") {
  const value = data.occurrences[key(name)]?.realizations[realization];
  if (!value) throw new Error("Missing fixture document");
  return value;
}
function text(result: Resolution | undefined): string {
  if (result?.status !== "ok") throw new Error("Expected resolved content");
  return "text" in result.payload ? result.payload.text : result.payload.blocks.map(block => block.text).join("|");
}
function screen(): BookView {
  return {
    page: [{ commit: "commit", occurrence: 0, role: "body", node: "node", content: { unit: ref("main.ledger"), first: "l1", last: "l1" }, args: {} }],
    graphs: [{ entry: "hidden", exported: true, reference: { package: "hidden", graph: "hidden" }, nodes: [{ id: "hidden", kind: "passage", edges: [], title: ref("road.fog.title") }] }],
    diagnostics: [],
    view: {
      artifact_id: "artifact", execution: "execution", cursor: "commit", depth: 0,
      product: { id: "mountain-letter", title: ref("product:title") },
      presentation: [], frames: [], shared: [], history: [],
      interaction: { kind: "choose", graph: { package: "main", graph: "journey" }, node: "node", choice_point: "point", min: 1, max: 1, args: {}, options: [
        { id: "sign", label: ref("main.journey:ledger.sign.label"), enabled: true, outcome: "local" },
        { id: "skip", label: ref("main.journey:ledger.skip.label"), enabled: false, reason: ref("main.journey:gate.deliver.reason"), outcome: "local" },
      ] },
    },
  };
}

test("body and option labels share selected realization across bounded screen batches", async () => {
  const mock = new MockRezicsContent(mountainLetterData(), 2);
  const items = collectScreenContent(screen());
  const contexts: unknown[] = [];
  const results = await resolveContent({ resolve: request => {
    contexts.push(request.context);
    expect(request.items.length).toBeLessThanOrEqual(2);
    return mock.resolve(request);
  } }, items, { languages: ["zh-Hans"], realization: "en", viewer: "reader:1" }, 2);
  expect(text(results[0])).toBe("Letter from the Pass");
  expect(text(results[1])).toContain("register lies open");
  expect(text(results[2])).toBe("Write your name with the charcoal");
  // This label has no English realization and falls back to its original language.
  expect(text(results[4])).toBe("需要先从营地老人那里取到信件");
  expect(contexts).toEqual(Array.from({ length: 3 }, () => ({ languages: ["zh-Hans"], realization: "en", viewer: "reader:1" })));
  expect(items.some(item => contentItemKey(item).includes(key("road.fog.title")))).toBe(false);
});

test("collection preserves bounds and arguments, deduplicates equivalent items and copies inputs", () => {
  const book = screen();
  const first = book.page[0];
  if (!first) throw new Error("No page");
  first.args = { b: { type: "text", value: "B" }, a: { type: "int", value: "1" } };
  book.page.push({ ...first, args: { a: { type: "int", value: "1" }, b: { type: "text", value: "B" } } });
  book.page.push({ ...first, content: { unit: ref("main.ledger"), first: "l2", last: "l2" } });
  book.page.push({ ...first, args: { a: { type: "int", value: "2" }, b: { type: "text", value: "B" } } });
  const collected = collectScreenContent(book);
  expect(collected.filter(item => "unit" in item.content)).toHaveLength(3);
  first.args.a = { type: "int", value: "999" };
  expect(collected.find(item => "unit" in item.content)?.args?.a).toEqual({ type: "int", value: "1" });
  const context = { languages: ["en"], realization: "en", viewer: "reader" };
  const batches = contentRequests(collected, context, 2);
  context.languages[0] = "zh-Hans";
  expect(batches.every(batch => batch.context.languages?.[0] === "en")).toBe(true);
  expect(batches.flatMap(batch => batch.items)).toEqual(collected);
  expect(contentRequests([], {}, 1)).toEqual([]);
  for (const limit of [0, -1, 1.5, Infinity, NaN]) expect(() => contentRequests([], {}, limit)).toThrow(RangeError);
});

test("unknown, unauthorized and withdrawn content return identical unavailable values", async () => {
  const mock = new MockRezicsContent(mountainLetterData());
  mock.restrict(key("main.ledger"), ["allowed"]);
  mock.withdraw(key("main.station"));
  const items: ResolveItem[] = [body("main.ledger"), body("main.station"), { content: { provider: "rezics", key: "https://example.test/absent" } }];
  expect(await mock.resolve({ context: { viewer: "denied" }, items })).toEqual(items.map(() => ({ status: "unavailable" })));
  expect(await mock.resolve({ context: {}, items: [body("main.ledger")] })).toEqual([{ status: "unavailable" }]);
  expect(text((await mock.resolve({ context: { viewer: "allowed" }, items: [body("main.ledger", "l1", "l1")] }))[0])).toContain("登记册");
  mock.withdraw(key("main.journey:ledger.sign.label"));
  expect(await mock.resolve({ context: {}, items: [{ content: ref("main.journey:ledger.sign.label") }] })).toEqual([{ status: "unavailable" }]);
});

test("missing realizations fall back to original, languages use lookup, version selection wins", async () => {
  const mock = new MockRezicsContent(mountainLetterData());
  const items = [body("main.ledger", "l1", "l1"), { content: ref("main.journey:ledger.sign.label") }];
  const english = await mock.resolve({ context: { languages: ["fr", "en-US"] }, items });
  expect(text(english[0])).toContain("register lies open");
  expect(text(english[1])).toContain("Write your name");
  for (const realization of ["absent", "constructor", "original"]) {
    const results = await mock.resolve({ context: { languages: ["en"], realization }, items });
    expect(text(results[0])).toContain("登记册");
    expect(text(results[1])).toContain("自己的名字");
  }
});

test("an explicit realization selects a version within the same language for bodies and labels", async () => {
  const data = mountainLetterData();
  const occurrence = data.occurrences[key("main.ledger")];
  const label = data.labels[key("main.journey:ledger.sign.label")];
  if (!occurrence || !label?.realizations.en) throw new Error("Missing fixture");
  occurrence.realizations["en-revised"] = structuredClone(document(data, "main.ledger"));
  const first = occurrence.realizations["en-revised"].blocks[0];
  if (first?.type !== "paragraph") throw new Error("Missing paragraph");
  first.text = "The revised register scene.";
  occurrence.realizations["en-revised"].revision = "en:2";
  label.realizations["en-revised"] = { ...label.realizations.en, revision: "en:2", text: "Sign the register" };
  const mock = new MockRezicsContent(data);
  const results = await mock.resolve({ context: { languages: ["en"], realization: "en-revised" }, items: [
    body("main.ledger", "l1", "l1"), { content: ref("main.journey:ledger.sign.label") },
  ] });
  expect(results).toEqual([
    { status: "ok", revision: "en:2", payload: { blocks: [{ id: "l1", text: "The revised register scene." }] } },
    { status: "ok", revision: "en:2", payload: { text: "Sign the register" } },
  ]);
});

test("missing and reversed endpoints are incompatible; inserted original blocks remain in the segment", async () => {
  const data = mountainLetterData();
  const original = document(data, "main.ledger", "original");
  original.blocks.splice(1, 0, { type: "paragraph", attrs: { id: "inserted" }, text: "新插入的一行。" });
  const english = document(data, "main.ledger");
  english.blocks = english.blocks.filter(block => block.attrs.id !== "l2");
  const mock = new MockRezicsContent(data);
  const missing = await mock.resolve({ context: { realization: "en" }, items: [body("main.ledger", "l1", "l2"), body("main.ledger", "l2", "l3")] });
  expect(missing.every(result => result.status === "incompatible")).toBe(true);
  const reversed = await mock.resolve({ context: {}, items: [body("main.ledger", "l3", "l1")] });
  expect(reversed[0]?.status).toBe("incompatible");
  const result = (await mock.resolve({ context: {}, items: [body("main.ledger", "l1", "l2")] }))[0];
  expect(text(result)).toContain("新插入的一行。");
});

test("outlines retain original block order and choice markers without leaking text", async () => {
  const data = mountainLetterData();
  const marker = { type: "narrata-choice" as const, attrs: { id: "marker", choice_point: "choice-point:" + "1".repeat(32) } };
  document(data, "main.ledger", "original").blocks.splice(1, 0, marker);
  document(data, "main.ledger").blocks.splice(1, 0, structuredClone(marker));
  const mock = new MockRezicsContent(data);
  const outline = mock.outline();
  expect(outline.units[key("main.ledger")]?.blocks.slice(0, 3)).toEqual(["l1", "marker", "l-sign"]);
  expect(outline.units[key("main.ledger")]?.markers).toEqual({ marker: marker.attrs.choice_point });
  expect(JSON.stringify(outline)).not.toContain("登记册");
  expect(JSON.stringify(outline)).not.toContain("Write your name");
  const result = (await mock.resolve({ context: { realization: "en" }, items: [body("main.ledger", "l1", "l-sign")] }))[0];
  if (result?.status !== "ok" || !("blocks" in result.payload)) throw new Error("No blocks");
  expect(result.payload.blocks.map(block => block.id)).toEqual(["l1", "l-sign"]);
});

test("translations reject renamed, duplicate, reordered anchors and changed markers", () => {
  for (const mutate of [
    (blocks: ReturnType<typeof document>["blocks"]) => { const first = blocks[0]; if (first) first.attrs.id = "renamed"; },
    (blocks: ReturnType<typeof document>["blocks"]) => { const first = blocks[0]; if (first) blocks.push(structuredClone(first)); },
    (blocks: ReturnType<typeof document>["blocks"]) => blocks.reverse(),
  ]) {
    const data = mountainLetterData(); mutate(document(data, "main.ledger").blocks);
    expect(() => new MockRezicsContent(data)).toThrow("original block IDs and order");
  }
  const data = mountainLetterData();
  document(data, "main.ledger", "original").blocks.splice(1, 0, { type: "narrata-choice", attrs: { id: "marker", choice_point: "choice-point:" + "1".repeat(32) } });
  document(data, "main.ledger").blocks.splice(1, 0, { type: "narrata-choice", attrs: { id: "marker", choice_point: "choice-point:" + "2".repeat(32) } });
  expect(() => new MockRezicsContent(data)).toThrow("changed a choice marker");
});

test("named reference arguments use the same realization and viewer", async () => {
  const data = mountainLetterData();
  const label = data.labels[key("product:title")];
  if (!label?.realizations.en) throw new Error("No English label");
  label.realizations.en.text = "{who}: {count} {{letters}}";
  const mock = new MockRezicsContent(data);
  const item: ResolveItem = { content: ref("product:title"), args: {
    who: { type: "ref", value: ref("main.journey:ledger.sign.label") }, count: { type: "int", value: "9007199254740993" },
  } };
  expect(text((await mock.resolve({ context: { realization: "en" }, items: [item] }))[0])).toBe("Write your name with the charcoal: 9007199254740993 {letters}");
  mock.restrict(key("main.journey:ledger.sign.label"), ["allowed"]);
  expect((await mock.resolve({ context: { realization: "en", viewer: "denied" }, items: [item] }))[0]?.status).toBe("incompatible");
});

test("batch helpers preserve order and fail on transport and cardinality errors", async () => {
  const items = [body("main.station"), body("main.ledger")];
  await expect(resolveContent({ resolve: async () => [] }, items, {}, 2)).rejects.toThrow("result count");
  await expect(resolveContent({ resolve: async () => { throw new Error("network"); } }, items, {}, 2)).rejects.toThrow("network");
  await expect(new MockRezicsContent(mountainLetterData(), 1).resolve({ context: {}, items })).rejects.toThrow("batch limit");
  const results = await resolveContent({ resolve: async request => request.items.map(item => ({ status: "ok", revision: "r", payload: { text: contentItemKey(item) } })) }, items, {}, 1);
  expect(results.map(result => text(result))).toEqual(items.map(contentItemKey));
});
