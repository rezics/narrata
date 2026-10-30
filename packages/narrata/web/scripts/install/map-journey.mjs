import { strict as assert } from "node:assert";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
export async function verifyMapJourney(page, origin, directory) {
  await mkdir(directory, { recursive: true });
  const requests = [], errors = [];
  page.on("request", request => requests.push(new URL(request.url()).pathname));
  page.on("pageerror", error => errors.push(error.message));
  await page.goto(origin + "/?reference");
  await page.getByRole("button", { name: "Accept the signal lantern", exact: true }).waitFor();
  const firstScreen = [...requests];
  assert.deepEqual(firstScreen.filter(path => /authoring|reader-map|relation|graph-map|renderer/.test(path)), []);
  async function map() {
    const response = page.waitForResponse(response => response.url().endsWith("/reference-host/reader-map"));
    await page.getByRole("button", { name: "Show revealed map", exact: true }).click();
    const payload = await (await response).json();
    await page.getByTestId("reader-map").waitFor();
    return payload;
  }
  const initial = await map();
  assert.equal(initial.units.length, 1);
  assert.equal(JSON.stringify(initial).includes('"keeper"'), false);
  await page.getByRole("button", { name: "Accept the signal lantern", exact: true }).click();
  await page.getByRole("button", { name: "Visit the keeper with your pass", exact: true }).click();
  const called = await map();
  assert.ok(called.units.some(unit => unit.content.key === "keeper"));
  assert.ok(called.nodes.length > initial.nodes.length);
  assert.deepEqual(called.nodes.slice(0, initial.nodes.length), initial.nodes);
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.screenshot({ path: join(directory, "reader-map-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(directory, "reader-map-mobile.png"), fullPage: true });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
  assert.deepEqual(errors, []);
  assert.deepEqual(requests.filter(path => /authoring.*wasm|relation-part|story\.graph/.test(path)), []);
  // Advance between inspecting progress and exporting its checkpoint, then rewind while
  // the checked response is in flight. The response belongs to its own cursor, not the
  // render-time cursor captured by the original button click.
  let release, started;
  const waiting = new Promise(done => { started = done; });
  const gate = new Promise(done => { release = done; });
  await page.route("**/reference-host/reader-map", async route => { const response = await route.fetch(); started(); await gate; await route.fulfill({ response }); });
  await page.evaluate(() => {
    const session = window.referenceJourney.getSnapshot().binding.session;
    const original = session.exportSave.bind(session);
    window.mapExportStarted = false;
    const gate = new Promise(done => { window.releaseMapExport = done; });
    session.exportSave = async () => { window.mapExportStarted = true; await gate; return original(); };
    window.restoreMapExport = () => { session.exportSave = original; };
  });
  await page.getByRole("button", { name: "Show revealed map", exact: true }).click();
  await page.waitForFunction(() => window.mapExportStarted);
  await page.getByRole("button", { name: "Return to the morning gate", exact: true }).click();
  await page.getByRole("button", { name: "Take the marked road home", exact: true }).waitFor();
  await page.evaluate(() => window.releaseMapExport());
  await waiting;
  await page.evaluate(cursor => window.referenceJourney.getSnapshot().binding.controller.checkout(cursor), called.sources[0].cursor);
  release();
  await page.waitForFunction(() => ![...document.querySelectorAll("button")].find(button => button.textContent === "Show revealed map")?.disabled);
  assert.equal(await page.getByTestId("reader-map").count(), 0);
  await page.evaluate(() => window.restoreMapExport());
  await page.unroute("**/reference-host/reader-map");
  await writeFile(join(directory, "reader-map-evidence.json"), JSON.stringify({ firstScreen, requests, initial, called, errors }, null, 2));
  await page.evaluate(() => window.referenceJourney.dispose());
}
