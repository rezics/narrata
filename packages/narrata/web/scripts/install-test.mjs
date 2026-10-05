import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { strict as assert } from "node:assert";
import { chromium } from "@playwright/test";
import { createServer } from "node:http";
import { demoPack, execution, splitPack } from "../test/fixture.ts";
import { npmCli, tarball } from "./pack.mjs";

const here = resolve(import.meta.dirname, "..");
const temporaryRoot = join(tmpdir(), ".temp");
mkdirSync(temporaryRoot, { recursive: true });
const directory = mkdtempSync(join(temporaryRoot, "narrata-web-install-"));
const parts = splitPack(demoPack);
const npm = (...args) => execFileSync(process.execPath, [npmCli, ...args], { cwd: directory, stdio: "inherit" });
writeFileSync(join(directory, "package.json"), JSON.stringify({ name: "narrata-install-test", private: true, type: "module" }));
npm("install", "--ignore-scripts", "--no-audit", "--no-fund", "--save-exact", tarball, "vite@8.2.2", "typescript@~5.9.3", "@types/node@^24.0.0");
const installed = join(directory, "node_modules/@rezics/narrata");
assert.equal(realpathSync(installed), installed, "Installation must be unpacked, not a workspace symlink");
const manifest = JSON.parse(readFileSync(join(installed, "package.json")));
assert.deepEqual(Object.keys(manifest.exports), [".", "./storage", "./react", "./testing"]);
assert.equal(manifest.dependencies, undefined, "Runtime assets and storage host are self-contained");
assert.deepEqual(manifest.peerDependenciesMeta, { react: { optional: true } });
assert.equal(existsSync(join(directory, "node_modules/react")), false, "Framework-free hosts must not install React");
assert.equal(createHash("sha256").update(readFileSync(join(installed, "dist/wasm/narrata_nodes_wasm_bg.wasm"))).digest("hex"),
  createHash("sha256").update(readFileSync(join(here, "dist/wasm/narrata_nodes_wasm_bg.wasm"))).digest("hex"));
const publicDirectory = join(directory, "public");
mkdirSync(join(publicDirectory, "objects"), { recursive: true });
writeFileSync(join(publicDirectory, "manifest.cbor"), parts.manifest);
for (const [id, bytes] of parts.objects) writeFileSync(join(publicDirectory, "objects", `${id.slice(7)}.cbor`), bytes);
writeFileSync(join(directory, "objects.json"), JSON.stringify(Object.fromEntries([...parts.objects].map(([id, bytes]) => [id, [...bytes]]))));
writeFileSync(join(directory, "manifest.cbor"), parts.manifest);
cpSync(resolve(here, "test/types.ts"), join(directory, "types.ts"));
writeFileSync(join(directory, "tsconfig.json"), JSON.stringify({ compilerOptions: { target: "ES2022", module: "NodeNext", moduleResolution: "NodeNext", lib: ["ES2022", "DOM"], strict: true, skipLibCheck: true, noEmit: true }, include: ["types.ts"] }));
execFileSync(process.execPath, [join(directory, "node_modules/typescript/bin/tsc"), "-p", join(directory, "tsconfig.json")], { cwd: directory, stdio: "inherit" });
writeFileSync(join(directory, "node.mjs"), `
import { readFile } from "node:fs/promises";
import { strict as assert } from "node:assert";
import { openBook, initialize, resolveContent } from "@rezics/narrata";
import { MockRezicsContent, mountainLetterData, mountainLetterKeys } from "@rezics/narrata/testing";
const mock = new MockRezicsContent(mountainLetterData(), 1);
const station = mountainLetterKeys["main.station"];
const label = mountainLetterKeys["main.journey:station.camp.label"];
assert.ok(station && label);
const resolved = await resolveContent(mock, [
  { content: { unit: { provider: "rezics", key: station }, first: "p1", last: "p1" } },
  { content: { provider: "rezics", key: label } },
], { realization: "en", viewer: "reader:installed" }, 1);
assert.equal(resolved[0].status, "ok");
assert.equal(resolved[0].payload.blocks[0].id, "p1");
assert.equal(resolved[1].payload.text, "Rest at the camp");
const objects = JSON.parse(await readFile(new URL("./objects.json", import.meta.url), "utf8"));
const wasm = new URL("./node_modules/@rezics/narrata/dist/wasm/narrata_nodes_wasm_bg.wasm", import.meta.url);
if (process.argv[2] === "bytes") await initialize(new Uint8Array(await readFile(wasm)));
if (process.argv[2] === "url") await initialize(wasm);
const requests = [];
const book = await openBook({ manifest: new Uint8Array(await readFile(new URL("./manifest.cbor", import.meta.url))), execution: ${JSON.stringify(execution)},
  fetchChunk: async id => { requests.push(id); assert.ok(objects[id]); return new Uint8Array(objects[id]); } });
try {
  const view = await book.inspect(); assert.equal(view.view.depth, 0); assert.equal(view.view.interaction.kind, "choose"); assert.equal(requests.length, 1);
  const save = await book.exportSave();
  const units = await book.nextContentUnits();
  assert.ok(units.some(reference => reference.key === "camp.fire"));
  assert.ok(units.some(reference => reference.key === "road.rocks"));
  assert.equal(await book.exportSave(), save);
}
finally { await book.close(); }
console.log("Installed package opened in " + process.argv[2]);
`);
for (const mode of ["default", "url", "bytes"]) execFileSync(process.execPath, [join(directory, "node.mjs"), mode], { cwd: directory, stdio: "inherit" });
// Bun is already part of the repository toolchain; test its file-URL loader too.
execFileSync("bun", [join(directory, "node.mjs"), "default"], { cwd: directory, stdio: "inherit" });

npm("install", "--ignore-scripts", "--no-audit", "--no-fund", "--save-exact", "react@19.2.8", "react-dom@19.2.8", "@types/react@^19.0.0", "@types/react-dom@^19.0.0");
cpSync(resolve(here, "test/reader-types.tsx"), join(directory, "reader-types.tsx"));
writeFileSync(join(directory, "tsconfig.json"), JSON.stringify({ compilerOptions: { target: "ES2022", module: "NodeNext", moduleResolution: "NodeNext", lib: ["ES2022", "DOM"], strict: true, skipLibCheck: true, noEmit: true, jsx: "react-jsx" }, include: ["types.ts", "reader-types.tsx"] }));
execFileSync(process.execPath, [join(directory, "node_modules/typescript/bin/tsc"), "-p", join(directory, "tsconfig.json")], { cwd: directory, stdio: "inherit" });

writeFileSync(join(directory, "index.html"), '<!doctype html><html><head><title>Narrata install test</title></head><body><p id="status">loading</p><div id="reader"></div><script type="module" src="/main.tsx"></script></body></html>');
writeFileSync(join(directory, "main.tsx"), `
import { openBook, createReader } from "@rezics/narrata";
import { IndexedDbStore } from "@rezics/narrata/storage";
import { Reader } from "@rezics/narrata/react";
import { createRoot } from "react-dom/client";
async function bytes(url) { const response = await fetch(url); if (!response.ok) throw new Error("HTTP " + response.status); return new Uint8Array(await response.arrayBuffer()); }
window.ready = (async () => {
  const store = await IndexedDbStore.open("installed-package-session");
  const book = await openBook({ manifest: await bytes("/manifest.cbor"), execution: ${JSON.stringify(execution)}, storage: store,
    fetchChunk: id => bytes("/objects/" + id.slice(7) + ".cbor") });
  const controller = createReader(book, { resolve: async request => request.items.map(() => ({ status: "ok", revision: "host", payload: { text: "HOST CONTENT" } })) }, { persistence: "durable", context: { languages: ["en"] } });
  const slot = props => <span data-host-role={props.role} data-realization={props.context.realization}>{props.resolution.status === "ok" ? props.resolution.payload.text : props.resolution.status}</span>;
  const root = createRoot(document.querySelector("#reader"));
  root.render(<Reader controller={controller} slots={{ body: slot, option: slot, reference: slot }} />);
  let firstScreenRequests;
  controller.subscribe(() => { if (firstScreenRequests === undefined && controller.getSnapshot().phase === "ready") firstScreenRequests = performance.getEntriesByType("resource").filter(entry => entry.name.endsWith(".cbor")).length; });
  window.installed = { inspect: () => book.inspect(), save: () => book.exportSave(), prefetch: () => book.nextContentUnits(), firstScreen: () => firstScreenRequests,
    close: async () => { controller.dispose(); root.unmount(); await book.close(); store.close(); } };
  if (!await controller.start()) throw new Error("Controller failed to open the screen");
  document.querySelector("#status").textContent = "ready";
})();
`);
// Build with the consumer's own Vite, then serve only its production output.
execFileSync(process.execPath, [join(directory, "node_modules/vite/bin/vite.js"), "build"], { cwd: directory, stdio: "inherit" });
const emitted = (await import("node:fs/promises")).readdir;
assert.ok((await emitted(join(directory, "dist/assets"))).some(name => name.endsWith(".wasm")), "Wasm must remain a separate request");
const port = 4193;
const server = createServer(async (request, response) => {
  try {
    const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
    if (path.includes("..")) { response.writeHead(400).end(); return; }
    const file = join(directory, "dist", path === "/" ? "index.html" : path);
    const body = await (await import("node:fs/promises")).readFile(file);
    response.setHeader("Content-Type", file.endsWith(".wasm") ? "application/wasm" : file.endsWith(".js") ? "text/javascript" : file.endsWith(".html") ? "text/html" : "application/octet-stream");
    response.end(body);
  } catch { response.writeHead(404).end(); }
});
await new Promise((done, fail) => { server.once("error", fail); server.listen(port, "127.0.0.1", done); });
let browser;
try {
  browser = await chromium.launch({ headless: true });
  const page = await browser.newPage();
  const errors = []; page.on("pageerror", error => errors.push(error.message));
  const requests = []; page.on("request", request => requests.push(new URL(request.url()).pathname));
  await page.goto(`http://127.0.0.1:${port}/`);
  await page.waitForFunction(() => document.querySelector("#status")?.textContent === "ready", { timeout: 30_000 });
  await page.evaluate(() => window.ready);
  assert.equal(await page.title(), "Narrata install test");
  await page.locator('[data-host-role="body"]').first().waitFor();
  assert.deepEqual(errors, []);
  assert.equal((await page.evaluate(() => window.installed.inspect())).view.depth, 0);
  assert.equal(await page.evaluate(() => window.installed.firstScreen()), 2, "First screen requests only the manifest and entry chunk before speculative prefetch");
  assert.equal(requests.filter(path => path.endsWith(".wasm")).length, 1);
  const beforePrefetch = await page.evaluate(() => window.installed.save());
  const units = await page.evaluate(() => window.installed.prefetch());
  assert.ok(units.some(reference => reference.key === "camp.fire"));
  assert.ok(units.some(reference => reference.key === "road.rocks"));
  assert.equal(await page.evaluate(() => window.installed.save()), beforePrefetch);
  await page.locator('button:has([data-host-role="option"])').first().press("Enter");
  await page.waitForFunction(async () => (await window.installed.inspect()).view.depth === 1);
  const before = await page.evaluate(() => window.installed.inspect());
  assert.equal(before.view.depth, 1);
  const save = await page.evaluate(() => window.installed.save());
  await page.evaluate(() => window.installed.close());
  await page.reload();
  await page.waitForFunction(() => document.querySelector("#status")?.textContent === "ready", { timeout: 30_000 });
  await page.evaluate(() => window.ready);
  const after = await page.evaluate(() => window.installed.inspect());
  assert.equal(after.view.cursor, before.view.cursor);
  assert.equal(await page.evaluate(() => window.installed.save()), save);
  assert.deepEqual(errors, []);
  await page.screenshot({ path: join(directory, "desktop-reader.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(directory, "mobile-reader.png"), fullPage: true });
  await page.evaluate(() => window.installed.close());
  console.log("Installed package: optional React peer, React slots, production Vite, first-screen requests, keyboard choice, durable save and reopen passed");
} finally {
  await browser?.close();
  await new Promise(done => server.close(done));
}
console.log(`Install evidence: ${directory}`);
