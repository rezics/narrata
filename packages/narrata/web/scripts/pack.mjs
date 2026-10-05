import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, readFileSync } from "node:fs";
import { resolve } from "node:path";

const here = resolve(import.meta.dirname, "..");
const directory = resolve(here, "../../../.temp/web-pack");
mkdirSync(directory, { recursive: true });
// Invoke npm's JS entrypoint on Windows as well: no shell interpolation or .cmd spawning.
export const npmCli = process.env.npm_execpath ?? resolve(process.execPath, "../node_modules/npm/bin/npm-cli.js");
const result = JSON.parse(execFileSync(process.execPath, [npmCli, "pack", "--json", "--pack-destination", directory], { cwd: here, encoding: "utf8" }));
export const tarball = resolve(directory, result[0].filename);
const bytes = readFileSync(tarball);
console.log(tarball);
for (const algorithm of ["sha256", "sha512"]) console.log(`${algorithm} ${createHash(algorithm).update(bytes).digest("hex")}`);
