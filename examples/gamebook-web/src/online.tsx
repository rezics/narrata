import { lazy, Suspense, useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { AuthenticatedLibrary, createOnlineSession, createReader, LibraryClosed, openBook, WriteRejected } from "@rezics/narrata";
import type { ArtifactSource, ContentResolver, OnlineSession, ReaderController, Resolution } from "@rezics/narrata";
import { Reader } from "@rezics/narrata/react";
import type { ContentSlotProps } from "@rezics/narrata/react";
import { decodeLoaded, encodeLoadRequest, equalBytes, IndexedDbStore } from "@rezics/narrata/storage";
import type { CacheStore, StoreState } from "@rezics/narrata/storage";
import type { Publication } from "@rezics/narrata/authoring";
import type { ReaderMapV1 } from "@rezics/narrata/authoring";
const RevealedReaderMap = lazy(() => import("@rezics/narrata/graph/react").then(module => ({ default: module.RevealedReaderMap })));

// This is host-owned sample content. Program artifacts and online saves contain only references.
const text: Readonly<Record<string, string>> = {
  title: "The Lantern Path", start: "At dusk, Mara found a lantern beside the fork in the path. A warm light moved between the trees; below, the river carried a silver reflection.",
  forest: "Under the cedar branches, Mara found the keeper searching for his lost lantern. He offered to guide her home, but a second light shone deeper in the woods.",
  river: "Mara followed the silver current until the lights of a village appeared across the water.",
  "option.forest": "Follow the light into the forest", "option.river": "Walk beside the river",
  "option.home": "Return home with the keeper", "option.wander": "Seek the second light",
  "ending.home": "A light in the window", "ending.away": "Beyond the familiar path",
  "scene.start": "The fork at dusk", "scene.forest": "The lantern keeper", "scene.river": "Silver water",
};
interface StoredPublication { artifact: string; source: ArtifactSource }
function stored(publication: Publication): StoredPublication {
  const objects = new Map([...publication.objects()].map(([reference, bytes]) => [reference.object_id, bytes.slice()]));
  const manifest = objects.get(publication.artifactId.slice(9));
  const names = publication.description.names ? objects.get(publication.description.names) : undefined;
  if (!manifest || !names) throw new Error("Publication omitted its reader objects");
  return { artifact: publication.artifactId, source: { manifest, names, fetchChunk: async id => {
    const bytes = objects.get(id.slice(7)); if (!bytes) throw new Error("Historic artifact object unavailable"); return bytes;
  } } };
}

/** Public authoring output, including an edit before preview and A, and a later edit for B. */
async function publications() {
  const [{ loadAuthoring, decodeRecord }, { shortStorySource, shortStoryOutline }] = await Promise.all([
    import("@rezics/narrata/authoring"), import("@rezics/narrata/testing"),
  ]);
  const api = await loadAuthoring();
  const source = shortStorySource();
  for (const [, pkg] of source.packages) for (const graph of Object.values(pkg.graphs)) for (const [key, node] of Object.entries(graph.nodes)) {
    if (node.type_id === "narrata.passage" && typeof node.data === "object" && node.data !== null && !Array.isArray(node.data)) {
      node.data = { ...node.data, title: { provider: "story", key: `scene.${key}` } };
    }
  }
  const minter = api.createMinter();
  const filled = minter.fill(source.manifest, source.packages); minter.free();
  const records = filled.records.map(record => decodeRecord(record.bytes));
  for (const record of records) if (record.kind === "choice_point" && record.payload.key === "path") record.payload.options.reverse();
  const drafts = api.createDrafts();
  for (const record of records) drafts.add(record);
  drafts.addOutline(shortStoryOutline());
  const preview = drafts.preview();
  try {
    const manifest = preview.manifest(); const names = preview.names();
    if (!manifest || !names) throw new Error("The short story did not compile");
    const book = await openBook({ manifest, names, execution: execution(), fetchChunk: async id => {
      const bytes = preview.object(id); if (!bytes) throw new Error("Preview object unavailable"); return bytes;
    } });
    try { if ((await book.inspect()).view.interaction.kind !== "choose") throw new Error("Preview has no opening choice"); }
    finally { await book.close(); }
  } finally { preview.free(); }
  const first = drafts.publish(); drafts.free();
  if (!first.publication) throw new Error("Publication A failed");
  const publicationA = first.publication;
  const a = stored(publicationA);
  return { a, publishB: () => {
    const next = api.createDrafts();
    try {
      for (const record of first.records) {
        const edited = decodeRecord(record.bytes);
        if (edited.kind === "choice_point" && edited.payload.key === "return") edited.payload.options = edited.payload.options.filter(option => option.key !== "wander");
        next.add(edited);
      }
      next.addOutline(shortStoryOutline());
      const result = next.publish({ previous: publicationA });
      if (!result.publication) throw new Error("Publication B failed");
      try { return stored(result.publication); } finally { result.publication.free(); }
    } finally { next.free(); }
  }, free: () => publicationA.free() };
}
function execution() { return `execution:${crypto.randomUUID().replaceAll("-", "")}`; }
const empty = encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] });
type Fault = "none" | "lost" | "quota" | "late";
interface LibraryRecord { store: IndexedDbStore; state: StoreState; active: boolean; database: string }
interface Binding { session: OnlineSession; controller: ReaderController }
interface HostSnapshot { binding: Binding; account: string; publication: "A" | "B" }
export interface OnlineReferenceSource {
  readerMap?(save: string, artifact: string): Promise<ReaderMapV1>;
  currentArtifact(): Promise<string>;
  loadArtifact(artifact: string): Promise<ArtifactSource>;
  resolve: ContentResolver["resolve"];
  publishB(): Promise<string>;
}

/** Simulated authenticated host. Replace its CacheStore with an authenticated server transport. */
export async function createOnlineReference(remote?: OnlineReferenceSource) {
  const published = remote ? undefined : await publications();
  const artifacts = new Map(published ? [[published.a.artifact, published.a.source]] : []);
  const artifactA = remote ? await remote.currentArtifact() : published?.a.artifact;
  if (!artifactA) throw new Error("Reference publication unavailable");
  const records = new Map<string, LibraryRecord>();
  const journeys = new Map<string, string>();
  const listeners = new Set<() => void>();
  const closing = new Set<Promise<void>>();
  const writes: Uint8Array[] = [];
  const chunkRequests: { artifact: string; chunk: string }[] = [];
  const late: (() => void)[] = [];
  let current = artifactA; let fault: Fault = "none"; let snapshot: HostSnapshot;
  let connectionGeneration = 0;
  function notify() { for (const listener of listeners) listener(); }
  function close(binding: Binding) {
    const pending = binding.session.close(); closing.add(pending);
    void pending.finally(() => closing.delete(pending)).catch(() => undefined);
  }
  async function connect(account: string, fresh = false) {
    const generation = ++connectionGeneration;
    if (snapshot) close(snapshot.binding);
    let record = records.get(account);
    if (!record?.active) {
      const database = `narrata-reference-online-${crypto.randomUUID()}`;
      const store = await IndexedDbStore.open(database);
      if (generation !== connectionGeneration) { store.close(); return; }
      const state = decodeLoaded(await store.load(empty)).store;
      if (generation !== connectionGeneration) { store.close(); return; }
      record = { store, database, active: true, state };
      records.set(account, record);
    }
    const fixed = record;
    const assertActive = () => { if (!fixed.active) throw new WriteRejected("unavailable"); };
    const transport: CacheStore = {
      load: request => { assertActive(); return fixed.store.load(request); },
      persist: async batch => {
        assertActive(); writes.push(batch.slice());
        const mode = fault; fault = "none";
        if (mode === "quota") throw new WriteRejected("quota");
        const reply = await fixed.store.persist(batch);
        if (mode === "lost") throw new Error("The save committed, but its response was lost");
        if (mode === "late") await new Promise<void>(done => late.push(done));
        return reply;
      },
    };
    let controller: ReaderController | undefined; let contentGeneration = 0;
    const cache = new Map<string, string>();
    const resolver: ContentResolver = { resolve: async request => {
      const generation = contentGeneration;
      await Promise.resolve();
      if (generation !== contentGeneration) throw new LibraryClosed();
      const result = remote ? await remote.resolve(request) : request.items.map((item): Resolution => {
        const reference = "unit" in item.content ? item.content.unit : item.content;
        if (reference.provider !== "story" || request.context.viewer !== account) return { status: "unavailable" };
        const value = text[reference.key]; if (!value) return { status: "unavailable" };
        cache.set(reference.key, value);
        return { status: "ok", revision: "lantern:1", payload: "unit" in item.content ? { blocks: [{ id: "p1", text: value }] } : { text: value } };
      });
      if (generation !== contentGeneration) throw new LibraryClosed();
      return result;
    } };
    const library = new AuthenticatedLibrary({ store: transport, storeId: fixed.state.id,
      release: () => undefined, clearContent: () => { contentGeneration++; cache.clear(); controller?.dispose(); },
      remove: async observed => {
        assertActive(); const actual = decodeLoaded(await fixed.store.load(empty)).store;
        if (!equalBytes(actual.id, observed.id) || actual.revision !== observed.revision) throw new Error("The library changed before deletion");
        fixed.active = false; fixed.store.close();
        await new Promise<void>((done, fail) => { const request = indexedDB.deleteDatabase(fixed.database); request.onsuccess = () => done(); request.onerror = () => fail(request.error); });
      } });
    let id = journeys.get(account); if (fresh || !id) { id = execution(); journeys.set(account, id); }
    const session = createOnlineSession({ library, execution: id, currentArtifact: () => remote ? remote.currentArtifact() : Promise.resolve(current),
      loadArtifact: async artifact => {
        const source = remote ? await remote.loadArtifact(artifact) : artifacts.get(artifact); if (!source) throw new Error("Saved artifact unavailable");
        return { ...source, fetchChunk: chunk => { chunkRequests.push({ artifact, chunk }); return source.fetchChunk(chunk); } };
      } });
    await session.start();
    if (generation !== connectionGeneration) { await session.close(); return; }
    controller = createReader(session, resolver, { online: session, context: { languages: ["en"], viewer: account } });
    await controller.start();
    if (generation !== connectionGeneration) { await session.close(); return; }
    snapshot = { binding: { session, controller }, account, publication: current === artifactA ? "A" : "B" };
    notify();
  }
  await connect("reader-one");
  return {
    canMap: remote?.readerMap !== undefined,
    readerMap: async () => {
      if (!remote?.readerMap) throw new Error("Map service unavailable");
      const binding = snapshot.binding;
      const view = (await binding.session.inspect()).view;
      const map = await remote.readerMap(await binding.session.exportSave(), view.artifact_id);
      if (binding !== snapshot.binding) throw new LibraryClosed();
      return map;
    },
    getSnapshot: () => snapshot,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => listeners.delete(listener); },
    setFault: (mode: Fault) => { fault = mode; },
    releaseLate: () => { for (const done of late.splice(0)) done(); },
    hasLate: () => late.length > 0,
    libraryId: () => Array.from(records.get(snapshot.account)?.state.id ?? []),
    evidence: () => ({ writes: writes.map(bytes => Array.from(bytes)), chunkRequests: [...chunkRequests], artifactA, current }),
    publishB: async () => {
      if (remote) current = await remote.publishB();
      else if (published) { const b = published.publishB(); artifacts.set(b.artifact, b.source); current = b.artifact; }
      snapshot = { ...snapshot, publication: "B" }; notify();
    },
    reopen: () => connect(snapshot.account),
    newJourney: () => connect(snapshot.account, true),
    switchAccount: () => connect(snapshot.account === "reader-one" ? "reader-two" : "reader-one"),
    otherDeviceChoose: async (labelKey: string) => {
      const record = records.get(snapshot.account); const id = journeys.get(snapshot.account);
      if (!record?.active || !id) throw new Error("No active reference library");
      const peer = createOnlineSession({ execution: id, library: new AuthenticatedLibrary({ store: record.store, storeId: record.state.id,
        clearContent: () => undefined, release: () => undefined, remove: async () => { throw new Error("Peer cannot delete this library"); } }),
        currentArtifact: () => remote ? remote.currentArtifact() : Promise.resolve(current),
        loadArtifact: async artifact => { const source = remote ? await remote.loadArtifact(artifact) : artifacts.get(artifact); if (!source) throw new Error("Historic artifact unavailable"); return source; },
      });
      try {
        await peer.start(); const view = (await peer.inspect()).view;
        if (view.interaction.kind !== "choose") throw new Error("Peer expected a choice");
        const option = view.interaction.options.find(option => option.label?.key === labelKey);
        if (!option) throw new Error("Peer option unavailable");
        await peer.choose(view.cursor, view.interaction.choice_point, [option.id]);
      } finally { await peer.close(); }
    },
    remove: async () => { const account = snapshot.account; await snapshot.binding.session.remove(); await connect(account, true); },
    dispose: async () => {
      connectionGeneration++;
      for (const done of late.splice(0)) done();
      await snapshot.binding.session.close(); await Promise.allSettled(closing);
      for (const record of records.values()) record.store.close();
      published?.free(); listeners.clear();
    },
  };
}
type ReferenceHost = Awaited<ReturnType<typeof createOnlineReference>>;
function slot({ role, resolution }: ContentSlotProps) {
  if (resolution.status !== "ok") return <span data-host-role={role}>Content unavailable</span>;
  return "text" in resolution.payload ? <span data-host-role={role}>{resolution.payload.text}</span>
    : <div data-host-role={role}>{resolution.payload.blocks.map(block => <p key={block.id}>{block.text}</p>)}</div>;
}
const slots = { body: slot, option: slot, reference: slot };
function OnlineReference({ host }: { host: ReferenceHost }) {
  const state = useSyncExternalStore(host.subscribe, host.getSnapshot);
  const save = useSyncExternalStore(state.binding.session.subscribe, state.binding.session.getSnapshot);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState(false);
  const [map, setMap] = useState<{ cursor: string; data: ReaderMapV1 } | null>(null);
  const busy = save.status === "saving" || pending;
  async function action(operation: () => Promise<unknown>) {
    setPending(true); setError(false);
    try { await operation(); } catch { setError(true); } finally { setPending(false); }
  }
  return <main style={{ maxWidth: 760, margin: "32px auto", padding: 24, lineHeight: 1.7 }}>
    <h1>Online story journey</h1>
    <p>Account: {state.account} · Current publication: {state.publication}</p>
    <nav aria-label="Online host actions" style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
      <button disabled={busy || state.publication === "B"} onClick={() => { void action(host.publishB); }}>Publish B</button>
      <button disabled={busy || save.status === "unknown"} onClick={() => { void action(host.reopen); }}>Reopen this journey</button>
      <button disabled={busy} onClick={() => { void action(host.newJourney); }}>Start a new journey</button>
      <button disabled={pending} onClick={() => { void action(host.switchAccount); }}>Switch account</button>
      {host.canMap ? <button disabled={busy || save.status !== "saved"} onClick={() => { void action(async () => { const data = await host.readerMap(); setMap({ cursor: data.sources[0]?.cursor ?? "", data }); }); }}>Show revealed map</button> : null}
    </nav>
    {error ? <p role="alert">The host action failed. Your saved journey is preserved; try the action again.</p> : null}
    <details><summary>Simulate a save failure</summary>
      <button onClick={() => host.setFault("lost")}>Lose the next save response</button>{" "}
      <button onClick={() => host.setFault("quota")}>Reject the next save: storage full</button>
    </details>
    <Reader controller={state.binding.controller} slots={slots} showPath={false} showState={false} />
    {map && map.cursor === save.progress?.view.cursor ? <Suspense fallback={<p role="status">Opening map…</p>}><RevealedReaderMap map={map.data} /></Suspense> : null}
  </main>;
}
export async function mountOnlineReference(element: HTMLElement, source?: OnlineReferenceSource) {
  const host = await createOnlineReference(source); const root = createRoot(element);
  root.render(<OnlineReference host={host} />);
  return { ...host, dispose: async () => { root.unmount(); await host.dispose(); } };
}
