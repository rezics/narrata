// The browser authoring subpath. Import it dynamically, ideally in an editing or publishing
// Web Worker; reader routes never load it. Node/Bun hosts use @rezics/narrata/server.
import { loadAuthoringFrom, type Authoring, type WasmSource } from "./core.js";

export * from "./core.js";

/** Omitted input locates the separately shipped authoring Wasm next to this module. */
export function loadAuthoring(source?: WasmSource): Promise<Authoring> {
  return loadAuthoringFrom(source ?? new URL("../authoring-wasm/narrata_authoring_wasm_bg.wasm", import.meta.url));
}
