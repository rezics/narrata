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

async function unlockSound(page: Page) {
  await page.getByRole("button", { name: "Turn sound on" }).click();
  await expect(page.getByRole("button", { name: "Turn sound off" })).toBeVisible();
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

async function buildNewPublicationCandidate(withFutureVoice = false): Promise<DemoCandidateTransfer> {
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
  if (withFutureVoice) {
    const line = asRecord(narrowNodes["line"]);
    const lineData = asRecord(line["data"]);
    lineData["voice"] = { kind: "audio", provider: "demo", key: "voice/future-branch", revision: "1" };
  }

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
  const sameArtifactReplacement = await authorSnapshot(page);
  expect(sameArtifactReplacement.artifact).toBe(editedCandidate.artifact);
  expect(sameArtifactReplacement.snapshot).not.toBe(editedCandidate.snapshot);

  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await waitForCandidatePreparation(page);
  expect((await page.evaluate(() => window.__vnTestHooks?.metrics.candidateQueries))).toBe(1);
  expect((await page.evaluate(() => window.__vnTestHooks?.metrics.mediaRequests.filter(item => item.key === "background/old-hall").length))).toBe(1);
  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  expect((await page.evaluate(() => window.__vnTestHooks?.metrics.mediaRequests.filter(item => item.key === "background/old-hall").length))).toBe(1);
  await page.evaluate(async () => {
    const controller = window.__vnTestHooks?.controller;
    if (!controller) throw new Error("The VN preview controller is unavailable.");
    for (let step = 0; step < 4; step++) await controller.back();
  });
  await expect(page.locator(".dialogue-copy")).toHaveText("A second draft changes the first line again.");

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

test("a replaced preview host prepares again after a same-breakpoint resize", async ({ page }) => {
  const formalBefore = await startAuthoring(page, 1586, 992);
  const dialogue = page.getByRole("textbox", { name: "Dialogue" });
  await dialogue.fill("The first isolated preview reaches the river choice.");
  await expect(page.locator(".dialogue-copy")).toHaveText("The first isolated preview reaches the river choice.");
  const firstHostId = await page.evaluate(() => window.__vnTestHooks?.metrics.audioTimelineId);
  if (!firstHostId) throw new Error("The first isolated preview host identity is unavailable.");
  await toChoice(page);
  await waitForCandidatePreparation(page);
  expect((await page.evaluate(() => window.__vnTestHooks?.metrics.candidateQueries))).toBe(1);

  await dialogue.fill("The replacement preview has its own preparation attempt.");
  await expect(page.locator(".dialogue-copy")).toHaveText("The replacement preview has its own preparation attempt.");
  await expect.poll(async () => page.evaluate(() => window.__vnTestHooks?.metrics.audioTimelineId)).not.toBe(firstHostId);
  const replacementHostId = await page.evaluate(() => window.__vnTestHooks?.metrics.audioTimelineId);
  if (!replacementHostId) throw new Error("The replacement preview host identity is unavailable.");

  await toChoice(page);
  await waitForCandidatePreparation(page);
  const beforeResize = await page.evaluate(async () => ({
    metrics: window.__vnTestHooks?.metrics,
    view: await window.__vnTestHooks?.currentView(),
  }));
  expect(beforeResize.metrics?.candidateQueries).toBe(1);
  if (!beforeResize.view) throw new Error("The replacement preview view is unavailable.");

  await page.setViewportSize({ width: 1586, height: 820 });
  await expect.poll(async () => page.evaluate(() => window.__vnTestHooks?.metrics.candidateQueries)).toBe(2);
  await waitForCandidatePreparation(page);
  const afterResize = await page.evaluate(async () => ({
    metrics: window.__vnTestHooks?.metrics,
    view: await window.__vnTestHooks?.currentView(),
  }));
  expect(afterResize.metrics?.audioTimelineId).toBe(replacementHostId);
  expect(afterResize.metrics?.candidateQueries).toBe(2);
  expect(afterResize.view?.view.cursor).toBe(beforeResize.view.view.cursor);
  expect(afterResize.view?.view.depth).toBe(beforeResize.view.view.depth);
  await page.getByRole("link", { name: "Back to story" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator(".dialogue-copy")).toHaveText("Then we will take the river road.");
  const formalAfter = await formalState(page);
  expect(formalAfter.save).toBe(formalBefore.save);
  expect(formalAfter.view?.cursor).toBe(formalBefore.view?.cursor);
  expect(formalAfter.view?.depth).toBe(formalBefore.view?.depth);
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
    path: "/assets/river-night-portrait.png",
    mimeType: "image/png",
    width: 853,
    height: 1844,
  }];
  const candidateHostId = await page.evaluate(async input => {
    const hooks = window.__vnTestHooks;
    if (!hooks) throw new Error("The VN publication probe is unavailable.");
    return hooks.openCandidate(input.candidate, input.contentDrafts, input.mediaOverrides);
  }, { candidate, contentDrafts, mediaOverrides });
  try {
    const publishedB = await page.evaluate(id => window.__vnTestHooks?.candidateState(id), candidateHostId);
    if (!publishedB) throw new Error("The live B reader probe is unavailable.");
    await mkdir(evidenceDirectory, { recursive: true });
    await writeFile(join(evidenceDirectory, "publication-b-start-state.json"), `${JSON.stringify({
      artifactId: publishedB.artifactId,
      candidateStatus: publishedB.candidateStatus,
      playerStatus: publishedB.playerStatus,
      metrics: publishedB.metrics,
    }, null, 2)}\n`);
    expect(publishedB.artifactId).toBe(candidate.artifactId);
    expect(publishedB.artifactId).not.toBe(originalArtifact);
    expect(publishedB.firstText).toBe("The new published version starts at the river.");
    expect(publishedB.atChoice).toBe(true);
    expect(publishedB.playerStatus).toBe("ready");
    expect(publishedB.candidateStatus?.status, JSON.stringify(publishedB.candidateStatus)).toBe("partial");
    expect(publishedB.candidateStatus?.completeProjection).toBe(true);
    expect(publishedB.candidateStatus?.mediaPrepared).toBe(1);
    expect(publishedB.candidateStatus?.mediaUnavailable).toBeGreaterThan(0);
    expect(publishedB.metrics.logicalRequests.every(item => item.artifact === candidate.artifactId)).toBe(true);

    const originalMetrics = await page.evaluate(() => window.__vnTestHooks?.metrics);
    if (!originalMetrics) throw new Error("The running A reader metrics are unavailable.");
    expect(publishedB.metrics.audioTimelineId).not.toBe(originalMetrics.audioTimelineId);
    for (const key of ["background/river-night", "character/mira", "music/river-road", "voice/mira-opening"]) {
      const fromA = originalMetrics.mediaRequests.find(item => item.contextScope === "demo-public" && item.key === key && item.source === "network");
      const fromB = publishedB.metrics.mediaRequests.find(item => item.contextScope === "demo-public" && item.key === key && item.source === "shared");
      expect(fromA, `A must fetch ${key} once`).toBeDefined();
      expect(fromB, `B should share only immutable bytes for ${key}`).toBeDefined();
      expect(fromB).toMatchObject({
        kind: fromA?.kind,
        provider: fromA?.provider,
        key: fromA?.key,
        revision: fromA?.revision,
        variant: fromA?.variant,
        contextScope: fromA?.contextScope,
      });
      expect(fromB?.sha256).toBe(fromA?.sha256);
      expect(fromB?.bytes).toBe(fromA?.bytes);
      expect(fromB?.networkBytes).toBe(0);
    }

    const bOldHall = publishedB.metrics.mediaRequests.filter(item => item.key === "background/old-hall" && item.contextScope === "demo-public");
    const bOldHallOne = bOldHall.find(item => item.revision === "1");
    const bOldHallTwo = bOldHall.find(item => item.revision === "2");
    expect(bOldHallOne?.source).toBe("network");
    expect(bOldHallTwo?.source).toBe("network");
    expect(bOldHallTwo?.bytes).toBeGreaterThan(0);
    expect(bOldHallTwo?.sha256).not.toBe(bOldHallOne?.sha256);
    expect(publishedB.metrics.mediaDecodes.some(item => item.phase === "candidate" && item.key === "background/old-hall" && item.revision === "1" && item.bytes === bOldHallOne?.bytes)).toBe(true);
    expect(publishedB.metrics.mediaDecodes.some(item => item.phase === "candidate" && item.key === "background/old-hall" && item.revision === "2" && item.bytes === bOldHallTwo?.bytes)).toBe(true);

    await toChoice(page);
    await waitForCandidatePreparation(page);
    const aAtChoice = await page.evaluate(() => window.__vnTestHooks?.metrics);
    if (!aAtChoice) throw new Error("A did not keep its live reader after B was created.");
    expect(aAtChoice.logicalRequests.filter(item => item.phase === "candidate").length).toBeGreaterThan(0);
    expect(aAtChoice.logicalRequests.every(item => item.artifact === originalArtifact)).toBe(true);
    const aSharedOldHall = aAtChoice.mediaRequests.find(item => item.key === "background/old-hall" && item.revision === "1" && item.contextScope === "demo-public");
    expect(aSharedOldHall?.source).toBe("shared");
    expect(aSharedOldHall?.sha256).toBe(bOldHallOne?.sha256);
    expect(aAtChoice.mediaRequests.some(item => item.key === "background/old-hall" && item.revision === "2")).toBe(false);

    await page.evaluate(async id => { await window.__vnTestHooks?.switchCandidateContext(id, "reader-two"); }, candidateHostId);
    const bSecondScope = await page.evaluate(id => window.__vnTestHooks?.candidateState(id), candidateHostId);
    if (!bSecondScope) throw new Error("The B reader context update was not observable.");
    expect(bSecondScope.playerStatus).toBe("ready");
    const aBackground = originalMetrics.mediaRequests.find(item => item.key === "background/river-night" && item.contextScope === "demo-public");
    const bBackgroundInSecondScope = bSecondScope.metrics.mediaRequests.find(item => item.key === "background/river-night" && item.revision === "1" && item.contextScope === "reader-two");
    expect(bBackgroundInSecondScope?.source).toBe("network");
    expect(bBackgroundInSecondScope?.networkBytes).toBeGreaterThan(0);
    expect(bBackgroundInSecondScope?.sha256).toBe(aBackground?.sha256);
    expect(bSecondScope.metrics.mediaRequests.some(item => item.contextScope === "reader-two" && item.source === "shared")).toBe(false);

    await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
    await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
    const aAfterB = await currentView(page);
    expect(aAfterB.view.artifact_id).toBe(originalArtifact);
    const finalAMetrics = await page.evaluate(() => window.__vnTestHooks?.metrics);
    expect(finalAMetrics?.logicalRequests.every(item => item.artifact === originalArtifact)).toBe(true);

    const sharedMediaProof = ["background/river-night", "character/mira", "music/river-road", "voice/mira-opening"].map(key => {
      const a = originalMetrics.mediaRequests.find(item => item.contextScope === "demo-public" && item.key === key && item.source === "network");
      const b = publishedB.metrics.mediaRequests.find(item => item.contextScope === "demo-public" && item.key === key && item.source === "shared");
      if (!a || !b) throw new Error(`The immutable A/B byte-sharing evidence is missing ${key}.`);
      return {
        tuple: { kind: a.kind, provider: a.provider, key: a.key, revision: a.revision, variant: a.variant, contextScope: a.contextScope },
        sharedTuple: { kind: b.kind, provider: b.provider, key: b.key, revision: b.revision, variant: b.variant, contextScope: b.contextScope },
        bytes: a.bytes,
        sha256: a.sha256,
        aSource: a.source,
        bSource: b.source,
        bNetworkBytes: b.networkBytes,
      };
    });
    const changedRevisionProof = {
      key: "background/old-hall",
      aRevision: bOldHallOne?.revision,
      aBytes: bOldHallOne?.bytes,
      aSha256: bOldHallOne?.sha256,
      bRevision: bOldHallTwo?.revision,
      bBytes: bOldHallTwo?.bytes,
      bSha256: bOldHallTwo?.sha256,
      bSource: bOldHallTwo?.source,
      bDecoded: publishedB.metrics.mediaDecodes.some(item => item.phase === "candidate" && item.key === "background/old-hall" && item.revision === "2"),
    };
    const authorizationScopeProof = {
      key: "background/river-night",
      aScope: aBackground?.contextScope,
      bOriginalScopeSource: publishedB.metrics.mediaRequests.find(item => item.key === "background/river-night" && item.contextScope === "demo-public")?.source,
      bChangedScope: bBackgroundInSecondScope?.contextScope,
      bChangedScopeSource: bBackgroundInSecondScope?.source,
      sameBytesSha256: bBackgroundInSecondScope?.sha256,
      originalBytesSha256: aBackground?.sha256,
      changedScopeUsedSeparateBytes: bSecondScope.metrics.mediaRequests.every(item => item.contextScope !== "reader-two" || item.source === "network"),
    };
    await mkdir(evidenceDirectory, { recursive: true });
    await writeFile(join(evidenceDirectory, "publication-b-metrics.json"), `${JSON.stringify({
      aArtifactId: originalArtifact,
      artifactId: bSecondScope.artifactId,
      firstText: bSecondScope.firstText,
      candidateStatus: bSecondScope.candidateStatus,
      audioTimelineIds: { a: originalMetrics.audioTimelineId, b: bSecondScope.metrics.audioTimelineId, distinct: originalMetrics.audioTimelineId !== bSecondScope.metrics.audioTimelineId },
      sharedMediaProof,
      changedRevisionProof,
      authorizationScopeProof,
      aPinnedAfterB: finalAMetrics?.logicalRequests.every(item => item.artifact === originalArtifact),
      metrics: bSecondScope.metrics,
    }, null, 2)}\n`);
  } finally {
    await page.evaluate(async id => { await window.__vnTestHooks?.closeCandidate(id); }, candidateHostId);
  }
});

test("a candidate request bound reports budget partial while the current B choice stays playable", async ({ page }) => {
  await openPlayer(page);
  const candidate = await buildNewPublicationCandidate();
  const contentDrafts: readonly (readonly [string, DemoContentDraft])[] = [["dialogue.mira.opening.b", {
    text: "The new published version starts at the river.",
    revision: "revision-b",
  }]];
  const mediaOverrides: readonly DemoMediaOverride[] = [{
    kind: "image",
    provider: "demo",
    key: "background/old-hall",
    revision: "2",
    path: "/assets/river-night-portrait.png",
    mimeType: "image/png",
    width: 853,
    height: 1844,
  }];
  const candidateHostId = await page.evaluate(async input => {
    const hooks = window.__vnTestHooks;
    if (!hooks) throw new Error("The VN budget probe is unavailable.");
    return hooks.openCandidate(input.candidate, input.contentDrafts, input.mediaOverrides, 1);
  }, { candidate, contentDrafts, mediaOverrides });
  try {
    const state = await page.evaluate(id => window.__vnTestHooks?.candidateState(id), candidateHostId);
    if (!state?.candidateStatus) throw new Error("The bounded B candidate did not expose preparation status.");
    expect(state.playerStatus).toBe("ready");
    expect(state.atChoice).toBe(true);
    expect(state.metrics.candidateBounds).toEqual({
      maxCandidateRequests: 1,
      maxRequests: 8,
      maxConcurrent: 3,
      maxEncodedBytesPerTarget: 12 * 1024 * 1024,
      maxEncodedBytesRetained: 24 * 1024 * 1024,
      maxDecodedBytesRetained: 32 * 1024 * 1024,
      maxImageDimension: 4096,
      maxAudioDurationMs: 60_000,
    });
    expect(state.candidateStatus.status).toBe("partial");
    expect(state.candidateStatus.completeProjection).toBe(true);
    expect(state.candidateStatus.projection?.complete).toBe(true);
    expect(state.candidateStatus.mediaPrepared).toBe(1);
    expect(state.candidateStatus.mediaUnavailable).toBeGreaterThan(0);
    const projectedVersions = state.candidateStatus.projection?.media
      .filter(item => item.reference.key === "background/old-hall")
      .map(item => item.reference.revision).sort() ?? [];
    expect(projectedVersions).toEqual(["1", "2"]);
    const attempted = state.metrics.mediaRequests.filter(item => item.key === "background/old-hall");
    expect(attempted).toHaveLength(1);
    expect(attempted.map(item => item.revision)).toEqual(["1"]);
    expect(attempted[0]?.status).toBe("available");
    expect(attempted.every(item => item.status === "available")).toBe(true);
    const refusedVersions = projectedVersions.filter(revision => !attempted.some(item => item.revision === revision));
    expect(refusedVersions).toEqual(["2"]);
    await page.evaluate(async id => {
      const hooks = window.__vnTestHooks;
      if (!hooks) throw new Error("The bounded B choice probe is unavailable.");
      await hooks.chooseCandidate(id);
    }, candidateHostId);
    const afterChoice = await page.evaluate(id => window.__vnTestHooks?.candidateState(id), candidateHostId);
    if (!afterChoice) throw new Error("The bounded B branch result was not observable.");
    expect(afterChoice.playerStatus).toBe("ready");
    expect(afterChoice.atChoice).toBe(false);
    expect(afterChoice.cursor).not.toBe(state.cursor);
    expect(afterChoice.metrics.readerCalls.choose).toBe(1);
    expect(afterChoice.metrics.logicalRequests.every(item => item.artifact === state.artifactId)).toBe(true);
    await mkdir(evidenceDirectory, { recursive: true });
    await writeFile(join(evidenceDirectory, "candidate-budget-metrics.json"), `${JSON.stringify({
      artifactId: state.artifactId,
      candidateBounds: state.metrics.candidateBounds,
      playerStatus: state.playerStatus,
      atChoice: state.atChoice,
      candidateStatus: state.candidateStatus,
      attempted,
      refusedVersions,
      refusedByCandidateRequestBound: { maxCandidateRequests: state.metrics.candidateBounds.maxCandidateRequests, revision: "2" },
      decodes: state.metrics.mediaDecodes,
      playableChoice: {
        playerStatus: afterChoice.playerStatus,
        artifactId: afterChoice.artifactId,
        cursorAdvanced: afterChoice.cursor !== state.cursor,
        choiceCalls: afterChoice.metrics.readerCalls.choose,
      },
    }, null, 2)}\n`);
  } finally {
    await page.evaluate(async id => { await window.__vnTestHooks?.closeCandidate(id); }, candidateHostId);
  }
});

test("candidate preparation decodes a future voice without playing it", async ({ page }) => {
  await openPlayer(page);
  await unlockSound(page);
  const candidate = await buildNewPublicationCandidate(true);
  const mediaOverrides: readonly DemoMediaOverride[] = [
    {
      kind: "image",
      provider: "demo",
      key: "background/old-hall",
      revision: "2",
      path: "/assets/river-night-portrait.png",
      mimeType: "image/png",
      width: 853,
      height: 1844,
    },
    {
      kind: "audio",
      provider: "demo",
      key: "voice/future-branch",
      revision: "1",
      path: "/assets/audio/tomas-road.wav",
      mimeType: "audio/wav",
    },
  ];
  const contentDrafts: readonly (readonly [string, DemoContentDraft])[] = [["dialogue.mira.opening.b", {
    text: "The new published version starts at the river.",
    revision: "revision-b",
  }]];
  const candidateHostId = await page.evaluate(async input => {
    const hooks = window.__vnTestHooks;
    if (!hooks) throw new Error("The future-audio candidate probe is unavailable.");
    return hooks.openCandidate(input.candidate, input.contentDrafts, input.mediaOverrides, 8, false);
  }, { candidate, contentDrafts, mediaOverrides });
  try {
    await page.evaluate(async id => {
      const hooks = window.__vnTestHooks;
      if (!hooks) throw new Error("The candidate audio controls are unavailable.");
      await hooks.unlockCandidateAudio(id);
    }, candidateHostId);
    const beforePreparation = await page.evaluate(id => window.__vnTestHooks?.candidateState(id), candidateHostId);
    if (!beforePreparation) throw new Error("The future-audio candidate did not expose its choice state.");
    expect(beforePreparation.playerStatus).toBe("ready");
    expect(beforePreparation.atChoice).toBe(true);
    expect(beforePreparation.soundStatus).toBe("on");
    const playCountBefore = beforePreparation.metrics.audioEvents.filter(item => item.event === "play").length;

    await page.evaluate(async id => {
      const hooks = window.__vnTestHooks;
      if (!hooks) throw new Error("The candidate preparation controls are unavailable.");
      await hooks.retryCandidatePreparation(id);
    }, candidateHostId);
    const prepared = await page.evaluate(id => window.__vnTestHooks?.candidateState(id), candidateHostId);
    if (!prepared?.candidateStatus) throw new Error("The future-audio preparation result was not observable.");
    expect(prepared.playerStatus).toBe("ready");
    expect(prepared.atChoice).toBe(true);
    const request = prepared.metrics.mediaRequests.find(item => item.key === "voice/future-branch");
    expect(request?.status).toBe("available");
    expect(request?.networkBytes).toBeGreaterThan(0);
    const decode = prepared.metrics.mediaDecodes.find(item => item.phase === "candidate" && item.kind === "audio" && item.key === "voice/future-branch");
    expect(decode?.bytes).toBe(request?.bytes);
    expect(prepared.metrics.audioEvents.filter(item => item.event === "play")).toHaveLength(playCountBefore);
    expect(prepared.metrics.audioEvents.filter(item => item.event === "play" && item.key === "voice/future-branch")).toHaveLength(0);
    await mkdir(evidenceDirectory, { recursive: true });
    await writeFile(join(evidenceDirectory, "candidate-audio-metrics.json"), `${JSON.stringify({
      artifactId: prepared.artifactId,
      candidateStatus: prepared.candidateStatus.status,
      atChoice: prepared.atChoice,
      bounds: prepared.metrics.candidateBounds,
      futureVoiceRequest: request,
      futureVoiceDecode: decode,
      playCountBeforePreparation: playCountBefore,
      playCountAfterPreparation: prepared.metrics.audioEvents.filter(item => item.event === "play").length,
      futureVoicePlayCount: prepared.metrics.audioEvents.filter(item => item.event === "play" && item.key === "voice/future-branch").length,
    }, null, 2)}\n`);
  } finally {
    await page.evaluate(async id => { await window.__vnTestHooks?.closeCandidate(id); }, candidateHostId);
  }
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
  await page.evaluate(async () => { await window.__vnTestHooks?.clearPreparedCaches(); });
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
  await page.evaluate(async () => { await window.__vnTestHooks?.clearPreparedCaches(); });
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
  const newer = await authorSnapshot(page);
  expect(newer.artifact).toBe(waiting.artifactId);
  expect(newer.snapshot).not.toBe(waiting.snapshotId);
  await firstGate.release();
  await page.waitForTimeout(300);
  await expect(page.locator(".dialogue-copy")).toHaveText("The newer host should become the only preview.");
  expect((await authorSnapshot(page)).snapshot).toBe(newer.snapshot);

  await page.evaluate(async () => { await window.__vnTestHooks?.clearPreparedCaches(); });
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

  await page.evaluate(async () => { await window.__vnTestHooks?.clearPreparedCaches(); });
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
