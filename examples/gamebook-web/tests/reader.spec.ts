import { expect, test, type Page } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { mkdir, readFile } from "node:fs/promises";
import { resolve } from "node:path";
import { z } from "zod";
import { checkedBook } from "../src/protocol";

const root = resolve(import.meta.dirname, "../../..");
const bundlePath = resolve(root, "products/gamebook-demo/story.nar.json");
const nativeCli = resolve(root, "target/debug", process.platform === "win32" ? "narrata-book.exe" : "narrata-book");

async function open(page: Page) {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "导出存档", exact: true })).toBeEnabled();
}

async function act(page: Page, action: string, title: string) {
  await page.getByRole("button", { name: action, exact: true }).click();
  await expect(page.getByRole("heading", { name: title, exact: true })).toBeVisible();
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

async function capture(page: Page, name: string) {
  const directory = process.env.NARRATA_QA_DIR;
  if (directory) { await mkdir(directory, { recursive: true }); await page.screenshot({ path: resolve(directory, name), fullPage: true }); }
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
  const nativeValue: unknown = JSON.parse(execFileSync(nativeCli, ["run", bundlePath, "--actions", "camp,letter,continue,rest,continue,road,help,continue,deliver"], { cwd: root, encoding: "utf8" }));
  const native = checkedBook({ view: nativeValue, graphs: [], diagnostics: [] }).view;
  await page.getByText("运行详情", { exact: true }).click();
  await expect(page.getByText(native.cursor, { exact: true })).toBeVisible();
  await expect(page.getByText(native.artifact_id, { exact: true })).toBeVisible();
  await expect(page.locator(".inspector .variable-list > div").filter({ has: page.getByText("干粮", { exact: true }) }).locator("dd")).toHaveText("1");
  await page.reload();
  await expect(page.getByRole("heading", { name: "旅程结束", exact: true })).toBeVisible();
  expect(errors).toEqual([]);
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
  const damaged = save.replace('"shared":{', '"shared":{"tampered":999,');
  expect(damaged).not.toBe(save);
  await importSave(page, damaged);
  await expect(page.getByRole("alert")).toContainText("snapshot");
  await expect(page.getByRole("heading", { name: "抵达山口", exact: true })).toBeVisible();
  expect(await exportSave(page)).toBe(save);
});

test("project imports are checked and exact artifact identity is required for saves", async ({ page }) => {
  await open(page);
  const save = await exportSave(page);
  await page.getByTestId("project-file").setInputFiles({ name: "invalid.json", mimeType: "application/json", buffer: Buffer.from('{"format_version":1,"format_version":2}') });
  await expect(page.getByRole("alert")).toContainText("duplicate key");
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  const source = (await readFile(bundlePath, "utf8")).replace('"title": "山口来信"', '"title": "另一部作品"');
  await page.getByTestId("project-file").setInputFiles({ name: "another.nar.json", mimeType: "application/json", buffer: Buffer.from(source) });
  await expect(page).toHaveTitle("另一部作品 · Narrata");
  await importSave(page, save);
  await expect(page.getByRole("alert")).toContainText("incompatible_save");
  await expect(page).toHaveTitle("另一部作品 · Narrata");
});

test("two tabs cannot overwrite each other's newer persisted state", async ({ page, context }) => {
  await open(page);
  const second = await context.newPage();
  await open(second);
  await act(page, "到营地歇脚", "篝火夜谈");
  await second.getByRole("button", { name: "沿山路出发", exact: true }).click();
  await expect(second.getByRole("alert")).toContainText("另一页面已更新");
  await expect(second.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  const archiveValue: unknown = JSON.parse(await exportSave(second));
  const archive = z.object({ commits: z.array(z.unknown()) }).parse(archiveValue);
  expect(archive.commits).toHaveLength(1);
  await second.reload();
  await expect(second.getByRole("heading", { name: "篝火夜谈", exact: true })).toBeVisible();
});

test("graph inspection is read-only and mobile controls remain usable", async ({ page }) => {
  await open(page);
  const before = await exportSave(page);
  await page.getByRole("button", { name: "结构图", exact: true }).click();
  await expect(page.getByRole("heading", { name: "故事结构图", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "检查 旧驿站", exact: true }).press("Enter");
  await expect(page.getByRole("heading", { name: "节点连接", exact: true })).toBeVisible();
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
