/**
 * Runs kernel operations over a storage cache the way ADR 0017 requires: when an operation
 * fails because the cache lacks something, load it and run the operation again; when it
 * succeeds, persist its batches and only then hand back its result.
 */

/** The cache as a Wasm module exposes it; messages are encoded as in `protocol.ts`. */
export interface CacheHandle {
  /** What failed calls lacked since the last request, if anything. */
  takeRequest(): Uint8Array | undefined;
  /** Adds a load answer; `false` when another writer moved the store under unconfirmed batches. */
  load(loaded: Uint8Array): boolean;
  /** The batches awaiting persistence, if any. */
  unconfirmed(): Uint8Array | undefined;
  /** Applies a flush reply; `false` when another writer came first. */
  confirm(reply: Uint8Array): boolean;
}

export interface CacheStore {
  load(request: Uint8Array): Promise<Uint8Array>;
  persist(flush: Uint8Array): Promise<Uint8Array>;
}

/**
 * Another tab or window wrote the store first. The cache dropped what it held and the batches
 * of the operation that failed; reload the session before acting again.
 */
export class StoreSuperseded extends Error {
  override name = "StoreSuperseded";

  constructor() {
    super("another writer changed the store; reload");
  }
}

export class StorageHost {
  private queue: Promise<unknown> = Promise.resolve();
  private persistence:
    | { kind: "ready" }
    | { kind: "pending"; flush: Uint8Array }
    | { kind: "superseded" } = { kind: "ready" };

  constructor(
    private readonly store: CacheStore,
    private readonly cache: CacheHandle,
    // Four values per transport load: a 10,000-op batch can require 2,500 absence
    // loads plus a successful invocation. Keep a finite allowance for generic callers.
    private readonly attempts = 4096,
  ) {
    if (!Number.isSafeInteger(attempts) || attempts < 1) throw new RangeError("Expected a positive finite attempt limit");
  }

  /**
   * Runs `operation` until it no longer lacks data, persists what it wrote, and resolves with
   * its result. Operations run one at a time, so no load overlaps a flush.
   */
  run<T>(operation: () => T): Promise<T> {
    return this.enqueue(() => {
      if (this.persistence.kind === "superseded") throw new StoreSuperseded();
      if (this.persistence.kind === "pending") {
        throw new Error("a write has an unknown outcome; retryPending or reload the cache before another operation");
      }
      return this.drive(operation);
    });
  }

  /** Retries the original flush without rerunning its operation. A conflict requires reload and verification. */
  retryPending(): Promise<boolean> {
    return this.enqueue(async () => {
      if (this.persistence.kind === "superseded") throw new StoreSuperseded();
      if (this.persistence.kind === "ready") return false;
      await this.persistPending(this.persistence.flush);
      return true;
    });
  }

  /**
   * Deliberately abandons the old operation after the caller invalidates its session/cache.
   * This does not establish whether its write committed; the caller must verify that separately.
   */
  discardPendingAfterReload(): Promise<void> {
    return this.enqueue(() => {
      if (this.cache.unconfirmed() !== undefined) {
        throw new Error("invalidate the cache before discarding a pending write");
      }
      this.persistence = { kind: "ready" };
    });
  }

  private enqueue<T>(operation: () => T | Promise<T>): Promise<T> {
    const result = this.queue.then(operation);
    this.queue = result.catch(() => undefined);
    return result;
  }

  private async drive<T>(operation: () => T): Promise<T> {
    for (let attempt = 0; attempt < this.attempts; attempt++) {
      let value: T;
      try {
        value = operation();
      } catch (error) {
        const request = this.cache.takeRequest();
        if (request === undefined) throw error;
        if (!this.cache.load(await this.store.load(request))) {
          this.persistence = { kind: "superseded" };
          throw new StoreSuperseded();
        }
        continue;
      }
      // A miss the operation recovered from needs no load.
      this.cache.takeRequest();
      await this.flush();
      return value;
    }
    throw new Error(`the operation still lacked data after ${this.attempts} loads`);
  }

  private async flush(): Promise<void> {
    const batches = this.cache.unconfirmed();
    if (batches === undefined) return;
    // Neither the cache nor a transport retaining its argument may change retry bytes.
    const flush = Uint8Array.from(batches);
    this.persistence = { kind: "pending", flush };
    await this.persistPending(flush);
  }

  private async persistPending(flush: Uint8Array): Promise<void> {
    const reply = await this.store.persist(Uint8Array.from(flush));
    if (!this.cache.confirm(reply)) {
      this.persistence = { kind: "superseded" };
      throw new StoreSuperseded();
    }
    this.persistence = { kind: "ready" };
  }
}
