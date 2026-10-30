import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, readFileSync, readdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { resolve, sep } from "node:path";
import { compileFromFile } from "json-schema-to-typescript";
import Ajv2020 from "ajv/dist/2020.js";
import standalone from "ajv/dist/standalone/index.js";
import { minify } from "rolldown/utils";

export const here = resolve(import.meta.dirname, "..");
export const root = resolve(here, "../../..");
const check = process.argv.includes("--check");
const interfaceSchema = "packages/narrata/tooling/crates/narrata-authoring-wasm/schemas/authoring-interface.schema.json";
const stage = resolve(root, ".temp/web-build");
const dist = resolve(here, "dist");
// Clear only this package's output and this worktree's dedicated staging directory.
if (!stage.startsWith(resolve(root, ".temp") + sep) || !dist.startsWith(here + sep)) throw new Error("Build output escaped its worktree");
rmSync(stage, { recursive: true, force: true });
const cargo = (...args) => execFileSync("cargo", args, { cwd: root, stdio: "inherit", env: { ...process.env, CARGO_TARGET_DIR: resolve(root, "target"), ...(args.includes("narrata-authoring-wasm") && args.includes("build") ? { CARGO_PROFILE_WASM_OPT_LEVEL: "z" } : {}) } });
const cargoVersion = readFileSync(resolve(root, "Cargo.toml"), "utf8").match(/\[workspace.package\]\s+version = "([^"]+)"/)?.[1];
if (JSON.parse(readFileSync(resolve(here, "package.json"))).version !== cargoVersion) throw new Error("npm and Cargo workspace versions differ");
if (!check) {
  rmSync(dist, { recursive: true, force: true });
  const version = readFileSync(resolve(root, "Cargo.lock"), "utf8").match(/name = "wasm-bindgen"\r?\nversion = "([^"]+)"/)?.[1];
  if (!version) throw new Error("wasm-bindgen version missing from Cargo.lock");
  const executable = resolve(root, ".temp/wasm-tools/bin", process.platform === "win32" ? "wasm-bindgen.exe" : "wasm-bindgen");
  const installed = existsSync(executable) ? execFileSync(executable, ["--version"], { encoding: "utf8" }).trim() : "";
  if (installed !== `wasm-bindgen ${version}`) cargo("install", "wasm-bindgen-cli", "--version", version, "--locked", "--root", ".temp/wasm-tools");
  // Separate invocations keep Cargo from unifying the authoring crate's features into the
  // reader. The raw dist/wasm module keeps its default legacy publishGraph for compatibility.
  const bindgen = (crate, features, directory) => {
    cargo("build", "--profile", "wasm", "-p", crate, ...features, "--target", "wasm32-unknown-unknown", "--locked");
    execFileSync(executable, [resolve(root, `target/wasm32-unknown-unknown/wasm/${crate.replaceAll("-", "_")}.wasm`), "--target", "web", "--remove-name-section", "--out-dir", resolve(dist, directory)], { cwd: root, stdio: "inherit" });
  };
  bindgen("narrata-nodes-wasm", ["--no-default-features"], "reader-wasm");
  bindgen("narrata-nodes-wasm", [], "wasm");
  bindgen("narrata-authoring-wasm", [], "authoring-wasm");
  cargo("run", "--quiet", "--locked", "-p", "narrata-node-tools", "--", "schemas", "--out", resolve(stage, "schemas"));
  cargo("run", "--quiet", "--locked", "-p", "narrata-authoring-wasm", "--example", "interface_schema", "--", resolve(root, interfaceSchema));
}
const schemaPath = resolve(check ? root : stage, check ? "packages/narrata/nodes/schemas/book-view.schema.json" : "schemas/book-view.schema.json");
mkdirSync(resolve(here, "src/generated"), { recursive: true });
// Declarations are generated from Rust schemas; check mode compares them with the committed ones.
for (const [schema, name, title] of [[schemaPath, "book-view.ts", "BookView"], [resolve(root, interfaceSchema), "authoring.ts", "authoring interface"]]) {
  const types = await compileFromFile(schema, { bannerComment: `/* Generated from Rust ${title} JSON Schema. Run task web:build. */`, additionalProperties: false });
  const typePath = resolve(here, "src/generated", name);
  if (check) {
    if (readFileSync(typePath, "utf8") !== types) throw new Error(`${title} declarations drifted; run task web:build`);
  } else writeFileSync(typePath, types);
}
mkdirSync(resolve(stage, "src"), { recursive: true });
// Resolve optional peer types from this package while compiling the copied source tree.
symlinkSync(resolve(here, "node_modules"), resolve(stage, "node_modules"), "junction");
cpSync(resolve(here, "src"), resolve(stage, "src"), { recursive: true });
cpSync(resolve(root, "packages/narrata/kernel/js/src"), resolve(stage, "src/storage"), { recursive: true });
for (const name of ["postgres", "relations"]) {
  const source = readFileSync(resolve(root, `packages/narrata/kernel/js/server/${name}.ts`), "utf8")
    .replaceAll('"../src/', '"../storage/').replaceAll('"../../../web/src/generated/', '"../generated/');
  writeFileSync(resolve(stage, `src/server/${name === "relations" ? "relation-driver" : name}.ts`), source.replace(/from "(\.\.[^"\n]+|\.\/[^"\n]+)"/g, (_, path) => `from "${path.endsWith(".js") ? path : path + ".js"}"`));
}
writeFileSync(resolve(stage, "src/storage.ts"), 'export * from "./storage/index.js";\n');
for (const name of readdirSync(resolve(stage, "src/storage"))) {
  const path = resolve(stage, "src/storage", name);
  writeFileSync(path, readFileSync(path, "utf8").replace(/from "(\.\/[^"]+)"/g, (_, path) => `from "${path}.js"`));
}
for (const directory of ["wasm", "reader-wasm", "authoring-wasm"]) cpSync(resolve(dist, directory), resolve(stage, "src", directory), { recursive: true });
const ajv = new Ajv2020({ strict: true, validateFormats: false, code: { source: true, esm: true } });
const validator = ajv.compile(JSON.parse(readFileSync(schemaPath, "utf8")));
// Ajv's Unicode length helper would introduce a runtime dependency; inline the equivalent
// helper into the generated validator so the tarball has no host framework/dependencies.
const code = standalone(ajv, validator).replace(/require\("ajv\/dist\/runtime\/ucs2length"\)\.default/g, '(value => Array.from(value).length)');
writeFileSync(resolve(stage, "src/generated/validate-book.js"), code);
writeFileSync(resolve(stage, "src/generated/validate-book.d.ts"), 'import type { BookView } from "./book-view.js";\ndeclare const validate: (value: unknown) => value is BookView;\nexport default validate;\n');
writeFileSync(resolve(stage, "tsconfig.json"), JSON.stringify({ compilerOptions: {
  target: "ES2022", module: "NodeNext", moduleResolution: "NodeNext", lib: ["ES2022", "DOM"],
  strict: true, noUncheckedIndexedAccess: true, exactOptionalPropertyTypes: true, noImplicitOverride: true,
  verbatimModuleSyntax: true, skipLibCheck: true, declaration: true, rootDir: "src", outDir: dist,
  jsx: "react-jsx",
  types: ["node"], typeRoots: [resolve(here, "node_modules/@types")],
}, include: ["src/**/*.ts", "src/**/*.tsx"] }, null, 2));
writeFileSync(resolve(stage, "package.json"), '{"type":"module"}\n');
execFileSync(process.execPath, [resolve(here, "node_modules/typescript/bin/tsc"), "-p", resolve(stage, "tsconfig.json"), ...(check ? ["--noEmit"] : [])], { cwd: here, stdio: "inherit" });
if (!check) {
  // Keep the expanded authoring/server capabilities inside their existing JS budget.
  // Per-module minification preserves ESM boundaries and package-relative Wasm URLs.
  for (const directory of ["authoring", "server", "authoring-wasm"]) {
    for (const name of readdirSync(resolve(dist, directory)).filter(name => name.endsWith(".js"))) {
      const path = resolve(dist, directory, name);
      const result = await minify(path, readFileSync(path, "utf8"), { module: true });
      if (result.errors.length) throw new Error(JSON.stringify(result.errors));
      writeFileSync(path, result.code);
    }
  }
  cpSync(resolve(stage, "src/generated/validate-book.js"), resolve(dist, "generated/validate-book.js"));
  cpSync(resolve(stage, "src/generated/validate-book.d.ts"), resolve(dist, "generated/validate-book.d.ts"));
  cpSync(schemaPath, resolve(dist, "generated/book-view.schema.json"));
}
