// Builds the counter test module for the browser tests: Rust to wasm32, then wasm-bindgen glue
// for the web, with the CLI version Cargo.lock pins (installed under .temp/wasm-tools).
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = resolve(here, "../../../..");
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
cargo("build", "--release", "-p", "narrata-storage-browser-counter", "--target", "wasm32-unknown-unknown", "--locked");
const target = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : resolve(root, "target");
execFileSync(executable, [
  resolve(target, "wasm32-unknown-unknown/release/narrata_storage_browser_counter.wasm"),
  "--target", "web", "--out-dir", resolve(here, "browser/generated"),
], { cwd: root, stdio: "inherit" });
