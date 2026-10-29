import { AuthenticatedLibrary, LibraryClosed, StoreSuperseded, WriteRejected } from "../storage.js";
import { locateSave, prepareBook, retainedSave, type RuntimeBook, type WasmSource } from "../runtime.js";
import type { BookView, ChoicePointId, CommitId, ContentRef, ExecutionId, OptionId } from "../generated/book-view.js";

export { AuthenticatedLibrary, LibraryClosed, WriteRejected } from "../storage.js";
export type { AuthenticatedLibraryOptions } from "../storage.js";
export type OnlineStatus = "saved" | "saving" | "unknown" | "conflict" | "quota" | "unavailable" | "closed";
export interface OnlineSnapshot {
  readonly status: OnlineStatus;
  /** Last inspected, confirmed progress; failed writes never replace it. */
  readonly progress: BookView | null;
  readonly candidate: CommitId | null;
}
export interface ArtifactSource {
  readonly manifest: Uint8Array;
  readonly names?: Uint8Array;
  readonly fetchChunk: (id: string) => Promise<Uint8Array>;
}
export interface OnlineSessionOptions {
  readonly library: AuthenticatedLibrary;
  readonly execution: ExecutionId;
  /** Consulted only when the execution has no active cursor. */
  readonly currentArtifact: () => Promise<string>;
  readonly loadArtifact: (artifact: string) => Promise<ArtifactSource>;
  readonly wasm?: WasmSource;
  readonly verificationLimits?: { readonly roots: number; readonly commits: number };
}

function freeze<T>(value: T): T {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    for (const child of Object.values(value)) freeze(child);
    Object.freeze(value);
  }
  return value;
}

/** Lifecycle and persistence coordinator; content remains entirely in the host. */
export class OnlineSession {
  private readonly library: AuthenticatedLibrary;
  private readonly execution: ExecutionId;
  private readonly currentArtifact: OnlineSessionOptions["currentArtifact"];
  private readonly loadArtifact: OnlineSessionOptions["loadArtifact"];
  private readonly wasm: WasmSource | undefined;
  private readonly limits: { roots: number; commits: number };
  private book: RuntimeBook | undefined;
  private snapshot: OnlineSnapshot = Object.freeze({ status: "unavailable", progress: null, candidate: null });
  private readonly listeners = new Set<() => void>();
  private queue: Promise<unknown> = Promise.resolve();
  private generation = 0;
  private verificationRequired = false;
  private closing: Promise<void> | undefined;

  constructor(options: OnlineSessionOptions) {
    this.library = options.library; this.execution = options.execution;
    this.library.assertOpen();
    this.library.subscribeClosed(() => { if (this.snapshot.status !== "closed") void this.close().catch(() => undefined); });
    this.currentArtifact = options.currentArtifact; this.loadArtifact = options.loadArtifact; this.wasm = options.wasm;
    this.limits = { ...(options.verificationLimits ?? { roots: 4096, commits: 65_536 }) };
  }
  readonly getSnapshot = (): OnlineSnapshot => this.snapshot;
  readonly subscribe = (listener: () => void): (() => void) => { this.listeners.add(listener); return () => this.listeners.delete(listener); };
  private publish(status: OnlineStatus, progress = this.snapshot.progress, candidate = this.snapshot.candidate): void {
    if (this.snapshot.status === "closed") return;
    this.snapshot = freeze({ status, progress, candidate });
    for (const listener of this.listeners) listener();
  }
  private assertOpen(): void { if (this.snapshot.status === "closed") throw new LibraryClosed(); this.library.assertOpen(); }
  private async checked<T>(value: Promise<T>): Promise<T> {
    const generation = this.generation;
    const result = await value;
    this.assertOpen();
    if (generation !== this.generation) throw new LibraryClosed();
    return result;
  }
  private opened(): RuntimeBook { if (!this.book) throw new Error("Start the online session first."); return this.book; }
  private enqueue<T>(action: () => Promise<T>): Promise<T> {
    try { this.assertOpen(); } catch (error) { return Promise.reject(error); }
    const result = this.queue.then(() => { this.assertOpen(); return action(); });
    this.queue = result.catch(() => undefined);
    return result;
  }
  private fail(error: unknown): never {
    this.assertOpen();
    const candidate = this.book?.pendingCommit ?? this.snapshot.candidate;
    if (error instanceof WriteRejected) this.publish(error.reason, undefined, null);
    else if (error instanceof StoreSuperseded || (error instanceof Error && /^stale_input(?:\s|:)/.test(error.message))) this.publish("conflict", undefined, null);
    else this.publish(candidate ? "unknown" : "unavailable", undefined, candidate ?? null);
    throw error;
  }
  private async confirmed(): Promise<void> {
    const progress = await this.checked(this.opened().inspect());
    this.verificationRequired = false;
    this.publish("saved", structuredClone(progress), null);
  }
  private async open(existingOnly = false): Promise<void> {
    const location = await this.checked(locateSave(this.library, this.execution, this.wasm));
    if (existingOnly && !location) throw new Error("The saved execution no longer has an active cursor.");
    const artifact = location?.artifact ?? await this.checked(this.currentArtifact());
    if (typeof artifact !== "string" || !/^artifact:[0-9a-f]{64}$/.test(artifact)) throw new Error("Invalid published artifact ID");
    const source = await this.checked(this.loadArtifact(artifact));
    const fetchChunk = source.fetchChunk.bind(source);
    const book = await prepareBook({ manifest: Uint8Array.from(source.manifest), execution: this.execution, storage: this.library,
      ...(source.names ? { names: Uint8Array.from(source.names) } : {}),
      fetchChunk: id => { this.assertOpen(); return this.checked(fetchChunk(id)); }, ...(this.wasm === undefined ? {} : { wasm: this.wasm }) });
    try { this.assertOpen(); } catch (error) { await book.close(); throw error; }
    if (book.artifactId !== artifact) { await book.close(); throw new Error("Loaded manifest differs from the saved artifact."); }
    this.book = book;
    await this.checked(book.start(existingOnly || location !== null));
    await this.confirmed();
  }
  start(): Promise<void> {
    return this.enqueue(async () => {
      if (this.book) throw new Error("The online session was already started.");
      this.publish("saving", undefined, null);
      try { await this.open(); } catch (error) { this.fail(error); }
    });
  }
  inspect(): Promise<BookView> {
    return this.enqueue(async () => {
      if (this.snapshot.status !== "saved") throw new Error("Resolve the online save before reading or advancing.");
      return this.checked(this.opened().inspect());
    });
  }
  nextContentUnits(): Promise<ContentRef[]> {
    return this.enqueue(async () => {
      if (this.snapshot.status !== "saved") throw new Error("Resolve the online save first.");
      return this.checked(this.opened().nextContentUnits());
    });
  }
  private mutate<T>(action: (book: RuntimeBook) => Promise<T>): Promise<T> {
    return this.enqueue(async () => {
      if (this.snapshot.status !== "saved") throw new Error("Resolve the online save before advancing.");
      this.publish("saving");
      try { const value = await this.checked(action(this.opened())); await this.confirmed(); return value; }
      catch (error) { this.fail(error); }
    });
  }
  choose(expected: CommitId, point: ChoicePointId, options: readonly OptionId[]): Promise<CommitId> {
    return this.mutate(book => book.choose(expected, point, options));
  }
  checkout(commit: CommitId): Promise<void> { return this.mutate(book => book.checkout(commit)); }

  /** Unknown writes retry their bytes, never their choice. CAS conflict needs a native proof. */
  retrySave(): Promise<void> {
    return this.enqueue(async () => {
      if (this.snapshot.status !== "unknown") throw new Error("There is no unknown save to retry.");
      const candidate = this.snapshot.candidate;
      try {
        if (!this.verificationRequired) {
          try { await this.checked(this.opened().retrySave()); }
          catch (error) { if (!(error instanceof StoreSuperseded)) throw error; this.verificationRequired = true; }
        }
        if (this.verificationRequired) {
          if (!candidate || !await this.checked(retainedSave(this.library, this.execution, candidate, this.limits))) {
            throw new Error("The candidate could not be verified in this execution's durable history.");
          }
          // The durable cursor may now name a different historic artifact after evolution.
          // Locate it again instead of trying to reopen it on this book's fixed program.
          if (this.book) { await this.checked(this.book.close()); this.book = undefined; }
          await this.open(true);
          return;
        }
        await this.confirmed();
      } catch (error) {
        this.assertOpen();
        // Even a later explicit rejection cannot decide an earlier unknown attempt.
        this.publish("unknown", undefined, candidate); throw error;
      }
    });
  }
  /** Safe after a definite rejection or CAS conflict; unknown writes require verification. */
  reload(): Promise<void> {
    return this.enqueue(async () => {
      if (this.snapshot.status === "unknown") throw new Error("Retry and verify the unknown save before reloading.");
      this.publish("saving", undefined, null);
      try {
        if (this.book) { await this.checked(this.book.close()); this.book = undefined; }
        await this.open();
      } catch (error) { this.fail(error); }
    });
  }
  /** Invalidate synchronously, then drain and release; a new account creates a new wrapper. */
  close(): Promise<void> { return this.finish(false); }
  remove(): Promise<void> { return this.finish(true); }
  private finish(remove: boolean): Promise<void> {
    if (this.closing) return this.closing;
    this.generation++;
    this.publish("closed", null, null);
    const released = remove ? this.library.remove() : this.library.close();
    const drained = this.queue.then(() => this.book?.close());
    this.closing = Promise.allSettled([released, drained]).then(results => {
      this.listeners.clear();
      for (const result of results) if (result.status === "rejected") throw result.reason;
    });
    return this.closing;
  }
}

/** Construct synchronously when the host needs to invalidate while opening. */
export function createOnlineSession(options: OnlineSessionOptions): OnlineSession { return new OnlineSession(options); }
