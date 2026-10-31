import init, { NodeBook, LocalContent, SaveLocator } from "./reader-wasm/narrata_nodes_wasm.js";
import { StorageHost, type CacheStore } from "./storage.js";
import validate from "./generated/validate-book.js";
import type { BookView, ChoicePointId, CommitId, ExecutionId, OptionId } from "./generated/book-view.js";
import type { ContentRef } from "./generated/book-view.js";
import type { ArtifactLoader } from "./artifacts.js";
import { RuntimeTransport } from "./transport.js";

export { LocalContent };
export type WasmSource = URL | string | Uint8Array | ArrayBuffer | WebAssembly.Module;

let initialized: Promise<unknown> | undefined;
/** Initialize once per JS realm. Omitted input locates the separately shipped Wasm asset. */
export function initialize(source?: WasmSource): Promise<void> {
  if (!initialized) {
    initialized = (async () => {
      let input = source ?? new URL("./reader-wasm/narrata_nodes_wasm_bg.wasm", import.meta.url);
      const url = input instanceof URL ? input : typeof input === "string" ? new URL(input, import.meta.url) : undefined;
      if (url?.protocol === "file:") {
        // A variable specifier keeps browser bundlers from importing a Node-only module.
        const module = "node:fs/promises";
        const { readFile }: typeof import("node:fs/promises") = await import(/* @vite-ignore */ module);
        input = new Uint8Array(await readFile(url));
      }
      await init({ module_or_path: input });
    })().catch(error => { initialized = undefined; throw error; });
  }
  return initialized.then(() => undefined);
}

/** Both declarations and this standalone validator are generated from Rust's BookView schema. */
export function checkedBook(value: unknown): BookView {
  if (!validate(value)) throw new Error("The runtime returned an invalid BookView; rebuild the runtime.");
  return value;
}
export function decodeBook(text: string): BookView { return checkedBook(JSON.parse(text)); }

function isContentRef(value: unknown): value is ContentRef {
  return typeof value === "object" && value !== null && "provider" in value && "key" in value
    && typeof value.provider === "string" && typeof value.key === "string";
}

type SessionOptions = {
  execution: ExecutionId; storage?: CacheStore; wasm?: WasmSource;
  /** Exact trusted historical artifacts needed by migrated save pages; at most 64 / 64 MiB. */
  historyPacks?: readonly Uint8Array[];
  /** Resolves the exact artifact requested by history, without downloading a full pack. */
  loadArtifact?: ArtifactLoader;
};
export type OpenBookOptions = SessionOptions & (
  | { manifest: Uint8Array; fetchChunk: (id: string) => Promise<Uint8Array>; names?: Uint8Array; pack?: never }
  // Complete packs are retained for reader imports and checked R1/R2 save migrations.
  | { pack: Uint8Array; manifest?: never; fetchChunk?: never; names?: never }
);

/** Open a checked program and restore/create its session before resolving. Storage stays host-owned. */
export async function openBook(options: OpenBookOptions): Promise<Book> {
  const book = await prepareBook(options);
  try { await book.start(); return book; }
  catch (error) { await book.close(); throw error; }
}

/** Internal two-phase opening lets online hosts retain an unacknowledged initial root. */
export async function prepareBook(options: OpenBookOptions): Promise<RuntimeBook> {
  await initialize(options.wasm);
  const history = options.historyPacks ?? [];
  if (history.length > 64 || history.reduce((sum, bytes) => sum + bytes.byteLength, 0) > 64 * 1024 * 1024) throw new RangeError("Historical packs exceed 64 artifacts / 64 MiB");
  const raw = options.pack !== undefined
    ? new NodeBook(options.pack, options.execution)
    : NodeBook.fromManifest(options.manifest, options.execution);
  try {
    for (const pack of history) raw.registerHistoryPack(pack);
    if (options.names) raw.loadNames(options.names);
    const book = new RuntimeBook(raw, options.storage, options.fetchChunk, options.loadArtifact);
    return book;
  } catch (error) { raw.free(); throw error; }
}

/** Async operations serialize program misses and save-cache retries, then confirm writes. */
export interface Book {
  readonly artifactId: string;
  inspect(): Promise<BookView>;
  /** Optional for older host proxies; artifact-specific content contexts require it. */
  historyArtifacts?(expected: CommitId): Promise<Readonly<Record<CommitId, string>>>;
  /** Possible body units after one choice, through automatic transitions to the next interaction. */
  nextContentUnits(): Promise<ContentRef[]>;
  choose(expected: CommitId, choicePoint: ChoicePointId, options: readonly OptionId[]): Promise<CommitId>;
  checkout(commit: CommitId): Promise<void>;
  exportSave(): Promise<string>;
  restore(save: string): Promise<void>;
  migrateR1(save: string, content: LocalContent, execution: ExecutionId): Promise<void>;
  /** Full-pack imports can be exported again; manifest hosts retain their own artifact bytes. */
  pack(): Uint8Array;
  /** Discards unconfirmed changes and reopens the durable session after a failed write. */
  reload(): Promise<void>;
  /** Retry save bytes without applying the choice again. On conflict, reload and inspect durable progress. */
  retrySave(): Promise<boolean>;
  /** Rejects new calls immediately and frees Wasm after queued operations settle. */
  close(): Promise<void>;
}

export class RuntimeBook implements Book {
  private readonly transport: RuntimeTransport;
  private readonly durable: boolean;
  private candidate: CommitId | undefined;
  get pendingCommit(): CommitId | undefined { return this.candidate; }

  constructor(
    private readonly raw: NodeBook,
    storage?: CacheStore,
    fetchChunk?: (id: string) => Promise<Uint8Array>,
    private readonly loadArtifact?: ArtifactLoader,
  ) {
    this.durable = storage !== undefined;
    this.transport = new RuntimeTransport(raw, storage, fetchChunk, loadArtifact);
  }

  get artifactId(): string { this.transport.assertOpen(); return this.raw.artifact_id; }
  start(existingOnly = false): Promise<void> { return this.run(() => {
    if (this.durable) { if (existingOnly) this.raw.resume(); else this.raw.open(); } else this.raw.memory();
    this.candidate = this.raw.cursor();
  }); }
  inspect(): Promise<BookView> { return this.run(() => decodeBook(this.raw.inspect())); }
  historyArtifacts(expected: CommitId): Promise<Readonly<Record<CommitId, string>>> {
    return this.run(() => {
      const value: unknown = JSON.parse(this.raw.historyArtifacts(expected));
      if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("Invalid history artifact provenance");
      const artifacts: Record<CommitId, string> = {};
      for (const [id, artifact] of Object.entries(value)) {
        if (!/^commit:[0-9a-f]{64}$/.test(id) || typeof artifact !== "string" || !/^artifact:[0-9a-f]{64}$/.test(artifact)) throw new Error("Invalid history artifact provenance");
        artifacts[id] = artifact;
      }
      return artifacts;
    });
  }
  nextContentUnits(): Promise<ContentRef[]> {
    return this.run(() => {
      const references: unknown = JSON.parse(this.raw.nextContentUnits());
      if (!Array.isArray(references) || !references.every(isContentRef)) throw new Error("Invalid content lookahead from the runtime");
      return references;
    });
  }
  choose(expected: CommitId, choicePoint: ChoicePointId, options: readonly OptionId[]): Promise<CommitId> {
    return this.run(() => { const commit = this.raw.choose(expected, choicePoint, [...options]); this.candidate = commit; return commit; });
  }
  checkout(commit: CommitId): Promise<void> { return this.run(() => { this.raw.checkout(commit); this.candidate = this.raw.cursor(); }); }
  exportSave(): Promise<string> { return this.run(() => this.raw.export()); }
  restore(save: string): Promise<void> { return this.run(() => this.raw.restore(save)); }
  migrateR1(save: string, content: LocalContent, execution: ExecutionId): Promise<void> {
    return this.run(() => this.raw.migrate_r1(save, content, execution));
  }
  /** Full-pack imports can be exported again; manifest hosts retain their own artifact bytes. */
  pack(): Uint8Array { this.assertOpen(); return this.raw.pack(); }
  reload(): Promise<void> {
    return this.transport.reload(() => this.durable ? this.raw.open() : this.raw.memory(), () => { this.candidate = undefined; });
  }
  retrySave(): Promise<boolean> {
    return this.transport.retrySave().then(retried => { this.candidate = undefined; return retried; });
  }
  /** Reject new calls immediately, then free Wasm after queued operations settle. */
  close(): Promise<void> { return this.transport.close(); }
  private assertOpen(): void { this.transport.assertOpen(); }
  private run<T>(operation: () => T, prepare?: () => void | Promise<void>): Promise<T> {
    return this.transport.run(operation, () => { this.candidate = undefined; }, prepare);
  }
}

/** Read-only checked discovery: it cannot initialize a library or flush a write. */
export async function locateSave(storage: CacheStore, execution: ExecutionId, wasm?: WasmSource): Promise<{ cursor: CommitId; artifact: string } | null> {
  await initialize(wasm);
  const locator = new SaveLocator(execution);
  try {
    const text = await new StorageHost(storage, locator, 100_000).run(() => locator.locate());
    if (text === undefined) return null;
    const value: unknown = JSON.parse(text);
    if (!Array.isArray(value) || value.length !== 2 || typeof value[0] !== "string" || typeof value[1] !== "string"
      || !/^commit:[0-9a-f]{64}$/.test(value[0]) || !/^artifact:[0-9a-f]{64}$/.test(value[1])) throw new Error("Invalid save location");
    return { cursor: value[0], artifact: value[1] };
  } finally { locator.free(); }
}

export async function retainedSave(storage: CacheStore, execution: ExecutionId, candidate: CommitId, limits = { roots: 4096, commits: 65_536 }): Promise<boolean> {
  for (const count of [limits.roots, limits.commits]) if (!Number.isSafeInteger(count) || count < 1 || count > 1_000_000) throw new RangeError("Invalid retention budget");
  await initialize();
  const locator = new SaveLocator(execution);
  try { return await new StorageHost(storage, locator, 100_000).run(() => locator.retained(candidate, limits.roots, limits.commits)); }
  finally { locator.free(); }
}
