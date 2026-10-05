# @rezics/narrata

The ESM package for hosts that run Narrata Gamebooks in a browser, Node or Bun. Content references
in `BookView` are resolved by the host; the runtime maintains structure, state and history.

Build and pack from the repository with `task web:pack`, then install the printed `.tgz` path
with npm. The tarball includes JavaScript, TypeScript declarations and a separate Wasm asset;
it needs no runtime npm dependencies or repository paths.

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

`task check:web` verifies the package, its fixed [size budget](size-budget.json), and installation
into an external project. The reference reader's lower-level Wasm conformance tests also inspect
internal build output; applications use only the two public package entries above.
