import { strict as assert } from "node:assert";
import { createHash } from "node:crypto";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import type { mountOnlineReference } from "../src/online.tsx";
import type { OnlineSession } from "@rezics/narrata/reader";
import type { createReferenceHost } from "./reference-host.ts";

declare global {
  interface Window {
    referenceJourney: Awaited<ReturnType<typeof mountOnlineReference>>;
    oldReferenceSession: OnlineSession;
    referenceLate: Promise<boolean>;
  }
}

/** Outcome matrix on the same public-authored story and visible reference controls. */
export async function verifyUpdateScenarios(page: Page, origin: string, directory: string, options: { mobile?: boolean; readerWasm?: { bytes: number; sha256: string }; authoringWasm?: { bytes: number; sha256: string } } = {}) {
  await mkdir(directory, { recursive: true });
  const mobile = options.mobile ?? false;
  await page.setViewportSize(mobile ? { width: 390, height: 844 } : { width: 1280, height: 900 });
  const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
  const requests: string[] = [], binaries: Promise<{ url: string; status: number; bytes: number; sha256: string }>[] = [];
  page.on("request", request => requests.push(new URL(request.url()).pathname));
  page.on("response", response => {
    if (/\.(cbor|wasm)$/.test(new URL(response.url()).pathname)) binaries.push(response.body().then(bytes => ({ url: response.url(), status: response.status(), bytes: bytes.length, sha256: sha256(bytes) }))
      .catch(() => ({ url: response.url(), status: response.status(), bytes: 0, sha256: sha256(new Uint8Array()) })));
  });
  const evidence: unknown[] = [];
  const button = (name: string) => page.getByRole("button", { name, exact: true });
  const click = async (name: string) => { await button(name).click(); };
  const snapshot = () => page.evaluate(() => window.referenceJourney.getSnapshot().binding.session.getSnapshot());
  const writes = () => page.evaluate(() => window.referenceJourney.evidence().writes);
  const ready = async (label = "reader ready") => {
    try {
      await page.waitForFunction(() => window.referenceJourney?.getSnapshot().binding.session.getSnapshot().status === "saved"
        && window.referenceJourney?.getSnapshot().binding.controller.getSnapshot().phase === "ready", undefined, { timeout: 20_000 });
      await page.locator("article h1").waitFor();
    } catch (error) {
      const state = await page.evaluate(() => {
        const host = window.referenceJourney, session = host?.getSnapshot().binding.session.getSnapshot(), reader = host?.getSnapshot().binding.controller.getSnapshot();
        return { publication: host?.getSnapshot().publication, target: host?.getSnapshot().targetArtifact, save: session?.status, artifact: session?.progress?.view.artifact_id,
          cursor: session?.progress?.view.cursor, phase: reader?.phase, readerSave: reader?.save.status, readerError: reader?.error,
          heading: document.querySelector("article h1")?.textContent, body: document.querySelector("article")?.textContent?.slice(-600) };
      });
      throw new Error(`${label} did not settle: ${JSON.stringify(state)}; ${error instanceof Error ? error.message : "check failed"}`);
    }
  };
  const details = async (name: string) => {
    const element = page.locator("details").filter({ has: page.getByText(name, { exact: true }) });
    if (!await element.evaluate(element => element.hasAttribute("open"))) await page.getByText(name, { exact: true }).click();
  };
  const prepare = async (at: "keeper" | "dawn" = "dawn") => {
    await details("Publication and recovery scenarios"); await click("Use original publication");
    await click("Start a new journey"); await click("Accept the signal lantern"); await click("Visit the keeper with your pass");
    if (at === "dawn") await click("Return to the morning gate");
    await ready();
    return (await snapshot()).progress!;
  };
  const contentOverlap = async () => {
    const beforeA = await prepare("keeper");
    await click("Publish B"); await button("Return to the morning gate").waitFor();
    let release!: () => void, arrived!: () => void;
    const gate = new Promise<void>(done => { release = done; }), requested = new Promise<void>(done => { arrived = done; });
    await page.route("**/reference-host/content", async route => {
      const payload: unknown = route.request().postDataJSON();
      const items = typeof payload === "object" && payload !== null && "items" in payload && Array.isArray(payload.items) ? payload.items : [];
      const dawn = items.some((item: unknown) => typeof item === "object" && item !== null && "content" in item && typeof item.content === "object"
        && item.content !== null && "unit" in item.content && typeof item.content.unit === "object" && item.content.unit !== null && "key" in item.content.unit
        && item.content.unit.key === "dawn" && "args" in item && typeof item.args === "object" && item.args !== null && "signal" in item.args);
      if (dawn) {
        const response = await route.fetch(); arrived(); await gate; await route.fulfill({ response });
      } else await route.continue();
    });
    try {
      await click("Return to the morning gate"); await requested;
      await page.waitForFunction(() => window.referenceJourney.getSnapshot().binding.session.getSnapshot().status === "saved"
        && window.referenceJourney.getSnapshot().binding.controller.getSnapshot().phase === "loading");
      const savedA = await snapshot(); assert.notEqual(savedA.progress?.view.cursor, beforeA.view.cursor);
      assert.equal(savedA.progress?.view.artifact_id, beforeA.view.artifact_id);
      await page.screenshot({ path: join(directory, "reader-content-pending-a.png"), fullPage: true });
      assert.equal(await button("Review story update").isDisabled(), true);
      assert.equal(await button("Publish C").isDisabled(), true);
      release(); await page.waitForFunction(() => window.referenceJourney.getSnapshot().binding.controller.getSnapshot().phase === "ready");
      await page.getByText("At dawn, the lantern Mara accepted on the platform lit the safe road home.", { exact: true }).waitFor();
    } finally { release(); await page.unroute("**/reference-host/content"); }
    await review("automatic");
    // While a visible migration write is pending, Reader controls remain disabled.
    const writesBeforeMigration = (await writes()).length;
    await details("Simulate a save failure"); await click("Delay the next save response");
    await click("Apply story update"); await page.waitForFunction(() => window.referenceJourney.hasLate());
    assert.equal(await button("Wait beside the river").isDisabled(), true);
    assert.equal((await snapshot()).status, "saving");
    await page.screenshot({ path: join(directory, "reader-disabled-during-update.png"), fullPage: true });
    await click("Release delayed save responses"); await ready();
    await page.getByRole("heading", { name: "At the morning gate", exact: true }).waitFor();
    const migrated = await snapshot(); assert.equal(migrated.progress?.view.artifact_id, (await (await page.request.get(`${origin}/reference-host/evidence`)).json()).b.artifactId);
    const writesAfterMigration = await writes();
    assert.equal(writesAfterMigration.length, writesBeforeMigration + 1, "The confirmed migration is persisted once before the refreshed reader is shown");
    const identity = await page.evaluate(() => {
      const { controller, session } = window.referenceJourney.getSnapshot().binding;
      const mounted = controller.getSnapshot().screen?.book.view, progress = session.getSnapshot().progress?.view;
      return {
        mounted: mounted ? { cursor: mounted.cursor, artifact_id: mounted.artifact_id } : null,
        progress: progress ? { cursor: progress.cursor, artifact_id: progress.artifact_id } : null,
        choices: mounted?.interaction.kind === "choose" ? mounted.interaction.options.map(option => ({ id: option.id, enabled: option.enabled })) : [],
      };
    });
    assert.deepEqual(identity.mounted, identity.progress, "The displayed B reader screen matches the durable B cursor and artifact");
    const bChoice = button("Take the marked road home"); await bChoice.waitFor();
    assert.equal(await bChoice.isEnabled(), true, "The mounted B choice is actionable");
    await bChoice.click(); await ready();
    await page.locator("article").getByRole("heading", { name: "A light in the window", exact: true }).waitFor();
    const continued = await snapshot(), writesAfterChoice = await writes();
    assert.equal(continued.progress?.view.artifact_id, migrated.progress?.view.artifact_id, "Continuing remains in the exact B artifact");
    assert.equal(writesAfterChoice.length, writesAfterMigration.length + 1, "Taking the B choice makes one choice save without repeating migration");
    evidence.push({ kind: "delayed-content-update", identity, migrated: migrated.progress?.view, continued: continued.progress?.view,
      writesBeforeMigration, writesAfterMigration: writesAfterMigration.length, writesAfterChoice: writesAfterChoice.length });
    assert.equal(await button("Wait beside the river").count(), 0);
    assert.deepEqual(errors, []);
    return migrated.progress!;
  };
  const review = async (kind: string, reason?: string) => {
    await ready(); const before = await snapshot(), count = (await writes()).length;
    await click("Review story update");
    const region = page.getByRole("region", { name: "Story update review" }); await region.waitFor();
    assert.equal(await region.getAttribute("data-assessment"), kind);
    if (reason) assert.equal(await region.getAttribute("data-reason"), reason);
    assert.equal((await snapshot()).progress?.view.cursor, before.progress?.view.cursor);
    assert.equal((await writes()).length, count, "A visible report never publishes an update before approval");
    evidence.push({ kind, reason, before });
  };
  const map = async () => {
    const reply = page.waitForResponse(response => response.url().endsWith("/reference-host/reader-map"));
    await click("Show revealed map"); await page.getByTestId("reader-map").waitFor();
    const value: unknown = await (await reply).json();
    assert.ok(typeof value === "object" && value !== null && "artifact_id" in value && "sources" in value && Array.isArray(value.sources)
      && "units" in value && Array.isArray(value.units));
    const current = (await snapshot()).progress!.view;
    assert.equal(value.artifact_id, current.artifact_id);
    const source: unknown = value.sources[0];
    assert.ok(typeof source === "object" && source !== null && "cursor" in source && source.cursor === current.cursor && "frontier" in source && Array.isArray(source.frontier));
    if (current.interaction.kind === "choose") {
      // The checked map withholds frontier claims when the shallow ancestor needed
      // to audit through this cursor is absent, even though the page itself is readable.
      const expected = "complete" in source && source.complete === true ? current.interaction.options.map(option => option.id).sort() : [];
      const actual = source.frontier.map((item: unknown) => {
        assert.ok(typeof item === "object" && item !== null && "option" in item && typeof item.option === "string"); return item.option;
      }).sort();
      assert.deepEqual(actual, expected, "The returned frontier belongs to the actual checked-out interaction");
    }
    const units = value.units.map((unit: unknown) => {
      assert.ok(typeof unit === "object" && unit !== null && "content" in unit && typeof unit.content === "object" && unit.content !== null
        && "key" in unit.content && typeof unit.content.key === "string"); return unit.content.key;
    }).sort();
    assert.equal(units.includes("river"), false, "Unvisited river content is never disclosed");
    evidence.push({ map: value, current });
    return { source, units };
  };
  await page.goto(`${origin}/?reference`); await ready();
  await contentOverlap();
  await click("Reopen this journey"); await ready();
  await click("Use original publication"); await click("Start a new journey");
  await click("Accept the signal lantern"); await click("Visit the keeper with your pass"); await click("Return to the morning gate"); await ready();
  await click("Wait beside the river"); await ready();
  const riverFailureBaseline = await snapshot(), riverEdition = await page.evaluate(() => window.referenceJourney.getSnapshot().targetArtifact);
  assert.equal(riverFailureBaseline.progress?.view.artifact_id, (await (await page.request.get(`${origin}/reference-host/evidence`)).json()).a.artifactId);
  await click("Publish delayed-content edition"); await page.getByText(/Current publication: C/).waitFor();
  await review("needs_confirmation", "divergence");
  let blockedRefresh!: () => void, refreshRequested!: () => void;
  const releaseRefresh = new Promise<void>(done => { blockedRefresh = done; }), refreshSeen = new Promise<void>(done => { refreshRequested = done; });
  await page.route("**/reference-host/content", async route => {
    const payload: unknown = route.request().postDataJSON();
    if (typeof payload === "object" && payload !== null && "context" in payload && typeof payload.context === "object" && payload.context !== null
      && "realization" in payload.context && payload.context.realization === riverEdition) {
      refreshRequested(); await releaseRefresh;
      await route.fulfill({ status: 503, contentType: "text/plain", body: "Reader content temporarily unavailable" });
    } else await route.continue();
  });
  await click("Confirm story changes"); await refreshSeen;
  await page.waitForFunction(() => window.referenceJourney.getSnapshot().binding.session.getSnapshot().status === "saved"
    && window.referenceJourney.getSnapshot().binding.controller.getSnapshot().phase === "loading");
  const pendingRefresh = await snapshot();
  assert.notEqual(pendingRefresh.progress?.view.artifact_id, riverFailureBaseline.progress?.view.artifact_id);
  assert.equal(await page.getByText("Story update saved.", { exact: true }).count(), 0);
  assert.equal(await button("Review story update").isDisabled(), true, "A saved migration cannot be reviewed while its reader screen is loading");
  const pendingChoice = button("Take the marked road home");
  if (await pendingChoice.count()) assert.equal(await pendingChoice.isDisabled(), true, "A reader choice stays disabled while its content is loading");
  await page.screenshot({ path: join(directory, "confirmed-update-content-pending.png"), fullPage: true });
  blockedRefresh();
  await page.waitForFunction(() => window.referenceJourney.getSnapshot().binding.session.getSnapshot().status === "saved"
    && window.referenceJourney.getSnapshot().binding.controller.getSnapshot().phase === "error");
  const failedRefresh = await snapshot(); assert.notEqual(failedRefresh.progress?.view.artifact_id, riverFailureBaseline.progress?.view.artifact_id);
  assert.equal(await page.getByText("Your update is saved, but the new story screen could not be loaded. Retry loading it before continuing.", { exact: true }).count(), 1);
  assert.equal(await page.getByText("Story update saved.", { exact: true }).count(), 0);
  assert.equal(await button("Take the marked road home").count(), 0, "Choices stay unavailable until the saved edition can be opened");
  assert.equal(await button("Retry").isEnabled(), true);
  await page.screenshot({ path: join(directory, "confirmed-update-content-retry.png"), fullPage: true });
  await page.unroute("**/reference-host/content");
  await page.locator("article").getByRole("button", { name: "Retry", exact: true }).click(); await ready();
  assert.equal((await snapshot()).progress?.view.artifact_id, failedRefresh.progress?.view.artifact_id);
  await page.getByText("The river carried Mara to a harbor she had not seen before.", { exact: true }).waitFor();
  assert.equal(await page.getByText("Your update is saved, but the new story screen could not be loaded. Retry loading it before continuing.", { exact: true }).count(), 0);
  await writeFile(join(directory, "content-refresh-retry.json"), JSON.stringify({ original: riverFailureBaseline, failedRefresh, recovered: await snapshot(), requests, responses: await Promise.all([...binaries]) }, null, 2));
  await prepare();
  await page.route("**/reference-host/publish-divergence", route => route.fulfill({ status: 503, body: "Publication service offline" }));
  await click("Publish changed earlier journey");
  const hostAlert = page.getByText("The host action failed. Your saved journey is preserved; try the action again.", { exact: true });
  await hostAlert.waitFor();
  await click("Take the marked road home"); await ready();
  assert.equal(await hostAlert.isVisible(), true, "A saved reading action does not conceal an unrelated failed publication");
  await page.unroute("**/reference-host/publish-divergence");
  let before = await prepare();
  await review("exact"); await click("Continue saved version");
  await click("Publish B");
  // A real missing publication object yields the core's needs_objects result, then retry.
  const host: Evidence = await (await page.request.get(`${origin}/reference-host/evidence`)).json(); assert.ok(host.b);
  const missing = `**/reference-host/objects/${host.b.artifactId.slice(9)}.cbor`;
  await page.route(missing, route => route.fulfill({ status: 503, body: "Missing update object" }));
  await review("needs_objects"); await page.getByText("Some saved story objects are unavailable. Retry the review.", { exact: true }).waitFor();
  await click("Continue saved version"); assert.equal((await snapshot()).progress?.view.cursor, before.view.cursor);
  await page.unroute(missing); await review("automatic");
  await page.getByText("Your earlier choices have the same results.", { exact: true }).waitFor();
  // Cursor movement expires the visible report and core approval together.
  await click("Previous step"); await button("Return to the morning gate").waitFor();
  assert.equal(await page.getByRole("region", { name: "Story update review" }).count(), 0);
  await click("Return to the morning gate"); await review("automatic");
  // Publishing a different target expires it too; the new target has a real replay divergence.
  const changedPublication = page.waitForResponse(response => response.url().endsWith("/reference-host/publish-divergence"));
  await click("Publish changed earlier journey");
  const changedReply = await changedPublication; assert.equal(changedReply.status(), 200, await changedReply.text());
  await page.getByText(/Current publication: C/).waitFor();
  assert.equal(await page.getByRole("region", { name: "Story update review" }).count(), 0);
  await review("needs_confirmation", "divergence");
  await page.getByText("Your earlier choices have different results in this edition.", { exact: true }).waitFor();
  const divergenceTarget = await page.evaluate(() => window.referenceJourney.getSnapshot().targetArtifact);
  await click("Publish incompatible edition");
  await page.waitForFunction(previous => window.referenceJourney.getSnapshot().targetArtifact !== previous, divergenceTarget);
  assert.equal(await page.getByRole("region", { name: "Story update review" }).count(), 0, "Exact target identity expires review even when both publication labels are C");
  await review("old_version_only");
  await click("Publish changed earlier journey");
  await page.waitForFunction(expected => window.referenceJourney.getSnapshot().targetArtifact === expected, divergenceTarget);
  await review("needs_confirmation", "divergence");
  await page.screenshot({ path: join(directory, "divergence-report.png"), fullPage: true });
  // The peer is a second authenticated writer; application itself remains a visible action.
  await page.evaluate(() => window.referenceJourney.otherDeviceChoose("option.home"));
  await click("Confirm story changes");
  await page.getByText("Another device updated this journey. Reload saved progress.", { exact: true }).waitFor();
  assert.equal((await snapshot()).progress?.view.cursor, before.view.cursor);
  await click("Reload saved progress"); await page.locator("article").getByRole("heading", { name: "A light in the window", exact: true }).waitFor();
  // Positive divergent application, distinct from loss and limited comparison.
  await prepare(); await click("Publish changed earlier journey"); await review("needs_confirmation", "divergence");
  await click("Confirm story changes"); await ready(); await page.getByText("Story update saved.", { exact: true }).waitFor();
  await click("Take the marked road home"); await page.locator("article").getByRole("heading", { name: "A light in the window", exact: true }).waitFor();
  // Known rejection and an uncommitted unknown update both retain the old durable page.
  for (const fault of ["Reject the next save: storage full", "Drop the next save before commit"]) {
    before = await prepare(); await click("Publish B"); await review("automatic"); await details("Simulate a save failure");
    await click(fault); await click("Apply story update");
    await page.getByRole("alert").filter({ hasText: fault.startsWith("Reject") ? "Storage is full" : "The save response was lost" }).waitFor();
    assert.equal((await snapshot()).progress?.view.cursor, before.view.cursor);
    if (fault.startsWith("Reject")) {
      await click("Reload saved progress"); await ready(); assert.equal((await snapshot()).progress?.view.cursor, before.view.cursor);
      assert.equal(await page.getByRole("region", { name: "Story update review" }).count(), 0, "A consumed approval must be reviewed again after rejection");
      assert.equal(await hostAlert.count(), 0);
    }
    else {
      await click("Retry and verify save"); await ready(); assert.notEqual((await snapshot()).progress?.view.artifact_id, before.view.artifact_id);
      const attempts = await writes(); assert.deepEqual(attempts.at(-1), attempts.at(-2));
      assert.equal(await page.getByText("The host action failed. Your saved journey is preserved; try the action again.", { exact: true }).count(), 0);
    }
  }
  before = await prepare(); await click("Publish incompatible edition"); await review("old_version_only");
  await page.getByText("This journey cannot move to the updated story. You can continue the old version.", { exact: true }).waitFor();
  assert.equal(await button("Confirm story changes").count(), 0); await click("Continue saved version");
  assert.equal((await snapshot()).progress?.view.cursor, before.view.cursor);
  await click("Take the marked road home"); await page.locator("article").getByRole("heading", { name: "A light in the window", exact: true }).waitFor();
  before = await prepare(); await click("Publish recovery edition"); await review("needs_confirmation", "recovery");
  await page.getByText("This journey will continue at The last train.", { exact: true }).waitFor();
  await page.getByText(/Scenes left behind: 1/).waitFor();
  await page.screenshot({ path: join(directory, "recovery-report.png"), fullPage: true });
  await click("Continue saved version"); assert.equal((await snapshot()).progress?.view.cursor, before.view.cursor);
  await review("needs_confirmation", "recovery"); await click("Confirm story changes"); await ready();
  await button("Accept the signal lantern").waitFor();
  assert.equal((await snapshot()).progress?.view.shared.find(value => value.name === "signal")?.value.value, true);
  await click("Leave the lantern on the bench"); await click("Reopen this journey"); await button("Visit the keeper with your pass").waitFor();
  // Full maintenance import keeps the exact parameterized page; shallow restore really
  // uses a cold library, so the missing ancestor cannot be supplied by the old store.
  before = await prepare("keeper"); const oldLibrary = await page.evaluate(() => window.referenceJourney.libraryId());
  await click("Import this journey into a new library");
  await page.waitForFunction(previous => JSON.stringify(window.referenceJourney.libraryId()) !== JSON.stringify(previous), oldLibrary);
  await ready();
  assert.deepEqual((await snapshot()).progress?.page, before.page);
  assert.deepEqual((await snapshot()).progress?.view.frames, before.view.frames);
  assert.notDeepEqual(await page.evaluate(() => window.referenceJourney.libraryId()), oldLibrary);
  await page.getByText(/The keeper examined pass 7/).waitFor();
  const importedMap = await map(); assert.deepEqual(importedMap.units, ["keeper", "station"]);
  await click("Return to the morning gate"); await ready(); before = (await snapshot()).progress!;
  const fullLibrary = await page.evaluate(() => window.referenceJourney.libraryId());
  await details("Publication and recovery scenarios"); await click("Restore recent history into a new library");
  await page.waitForFunction(previous => JSON.stringify(window.referenceJourney.libraryId()) !== JSON.stringify(previous), fullLibrary);
  await ready();
  assert.deepEqual((await snapshot()).progress?.page, before.page); assert.deepEqual((await snapshot()).progress?.view.frames, before.view.frames);
  await click("Publish B"); await review("needs_confirmation", "unverified");
  await page.getByText("Some earlier choices could not be compared. Continuing may change their results.", { exact: true }).waitFor();
  await page.screenshot({ path: join(directory, "shallow-report.png"), fullPage: true });
  await click("Confirm story changes"); await ready();
  const shallowMap = await map();
  assert.ok("complete" in shallowMap.source && shallowMap.source.complete === false && "missing" in shallowMap.source && Array.isArray(shallowMap.source.missing) && shallowMap.source.missing.length > 0);
  assert.deepEqual(shallowMap.units, [], "A readable shallow page does not manufacture audited visits");
  const restored = (await snapshot()).progress!;
  await page.reload(); await ready();
  assert.equal((await snapshot()).progress?.view.cursor, restored.view.cursor); assert.deepEqual((await snapshot()).progress?.page, restored.page);
  await click("Take the marked road home"); await page.locator("article").getByRole("heading", { name: "A light in the window", exact: true }).waitFor();
  assert.deepEqual(errors, []); assert.equal(await page.locator("vite-error-overlay").count(), 0);
  const reportHost: Evidence = await (await page.request.get(`${origin}/reference-host/evidence`)).json();
  const objectDescriptions = new Map([reportHost.a, reportHost.b, reportHost.c, ...(reportHost.variants ?? [])].flatMap(publication => publication?.objects ?? []).map(object => [object.id, object]));
  const responseEvidence = await Promise.all([...binaries]);
  for (const response of responseEvidence.filter(response => response.status === 200)) {
    if (response.url.endsWith(".cbor")) {
      const id = new URL(response.url).pathname.split("/").at(-1)?.slice(0, -5), expected = id && objectDescriptions.get(id);
      assert.ok(expected); assert.equal(response.bytes, expected.bytes); assert.equal(response.sha256, expected.sha256);
    } else if (response.url.endsWith(".wasm")) {
      const expected = response.url.includes("authoring") ? options.authoringWasm : options.readerWasm;
      if (expected) { assert.equal(response.bytes, expected.bytes); assert.equal(response.sha256, expected.sha256); }
    }
  }
  await writeFile(join(directory, "visible-update-scenarios.json"), JSON.stringify({ mobile, evidence, requests, responses: responseEvidence, host: reportHost, errors, final: await snapshot() }, null, 2));
  await page.evaluate(() => window.referenceJourney.dispose());
}
type Evidence = ReturnType<Awaited<ReturnType<typeof createReferenceHost>>["evidence"]>;
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

/** One actual reference page: public compound publication, cold identities, rendered story and faults. */
export async function verifyReferenceJourney(page: Page, origin: string, directory: string, readerWasm: { bytes: number; sha256: string }, options: { mobile?: boolean; authoringWasm?: { bytes: number; sha256: string } } = {}) {
  await mkdir(directory, { recursive: true });
  const interactionViewport = options.mobile ? { width: 390, height: 844 } : { width: 1280, height: 900 };
  await page.setViewportSize(interactionViewport);
  const requests: string[] = []; const errors: string[] = []; const consoleMessages: string[] = [];
  const binary: Promise<{ url: string; bytes: Uint8Array; status: number }>[] = [];
  page.on("request", request => requests.push(new URL(request.url()).pathname));
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (["error", "warning"].includes(message.type())) consoleMessages.push(message.text()); });
  page.on("response", response => {
    if (/\.(cbor|wasm)$/.test(new URL(response.url()).pathname)) binary.push(response.body().then(bytes => ({ url: response.url(), bytes, status: response.status() }))
      .catch(() => ({ url: response.url(), bytes: new Uint8Array(), status: 0 })));
  });
  const button = (name: string) => page.getByRole("button", { name, exact: true });
  const visible = async (text: string | RegExp) => page.getByText(text, { exact: typeof text === "string" }).waitFor();
  const snapshot = () => page.evaluate(() => window.referenceJourney.getSnapshot().binding.session.getSnapshot());
  const context = () => page.evaluate(() => window.referenceJourney.getSnapshot().binding.controller.getSnapshot().screen?.context);
  const click = async (name: string) => { await button(name).click(); };
  const newJourney = async () => { await click("Start a new journey"); await button("Accept the signal lantern").waitFor(); };
  await page.goto(`${origin}/?reference`);
  await page.getByRole("heading", { name: "The last train", exact: true }).waitFor();
  assert.equal(await page.title(), "Narrata cold reference journey");
  await visible(/The last train left Mara/);
  const initial = await snapshot(); assert.equal(initial.status, "saved"); assert.equal(initial.progress?.view.depth, 0);
  const host: Evidence = await (await page.request.get(`${origin}/reference-host/evidence`)).json();
  assert.equal(host.previewed, true); assert.equal(host.editedPass, 7);
  const firstResponses = await Promise.all([...binary]);
  const firstObjects = firstResponses.filter(item => item.url.endsWith(".cbor"));
  const expectedIds = [host.a.artifactId.slice(9), host.entryChunk].sort();
  assert.deepEqual(firstObjects.map(item => new URL(item.url).pathname.split("/").at(-1)?.slice(0,-5)).sort(), expectedIds,
    "The accepted route itself fetches exactly its manifest and entry chunk");
  for (const item of firstObjects) {
    const id = new URL(item.url).pathname.split("/").at(-1)?.slice(0,-5);
    const description = host.a.objects.find(object => object.id === id); assert.ok(description);
    assert.equal(item.status, 200); assert.equal(item.bytes.length, description.bytes); assert.equal(sha256(item.bytes), description.sha256);
    assert.equal(createHash("sha256").update("narrata-object\0").update(item.bytes.subarray(10,14)).update(item.bytes.subarray(56)).digest("hex"), id);
  }
  const firstWasm = firstResponses.filter(item => item.url.endsWith(".wasm")); assert.equal(firstWasm.length, 1);
  assert.equal(firstWasm[0]?.bytes.length, readerWasm.bytes); assert.equal(sha256(firstWasm[0]!.bytes), readerWasm.sha256);
  assert.ok(host.a.objects.length > firstObjects.length);
  assert.deepEqual(requests.filter(path => /\.narpack$|authoring|\/testing|graph-map|renderer/i.test(path)), []);
  const firstEvidence = { route: page.url(), requests: [...requests], objects: firstObjects.map(item => ({ url: item.url, bytes: item.bytes.length, sha256: sha256(item.bytes) })),
    wasm: firstWasm.map(item => ({ url: item.url, bytes: item.bytes.length, sha256: sha256(item.bytes) })), publishedObjects: host.a.objects.length };
  await writeFile(join(directory, "cold-reference-first-screen.json"), JSON.stringify(firstEvidence, null, 2));
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.screenshot({ path: join(directory, "cold-reference-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(directory, "cold-reference-mobile.png"), fullPage: true });
  await page.setViewportSize(interactionViewport);
  const keeper = host.a.objects.find(object => object.role === "chunk" && object.id !== host.entryChunk); assert.ok(keeper);
  const keeperPattern = `**/reference-host/objects/${keeper.id}.cbor`;
  await page.route(keeperPattern, route => route.fulfill({ status: 503, body: "Controlled unavailable immutable object" }));
  await click("Accept the signal lantern");
  await visible("Mara lifted the lantern. Its warm handle fitted her hand.");
  await visible("Beyond the platform, both paths met beneath the station clock.");
  const merged = await snapshot(); assert.equal(merged.progress?.view.depth, 1);
  assert.ok(merged.progress?.page.some(item => item.role === "reply"));
  await click("Visit the keeper with your pass");
  await visible("Saved progress is unavailable. Try loading it again.");
  assert.equal((await snapshot()).progress?.view.cursor, merged.progress?.view.cursor);
  assert.equal(await page.getByRole("heading", { name: "The open sea", exact: true }).count(), 0);
  await page.screenshot({ path: join(directory, "cold-reference-missing-chunk.png"), fullPage: true });
  await page.unroute(keeperPattern); await click("Reload saved progress");
  await click("Visit the keeper with your pass");
  await visible(/The keeper examined pass 7 and stamped it/);
  const called = await snapshot();
  assert.deepEqual(called.progress?.view.frames.at(-1)?.parameters.find(value => value.name === "pass")?.value, { type: "int", value: "7" });
  await click("Return to the morning gate");
  await visible("At dawn, the lantern Mara accepted on the platform lit the safe road home.");
  assert.equal(await button("Take the marked road home").isEnabled(), true);
  assert.deepEqual(await context(), { languages: ["en"], viewer: "reader-one", realization: host.a.artifactId });
  await page.getByText("Simulate a save failure", { exact: true }).click();
  await click("Lose the next save response"); await click("Take the marked road home");
  await button("Retry and verify save").waitFor();
  assert.equal((await snapshot()).status, "unknown");
  assert.equal(await page.getByRole("heading", { name: "A light in the window", exact: true }).count(), 0);
  await page.screenshot({ path: join(directory, "cold-reference-unknown.png"), fullPage: true });
  await click("Retry and verify save");
  await visible(/The green lantern marked the road through the mist/);
  const home = await snapshot(); assert.equal(home.progress?.view.interaction.kind, "finished");
  const writes = await page.evaluate(() => window.referenceJourney.evidence().writes); assert.deepEqual(writes.at(-1), writes.at(-2));
  await page.screenshot({ path: join(directory, "cold-reference-ending-home.png"), fullPage: true });
  // Both endings and the delayed negative consequence are exercised on A before editing B.
  await newJourney(); await click("Leave the lantern on the bench"); await click("Visit the keeper with your pass");
  await visible(/The keeper examined pass 7/); await click("Return to the morning gate");
  await visible(/Without the lantern, Mara could not find the way home/);
  assert.equal(await button("Take the marked road home").isDisabled(), true);
  await click("Set out for the coast"); await visible(/At the coast, Mara watched a fishing boat/);
  const awayA = await snapshot(); assert.equal(awayA.progress?.view.artifact_id, host.a.artifactId);
  assert.equal(awayA.progress?.view.interaction.kind, "finished");
  await page.screenshot({ path: join(directory, "cold-reference-ending-away-a.png"), fullPage: true });
  await newJourney(); await click("Accept the signal lantern"); await click("Visit the keeper with your pass");
  await visible(/The keeper examined pass 7/); await click("Return to the morning gate"); await button("Wait beside the river").waitFor();
  const durableA = await snapshot();
  await click("Publish B"); await visible(/Current publication: B/);
  await click("Reopen this journey"); await button("Wait beside the river").waitFor();
  const restoredA = await snapshot(); assert.equal(restoredA.progress?.view.artifact_id, host.a.artifactId);
  assert.equal(restoredA.progress?.view.cursor, durableA.progress?.view.cursor);
  // Review is visible and read-only; continuing old does not apply the plan.
  await click("Review story update"); await page.getByRole("region", { name: "Story update review" }).waitFor();
  assert.equal((await snapshot()).progress?.view.cursor, durableA.progress?.view.cursor);
  await click("Continue saved version"); await button("Wait beside the river").waitFor();
  await click("Review story update"); await page.getByRole("region", { name: "Story update review" }).waitFor();
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.screenshot({ path: join(directory, "update-review-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(directory, "update-review-mobile.png"), fullPage: true });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
  await page.setViewportSize(interactionViewport);
  const apply = button("Apply story update");
  assert.equal(await page.getByRole("region", { name: "Story update review" }).getAttribute("data-assessment"), "automatic");
  // A late migration reply cannot publish success into the next account's reader.
  await page.evaluate(() => { window.oldReferenceSession = window.referenceJourney.getSnapshot().binding.session; });
  await click("Delay the next save response"); await apply.click();
  await page.waitForFunction(() => window.referenceJourney.hasLate());
  await click("Switch account"); await visible(/Account: reader-two/);
  await click("Release delayed save responses");
  assert.deepEqual(await page.evaluate(() => window.oldReferenceSession.getSnapshot()), { status: "closed", progress: null, candidate: null });
  assert.equal(await page.getByText("Story update saved.", { exact: true }).count(), 0);
  await click("Switch account"); await visible(/Account: reader-one/); await button("Take the marked road home").waitFor();
  await click("Previous step"); await button("Wait beside the river").waitFor();
  await click("Review story update"); await apply.waitFor();
  await apply.waitFor(); await click("Lose the next save response"); await apply.click();
  await button("Retry and verify save").waitFor();
  assert.equal((await snapshot()).status, "unknown");
  assert.equal((await snapshot()).progress?.view.cursor, restoredA.progress?.view.cursor);
  assert.equal(await page.getByText("Story update saved.", { exact: true }).count(), 0);
  await click("Retry and verify save");
  await page.waitForFunction(() => window.referenceJourney.getSnapshot().binding.session.getSnapshot().status === "saved");
  const migratedB = await snapshot(); assert.equal(migratedB.status, "saved");
  assert.notEqual(migratedB.progress?.view.artifact_id, host.a.artifactId);
  await page.locator("article").getByRole("heading", { name: "At the morning gate", exact: true }).waitFor();
  assert.equal(await page.locator("article").getByRole("button", { name: "Take the marked road home", exact: true }).isEnabled(), true);
  await page.getByRole("status").filter({ hasText: /^Saved/ }).waitFor();
  assert.equal(await page.getByText("The host action failed. Your saved journey is preserved; try the action again.", { exact: true }).count(), 0);
  assert.equal(await button("Wait beside the river").count(), 0);
  const updateWrites = await page.evaluate(() => window.referenceJourney.evidence().writes);
  assert.deepEqual(updateWrites.at(-1), updateWrites.at(-2), "Update recovery retries the original migration bytes");
  // The visible navigation checks out A, then continues a real old choice and saves it.
  await click("Previous step"); await button("Wait beside the river").waitFor();
  assert.equal((await snapshot()).progress?.view.artifact_id, host.a.artifactId);
  await click("Wait beside the river"); await visible(/Along the river, the water carried/);
  assert.equal((await snapshot()).progress?.view.artifact_id, host.a.artifactId);
  const bIndex = (await snapshot()).progress!.view.history.findIndex(row => row.id === migratedB.progress?.view.cursor);
  assert.ok(bIndex >= 0, "The migrated branch remains reachable after continuing A");
  await page.getByRole("navigation", { name: "Journey", exact: true }).getByRole("button").nth(bIndex).click();
  await button("Take the marked road home").waitFor();
  assert.equal((await snapshot()).progress?.view.artifact_id, migratedB.progress?.view.artifact_id);
  // Re-publication and the second migration retain A/B/C in this same library.
  await click("Review story update"); await visible("This journey already uses the current story."); await click("Continue saved version");
  await click("Publish C"); await click("Review story update");
  await page.getByRole("region", { name: "Story update review" }).waitFor();
  await page.getByRole("combobox", { name: "Update review depth" }).selectOption("short");
  assert.equal(await page.getByRole("region", { name: "Story update review" }).count(), 0, "Changing review budget invalidates approval");
  await click("Review story update"); await button("Confirm story changes").waitFor();
  await visible("Some earlier choices could not be compared. Continuing may change their results.");
  await click("Confirm story changes"); await visible("Story update saved.");
  const migratedC = await snapshot(); assert.equal(migratedC.status, "saved");
  assert.notEqual(migratedC.progress?.view.artifact_id, migratedB.progress?.view.artifact_id);
  assert.ok(migratedC.progress!.view.history.some(row => row.id === migratedB.progress?.view.cursor));
  assert.ok(migratedC.progress!.view.history.some(row => row.id === restoredA.progress?.view.cursor));
  const mapResponse = page.waitForResponse(response => response.url().endsWith("/reference-host/reader-map"));
  await click("Show revealed map"); await page.getByTestId("reader-map").waitFor();
  const updateMap: unknown = await (await mapResponse).json();
  assert.ok(typeof updateMap === "object" && updateMap !== null && "artifact_id" in updateMap && updateMap.artifact_id === migratedC.progress?.view.artifact_id
    && "units" in updateMap && Array.isArray(updateMap.units));
  assert.equal(updateMap.units.some((unit: unknown) => typeof unit === "object" && unit !== null && "content" in unit && typeof unit.content === "object"
    && unit.content !== null && "key" in unit.content && unit.content.key === "river"), false, "The map does not reveal the river taken only on the old A sibling branch");
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.screenshot({ path: join(directory, "update-map-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(directory, "update-map-mobile.png"), fullPage: true });
  await page.setViewportSize(interactionViewport);
  const oldManifestPattern = `**/reference-host/objects/${host.a.artifactId.slice(9)}.cbor`;
  await page.route(oldManifestPattern, route => route.fulfill({ status: 503, body: "Controlled unavailable historical manifest" }));
  await page.reload();
  await visible("The saved story version is unavailable. Your progress is preserved.");
  await page.screenshot({ path: join(directory, "update-missing-history-manifest.png"), fullPage: true });
  await page.unroute(oldManifestPattern); await click("Retry opening saved journey");
  await button("Take the marked road home").waitFor();
  const refreshedC = await snapshot(); assert.equal(refreshedC.progress?.view.cursor, migratedC.progress?.view.cursor);
  assert.equal(refreshedC.progress?.view.artifact_id, migratedC.progress?.view.artifact_id);
  await page.getByText("Simulate a save failure", { exact: true }).click();
  const historicalMaps: unknown[] = [];
  const checkoutVisible = async (commit: string) => {
    const index = (await snapshot()).progress!.view.history.findIndex(row => row.id === commit); assert.ok(index >= 0);
    await page.getByRole("navigation", { name: "Journey", exact: true }).getByRole("button").nth(index).click();
    await page.waitForFunction(commit => window.referenceJourney.getSnapshot().binding.controller.getSnapshot().screen?.book.view.cursor === commit
      && window.referenceJourney.getSnapshot().binding.controller.getSnapshot().phase === "ready", commit);
  };
  const historicalMap = async (expectedUnits: string[]) => {
    const response = page.waitForResponse(response => response.url().endsWith("/reference-host/reader-map"));
    await click("Show revealed map"); await page.getByTestId("reader-map").waitFor();
    const value: unknown = await (await response).json(), current = (await snapshot()).progress!.view;
    assert.ok(typeof value === "object" && value !== null && "artifact_id" in value && value.artifact_id === current.artifact_id && "units" in value && Array.isArray(value.units)
      && "sources" in value && Array.isArray(value.sources));
    const units = value.units.map((unit: unknown) => { assert.ok(typeof unit === "object" && unit !== null && "content" in unit && typeof unit.content === "object"
      && unit.content !== null && "key" in unit.content && typeof unit.content.key === "string"); return unit.content.key; }).sort();
    assert.deepEqual(units, expectedUnits.sort());
    const source: unknown = value.sources[0];
    assert.ok(typeof source === "object" && source !== null && "cursor" in source && source.cursor === current.cursor && "complete" in source && source.complete === true
      && "frontier" in source && Array.isArray(source.frontier));
    assert.ok(current.interaction.kind === "choose");
    assert.deepEqual(source.frontier.map((item: unknown) => { assert.ok(typeof item === "object" && item !== null && "option" in item); return item.option; }).sort(), current.interaction.options.map(option => option.id).sort());
    historicalMaps.push({ map: value, current });
  };
  await checkoutVisible(restoredA.progress!.view.cursor); await button("Wait beside the river").waitFor();
  await historicalMap(["station", "keeper", "dawn"]);
  assert.equal((await context())?.realization, host.a.artifactId);
  await click("Wait beside the river"); await visible(/Along the river, the water carried/);
  assert.equal((await snapshot()).progress?.view.artifact_id, host.a.artifactId);
  await checkoutVisible(migratedB.progress!.view.cursor);
  await historicalMap(["dawn"]);
  assert.equal((await context())?.realization, migratedB.progress?.view.artifact_id);
  await click("Take the marked road home"); await visible(/The green lantern marked the road/);
  const continuedB = await snapshot(); assert.equal(continuedB.progress?.view.artifact_id, migratedB.progress?.view.artifact_id);
  await checkoutVisible(migratedC.progress!.view.cursor);
  await click("Take the marked road home"); await visible(/The green lantern marked the road/);
  await newJourney(); await click("Leave the lantern on the bench");
  await visible("Mara left the lantern on the bench and tightened her coat.");
  await visible("Beyond the platform, both paths met beneath the station clock.");
  await click("Visit the keeper with your pass"); await visible(/The keeper examined pass 7/);
  await click("Return to the morning gate");
  await visible(/Without the lantern, Mara could not find the way home/);
  assert.equal(await button("Take the marked road home").isDisabled(), true);
  assert.equal(await button("Wait beside the river").count(), 0);
  const onB = await snapshot(); assert.notEqual(onB.progress?.view.artifact_id, host.a.artifactId);
  assert.deepEqual(await context(), { languages: ["en"], viewer: "reader-one", realization: onB.progress?.view.artifact_id });
  await click("Set out for the coast"); await visible(/At the coast, Mara watched a fishing boat/);
  const away = await snapshot(); assert.equal(away.progress?.view.interaction.kind, "finished");
  await page.screenshot({ path: join(directory, "cold-reference-ending-away.png"), fullPage: true });
  await newJourney(); await click("Reject the next save: storage full"); await click("Accept the signal lantern");
  await visible("Storage is full. Your last saved progress is preserved."); assert.equal((await snapshot()).progress?.view.depth, 0);
  await click("Reload saved progress"); await click("Accept the signal lantern"); await button("Visit the keeper with your pass").waitFor();
  await page.evaluate(() => window.referenceJourney.otherDeviceChoose("option.river"));
  await click("Visit the keeper with your pass");
  await visible("Another device updated this journey. Reload saved progress.");
  await click("Reload saved progress"); await visible(/At the coast, Mara watched a fishing boat/);
  await newJourney(); await click("Lose the next save response"); await click("Accept the signal lantern");
  await button("Retry and verify save").waitFor();
  await page.evaluate(() => window.referenceJourney.otherDeviceChoose("option.visit"));
  await click("Retry and verify save"); await visible(/The keeper examined pass 7/);
  assert.equal((await snapshot()).progress?.view.depth, 2, "Recovery follows the other device beyond the candidate");
  await page.evaluate(() => {
    const { session, controller } = window.referenceJourney.getSnapshot().binding;
    const view = session.getSnapshot().progress?.view;
    if (view?.interaction.kind !== "choose") throw new Error("Expected keeper choice");
    const option = view.interaction.options[0]; if (!option) throw new Error("Missing keeper option");
    window.oldReferenceSession = session; window.referenceJourney.setFault("late"); window.referenceLate = controller.choose([option.id]);
  });
  await page.waitForFunction(() => window.referenceJourney.hasLate());
  await click("Switch account"); await visible(/Account: reader-two/);
  await page.evaluate(() => window.referenceJourney.releaseLate());
  assert.equal(await page.evaluate(() => window.referenceLate), false);
  assert.deepEqual(await page.evaluate(() => window.oldReferenceSession.getSnapshot()), { status: "closed", progress: null, candidate: null });
  const oldId = await page.evaluate(() => { window.oldReferenceSession = window.referenceJourney.getSnapshot().binding.session; return window.referenceJourney.libraryId(); });
  await page.evaluate(() => window.referenceJourney.remove());
  assert.notDeepEqual(await page.evaluate(() => window.referenceJourney.libraryId()), oldId);
  const writeCount = await page.evaluate(() => window.referenceJourney.evidence().writes.length);
  assert.equal(await page.evaluate(async () => { try { await window.oldReferenceSession.retrySave(); return false; } catch { return true; } }), true);
  assert.equal(await page.evaluate(() => window.referenceJourney.evidence().writes.length), writeCount);
  assert.deepEqual(errors, []); assert.equal(await page.locator("vite-error-overlay").count(), 0);
  assert.deepEqual(consoleMessages.filter(message => !message.includes("503")), []);
  const finalHost: Evidence = await (await page.request.get(`${origin}/reference-host/evidence`)).json();
  const objectDescriptions = new Map([finalHost.a, finalHost.b, finalHost.c].flatMap(publication => publication?.objects ?? []).map(object => [object.id, object]));
  const responses = (await Promise.all([...binary])).map(response => ({ url: response.url, status: response.status, bytes: response.bytes.length, sha256: sha256(response.bytes) }));
  for (const response of responses.filter(response => response.status === 200)) {
    if (response.url.endsWith(".cbor")) {
      const id = new URL(response.url).pathname.split("/").at(-1)?.slice(0, -5);
      assert.ok(id); const expected = objectDescriptions.get(id); assert.ok(expected);
      assert.equal(response.bytes, expected.bytes); assert.equal(response.sha256, expected.sha256);
    } else if (options.authoringWasm) {
      const expected = response.sha256 === readerWasm.sha256 ? readerWasm : options.authoringWasm;
      assert.equal(response.bytes, expected.bytes); assert.equal(response.sha256, expected.sha256);
    }
  }
  await writeFile(join(directory, "cold-reference-journey.json"), JSON.stringify({ initial, merged, called, home, awayA, restoredA, migratedB, migratedC, refreshedC, continuedB, historicalMaps, onB, away,
    requests, responses, host: finalHost, client: await page.evaluate(() => window.referenceJourney.evidence()), errors, consoleMessages }, null, 2));
  await page.evaluate(() => window.referenceJourney.dispose());
}
