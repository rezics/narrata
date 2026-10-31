import { ArtifactUnavailable, type ArtifactLoader } from "./artifacts.js";
import { StorageHost, type CacheStore, type CacheHandle } from "./storage.js";

/** The shared Wasm/cache seam used by the Gamebook and VN readers. */
export interface ReaderEngine extends CacheHandle {
  readonly artifact_id: string;
  open(): void;
  resume(): void;
  memory(): void;
  confirm_memory(): void;
  reload(): void;
  free(): void;
  takeProgramRequest(): string | undefined;
  registerHistoryManifest(expected: string, manifest: Uint8Array, names?: Uint8Array): void;
  loadArtifactChunk(artifact: string, id: string, bytes: Uint8Array): void;
}

/** Serializes logical chunk retry, storage confirmation and Wasm lifetime for either reader. */
export class RuntimeTransport {
  private queue: Promise<unknown> = Promise.resolve();
  private readonly host: StorageHost | undefined;
  private closed = false;
  private closing: Promise<void> | undefined;
  private readonly loaders = new Map<string, (id: string) => Promise<Uint8Array>>();
  private readonly closedMessage: string;

  constructor(
    private readonly raw: ReaderEngine,
    storage?: CacheStore,
    fetchChunk?: (id: string) => Promise<Uint8Array>,
    private readonly loadArtifact?: ArtifactLoader,
    closedMessage = "The book is closed.",
  ) {
    this.host = storage ? new StorageHost(storage, raw, 100_000) : undefined;
    if (fetchChunk) this.loaders.set(raw.artifact_id, fetchChunk);
    this.closedMessage = closedMessage;
  }

  get durable(): boolean { return this.host !== undefined; }

  assertOpen(): void {
    if (this.closed) throw new Error(this.closedMessage);
  }

  run<T>(operation: () => T, confirmed?: (value: T) => void, prepare?: () => void | Promise<void>): Promise<T> {
    if (this.closed) return Promise.reject(new Error(this.closedMessage));
    const result = this.queue.then(async () => {
      this.assertOpen();
      await prepare?.();
      this.assertOpen();
      for (let attempt = 0; attempt < 100_000; attempt++) {
        try {
          let value: T;
          if (this.host) {
            value = await this.host.run(operation);
          } else {
            value = operation();
            this.raw.confirm_memory();
          }
          this.assertOpen();
          confirmed?.(value);
          return value;
        } catch (error) {
          this.assertOpen();
          const missing = error instanceof Error ? /^needs_artifact at (artifact:[0-9a-f]{64}):/.exec(error.message)?.[1] : undefined;
          if (missing && this.loadArtifact) {
            if (this.loaders.size >= 65) throw new RangeError("At most 64 historical artifacts");
            const source = await this.loadArtifact(missing).catch(cause => { throw new ArtifactUnavailable(missing, undefined, { cause }); });
            this.assertOpen();
            this.raw.registerHistoryManifest(missing, source.manifest, source.names);
            this.loaders.set(missing, source.fetchChunk.bind(source));
            continue;
          }
          const request = this.raw.takeProgramRequest();
          if (request === undefined) throw error;
          const value: unknown = JSON.parse(request);
          if (!Array.isArray(value) || value.length !== 2 || typeof value[0] !== "string" || !/^artifact:[0-9a-f]{64}$/.test(value[0])
            || typeof value[1] !== "string" || !/^object:[0-9a-f]{64}$/.test(value[1])) throw new Error("Invalid program request");
          const [artifact, id] = value;
          const loader = this.loaders.get(artifact);
          if (!loader) throw error;
          const bytes = await loader(id).catch(cause => { throw new ArtifactUnavailable(artifact, id, { cause }); });
          this.assertOpen();
          this.raw.loadArtifactChunk(artifact, id, bytes);
        }
      }
      throw new Error("The operation did not settle after 100000 program loads.");
    });
    this.queue = result.catch(() => undefined);
    return result;
  }

  reload(reopen: () => void, confirmed?: () => void): Promise<void> {
    return this.run(reopen, confirmed, async () => {
      this.raw.reload();
      await this.host?.discardPendingAfterReload();
    });
  }

  retrySave(): Promise<boolean> {
    if (this.closed) return Promise.reject(new Error(this.closedMessage));
    const result = this.queue.then(async () => {
      this.assertOpen();
      const retried = this.host ? await this.host.retryPending() : false;
      this.assertOpen();
      return retried;
    });
    this.queue = result.catch(() => undefined);
    return result;
  }

  /** Rejects new operations immediately; a pending load can settle before Wasm is freed. */
  close(): Promise<void> {
    if (this.closing) return this.closing;
    this.closed = true;
    const result = this.queue.then(() => { this.raw.free(); });
    this.queue = result.catch(() => undefined);
    this.closing = result;
    return result;
  }
}
