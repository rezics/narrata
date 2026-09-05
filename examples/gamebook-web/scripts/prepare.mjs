import { execFileSync } from "node:child_process";
import { readFileSync, writeFileSync, mkdirSync, copyFileSync, existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { compileFromFile } from "json-schema-to-typescript";

const here = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = resolve(here, "../..");
const cargo = (...args) => execFileSync("cargo", args, { cwd: root, stdio: "inherit" });
const lock = readFileSync(resolve(root, "Cargo.lock"), "utf8");
const version = lock.match(/name = "wasm-bindgen"\r?\nversion = "([^"]+)"/)?.[1];
if (!version) throw new Error("wasm-bindgen version missing from Cargo.lock");
const executable = resolve(root, ".temp/wasm-tools/bin", process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen");
let installed = "";
if (existsSync(executable)) installed = execFileSync(executable, ["--version"], { encoding: "utf8" }).trim();
if (installed !== `wasm-bindgen ${version}`) {
  cargo("install", "wasm-bindgen-cli", "--version", version, "--locked", "--root", ".temp/wasm-tools");
}
cargo("build", "--release", "-p", "narrata-nodes-wasm", "--target", "wasm32-unknown-unknown", "--locked");
execFileSync(executable, [
  resolve(root, "target/wasm32-unknown-unknown/release/narrata_nodes_wasm.wasm"),
  "--target", "web", "--out-dir", resolve(here, "src/generated/wasm"),
], { cwd: root, stdio: "inherit" });
cargo("run", "--quiet", "--locked", "-p", "narrata-node-tools", "--", "compose", "products/gamebook-demo/project.json", "--out", "examples/gamebook-web/src/generated/story.nar.json", "--locked");
cargo("run", "--quiet", "--locked", "-p", "narrata-node-tools", "--", "schemas", "--out", "packages/narrata/nodes/schemas");
const source = resolve(root, "packages/narrata/nodes/schemas/book-view.schema.json");
mkdirSync(resolve(here, "src/generated"), { recursive: true });
copyFileSync(source, resolve(here, "src/generated/book-view.schema.json"));
const types = await compileFromFile(source, { bannerComment: "/* Generated from the Rust BookView JSON Schema. Run npm run prepare:runtime. */", additionalProperties: false });
writeFileSync(resolve(here, "src/generated/book-view.ts"), types);
