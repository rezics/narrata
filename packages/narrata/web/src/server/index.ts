// The Node/Bun authoring subpath: the same Wasm bytes as the browser subpath, read from this
// installed package with no browser globals, working-directory or repository paths.
import { readFile } from "node:fs/promises";
import { loadAuthoringFrom, type Authoring, type WasmSource } from "../authoring/core.js";

export * from "../authoring/core.js";

export function loadAuthoring(source?: WasmSource): Promise<Authoring> {
  return loadAuthoringFrom(source ?? (async () => new Uint8Array(await readFile(new URL("../authoring-wasm/narrata_authoring_wasm_bg.wasm", import.meta.url)))));
}
