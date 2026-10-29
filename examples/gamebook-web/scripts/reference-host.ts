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
  const aEvidence = collect(a); let bEvidence: ReturnType<typeof collect> | undefined; let current = a.artifactId;
  function publishB() {
    if (bEvidence) { current = bEvidence.artifactId; return; }
    const next = api.createDrafts();
    try {
      for (const record of first.records) {
        const edited = decodeRecord(record.bytes);
        if (edited.kind === "choice_point" && edited.payload.key === "finish") edited.payload.options = edited.payload.options.filter(option => option.key !== "linger");
        next.add(edited);
      }
      next.addOutline(referenceStoryOutline());
      const result = next.publish({ previous: a });
      const b = requireValue(result.publication, JSON.stringify(result.report.diagnostics));
      try { bEvidence = collect(b); current = b.artifactId; } finally { b.free(); }
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
    evidence: () => ({ previewed: true, editedPass: 7, a: aEvidence, b: bEvidence, entryChunk: requireValue(requests[0], "Missing entry chunk").slice(7), current }),
    close: () => a.free(),
    async handle(request: IncomingMessage, response: ServerResponse): Promise<boolean> {
      const path = new URL(request.url ?? "/", "http://127.0.0.1").pathname;
      if (!path.startsWith("/reference-host/")) return false;
      try {
        const object = /^\/reference-host\/objects\/([0-9a-f]{64})\.cbor$/.exec(path);
        if (object?.[1] && request.method === "GET") {
          const bytes = objects.get(object[1]); if (!bytes) { response.writeHead(404).end(); return true; }
          response.writeHead(200, { "Content-Type": "application/cbor", "Content-Length": bytes.length, "Cache-Control": "public, max-age=31536000, immutable" }).end(bytes);
        } else if (path === "/reference-host/catalog" && request.method === "GET") send(response, { artifactId: current });
        else if (path === "/reference-host/publish-b" && request.method === "POST") { publishB(); send(response, { artifactId: current }); }
        else if (path === "/reference-host/content" && request.method === "POST") send(response, referenceContent(await body(request)));
        else if (path === "/reference-host/evidence" && request.method === "GET") send(response, this.evidence());
        else response.writeHead(404).end();
      } catch (error) { response.writeHead(400, { "Content-Type": "text/plain" }).end(error instanceof Error ? error.message : "Reference host failed"); }
      return true;
    },
  };
}
