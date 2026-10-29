import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { resolve, join, sep, extname } from "node:path";
import { build } from "vite";
import { chromium } from "@playwright/test";
import { expect, test } from "vitest";

test("an unpacked tarball executes the Rust evolution journeys in Node, Bun and a lazy browser module",async () => {
  const here = resolve(import.meta.dirname,"..");
  const npmCli = process.env.npm_execpath ?? resolve(process.execPath,"../node_modules/npm/bin/npm-cli.js");
  const base = join(tmpdir(),".temp"); mkdirSync(base,{recursive:true});
  const directory = mkdtempSync(join(base,"narrata-evolution-install-"));
  const packed = JSON.parse(execFileSync(process.execPath,[npmCli,"pack","--json","--pack-destination",directory],{cwd:here,encoding:"utf8"})) as {filename:string}[];
  const filename = packed[0]?.filename; if (!filename) throw new Error("npm pack returned no artifact");
  writeFileSync(join(directory,"package.json"),JSON.stringify({name:"narrata-evolution-consumer",type:"module",private:true}));
  execFileSync(process.execPath,[npmCli,"install","--ignore-scripts","--no-audit","--no-fund",join(directory,filename)],{cwd:directory,stdio:"pipe"});
  const installed = join(directory,"node_modules/@rezics/narrata");
  expect(realpathSync(installed)).toBe(installed);
  const root = resolve(here,"../../../fixtures/compat/nodes-evolution-v1");
  const vectors = ["content-alias-exact","unseen-rule","target-rejoins","deleted-lower-call","proposal-retained","shallow-history","republished-after-migration","variable-type"].map(name => {
    const at = join(root,name), file = (name: string) => readFileSync(join(at,name)), has = (name: string) => existsSync(join(at,name));
    return { name,source:[...file("source.narpack")],target:[...file("target.narpack")],save:file("source.checkpoint").toString("hex"),expected:JSON.parse(file("assessment.json").toString()),
      ...(has("declarations.json") ? {declarations:file("declarations.json").toString()} : {}),
      ...(has("descriptor.cbor") ? {descriptor:[...file("descriptor.cbor")]} : {}),
      ...(has("apply.json") ? {applied:JSON.parse(file("apply.json").toString())} : {}),
      ...(has("target.checkpoint") ? {targetSave:file("target.checkpoint").toString("hex")} : {}),
      ...(has("ancestor.narpack") ? {ancestor:[...file("ancestor.narpack")]} : {}),
    };
  });
  mkdirSync(join(directory,"public")); writeFileSync(join(directory,"public/vectors.json"),JSON.stringify(vectors));
  cpSync(join(here,"test/authoring-evolution-journey.ts"),join(directory,"journey.ts"));
  const impact = `
async function impact(api, assessImpact, v, stored) {
  const source = api.publicationFromPack(Uint8Array.from(v.source));
  let target = api.publicationFromPack(Uint8Array.from(v.target));
  const planned = api.planEvolution(source,target,v.declarations), e = api.createEvolution();
  try {
    target = target.withEvolution(source,planned);
    const id = await e.registerEvolution(source,target);
    const head = {commit:v.expected.plan.source_commit,artifact:source.artifactId};
    let loads = 0;
    const report = await assessImpact({evolution:e,heads:[head,head],options:{target:target.artifactId,migrationIds:[id]},loadSave:async () => {loads++; return v.save;}});
    if (loads !== 1 || report.counts.automatic !== 2 || report.distinct_counts.automatic !== 1 || !report.complete) throw new Error("impact Ref weighting");
    const limited = await assessImpact({evolution:e,heads:[head],options:{target:target.artifactId,migrationIds:[id]},loadSave:async () => v.save,limits:{saves:1,totalBytes:8,saveBytes:8}});
    if (limited.skipped.limit !== 1 || limited.complete) throw new Error("impact budget");
    if (stored) {
      const host = await stored(v);
      try {
        const objectReport = await assessImpact({evolution:e,heads:[host.head],options:{target:target.artifactId,migrationIds:[id]},loadSave:async () => ({kind:"store",execution:host.execution,storage:{load:request => host.storage.load(request)}})});
        if (objectReport.counts.automatic !== 1 || !objectReport.complete || objectReport.read_bytes === 0) throw new Error("object-loader impact");
      } finally {await host.close();}
    }
  } finally {await e.close(); planned.free(); source.free(); target.free();}
}`;
  writeFileSync(join(directory,"node.mjs"),`
import {readFile} from "node:fs/promises";
import {loadAuthoring,assessImpact,EvolutionController} from "@rezics/narrata/server";
import {evolutionJourney} from "./journey.ts";
${impact}
const vectors = JSON.parse(await readFile(new URL("./public/vectors.json",import.meta.url),"utf8"));
const api = await loadAuthoring();
const kinds = [];
for (const vector of vectors) kinds.push(await evolutionJourney(api,vector,EvolutionController));
await impact(api,assessImpact,vectors.find(v => v.name === "unseen-rule"));
console.log(JSON.stringify(kinds));
`);
  const expected = vectors.map(v => v.expected.kind);
  for (const executable of [process.execPath,"bun"]) {
    const result = execFileSync(executable,[join(directory,"node.mjs")],{cwd:directory,encoding:"utf8"});
    expect(JSON.parse(result.trim().split(/\r?\n/).at(-1) ?? "null")).toEqual(expected);
  }
  writeFileSync(join(directory,"index.html"),'<html><body><script type="module" src="/main.js"></script></body></html>');
  writeFileSync(join(directory,"main.js"),`
import {openBook} from "@rezics/narrata/reader";
import {evolutionJourney} from "./journey.ts";
${impact}
let vectors;
window.ready = (async () => {
  vectors = await (await fetch("/vectors.json")).json();
  const book = await openBook({pack:Uint8Array.from(vectors[0].source),execution:"execution:0190f2a0000070008000000000000001"});
  try {return (await book.inspect()).view.interaction.kind;} finally {await book.close();}
})();
window.evolve = async () => {
  await window.ready;
  const {loadAuthoring,assessImpact,EvolutionController} = await import("@rezics/narrata/authoring");
  const api = await loadAuthoring(), kinds = [];
  for (const vector of vectors) kinds.push(await evolutionJourney(api,vector,EvolutionController));
  const {IndexedDbStore} = await import("@rezics/narrata/storage");
  await impact(api,assessImpact,vectors.find(v => v.name === "unseen-rule"),async v => {
    const name = "installed-impact-" + crypto.randomUUID(), storage = await IndexedDbStore.open(name);
    const execution = "execution:0190f2a0000070008000000000000001";
    const book = await openBook({pack:Uint8Array.from(v.source),execution,storage});
    const view = (await book.inspect()).view, head = {commit:view.cursor,artifact:book.artifactId}; await book.close();
    const before = await storage.export();
    return {head,execution,storage,close:async () => {
      try {
        const after = await storage.export();
        if (before.length !== after.length || before.some((b,i) => b !== after[i])) throw new Error("impact wrote storage");
      } finally {storage.close(); await IndexedDbStore.delete(name);}
    }};
  });
  return kinds;
};
`);
  const site = join(directory,"site");
  await build({configFile:false,root:directory,logLevel:"error",build:{outDir:site,emptyOutDir:true}});
  const server = createServer((request,response) => {
    const path = resolve(site,`.${new URL(request.url ?? "/","http://localhost").pathname === "/" ? "/index.html" : new URL(request.url ?? "/","http://localhost").pathname}`);
    if (!path.startsWith(site+sep) || !existsSync(path)) { response.writeHead(404).end(); return; }
    response.setHeader("Content-Type",extname(path) === ".wasm" ? "application/wasm" : extname(path) === ".js" ? "text/javascript" : extname(path) === ".html" ? "text/html" : "application/json");
    response.end(readFileSync(path));
  });
  await new Promise<void>(done => server.listen(0,"127.0.0.1",done));
  const address = server.address(); if (!address || typeof address === "string") throw new Error("No browser test address");
  let browser: Awaited<ReturnType<typeof chromium.launch>> | undefined;
  try {
    browser = await chromium.launch();
    const page = await browser.newPage(); const requests: string[] = [], errors: string[] = [];
    page.on("request",r => { if (r.url().endsWith(".wasm")) requests.push(r.url()); });
    page.on("pageerror",e => errors.push(e.message));
    await page.goto(`http://127.0.0.1:${address.port}`);
    expect(await page.evaluate("window.ready")).toBe("choose");
    expect(requests).toHaveLength(1); expect(requests[0]).toContain("narrata_nodes_wasm");
    expect(await page.evaluate("window.evolve()")).toEqual(expected);
    expect(requests.some(url => url.includes("narrata_authoring_wasm"))).toBe(true);
    expect(errors).toEqual([]);
  } finally { await browser?.close(); await new Promise<void>((done,reject) => server.close(error => error ? reject(error) : done())); }
},180_000);
