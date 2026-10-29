import { readFileSync, readdirSync } from "node:fs";
import { resolve, join, relative } from "node:path";
import { gzipSync } from "node:zlib";
import { build } from "vite";

const here = resolve(import.meta.dirname, "..");
function files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? files(join(directory, entry.name)) : [join(directory, entry.name)]);
}
const wasm = readFileSync(resolve(here, "dist/wasm/narrata_nodes_wasm_bg.wasm"));
const javascript = files(resolve(here, "dist")).filter(path => path.endsWith(".js"));
function measure(paths) {
  const bytes = paths.map(path => readFileSync(path));
  return { raw: bytes.reduce((sum, item) => sum + item.length, 0), gzip: bytes.reduce((sum, item) => sum + gzipSync(item, { level: 9 }).length, 0) };
}
const modulePath = path => relative(resolve(here, "dist"), path).replaceAll("\\", "/");
const group = path => modulePath(path).split("/")[0];
const nativeModules = new Set(["content/native.js", "testing/native-document.js"]);
const isNativeContent = path => nativeModules.has(modulePath(path));
// Include Sigma, graphology and their transitive modules, not just our import wrappers.
const graphBuild = await build({ configFile: false, logLevel: "error", build: { write: false, minify: true,
  lib: { entry: { graph: resolve(here, "dist/graph/index.js"), react: resolve(here, "dist/graph/react.js") }, formats: ["es"] },
  rolldownOptions: { external: ["react", "react/jsx-runtime"] } } });
const graphChunks = (Array.isArray(graphBuild) ? graphBuild : [graphBuild]).flatMap(result => result.output).filter(item => item.type === "chunk").map(item => Buffer.from(item.code));
const graph = { raw: graphChunks.reduce((sum, chunk) => sum + chunk.length, 0), gzip: graphChunks.reduce((sum, chunk) => sum + gzipSync(chunk, { level: 9 }).length, 0) };
const ordinary = measure(javascript.filter(path => group(path) !== "graph"));
export const sizes = {
  wasm: { raw: wasm.length, gzip: gzipSync(wasm, { level: 9 }).length },
  javascript: { raw: ordinary.raw + graph.raw, gzip: ordinary.gzip + graph.gzip },
  runtime: measure(javascript.filter(path => !["react", "testing", "graph"].includes(group(path)) && !isNativeContent(path))),
  react: measure(javascript.filter(path => group(path) === "react")),
  testing: measure(javascript.filter(path => group(path) === "testing" && !isNativeContent(path))),
  nativeContent: measure(javascript.filter(isNativeContent)),
  graph,
};
console.log(JSON.stringify(sizes, null, 2));
const budget = JSON.parse(readFileSync(resolve(here, "size-budget.json")));
for (const asset of Object.keys(sizes)) for (const compression of ["raw", "gzip"]) {
  if (!budget.maximum[asset]) throw new Error(`Missing ${asset} budget`);
  if (sizes[asset][compression] > budget.maximum[asset][compression]) throw new Error(`${asset} ${compression}: ${sizes[asset][compression]} exceeds ${budget.maximum[asset][compression]}`);
}
