import { createHash, randomUUID } from "node:crypto";
import type { IncomingMessage, ServerResponse } from "node:http";
import { decodeRecord, loadAuthoring } from "@rezics/narrata/server";
import type { Publication } from "@rezics/narrata/server";
import { openBook } from "@rezics/narrata/reader";
import type { Book } from "@rezics/narrata/reader";
import { referenceStorySource, referenceStoryOutline } from "./reference-story.ts";
import { referenceContent } from "./reference-content.ts";

function requireValue<T>(value: T | null | undefined, message: string): T { if (value == null) throw new Error(message); return value; }
async function choose(book: Book, key: string) {
  const view = (await book.inspect()).view;
  if (view.interaction.kind !== "choose") throw new Error("Expected preview choice");
  const option = requireValue(view.interaction.options.find(option => option.key === key), `Missing ${key}`);
  await book.choose(view.cursor, view.interaction.choice_point, [option.id]);
}
const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

/** Test/reference control plane. All compilation and whole-publication enumeration stay in Node. */
export async function createReferenceHost() {
  const api = await loadAuthoring(); const source = referenceStorySource(); const minter = api.createMinter();
  const filled = minter.fill(source.manifest, source.packages); minter.free();
  if (filled.diagnostics.some(d => d.severity === "error")) throw new Error(JSON.stringify(filled.diagnostics));
  const records = filled.records.map(record => decodeRecord(record.bytes));
  // An actual record edit before preview/publication: the call passes 7, replacing the draft's 3.
  for (const record of records) if (record.kind === "node" && record.key === "visit") {
    const data = record.payload.data;
    if (typeof data !== "object" || data === null || Array.isArray(data)) throw new Error("Invalid call draft");
    record.payload.data = { ...data, arguments: { pass: { kind: "literal", value: 7 } } };
  }
  const drafts = api.createDrafts(); for (const record of records) drafts.add(record); drafts.addOutline(referenceStoryOutline());
  const preview = drafts.preview(); const requests: string[] = [];
  let previewArtifact: string;
  try {
    previewArtifact = requireValue(preview.report.artifact_id, JSON.stringify(preview.report.diagnostics));
    const book = await openBook({ manifest: requireValue(preview.manifest(), "Missing preview manifest"), names: requireValue(preview.names(), "Missing preview names"),
      execution: `execution:${randomUUID().replaceAll("-", "")}`, fetchChunk: async id => { requests.push(id); return requireValue(preview.object(id), "Missing preview chunk"); } });
    try {
      await choose(book, "take");
      const merge = await book.inspect();
      if (!merge.page.some(item => item.role === "reply") || merge.view.interaction.kind !== "choose") throw new Error("Preview lost local merge");
      await choose(book, "visit");
      const parameter = (await book.inspect()).view.frames.at(-1)?.parameters.find(value => value.name === "pass")?.value;
      if (parameter?.type !== "int" || parameter.value !== "7") throw new Error("Preview did not pass edited parameter");
      await choose(book, "return"); await choose(book, "home");
      if ((await book.inspect()).view.interaction.kind !== "finished") throw new Error("Preview did not reach ending");
    } finally { await book.close(); }
  } finally { preview.free(); }
  const first = drafts.publish(); drafts.free();
  const a = requireValue(first.publication, JSON.stringify(first.report.diagnostics));
  if (a.artifactId !== previewArtifact) throw new Error("Publication differs from edited preview");
  const objects = new Map<string, Uint8Array>();
  function collect(publication: Publication) {
    const descriptors = [];
    for (const [reference, bytes] of publication.objects()) {
      objects.set(reference.object_id, bytes.slice());
      descriptors.push({ id: reference.object_id, role: reference.role, bytes: bytes.length, sha256: digest(bytes) });
    }
    return { artifactId: publication.artifactId, objects: descriptors };
  }
  const aEvidence = collect(a); let bEvidence: ReturnType<typeof collect> | undefined; let cEvidence: ReturnType<typeof collect> | undefined; let current = a.artifactId;
  const publications = new Map([[a.artifactId, a]]);
  const edges = new Map<string, string>();
  const variantEvidence = new Map<string, ReturnType<typeof collect>>();
  const recoveryNames = new Map<string, Record<string, string>>();
  let currentRecords = first.records;
  function publishNext(edition: "B" | "C") {
    const existing = edition === "B" ? bEvidence : cEvidence;
    if (existing) { current = existing.artifactId; return; }
    if (edition === "C" && !bEvidence) throw new Error("Publish B first");
    const previous = requireValue(publications.get(current), "Missing previous publication");
    const next = api.createDrafts();
    try {
      for (const record of currentRecords) {
        const edited = decodeRecord(record.bytes);
        if (edition === "B" && edited.kind === "choice_point" && edited.payload.key === "finish") edited.payload.options = edited.payload.options.filter(option => option.key !== "linger");
        if (edition === "C" && edited.kind === "node" && edited.key === "river" && typeof edited.payload.data === "object" && edited.payload.data !== null && !Array.isArray(edited.payload.data)) {
          edited.payload.data = { ...edited.payload.data, body: { unit: { provider: "signal", key: "river" }, first: "p1", last: "p1" } };
        }
        next.add(edited);
      }
      next.addOutline(referenceStoryOutline());
      const result = next.publish({ previous });
      const candidate = requireValue(result.publication, JSON.stringify(result.report.diagnostics));
      const plan = api.planEvolution(previous, candidate);
      try {
        const publication = candidate.withEvolution(previous, plan);
        const evidence = collect(publication);
        if (edition === "B") bEvidence = evidence; else cEvidence = evidence;
        current = publication.artifactId; publications.set(current, publication); currentRecords = result.records;
        edges.set(current, previous.artifactId);
      } finally { candidate.free(); plan.free(); }
    } finally { next.free(); }
  }
  function publishVariant(kind: "divergence" | "recovery" | "incompatible" | "river-update") {
    const next = api.createDrafts();
    const base = first.records.map(record => decodeRecord(record.bytes));
    const dawn = base.find(record => record.kind === "node" && record.key === "dawn");
    const station = base.find(record => record.kind === "node" && record.key === "station");
    const finish = base.find(record => record.kind === "choice_point" && record.payload.key === "finish");
    const listen = base.find(record => record.kind === "choice_point" && record.payload.key === "listen");
    if (dawn?.kind !== "node" || station?.kind !== "node" || finish?.kind !== "choice_point" || listen?.kind !== "choice_point") throw new Error("Missing recovery positions");
    const dawnId = requireValue(dawn.payload.id, "Missing dawn identity"), stationId = requireValue(station.payload.id, "Missing station identity");
    const finishId = requireValue(finish.payload.id, "Missing finish identity"), listenId = requireValue(listen.payload.id, "Missing opening identity");
    try {
      for (const record of base) {
        if ((kind === "recovery" || kind === "incompatible") && ((record.kind === "node" && record.key === "dawn") || (record.kind === "choice_point" && record.node === dawn.payload.id))) continue;
        if (record.kind === "node" && record.key === "visit" && typeof record.payload.data === "object" && record.payload.data !== null && !Array.isArray(record.payload.data)) {
          record.payload.data = kind === "divergence" || kind === "river-update" ? { ...record.payload.data, arguments: { pass: { kind: "literal", value: 8 } } }
            : { ...record.payload.data, on_return: { back: "station" } };
        }
        if (kind === "river-update" && record.kind === "node" && record.key === "river" && typeof record.payload.data === "object" && record.payload.data !== null && !Array.isArray(record.payload.data)) {
          record.payload.data = { ...record.payload.data, body: { unit: { provider: "signal", key: "station" }, first: "arrival", last: "arrival" } };
        }
        if (kind === "incompatible" && record.kind === "node" && record.key === "river" && typeof record.payload.data === "object" && record.payload.data !== null && !Array.isArray(record.payload.data)) {
          record.payload.data = { ...record.payload.data, body: { unit: { provider: "signal", key: "river" }, first: "p1" } };
        }
        next.add(record);
      }
      next.addOutline(referenceStoryOutline());
      const result = next.publish({ previous: a });
      const candidate = requireValue(result.publication, JSON.stringify(result.report.diagnostics));
      const declarations = kind === "recovery" ? { format_version: 1, from_artifact: a.artifactId, recoveries: [dawnId, finishId].map(deleted => ({
        deleted, target: { node: stationId, choice_point: listenId },
      })) } : undefined;
      const plan = api.planEvolution(a, candidate, declarations);
      try {
        const publication = candidate.withEvolution(a, plan);
        const existing = publications.get(publication.artifactId);
        if (existing) {
          if (existing.publicationId !== publication.publicationId) { publication.free(); throw new Error("A distinct evolution path needs a distinct reference edition"); }
          current = existing.artifactId; publication.free(); return;
        }
        variantEvidence.set(publication.artifactId, collect(publication)); current = publication.artifactId; publications.set(current, publication); edges.set(current, a.artifactId);
        if (kind === "recovery") {
          const [title] = referenceContent({ context: {}, items: [{ content: { provider: "signal", key: "scene.station" } }] });
          if (title?.status === "ok" && "text" in title.payload) recoveryNames.set(current, { [stationId]: title.payload.text });
        }
      } finally { candidate.free(); plan.free(); }
    } finally { next.free(); }
  }
  async function body(request: IncomingMessage): Promise<unknown> {
    const chunks: Uint8Array[] = []; let bytes = 0;
    for await (const chunk of request) {
      if (!(chunk instanceof Uint8Array)) throw new Error("Invalid request body");
      bytes += chunk.length; if (bytes > 256 * 1024) throw new Error("Reference request exceeds limit"); chunks.push(chunk);
    }
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  }
  function send(response: ServerResponse, value: unknown) { const bytes = Buffer.from(JSON.stringify(value)); response.writeHead(200, { "Content-Type": "application/json", "Content-Length": bytes.length }).end(bytes); }
  return {
    evidence: () => ({ previewed: true, editedPass: 7, a: aEvidence, b: bEvidence, c: cEvidence, variants: [...variantEvidence.values()], entryChunk: requireValue(requests[0], "Missing entry chunk").slice(7), current }),
    close: () => { for (const publication of publications.values()) publication.free(); },
    async handle(request: IncomingMessage, response: ServerResponse): Promise<boolean> {
      const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
      if (!path.startsWith("/reference-host/")) return false;
      try {
        const object = /^\/reference-host\/objects\/([0-9a-f]{64})\.cbor$/.exec(path);
        if (object?.[1] && request.method === "GET") {
          const bytes = objects.get(object[1]); if (!bytes) { response.writeHead(404).end(); return true; }
          response.writeHead(200, { "Content-Type": "application/cbor", "Content-Length": bytes.length, "Cache-Control": "public, max-age=31536000, immutable" }).end(bytes);
        } else if (path === "/reference-host/catalog" && request.method === "GET") send(response, { artifactId: current, edition: current === a.artifactId ? "A" : current === bEvidence?.artifactId ? "B" : "C" });
        else if (path === "/reference-host/publish-b" && request.method === "POST") { publishNext("B"); send(response, { artifactId: current }); }
        else if (path === "/reference-host/publish-c" && request.method === "POST") { publishNext("C"); send(response, { artifactId: current }); }
        else if (path === "/reference-host/original" && request.method === "POST") { current = a.artifactId; currentRecords = first.records; send(response, { artifactId: current }); }
        else if (path.startsWith("/reference-host/publish-") && request.method === "POST" && ["divergence", "recovery", "incompatible", "river-update"].includes(path.slice(24))) {
          const kind = path.slice(24);
          if (kind !== "divergence" && kind !== "recovery" && kind !== "incompatible" && kind !== "river-update") throw new Error("Unknown update variant");
          publishVariant(kind); send(response, { artifactId: current });
        }
        // Host-owned discovery for the optional evaluator, never used by cold reading.
        else if (path === "/reference-host/update-publications" && request.method === "GET") send(response, [...publications.values()].map(publication => ({
          publicationId: publication.publicationId, artifact: publication.artifactId, description: [...publication.descriptionBytes],
          previous: edges.get(publication.artifactId), selected: publication.artifactId === current, recoveryNames: recoveryNames.get(publication.artifactId),
        })));
        else if (path === "/reference-host/content" && request.method === "POST") send(response, referenceContent(await body(request)));
        else if (path === "/reference-host/reader-map" && request.method === "POST") {
          const value = await body(request);
          if (typeof value !== "object" || value === null || !("save" in value) || typeof value.save !== "string" || !("artifact" in value) || typeof value.artifact !== "string") throw new Error("Invalid map request");
          const publication = publications.get(value.artifact); if (!publication) throw new Error("Unknown saved publication");
          // This reference endpoint accepts the caller's checkpoint for personal spoiler
          // protection. It grants no authorization, rewards or proof of human reading.
          const engine = api.createEvolution();
          try {
            for (const publication of publications.values()) {
              const previous = edges.get(publication.artifactId);
              if (previous) await engine.registerEvolution(requireValue(publications.get(previous), "Missing previous publication"), publication);
              else await engine.registerPublication(publication);
            }
            await engine.restoreSave(value.save);
            const visits = await engine.visits();
            try { send(response, visits.readerMap(publication)); } finally { visits.free(); }
          } finally { await engine.close(); }
        }
        else if (path === "/reference-host/evidence" && request.method === "GET") send(response, this.evidence());
        else response.writeHead(404).end();
      } catch (error) { response.writeHead(400, { "Content-Type": "text/plain" }).end(error instanceof Error ? error.message : "Reference host failed"); }
      return true;
    },
  };
}
