import { type Page, expect, test } from "@playwright/test";

import type { CounterPage } from "../counter-page";

declare global {
  var counter: CounterPage;
}

async function visit(page: Page, database: string): Promise<void> {
  await page.goto(`/?db=${database}`);
  await expect(page.locator("#status")).toHaveText("ready");
}

const open = (page: Page) => page.evaluate(() => counter.open());
const advance = (page: Page, increment: number) => page.evaluate((increment) => counter.advance(increment), increment);
const save = (page: Page) => page.evaluate(() => counter.save());
const reads = (page: Page) => page.evaluate(() => counter.reads());

async function readsOf(page: Page, operation: () => Promise<unknown>) {
  const before = await reads(page);
  await operation();
  const after = await reads(page);
  return {
    loads: after.loads - before.loads,
    keys: after.keys - before.keys,
    objects: after.objects - before.objects,
    keyEntries: after.keyEntries - before.keyEntries,
    objectEntries: after.objectEntries - before.objectEntries,
  };
}

test("a session survives a reload and continues", async ({ page }) => {
  await visit(page, "reload");
  expect(await open(page)).toEqual({ ok: 0 });
  for (const increment of [1, 2, 3]) await advance(page, increment);
  await page.reload();
  await expect(page.locator("#status")).toHaveText("ready");
  expect(await open(page)).toEqual({ ok: 6 });
  expect(await advance(page, 4)).toEqual({ ok: 10 });
});

test("opening and stepping read the same few items at any depth", async ({ page }) => {
  await visit(page, "depth");
  await open(page);
  const reopen = async () => {
    await page.reload();
    await expect(page.locator("#status")).toHaveText("ready");
    return readsOf(page, () => open(page));
  };
  // A tab that created the store knows all of it; measure one that opened it.
  const shallowOpen = await reopen();
  const shallowStep = await readsOf(page, () => advance(page, 1));
  for (let step = 0; step < 40; step++) await advance(page, 1);
  const deepOpen = await reopen();
  const deepStep = await readsOf(page, () => advance(page, 1));

  expect(deepOpen).toEqual(shallowOpen);
  expect(deepOpen).toEqual({ loads: 5, keys: 2, objects: 2, keyEntries: 1, objectEntries: 0 });
  expect(deepStep).toEqual(shallowStep);
  // The first step after opening asks for its transition key, then for the sweep marker every
  // write checks, then whether its input, state and commit exist.
  expect(deepStep).toEqual({ loads: 3, keys: 2, objects: 3, keyEntries: 0, objectEntries: 0 });
});

test("another tab's write is detected and never overwritten", async ({ context }) => {
  const first = await context.newPage();
  await visit(first, "tabs");
  expect(await open(first)).toEqual({ ok: 0 });
  const second = await context.newPage();
  await visit(second, "tabs");
  expect(await open(second)).toEqual({ ok: 0 });
  expect(await save(second)).toHaveProperty("ok");

  // The first tab created the store and its step loads nothing, so the revision check of the
  // flush catches the second tab's save; the step is not persisted and the tab reloads.
  expect(await advance(first, 1)).toMatchObject({ error: "StoreSuperseded" });
  expect(await open(first)).toEqual({ ok: 0 });
  expect(await advance(first, 1)).toEqual({ ok: 1 });

  // The second tab's step loads its transition key, finds the store moved, and then its cursor
  // somewhere else than the session stands.
  expect(await advance(second, 10)).toMatchObject({ message: expect.stringContaining("session head moved") });
  expect(await open(second)).toEqual({ ok: 1 });
  expect(await advance(second, 10)).toEqual({ ok: 11 });

  // Opening again reloads, so an idle tab sees the other tab's step.
  expect(await open(first)).toEqual({ ok: 11 });
});

test("an exported store comes back after the browser drops it", async ({ page }) => {
  await visit(page, "evicted");
  await open(page);
  for (const increment of [2, 3]) await advance(page, increment);
  const exported = await page.evaluate(() => counter.export());
  await page.evaluate((exported) => counter.restore(exported), exported);
  expect(await open(page)).toEqual({ ok: 5 });
  expect(await advance(page, 1)).toEqual({ ok: 6 });
  await page.reload();
  await expect(page.locator("#status")).toHaveText("ready");
  expect(await open(page)).toEqual({ ok: 6 });
  expect(typeof (await page.evaluate(() => counter.requestPersistence()))).toBe("boolean");
});
