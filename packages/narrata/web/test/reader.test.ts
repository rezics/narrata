import "fake-indexeddb/auto";
import { afterEach, expect, test, vi } from "vitest";
import { collectScreenContent, contentItemKey, createReader, openBook, readerContent } from "@rezics/narrata";
import type { BookView, ContentResolver, ReaderBook, ReaderController, ResolveRequest } from "@rezics/narrata";
import { IndexedDbStore, StoreSuperseded, type CacheStore } from "@rezics/narrata/storage";
import { demoPack, execution } from "./fixture.js";

afterEach(() => vi.useRealTimers());

function choice(view: BookView) {
  const interaction = view.view.interaction;
  if (interaction.kind !== "choose") throw new Error("Expected an interaction");
  return interaction;
}
const resolver: ContentResolver = { resolve: async request => request.items.map(() => ({ status: "ok", revision: "1", payload: { text: "Host content" } })) };
function screen(controller: ReaderController) {
  const value = controller.getSnapshot().screen;
  if (!value) throw new Error("Expected a screen");
  return value;
}

test("one immutable screen context batches every displayed reference, sanitizes failures and retries resolution", async () => {
  const book = await openBook({ pack: demoPack, execution });
  const requests: ResolveRequest[] = [];
  let fail = true;
  const context = { languages: ["en", "zh-Hans"], realization: "edition", viewer: "reader" };
  const reader = createReader(book, { resolve: async request => {
    requests.push(request);
    if (fail) throw new Error("private provider failure");
    return request.items.map((_, index) => index === 0 ? { status: "incompatible", reason: "private diagnostic" } : { status: "unavailable" });
  } }, { context, maxItems: 2 });
  context.languages[0] = "mutated";
  try {
    expect(reader.getSnapshot()).toBe(reader.getSnapshot());
    expect(await reader.start()).toBe(false);
    expect(reader.getSnapshot()).toMatchObject({ phase: "error", error: { kind: "content" }, screen: null });
    fail = false; requests.length = 0;
    expect(await reader.retry()).toBe(true);
    const snapshot = reader.getSnapshot();
    const value = screen(reader);
    expect(requests.flatMap(request => request.items)).toEqual(collectScreenContent(value.book));
    expect(requests.every(request => request.items.length <= 2 && request.context.languages?.[0] === "en")).toBe(true);
    expect(JSON.stringify(snapshot)).not.toContain("private");
    expect(value.content.some(entry => entry.resolution.status === "unavailable")).toBe(true);
    expect(Object.isFrozen(value.book.view.interaction)).toBe(true);
    expect(Object.isFrozen(value.context.languages)).toBe(true);
    const first = value.content[0];
    if (!first) throw new Error("Expected screen content");
    expect(readerContent(value, first.item)).toEqual({ status: "incompatible" });
    expect(await reader.setContext({ languages: ["fr"], realization: "other" })).toBe(true);
    expect(screen(reader).context).toEqual({ languages: ["fr"], realization: "other" });
    expect(value.context.languages?.[0]).toBe("en");
  } finally { reader.dispose(); await book.close(); }
});

test("multi-selection enforces min/max, uniqueness and enabled options, and submits in author order", async () => {
  const real = await openBook({ pack: demoPack, execution });
  const initial = await real.inspect();
  await real.close();
  const interaction = choice(initial);
  interaction.min = 2; interaction.max = 2;
  interaction.options = [
    { id: "a", enabled: true, outcome: "local" }, { id: "b", enabled: true, outcome: "local" },
    { id: "c", enabled: true, outcome: "local" }, { id: "disabled", enabled: false, outcome: "local" },
  ];
  const choose = vi.fn(async () => initial.view.cursor);
  const book: ReaderBook = { inspect: async () => initial, choose, checkout: async () => undefined, reload: async () => undefined, nextContentUnits: async () => [] };
  const reader = createReader(book, resolver);
  try {
    await reader.start();
    expect(await reader.choose(["a"])).toBe(false);
    expect(reader.selection(["a", "a"])?.issue).toBe("duplicate");
    expect(reader.selection(["a", "unknown"])?.issue).toBe("unknown");
    expect(reader.selection(["a", "disabled"])?.issue).toBe("disabled");
    expect(reader.toggle("disabled")).toBe(false);
    expect(reader.toggle("b")).toBe(true);
    expect(reader.toggle("a")).toBe(true);
    expect(reader.toggle("c")).toBe(false);
    expect(reader.selection()?.canSubmit).toBe(true);
    expect(await reader.choose(["b", "a"])).toBe(true);
    expect(choose).toHaveBeenCalledExactlyOnceWith(initial.view.cursor, interaction.choice_point, ["a", "b"]);
    expect(reader.getSnapshot().selected).toEqual([]);
    expect(await reader.checkout("not-in-history")).toBe(false);
    interaction.min = 0; interaction.max = 1;
    await reader.refresh();
    expect(reader.selection()?.canSubmit).toBe(true);
    expect(await reader.choose([])).toBe(true);
    expect(choose).toHaveBeenLastCalledWith(initial.view.cursor, interaction.choice_point, []);
  } finally { reader.dispose(); }
});

test("a confirmed choice and checkout match Book saves and reject overlapping or stale actions", async () => {
  const book = await openBook({ pack: demoPack, execution });
  const reference = await openBook({ pack: demoPack, execution });
  const reader = createReader(book, resolver);
  try {
    await reader.start();
    const before = screen(reader).book;
    const interaction = choice(before);
    const option = interaction.options.find(option => option.enabled);
    if (!option) throw new Error("Expected an option");
    const first = reader.choose([option.id]);
    expect(await reader.choose([option.id])).toBe(false);
    expect(await first).toBe(true);
    await reference.choose(before.view.cursor, interaction.choice_point, [option.id]);
    expect(await book.exportSave()).toBe(await reference.exportSave());
    expect(await reader.choose([option.id])).toBe(false);
    expect(await reader.checkout(before.view.cursor)).toBe(true);
    expect(screen(reader).book.view.cursor).toBe(before.view.cursor);
    expect(screen(reader).book.view.history.length).toBeGreaterThan(1);
    expect(reader.getSnapshot().save.status).toBe("memory");
  } finally { reader.dispose(); await book.close(); await reference.close(); }
});

test("save failures freeze confirmed progress until reload; superseded reload opens the newer durable cursor", async () => {
  const store = await IndexedDbStore.open(`reader-${crypto.randomUUID()}`);
  let failure: Error | undefined;
  const storage: CacheStore = { load: request => store.load(request), persist: batch => failure ? Promise.reject(failure) : store.persist(batch) };
  const book = await openBook({ pack: demoPack, execution, storage });
  const reader = createReader(book, resolver, { persistence: "durable" });
  const states: string[] = [];
  const unsubscribe = reader.subscribe(() => states.push(reader.getSnapshot().save.status));
  try {
    await reader.start();
    const initial = screen(reader).book;
    const option = choice(initial).options.find(option => option.enabled);
    if (!option) throw new Error("Expected an option");
    failure = new Error("quota");
    expect(await reader.choose([option.id])).toBe(false);
    expect(states).toContain("saving");
    expect(reader.getSnapshot().save.status).toBe("failed");
    expect(screen(reader).book.view.cursor).toBe(initial.view.cursor);
    expect(await reader.choose([option.id])).toBe(false);
    expect(await reader.refresh()).toBe(false);
    failure = undefined;
    expect(await reader.retry()).toBe(true);
    expect(states).toContain("reloading");
    expect(screen(reader).book.view.cursor).toBe(initial.view.cursor);
    const other = await openBook({ pack: demoPack, execution, storage: store });
    try {
      const point = choice(initial);
      await other.choose(initial.view.cursor, point.choice_point, [option.id]);
      expect(await reader.choose([option.id])).toBe(false);
      expect(reader.getSnapshot().save.status).toBe("superseded");
      expect(await reader.reload()).toBe(true);
      expect(screen(reader).book.view.cursor).toBe((await other.inspect()).view.cursor);
      expect(reader.getSnapshot().save.status).toBe("saved");
    } finally { await other.close(); }
    expect(new StoreSuperseded().name).toBe("StoreSuperseded");
  } finally { unsubscribe(); reader.dispose(); await book.close(); store.close(); }
});

test("prefetch follows every screen in the same context and never publishes speculative failure", async () => {
  vi.useFakeTimers();
  const book = await openBook({ pack: demoPack, execution });
  const requests: ResolveRequest[] = [];
  const next = vi.spyOn(book, "nextContentUnits");
  const reader = createReader(book, { resolve: async request => {
    requests.push(request);
    if (request.items.every(item => "unit" in item.content && !item.args)) throw new Error("offline speculation");
    return resolver.resolve(request);
  } }, { context: { realization: "chosen", languages: ["en"] } });
  try {
    await reader.start();
    const confirmed = reader.getSnapshot();
    expect(next).not.toHaveBeenCalled();
    await vi.runAllTimersAsync();
    expect(next).toHaveBeenCalledTimes(1);
    const prefetched = requests.at(-1);
    expect(prefetched?.context).toEqual(screen(reader).context);
    expect(prefetched?.items.every(item => "unit" in item.content && item.args === undefined)).toBe(true);
    expect(reader.getSnapshot()).toBe(confirmed);
    const keys = prefetched?.items.map(contentItemKey);
    expect(keys?.length).toBeGreaterThan(0);
    await reader.refresh();
    await vi.runAllTimersAsync();
    expect(next).toHaveBeenCalledTimes(2);
    reader.dispose();
    expect(await reader.refresh()).toBe(false);
  } finally { reader.dispose(); await book.close(); }
});

test("resolver count mismatches fail closed; disposal prevents a late screen and speculative reads", async () => {
  const book = await openBook({ pack: demoPack, execution });
  const reader = createReader(book, { resolve: async () => [] });
  expect(await reader.start()).toBe(false);
  expect(reader.getSnapshot()).toMatchObject({ phase: "error", screen: null });
  reader.dispose();
  let release: (() => void) | undefined;
  const wait = new Promise<void>(resolve => { release = resolve; });
  const late = createReader(book, { resolve: async request => { await wait; return resolver.resolve(request); } });
  const pending = late.start();
  await Promise.resolve();
  late.dispose(); release?.();
  expect(await pending).toBe(false);
  expect(late.getSnapshot().screen).toBeNull();
  await book.close();
});

test("a content failure after a confirmed choice retries presentation without writing that choice twice", async () => {
  const book = await openBook({ pack: demoPack, execution });
  let offline = false;
  const reader = createReader(book, { resolve: async request => {
    if (offline) throw new Error("offline");
    return resolver.resolve(request);
  } }, { persistence: "durable" });
  const chosen = vi.spyOn(book, "choose");
  let availableAtReady = false;
  reader.subscribe(() => { if (reader.getSnapshot().phase === "ready") availableAtReady = reader.selection([choice(screen(reader).book).options[0]?.id ?? ""])?.canSubmit ?? false; });
  try {
    await reader.start();
    expect(availableAtReady).toBe(true);
    const before = screen(reader).book;
    const option = choice(before).options.find(option => option.enabled);
    if (!option) throw new Error("Expected an option");
    offline = true;
    expect(await reader.choose([option.id])).toBe(false);
    expect(reader.getSnapshot()).toMatchObject({ phase: "error", save: { status: "saved" }, error: { kind: "content" } });
    expect(screen(reader).book.view.cursor).toBe(before.view.cursor);
    const confirmed = await book.exportSave();
    offline = false;
    expect(await reader.retry()).toBe(true);
    expect(screen(reader).book.view.depth).toBe(1);
    expect(chosen).toHaveBeenCalledTimes(1);
    expect(await book.exportSave()).toBe(confirmed);
  } finally { reader.dispose(); await book.close(); }
});
