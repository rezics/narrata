import { expect, test } from "@playwright/test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const root = resolve(import.meta.dirname, "../../..");

test("author view loads after reading, switches three levels, resolves titles and leaves progress untouched", async ({ page }) => {
  const requests: string[] = [];
  page.on("request", request => requests.push(request.url()));
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  expect(requests.some(url => /sigma|graphology|graph-map|\/story\.graph\//.test(url))).toBe(false);
  const title = await page.title();
  await page.getByRole("button", { name: "结构图", exact: true }).click();
  await expect(page.getByTestId("graph-level")).toHaveText("作品");
  await expect(page.locator(".author-graph canvas")).not.toHaveCount(0);
  const fetched = requests.filter(url => /\/story\.graph\/.+\.cbor/.test(url) && !url.includes("import"));
  expect(fetched).toHaveLength(2);
  expect(requests.some(url => /sigma|renderer/.test(url))).toBe(true);
  // Find the author's station unit via its published labels, without assuming layout order.
  let found = false;
  const count = await page.locator(".graph-list button").count();
  for (let row = 0; row < count; row++) {
    await page.locator(".graph-list button").nth(row).click();
    await expect(page.getByTestId("graph-level")).toHaveText("作者簇");
    await expect(page.getByTestId("graph-status")).not.toHaveText("正在载入图……");
    const station = page.getByRole("button", { name: "进入 旧驿站", exact: true });
    if (await station.count()) { await station.click(); found = true; break; }
    await page.getByRole("button", { name: "作品概览", exact: true }).click();
    await expect(page.getByTestId("graph-level")).toHaveText("作品");
  }
  expect(found).toBe(true);
  await expect(page.getByTestId("graph-level")).toHaveText("节点");
  await expect(page.getByRole("button", { name: "检查 旧驿站", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "检查 旧驿站", exact: true }).press("Enter");
  await expect(page.locator(".node-details")).toContainText("main.journey.station");
  await expect(page.getByTestId("graph-level")).toHaveText("节点");
  await page.getByRole("button", { name: "上一层", exact: true }).click();
  await expect(page.getByTestId("graph-level")).toHaveText("作者簇");
  await page.getByRole("button", { name: "返回阅读", exact: true }).click();
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  expect(await page.title()).toBe(title);
  expect(errors).toEqual([]);
});

test("framework-free mount enforces drawing limits in WebGL and retains the overview", async ({ page }) => {
  const directory = resolve(root, "fixtures/compat/graph-v1");
  const manifest: { index_object_id: string } = JSON.parse(readFileSync(resolve(directory, "manifest.json"), "utf8"));
  await page.route("**/graph-test/*.cbor", async route => {
    const filename = new URL(route.request().url()).pathname.split("/").at(-1)!;
    await route.fulfill({ contentType: "application/octet-stream", body: readFileSync(resolve(directory, filename)) });
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  const mounted = await page.evaluateHandle(async ({ moduleUrl, id }) => {
    const { mountAuthorGraph } = await import(/* @vite-ignore */ moduleUrl);
    const container = document.createElement("div"); container.id = "graph-test"; container.style.height = "480px";
    document.body.append(container);
    const result = mountAuthorGraph(container, { indexId: id, maxNodes: 6, maxEdges: 150000,
      fetchObject: async (id: string) => new Uint8Array(await (await fetch(`/graph-test/${id}.cbor`)).arrayBuffer()),
      resolver: { resolve: async (request: { items: unknown[] }) => request.items.map(() => ({ status: "unavailable" })) } });
    await result.ready;
    return result;
  }, { moduleUrl: `/@fs/${root.replaceAll("\\", "/")}/packages/narrata/web/dist/graph/index.js`, id: manifest.index_object_id });
  try {
    expect(await mounted.evaluate(value => value.controller.getSnapshot().screen?.level)).toBe("work");
    await expect(page.locator("#graph-test canvas")).not.toHaveCount(0);
    // The frozen corpus's first partition contains eight rows including ghosts.
    expect(await mounted.evaluate(async value => { await value.controller.enterCluster(0); return value.controller.enterPartition(0, 0); })).toBe(false);
    expect(await mounted.evaluate(value => ({ limited: value.controller.getSnapshot().limited, level: value.controller.getSnapshot().screen?.level, nodes: value.controller.getSnapshot().screen?.nodes.length }))).toMatchObject({ limited: true, level: "cluster" });
    expect(await mounted.evaluate(value => value.controller.getSnapshot().screen.nodes.length)).toBeLessThanOrEqual(6);
  } finally { await mounted.evaluate(value => value.dispose()); await mounted.dispose(); }
});

test("corrupt lazy tiles surface a checked error and retain the prior level", async ({ page }) => {
  await page.goto("/");
  await expect(page.getByRole("heading", { name: "旧驿站", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "结构图", exact: true }).click();
  await expect(page.getByTestId("graph-level")).toHaveText("作品");
  await page.route("**/story.graph/*.cbor*", async route => {
    const filename = new URL(route.request().url()).pathname.split("/").at(-1)!;
    const bytes = Buffer.from(readFileSync(resolve(root, "products/gamebook-demo/story.graph", filename))); bytes[bytes.length - 1]! ^= 1;
    await route.fulfill({ contentType: "application/octet-stream", body: bytes });
  });
  await page.locator(".graph-list button").first().click();
  await expect(page.getByRole("alert")).toContainText("payload SHA-256 mismatch");
  await expect(page.getByTestId("graph-level")).toHaveText("作品");
});
