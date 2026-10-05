import init, { NodeBook, LocalContent } from "./wasm/narrata_nodes_wasm.js";
import { StorageHost, type CacheStore } from "./storage.js";
import validate from "./generated/validate-book.js";
import type { BookView, ChoicePointId, CommitId, ExecutionId, OptionId } from "./generated/book-view.js";

export { LocalContent };
export type WasmSource = URL | string | Uint8Array | ArrayBuffer | WebAssembly.Module;

let initialized: Promise<unknown> | undefined;
/** Initialize once per JS realm. Omitted input locates the separately shipped Wasm asset. */
export function initialize(source?: WasmSource): Promise<void> {
  if (!initialized) {
    initialized = (async () => {
      let input = source ?? new URL("./wasm/narrata_nodes_wasm_bg.wasm", import.meta.url);
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

type SessionOptions = { execution: ExecutionId; storage?: CacheStore; wasm?: WasmSource };
export type OpenBookOptions = SessionOptions & (
  | { manifest: Uint8Array; fetchChunk: (id: string) => Promise<Uint8Array>; names?: Uint8Array; pack?: never }
  // Complete packs are retained for reader imports and checked R1/R2 save migrations.
  | { pack: Uint8Array; manifest?: never; fetchChunk?: never; names?: never }
);

/** Open a checked program and restore/create its session before resolving. Storage stays host-owned. */
export async function openBook(options: OpenBookOptions): Promise<Book> {
  await initialize(options.wasm);
  const raw = options.pack !== undefined
    ? new NodeBook(options.pack, options.execution)
    : NodeBook.fromManifest(options.manifest, options.execution);
  try {
    if (options.names) raw.loadNames(options.names);
    const book = new RuntimeBook(raw, options.storage, options.fetchChunk);
    await book.start();
    return book;
  } catch (error) { raw.free(); throw error; }
}

/** Async operations serialize program misses and save-cache retries, then confirm writes. */
export interface Book {
  readonly artifactId: string;
  inspect(): Promise<BookView>;
  choose(expected: CommitId, choicePoint: ChoicePointId, options: readonly OptionId[]): Promise<CommitId>;
  checkout(commit: CommitId): Promise<void>;
  exportSave(): Promise<string>;
  restore(save: string): Promise<void>;
  migrateR1(save: string, content: LocalContent, execution: ExecutionId): Promise<void>;
  /** Full-pack imports can be exported again; manifest hosts retain their own artifact bytes. */
  pack(): Uint8Array;
  /** Discards unconfirmed changes and reopens the durable session after a failed write. */
  reload(): Promise<void>;
  /** Closes after queued operations settle. The caller owns and closes the storage connection. */
  close(): Promise<void>;
}

class RuntimeBook implements Book {
  private queue: Promise<unknown> = Promise.resolve();
  private readonly host: StorageHost | undefined;
  private closed = false;

  constructor(
    private readonly raw: NodeBook,
    storage?: CacheStore,
    private readonly fetchChunk?: (id: string) => Promise<Uint8Array>,
  ) { this.host = storage ? new StorageHost(storage, raw, 100_000) : undefined; }

  get artifactId(): string { this.assertOpen(); return this.raw.artifact_id; }
  start(): Promise<void> { return this.run(() => this.host ? this.raw.open() : this.raw.memory()); }
  inspect(): Promise<BookView> { return this.run(() => decodeBook(this.raw.inspect())); }
  choose(expected: CommitId, choicePoint: ChoicePointId, options: readonly OptionId[]): Promise<CommitId> {
    return this.run(() => this.raw.choose(expected, choicePoint, [...options]));
  }
  checkout(commit: CommitId): Promise<void> { return this.run(() => this.raw.checkout(commit)); }
  exportSave(): Promise<string> { return this.run(() => this.raw.export()); }
  restore(save: string): Promise<void> { return this.run(() => this.raw.restore(save)); }
  migrateR1(save: string, content: LocalContent, execution: ExecutionId): Promise<void> {
    return this.run(() => this.raw.migrate_r1(save, content, execution));
  }
  /** Full-pack imports can be exported again; manifest hosts retain their own artifact bytes. */
  pack(): Uint8Array { this.assertOpen(); return this.raw.pack(); }
  reload(): Promise<void> {
    return this.run(() => this.host ? this.raw.open() : this.raw.memory(), () => this.raw.reload());
  }
  /** Closes after queued operations settle. The caller owns and closes the storage connection. */
  close(): Promise<void> {
    const result = this.queue.then(() => { if (!this.closed) { this.closed = true; this.raw.free(); } });
    this.queue = result.catch(() => undefined);
    return result;
  }
  private assertOpen(): void { if (this.closed) throw new Error("The book is closed."); }
  private run<T>(operation: () => T, prepare?: () => void): Promise<T> {
    const result = this.queue.then(async () => {
      this.assertOpen();
      prepare?.();
      // The storage host executes synchronous Wasm calls. A program miss escapes it without
      // confirming anything; supplying that checked chunk retries the same serialized call.
      for (let attempt = 0; attempt < 100_000; attempt++) {
        try {
          if (this.host) return await this.host.run(operation);
          const value = operation(); this.raw.confirm_memory(); return value;
        } catch (error) {
          const id = this.raw.takeChunkRequest();
          if (id === undefined || !this.fetchChunk) throw error;
          this.raw.loadChunk(id, await this.fetchChunk(id));
        }
      }
      throw new Error("The operation did not settle after 100000 program loads.");
    });
    this.queue = result.catch(() => undefined);
    return result;
  }
}
