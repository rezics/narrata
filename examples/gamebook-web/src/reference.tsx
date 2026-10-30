import { mountOnlineReference } from "./online.js";
import type { Resolution } from "@rezics/narrata/reader";

function record(value: unknown): value is Record<string, unknown> { return typeof value === "object" && value !== null && !Array.isArray(value); }
function resolutions(value: unknown): Resolution[] {
  if (!Array.isArray(value) || value.length > 4096) throw new Error("Invalid content response");
  return value.map((item: unknown): Resolution => {
    if (!record(item)) throw new Error("Invalid content result");
    if (item.status === "unavailable") return { status: "unavailable" };
    if (item.status !== "ok" || typeof item.revision !== "string" || !record(item.payload)) throw new Error("Invalid content payload");
    if (typeof item.payload.text === "string") return { status: "ok", revision: item.revision, payload: { text: item.payload.text } };
    if (!Array.isArray(item.payload.blocks) || item.payload.blocks.length > 4096) throw new Error("Invalid content blocks");
    const blocks = item.payload.blocks.map((block: unknown) => {
      if (!record(block) || typeof block.id !== "string" || typeof block.text !== "string") throw new Error("Invalid content block");
      return { id: block.id, text: block.text };
    });
    return { status: "ok", revision: item.revision, payload: { blocks } };
  });
}

async function artifact(response: Response): Promise<string> {
  if (!response.ok) throw new Error("Publication service unavailable");
  const value: unknown = await response.json();
  if (typeof value !== "object" || value === null || !("artifactId" in value) || typeof value.artifactId !== "string"
    || !/^artifact:[0-9a-f]{64}$/.test(value.artifactId)) throw new Error("Invalid publication identity");
  return value.artifactId;
}
async function object(id: string): Promise<Uint8Array> {
  if (!/^[0-9a-f]{64}$/.test(id)) throw new Error("Invalid object identity");
  const response = await fetch(`/reference-host/objects/${id}.cbor`);
  if (!response.ok) throw new Error("Saved artifact object unavailable");
  return new Uint8Array(await response.arrayBuffer());
}

/** Accepted cold reference route: only immutable manifests/chunks enter this reader realm. */
export async function mountColdReference(element: HTMLElement) {
  const host = await mountOnlineReference(element, {
    currentArtifact: async () => artifact(await fetch("/reference-host/catalog", { cache: "no-store" })),
    loadArtifact: async id => ({ manifest: await object(id.slice(9)), fetchChunk: chunk => object(chunk.slice(7)) }),
    publishB: async () => artifact(await fetch("/reference-host/publish-b", { method: "POST" })),
    readerMap: async (save, artifact, cursor) => {
      const response = await fetch("/reference-host/reader-map", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ save, artifact }) });
      const { readReaderMapResponse } = await import("@rezics/narrata/reader-map");
      return readReaderMapResponse(response, { artifactId: artifact, cursor });
    },
    resolve: async request => {
      const response = await fetch("/reference-host/content", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(request) });
      if (!response.ok) throw new Error("Content service unavailable");
      return resolutions(await response.json());
    },
  });
  // The reference harness observes the same host/session used by the visible reading UI.
  Object.assign(globalThis, { referenceJourney: host });
  return host;
}
