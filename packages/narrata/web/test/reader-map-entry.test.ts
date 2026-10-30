import { resolve } from "node:path";
import { expect, test, vi } from "vitest";
import { build } from "vite";

test("direct reader-map entries import no author graph, compiler or Wasm initialization", async () => {
  const initialize = vi.spyOn(WebAssembly, "instantiate");
  try {
    await import("@rezics/narrata/reader-map");
    await import("@rezics/narrata/reader-map/react");
    expect(initialize).not.toHaveBeenCalled();
  } finally { initialize.mockRestore(); }
  // Disable tree shaking so a mistaken barrel import cannot conceal a forbidden module.
  const result = await build({ configFile: false, logLevel: "error", build: { write: false,
    lib: { entry: { decoder: resolve(import.meta.dirname, "../dist/graph/reader-map.js"), react: resolve(import.meta.dirname, "../dist/graph/reader-map-react.js") }, formats: ["es"] },
    rolldownOptions: { external: ["react", "react/jsx-runtime"], treeshake: false } } });
  const modules = (Array.isArray(result) ? result : [result]).flatMap(output => {
    if (!("output" in output)) throw new Error("Expected a one-shot bundle, not a watcher");
    return output.output;
  }).flatMap(output => output.type === "chunk" ? Object.keys(output.modules) : []).map(path => path.replaceAll("\\", "/"));
  expect(modules.some(path => /\/(?:authoring|authoring-wasm|reader-wasm|wasm)\//.test(path))).toBe(false);
  expect(modules.some(path => /\/graph\/(?:index|react|controller|wire|renderer)\.js$/.test(path))).toBe(false);
  expect(modules.some(path => /\/node_modules\/(?:sigma|graphology)(?:\/|$)/.test(path))).toBe(false);
  expect(modules.some(path => path.endsWith("/validate-reader-map.js"))).toBe(true);
});
