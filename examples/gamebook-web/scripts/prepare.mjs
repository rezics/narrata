import { execFileSync } from "node:child_process";
import { mkdirSync, copyFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const root = resolve(here, "../..");
const cargo = (...args) => execFileSync("cargo", args, { cwd: root, stdio: "inherit", env: { ...process.env, CARGO_TARGET_DIR: resolve(root, "target") } });
execFileSync("task", ["web:build"], { cwd: root, stdio: "inherit" });
// The committed pack must match its sources and lock; the reader bundles a copy of it and of
// the work's content pack.
const outline = ".temp/gamebook-demo.outline.json";
cargo("run", "--quiet", "--locked", "-p", "narrata-node-tools", "--", "outline", "products/gamebook-demo/content/zh-Hans.json", "--out", outline);
cargo("run", "--quiet", "--locked", "-p", "narrata-node-tools", "--", "compose", "products/gamebook-demo/project.json", "--out", "products/gamebook-demo/story.narpack", "--locked", "--outline", outline);
cargo("run", "--quiet", "--locked", "-p", "narrata-node-tools", "--", "schemas", "--out", "packages/narrata/nodes/schemas");
const story = resolve(here, "src/generated/story");
mkdirSync(story, { recursive: true });
copyFileSync(resolve(root, "products/gamebook-demo/story.narpack"), resolve(story, "story.narpack"));
copyFileSync(resolve(root, "products/gamebook-demo/content/zh-Hans.json"), resolve(story, "zh-Hans.json"));
