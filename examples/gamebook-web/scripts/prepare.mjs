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
// The committed pack must match its sources and lock; the reader bundles a copy of it and of
// the work's content pack.
cargo("run", "--quiet", "--locked", "-p", "narrata-node-tools", "--", "compose", "products/gamebook-demo/project.json", "--out", "products/gamebook-demo/story.narpack", "--locked");
cargo("run", "--quiet", "--locked", "-p", "narrata-node-tools", "--", "schemas", "--out", "packages/narrata/nodes/schemas");
const story = resolve(here, "src/generated/story");
mkdirSync(story, { recursive: true });
copyFileSync(resolve(root, "products/gamebook-demo/story.narpack"), resolve(story, "story.narpack"));
copyFileSync(resolve(root, "products/gamebook-demo/content/zh-Hans.json"), resolve(story, "zh-Hans.json"));
const source = resolve(root, "packages/narrata/nodes/schemas/book-view.schema.json");
copyFileSync(source, resolve(here, "src/generated/book-view.schema.json"));
const types = await compileFromFile(source, { bannerComment: "/* Generated from the Rust BookView JSON Schema. Run npm run prepare:runtime. */", additionalProperties: false });
writeFileSync(resolve(here, "src/generated/book-view.ts"), types);
