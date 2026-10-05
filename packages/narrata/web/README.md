# @rezics/narrata

The ESM package for hosts that run Narrata Gamebooks in a browser, Node or Bun. Content references
in `BookView` are resolved by the host; the runtime maintains structure, state and history.

Build and pack from the repository with `task web:pack`, then install the printed `.tgz` path
with npm. The tarball includes JavaScript, TypeScript declarations and a separate Wasm asset;
the framework-free entries need no runtime npm dependencies or repository paths. The optional
`@rezics/narrata/react` entry uses the host's React 18 or 19 installation; React is an optional
peer and is not shipped inside the package.

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
batches in one context. A snapshot is stable until an update and its nested values are frozen.
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
the original item (including arguments), its resolution and the screen context. Presented items
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

`task check:web` verifies declarations, controller behavior, slot contracts, the fixed
[size budget](size-budget.json), and external tarball installation. The production Vite install
test uses the React entry, checks first-screen manifest/entry-chunk requests before speculative
prefetch, makes a keyboard choice, and reopens its durable save. JavaScript is measured as three
exclusive groups: framework-free runtime (including storage and controller), React binding, and
testing fixtures, plus the aggregate. React itself belongs to the host's bundle budget. Limits
and measured baselines record the remaining headroom in `size-budget.json`.
