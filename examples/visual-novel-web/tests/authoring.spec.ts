import { expect, test, type Page } from "@playwright/test";
import { mkdir } from "node:fs/promises";
import { join, resolve } from "node:path";

const screenshots = resolve(import.meta.dirname, "../../../.temp/goal/runs/vn-authoring/screenshots");

async function startAuthoring(page: Page, width: number, height: number) {
  await page.setViewportSize({ width, height });
  const authorRequests: string[] = [];
  page.on("request", request => {
    if (/AuthoringApp|vn\/authoring|authoring-wasm/.test(request.url())) authorRequests.push(request.url());
  });
  await page.goto("/");
  await expect(page).toHaveTitle("The River Road");
  await expect(page.locator(".dialogue-copy")).toHaveText("The last train is already gone.");
  expect(authorRequests.some(url => /AuthoringApp|authoring-wasm/.test(url))).toBe(false);
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  const saved = await formalState(page);
  await page.getByRole("link", { name: "Edit story" }).click();
  await expect(page).toHaveURL(/\/author$/);
  await expect(page.getByRole("heading", { name: "Edit the opening" })).toBeVisible();
  await expect(page.locator(".dialogue-copy")).toHaveText("The last train is already gone.");
  await expect.poll(async () => authorRequests.some(url => /AuthoringApp|authoring-wasm/.test(url))).toBe(true);
  return saved;
}

async function formalState(page: Page) {
  return page.evaluate(async () => ({
    save: await window.__vnTestHooks?.save(),
    view: (await window.__vnTestHooks?.currentView())?.view,
  }));
}

async function authorSnapshot(page: Page) {
  return page.evaluate(() => ({
    artifact: window.__vnAuthorHooks?.candidate(),
    snapshot: window.__vnAuthorHooks?.snapshot(),
  }));
}

async function authorJobs(page: Page) {
  const jobs = await page.evaluate(() => window.__vnAuthorHooks?.jobs());
  if (!jobs) throw new Error("The VN author lifecycle probe is unavailable.");
  return jobs;
}

async function capture(page: Page, name: string) {
  await mkdir(screenshots, { recursive: true });
  await page.screenshot({ path: join(screenshots, name), fullPage: true });
}

async function playNarrowBranchToEnding(page: Page) {
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  await capture(page, "desktop-branch.png");
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("We found the same door from different sides.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect.poll(async () => (await page.evaluate(async () => await window.__vnTestHooks?.currentView()))?.view.interaction.kind).toBe("finished");
  await expect(page.getByRole("heading", { name: "The end", exact: true })).toBeVisible();
}

test("desktop author preview edits host content, plays a branch, recovers, and preserves saved reading", async ({ page }) => {
  const saved = await startAuthoring(page, 1586, 992);
  const originalCandidate = await authorSnapshot(page);
  const dialogue = page.getByRole("textbox", { name: "Dialogue" });
  await dialogue.fill("The station clock has stopped, but the river is still moving.");
  await page.getByLabel("Speaker").selectOption("speaker.tomas");
  await expect(page.locator(".dialogue-copy")).toHaveText("The station clock has stopped, but the river is still moving.");
  await expect(page.locator(".speaker-name")).toHaveText("Tomas");
  await expect.poll(async () => (await authorSnapshot(page)).artifact).toMatch(/^artifact:[0-9a-f]{64}$/);
  await expect.poll(async () => authorSnapshot(page)).not.toEqual(originalCandidate);
  const editedCandidate = await authorSnapshot(page);
  expect(editedCandidate.artifact).not.toBe(originalCandidate.artifact);
  await capture(page, "desktop-editing.png");

  await playNarrowBranchToEnding(page);
  await capture(page, "desktop-ending.png");
  const beforeTextEdit = await authorSnapshot(page);
  await dialogue.fill("A second draft changes the first line again.");
  await expect(page.locator(".dialogue-copy")).toHaveText("A second draft changes the first line again.");
  await expect.poll(async () => authorSnapshot(page)).not.toEqual(beforeTextEdit);

  await dialogue.fill("");
  await expect(page.locator(".author-status")).toContainText("Add dialogue text");
  await expect(page.locator(".dialogue-copy")).toHaveText("A second draft changes the first line again.");
  await dialogue.fill("The editor recovered with a new valid line.");
  await expect(page.locator(".dialogue-copy")).toHaveText("The editor recovered with a new valid line.");

  await page.getByRole("link", { name: "Back to story" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator(".dialogue-copy")).toHaveText("Then we will take the river road.");
  await expect(page.getByTestId("save-status")).toHaveText("Saved");
  const restored = await formalState(page);
  expect(restored.save).toBe(saved.save);
  expect(restored.view?.cursor).toBe(saved.view?.cursor);
  expect(restored.view?.depth).toBe(saved.view?.depth);
  expect(restored.view?.artifact_id).toBe(saved.view?.artifact_id);

  await page.getByRole("link", { name: "Edit story" }).click();
  await expect(page.getByRole("heading", { name: "Edit the opening" })).toBeVisible();
  await expect(page.locator(".dialogue-copy")).toHaveText("The last train is already gone.");
  await capture(page, "desktop-reopened.png");
});

test("390px author preview stays usable, discards a late candidate, and recovers from resource failure", async ({ page }) => {
  let delayNextImage = false;
  let delayed = false;
  let signalDelayed!: () => void;
  const delayedRequest = new Promise<void>(resolveDelay => { signalDelayed = resolveDelay; });
  await page.route("**/assets/river-night-portrait.png", async route => {
    if (delayNextImage && !delayed) {
      delayed = true;
      signalDelayed();
      await new Promise(resolveDelay => setTimeout(resolveDelay, 650));
    }
    await route.continue();
  });
  await startAuthoring(page, 390, 844);
  const dialogue = page.getByRole("textbox", { name: "Dialogue" });
  const firstEdit = "This slower draft must not replace the newer text.";
  const latestEdit = "The latest phone draft is the one on stage.";
  delayNextImage = true;
  await dialogue.fill(firstEdit);
  await delayedRequest;
  await dialogue.fill(latestEdit);
  await page.getByLabel("Speaker").selectOption("speaker.tomas");
  await expect(page.locator(".dialogue-copy")).toHaveText(latestEdit);
  await expect(page.locator(".speaker-name")).toHaveText("Tomas");
  await page.waitForTimeout(750);
  await expect(page.locator(".dialogue-copy")).toHaveText(latestEdit);
  await expect(page.locator(".dialogue-copy")).not.toHaveText(firstEdit);
  const latestCandidate = await authorSnapshot(page);
  expect(latestCandidate.artifact).toMatch(/^artifact:[0-9a-f]{64}$/);

  const panel = await page.locator(".author-panel").boundingBox();
  expect(panel).not.toBeNull();
  if (!panel) throw new Error("The phone editor panel should have visible bounds.");
  expect(panel.width).toBeLessThanOrEqual(390);
  expect(panel.x).toBeGreaterThanOrEqual(0);
  expect(panel.y).toBeGreaterThanOrEqual(0);
  expect(panel.y + panel.height).toBeLessThanOrEqual(844);
  for (const name of ["Refresh preview", "Close preview"]) {
    const bounds = await page.getByRole("button", { name }).boundingBox();
    expect(bounds).not.toBeNull();
    if (!bounds) throw new Error(`${name} should have visible phone bounds.`);
    expect(bounds.height).toBeGreaterThanOrEqual(44);
    expect(bounds.y).toBeGreaterThanOrEqual(panel.y);
    expect(bounds.y + bounds.height).toBeLessThanOrEqual(panel.y + panel.height);
  }
  await capture(page, "phone-editing.png");

  await dialogue.fill("");
  await expect(page.locator(".author-status")).toContainText("Add dialogue text");
  await expect(page.locator(".dialogue-copy")).toHaveText(latestEdit);
  await dialogue.fill("The mobile draft is ready for another preview.");
  await expect(page.locator(".dialogue-copy")).toHaveText("The mobile draft is ready for another preview.");
  await page.getByRole("button", { name: "Hide" }).click();
  await expect(page.getByRole("button", { name: "Edit draft" })).toBeVisible();
  await page.route("**/assets/old-hall-interior.png", route => route.abort());
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Scene unavailable" })).toBeVisible();
  await capture(page, "phone-resource-unavailable.png");
  await page.unroute("**/assets/old-hall-interior.png");
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  await capture(page, "phone-branch-recovered.png");
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("We found the same door from different sides.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect.poll(async () => (await page.evaluate(async () => await window.__vnTestHooks?.currentView()))?.view.interaction.kind).toBe("finished");
  await expect(page.getByRole("heading", { name: "The end", exact: true })).toBeVisible();
  await capture(page, "phone-ending.png");
});

test("pending author hosts close before held media is released and cannot reinstall", async ({ page }) => {
  let nextGate: { readonly started: Promise<void>; readonly release: () => void; readonly markStarted: () => void; readonly wait: Promise<void> } | undefined;
  await page.route("**/assets/river-night-wide.png", async route => {
    const gate = nextGate;
    if (gate) {
      nextGate = undefined;
      gate.markStarted();
      await gate.wait;
    }
    try { await route.continue(); } catch { /* Closing a preview may abort the browser request. */ }
  });
  await startAuthoring(page, 1586, 992);
  const dialogue = page.getByRole("textbox", { name: "Dialogue" });
  const firstGate = deferredMedia();
  nextGate = firstGate;
  await dialogue.fill("This candidate waits for its scene image.");
  await firstGate.started;
  await expect.poll(async () => (await authorJobs(page)).at(-1)?.status).toBe("preparing");
  const waiting = (await authorJobs(page)).at(-1);
  if (!waiting) throw new Error("The delayed candidate was not recorded.");

  await dialogue.fill("The newer host should become the only preview.");
  await expect(page.locator(".dialogue-copy")).toHaveText("The newer host should become the only preview.");
  await expect.poll(async () => (await authorJobs(page)).find(job => job.snapshotId === waiting.snapshotId)?.status).toBe("closed");
  const closedBeforeRelease = (await authorJobs(page)).find(job => job.snapshotId === waiting.snapshotId);
  expect(closedBeforeRelease).toMatchObject({ closeStarted: true, readerClosed: true, candidateFreed: true, draftsFreed: true, installed: false });
  expect(closedBeforeRelease?.aborts).toBeGreaterThan(0);
  expect(closedBeforeRelease?.releases).toBeGreaterThan(0);
  const newer = await authorSnapshot(page);
  expect(newer.artifact).toBe(waiting.artifactId);
  expect(newer.snapshot).not.toBe(waiting.snapshotId);
  await firstGate.release();
  await page.waitForTimeout(300);
  await expect(page.locator(".dialogue-copy")).toHaveText("The newer host should become the only preview.");
  expect((await authorSnapshot(page)).snapshot).toBe(newer.snapshot);

  const blankGate = deferredMedia();
  nextGate = blankGate;
  await dialogue.fill("This pending edit is canceled by empty input.");
  await blankGate.started;
  await expect.poll(async () => (await authorJobs(page)).at(-1)?.status).toBe("preparing");
  const blanked = (await authorJobs(page)).at(-1);
  if (!blanked) throw new Error("The blank-input candidate was not recorded.");
  await dialogue.fill("");
  await expect(page.locator(".author-status")).toContainText("Add dialogue text");
  await expect(page.locator(".dialogue-copy")).toHaveText("The newer host should become the only preview.");
  await expect.poll(async () => (await authorJobs(page)).find(job => job.snapshotId === blanked.snapshotId)?.status).toBe("closed");
  expect((await authorJobs(page)).find(job => job.snapshotId === blanked.snapshotId)).toMatchObject({
    closeStarted: true, readerClosed: true, candidateFreed: true, draftsFreed: true, installed: false,
  });
  await blankGate.release();
  await page.waitForTimeout(300);
  expect((await authorSnapshot(page)).snapshot).toBe(newer.snapshot);
  await dialogue.fill("A valid edit after empty input can still preview.");
  await expect(page.locator(".dialogue-copy")).toHaveText("A valid edit after empty input can still preview.");

  const closeGate = deferredMedia();
  nextGate = closeGate;
  await dialogue.fill("This host is closed while its image is still pending.");
  await closeGate.started;
  await expect.poll(async () => (await authorJobs(page)).at(-1)?.status).toBe("preparing");
  const closing = (await authorJobs(page)).at(-1);
  if (!closing) throw new Error("The close-test candidate was not recorded.");
  await page.getByRole("button", { name: "Close preview" }).click();
  await expect(page.locator(".dialogue-copy")).toHaveCount(0);
  await expect.poll(async () => (await authorJobs(page)).find(job => job.snapshotId === closing.snapshotId)?.status).toBe("closed");
  const closed = (await authorJobs(page)).find(job => job.snapshotId === closing.snapshotId);
  expect(closed).toMatchObject({ closeStarted: true, readerClosed: true, candidateFreed: true, draftsFreed: true, installed: false });
  expect(closed?.aborts).toBeGreaterThan(0);
  expect(closed?.releases).toBeGreaterThan(0);
  await closeGate.release();
  await page.waitForTimeout(300);
  await expect(page.locator(".dialogue-copy")).toHaveCount(0);
  expect(await authorSnapshot(page)).toEqual({ artifact: undefined, snapshot: undefined });

  await page.getByRole("button", { name: "Refresh preview" }).click();
  await expect(page.locator(".dialogue-copy")).toHaveText("This host is closed while its image is still pending.");
});

function deferredMedia() {
  let markStarted!: () => void;
  let release!: () => void;
  const started = new Promise<void>(resolveStarted => { markStarted = resolveStarted; });
  const wait = new Promise<void>(resolveWait => { release = resolveWait; });
  return { started, wait, markStarted, release };
}
