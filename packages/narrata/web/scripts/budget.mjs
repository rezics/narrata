import { readFileSync, readdirSync } from "node:fs";
import { resolve, join, relative } from "node:path";
import { gzipSync } from "node:zlib";

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
const group = path => relative(resolve(here, "dist"), path).replaceAll("\\", "/").split("/")[0];
export const sizes = {
  wasm: { raw: wasm.length, gzip: gzipSync(wasm, { level: 9 }).length },
  javascript: measure(javascript),
  runtime: measure(javascript.filter(path => !["react", "testing"].includes(group(path)))),
  react: measure(javascript.filter(path => group(path) === "react")),
  testing: measure(javascript.filter(path => group(path) === "testing")),
};
console.log(JSON.stringify(sizes, null, 2));
const budget = JSON.parse(readFileSync(resolve(here, "size-budget.json")));
for (const asset of Object.keys(sizes)) for (const compression of ["raw", "gzip"]) {
  if (!budget.maximum[asset]) throw new Error(`Missing ${asset} budget`);
  if (sizes[asset][compression] > budget.maximum[asset][compression]) throw new Error(`${asset} ${compression}: ${sizes[asset][compression]} exceeds ${budget.maximum[asset][compression]}`);
}
