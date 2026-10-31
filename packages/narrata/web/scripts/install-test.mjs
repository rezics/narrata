import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { strict as assert } from "node:assert";
import { chromium } from "@playwright/test";
import { createServer } from "node:http";
import { pathToFileURL } from "node:url";
import { verifyReferenceJourney, verifyUpdateScenarios } from "../../../../examples/gamebook-web/scripts/reference-journey.ts";
import { verifyMapJourney } from "./install/map-journey.mjs";
import { demoPack, execution, splitPack, objectId, vnPack } from "../test/fixture.ts";
import { npmCli, tarball } from "./pack.mjs";

const here = resolve(import.meta.dirname, "..");
const packageMetadata = JSON.parse(readFileSync(join(here, "package.json")));
const temporaryRoot = join(tmpdir(), ".temp");
mkdirSync(temporaryRoot, { recursive: true });
const directory = mkdtempSync(join(temporaryRoot, "narrata-web-install-"));
const parts = splitPack(demoPack);
const vnParts = splitPack(vnPack);
const npm = (...args) => execFileSync(process.execPath, [npmCli, ...args], { cwd: directory, stdio: "inherit" });
writeFileSync(join(directory, "package.json"), JSON.stringify({ name: "narrata-install-test", private: true, type: "module" }));
npm("install", "--ignore-scripts", "--no-audit", "--no-fund", "--save-exact", tarball, `vite@${packageMetadata.devDependencies.vite}`, "typescript@~5.9.3", "@types/node@^24.0.0");
const installed = join(directory, "node_modules/@rezics/narrata");
assert.equal(realpathSync(installed), installed, "Installation must be unpacked, not a workspace symlink");
const manifest = JSON.parse(readFileSync(join(installed, "package.json")));
assert.deepEqual(Object.keys(manifest.exports), [".", "./reader", "./vn", "./authoring", "./server", "./storage", "./react", "./testing", "./graph", "./graph/react", "./reader-map", "./reader-map/react", "./content/native"]);
assert.deepEqual(manifest.dependencies, { graphology: "0.26.0", sigma: "3.0.3" }, "Only the lazy graph entry uses runtime dependencies");
assert.deepEqual(manifest.peerDependenciesMeta, { react: { optional: true } });
assert.equal(existsSync(join(directory, "node_modules/react")), false, "Framework-free hosts must not install React");
const sha256 = bytes => createHash("sha256").update(bytes).digest("hex");
assert.ok(existsSync(join(installed, "dist/generated/vn-view.schema.json")), "The VN schema must ship in the tarball");
assert.ok(existsSync(join(installed, "dist/generated/validate-vn-view.js")), "The checked VN validator must ship in the tarball");
// Served Wasm is identified by content: reader, raw compatibility module or authoring.
const wasmKinds = new Map();
for (const [kind, path] of [["reader", "dist/reader-wasm/narrata_nodes_wasm_bg.wasm"], ["compat", "dist/wasm/narrata_nodes_wasm_bg.wasm"], ["authoring", "dist/authoring-wasm/narrata_authoring_wasm_bg.wasm"]]) {
  const hash = sha256(readFileSync(join(installed, path)));
  assert.equal(hash, sha256(readFileSync(join(here, path))), `${kind} Wasm differs from the build`);
  wasmKinds.set(hash, kind);
}
const publicDirectory = join(directory, "public");
mkdirSync(join(publicDirectory, "objects"), { recursive: true });
writeFileSync(join(publicDirectory, "manifest.cbor"), parts.manifest);
writeFileSync(join(publicDirectory, "story.narpack"), demoPack);
writeFileSync(join(publicDirectory, "vn-manifest.cbor"), vnParts.manifest);
writeFileSync(join(publicDirectory, "vn-story.narpack"), vnPack);
mkdirSync(join(publicDirectory, "vn-objects"), { recursive: true });
for (const [id, bytes] of vnParts.objects) writeFileSync(join(publicDirectory, "vn-objects", `${id.slice(7)}.cbor`), bytes);
for (const name of ["unseen-rule","republished-after-migration","proposal-retained","shallow-history"]) {
  cpSync(resolve(here,"../../../fixtures/compat/nodes-evolution-v1",name),join(publicDirectory,"maintenance-fixtures",name),{recursive:true});
}
cpSync(resolve(here,"../../../fixtures/compat/nodes-r2-proposals"),join(publicDirectory,"maintenance-fixtures/legacy"),{recursive:true});
for (const [id, bytes] of parts.objects) writeFileSync(join(publicDirectory, "objects", `${id.slice(7)}.cbor`), bytes);
// Extract immutable objects in Node. The independent consumers request each old manifest,
// provided names table and required chunk separately, never a historical full pack.
const { openBook: installedBook } = await import(pathToFileURL(join(installed, "dist/reader/index.js")).href);
const historyRoot = resolve(here, "../../../fixtures/compat/nodes-evolution-v1/republished-after-migration");
const historyPacks = ["ancestor", "source", "target"].map(name => new Uint8Array(readFileSync(join(historyRoot, `${name}.narpack`))));
const historyInput = { execution, current: "", checkpoint: readFileSync(join(historyRoot, "target.checkpoint")).toString("hex"), artifacts: [], steps: [], incoming: [] };
mkdirSync(join(publicDirectory, "historical"), { recursive: true });
for (const pack of historyPacks) {
  const probe = await installedBook({ pack, execution });
  const artifact = probe.artifactId; await probe.close();
  const parts = splitPack(pack);
  const objects = new Map([...parts.objects, [objectId(parts.names), parts.names], [`object:${artifact.slice(9)}`, parts.manifest]]);
  for (const [id, bytes] of objects) writeFileSync(join(publicDirectory, "historical", `${id.slice(7)}.cbor`), bytes);
  historyInput.artifacts.push({ artifact, manifest: `/historical/${artifact.slice(9)}.cbor`, names: `/historical/${objectId(parts.names).slice(7)}.cbor`,
    objects: Object.fromEntries([...parts.objects.keys()].map(id => [id, `/historical/${id.slice(7)}.cbor`])) });
  historyInput.current = artifact;
}
const historyReference = await installedBook({ pack: historyPacks[2], historyPacks: historyPacks.slice(0, 2), execution });
try {
  await historyReference.restore(historyInput.checkpoint);
  for (const row of (await historyReference.inspect()).view.history) {
    await historyReference.checkout(row.id);
    const before = await historyReference.inspect(), lookahead = await historyReference.nextContentUnits();
    const interaction = before.view.interaction;
    const choice = interaction.kind === "choose" ? { point: interaction.choice_point, options: interaction.options.filter(option => option.enabled).slice(0, Math.max(1, interaction.min)).map(option => option.id) } : undefined;
    if (choice) await historyReference.choose(row.id, choice.point, choice.options);
    historyInput.steps.push({ commit: row.id, before: { view: before.view, page: before.page }, lookahead, choice,
      after: await historyReference.inspect(), save: await historyReference.exportSave() });
  }
} finally { await historyReference.close(); }
for (const format of ["native", "legacy"]) {
  const root = resolve(historyRoot, "../old-save-continue");
  const save = format === "native" ? readFileSync(join(root, "source.checkpoint")).toString("hex") : readFileSync(join(root, "legacy.json"), "utf8");
  const reference = await installedBook({ pack: historyPacks[2], historyPacks: [historyPacks[0]], execution });
  try {
    await reference.restore(save); const expected = await reference.inspect();
    historyInput.incoming.push({ format, save, view: expected.view, page: expected.page, lookahead: await reference.nextContentUnits() });
  } finally { await reference.close(); }
}
writeFileSync(join(publicDirectory, "historical/input.json"), JSON.stringify(historyInput));
cpSync(resolve(here, "scripts/install/history-reader.mjs"), join(directory, "history-reader.mjs"));
writeFileSync(join(directory, "history-reader-node.mjs"), `import { readFile } from "node:fs/promises";
import { historyReaderJourney } from "./history-reader.mjs";
const input = JSON.parse(await readFile(new URL("./public/historical/input.json", import.meta.url), "utf8"));
const result = await historyReaderJourney(input, async path => new Uint8Array(await readFile(new URL("./public" + path, import.meta.url))));
console.log(JSON.stringify(result));`);
for (const runtime of [process.execPath, "bun"]) execFileSync(runtime, [join(directory, "history-reader-node.mjs")], { cwd: directory, stdio: "inherit" });
writeFileSync(join(directory, "objects.json"), JSON.stringify(Object.fromEntries([...parts.objects].map(([id, bytes]) => [id, [...bytes]]))));
writeFileSync(join(directory, "manifest.cbor"), parts.manifest);
cpSync(resolve(here, "test/types.ts"), join(directory, "types.ts"));
cpSync(resolve(here, "test/vn-types.ts"), join(directory, "vn-types.ts"));
cpSync(resolve(here, "scripts/install/source.ts"), join(directory, "source.ts"));
writeFileSync(join(directory, "native-types.ts"), `
import { createNativeContentAdapter, type NativeDocument } from "@rezics/narrata/content/native";
import { nativeDocumentSample, type SampleNativeValue } from "@rezics/narrata/testing";
const document: NativeDocument<SampleNativeValue> = nativeDocumentSample();
const adapter = createNativeContentAdapter<SampleNativeValue>({ resolve: async () => [{ status: "document", document }] });
void adapter;
`);
cpSync(resolve(here, "../../../fixtures/compat/graph-v1"), join(directory, "graph"), { recursive: true });
writeFileSync(join(directory, "tsconfig.json"), JSON.stringify({ compilerOptions: { target: "ES2022", module: "NodeNext", moduleResolution: "NodeNext", lib: ["ES2022", "DOM"], strict: true, skipLibCheck: true, noEmit: true }, include: ["types.ts", "vn-types.ts", "native-types.ts", "source.ts"] }));
execFileSync(process.execPath, [join(directory, "node_modules/typescript/bin/tsc"), "-p", join(directory, "tsconfig.json")], { cwd: directory, stdio: "inherit" });
writeFileSync(join(directory, "node.mjs"), `
import { readFile } from "node:fs/promises";
import { strict as assert } from "node:assert";
import { openBook, initialize, resolveContent } from "@rezics/narrata";
import { openVn } from "@rezics/narrata/vn";
import { MockRezicsContent, mountainLetterData, mountainLetterKeys, nativeDocumentSample } from "@rezics/narrata/testing";
import { createNativeContentAdapter, nativeContentOutline } from "@rezics/narrata/content/native";
import { decodeIndex, decodeTile } from "@rezics/narrata/graph";
const document = nativeDocumentSample("installed:1");
const nativeAdapter = createNativeContentAdapter({ resolve: async () => [{ status: "document", document }] });
const nativeRequest = { context: { languages: ["en"], realization: "edition:1", viewer: "reader:installed" }, items: [
  { content: { unit: { provider: "native", key: "chapter" }, first: "arrival", last: "map" } },
] };
const [nativeResolution] = await nativeAdapter.resolve(nativeRequest);
assert.equal(nativeResolution.status, "ok");
assert.deepEqual(nativeResolution.payload.blocks.map(block => block.id), ["arrival", "map"]);
assert.equal(nativeAdapter.lookup(nativeRequest.items[0], nativeRequest.context, nativeResolution).blocks[1].value.kind, "image");
assert.equal(nativeContentOutline("native", { chapter: document }).units.chapter.markers.decision, "choice-point:" + "1".repeat(32));
nativeAdapter.clear();
assert.equal(nativeAdapter.lookup(nativeRequest.items[0], nativeRequest.context, nativeResolution), undefined);
const graphManifest = JSON.parse(await readFile(new URL("./graph/manifest.json", import.meta.url), "utf8"));
const index = await decodeIndex(new Uint8Array(await readFile(new URL("./graph/" + graphManifest.index_object_id + ".cbor", import.meta.url))), graphManifest.index_object_id);
for (const [row, tile] of index.tiles.entries()) await decodeTile(new Uint8Array(await readFile(new URL("./graph/" + tile.objectId + ".cbor", import.meta.url))), index, row);
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
const vnPack = new Uint8Array(await readFile(new URL("./public/vn-story.narpack", import.meta.url)));
const vnRequests = [];
const vn = await openVn({
  manifest: new Uint8Array(await readFile(new URL("./public/vn-manifest.cbor", import.meta.url))),
  execution: ${JSON.stringify(execution)},
  fetchChunk: async id => { vnRequests.push(id); return new Uint8Array(await readFile(new URL("./public/vn-objects/" + id.slice(7) + ".cbor", import.meta.url))); },
});
let vnSave;
try {
  const start = await vn.inspect();
  assert.equal(start.version, 1); assert.equal(start.view.interaction.kind, "advance");
  assert.equal(start.view.scene.music.key, "music/theme"); assert.equal(vnRequests.length, 1);
  await vn.advance(start.view.cursor, start.view.interaction.dialogue);
  vnSave = await vn.exportSave();
} finally { await vn.close(); }
const coldVn = await openVn({ pack: vnPack, execution: ${JSON.stringify(execution)} });
try {
  await coldVn.restore(vnSave);
  const reopened = await coldVn.inspect();
  assert.equal(reopened.view.artifact_id, coldVn.artifactId);
  assert.equal(reopened.view.depth, 1); assert.equal(reopened.view.interaction.kind, "advance");
  assert.equal(reopened.view.interaction.line.unit.key, "dialogue.two");
} finally { await coldVn.close(); }
console.log("Installed package opened in " + process.argv[2]);
`);
cpSync(resolve(here, "scripts/install/journey.mjs"), join(directory, "journey.mjs"));
cpSync(resolve(here, "scripts/install/maps.mjs"), join(directory, "maps.mjs"));
cpSync(resolve(here, "../../../fixtures/compat/reader-map-v1"), join(publicDirectory, "reader-map-fixtures"), { recursive: true });
mkdirSync(join(publicDirectory, "reader-map-fixtures/extra"), { recursive: true });
cpSync(resolve(here, "../../../fixtures/compat/nodes-r2-proposals"), join(publicDirectory, "reader-map-fixtures/extra/proposals"), { recursive: true });
cpSync(resolve(here, "../../../fixtures/compat/nodes-evolution-v1/deleted-lower-call"), join(publicDirectory, "reader-map-fixtures/extra/evolution"), { recursive: true });
writeFileSync(join(directory, "authoring.mjs"), `
import { readFile } from "node:fs/promises";
import { strict as assert } from "node:assert";
import { loadAuthoring, referenceRelationsSql } from "@rezics/narrata/server";
import { openBook } from "@rezics/narrata/reader";
import { shortStorySource, shortStoryOutline } from "@rezics/narrata/testing";
import { author, read } from "./journey.mjs";
import { constructAndEdit } from "./source.ts";
import { verifyMaps } from "./maps.mjs";
const wasm = new URL("./node_modules/@rezics/narrata/dist/authoring-wasm/narrata_authoring_wasm_bg.wasm", import.meta.url);
const mode = process.argv[2];
assert.equal(await readFile(referenceRelationsSql, "utf8"), ${JSON.stringify(readFileSync(resolve(here, "../tooling/crates/narrata-authoring/sql/relations-v1.sql"), "utf8"))});
// "browser-entry" proves the browser subpath also locates its Wasm from a file URL.
const api = mode === "browser-entry" ? await (await import("@rezics/narrata/authoring")).loadAuthoring()
  : await loadAuthoring(mode === "bytes" ? new Uint8Array(await readFile(wasm)) : mode === "url" ? wasm : undefined);
constructAndEdit(api);
await verifyMaps(api, async name => new Uint8Array(await readFile(new URL("./public/reader-map-fixtures/" + name, import.meta.url))));
const published = await author(api, { shortStorySource, shortStoryOutline });
const journeys = await read(openBook, ${JSON.stringify(execution)}, published);
assert.equal(journeys.firstScreen, 1, "A's first screen fetches only the entry chunk");
assert.notEqual(journeys.artifacts[0], journeys.artifacts[1]);
console.log("Installed authoring published A and B in " + mode + ": " + JSON.stringify(journeys));
`);
for (const mode of ["default", "url", "bytes"]) {
  execFileSync(process.execPath, [join(directory, "node.mjs"), mode], { cwd: directory, stdio: "inherit" });
  execFileSync(process.execPath, [join(directory, "authoring.mjs"), mode], { cwd: directory, stdio: "inherit" });
}
execFileSync(process.execPath, [join(directory, "authoring.mjs"), "browser-entry"], { cwd: directory, stdio: "inherit" });
// Bun is already part of the repository toolchain; test its file-URL loaders too.
execFileSync("bun", [join(directory, "node.mjs"), "default"], { cwd: directory, stdio: "inherit" });
execFileSync("bun", [join(directory, "authoring.mjs"), "default"], { cwd: directory, stdio: "inherit" });

npm("install", "--ignore-scripts", "--no-audit", "--no-fund", "--save-exact", "react@19.2.8", "react-dom@19.2.8", "@types/react@^19.0.0", "@types/react-dom@^19.0.0", `vitest@${packageMetadata.devDependencies.vitest}`, `fake-indexeddb@${packageMetadata.devDependencies["fake-indexeddb"]}`);
cpSync(resolve(here,"scripts/install/maintenance.mjs"),join(directory,"maintenance.mjs"));
writeFileSync(join(directory,"maintenance-node.mjs"), `
import "fake-indexeddb/auto";
import { readFile, writeFile } from "node:fs/promises";
import { loadAuthoring } from "@rezics/narrata/server";
import { verifyMaintenance, verifyHistoricalMaintenance } from "./maintenance.mjs";
const api = await loadAuthoring();
const evidence = {journey:await verifyMaintenance(api, new Uint8Array(await readFile(new URL("./public/story.narpack",import.meta.url)))),history:await verifyHistoricalMaintenance(api,async name=>{
  try {return new Uint8Array(await readFile(new URL("./public/maintenance-fixtures/"+name,import.meta.url)));}
  catch(error) {if(error.code === "ENOENT") return undefined;throw error;}
})};
await writeFile(new URL("./maintenance-" + (globalThis.Bun ? "bun" : "node") + ".json",import.meta.url),JSON.stringify(evidence,null,2));
console.log("Installed maintenance",evidence);
`);
execFileSync(process.execPath,[join(directory,"maintenance-node.mjs")],{cwd:directory,stdio:"inherit"});
execFileSync("bun",[join(directory,"maintenance-node.mjs")],{cwd:directory,stdio:"inherit"});
// The fault matrix imports only the unpacked public entries, never workspace implementations.
cpSync(resolve(here, "test/online.test.ts"), join(directory, "online.test.ts"));
cpSync(resolve(here, "test/maintenance.test.ts"), join(directory, "maintenance.test.ts"));
writeFileSync(join(directory, "story.narpack"), demoPack);
writeFileSync(join(directory, "fixture.ts"), readFileSync(resolve(here, "test/fixture.ts"), "utf8").replace("../../../../products/gamebook-demo/story.narpack", "story.narpack"));
mkdirSync(join(directory, "fixtures/lazy-vn"), { recursive: true });
writeFileSync(join(directory, "fixtures/lazy-vn/story.narpack"), readFileSync(resolve(here, "test/fixtures/lazy-vn/story.narpack")));
execFileSync(process.execPath, [join(directory, "node_modules/vitest/vitest.mjs"), "run", "online.test.ts", "maintenance.test.ts"], { cwd: directory, stdio: "inherit" });
cpSync(resolve(here, "test/reader-types.tsx"), join(directory, "reader-types.tsx"));
cpSync(resolve(here, "test/graph-types.tsx"), join(directory, "graph-types.tsx"));
// Run the reference host unchanged against the unpacked public tarball.
cpSync(resolve(here, "../../../examples/gamebook-web/src/online.tsx"), join(directory, "online.tsx"));
cpSync(resolve(here, "../../../examples/gamebook-web/src/reference.tsx"), join(directory, "reference.tsx"));
for (const name of ["reference-host.ts", "reference-story.ts", "reference-content.ts"]) cpSync(resolve(here, "../../../examples/gamebook-web/scripts", name), join(directory, name));
writeFileSync(join(directory, "tsconfig.json"), JSON.stringify({ compilerOptions: { target: "ES2022", module: "NodeNext", moduleResolution: "NodeNext", lib: ["ES2022", "DOM"], strict: true, skipLibCheck: true, noEmit: true, allowImportingTsExtensions: true, jsx: "react-jsx" }, include: ["types.ts", "native-types.ts", "reader-types.tsx", "graph-types.tsx", "reference-host.ts", "reference-story.ts", "reference-content.ts", "online.tsx", "reference.tsx"] }));
execFileSync(process.execPath, [join(directory, "node_modules/typescript/bin/tsc"), "-p", join(directory, "tsconfig.json")], { cwd: directory, stdio: "inherit" });

writeFileSync(join(directory, "vite.config.js"), 'export default { worker: { format: "es" } };\n');
writeFileSync(join(directory, "worker.js"), `
import { author } from "./journey.mjs";
import { verifyMaps } from "./maps.mjs";
self.onmessage = async () => {
  try {
    // The authoring subpath is imported only here, in the editing/publishing Worker.
    const [{ loadAuthoring }, testing] = await Promise.all([import("@rezics/narrata/authoring"), import("@rezics/narrata/testing")]);
    const api = await loadAuthoring();
    await verifyMaps(api, async name => new Uint8Array(await (await fetch("/reader-map-fixtures/" + name)).arrayBuffer()));
    self.postMessage({ ok: true, published: await author(api, testing) });
  } catch (error) { self.postMessage({ ok: false, error: String(error?.stack ?? error) }); }
};
`);
writeFileSync(join(directory, "index.html"), '<!doctype html><html><head><title>Narrata install test</title></head><body><p id="status">loading</p><div id="reader"></div><script type="module" src="/main.tsx"></script></body></html>');
writeFileSync(join(directory, "main.tsx"), `
import { openBook, createReader } from "@rezics/narrata";
import { read } from "./journey.mjs";
import { IndexedDbStore } from "@rezics/narrata/storage";
import { Reader } from "@rezics/narrata/react";
import { createRoot } from "react-dom/client";
async function bytes(url) { const response = await fetch(url); if (!response.ok) throw new Error("HTTP " + response.status); return new Uint8Array(await response.arrayBuffer()); }
window.ready = new URLSearchParams(location.search).has("reference") ? (async () => {
  document.title = "Narrata cold reference journey";
  document.querySelector("#status")?.remove();
  const { mountColdReference } = await import("./reference.tsx");
  await mountColdReference(document.querySelector("#reader"));
})() : (async () => {
  const store = await IndexedDbStore.open("installed-package-session");
  const book = await openBook({ manifest: await bytes("/manifest.cbor"), execution: ${JSON.stringify(execution)}, storage: store,
    fetchChunk: id => bytes("/objects/" + id.slice(7) + ".cbor") });
  let failContent = false;
  const controller = createReader(book, { resolve: async request => {
    if (failContent) { failContent = false; throw new Error("content service unavailable"); }
    return request.items.map(() => ({ status: "ok", revision: "host", payload: { text: "HOST CONTENT" } }));
  } }, { persistence: "durable", context: { languages: ["en"] } });
  const slot = props => <span data-host-role={props.role} data-realization={props.context.realization}>{props.resolution.status === "ok" ? props.resolution.payload.text : props.resolution.status}</span>;
  const root = createRoot(document.querySelector("#reader"));
  root.render(<Reader controller={controller} slots={{ body: slot, option: slot, reference: slot }} />);
  let firstScreenRequests; let firstScreenUrls;
  controller.subscribe(() => { if (firstScreenRequests === undefined && controller.getSnapshot().phase === "ready") {
    firstScreenUrls = performance.getEntriesByType("resource").filter(entry => entry.name.endsWith(".cbor")).map(entry => entry.name);
    firstScreenRequests = firstScreenUrls.length;
  } });
  window.installed = { inspect: () => book.inspect(), save: () => book.exportSave(), prefetch: () => book.nextContentUnits(), firstScreen: () => firstScreenRequests, firstScreenUrls: () => firstScreenUrls,
    history: async () => { const { historyReaderJourney } = await import("./history-reader.mjs"); return historyReaderJourney(await (await fetch("/historical/input.json")).json(), bytes); },
    maintenance: async () => {
      const [{loadAuthoring},{verifyMaintenance,verifyHistoricalMaintenance}] = await Promise.all([import("@rezics/narrata/authoring"),import("./maintenance.mjs")]);
      const api = await loadAuthoring();
      return {journey:await verifyMaintenance(api,await bytes("/story.narpack")),history:await verifyHistoricalMaintenance(api,async name=>{
        const reply = await fetch("/maintenance-fixtures/"+name);return reply.ok ? new Uint8Array(await reply.arrayBuffer()) : undefined;
      })};
    },
    failContentOnce: async () => { failContent = true; const failed = await controller.refresh(); const phase = controller.getSnapshot().phase; const retried = await controller.retry(); return { failed, phase, retried, after: controller.getSnapshot().phase }; },
    vn: async () => {
      const { openVn } = await import("@rezics/narrata/vn");
      const requested = [];
      const novel = await openVn({ manifest: await bytes("/vn-manifest.cbor"), execution: ${JSON.stringify(execution)}, fetchChunk: async id => {
        requested.push(id); return bytes("/vn-objects/" + id.slice(7) + ".cbor");
      } });
      let saved;
      let first;
      try {
        first = await novel.inspect();
        await novel.advance(first.view.cursor, first.view.interaction.dialogue);
        saved = await novel.exportSave();
      } finally { await novel.close(); }
      const cold = await openVn({ pack: await bytes("/vn-story.narpack"), execution: ${JSON.stringify(execution)} });
      try {
        await cold.restore(saved);
        const reopened = await cold.inspect();
        return { version: first.version, artifact: reopened.view.artifact_id, cursor: reopened.view.cursor, depth: reopened.view.depth,
          line: reopened.view.interaction.kind === "advance" ? reopened.view.interaction.line.unit.key : "wrong-interaction", requested: requested.length };
      } finally { await cold.close(); }
    },
    author: () => new Promise((done, fail) => {
      const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
      worker.onerror = event => { worker.terminate(); fail(new Error(event.message)); };
      worker.onmessage = async ({ data }) => {
        worker.terminate();
        if (!data.ok) { fail(new Error(data.error)); return; }
        try { done(await read(openBook, ${JSON.stringify(execution)}, data.published)); } catch (error) { fail(error); }
      };
      worker.postMessage("publish");
    }),
    native: async () => {
      const { createNativeContentAdapter, nativeContentOutline } = await import("@rezics/narrata/content/native");
      const { nativeDocumentSample } = await import("@rezics/narrata/testing");
      const document = nativeDocumentSample("browser:1");
      const adapter = createNativeContentAdapter({ resolve: async () => [{ status: "document", document }] });
      const request = { context: { languages: ["en"] }, items: [{ content: { unit: { provider: "native", key: "chapter" }, first: "arrival", last: "map" } }] };
      const [resolution] = await adapter.resolve(request);
      const slice = adapter.lookup(request.items[0], request.context, resolution);
      return { status: resolution.status, ids: slice?.blocks.map(block => block.id), image: slice?.blocks[1]?.value.kind,
        marker: nativeContentOutline("native", { chapter: document }).units.chapter.markers.decision };
    },
    online: async () => {
      controller.dispose(); root.unmount(); await book.close(); store.close();
      const { mountOnlineReference } = await import("./online.tsx");
      window.onlineReference = await mountOnlineReference(document.querySelector("#reader"));
    },
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
const served = [];
// Loaded from the unpacked consumer, this Node host publishes through its installed server API.
const { createReferenceHost } = await import(pathToFileURL(join(directory, "reference-host.ts")).href);
let referenceHost = await createReferenceHost();
const server = createServer(async (request, response) => {
  try {
    if (await referenceHost.handle(request, response)) return;
    const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
    if (path.includes("..")) { response.writeHead(400).end(); return; }
    const file = join(directory, "dist", path === "/" ? "index.html" : path);
    const body = await (await import("node:fs/promises")).readFile(file);
    if (file.endsWith(".wasm")) served.push(wasmKinds.get(sha256(body)) ?? "unknown");
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
  const consoleMessages = []; page.on("console", message => { if (["error", "warning"].includes(message.type())) consoleMessages.push(message.text()); });
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
  assert.deepEqual(served, ["reader"], "The reader route fetches only the reader Wasm: no authoring or compatibility module");
  writeFileSync(join(directory, "first-screen-requests.json"), JSON.stringify({ cbor: await page.evaluate(() => window.installed.firstScreenUrls()), wasm: requests.filter(path => path.endsWith(".wasm")), requests: [...requests] }, null, 2));
  const vnEvidence = await page.evaluate(() => window.installed.vn());
  assert.equal(vnEvidence.version, 1); assert.equal(vnEvidence.depth, 1); assert.equal(vnEvidence.line, "dialogue.two");
  assert.equal(vnEvidence.artifact, "artifact:e4833208cb0e335d55cdfbc27cee38568bc916f05c60337f14c028f0a4b72b06");
  assert.equal(vnEvidence.requested, 1);
  assert.deepEqual(served, ["reader"], "The VN entry reuses the package reader Wasm and adds no authoring or compatibility module request");
  assert.deepEqual(requests.filter(path => /sigma|graphology|renderer|graph/i.test(path)), [], "The first screen loads no graph code or tiles");
  assert.deepEqual(await page.evaluate(() => window.installed.native()), { status: "ok", ids: ["arrival", "map"], image: "image", marker: "choice-point:" + "1".repeat(32) });
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
  const coldContext = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const authoringBytes = readFileSync(join(installed, "dist/authoring-wasm/narrata_authoring_wasm_bg.wasm"));
  const authoringWasm = { bytes: authoringBytes.length, sha256: sha256(authoringBytes) };
  try {
    const bytes = readFileSync(join(installed, "dist/reader-wasm/narrata_nodes_wasm_bg.wasm"));
    await verifyReferenceJourney(await coldContext.newPage(), `http://127.0.0.1:${port}`, join(directory, "cold-reference"), { bytes: bytes.length, sha256: sha256(bytes) }, { authoringWasm });
    const readerBytes = readFileSync(join(installed, "dist/reader-wasm/narrata_nodes_wasm_bg.wasm"));
    await verifyUpdateScenarios(await coldContext.newPage(), `http://127.0.0.1:${port}`, join(directory, "visible-updates"),
      { readerWasm: { bytes: readerBytes.length, sha256: sha256(readerBytes) }, authoringWasm });
    await verifyMapJourney(await coldContext.newPage(), `http://127.0.0.1:${port}`, join(directory, "reader-map"));
  } finally { await coldContext.close(); }
  // Repeat the complete public-page journey on a narrow viewport against a freshly
  // authored A/B/C catalog, still using only this independently installed tarball.
  referenceHost.close(); referenceHost = await createReferenceHost();
  const mobileContext = await browser.newContext({ viewport: { width: 390, height: 844 } });
  try {
    const bytes = readFileSync(join(installed, "dist/reader-wasm/narrata_nodes_wasm_bg.wasm"));
    await verifyReferenceJourney(await mobileContext.newPage(), `http://127.0.0.1:${port}`, join(directory, "cold-reference-mobile"),
      { bytes: bytes.length, sha256: sha256(bytes) }, { mobile: true, authoringWasm });
    await verifyUpdateScenarios(await mobileContext.newPage(), `http://127.0.0.1:${port}`, join(directory, "visible-updates-mobile"),
      { mobile: true, readerWasm: { bytes: bytes.length, sha256: sha256(bytes) }, authoringWasm });
  } finally { await mobileContext.close(); }
  await page.reload();
  await page.waitForFunction(() => document.querySelector("#status")?.textContent === "ready", { timeout: 30_000 });
  await page.evaluate(() => window.ready);
  const after = await page.evaluate(() => window.installed.inspect());
  assert.equal(after.view.cursor, before.view.cursor);
  assert.equal(await page.evaluate(() => window.installed.save()), save);
  assert.deepEqual(errors, []);
  assert.deepEqual(await page.evaluate(() => window.installed.failContentOnce()), { failed: false, phase: "error", retried: true, after: "ready" }, "A failed content load is retried without losing progress");
  assert.equal((await page.evaluate(() => window.installed.inspect())).view.cursor, after.view.cursor);
  const authored = await page.evaluate(() => window.installed.author());
  assert.equal(authored.firstScreen, 1, "Published A's first screen fetches only the entry chunk");
  assert.ok(served.includes("authoring") && !served.includes("compat"), "The Worker loads the authoring Wasm on demand; nothing loads the compatibility module");
  const maintenance = await page.evaluate(() => window.installed.maintenance());
  const historical = await page.evaluate(() => window.installed.history());
  writeFileSync(join(directory, "provided-history-names.json"), JSON.stringify(historical, null, 2));
  writeFileSync(join(directory,"maintenance-browser.json"),JSON.stringify(maintenance,null,2));
  assert.deepEqual(errors, []);
  writeFileSync(join(directory, "browser-evidence.json"), JSON.stringify({ served, authored, vn: vnEvidence }, null, 2));
  await page.screenshot({ path: join(directory, "desktop-reader.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(directory, "mobile-reader.png"), fullPage: true });
  // Public authoring -> host prose/labels -> online persistence -> exact A/B restoration.
  await page.setViewportSize({ width: 1280, height: 900 });
  await page.evaluate(() => window.installed.online());
  await page.getByText(/At dusk, Mara found a lantern/).waitFor();
  const initialOnline = await page.evaluate(() => window.onlineReference.getSnapshot().binding.session.getSnapshot());
  assert.equal(initialOnline.status, "saved");
  await page.getByText("Simulate a save failure", { exact: true }).click();
  await page.getByRole("button", { name: "Lose the next save response", exact: true }).click();
  await page.getByRole("button", { name: "Follow the light into the forest", exact: true }).click();
  await page.getByRole("button", { name: "Retry and verify save", exact: true }).waitFor();
  assert.equal(await page.getByText("Story finished", { exact: true }).count(), 0);
  assert.equal((await page.evaluate(() => window.onlineReference.getSnapshot().binding.session.getSnapshot())).progress.view.cursor, initialOnline.progress.view.cursor);
  await page.screenshot({ path: join(directory, "online-unknown.png"), fullPage: true });
  await page.getByRole("button", { name: "Retry and verify save", exact: true }).click();
  await page.getByText(/Under the cedar branches/).waitFor();
  const recovered = await page.evaluate(() => window.onlineReference.getSnapshot().binding.session.getSnapshot());
  assert.equal(recovered.status, "saved"); assert.equal(recovered.progress.view.depth, 1);
  const retryWrites = await page.evaluate(() => window.onlineReference.evidence().writes);
  assert.deepEqual(retryWrites.at(-1), retryWrites.at(-2), "Unknown save retries exactly the original bytes");
  await page.getByRole("button", { name: "Publish B", exact: true }).click();
  await page.getByRole("button", { name: "Reopen this journey", exact: true }).click();
  await page.getByRole("button", { name: "Seek the second light", exact: true }).waitFor();
  const resumedOnline = await page.evaluate(() => window.onlineReference.getSnapshot().binding.session.getSnapshot());
  assert.equal(resumedOnline.progress.view.artifact_id, initialOnline.progress.view.artifact_id);
  assert.equal(resumedOnline.progress.view.cursor, recovered.progress.view.cursor);
  await page.getByRole("button", { name: "Start a new journey", exact: true }).click();
  await page.getByText(/At dusk, Mara found a lantern/).waitFor();
  await page.getByRole("button", { name: "Reject the next save: storage full", exact: true }).click();
  await page.getByRole("button", { name: "Follow the light into the forest", exact: true }).click();
  await page.getByText("Storage is full. Your last saved progress is preserved.", { exact: true }).waitFor();
  assert.equal((await page.evaluate(() => window.onlineReference.getSnapshot().binding.session.getSnapshot())).progress.view.depth, 0);
  await page.getByRole("button", { name: "Reload saved progress", exact: true }).click();
  await page.getByRole("button", { name: "Follow the light into the forest", exact: true }).click();
  await page.getByRole("button", { name: "Return home with the keeper", exact: true }).waitFor();
  assert.equal(await page.getByRole("button", { name: "Seek the second light", exact: true }).count(), 0);
  const onB = await page.evaluate(() => window.onlineReference.getSnapshot().binding.session.getSnapshot());
  assert.notEqual(onB.progress.view.artifact_id, initialOnline.progress.view.artifact_id);
  // An old account's successful response arrives after the new account is already reading.
  await page.evaluate(() => {
    const host = window.onlineReference; const { session, controller } = host.getSnapshot().binding;
    const view = session.getSnapshot().progress.view;
    host.setFault("late"); window.oldOnlineSession = session;
    window.lateOnlineChoice = controller.choose([view.interaction.options[0].id]);
  });
  await page.waitForFunction(() => window.onlineReference.hasLate());
  await page.getByRole("button", { name: "Switch account", exact: true }).click();
  await page.getByText("Account: reader-two", { exact: false }).waitFor();
  await page.evaluate(() => window.onlineReference.releaseLate());
  assert.equal(await page.evaluate(() => window.lateOnlineChoice), false);
  assert.deepEqual(await page.evaluate(() => window.oldOnlineSession.getSnapshot()), { status: "closed", progress: null, candidate: null });
  assert.equal((await page.evaluate(() => window.onlineReference.getSnapshot().binding.session.getSnapshot())).progress.view.depth, 0);
  // Delete the observed instance, create a new ID, then attempt a late retry on the old handle.
  const beforeDelete = await page.evaluate(() => {
    const host = window.onlineReference; window.deletedOnlineSession = host.getSnapshot().binding.session;
    return { writes: host.evidence().writes.length, id: host.libraryId() };
  });
  await page.evaluate(() => window.onlineReference.remove());
  const afterDelete = await page.evaluate(() => window.onlineReference.evidence().writes.length);
  assert.ok(afterDelete > beforeDelete.writes, "New library creates its own durable root");
  assert.notDeepEqual(await page.evaluate(() => window.onlineReference.libraryId()), beforeDelete.id, "Recreated library has a new StoreId");
  assert.equal(await page.evaluate(async () => { try { await window.deletedOnlineSession.retrySave(); return false; } catch { return true; } }), true);
  assert.equal(await page.evaluate(() => window.onlineReference.evidence().writes.length), afterDelete);
  const onlineEvidence = await page.evaluate(() => ({ ...window.onlineReference.evidence(), snapshot: window.onlineReference.getSnapshot().binding.session.getSnapshot() }));
  writeFileSync(join(directory, "online-evidence.json"), JSON.stringify(onlineEvidence, null, 2));
  writeFileSync(join(directory, "browser-console.json"), JSON.stringify({ errors, consoleMessages }, null, 2));
  await page.screenshot({ path: join(directory, "online-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.screenshot({ path: join(directory, "online-mobile.png"), fullPage: true });
  assert.deepEqual(errors, []);
  assert.equal(await page.locator("vite-error-overlay").count(), 0);
  await page.evaluate(() => window.onlineReference.dispose());
  await page.evaluate(() => window.installed.close());
  console.log("Installed package: optional React peer, production Vite reader, first-screen requests, content retry, Worker authoring, real short-story prose/labels, online save verification, A/B recovery, quota, late account responses and deletion/recreation passed");
} finally {
  await browser?.close();
  await new Promise(done => server.close(done));
  referenceHost.close();
}
console.log(`Install evidence: ${directory}`);
