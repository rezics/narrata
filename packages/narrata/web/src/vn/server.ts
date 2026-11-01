import { readFile } from "node:fs/promises";
import { loadVnAuthoringFrom } from "./authoring.js";
import type { VnAuthoring } from "./authoring.js";
import type { WasmSource } from "../authoring/shared.js";

/** Node/Bun VN authoring entry; exposes only the explicit format-3 authoring facade. */
export function loadVnAuthoring(source?: WasmSource): Promise<VnAuthoring> {
  return loadVnAuthoringFrom(source ?? (async () => new Uint8Array(await readFile(new URL("../authoring-wasm/narrata_authoring_wasm_bg.wasm", import.meta.url)))));
}
