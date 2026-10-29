import { equalBytes } from "./cbor";
import type { CacheStore } from "./host";
import { decodeFlush, decodeFlushReply, decodeLoaded, type StoreState } from "./protocol";

/** Only use when the authenticated host guarantees the entire write was rejected. */
export class WriteRejected extends Error {
  override name = "WriteRejected";
  constructor(readonly reason: "quota" | "unavailable", message: string = reason) { super(message); }
}
export class LibraryClosed extends Error {
  override name = "LibraryClosed";
  constructor() { super("The authenticated library is closed."); }
}
export interface AuthenticatedLibraryOptions {
  readonly store: CacheStore;
  /** Issued by the authenticated host. Recreating a deleted library must issue a new ID. */
  readonly storeId: Uint8Array;
  readonly release: () => void | Promise<void>;
  readonly remove: (observed: StoreState) => Promise<void>;
  /** Invalidate host content caches on account loss, close or deletion. */
  readonly clearContent: () => void;
}

/** One immutable authenticated library instance. A new account always needs a new instance. */
export class AuthenticatedLibrary implements CacheStore {
  private readonly id: Uint8Array;
  private readonly transport: CacheStore;
  private readonly release: AuthenticatedLibraryOptions["release"];
  private readonly removeObserved: AuthenticatedLibraryOptions["remove"];
  private readonly clearContent: () => void;
  private readonly inFlight = new Set<Promise<unknown>>();
  private readonly closeListeners = new Set<() => void>();
  private revision: number | undefined;
  private generation = 0;
  private closed = false;
  private closing: Promise<void> | undefined;

  constructor(options: AuthenticatedLibraryOptions) {
    if (!(options.storeId instanceof Uint8Array) || options.storeId.length !== 16) throw new TypeError("Expected a 16-byte authenticated store ID");
    this.id = Uint8Array.from(options.storeId);
    this.transport = { load: options.store.load.bind(options.store), persist: options.store.persist.bind(options.store) };
    this.release = options.release; this.removeObserved = options.remove; this.clearContent = options.clearContent;
  }
  get isClosed(): boolean { return this.closed; }
  subscribeClosed(listener: () => void): () => void {
    this.closeListeners.add(listener);
    return () => this.closeListeners.delete(listener);
  }
  get observed(): StoreState | undefined {
    return this.revision === undefined ? undefined : { id: this.id.slice(), revision: this.revision };
  }
  assertOpen(): void { if (this.closed) throw new LibraryClosed(); }
  private observe(state: StoreState): void {
    if (!equalBytes(state.id, this.id)) throw new WriteRejected("unavailable", "The authenticated library instance was replaced.");
    this.revision = Math.max(this.revision ?? 0, state.revision);
  }
  load(request: Uint8Array): Promise<Uint8Array> {
    return this.perform(async () => {
      const reply = await this.transport.load(request);
      this.assertOpen(); this.observe(decodeLoaded(reply).store);
      return reply;
    });
  }
  persist(flush: Uint8Array): Promise<Uint8Array> {
    return this.perform(async () => {
      if (!equalBytes(decodeFlush(flush).store, this.id)) throw new WriteRejected("unavailable", "The write belongs to a different library.");
      const reply = await this.transport.persist(flush);
      this.assertOpen();
      const result = decodeFlushReply(reply);
      this.observe("conflict" in result ? result.conflict : { id: this.id, revision: result.persisted });
      return reply;
    });
  }
  /** Reject at the call boundary; late replies cannot observe or publish the old account. */
  private perform<T>(action: () => Promise<T>): Promise<T> {
    try { this.assertOpen(); } catch (error) { return Promise.reject(error); }
    const generation = this.generation;
    const result = Promise.resolve().then(() => { this.assertOpen(); return action(); }).then(value => {
      if (this.closed || generation !== this.generation) throw new LibraryClosed();
      return value;
    }, error => { if (this.closed || generation !== this.generation) throw new LibraryClosed(); throw error; });
    this.inFlight.add(result);
    void result.finally(() => this.inFlight.delete(result)).catch(() => undefined);
    return result;
  }
  close(): Promise<void> {
    if (this.closing) return this.closing;
    this.closed = true; this.generation++;
    const pending = [...this.inFlight];
    const failures: unknown[] = [];
    // Establish the drain promise before callbacks, including callbacks that close a session.
    this.closing = Promise.resolve().then(async () => {
      await Promise.allSettled(pending);
      await this.release();
      if (failures.length) throw failures[0];
    });
    for (const listener of this.closeListeners) {
      try { listener(); } catch (error) { failures.push(error); }
    }
    this.closeListeners.clear();
    // Notify readers before a host callback disposes them; clear content in the same call.
    try { this.clearContent(); } catch (error) { failures.push(error); }
    return this.closing;
  }
  /** The host performs an ID/revision CAS, never a delete by a mutable account/work name. */
  remove(): Promise<void> {
    try { this.assertOpen(); } catch (error) { return Promise.reject(error); }
    const observed = this.observed;
    const close = this.close();
    if (!observed) return close.then(() => { throw new Error("Observe the library before deleting it."); });
    return close.then(() => this.removeObserved(observed));
  }
}
