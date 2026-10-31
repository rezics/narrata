# @rezics/narrata

The ESM package for hosts that run Narrata Gamebooks in a browser, Node or Bun. Content references
in `BookView` are resolved by the host; the runtime maintains structure, state and history.

Build and pack from the repository with `task web:pack`, then install the printed `.tgz` path
with npm. The tarball includes JavaScript, TypeScript declarations and a separate Wasm asset;
the runtime and storage entries need no runtime npm imports or repository paths. The graph
entry installs Sigma and graphology, loaded only when mounting an author view. The optional
`@rezics/narrata/react` entry uses the host's React 18 or 19 installation; React is an optional
peer and is not shipped inside the package; `@rezics/narrata/graph/react` follows the same convention.

```ts
import { openBook } from "@rezics/narrata";
import { IndexedDbStore } from "@rezics/narrata/storage";

async function bytes(url: string): Promise<Uint8Array> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return new Uint8Array(await response.arrayBuffer());
}

// Keep the same execution ID and database name when reopening this reading session.
const execution = "execution:0190f2a0000070008000000000000001";
const store = await IndexedDbStore.open(`my-gamebook-${execution}`);
const book = await openBook({
  execution,
  manifest: await bytes("/story/manifest.cbor"),
  fetchChunk: id => bytes(`/story/objects/${id.slice(7)}.cbor`),
  storage: store,
});
try {
  const view = await book.inspect();
  const interaction = view.view.interaction;
  if (interaction.kind === "choose") {
    const option = interaction.options.find(option => option.enabled);
    if (option) await book.choose(view.view.cursor, interaction.choice_point, [option.id]);
  }
  const checkpoint = await book.exportSave(); // Hand these bytes to the host's export UI.
} finally {
  await book.close();
  store.close();
}
```

`openBook` resolves after creating or reopening the session. Missing program objects are fetched
and checked before retrying the pending operation. Calls on a book run in order; choices resolve
only after the storage host persists and confirms their batches. A failed write rejects the call;
use `reload()` to discard unconfirmed changes before continuing. Another writer's newer state
is rejected rather than overwritten. Without `storage`, the session stays in memory.

## Save maintenance

Load maintenance from the authoring entry on demand (`server` in Node/Bun). Supply a host-authorized
library transport and its immutable StoreId, then register the exact trusted publications needed
by the save, including historical artifacts after migrations.
When reopening with `openBook`, supply `loadArtifact(artifactId)` for historical manifests,
optional names and their own `fetchChunk` loader. Registration reads metadata only; history
rendering loads the chunks it uses. `historyPacks` remains available for full-pack imports.
`ArtifactUnavailable` identifies a retryable source failure separately from rejected save bytes.
Keep the existing library and execution when retrying; do not replace a missing historic artifact
with the current publication.

```ts
const { loadAuthoring } = await import("@rezics/narrata/authoring");
const api = await loadAuthoring();
const maintenance = api.createMaintenance({ storage: library, storeId: libraryId });
try {
  await maintenance.registerPublication(publication);
  const checked = await maintenance.inspectSave(checkpointBytes); // also reads legacy JSON
  await maintenance.importSave(checkpointBytes, {
    execution: newExecution, ...checked, expected_revision: null, now: trustedUnixSeconds,
  });
  const shallow = await maintenance.exportSave({ execution: newExecution, ...checked, ancestors: 0 });
} finally { await maintenance.close(); }
```

`ancestors` is a minimum: shallow export retains the checked context needed for the exact current
page, including local replies and call arguments. Inadequate older bundles fail with
`history_truncated` before import moves a durable root. Full export/replay still refuse a genuinely
truncated path. Existing `Book.exportSave()` and `Book.restore()` retain their save transports.

`run({ kind: "roots", after: null, limit: 256 })` and `pins` return bounded pages; continue using the
last item's key or owner/object pair when `more` is true. Root namespace codes distinguish saves,
branches, bookmarks, active cursors and temporary roots. Deleting selected roots requires their
observed decimal revision and commit via `delete_roots`; a stale selection must be read again.
`integrity` is read-only. `collect` requires explicit `now`, `grace_seconds` and `dry_run`; its
`removed` amounts are candidates in dry-run and reclaimed amounts after a confirmed sweep.

Limits bound object reads, bytes, scan pages, work and cumulative cold-cache loads, including retries.
`load_bytes` counts complete encoded replies when received. One conforming reply may arrive before
the threshold rejects it; that reply never enters the maintenance cache/Wasm. Host transport limits
bound each reply (64 MiB of values, 128 MiB framed), independently of this acceptance threshold.
Collection rejects a sweep that cannot fit one atomic flush. Incomplete or invalid scans never
authorize collection. Quota rejection preserves durable progress: select roots
to delete, inspect a dry-run, collect explicitly, then retry the import. Collection may free nothing.
In Node/Bun, `openLibraryMaintenance(authorizedPostgresStore)` maps definite quota rejection to
`WriteRejected("quota")`; `PostgresStore.usage()` returns consistent usage and configured quotas.
Unknown writes pause the handle; `retryPending()` resends the original bytes, after which the host
re-reads roots to establish the outcome. Pass the existing `AuthenticatedLibrary` as `storage` to
inherit account closure and rejection of late replies. With a plain authorized transport, close
the old maintenance handle on account switch or library removal.

## Authenticated online journeys

Use `createOnlineSession` for server-backed saves. Supply a new `AuthenticatedLibrary` for each
authenticated library instance, with its host-issued 16-byte StoreId, an authenticated `CacheStore`,
and release, deletion and content-cache invalidation callbacks. The transport stays bound to that
instance; account switching closes the old session and creates a new wrapper. Deletion receives
the checked, observed StoreId/revision for the host's CAS. A replacement library needs a new ID.

```ts
import { AuthenticatedLibrary, createOnlineSession, createReader } from "@rezics/narrata";

const library = new AuthenticatedLibrary({
  store: authenticatedTransport,
  storeId: authenticatedLibraryId,
  release: () => authenticatedTransport.close(),
  remove: observed => host.deleteLibrary(observed),
  clearContent: () => hostContent.clear(),
});
const session = createOnlineSession({
  library, execution,
  currentArtifact: () => host.currentPublishedArtifact(),
  loadArtifact: artifact => host.readerObjects(artifact), // manifest, optional names, fetchChunk
});
await session.start();
const reader = createReader(session, hostContent, { online: session, context: { viewer: account } });
await reader.start();
// On logout, invalidate synchronously, then await queued work and handle release.
await session.close();
```

`OnlineSession` uses the same `loadArtifact` callback for the exact programs named by historical
commits. For edition-dependent content, set `ReaderOptions.contextForArtifact(artifact, context)`
to select the host realization for that artifact. The built-in runtime supplies checked commit
provenance; older worker proxies can retain their existing interface unless using this option.
Content slots then receive each presentation or timeline entry's context, including after checkout.

To offer a story update, load the authoring evaluator on demand (or use the host's server boundary),
register the trusted publication path, and connect it to the same library and execution. Call
`session.assessUpdate(evolution, options)` and display its assessment/report. `continueOld()`
dismisses the review. For `automatic`, an explicit apply action calls `applyUpdate()`;
`needs_confirmation` requires a separate confirmation action calling `applyUpdate(true)`.
Changing the target or comparison budget requires another assessment. A reading choice or checkout
also invalidates the review. Core reassessment and cursor CAS remain authoritative.

Application reopens and inspects the durable artifact before resolving as migrated. Refresh the
reader controller after success. An unknown result keeps the previous confirmed screen; use the
session's `retrySave()` and then refresh, or let the reader's retry control do both. Definite
rejections/conflicts permit reload and reassessment. Keep the evaluator alive through unknown
outcomes, close it when its session closes, and discard old-account content and review UI on account
switch. `exact`, `old_version_only`, `needs_objects` and `invalid_save` do not apply a migration;
offer the assessment's available continuation/retry/restart actions without changing the old root.
Keep host review/apply actions disabled while the Reader controller resolves a screen. Disable
Reader mutations while a migration saves or switches programs. After a confirmed migration, call
`reader.refresh()` and report the mounted edition as ready only if it returns `true`. When it
returns `false`, leave choices unavailable, show the saved outcome, and let the reader retry the
saved screen before continuing.

Only an absent active cursor consults the current publication. Existing executions locate their
exact artifact through the checked read-only `locateSave` helper, then verify the loaded manifest's
identity before fetching its chunks. Each journey keeps that artifact's chunk loader even if the
work publishes a newer version.

The snapshot distinguishes `saved`, `saving`, `unknown`, `conflict`, `quota`, `unavailable` and
`closed`. `progress` remains the last confirmed view on failure. An unknown write retains its
candidate commit; `retrySave()` retries the original bytes without replaying a choice. A conflict
on that retry uses bounded native history traversal to prove the candidate is retained by this
execution's durable roots, including when another device has continued beyond it. Missing objects,
read errors, exhausted budgets and orphan candidates leave the outcome unknown. Resolve it before
reloading. Definite conflicts and rejections permit `reload()`.

Transport errors are unknown unless the host can guarantee the entire write had no effect; only
then map quota or explicit unavailability to `WriteRejected`. A later rejection cannot decide an
earlier unknown attempt. The server reference host remains the storage transaction authority;
this coordinator adds no HTTP protocol, database schema or login/offline synchronization.

The accepted cold reading reference is the example's **`/?reference`** route. Its thin Node host
creates the separate Signal Lantern work through the public server API, edits the call parameter
from 3 to 7, previews the journey and publishes A. A fresh reader realm fetches only the exact
manifest, necessary entry chunk and reader Wasm. Compilation, whole-publication enumeration and
immutable-object serving stay in the Node host. The browser never initializes the compiler or
assembles a pack for persistence.

This one work contains a local reply/rejoin, a later consequence of that choice, a parameterized
keeper subgraph and both endings. The same rendered journey exercises A/B/C updates, historical
checkout and choices, refresh and online failures. Its body blocks and distinct labels come from the example's content host, outside the
program and save. `online-cold.spec.ts` and the installed-tarball suite run the same route and check
its actual response identities and lengths, including unavailable chunks and recovery. Screenshots
and request/byte evidence accompany those checks.

The reference's “Publication and recovery scenarios” controls publish divergent, recovery and
incompatible editions through the same authoring API. Import controls use checked maintenance
export/import into a cold library and select it only after acknowledgement; the original library
remains stored. A readable shallow page still has incomplete map evidence: the host exports its
checked shallow checkpoint for the server map, which retains missing-ancestor markers and does
not invent visits or a frontier. Complete saves continue to use `exportConfirmedSave()`.

The cold route intentionally omits optional names on its manifest-plus-entry first screen.
Independent tarball probes in Node, Bun and Chromium separately supply each artifact's names,
load historical manifests/chunks on demand, compare timeline keys through checkout and choices,
and restore both native and retained legacy A roots from an initially B/C-only reader.

`/` remains the legacy pack demo with custom-pack import compatibility. `/?online` remains the
minimal in-browser authoring sample; neither is the evidence for cold M1 loading. These reference
hosts use IndexedDB to isolate client behavior and do not implement server authentication.

`@rezics/narrata/reader` exports the same reader surface as the root entry. Both load only the
reader Wasm, without compiler, publication, layout or trace code. The raw
`dist/wasm/narrata_nodes_wasm.js` module keeps its default build and `publishGraph` for existing
callers; no package entry imports it.

Wasm initializes once per JS realm. The default `new URL(..., import.meta.url)` asset works in
Vite production builds and the package's Node/Bun file loader. To supply an asset yourself,
pass `wasm` to the first `openBook` call or call `initialize(urlOrBytes)` first. `LocalContent`
also needs initialization before construction.

Optional `names` loads the artifact's checked alias table. Full-pack imports use
`openBook({ pack, execution, storage })`; that path retains `pack()` and R1 migration for the
reference reader. Manifest hosts retain their own artifact files for export. `checkout`,
`restore`, and `exportSave` use the existing checked kernel save formats.

## Reader controller and host rendering

Create one controller per open book. The host implements `ContentResolver`, including checking
its provider's results and caching content if desired. Start the controller before using its
snapshot, or let the React binding start it on mount.

```ts
import { createReader, type ContentResolver } from "@rezics/narrata";

const resolver: ContentResolver = {
  resolve: request => hostContent.resolve(request), // Host implementation; results in item order.
};
const reader = createReader(book, resolver, {
  context: { languages: ["en", "zh-Hans"], realization: "selected-edition" },
  persistence: "durable", // Use "memory" when openBook has no storage host.
  maxItems: 4096,
});
await reader.start();
const unsubscribe = reader.subscribe(() => render(reader.getSnapshot()));
// During host cleanup:
unsubscribe();
reader.dispose();
// The host then closes book and its storage connection.
```

The controller collects screen references with `collectScreenContent` and resolves bounded
batches in the selected contexts. A snapshot is stable until an update and its nested values are frozen.
`setContext` resolves the screen in a new context; it refuses changes while another action is
pending. `toggle`, `selection`, and `choose` enforce enabled options and selection counts;
submissions use author order. `checkout` accepts only commits in the revealed history.

Actions resolve to `true` on success and `false` on failure or when an action is unavailable.
Failures are represented in the snapshot. A failed write or another tab's update keeps the
previous screen visible and prevents further choices until `reload` discards unconfirmed changes
and opens the saved cursor. `retry` reloads when needed, otherwise reinspects and resolves the
screen. After a host's import or restore, call `refresh`. Content-fetch failures do not undo
confirmed writes. Reader resolutions expose `ok`, `unavailable`, or `incompatible`; provider
diagnostic reasons are removed before reaching slots.

After publishing each screen, the controller calls `nextContentUnits` and resolves whole body
units in the same context. This speculative work starts after the first screen, may fetch later
program chunks, and never publishes an error or changes progress. A host resolver can use it
to warm its cache; future option labels and arguments are not prefetched.

```tsx
import { Reader, type ContentSlotProps, type ReaderSlots } from "@rezics/narrata/react";

function hostContentSlot(props: ContentSlotProps) {
  // DocumentBody, a game renderer, or another host-owned content component.
  return <HostContent content={props.content} resolution={props.resolution}
    context={props.context} presentation={props.presentation} />;
}
const slots: ReaderSlots = {
  body: hostContentSlot,
  option: hostContentSlot,
  reference: hostContentSlot,
};
<Reader controller={reader} slots={slots} />;
```

The body slot receives body and reply segments; the option slot receives labels; the reference
slot receives titles, disabled reasons, path titles and reference-valued state. Each receives
the original item (including arguments), its resolution and selected content context. Presented items
also carry `(execution, commit, occurrence)` and node identity, so a host can use the stable
presentation key. Keep option and short-reference output suitable for a button or inline label;
Narrata owns the interactive controls.

The component has no stylesheet. Override its `classes` and `messages` for host styling and
localization. Native buttons and checkboxes provide keyboard controls; disabled reasons are
associated descriptions, the current path entry has `aria-current="step"`, and the new heading
receives focus after checkout or a choice. `showPath`, `showState`, and `showNavigation` allow
hosts to place those parts elsewhere with `ReaderPath`, `ReaderNavigation`, `ReaderSaveStatus`
and `useReader`. Set `disabled` while a host-owned import or export is pending. Rendering on a
server requires the same prepared controller snapshot during
hydration. Disposing the controller remains the host's responsibility.

## Authoring and publication

`@rezics/narrata/authoring` (browser) and `@rezics/narrata/server` (Node/Bun) expose one API over a
separate authoring Wasm. Import the browser entry dynamically, preferably in an editing or
publishing Worker; the server entry reads the same Wasm bytes from the installed package.
`loadAuthoring(urlOrBytes?)` initializes it once per realm and resolves to the API.

```ts
const { loadAuthoring } = await import("@rezics/narrata/authoring");
const authoring = await loadAuthoring();

// New nodes, choice points and options get IDs from an explicit mint; existing IDs never change.
const minter = authoring.createMinter(); // Date.now and Web Crypto; tests may inject Entropy.
minter.occupy(liveAndDeletedIds);         // In batches: every live ID and every tombstone.
const { choice_points, options } = minter.mint({ choicePoints: 1, options: 2 });

const drafts = authoring.createDrafts();
for (const record of storedRecordBytes) drafts.add(record);  // One record at a time.
for (const batch of outlineBatches) drafts.addOutline(batch); // Units of one provider assemble.
const check = drafts.check();      // Located diagnostics; canonical bytes and index rows per record.
const preview = drafts.preview();  // Program objects for openBook({ manifest, names, fetchChunk }).

const previous = await authoring.readPublication(currentDescription, currentPublicationId,
  reference => hostObjects.get(reference.object_id));
const { report, records, publication } = drafts.publish({ previous });
if (report.status === "switchable" && publication) {
  for (const [reference, bytes] of publication.objects()) await host.putImmutable(reference, bytes);
  await host.persistRecords(records);  // Includes appended tombstones, with their index rows.
  await host.switchPublication(publication.publicationId, publication.artifactId);
}
publication?.free(); previous.free(); preview.free(); drafts.free(); minter.free();
```

Diagnostics locate `records[<insertion index>]`, the record key, JSON pointer and node, choice
point and option IDs. `check`, `preview` and `publish` report diagnostics instead of throwing; API
misuse and corrupt objects throw `AuthoringError` with `code` and `path`. A `compiled` result is
inspectable but fails the outline policy; only `switchable` candidates may become current.
Pass `lock` for locked composition: it fails rather than append tombstones. Every publication
after a work's first passes the current one as `previous`, restored from its description and
individual objects (`openPublication` supplies them one at a time; `publicationFromPack` reads a
legacy pack of at most 64 MiB). Handles hold Wasm memory until `free()`. Reader chunk IDs are
`object:` plus the publication's hex object ID; `object()` accepts either form. Existing saves
stay bound to their artifact: reopen an old journey on its own publication. `splitSource` and
`minter.fill` convert R2 project files, given as the manifest and one package per alias.

## Reader maps and relation hosting

For a revealed map, initialize the authoring/server module, register the exact trusted
publications on `api.createEvolution()`, restore the checkpoint (or connect an authorized
store), and call `visits()`. `visits.readerMap(publication)` returns only revealed references,
executed connections and visible option placeholders. Resolve content through the host;
`RevealedReaderMap` in the optional `@rezics/narrata/reader-map/react` entry renders this data
without program objects or author-graph imports. The framework-free `@rezics/narrata/reader-map`
entry provides the transport decoder; both entries load only when the host requests a map.
Free visits and relation handles, then close the evolution context. The `/?reference` example
requests the map from its Node host using the same confirmed checkpoint and artifact.

`Publication.relationRows`, `relationPart` and `queryRelations` provide bounded Rust-backed
author access. `visits.readerRelations(publication)` creates a separate sparse reader allowlist;
it preserves original edge numbers. `PostgresRelations` from the server entry accepts an
authorized `TransactionalSql`, loads the complete projection atomically, and validates every
row on retries and query readback. Install the repository's authoritative `relations-v1.sql`
before using it; the adapter installs no schema or migrations. `load(publication)`,
`query(publication, query)` and `readReader(allowlist)` have separate host authorization needs.
The server entry exports `referenceRelationsSql`, a package-relative URL to the exact DDL asset;
read it with the host's filesystem API and install it explicitly before loading projections.
Structural first/route queries do not prove first reading or an executable narrative route.
Client checkpoints and visits support personal spoiler protection, never entitlement or rewards.
Incomplete history retains missing-object metadata; keep old publications and migration evidence
to recover complete maps. Reader imports and their first screen never initialize authoring/trace.
For an HTTP map response, use `@rezics/narrata/reader-map`'s `readReaderMapResponse(response, identity)`
with the artifact and cursor from `OnlineSession.exportConfirmedSave()`. It validates the generated
schema, supported versions, single-source identity and transport limits before rendering. Discard
responses after the reader's binding or confirmed cursor changes. The reference endpoint retains
its 256 KiB checkpoint request cap; it is a host for the finite reference story, not a general
64 MiB checkpoint transport. The optional map renderer keeps readable SVG dimensions inside a
scrollable viewport, so long revealed journeys remain accessible on narrow screens.

## Author graph

Open this full-structure view only for an audience allowed to see unrevealed routes. It uses
published ADR 0016 objects and precomputed coordinates. Hosts supply object fetching and their
existing content resolver; geometry contains IDs and references, never body text.

```ts
// Run inside the host's “open author graph” action to keep it off the reading first screen.
const { mountAuthorGraph } = await import("@rezics/narrata/graph");
const mounted = mountAuthorGraph(container, {
  indexId: publication.index_object_id,
  labels: publication.labels.map(row => ({
    cluster: row.cluster.slice("cluster:".length), objectId: row.object_id,
  })),
  fetchObject: id => bytes(`/story.graph/${id}.cbor`),
  resolver,
  context: { languages: ["en"], realization: "selected-edition" },
});
await mounted.ready;
// Host controls can use mounted.controller.back(), showWork(), enterCluster(index),
// enterPartition(clusterIndex, partitionIndex), subscribe(), and getSnapshot().
// During cleanup:
mounted.dispose();
```

`AuthorGraph` from `@rezics/narrata/graph/react` accepts the same options, a container style/class,
and `onController` / `onError` callbacks. Give it a nonzero height and keep the options object
stable. Clicking clusters and partition representatives enters their next level; `onSelect`
receives a node at the deepest level. Use the controller snapshot for keyboard-accessible host
navigation and lists alongside the canvas. `createGraphController` also works without a renderer.

Defaults cap a screen at 50,000 unique nodes and 150,000 unique relations. `maxNodes` / `maxEdges`
override these limits. An over-budget transition keeps the previous screen and sets `limited`;
an over-budget overview has no drawable screen. `maxCachedObjects` (64) and `maxCacheBytes`
(32 MiB) bound an LRU cache of fetched bytes. Disposing releases the cache and WebGL resources.
Visible node titles resolve in batches of `maxResolveItems` (256) in the supplied context;
unavailable titles and nodes without titles show their IDs. Cluster IDs have no inferred titles.
Coordinates outside JavaScript's exact integer range report an error instead of rounding.

`decodeIndex(bytes, objectId)`, `decodeTile(bytes, checkedIndex, tileIndex)` and
`decodeLabels(bytes, objectId, clusterId)` are asynchronous checked readers. Object IDs are
64 lowercase hex digits, cluster IDs are 32 digits, both without prefixes. The readers check
envelopes, payload checksums, object identity, canonical CBOR and the format's structural budgets;
`GraphDecodeError` gives the failed check. An accepted index does not prove referenced objects
exist or match the original program's semantics. Hosts choose the publication for the correct
artifact; the reference reader checks this before displaying the demo's graph.

`task check:web` verifies declarations, controller behavior, slot contracts, the fixed
[size budget](size-budget.json), and external tarball installation. The production Vite install
test uses the React entry, checks first-screen manifest/entry-chunk requests before speculative
prefetch and that only the reader Wasm loads, retries a failed content load, makes a keyboard
choice, and reopens its durable save. Node, Bun and a browser Worker author, preview and publish
a short story twice through the installed entries. The tarball checks also decode
frozen graph objects without React installed. Reader, compatibility and authoring Wasm have separate
budgets; JavaScript budgets separate the runtime, authoring, React binding, testing fixtures, and
the graph build including Sigma/graphology. React itself belongs
to the host's bundle budget. The existing first-screen limits are retained. Limits
and measured baselines record the remaining headroom in `size-budget.json`.
