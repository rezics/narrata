import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { cp, mkdir, mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { z } from "zod";
import { checkedBook } from "../src/protocol";

const root = resolve(import.meta.dirname, "../../..");
const demo = resolve(root, "products/gamebook-demo");
const packPath = resolve(demo, "story.narpack");
const contentPath = resolve(demo, "content/zh-Hans.json");
const r1 = resolve(root, "fixtures/compat/nodes-r1");
const nativeCli = resolve(root, "target/debug", process.platform === "win32" ? "narrata-book.exe" : "narrata-book");
const exportSchema = z.object({ artifact_id: z.string(), execution: z.string(), cursor: z.string(), objects: z.array(z.string()) });

function native(...args: string[]): string { return execFileSync(nativeCli, args, { cwd: root, encoding: "utf8" }); }
function nativeBook(execution: string, actions: string) {
  const value: unknown = JSON.parse(native("run", packPath, "--execution", execution, "--actions", actions));
  return checkedBook(value).view;
}

async function open(page: Page) {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "导出存档", exact: true })).toBeEnabled();
}

async function act(page: Page, action: string, title: string) {
  await page.getByRole("button", { name: action, exact: true }).click();
  await expect(page.getByRole("heading", { name: title, exact: true, level: 1 })).toBeVisible();
  await expect(page.getByRole("button", { name: "导出存档", exact: true })).toBeEnabled();
}

async function exportSave(page: Page): Promise<string> {
  const downloading = page.waitForEvent("download");
  await page.getByRole("button", { name: "导出存档", exact: true }).click();
  const download = await downloading;
  const path = await download.path();
  if (!path) throw new Error("download path unavailable");
  return readFile(path, "utf8");
}

async function importSave(page: Page, save: string) {
  await page.getByTestId("save-file").setInputFiles({ name: "journey.save.json", mimeType: "application/json", buffer: Buffer.from(save) });
}

async function importWork(page: Page, pack: Buffer, content: string, packName = "story.narpack") {
  await page.getByTestId("work-files").setInputFiles([
    { name: packName, mimeType: "application/octet-stream", buffer: pack },
    { name: "zh-Hans.json", mimeType: "application/json", buffer: Buffer.from(content) },
  ]);
}

async function capture(page: Page, name: string) {
  const directory = process.env.NARRATA_QA_DIR;
  if (directory) { await mkdir(directory, { recursive: true }); await page.screenshot({ path: resolve(directory, name), fullPage: true }); }
}

function retitled(content: string, title: string): string {
  const value = z.object({ entries: z.record(z.string(), z.unknown()) }).passthrough().parse(JSON.parse(content));
  value.entries["product:title"] = { text: title };
  return JSON.stringify(value);
}

/** Writes `record` as the reader's IndexedDB state before the reader first opens. */
async function seed(page: Page, records: Record<string, unknown>) {
  await page.route("**/__seed", route => route.fulfill({ contentType: "text/html", body: "<!doctype html><title>seed</title>" }));
  await page.goto("/__seed");
  await page.evaluate(async values => {
    const db = await new Promise<IDBDatabase>((done, fail) => {
      const request = indexedDB.open("narrata-gamebook", 1);
      request.onupgradeneeded = () => request.result.createObjectStore("session");
      request.onsuccess = () => done(request.result);
      request.onerror = () => fail(request.error);
    });
    await new Promise<void>((done, fail) => {
      const tx = db.transaction("session", "readwrite");
      for (const [key, value] of Object.entries(values)) tx.objectStore("session").put(value, key);
      tx.oncomplete = () => done();
      tx.onerror = () => fail(tx.error);
    });
    db.close();
  }, records);
}

async function stored(page: Page): Promise<Record<string, unknown>> {
  return page.evaluate(async () => {
    const db = await new Promise<IDBDatabase>((done, fail) => {
      const request = indexedDB.open("narrata-gamebook", 1);
      request.onsuccess = () => done(request.result);
      request.onerror = () => fail(request.error);
    });
    const result: Record<string, unknown> = {};
    await new Promise<void>((done, fail) => {
      const store = db.transaction("session", "readonly").objectStore("session");
      const keys = store.getAllKeys();
      keys.onsuccess = () => {
        let left = keys.result.length;
        if (left === 0) done();
        for (const key of keys.result) {
          const value = store.get(key);
          value.onsuccess = () => { result[String(key)] = value.result instanceof Uint8Array ? `${value.result.byteLength} bytes` : value.result; if (--left === 0) done(); };
          value.onerror = () => fail(value.error);
        }
      };
      keys.onerror = () => fail(keys.error);
    });
    db.close();
    return JSON.parse(JSON.stringify(result)) as Record<string, unknown>;
  });
}

async function r1Record() {
  return {
    version: 1, revision: "0190f2a0-0000-7000-8000-000000000001", saved_at: "2026-09-01T08:00:00.000Z",
    source: await readFile(resolve(r1, "story.nar.json"), "utf8"), save: await readFile(resolve(r1, "branched.save.json"), "utf8"),
  };
}

test("three-pack story runs in Wasm and matches the native Rust commit", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await open(page);
  await expect(page).toHaveTitle("山口来信 · Narrata");
  await expect(page.getByRole("alert")).toHaveCount(0);
  await capture(page, "desktop-reader.png");
  for (const [action, title] of [
    ["到营地歇脚", "篝火夜谈"], ["询问老人留下的信", "老者的信"], ["收好信，回到篝火旁", "篝火夜谈"],
    ["吃一份干粮，在火边歇脚", "整理行囊"], ["向老人告别", "旧驿站"], ["沿山路出发", "落石滚下"],
    ["分给旅人一份干粮，请他带路", "发现小径"], ["继续前往山口", "抵达山口"], ["将老人的信交给守门人", "旅程结束"],
  ]) { if (!action || !title) throw new Error("invalid test step"); await act(page, action, title); }
  await expect(page.locator(".prose")).toContainText("这段旅程已经结束");
  await capture(page, "desktop-ending.png");
  const save = exportSchema.parse(JSON.parse(await exportSave(page)));
  const expected = nativeBook(save.execution, "camp,letter,continue,rest,continue,road,help,continue,deliver");
  expect(save.cursor).toBe(expected.cursor);
  await page.getByText("运行详情", { exact: true }).click();
  await expect(page.getByText(expected.cursor, { exact: true })).toBeVisible();
  await expect(page.getByText(expected.artifact_id, { exact: true })).toBeVisible();
  await expect(page.locator(".inspector .variable-list > div").filter({ has: page.getByText("干粮", { exact: true }) }).locator("dd")).toHaveText("1");
  await page.reload();
  await expect(page.getByRole("heading", { name: "旅程结束", exact: true })).toBeVisible();
  expect(errors).toEqual([]);
});

test("local replies rejoin the passage and a multi-select takes up to two items", async ({ page }) => {
  await open(page);
  await act(page, "翻看桌上的登记册", "驿站登记册");
  await act(page, "用炭笔写下自己的名字", "驿站登记册");
  await expect(page.locator(".prose .reply")).toHaveText("你写下自己的名字。炭笔有些钝，最后一笔拖得很长。");
  await expect(page.getByText("最多选择 2 项，也可以都不选", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: /都不选，继续/ })).toBeEnabled();
  await page.getByRole("checkbox", { name: "带上一截蜡烛", exact: true }).check();
  await page.getByRole("checkbox", { name: "带上一只空水壶", exact: true }).check();
  await expect(page.getByRole("checkbox", { name: "带上一卷麻绳", exact: true })).toBeDisabled();
  await capture(page, "desktop-multi-select.png");
  await page.getByRole("button", { name: /确定（2 项）/ }).click();
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true, level: 1 })).toBeVisible();
  await expect(page.locator(".lead-in .reply")).toHaveCount(2);
  await expect(page.locator(".lead-in")).toContainText("你合上木箱");
  await expect(page.getByRole("button", { name: "翻看桌上的登记册", exact: true })).toHaveCount(0);
  const variable = (label: string) => page.locator(".inspector .variable-list > div").filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
  await expect(variable("登记册留名")).toHaveText("是");
  await expect(variable("行囊物件")).toHaveText("2");
  await capture(page, "desktop-lead-in.png");
  const save = exportSchema.parse(JSON.parse(await exportSave(page)));
  expect(save.cursor).toBe(nativeBook(save.execution, "ledger,sign,candle+flask").cursor);
});

test("backtracking creates branches, and import restores a subgraph instance", async ({ page }) => {
  await open(page);
  await act(page, "到营地歇脚", "篝火夜谈");
  const save = await exportSave(page);
  await act(page, "上一步", "旧驿站");
  await act(page, "沿山路出发", "落石滚下");
  await expect(page.locator(".history")).toContainText("篝火夜谈");
  await expect(page.locator(".history")).toContainText("落石滚下");
  await expect(page.locator(".history .branch")).toHaveCount(2);
  await importSave(page, save);
  await expect(page.getByRole("heading", { name: "篝火夜谈", exact: true })).toBeVisible();
  await page.getByText("运行详情", { exact: true }).click();
  await expect(page.getByRole("heading", { name: "camp.visit · 实例 2", exact: true })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("heading", { name: "篝火夜谈", exact: true })).toBeVisible();
  await expect(page.locator(".prose")).toContainText("招呼过你 1 次");
});

test("locked choices and corrupt saves preserve the current story", async ({ page }) => {
  await open(page);
  await act(page, "沿山路出发", "落石滚下");
  await act(page, "独自沿旧路继续", "迷雾弥漫");
  await act(page, "走向灯火", "抵达山口");
  await expect(page.getByRole("button", { name: "将老人的信交给守门人", exact: true })).toBeDisabled();
  await expect(page.getByText("需要先从营地老人那里取到信件", { exact: true })).toBeVisible();
  const save = await exportSave(page);
  const archive = exportSchema.passthrough().parse(JSON.parse(save));
  const last = archive.objects.at(-1) ?? "";
  const flipped = `${last.slice(0, -1)}${last.endsWith("0") ? "1" : "0"}`;
  const damaged = JSON.stringify({ ...archive, objects: [...archive.objects.slice(0, -1), flipped] });
  await importSave(page, damaged);
  await expect(page.getByRole("alert")).toContainText(`objects[${archive.objects.length - 1}]`);
  await expect(page.getByRole("alert")).toContainText("digest mismatch");
  await expect(page.getByRole("heading", { name: "抵达山口", exact: true })).toBeVisible();
  expect(await exportSave(page)).toBe(save);
});

test("work imports are checked, content swaps keep saves, and saves need their artifact", async ({ page }) => {
  await open(page);
  const save = await exportSave(page);
  const pack = await readFile(packPath);
  const content = await readFile(contentPath, "utf8");
  await importWork(page, Buffer.from("not a pack"), content, "broken.narpack");
  await expect(page.getByRole("alert")).toBeVisible();
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  await page.getByTestId("work-files").setInputFiles(resolve(r1, "story.nar.json"));
  await expect(page.getByRole("alert")).toContainText("migrate-r1");
  // Text lives in the content pack: another telling of the same structure keeps every save.
  await importWork(page, pack, retitled(content, "山口来信（重述）"));
  await expect(page).toHaveTitle("山口来信（重述） · Narrata");
  await importSave(page, save);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  // A structural change is another artifact, and the save no longer fits it.
  const directory = await mkdtemp(join(tmpdir(), "narrata-reader-"));
  await cp(demo, directory, { recursive: true });
  const project = resolve(directory, "project.json");
  await writeFile(project, (await readFile(project, "utf8")).replace('"rations": 3', '"rations": 4'));
  native("compose", project, "--out", resolve(directory, "story.narpack"));
  await importWork(page, await readFile(resolve(directory, "story.narpack")), retitled(content, "另一部作品"));
  await expect(page).toHaveTitle("另一部作品 · Narrata");
  await importSave(page, save);
  await expect(page.getByRole("alert")).toContainText("incompatible_save");
  await expect(page).toHaveTitle("另一部作品 · Narrata");
  await page.reload();
  await expect(page).toHaveTitle("另一部作品 · Narrata");
});

test("two tabs cannot overwrite each other's newer persisted state", async ({ page, context }) => {
  await open(page);
  const second = await context.newPage();
  await open(second);
  const before = await exportSave(second);
  await act(page, "到营地歇脚", "篝火夜谈");
  await second.getByRole("button", { name: "沿山路出发", exact: true }).click();
  await expect(second.getByRole("alert")).toContainText("另一页面已更新");
  await expect(second.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  expect(await exportSave(second)).toBe(before);
  await second.reload();
  await expect(second.getByRole("heading", { name: "篝火夜谈", exact: true })).toBeVisible();
});

test("an R1 autosave is migrated on the default work and its record is kept", async ({ page }) => {
  const record = await r1Record();
  await seed(page, { active: record });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "旅程结束", exact: true })).toBeVisible();
  await expect(page.getByRole("status").filter({ hasText: "无法使用本机自动存档" })).toHaveCount(0);
  await expect(page.locator(".history li")).toHaveCount(15);
  const save = exportSchema.parse(JSON.parse(await exportSave(page)));
  const directory = await mkdtemp(join(tmpdir(), "narrata-reader-"));
  const cursor = native("migrate-r1", packPath, "--save", resolve(r1, "branched.save.json"), "--content", contentPath, "--execution", save.execution, "--out", resolve(directory, "migrated.json")).trim();
  expect(save.cursor).toBe(cursor);
  const values = await stored(page);
  expect(values["r1-backup"]).toEqual(record);
  expect(values.active).toMatchObject({ version: 2, artifact_id: save.artifact_id });
  await page.reload();
  await expect(page.getByRole("heading", { name: "旅程结束", exact: true })).toBeVisible();
  await expect(page.locator(".history li")).toHaveCount(15);
});

test("an R1 autosave that cannot be migrated stays untouched while the reader runs in memory", async ({ page }) => {
  const record = await r1Record();
  record.save = record.save.replace(/"artifact_id":\s*"[0-9a-f]{64}"/, `"artifact_id": "${"0".repeat(64)}"`);
  await seed(page, { active: record });
  await page.goto("/");
  await expect(page.getByRole("status").filter({ hasText: "无法使用本机自动存档" })).toContainText("incompatible_save");
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  await capture(page, "desktop-memory-mode.png");
  await act(page, "到营地歇脚", "篝火夜谈");
  expect(await stored(page)).toEqual({ active: record });
});

test("graph inspection is read-only and mobile controls remain usable", async ({ page }) => {
  await open(page);
  const before = await exportSave(page);
  await page.getByRole("button", { name: "结构图", exact: true }).click();
  await expect(page.getByRole("heading", { name: "故事结构图", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "检查 旧驿站", exact: true }).press("Enter");
  await expect(page.getByRole("heading", { name: "节点连接", exact: true })).toBeVisible();
  await expect(page.locator(".node-details")).toContainText("main.journey.station");
  await capture(page, "desktop-graph.png");
  expect(await exportSave(page)).toBe(before);
  await page.getByRole("button", { name: "返回阅读", exact: true }).click();
  await page.setViewportSize({ width: 390, height: 844 });
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  await capture(page, "mobile-reader.png");
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await page.getByRole("button", { name: /^目录/ }).click();
  await expect(page.getByRole("complementary", { name: "叙事目录" })).toBeVisible();
  await page.getByRole("button", { name: /^目录/ }).click();
  await page.getByRole("button", { name: "到营地歇脚", exact: true }).press("Enter");
  await expect(page.getByRole("heading", { name: "篝火夜谈", exact: true })).toBeFocused();
  await page.getByRole("button", { name: /^旅程/ }).click();
  await expect(page.getByRole("complementary", { name: "旅程检查器" })).toBeVisible();
});
