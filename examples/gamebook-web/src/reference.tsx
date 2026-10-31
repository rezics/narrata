import { mountOnlineReference } from "./online.js";
import { ArtifactUnavailable, type Resolution } from "@rezics/narrata/reader";

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
  let edition: "A" | "B" | "C" = "A";
  let editionWithChangedEnding: string | undefined;
  const host = await mountOnlineReference(element, {
    currentArtifact: async () => {
      const response = await fetch("/reference-host/catalog", { cache: "no-store" });
      const value: unknown = await response.clone().json();
      if (!record(value) || (value.edition !== "A" && value.edition !== "B" && value.edition !== "C")) throw new Error("Invalid publication edition");
      edition = value.edition;
      return artifact(response);
    },
    currentEdition: () => edition,
    loadArtifact: async id => ({ manifest: await object(id.slice(9)), fetchChunk: chunk => object(chunk.slice(7)) }),
    publishB: async () => artifact(await fetch("/reference-host/publish-b", { method: "POST" })),
    publishC: async () => artifact(await fetch("/reference-host/publish-c", { method: "POST" })),
    useOriginal: async () => artifact(await fetch("/reference-host/original", { method: "POST" })),
    publishVariant: async kind => {
      const id = await artifact(await fetch(`/reference-host/publish-${kind}`, { method: "POST" }));
      editionWithChangedEnding = kind === "river-update" ? id : undefined;
      return id;
    },
    createUpdate: async (execution, storage, sourceArtifact) => {
      // Only an explicit review loads the evaluator and complete publications. Ordinary
      // historical reading stays on the manifest/chunk reader path above.
      const { loadAuthoring } = await import("@rezics/narrata/authoring");
      const api = await loadAuthoring();
      const response = await fetch("/reference-host/update-publications", { cache: "no-store" });
      if (!response.ok) throw new Error("Update service unavailable");
      const value: unknown = await response.json();
      if (!Array.isArray(value) || value.length < 1 || value.length > 64) throw new Error("Invalid publication list");
      const owned: import("@rezics/narrata/authoring").Publication[] = [];
      const evolution = api.createEvolution();
      const close = async () => { await evolution.close(); for (const publication of owned) publication.free(); };
      try {
        const edges = new Map<string, { previous: string; id: string }>();
        let selected: Record<string, unknown> | undefined;
        let unavailable = false;
        for (const entry of value) {
          if (!record(entry) || typeof entry.artifact !== "string" || !/^artifact:[0-9a-f]{64}$/.test(entry.artifact) || typeof entry.publicationId !== "string" || !Array.isArray(entry.description)
            || entry.description.length > 1024 * 1024 || !entry.description.every((byte: unknown) => typeof byte === "number" && Number.isInteger(byte) && byte >= 0 && byte <= 255)) throw new Error("Invalid publication metadata");
          if (entry.selected === true) selected = entry;
        }
        if (!selected || typeof selected.artifact !== "string") throw new Error("Missing update target");
        for (const entry of value) {
          if (!record(entry) || typeof entry.artifact !== "string" || typeof entry.publicationId !== "string" || !Array.isArray(entry.description)) throw new Error("Invalid publication metadata");
          const artifact = entry.artifact;
          const previous = typeof entry.previous === "string" ? owned.find(publication => publication.artifactId === entry.previous) : undefined;
          let publication: import("@rezics/narrata/authoring").Publication;
          try { publication = await api.readPublication(Uint8Array.from(entry.description), entry.publicationId, reference => object(reference.object_id)
            .catch(cause => { throw new ArtifactUnavailable(artifact, reference.object_id, { cause }); }), previous); }
          catch (error) { if (error instanceof ArtifactUnavailable) { unavailable = true; break; } throw error; }
          owned.push(publication);
          if (previous) edges.set(publication.artifactId, { previous: previous.artifactId, id: await evolution.registerEvolution(previous, publication) });
          else await evolution.registerPublication(publication);
        }
        const index = owned.findIndex(publication => publication.artifactId === sourceArtifact);
        if (index < 0 && !unavailable) throw new Error("Saved publication unavailable");
        const ids: string[] = []; let at = selected.artifact;
        while (at !== sourceArtifact) { const edge = edges.get(at); if (!edge) { ids.length = 0; break; } ids.unshift(edge.id); at = edge.previous; }
        const recoveryNames: Record<string, string> = {};
        if (record(selected.recoveryNames)) for (const [node, name] of Object.entries(selected.recoveryNames)) {
          if (!/^node:[0-9a-f]{32}$/.test(node) || typeof name !== "string" || name.length > 1024) throw new Error("Invalid recovery point name");
          recoveryNames[node] = name;
        }
        await evolution.connectStore(execution, storage);
        return { evolution, options: { target: selected.artifact, migrationIds: ids }, recoveryNames, close,
          checkpointForMap: async selection => {
            const { decodeLoaded, encodeLoadRequest } = await import("@rezics/narrata/storage");
            const storeId = decodeLoaded(await storage.load(encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] }))).store.id;
            const maintenance = api.createMaintenance({ storage, storeId });
            try {
              for (const publication of owned) await maintenance.registerPublication(publication);
              const bytes = await maintenance.exportSave({ execution, artifact: selection.artifact, cursor: selection.cursor, ancestors: 0 });
              return [...bytes].map(byte => byte.toString(16).padStart(2, "0")).join("");
            } finally { await maintenance.close(); }
          },
          copyTo: async (destination, storeId, nextExecution, selection, shallow) => {
            const { decodeLoaded, encodeLoadRequest } = await import("@rezics/narrata/storage");
            const sourceId = decodeLoaded(await storage.load(encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] }))).store.id;
            const from = api.createMaintenance({ storage, storeId: sourceId }), to = api.createMaintenance({ storage: destination, storeId });
            try {
              for (const publication of owned) { await from.registerPublication(publication); await to.registerPublication(publication); }
              const bytes = await from.exportSave({ execution, artifact: selection.artifact, cursor: selection.cursor, ...(shallow ? { ancestors: 0 } : {}) });
              const checked = await to.inspectSave(bytes);
              await to.importSave(bytes, { execution: nextExecution, ...checked, expected_revision: null, now: 0 });
            } finally { await from.close(); await to.close(); }
          } };
      } catch (error) { await close(); throw error; }
    },
    readerMap: async (save, artifact, cursor) => {
      const response = await fetch("/reference-host/reader-map", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ save, artifact }) });
      const { readReaderMapResponse } = await import("@rezics/narrata/reader-map");
      return readReaderMapResponse(response, { artifactId: artifact, cursor });
    },
    resolve: async request => {
      const response = await fetch("/reference-host/content", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(request) });
      if (!response.ok) throw new Error("Content service unavailable");
      const result = resolutions(await response.json());
      if (request.context.realization === editionWithChangedEnding) for (const [index, item] of request.items.entries()) {
        const content = item.content;
        const reference = "unit" in content ? content.unit : undefined;
        const resolution = result[index];
        if (reference?.key === "away" && resolution?.status === "ok" && "blocks" in resolution.payload) {
          result[index] = { ...resolution, revision: `${resolution.revision}:new-edition`, payload: { blocks: resolution.payload.blocks.map(block => ({ ...block,
            text: "The river carried Mara to a harbor she had not seen before." })) } };
        }
      }
      return result;
    },
  });
  // The reference harness observes the same host/session used by the visible reading UI.
  Object.assign(globalThis, { referenceJourney: host });
  return host;
}
