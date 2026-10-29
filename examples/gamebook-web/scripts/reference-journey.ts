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
type Evidence = ReturnType<Awaited<ReturnType<typeof createReferenceHost>>["evidence"]>;
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

/** One actual reference page: public compound publication, cold identities, rendered story and faults. */
export async function verifyReferenceJourney(page: Page, origin: string, directory: string, readerWasm: { bytes: number; sha256: string }) {
  await mkdir(directory, { recursive: true });
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
  await page.screenshot({ path: join(directory, "cold-reference-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(directory, "cold-reference-mobile.png"), fullPage: true });
  await page.setViewportSize({ width: 1280, height: 900 });
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
  assert.deepEqual(await context(), { languages: ["en"], viewer: "reader-one" });
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
  await newJourney(); await click("Leave the lantern on the bench");
  await visible("Mara left the lantern on the bench and tightened her coat.");
  await visible("Beyond the platform, both paths met beneath the station clock.");
  await click("Visit the keeper with your pass"); await visible(/The keeper examined pass 7/);
  await click("Return to the morning gate");
  await visible(/Without the lantern, Mara could not find the way home/);
  assert.equal(await button("Take the marked road home").isDisabled(), true);
  assert.equal(await button("Wait beside the river").count(), 0);
  const onB = await snapshot(); assert.notEqual(onB.progress?.view.artifact_id, host.a.artifactId);
  assert.deepEqual(await context(), { languages: ["en"], viewer: "reader-one" });
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
  await writeFile(join(directory, "cold-reference-journey.json"), JSON.stringify({ initial, merged, called, home, awayA, restoredA, onB, away,
    host: await (await page.request.get(`${origin}/reference-host/evidence`)).json(), client: await page.evaluate(() => window.referenceJourney.evidence()), errors, consoleMessages }, null, 2));
  await page.evaluate(() => window.referenceJourney.dispose());
}
