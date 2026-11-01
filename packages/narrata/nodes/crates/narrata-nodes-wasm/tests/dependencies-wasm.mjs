// Replays fixtures/compat/vn-dependencies-v1 through the generated Wasm module and compares
// every request and result byte for byte. Usage:
//   node dependencies-wasm.mjs <directory holding narrata_nodes_wasm.js and its .wasm>
// The directory is the wasm-bindgen `--target web` output of the reader build
// (`cargo build --profile wasm -p narrata-nodes-wasm --no-default-features
// --features vn-dependencies --target wasm32-unknown-unknown`); nothing here writes to the corpus.
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { pathToFileURL } from "node:url";

const [, , wasmDirectory] = process.argv;
if (!wasmDirectory) throw new Error("usage: dependencies-wasm.mjs <wasm-bindgen output directory>");
const corpus = resolve(import.meta.dirname, "../../../../../../fixtures/compat/vn-dependencies-v1");
const bytes = (name) => new Uint8Array(readFileSync(resolve(corpus, name)));
const text = (name) => readFileSync(resolve(corpus, name), "utf8");
const glue = await import(pathToFileURL(resolve(wasmDirectory, "narrata_nodes_wasm.js")).href);
glue.initSync({ module: new Uint8Array(readFileSync(resolve(wasmDirectory, "narrata_nodes_wasm_bg.wasm"))) });
const { VnBook } = glue;

const scenarios = JSON.parse(text("scenarios.json"));
const objects = JSON.parse(text("objects.json"));
const execution = scenarios.execution;
const pack = bytes("story.narpack");

const apply = (book, step) => {
  const { view } = JSON.parse(book.inspect());
  if (step === "advance") return book.advance(view.cursor, view.interaction.dialogue);
  const ids = step.choose.map((key) => view.interaction.options.find((option) => option.key === key).id);
  return book.choose(view.cursor, view.interaction.choice_point, ids);
};

const failure = (call) => {
  try {
    call();
  } catch (error) {
    return String(error instanceof Error ? error.message : error);
  }
  throw new Error("expected the call to fail");
};

let queries = 0;
for (const scenario of scenarios.scenarios) {
  const full = new VnBook(pack, execution);
  full.memory();
  for (const step of scenario.steps) apply(full, step);
  let book = full;
  if (scenario.mode === "manifest") {
    const exported = full.export();
    book = VnBook.fromManifest(bytes("story.manifest"), execution);
    book.loadChunk(objects["main.start"].id, bytes(objects["main.start"].file));
    book.memory();
    book.restore(exported);
    full.free();
  }
  for (const query of scenario.queries) {
    for (const graph of query.supply ?? []) book.loadChunk(objects[graph].id, bytes(objects[graph].file));
    const request = text(`${scenario.name}.${query.name}.request.json`);
    assert.equal(JSON.parse(request).cursor, book.cursor());
    const expected = text(`${scenario.name}.${query.name}.result.json`);
    assert.equal(book.dependencies(request), expected, `${scenario.name}.${query.name}`);
    // A repeat is identical, and the query leaves no program request for the host to fetch.
    assert.equal(book.dependencies(request), expected);
    assert.equal(book.takeProgramRequest() ?? null, null, `${scenario.name}.${query.name}: spurious request`);
    assert.equal(book.takeChunkRequest() ?? null, null);
    queries += 1;
  }
  if (scenario.mode === "manifest") {
    // The optional query never blocks the next input: the mandatory fetch comes from the input.
    const { view } = JSON.parse(book.inspect());
    assert.equal(view.interaction.kind, "choose");
  }
  book.free();
}

// Checked boundaries: every rejected document fails with its code, and stale or closed use fails.
const book = new VnBook(pack, execution);
book.memory();
const good = JSON.parse(text("choice.default.request.json"));
for (const rejected of JSON.parse(text("rejected-requests.json"))) {
  assert.match(failure(() => book.dependencies(rejected.text)), new RegExp(`^${rejected.code} at`), rejected.name);
}
assert.match(failure(() => book.dependencies(JSON.stringify(good))), /^stale_input at/);
book.reload();
assert.match(failure(() => book.dependencies(JSON.stringify(good))), /^state at/);
book.free();
console.log(`vn-dependencies-v1: ${queries} queries byte-identical through the generated Wasm module`);
