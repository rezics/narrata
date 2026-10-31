import { lazy, Suspense, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { ArtifactUnavailable, AuthenticatedLibrary, createOnlineSession, createReader, LibraryClosed, openBook, WriteRejected } from "@rezics/narrata";
import type { ArtifactSource, ContentResolver, OnlineSession, ReaderController, Resolution } from "@rezics/narrata";
import { Reader } from "@rezics/narrata/react";
import type { ContentSlotProps } from "@rezics/narrata/react";
import { decodeLoaded, encodeLoadRequest, equalBytes, IndexedDbStore } from "@rezics/narrata/storage";
import type { CacheStore, StoreState } from "@rezics/narrata/storage";
import type { AssessmentV1, ComparisonBudgetV1, Evolution, EvolutionOptions, Publication } from "@rezics/narrata/authoring";
import type { ReaderMapV1 } from "@rezics/narrata/authoring";
const RevealedReaderMap = lazy(() => import("@rezics/narrata/reader-map/react").then(module => ({ default: module.RevealedReaderMap })));

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
type Fault = "none" | "lost" | "quota" | "late" | "uncommitted";
interface LibraryRecord { store: IndexedDbStore; state: StoreState; active: boolean; database: string }
interface UpdateContext {
  evolution: Evolution; options: EvolutionOptions; recoveryNames?: Readonly<Record<string, string>>; close(): Promise<void>;
  copyTo?(destination: CacheStore, storeId: Uint8Array, execution: string, selection: { artifact: string; cursor: string }, shallow: boolean): Promise<void>;
  checkpointForMap?(selection: { artifact: string; cursor: string }): Promise<string>;
}
interface Binding { session: OnlineSession; controller: ReaderController; reviewUpdate(budget?: ComparisonBudgetV1): Promise<AssessmentV1>; recoveryName(node: string): string | undefined;
  copyJourney(shallow: boolean): Promise<void>; mapSave(): Promise<{ save: string; artifactId: string; cursor: string }> }
interface HostSnapshot { binding: Binding; account: string; publication: "A" | "B" | "C"; targetArtifact: string }
export interface OnlineReferenceSource {
  readerMap?(save: string, artifact: string, cursor: string): Promise<ReaderMapV1>;
  currentArtifact(): Promise<string>;
  currentEdition?(): "A" | "B" | "C";
  loadArtifact(artifact: string): Promise<ArtifactSource>;
  resolve: ContentResolver["resolve"];
  publishB(): Promise<string>;
  publishC?(): Promise<string>;
  useOriginal?(): Promise<string>;
  publishVariant?(kind: "divergence" | "recovery" | "incompatible" | "river-update"): Promise<string>;
  createUpdate?(execution: string, storage: CacheStore, sourceArtifact: string): Promise<UpdateContext>;
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
  const directoryKey = remote ? "narrata-reference-cold" : "narrata-reference-online";
  let tabId = sessionStorage.getItem(directoryKey);
  if (!tabId || !/^[0-9a-f-]{36}$/.test(tabId)) { tabId = crypto.randomUUID(); sessionStorage.setItem(directoryKey, tabId); }
  let current = artifactA; let edition: HostSnapshot["publication"] = remote?.currentEdition?.() ?? "A"; let fault: Fault = "none"; let snapshot: HostSnapshot;
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
      const base = `${directoryKey}-${tabId}-${account}`;
      const selected = sessionStorage.getItem(`${base}-database`);
      const database = selected?.startsWith(base) ? selected : base;
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
        if (mode === "uncommitted") throw new Error("The save response was lost before commit");
        const reply = await fixed.store.persist(batch);
        if (mode === "lost") throw new Error("The save committed, but its response was lost");
        if (mode === "late") await new Promise<void>(done => late.push(done));
        return reply;
      },
    };
    let controller: ReaderController | undefined; let contentGeneration = 0;
    let update: UpdateContext | undefined;
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
      release: async () => { await update?.close(); }, clearContent: () => { contentGeneration++; cache.clear(); controller?.dispose(); },
      remove: async observed => {
        assertActive(); const actual = decodeLoaded(await fixed.store.load(empty)).store;
        if (!equalBytes(actual.id, observed.id) || actual.revision !== observed.revision) throw new Error("The library changed before deletion");
        fixed.active = false; fixed.store.close();
        await new Promise<void>((done, fail) => { const request = indexedDB.deleteDatabase(fixed.database); request.onsuccess = () => done(); request.onerror = () => fail(request.error); });
      } });
    const journeyKey = `${directoryKey}-${tabId}-${account}-execution`;
    let id = journeys.get(account) ?? sessionStorage.getItem(journeyKey) ?? undefined;
    if (fresh || !id || !/^execution:[0-9a-f]{32}$/.test(id)) id = execution();
    journeys.set(account, id); sessionStorage.setItem(journeyKey, id);
    const session = createOnlineSession({ library, execution: id, currentArtifact: () => remote ? remote.currentArtifact() : Promise.resolve(current),
      loadArtifact: async artifact => {
        const source = remote ? await remote.loadArtifact(artifact) : artifacts.get(artifact); if (!source) throw new Error("Saved artifact unavailable");
        return { ...source, fetchChunk: chunk => { chunkRequests.push({ artifact, chunk }); return source.fetchChunk(chunk); } };
      } });
    try { await session.start(); } catch (error) { await session.close(); throw error; }
    if (generation !== connectionGeneration) { await session.close(); return; }
    controller = createReader(session, resolver, { online: session, context: { languages: ["en"], viewer: account },
      ...(remote ? { contextForArtifact: (artifact: string, context: import("@rezics/narrata").ResolveContext) => ({ ...context, realization: artifact }) } : {}) });
    await controller.start();
    if (generation !== connectionGeneration) { await session.close(); return; }
    const reviewUpdate = async (budget?: ComparisonBudgetV1) => {
      if (!remote?.createUpdate || session.getSnapshot().status !== "saved") throw new Error("Resolve the saved journey first");
      await update?.close(); update = undefined;
      const source = session.getSnapshot().progress?.view.artifact_id;
      if (!source) throw new Error("Saved artifact unavailable");
      const context = await remote.createUpdate(id, library, source);
      if (generation !== connectionGeneration) { await context.close(); throw new LibraryClosed(); }
      update = context;
      return session.assessUpdate(context.evolution, { ...context.options, ...(budget ? { budget } : {}) });
    };
    const copyJourney = async (shallow: boolean) => {
      if (!remote?.createUpdate || session.getSnapshot().status !== "saved") throw new Error("Resolve the saved journey first");
      const selected = await session.exportConfirmedSave();
      const context = await remote.createUpdate(id, library, selected.artifactId);
      const base = `${directoryKey}-${tabId}-${account}`;
      const database = `${base}-import-${crypto.randomUUID()}`;
      const destination = await IndexedDbStore.open(database);
      try {
        const state = decodeLoaded(await destination.load(empty)).store;
        if (!context.copyTo) throw new Error("Maintenance service unavailable");
        const nextExecution = execution();
        await context.copyTo(destination, state.id, nextExecution, { artifact: selected.artifactId, cursor: selected.cursor }, shallow);
        if (generation !== connectionGeneration) throw new LibraryClosed();
        // Select only the acknowledged cold import. The original library remains on disk.
        sessionStorage.setItem(`${base}-database`, database);
        sessionStorage.setItem(journeyKey, nextExecution); journeys.set(account, nextExecution);
        records.delete(account);
      } finally { destination.close(); await context.close(); }
      try { await connect(account); } finally { fixed.store.close(); }
    };
    const mapSave = async () => {
      try { return await session.exportConfirmedSave(); }
      catch (error) {
        if (!(error instanceof Error) || !/^history_truncated(?:\s|:)/.test(error.message) || !remote?.createUpdate) throw error;
        const selected = session.getSnapshot();
        if (selected.status !== "saved" || !selected.progress) throw error;
        const { artifact_id: artifact, cursor } = selected.progress.view;
        const context = await remote.createUpdate(id, library, artifact);
        try {
          if (!context.checkpointForMap) throw error;
          const save = await context.checkpointForMap({ artifact, cursor });
          if (generation !== connectionGeneration || session.getSnapshot().status !== "saved" || session.getSnapshot().progress?.view.cursor !== cursor) throw new LibraryClosed();
          return { save, artifactId: artifact, cursor };
        } finally { await context.close(); }
      }
    };
    snapshot = { binding: { session, controller, reviewUpdate, copyJourney, mapSave, recoveryName: node => update?.recoveryNames?.[node] }, account, publication: edition, targetArtifact: current };
    sessionStorage.setItem(`${directoryKey}-account`, account);
    notify();
  }
  const account = sessionStorage.getItem(`${directoryKey}-account`);
  try { await connect(account === "reader-two" ? account : "reader-one"); }
  catch (error) { for (const record of records.values()) record.store.close(); published?.free(); throw error; }
  return {
    canMap: remote?.readerMap !== undefined,
    canUpdate: remote?.createUpdate !== undefined,
    canPublishC: remote?.publishC !== undefined,
    canTestUpdates: remote?.publishVariant !== undefined,
    readerMap: async () => {
      if (!remote?.readerMap) throw new Error("Map service unavailable");
      const binding = snapshot.binding;
      const exported = await binding.mapSave();
      const map = await remote.readerMap(exported.save, exported.artifactId, exported.cursor);
      const current = snapshot.binding.session.getSnapshot();
      if (binding !== snapshot.binding || current.status !== "saved" || !current.progress || current.progress.view.cursor !== map.sources[0]?.cursor || current.progress.view.artifact_id !== map.artifact_id) return undefined;
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
      edition = "B";
      if (snapshot.binding.session.getSnapshot().status === "saved") await snapshot.binding.session.continueOld();
      snapshot = { ...snapshot, publication: edition, targetArtifact: current }; notify();
    },
    publishC: async () => {
      if (!remote?.publishC) throw new Error("Publication service unavailable");
      current = await remote.publishC(); edition = "C";
      if (snapshot.binding.session.getSnapshot().status === "saved") await snapshot.binding.session.continueOld();
      snapshot = { ...snapshot, publication: edition, targetArtifact: current }; notify();
    },
    useOriginal: async () => {
      if (!remote?.useOriginal) throw new Error("Publication service unavailable");
      current = await remote.useOriginal(); edition = "A";
      await snapshot.binding.session.continueOld(); snapshot = { ...snapshot, publication: edition, targetArtifact: current }; notify();
    },
    publishVariant: async (kind: "divergence" | "recovery" | "incompatible" | "river-update") => {
      if (!remote?.publishVariant) throw new Error("Publication service unavailable");
      current = await remote.publishVariant(kind); edition = "C";
      await snapshot.binding.session.continueOld(); snapshot = { ...snapshot, publication: edition, targetArtifact: current }; notify();
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
  const reader = useSyncExternalStore(state.binding.controller.subscribe, state.binding.controller.getSnapshot);
  const [pending, setPending] = useState(false);
  const [updating, setUpdating] = useState(false);
  const [error, setError] = useState<{ binding: Binding; operation: "host" | "unknown_update" | "rejected_update"; sourceCursor: string | undefined } | null>(null);
  const [map, setMap] = useState<{ binding: Binding; cursor: string; data: ReaderMapV1 } | null>(null);
  const [review, setReview] = useState<{ binding: Binding; cursor: string; publication: string; assessment: AssessmentV1 } | null>(null);
  const [updateMessage, setUpdateMessage] = useState<{ binding: Binding; text: string } | null>(null);
  const showUpdate = (text: string) => setUpdateMessage(text ? { binding: state.binding, text } : null);
  const [shortReview, setShortReview] = useState(false);
  const actionGeneration = useRef(0);
  const assessment = review?.binding === state.binding && review.cursor === save.progress?.view.cursor && review.publication === state.targetArtifact ? review.assessment : undefined;
  const busy = save.status === "saving" || pending || reader.phase !== "ready";
  const recoveredUpdate = error?.binding === state.binding && save.status === "saved"
    && (error.operation === "rejected_update" || (error.operation === "unknown_update" && save.progress?.view.cursor !== error.sourceCursor));
  useEffect(() => { if (recoveredUpdate) setError(current => current === error ? null : current); }, [recoveredUpdate, error]);
  useEffect(() => {
    if (updateMessage?.binding === state.binding && updateMessage.text === "Your update is saved, but the new story screen could not be loaded. Retry loading it before continuing." && reader.phase === "ready") setUpdateMessage(null);
  }, [reader.phase, state.binding, updateMessage]);
  async function action(operation: () => Promise<unknown>, kind: "host" | "update" = "host") {
    const generation = ++actionGeneration.current;
    setPending(true); setError(null);
    if (kind === "update") setUpdating(true);
    try { await operation(); } catch { if (generation === actionGeneration.current) {
      const status = state.binding.session.getSnapshot().status;
      setError({ binding: state.binding, operation: kind === "update" && status === "unknown" ? "unknown_update"
        : kind === "update" && status !== "saved" ? "rejected_update" : "host", sourceCursor: save.progress?.view.cursor });
    } }
    finally { if (generation === actionGeneration.current) { setPending(false); setUpdating(false); } }
  }
  return <main style={{ maxWidth: 760, margin: "32px auto", padding: 24, lineHeight: 1.7 }}>
    <h1>Online story journey</h1>
    <p>Account: {state.account} · Current publication: {state.publication}</p>
    <nav aria-label="Online host actions" style={{ display: "flex", flexWrap: "wrap", gap: 8 }}>
      <button disabled={busy || state.publication !== "A"} onClick={() => { void action(host.publishB); }}>Publish B</button>
      {host.canPublishC ? <button disabled={busy || state.publication !== "B"} onClick={() => { void action(host.publishC); }}>Publish C</button> : null}
      <button disabled={busy || save.status === "unknown"} onClick={() => { void action(host.reopen); }}>Reopen this journey</button>
      <button disabled={busy} onClick={() => { void action(host.newJourney); }}>Start a new journey</button>
      <button disabled={save.status === "closed"} onClick={() => { void action(host.switchAccount); }}>Switch account</button>
      {host.canMap ? <button disabled={busy || save.status !== "saved"} onClick={() => { setMap(null); void action(async () => { const data = await host.readerMap(); if (data) setMap({ binding: state.binding, cursor: data.sources[0]?.cursor ?? "", data }); }); }}>Show revealed map</button> : null}
      {host.canUpdate ? <button disabled={busy || save.status !== "saved"} onClick={() => { setReview(null); showUpdate(""); void action(async () => {
        const cursor = save.progress?.view.cursor; if (!cursor) return;
        const assessment = await state.binding.reviewUpdate(shortReview ? { transitions: 0, automatic_steps: 100_000, object_bytes: 64 * 1024 * 1024 } : undefined);
        setReview({ binding: state.binding, cursor, publication: state.targetArtifact, assessment });
      }); }}>Review story update</button> : null}
    </nav>
    {host.canUpdate ? <label>Update review <select aria-label="Update review depth" disabled={busy || save.status !== "saved"} value={shortReview ? "short" : "standard"} onChange={event => {
      setShortReview(event.target.value === "short"); setReview(null); showUpdate(""); void action(() => state.binding.session.continueOld());
    }}><option value="standard">Compare earlier choices</option><option value="short">Current position only (requires confirmation)</option></select></label> : null}
    {error?.binding === state.binding && !recoveredUpdate ? <p role="alert">The host action failed. Your saved journey is preserved; try the action again.</p> : null}
    {assessment ? <section aria-label="Story update review" data-assessment={assessment.kind}
      data-reason={assessment.kind === "needs_confirmation" ? assessment.reason : undefined}>
      <p>{assessment.kind === "exact" ? "This journey already uses the current story."
        : assessment.kind === "automatic" ? "Your journey can continue in the updated story."
        : assessment.kind === "needs_confirmation" ? "The updated story changes this journey. Review the consequences before confirming."
        : assessment.kind === "old_version_only" ? "This journey cannot move to the updated story. You can continue the old version."
        : assessment.kind === "needs_objects" ? "Some saved story objects are unavailable. Retry the review."
        : "This save could not be read. Your stored progress has been preserved."}</p>
      {assessment.kind === "automatic" || assessment.kind === "needs_confirmation" ? <>
        <p>{assessment.plan.report.path.kind === "equivalent" ? "Your earlier choices have the same results."
          : assessment.plan.report.path.kind === "diverged" ? "Your earlier choices have different results in this edition."
          : "Some earlier choices could not be compared. Continuing may change their results."}</p>
        {assessment.plan.report.losses.map((loss, index) => <div key={index}>
          <p>This journey will continue at {state.binding.recoveryName(loss.target.node) ?? "a recovery choice"}.{loss.cleared_finished ? " Your ending will reopen." : ""}</p>
          {loss.reset_local_keys.length ? <p>Story values reset: {loss.reset_local_keys.join(", ")}.</p> : null}
          {loss.discarded_frames.length ? <p>Scenes left behind: {loss.discarded_frames.length}. Their pending choices and scene values will be discarded.</p> : null}
          {loss.retained_shared_keys.length ? <p>Journey values kept: {loss.retained_shared_keys.join(", ")}.</p> : null}
        </div>)}
        <button disabled={busy || save.status !== "saved"} onClick={() => { void action(async () => {
          setReview(null);
          const result = await state.binding.session.applyUpdate(assessment.kind === "needs_confirmation");
          if (result.kind === "migrated") {
            if (host.getSnapshot().binding !== state.binding || state.binding.session.getSnapshot().status !== "saved") return;
            const refreshed = await state.binding.controller.refresh();
            showUpdate(refreshed ? "Story update saved." : "Your update is saved, but the new story screen could not be loaded. Retry loading it before continuing.");
          }
          else showUpdate("The update was not applied. Reload saved progress and review again.");
        }, "update"); }}>{assessment.kind === "needs_confirmation" ? "Confirm story changes" : "Apply story update"}</button>
      </> : null}
      {["exact", "automatic", "needs_confirmation", "old_version_only", "needs_objects"].includes(assessment.kind) ? <button disabled={busy || save.status !== "saved"} onClick={() => { void action(async () => { await state.binding.session.continueOld(); setReview(null); showUpdate("Continuing the saved story version."); }); }}>Continue saved version</button> : null}
    </section> : null}
    {updateMessage?.binding === state.binding ? <p role="status">{updateMessage.text}</p> : null}
    <details><summary>Simulate a save failure</summary>
      <button onClick={() => host.setFault("lost")}>Lose the next save response</button>{" "}
      <button onClick={() => host.setFault("quota")}>Reject the next save: storage full</button>
      <button onClick={() => host.setFault("uncommitted")}>Drop the next save before commit</button>
      <button onClick={() => host.setFault("late")}>Delay the next save response</button>{" "}
      <button onClick={() => host.releaseLate()}>Release delayed save responses</button>
    </details>
    {host.canTestUpdates ? <details><summary>Publication and recovery scenarios</summary>
      <button disabled={busy || save.status !== "saved"} onClick={() => { void action(host.useOriginal); }}>Use original publication</button>
      <button disabled={busy || save.status !== "saved"} onClick={() => { void action(() => host.publishVariant("divergence")); }}>Publish changed earlier journey</button>
      <button disabled={busy || save.status !== "saved"} onClick={() => { void action(() => host.publishVariant("recovery")); }}>Publish recovery edition</button>
      <button disabled={busy || save.status !== "saved"} onClick={() => { void action(() => host.publishVariant("incompatible")); }}>Publish incompatible edition</button>
      <button disabled={busy || save.status !== "saved"} onClick={() => { void action(() => host.publishVariant("river-update")); }}>Publish delayed-content edition</button>
      <button disabled={busy || save.status !== "saved"} onClick={() => { void action(() => state.binding.copyJourney(false)); }}>Import this journey into a new library</button>
      <button disabled={busy || save.status !== "saved"} onClick={() => { void action(() => state.binding.copyJourney(true)); }}>Restore recent history into a new library</button>
    </details> : null}
    <Reader controller={state.binding.controller} slots={slots} showState={false} disabled={save.status === "saving" || updating} />
    {map && save.status === "saved" && save.progress && map.binding === state.binding && map.data.artifact_id === save.progress.view.artifact_id && map.cursor === save.progress.view.cursor ? <Suspense fallback={<p role="status">Opening map…</p>}><RevealedReaderMap map={map.data} /></Suspense> : null}
  </main>;
}
export async function mountOnlineReference(element: HTMLElement, source?: OnlineReferenceSource) {
  let host: ReferenceHost;
  for (;;) {
    try { host = await createOnlineReference(source); break; }
    catch (error) {
      const message = document.createElement("p"); message.role = "alert";
      message.textContent = error instanceof ArtifactUnavailable ? "The saved story version is unavailable. Your progress is preserved."
        : "Saved progress could not be read. Your stored journey is preserved.";
      const retry = document.createElement("button"); retry.textContent = "Retry opening saved journey";
      element.replaceChildren(message, retry);
      await new Promise<void>(done => { retry.addEventListener("click", () => { retry.disabled = true; done(); }, { once: true }); });
    }
  }
  element.replaceChildren(); const root = createRoot(element);
  root.render(<OnlineReference host={host} />);
  return { ...host, dispose: async () => { root.unmount(); await host.dispose(); } };
}
