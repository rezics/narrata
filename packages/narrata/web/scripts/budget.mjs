import { readFileSync, readdirSync } from "node:fs";
import { resolve, join, relative } from "node:path";
import { gzipSync } from "node:zlib";
import { build } from "vite";

const here = resolve(import.meta.dirname, "..");
function files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? files(join(directory, entry.name)) : [join(directory, entry.name)]);
}
const wasm = path => { const bytes = readFileSync(resolve(here, "dist", path)); return { raw: bytes.length, gzip: gzipSync(bytes, { level: 9 }).length }; };
const javascript = files(resolve(here, "dist")).filter(path => path.endsWith(".js"));
function measure(paths) {
  const bytes = paths.map(path => readFileSync(path));
  return { raw: bytes.reduce((sum, item) => sum + item.length, 0), gzip: bytes.reduce((sum, item) => sum + gzipSync(item, { level: 9 }).length, 0) };
}
const modulePath = path => relative(resolve(here, "dist"), path).replaceAll("\\", "/");
const group = path => modulePath(path).split("/")[0];
const nativeModules = new Set(["content/native.js", "testing/native-document.js"]);
const isNativeContent = path => nativeModules.has(modulePath(path));
// Authoring and server entries, their Wasm glue, generated types and sample; never on a reader route.
const isAuthoring = path => ["authoring", "server", "authoring-wasm"].includes(group(path)) || ["generated/authoring.js", "testing/authoring.js"].includes(modulePath(path));
// The raw dist/wasm module keeps legacy publishGraph; no package entry imports it.
const isCompat = path => group(path) === "wasm";
// Measure both independent optional capabilities with identical bundling settings. Every
// transitive module is included; only the host's optional React peer stays external.
async function bundled(entry) {
  const result = await build({ configFile: false, logLevel: "error", build: { write: false, minify: true,
    lib: { entry: Object.fromEntries(Object.entries(entry).map(([name, path]) => [name, resolve(here, path)])), formats: ["es"] },
    rolldownOptions: { external: ["react", "react/jsx-runtime"] } } });
  const chunks = (Array.isArray(result) ? result : [result]).flatMap(result => result.output).filter(item => item.type === "chunk").map(item => Buffer.from(item.code));
  return { raw: chunks.reduce((sum, chunk) => sum + chunk.length, 0), gzip: chunks.reduce((sum, chunk) => sum + gzipSync(chunk, { level: 9 }).length, 0) };
}
const graph = await bundled({ graph: "dist/graph/index.js", react: "dist/graph/react.js" });
const readerMap = await bundled({ map: "dist/graph/reader-map.js", react: "dist/graph/reader-map-react.js" });
// All graph-directory JS is already included in one of these two bundles (including the
// generated map validator), so ordinary modules must not count those files a second time.
const ordinary = measure(javascript.filter(path => group(path) !== "graph"));
export const sizes = {
  readerWasm: wasm("reader-wasm/narrata_nodes_wasm_bg.wasm"),
  compatWasm: wasm("wasm/narrata_nodes_wasm_bg.wasm"),
  authoringWasm: wasm("authoring-wasm/narrata_authoring_wasm_bg.wasm"),
  javascript: { raw: ordinary.raw + graph.raw + readerMap.raw, gzip: ordinary.gzip + graph.gzip + readerMap.gzip },
  runtime: measure(javascript.filter(path => !["react", "testing", "graph"].includes(group(path)) && !isNativeContent(path) && !isAuthoring(path) && !isCompat(path))),
  authoring: measure(javascript.filter(isAuthoring)),
  compat: measure(javascript.filter(isCompat)),
  react: measure(javascript.filter(path => group(path) === "react")),
  testing: measure(javascript.filter(path => group(path) === "testing" && !isNativeContent(path) && !isAuthoring(path))),
  nativeContent: measure(javascript.filter(isNativeContent)),
  graph,
  readerMap,
};
console.log(JSON.stringify(sizes, null, 2));
const budget = JSON.parse(readFileSync(resolve(here, "size-budget.json")));
for (const asset of Object.keys(sizes)) for (const compression of ["raw", "gzip"]) {
  if (!budget.maximum[asset]) throw new Error(`Missing ${asset} budget`);
  if (sizes[asset][compression] > budget.maximum[asset][compression]) throw new Error(`${asset} ${compression}: ${sizes[asset][compression]} exceeds ${budget.maximum[asset][compression]}`);
}
