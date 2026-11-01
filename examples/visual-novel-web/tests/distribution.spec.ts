import { waitForPreparedBackground } from "./candidate-preparation.js";
import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { expect, test } from "@playwright/test";

const root = resolve(import.meta.dirname, "../../..");
const evidenceDirectory = resolve(root, ".temp/goal/runs/vn-player");
const screenshots = join(evidenceDirectory, "screenshots");

test("installed VN loads its entry lazily, prepares a later branch and records exact bytes", async ({ page }) => {
  await page.setViewportSize({ width: 1586, height: 992 });
  await mkdir(screenshots, { recursive: true });
  const wasmResponses: Array<{ url: string; bytes: number; sha256: string }> = [];
  const responseJobs: Promise<void>[] = [];
  const authorRequests: string[] = [];
  page.on("request", request => {
    if (/AuthoringApp|vn\/authoring|authoring-wasm/.test(request.url())) authorRequests.push(request.url());
  });
  page.on("response", response => {
    if (!/\.wasm(?:\?|$)/i.test(response.url())) return;
    responseJobs.push((async () => {
      const bytes = await response.body();
      wasmResponses.push({ url: response.url(), bytes: bytes.byteLength, sha256: createHash("sha256").update(bytes).digest("hex") });
    })());
  });

  await page.goto("/");
  await expect(page.getByText("The last train is already gone.", { exact: true })).toBeVisible();
  await expect(page.locator("[data-background='background/river-night']")).toBeVisible();
  const firstMetrics = await page.evaluate(() => window.__vnTestHooks?.metrics);
  if (!firstMetrics) throw new Error("The installed VN first-screen probe is unavailable.");
  const firstScreen = await resourceTiming(page);
  const firstWasm = firstScreen.filter(item => /\.wasm(?:[?#]|$)/i.test(item.name));
  expect(firstMetrics.logicalRequests.filter(item => item.kind === "chunk")).toHaveLength(1);
  expect(firstMetrics.logicalRequests.filter(item => item.kind === "chunk").every(item => item.phase === "required")).toBe(true);
  expect(firstMetrics.candidateQueries).toBe(0);
  expect(firstMetrics.mediaRequests.some(item => item.key === "background/old-hall")).toBe(false);
  expect(firstWasm).toHaveLength(1);
  expect(firstWasm[0]?.name).toContain("vn-reader-wasm");
  expect(firstWasm[0]?.encodedBytes).toBeGreaterThan(0);
  expect(authorRequests).toEqual([]);
  await expect.poll(() => page.evaluate(() => window.__vnTestHooks?.metrics.logicalRequests.find(item => item.kind === "manifest")?.bytes)).toBeGreaterThan(0);
  await page.screenshot({ path: join(screenshots, "installed-first-screen.png"), fullPage: true });

  await page.getByRole("button", { name: "Turn sound on" }).click();
  await expect(page.getByRole("button", { name: "Turn sound off" })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("Then we will take the river road.", { exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByText("It ends at the old hall.", { exact: true })).toBeVisible();
  const playsBeforePreparation = (await page.evaluate(() => window.__vnTestHooks?.metrics.audioEvents.filter(item => item.event === "play").length)) ?? 0;
  await page.getByRole("button", { name: "Continue dialogue" }).click();
  await expect(page.getByRole("button", { name: "Take the narrow passage.", exact: true })).toBeVisible();
  await waitForPreparedBackground(page);
  const choiceMetrics = await page.evaluate(() => window.__vnTestHooks?.metrics);
  if (!choiceMetrics) throw new Error("The installed VN candidate-preparation probe is unavailable.");
  const atChoice = await resourceTiming(page);
  const preparedBackground = choiceMetrics.mediaRequests.filter(item => item.key === "background/old-hall");
  const preparedBackgroundDecodes = choiceMetrics.mediaDecodes.filter(item => item.phase === "candidate" && item.kind === "image" && item.key === "background/old-hall");
  const candidateAudioDecodes = choiceMetrics.mediaDecodes.filter(item => item.phase === "candidate" && item.kind === "audio");
  expect(choiceMetrics.logicalRequests.some(item => item.phase === "candidate" && item.kind === "chunk")).toBe(true);
  expect(preparedBackground).toHaveLength(1);
  expect(preparedBackground[0]?.bytes).toBeGreaterThan(0);
  expect(preparedBackgroundDecodes).toHaveLength(1);
  expect(choiceMetrics.audioEvents.filter(item => item.event === "play")).toHaveLength(playsBeforePreparation);
  expect(authorRequests).toEqual([]);
  await page.screenshot({ path: join(screenshots, "installed-choice-prepared.png"), fullPage: true });

  await page.getByRole("button", { name: "Take the narrow passage.", exact: true }).click();
  await expect(page.getByText("The side door is open.", { exact: true })).toBeVisible();
  await expect(page.locator("[data-background='background/old-hall']")).toHaveJSProperty("naturalWidth", 1586);
  const afterEntry = await page.evaluate(() => window.__vnTestHooks?.metrics);
  if (!afterEntry) throw new Error("The installed VN branch-entry probe is unavailable.");
  expect(afterEntry.mediaRequests.filter(item => item.key === "background/old-hall")).toHaveLength(1);
  expect(afterEntry.mediaDecodes.filter(item => item.key === "background/old-hall")).toEqual(preparedBackgroundDecodes);
  expect(afterEntry.logicalRequests.every(item => item.artifact === firstMetrics.logicalRequests[0]?.artifact)).toBe(true);
  expect(authorRequests).toEqual([]);
  await page.screenshot({ path: join(screenshots, "installed-branch-reused.png"), fullPage: true });

  await Promise.all(responseJobs);
  expect(wasmResponses.length).toBeGreaterThan(0);
  expect(wasmResponses.every(item => item.bytes > 0 && /^[0-9a-f]{64}$/.test(item.sha256))).toBe(true);
  expect(wasmResponses.some(item => item.url.includes("vn-reader-wasm"))).toBe(true);
  expect(wasmResponses.some(item => /authoring/i.test(item.url))).toBe(false);

  const evidence = {
    interpretation: "The previous installed-host baseline is a comparison for lazy loading, not a new VN reader size ceiling.",
    wasmResponses,
    wasm: {
      firstScreen: firstWasm,
      firstScreenReaderModuleCount: firstWasm.length,
      firstScreenAuthoringModuleRequested: firstWasm.some(item => /authoring/i.test(item.name)),
    },
    phases: {
      firstScreen: firstScreen.map(timing => ({ ...timing })),
      choicePreparation: diffTiming(firstScreen, atChoice),
      preparedBranchEntry: diffTiming(atChoice, await resourceTiming(page)),
    },
    logical: {
      firstManifest: firstMetrics.logicalRequests.find(item => item.kind === "manifest"),
      firstEntryGraph: firstMetrics.logicalRequests.find(item => item.kind === "chunk"),
      candidateGraphs: choiceMetrics.logicalRequests.filter(item => item.phase === "candidate"),
    },
    media: {
      firstScreenRequests: firstMetrics.mediaRequests.map(({ role, kind, key, revision, variant, bytes, networkBytes, status }) => ({ role, kind, key, revision, variant, bytes, networkBytes, status })),
      preparedBackground: preparedBackground.map(({ role, kind, key, revision, variant, bytes, status }) => ({ role, kind, key, revision, variant, bytes, status })),
      preparedBackgroundDecodes,
      candidateAudioDecodes,
      audioPlayCountBeforePreparation: playsBeforePreparation,
      audioPlayCountAfterPreparation: choiceMetrics.audioEvents.filter(item => item.event === "play").length,
      decodeCount: afterEntry.mediaDecodes.length,
      releases: afterEntry.releases,
    },
    authoringRequestedBeforeExplicitAuthorRoute: authorRequests.length > 0,
  };
  await mkdir(evidenceDirectory, { recursive: true });
  await writeFile(join(evidenceDirectory, "distribution.json"), `${JSON.stringify(evidence, null, 2)}\n`);
});

async function resourceTiming(page: import("@playwright/test").Page) {
  return page.evaluate(() => performance.getEntriesByType("resource").map(entry => {
    const resource = entry as PerformanceResourceTiming;
    return {
      name: resource.name,
      startTimeMs: Math.round(resource.startTime * 100) / 100,
      durationMs: Math.round(resource.duration * 100) / 100,
      transferBytes: resource.transferSize,
      encodedBytes: resource.encodedBodySize,
      decodedBytes: resource.decodedBodySize,
    };
  }));
}

function diffTiming(
  before: readonly { readonly name: string; readonly startTimeMs: number; readonly durationMs: number; readonly transferBytes: number; readonly encodedBytes: number; readonly decodedBytes: number }[],
  after: readonly { readonly name: string; readonly startTimeMs: number; readonly durationMs: number; readonly transferBytes: number; readonly encodedBytes: number; readonly decodedBytes: number }[],
) {
  const previousCounts = new Map<string, number>();
  for (const item of before) previousCounts.set(item.name, (previousCounts.get(item.name) ?? 0) + 1);
  return after.filter(item => {
    const count = previousCounts.get(item.name) ?? 0;
    if (count === 0) return true;
    previousCounts.set(item.name, count - 1);
    return false;
  });
}
