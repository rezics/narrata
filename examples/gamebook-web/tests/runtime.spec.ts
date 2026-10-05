import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { cp, mkdtemp, readFile, readdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { z } from "zod";
import type { NodeBook } from "../../../packages/narrata/web/dist/wasm/narrata_nodes_wasm";
import type { checkedBook } from "../src/protocol";

const root = resolve(import.meta.dirname, "../../..");
const demo = resolve(root, "products/gamebook-demo");
const cli = resolve(root, "target/debug", process.platform === "win32" ? "narrata-book.exe" : "narrata-book");
const execution = "execution:0190f2a0000070008000000000000001";
const object = z.object({ object_id: z.string().regex(/^[0-9a-f]{64}$/), bytes: z.array(z.number().int().min(0).max(255)) });
const publication = z.object({
  index: object, tiles: z.array(object), labels: z.array(object),
  files_json: z.string(), summary_json: z.string(), analysis_json: z.string(),
  diagnostics: z.array(z.object({ code: z.string(), path: z.string(), message: z.string() })),
});

function native(...args: string[]) {
  return execFileSync(cli, args, { cwd: root, encoding: "utf8" });
}

// Test-fixture extraction only: Pack's canonical map of byte-string envelopes (ADR 0013).
// Wasm separately checks the manifest, every used object and the complete pack.
function splitPack(pack: Buffer) {
  let at = 56;
  function size(major: number): number {
    const head = pack.readUInt8(at++);
    if (head >> 5 !== major) throw new Error("unexpected pack fixture CBOR type");
    const info = head & 31;
    if (info < 24) return info;
    const width = info === 24 ? 1 : info === 25 ? 2 : info === 26 ? 4 : 0;
    if (!width) throw new Error("unexpected pack fixture CBOR size");
    const value = pack.readUIntBE(at, width); at += width;
    return value;
  }
  function bytes() { const length = size(2); const value = Array.from(pack.subarray(at, at + length)); at += length; return value; }
  expect(size(5)).toBe(4);
  expect(size(0)).toBe(0); const manifest = bytes();
  expect(size(0)).toBe(1); const chunks = Array.from({ length: size(4) }, bytes);
  expect(size(0)).toBe(2); const tombstones = bytes();
  expect(size(0)).toBe(3); const names = bytes();
  expect(at).toBe(pack.length);
  return { manifest, chunks, tombstones, names };
}

test("Wasm publication matches CLI compose bytes with absent, complete and damaged outlines", async ({ page }) => {
  test.setTimeout(90_000);
  const directory = await mkdtemp(join(tmpdir(), "narrata-publication-"));
  await cp(demo, directory, { recursive: true });
  const pack = Array.from(await readFile(join(directory, "story.narpack")));
  const outlinePath = join(directory, "outline.json");
  native("outline", join(directory, "content/zh-Hans.json"), "--out", outlinePath);
  const complete = z.record(z.string(), z.unknown()).parse(JSON.parse(await readFile(outlinePath, "utf8")));
  const damaged = { ...complete, units: {} };
  await page.goto("/");
  for (const outlines of [[], [complete], [damaged]]) {
    const args = ["compose", join(directory, "project.json"), "--out", join(directory, "story.narpack"), "--locked"];
    if (outlines.length) {
      await writeFile(outlinePath, JSON.stringify(outlines[0]));
      args.push("--outline", outlinePath);
    }
    native(...args);
    const raw: string = await page.evaluate(async ({ pack, outlines }) => {
      const moduleUrl = "/node_modules/@rezics/narrata/dist/wasm/narrata_nodes_wasm.js";
      const wasm = await import(moduleUrl); await wasm.default();
      return wasm.publishGraph(new Uint8Array(pack), outlines.length ? JSON.stringify(outlines) : undefined);
    }, { pack, outlines });
    const result = publication.parse(JSON.parse(raw));
    expect(result.files_json).toBe(await readFile(join(directory, "story.graph.json"), "utf8"));
    expect(result.summary_json).toBe(await readFile(join(directory, "story.summary.json"), "utf8"));
    expect(result.analysis_json).toBe(await readFile(join(directory, "story.analysis.json"), "utf8"));
    const analysis = z.object({ diagnostics: publication.shape.diagnostics }).parse(JSON.parse(result.analysis_json));
    expect(result.diagnostics).toEqual(analysis.diagnostics);
    const objects = [result.index, ...result.tiles, ...result.labels];
    expect(objects.map(value => `${value.object_id}.cbor`).sort()).toEqual((await readdir(join(directory, "story.graph"))).sort());
    for (const value of objects) {
      expect(Buffer.from(value.bytes)).toEqual(await readFile(join(directory, "story.graph", `${value.object_id}.cbor`)));
    }
    expect(result.diagnostics.some(value => value.code === "outline_checks_skipped")).toBe(!outlines.length);
    if (outlines[0] === damaged) expect(result.diagnostics.some(value => value.code === "outline_unit_missing")).toBe(true);
  }
  const errors: string[] = await page.evaluate(async ({ pack, outline }) => {
    const moduleUrl = "/node_modules/@rezics/narrata/dist/wasm/narrata_nodes_wasm.js";
    const wasm = await import(moduleUrl); await wasm.default();
    const badPack = new Uint8Array(pack); badPack[badPack.length - 1] = (badPack.at(-1) ?? 0) ^ 1;
    const cases: Array<[Uint8Array, string | undefined]> = [
      [badPack, undefined], [new Uint8Array(pack), JSON.stringify([outline, outline])],
      [new Uint8Array(pack), '[{"format_version":1,"format_version":1,"provider":"local","units":{}}]'],
      [new Uint8Array(pack), '[{"format_version":1,"provider":"local","units":{},"unknown":true}]'],
    ];
    return cases.map(([bytes, outlines]) => {
      try { wasm.publishGraph(bytes, outlines); return "accepted"; } catch (error) { return String(error); }
    });
  }, { pack, outline: complete });
  expect(errors[0]).toContain("decode");
  expect(errors[1]).toContain("outline_provider");
  expect(errors[2]).toContain("json");
  expect(errors[3]).toContain("schema");
});

test("manifest-only Wasm loads the first chunk, then retries cold choices without changing history", async ({ page }) => {
  const pack = await readFile(join(demo, "story.narpack"));
  const parts = splitPack(pack);
  await page.goto("/");
  const result = await page.evaluate(async ({ parts, pack, execution }) => {
    const moduleUrl = "/node_modules/@rezics/narrata/dist/wasm/narrata_nodes_wasm.js";
    const wasm = await import(moduleUrl); await wasm.default();
    const protocolUrl = "/src/protocol.ts";
    const protocol = await import(protocolUrl);
    const lazy = wasm.NodeBook.fromManifest(new Uint8Array(parts.manifest), execution);
    const full = new wasm.NodeBook(new Uint8Array(pack), execution);
    const requests: string[] = [];
    let retries = 0;
    let invalidRejected = false;
    let wrongObjectRejected = false;
    function retry<T>(operation: () => T, onMissing?: () => void): T {
      for (let attempt = 0; attempt < 30; attempt++) {
        try { return operation(); } catch (error) {
          const id: string | undefined = lazy.takeChunkRequest();
          if (!id) throw error;
          onMissing?.();
          requests.push(id);
          let found = false;
          for (const bytes of [...parts.chunks, parts.tombstones]) {
            try { lazy.loadChunk(id, new Uint8Array(bytes)); found = true; break; }
            catch (error) { if (!String(error).includes("artifact")) throw error; }
          }
          if (!found) throw new Error(`missing test object ${id}`);
          retries++;
        }
      }
      throw new Error("program requests did not settle");
    }
    function inspect(book: NodeBook) {
      const raw: unknown = JSON.parse(book.inspect());
      const value: ReturnType<typeof checkedBook> = protocol.checkedBook(raw);
      return { view: value.view, page: value.page };
    }
    try {
      const initialRequest = lazy.takeChunkRequest();
      try { lazy.memory(); throw new Error("expected first chunk request"); } catch (error) {
        if (!String(error).includes("not_loaded")) throw error;
      }
      const first: unknown = lazy.takeChunkRequest();
      if (typeof first !== "string") throw new Error("first chunk request missing");
      requests.push(first);
      const firstFixture = parts.chunks[0];
      if (!firstFixture) throw new Error("pack fixture has no chunks");
      const corrupt = new Uint8Array(firstFixture); corrupt[corrupt.length - 1] = (corrupt.at(-1) ?? 0) ^ 1;
      try { lazy.loadChunk(first, corrupt); } catch (error) { invalidRejected = String(error).includes("decode"); }
      try { lazy.loadChunk(first, new Uint8Array(parts.tombstones)); } catch (error) { wrongObjectRejected = String(error).includes("artifact"); }
      let firstChunk = -1;
      for (const [index, bytes] of parts.chunks.entries()) {
        try { lazy.loadChunk(first, new Uint8Array(bytes)); firstChunk = index; break; }
        catch (error) { if (!String(error).includes("artifact")) throw error; }
      }
      if (firstChunk < 0) throw new Error("entry chunk absent from fixture");
      lazy.memory(); full.memory();
      const firstScreen = inspect(lazy);
      const firstScreenRequests = requests.length;
      // Names are optional; the first screen was already available without them.
      lazy.loadNames(new Uint8Array(parts.names));
      const firstScreenEqual = JSON.stringify(inspect(lazy)) === JSON.stringify(inspect(full));
      const node = firstScreen.view.frames[0]?.node;
      if (!node) throw new Error("first screen has no node");
      let coldChoices = 0;
      for (const key of "camp,letter,continue,rest,continue,road,help,continue,deliver".split(",")) {
        const before = inspect(full).view;
        const pendingBefore = JSON.stringify(lazy.unconfirmed());
        const interaction = before.interaction;
        if (interaction.kind !== "choose") throw new Error("route ended too early");
        const option = interaction.options.find(option => option.key === key);
        if (!option) throw new Error(`missing option ${key}`);
        retry(() => lazy.choose(before.cursor, interaction.choice_point, [option.id]), () => {
          coldChoices++;
          if (JSON.stringify(lazy.unconfirmed()) !== pendingBefore) throw new Error("failed cold choice changed history");
        });
        full.choose(before.cursor, interaction.choice_point, [option.id]);
        if (JSON.stringify(inspect(lazy)) !== JSON.stringify(inspect(full))) throw new Error("lazy/full state divergence");
      }
      // Ordinary reading did not need tombstones. Lookup requests them separately.
      try { lazy.lookup(node); throw new Error("expected tombstone request"); } catch (error) {
        if (!String(error).includes("not_loaded")) throw error;
      }
      const tombstoneRequest: unknown = lazy.takeChunkRequest();
      if (typeof tombstoneRequest !== "string") throw new Error("tombstone request missing");
      lazy.loadChunk(tombstoneRequest, new Uint8Array(parts.tombstones));
      const lookup: unknown = JSON.parse(retry(() => lazy.lookup(node)));
      const beforeVerify = requests.length;
      // All chunks, including the initially cached chunk, are still checked against tombstones.
      retry(() => lazy.verifyArtifact());
      const verificationRequests = requests.length - beforeVerify;
      return {
        initialRequest, firstScreenEqual, firstScreenRequests, firstChunk,
        invalidRejected, wrongObjectRejected, lookup, verificationRequests, retries, coldChoices,
        sameCheckpoint: lazy.export() === full.export(),
        requestIds: requests,
      };
    } finally { lazy.free(); full.free(); }
  }, { parts, pack: Array.from(pack), execution });
  expect(result.initialRequest).toBeUndefined();
  expect(result.firstScreenEqual).toBe(true);
  expect(result.firstScreenRequests).toBe(1);
  expect(result.firstChunk).toBeGreaterThanOrEqual(0);
  expect(result.invalidRejected && result.wrongObjectRejected).toBe(true);
  expect(z.object({ kind: z.string() }).parse(result.lookup).kind).toBe("live");
  expect(result.verificationRequests).toBeLessThan(parts.chunks.length);
  expect(result.coldChoices).toBeGreaterThan(0);
  expect(result.sameCheckpoint).toBe(true);
  expect(result.requestIds.every(id => /^object:[0-9a-f]{64}$/.test(id))).toBe(true);
});

test("Wasm retries finish full scans larger than the decoded cache and release supplied bytes", async ({ page }) => {
  const directory = await mkdtemp(join(tmpdir(), "narrata-lazy-scan-"));
  await cp(demo, directory, { recursive: true });
  const packagePath = join(directory, "packages/road.json");
  const packageSource = z.object({ graphs: z.record(z.string(), z.unknown()) }).passthrough().parse(JSON.parse(await readFile(packagePath, "utf8")));
  let target = "";
  for (let index = 0; index < 32; index++) {
    target = `node:${(100_000 + index).toString(16).padStart(32, "0")}`;
    packageSource.graphs[`zzz_${index.toString().padStart(2, "0")}`] = {
      entry: "out", outcomes: ["done"], nodes: {
        out: { id: target, type_id: "narrata.return", data: { outcome: "done" } },
      },
    };
  }
  await writeFile(packagePath, JSON.stringify(packageSource));
  native("compose", join(directory, "project.json"), "--out", join(directory, "story.narpack"));
  const parts = splitPack(await readFile(join(directory, "story.narpack")));
  expect(parts.chunks.length).toBeGreaterThan(16);
  await page.goto("/");
  const result = await page.evaluate(async ({ parts, target, execution }) => {
    const moduleUrl = "/node_modules/@rezics/narrata/dist/wasm/narrata_nodes_wasm.js";
    const wasm = await import(moduleUrl); await wasm.default();
    const book = wasm.NodeBook.fromManifest(new Uint8Array(parts.manifest), execution);
    let requests = 0;
    function retry<T>(operation: () => T): T {
      for (let attempt = 0; attempt < 100; attempt++) {
        try { return operation(); } catch (error) {
          const id: unknown = book.takeChunkRequest();
          if (typeof id !== "string") throw error;
          requests++;
          let found = false;
          for (const bytes of [parts.tombstones, ...parts.chunks]) {
            try { book.loadChunk(id, new Uint8Array(bytes)); found = true; break; }
            catch (error) { if (!String(error).includes("artifact")) throw error; }
          }
          if (!found) throw new Error(`missing test object ${id}`);
        }
      }
      throw new Error("scan did not progress through decoded eviction");
    }
    try {
      retry(() => book.verifyArtifact());
      const firstScanRequests = requests;
      const lookup: unknown = JSON.parse(retry(() => book.lookup(target)));
      const lookupRequests = requests - firstScanRequests;
      // A successful scan releases encoded source bytes. The next scan re-fetches
      // evicted chunks while retaining enough bytes across retries to finish again.
      retry(() => book.verifyArtifact());
      return { firstScanRequests, lookupRequests, lookup, requestAfterSuccess: book.takeChunkRequest() };
    } finally { book.free(); }
  }, { parts, target, execution });
  expect(result.firstScanRequests).toBe(parts.chunks.length + 1);
  expect(result.lookupRequests).toBeGreaterThan(16);
  expect(z.object({ kind: z.string(), node: z.string() }).parse(result.lookup)).toEqual({ kind: "live", node: target });
  expect(result.requestAfterSuccess).toBeUndefined();
});
