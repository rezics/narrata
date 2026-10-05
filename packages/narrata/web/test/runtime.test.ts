import "fake-indexeddb/auto";
import { expect, test } from "vitest";
import { checkedBook, openBook, type BookView } from "@rezics/narrata";
import { IndexedDbStore, StoreSuperseded, type CacheStore } from "@rezics/narrata/storage";
import { demoPack, execution, splitPack } from "./fixture.js";

function firstChoice(book: BookView) {
  const interaction = book.view.interaction;
  if (interaction.kind !== "choose") throw new Error("Expected a choice");
  const option = interaction.options.find(option => option.enabled);
  if (!option) throw new Error("No enabled option");
  return { cursor: book.view.cursor, point: interaction.choice_point, option: option.id };
}

test("manifest opening loads only the entry chunk, retries cold choices and matches full-pack saves", async () => {
  const parts = splitPack(demoPack);
  const requests: string[] = [];
  const lazy = await openBook({ manifest: parts.manifest, execution, fetchChunk: async id => {
    requests.push(id);
    const bytes = parts.objects.get(id);
    if (!bytes) throw new Error(`Missing object ${id}`);
    return bytes;
  } });
  const full = await openBook({ pack: demoPack, execution });
  try {
    expect(requests).toHaveLength(1);
    expect((await lazy.inspect()).view).toMatchObject({ depth: 0 });
    expect(requests).toHaveLength(1);
    for (const key of "camp,letter,continue,rest,continue,road,help,continue,deliver".split(",")) {
      const before = await full.inspect();
      const interaction = before.view.interaction;
      if (interaction.kind !== "choose") throw new Error("Route ended");
      const option = interaction.options.find(option => option.key === key);
      if (!option) throw new Error(`Missing route option ${key}`);
      await lazy.choose(before.view.cursor, interaction.choice_point, [option.id]);
      await full.choose(before.view.cursor, interaction.choice_point, [option.id]);
      const actual = await lazy.inspect();
      expect(actual.view.cursor).toBe((await full.inspect()).view.cursor);
      expect(actual.page).toEqual((await full.inspect()).page);
    }
    expect(requests.length).toBeGreaterThan(1);
    expect(await lazy.exportSave()).toBe(await full.exportSave());
  } finally { await lazy.close(); await full.close(); }
});

test("bad chunks and failed fetches never advance the session and can be retried", async () => {
  const parts = splitPack(demoPack);
  let damaged = false;
  await expect(openBook({ manifest: parts.manifest, execution, fetchChunk: async id => {
    const bytes = parts.objects.get(id);
    if (!bytes) throw new Error("Missing fixture");
    const corrupt = bytes.slice(); corrupt[corrupt.length - 1] = (corrupt.at(-1) ?? 0) ^ 1;
    return corrupt;
  } })).rejects.toThrow("decode");
  const book = await openBook({ manifest: parts.manifest, execution, names: parts.names, fetchChunk: async id => {
    if (damaged) throw new Error("network unavailable");
    const bytes = parts.objects.get(id);
    if (!bytes) throw new Error("Missing fixture");
    return bytes;
  } });
  try {
    const before = await book.inspect();
    const interaction = before.view.interaction;
    if (interaction.kind !== "choose") throw new Error("Expected a choice");
    const option = interaction.options.find(option => option.key === "camp");
    if (!option) throw new Error("Missing camp route");
    const pending = { cursor: before.view.cursor, point: interaction.choice_point, option: option.id };
    // An invalid choice is rejected without a commit, even after automatic loading.
    await expect(book.choose(pending.cursor, pending.point, ["option:" + "0".repeat(32)])).rejects.toThrow();
    expect((await book.inspect()).view.cursor).toBe(before.view.cursor);
    damaged = true;
    await expect(book.choose(pending.cursor, pending.point, [pending.option])).rejects.toThrow("network unavailable");
    expect((await book.inspect()).view.cursor).toBe(before.view.cursor);
    damaged = false;
    await book.choose(pending.cursor, pending.point, [pending.option]);
    expect((await book.inspect()).view.depth).toBe(1);
  } finally { await book.close(); }
});

test("IndexedDB confirms before returning, reopens, reloads and rejects another writer", async () => {
  const database = `package-${crypto.randomUUID()}`;
  const store = await IndexedDbStore.open(database);
  const secondStore = await IndexedDbStore.open(database);
  const first = await openBook({ pack: demoPack, execution, storage: store });
  const second = await openBook({ pack: demoPack, execution, storage: secondStore });
  try {
    const choice = firstChoice(await first.inspect());
    await first.choose(choice.cursor, choice.point, [choice.option]);
    await expect(second.choose(choice.cursor, choice.point, [choice.option])).rejects.toThrow();
    await second.reload();
    expect((await second.inspect()).view.cursor).toBe((await first.inspect()).view.cursor);
    const save = await first.exportSave();
    await first.checkout(choice.cursor);
    await first.restore(save);
    expect(await first.exportSave()).toBe(save);
  } finally { await first.close(); await second.close(); store.close(); secondStore.close(); }
  const reopenedStore = await IndexedDbStore.open(database);
  const reopened = await openBook({ pack: demoPack, execution, storage: reopenedStore });
  try { expect((await reopened.inspect()).view.depth).toBe(1); }
  finally { await reopened.close(); reopenedStore.close(); }
});

test("persistence errors are reported and a reload discards unconfirmed changes", async () => {
  const store = await IndexedDbStore.open(`failed-${crypto.randomUUID()}`);
  let fail = false;
  const storage: CacheStore = { load: request => store.load(request), persist: batch => {
    if (fail) return Promise.reject(new Error("quota exceeded"));
    return store.persist(batch);
  } };
  const book = await openBook({ pack: demoPack, execution, storage });
  try {
    const choice = firstChoice(await book.inspect()); fail = true;
    await expect(book.choose(choice.cursor, choice.point, [choice.option])).rejects.toThrow("quota exceeded");
    fail = false; await book.reload();
    expect((await book.inspect()).view.cursor).toBe(choice.cursor);
  } finally { await book.close(); store.close(); }
});

test("calls serialize, close releases the handle and schema parsing rejects unproved data", async () => {
  const book = await openBook({ pack: demoPack, execution });
  const choice = firstChoice(await book.inspect());
  const results = await Promise.allSettled([
    book.choose(choice.cursor, choice.point, [choice.option]),
    book.choose(choice.cursor, choice.point, [choice.option]),
  ]);
  expect(results.map(result => result.status)).toEqual(["fulfilled", "rejected"]);
  expect(() => checkedBook({ view: {}, page: [], graphs: [], diagnostics: [] })).toThrow("invalid BookView");
  expect(new StoreSuperseded()).toBeInstanceOf(Error);
  await book.close(); await book.close();
  await expect(book.inspect()).rejects.toThrow("closed");
});
