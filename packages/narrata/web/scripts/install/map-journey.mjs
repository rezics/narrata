import { strict as assert } from "node:assert";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
export async function verifyMapJourney(page, origin, directory) {
  await mkdir(directory, { recursive: true });
  const requests = [], errors = [];
  const readyMapButton = async () => page.waitForFunction(() => { const button = [...document.querySelectorAll("button")].find(button => button.textContent === "Show revealed map"); return button instanceof HTMLButtonElement && !button.disabled; });
  const currentContent = () => page.getByRole("button", { name: "Return to the morning gate", exact: true }).waitFor();
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
    const original = session.exportConfirmedSave.bind(session);
    window.mapExportStarted = false;
    const gate = new Promise(done => { window.releaseMapExport = done; });
    session.exportConfirmedSave = async () => { window.mapExportStarted = true; await gate; return original(); };
    window.restoreMapExport = () => { session.exportConfirmedSave = original; };
  });
  await page.getByRole("button", { name: "Show revealed map", exact: true }).click();
  await page.waitForFunction(() => window.mapExportStarted);
  await page.getByRole("button", { name: "Return to the morning gate", exact: true }).click();
  await page.getByRole("button", { name: "Take the marked road home", exact: true }).waitFor();
  await page.evaluate(() => window.releaseMapExport());
  await waiting;
  await page.evaluate(cursor => window.referenceJourney.getSnapshot().binding.controller.checkout(cursor), called.sources[0].cursor);
  release();
  await readyMapButton();
  await currentContent();
  assert.equal(await page.getByTestId("reader-map").count(), 0);
  await page.evaluate(() => window.restoreMapExport());
  await page.unroute("**/reference-host/reader-map");
  assert.deepEqual(errors, []);
  const lifecycle = [];
  for (const transition of ["switchAccount", "newJourney"]) {
    let unblock, arrived;
    const pending = new Promise(done => { arrived = done; });
    const gate = new Promise(done => { unblock = done; });
    await page.route("**/reference-host/reader-map", async route => { const response = await route.fetch(); arrived(); await gate; await route.fulfill({ response }); });
    await page.getByRole("button", { name: "Show revealed map", exact: true }).click(); await pending;
    await page.evaluate(transition => window.referenceJourney[transition](), transition);
    await page.getByRole("button", { name: "Accept the signal lantern", exact: true }).waitFor();
    unblock(); await readyMapButton();
    assert.equal(await page.getByTestId("reader-map").count(), 0);
    lifecycle.push(transition);
    await page.unroute("**/reference-host/reader-map");
    assert.deepEqual(errors, []);
  }
  const guards = [];
  async function confirmedGuard(status) {
    await page.waitForFunction(status => window.referenceJourney.getSnapshot().binding.session.getSnapshot().status === status, status);
    assert.equal(await page.getByRole("button", { name: "Show revealed map", exact: true }).isDisabled(), true);
    const before = requests.filter(path => path === "/reference-host/reader-map").length;
    assert.equal(await page.evaluate(async () => { try { await window.referenceJourney.readerMap(); return false; } catch { return true; } }), true);
    assert.equal(requests.filter(path => path === "/reference-host/reader-map").length, before);
    guards.push(status);
    await page.evaluate(() => window.referenceJourney.getSnapshot().binding.controller.retry());
    await readyMapButton();
  }
  await page.evaluate(() => window.referenceJourney.setFault("lost"));
  await page.getByRole("button", { name: "Accept the signal lantern", exact: true }).click();
  await confirmedGuard("unknown");
  await page.evaluate(() => window.referenceJourney.setFault("quota"));
  await page.getByRole("button", { name: "Visit the keeper with your pass", exact: true }).click();
  await confirmedGuard("quota");
  await page.evaluate(() => window.referenceJourney.otherDeviceChoose("option.visit"));
  await page.getByRole("button", { name: "Follow the river instead", exact: true }).click();
  await confirmedGuard("conflict");
  await currentContent();
  const invalid = [];
  for (const kind of ["malformed", "format", "projection", "artifact", "cursor", "sources", "coordinates", "unknown_field"]) {
    const before = await page.evaluate(() => window.referenceJourney.getSnapshot().binding.session.getSnapshot().progress.view.cursor);
    await page.route("**/reference-host/reader-map", async route => {
      const response = await route.fetch(), data = await response.json();
      if (kind === "malformed") data.nodes = "bad";
      if (kind === "format") data.format_version = 0;
      if (kind === "projection") data.projection_version = 2;
      if (kind === "artifact") data.artifact_id = "artifact:" + "00".repeat(32);
      if (kind === "cursor") data.sources[0].cursor = "commit:" + "00".repeat(32);
      if (kind === "sources") data.sources.push(structuredClone(data.sources[0]));
      if (kind === "coordinates") data.nodes[0].x = 0.5;
      if (kind === "unknown_field") data.hidden_target = "must reject an unknown field";
      await route.fulfill({ response, body: JSON.stringify(data) });
    });
    await page.getByRole("button", { name: "Show revealed map", exact: true }).click(); await readyMapButton();
    await page.getByRole("alert").filter({ hasText: "The host action failed" }).waitFor();
    await currentContent();
    assert.equal(await page.evaluate(() => window.referenceJourney.getSnapshot().binding.session.getSnapshot().progress.view.cursor), before);
    assert.equal(await page.getByTestId("reader-map").count(), 0);
    assert.deepEqual(errors, []); invalid.push(kind);
    await page.unroute("**/reference-host/reader-map");
  }
  await map(); // A successful retry remains available after every recoverable HTTP failure.
  await page.route("**/reference-host/reader-map", async route => {
    const response = await route.fetch(), data = await response.json();
    // A schema-valid 800-point host fixture isolates geometry from execution semantics.
    // Native/installed replay tests separately establish which points may be revealed.
    data.nodes = Array.from({ length: 800 }, (_, i) => ({ node: "node:" + (i + 900000).toString(16).padStart(32, "0"), unit_id: null, x: i % 8, y: Math.floor(i / 8) }));
    data.edges = data.nodes.slice(1).map((node, i) => ({ source: data.nodes[i].node, target: node.node, choice_point: null, option: null }));
    await route.fulfill({ response, body: JSON.stringify(data) });
  });
  await map(); const geometry = [];
  for (const width of [1280, 390]) {
    await page.setViewportSize({ width, height: 900 });
    const viewport = page.getByRole("region", { name: "Scrollable revealed map", exact: true });
    await viewport.evaluate(element => { element.scrollTop = element.scrollHeight; element.scrollLeft = element.scrollWidth; });
    const metrics = await page.locator('[data-map-node="800"]').evaluate(element => {
      const circle = element.querySelector("circle").getBoundingClientRect(), text = element.querySelector("text").getBoundingClientRect();
      const viewport = element.closest('[role="region"]').getBoundingClientRect();
      return { circle: circle.width, text: text.height, visible: circle.left >= viewport.left && circle.right <= viewport.right && circle.top >= viewport.top && circle.bottom <= viewport.bottom, viewport: viewport.height, overflow: document.documentElement.scrollWidth > innerWidth + 1 };
    });
    assert.ok(metrics.circle >= 32 && metrics.text >= 14 && metrics.visible && metrics.viewport <= 401 && !metrics.overflow, JSON.stringify(metrics));
    geometry.push({ width, ...metrics });
    await page.screenshot({ path: join(directory, `reader-map-long-${width}.png`), fullPage: true });
  }
  await page.unroute("**/reference-host/reader-map");
  assert.deepEqual(errors, []);
  assert.deepEqual(requests.filter(path => /\/graph\/(?:index|react|controller|wire|renderer)\.[jt]sx?|\/node_modules\/(?:sigma|graphology)(?:\/|$)/.test(path)), []);
  await writeFile(join(directory, "reader-map-evidence.json"), JSON.stringify({ firstScreen, requests, initial, called, lifecycle, guards, invalid, geometry, errors }, null, 2));
  await page.evaluate(() => window.referenceJourney.dispose());
}
