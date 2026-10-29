import { useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { AuthenticatedLibrary, createOnlineSession, createReader, LibraryClosed, openBook, WriteRejected } from "@rezics/narrata";
import type { ArtifactSource, ContentResolver, OnlineSession, ReaderController } from "@rezics/narrata";
import { Reader } from "@rezics/narrata/react";
import type { ContentSlotProps } from "@rezics/narrata/react";
import { decodeLoaded, encodeLoadRequest, equalBytes, IndexedDbStore } from "@rezics/narrata/storage";
import type { CacheStore, StoreState } from "@rezics/narrata/storage";
import type { Publication } from "@rezics/narrata/authoring";

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

/** Simulated authenticated host. Replace its CacheStore with an authenticated server transport. */
export async function createOnlineReference() {
  const published = await publications();
  const artifacts = new Map([[published.a.artifact, published.a.source]]);
  const records = new Map<string, LibraryRecord>();
  const journeys = new Map<string, string>();
  const listeners = new Set<() => void>();
  const closing = new Set<Promise<void>>();
  const writes: Uint8Array[] = [];
  const chunkRequests: { artifact: string; chunk: string }[] = [];
  const late: (() => void)[] = [];
  let current = published.a.artifact; let fault: Fault = "none"; let snapshot: HostSnapshot;
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
      return request.items.map(item => {
        const reference = "unit" in item.content ? item.content.unit : item.content;
        if (reference.provider !== "story" || request.context.viewer !== account) return { status: "unavailable" };
        const value = text[reference.key]; if (!value) return { status: "unavailable" };
        cache.set(reference.key, value);
        return { status: "ok", revision: "lantern:1", payload: "unit" in item.content ? { blocks: [{ id: "p1", text: value }] } : { text: value } };
      });
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
    const session = createOnlineSession({ library, execution: id, currentArtifact: async () => current,
      loadArtifact: async artifact => {
        const source = artifacts.get(artifact); if (!source) throw new Error("Saved artifact unavailable");
        return { ...source, fetchChunk: chunk => { chunkRequests.push({ artifact, chunk }); return source.fetchChunk(chunk); } };
      } });
    await session.start();
    if (generation !== connectionGeneration) { await session.close(); return; }
    controller = createReader(session, resolver, { online: session, context: { languages: ["en"], viewer: account } });
    await controller.start();
    if (generation !== connectionGeneration) { await session.close(); return; }
    snapshot = { binding: { session, controller }, account, publication: current === published.a.artifact ? "A" : "B" };
    notify();
  }
  await connect("reader-one");
  return {
    getSnapshot: () => snapshot,
    subscribe: (listener: () => void) => { listeners.add(listener); return () => listeners.delete(listener); },
    setFault: (mode: Fault) => { fault = mode; },
    releaseLate: () => { for (const done of late.splice(0)) done(); },
    hasLate: () => late.length > 0,
    libraryId: () => Array.from(records.get(snapshot.account)?.state.id ?? []),
    evidence: () => ({ writes: writes.map(bytes => Array.from(bytes)), chunkRequests: [...chunkRequests], artifactA: published.a.artifact, current }),
    publishB: () => { const b = published.publishB(); artifacts.set(b.artifact, b.source); current = b.artifact; snapshot = { ...snapshot, publication: "B" }; notify(); },
    reopen: () => connect(snapshot.account),
    newJourney: () => connect(snapshot.account, true),
    switchAccount: () => connect(snapshot.account === "reader-one" ? "reader-two" : "reader-one"),
    remove: async () => { const account = snapshot.account; await snapshot.binding.session.remove(); await connect(account, true); },
    dispose: async () => {
      connectionGeneration++;
      for (const done of late.splice(0)) done();
      await snapshot.binding.session.close(); await Promise.allSettled(closing);
      for (const record of records.values()) record.store.close();
      published.free(); listeners.clear();
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
  const busy = save.status === "saving";
  return <main style={{ maxWidth: 760, margin: "32px auto", padding: 24, lineHeight: 1.7 }}>
    <h1>Online story journey</h1>
    <p>Account: {state.account} · Current publication: {state.publication}</p>
    <nav aria-label="Online host actions" style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
      <button disabled={busy || state.publication === "B"} onClick={() => host.publishB()}>Publish B</button>
      <button disabled={busy || save.status === "unknown"} onClick={() => { void host.reopen(); }}>Reopen this journey</button>
      <button disabled={busy} onClick={() => { void host.newJourney(); }}>Start a new journey</button>
      <button onClick={() => { void host.switchAccount(); }}>Switch account</button>
    </nav>
    <details><summary>Simulate a save failure</summary>
      <button onClick={() => host.setFault("lost")}>Lose the next save response</button>{" "}
      <button onClick={() => host.setFault("quota")}>Reject the next save: storage full</button>
    </details>
    <Reader controller={state.binding.controller} slots={slots} showPath={false} showState={false} />
  </main>;
}
export async function mountOnlineReference(element: HTMLElement) {
  const host = await createOnlineReference(); const root = createRoot(element);
  root.render(<OnlineReference host={host} />);
  return { ...host, dispose: async () => { root.unmount(); await host.dispose(); } };
}
