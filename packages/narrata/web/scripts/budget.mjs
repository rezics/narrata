import { readFileSync, readdirSync } from "node:fs";
import { resolve, join } from "node:path";
import { gzipSync } from "node:zlib";

const here = resolve(import.meta.dirname, "..");
function files(directory) {
  return readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? files(join(directory, entry.name)) : [join(directory, entry.name)]);
}
const wasm = readFileSync(resolve(here, "dist/wasm/narrata_nodes_wasm_bg.wasm"));
const javascript = files(resolve(here, "dist")).filter(path => path.endsWith(".js")).map(path => readFileSync(path));
export const sizes = {
  wasm: { raw: wasm.length, gzip: gzipSync(wasm, { level: 9 }).length },
  javascript: { raw: javascript.reduce((sum, bytes) => sum + bytes.length, 0), gzip: javascript.reduce((sum, bytes) => sum + gzipSync(bytes, { level: 9 }).length, 0) },
};
console.log(JSON.stringify(sizes, null, 2));
const budget = JSON.parse(readFileSync(resolve(here, "size-budget.json")));
for (const asset of Object.keys(sizes)) for (const compression of ["raw", "gzip"]) {
  if (sizes[asset][compression] > budget.maximum[asset][compression]) throw new Error(`${asset} ${compression}: ${sizes[asset][compression]} exceeds ${budget.maximum[asset][compression]}`);
}
