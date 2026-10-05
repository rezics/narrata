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

  constructor(
    private readonly store: CacheStore,
    private readonly cache: CacheHandle,
    private readonly attempts = 64,
  ) {}

  /**
   * Runs `operation` until it no longer lacks data, persists what it wrote, and resolves with
   * its result. Operations run one at a time, so no load overlaps a flush.
   */
  run<T>(operation: () => T): Promise<T> {
    const result = this.queue.then(() => this.drive(operation));
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
        if (!this.cache.load(await this.store.load(request))) throw new StoreSuperseded();
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
    if (!this.cache.confirm(await this.store.persist(batches))) throw new StoreSuperseded();
  }
}
