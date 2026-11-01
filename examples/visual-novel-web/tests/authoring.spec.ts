import { createHash } from "node:crypto";
import { expect, test, type Page } from "@playwright/test";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { loadVnAuthoring } from "@rezics/narrata/vn/server";
import type { DemoCandidateTransfer, DemoContentDraft, DemoMediaOverride } from "../src/player-host.js";

const screenshots = resolve(import.meta.dirname, "../../../.temp/goal/runs/vn-authoring/screenshots");
const evidenceDirectory = resolve(import.meta.dirname, "../../../.temp/goal/runs/vn-authoring");

async function openPlayer(page: Page) {
  await page.setViewportSize({ width: 1586, height: 992 });
  await page.goto("/");
  await expect(page).toHaveTitle("The River Road");
  await expect(page.getByText("The last train is already gone.", { exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Continue dialogue" })).toBeEnabled();
}

async function currentView(page: Page) {
  const view = await page.evaluate(async () => await window.__vnTestHooks?.currentView());
  if (!view) throw new Error("The VN browser probe did not expose a current view.");
  return view;
}

async function toChoice(page: Page) {
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeVisible();
}

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

async function waitForCandidatePreparation(page: Page) {
  await expect.poll(async () => page.evaluate(() => window.__vnTestHooks?.controller.getSnapshot().candidates?.status)).toBe("complete");
}

async function buildNewPublicationCandidate(): Promise<DemoCandidateTransfer> {
  const productDirectory = resolve(import.meta.dirname, "../../../products/visual-novel-demo");
  const manifest = asRecord(JSON.parse(await readFile(join(productDirectory, "project.json"), "utf8")) as unknown);
  const product = asRecord(manifest["product"]);
  const entry = asRecord(product["entry"]);
  const alias = entry["package"];
  if (typeof alias !== "string") throw new Error("The reference VN project has no entry package.");
  const packages = asRecord(manifest["packages"]);
  const packagePath = packages[alias];
  if (typeof packagePath !== "string") throw new Error("The reference VN project has no source package.");
  const source = asRecord(JSON.parse(await readFile(resolve(productDirectory, packagePath), "utf8")) as unknown);

  const graphs = asRecord(source["graphs"]);
  const start = asRecord(graphs["start"]);
  const startNodes = asRecord(start["nodes"]);
  const opening = asRecord(startNodes["mira-opening"]);
  const openingData = asRecord(opening["data"]);
  const line = asRecord(openingData["line"]);
  const contentUnit = asRecord(line["unit"]);
  contentUnit["key"] = "dialogue.mira.opening.b";

  const narrow = asRecord(graphs["narrow"]);
  const narrowNodes = asRecord(narrow["nodes"]);
  const scene = asRecord(narrowNodes["scene"]);
  const sceneData = asRecord(scene["data"]);
  const background = asRecord(sceneData["background"]);
  background["revision"] = "2";

  const authoring = await loadVnAuthoring();
  const split = authoring.splitVnSource(manifest, [[alias, source]]);
  if (!split.complete || split.diagnostics.some(item => item.severity === "error")) {
    throw new Error(`The in-memory publication candidate did not validate: ${JSON.stringify(split.diagnostics)}`);
  }
  const drafts = authoring.createVnDrafts();
  let candidate: ReturnType<typeof drafts.preview> | undefined;
  try {
    for (const record of split.records) drafts.add(record.bytes);
    const checked = drafts.check();
    if (!checked.complete || checked.diagnostics.some(item => item.severity === "error")) {
      throw new Error(`The in-memory publication candidate did not compile: ${JSON.stringify(checked.diagnostics)}`);
    }
    candidate = drafts.preview();
    const artifactId = candidate.report.artifact_id;
    const manifestBytes = candidate.manifest();
    if (!candidate.compiled || !artifactId || !manifestBytes) throw new Error("The in-memory publication candidate is incomplete.");
    const pack = candidate.pack();
    const objects = candidate.report.objects.map(id => {
      const bytes = candidate?.object(id);
      if (!bytes) throw new Error(`The in-memory publication omitted ${id}.`);
      return { id, bytes: Array.from(bytes) };
    });
    return {
      artifactId,
      manifest: Array.from(manifestBytes),
      objects,
      publication: { bytes: pack.byteLength, sha256: createHash("sha256").update(pack).digest("hex") },
    };
  } finally {
    candidate?.free();
    drafts.free();
  }
}

function asRecord(value: unknown): Record<string, unknown> {
  if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("The reference VN source has an unexpected shape.");
  return value as Record<string, unknown>;
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
  await waitForCandidatePreparation(page);
  const prepared = await page.evaluate(() => window.__vnTestHooks?.metrics.mediaRequests.filter(item => item.key === "background/old-hall").length);
  expect(prepared).toBe(1);
  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  expect(await page.evaluate(() => window.__vnTestHooks?.metrics.mediaRequests.filter(item => item.key === "background/old-hall").length)).toBe(1);
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

test("running publication A stays pinned while public pack B starts under its own identity", async ({ page }) => {
  await openPlayer(page);
  const original = await currentView(page);
  const originalArtifact = original.view.artifact_id;
  const candidate = await buildNewPublicationCandidate();
  expect(candidate.artifactId).not.toBe(originalArtifact);
  expect(candidate.publication.bytes).toBeGreaterThan(0);
  expect(candidate.publication.sha256).toMatch(/^[0-9a-f]{64}$/);

  const contentDrafts: readonly (readonly [string, DemoContentDraft])[] = [["dialogue.mira.opening.b", {
    text: "The new published version starts at the river.",
    revision: "revision-b",
  }]];
  const mediaOverrides: readonly DemoMediaOverride[] = [{
    kind: "image",
    provider: "demo",
    key: "background/old-hall",
    revision: "2",
    path: "/assets/old-hall-interior.png",
    mimeType: "image/png",
    width: 1586,
    height: 992,
  }];
  const publishedB = await page.evaluate(async input => {
    const hooks = window.__vnTestHooks;
    if (!hooks) throw new Error("The VN publication probe is unavailable.");
    return hooks.inspectCandidate(input.candidate, input.contentDrafts, input.mediaOverrides);
  }, { candidate, contentDrafts, mediaOverrides });
  expect(publishedB.artifactId).toBe(candidate.artifactId);
  expect(publishedB.artifactId).not.toBe(originalArtifact);
  expect(publishedB.firstText).toBe("The new published version starts at the river.");
  expect(publishedB.atChoice).toBe(true);
  expect(publishedB.metrics.logicalRequests.every(item => item.artifact === candidate.artifactId)).toBe(true);
  expect(publishedB.metrics.mediaRequests.filter(item => item.key === "background/old-hall").map(item => [item.revision, item.status])).toEqual([
    ["1", "available"],
    ["2", "available"],
  ]);
  expect(publishedB.metrics.mediaDecodes.some(item => item.phase === "candidate" && item.key === "background/old-hall" && item.revision === "2")).toBe(true);
  await mkdir(evidenceDirectory, { recursive: true });
  await writeFile(join(evidenceDirectory, "publication-b-metrics.json"), `${JSON.stringify({
    artifactId: publishedB.artifactId,
    firstText: publishedB.firstText,
    candidateStatus: publishedB.candidateStatus,
    metrics: publishedB.metrics,
  }, null, 2)}\n`);

  await toChoice(page);
  await waitForCandidatePreparation(page);
  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  const originalMetrics = await page.evaluate(() => window.__vnTestHooks?.metrics);
  expect(originalMetrics?.logicalRequests.length).toBeGreaterThan(1);
  expect(originalMetrics?.logicalRequests.every(item => item.artifact === originalArtifact)).toBe(true);
  expect(originalMetrics?.mediaRequests.filter(item => item.key === "background/old-hall").map(item => item.revision)).toEqual(["1"]);
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
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await waitForCandidatePreparation(page);
  await page.evaluate(async () => { await window.__vnTestHooks?.clearPreparedCaches(); });
  await page.route("**/assets/old-hall-interior.png", route => route.abort());
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
