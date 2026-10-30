import { Maintenance as RawMaintenance } from "../authoring-wasm/narrata_authoring_wasm.js";
import { AuthenticatedLibrary, StorageHost, StoreSuperseded, WriteRejected, equalBytes, decodeLoaded, decodeFlush, type CacheStore } from "../storage.js";
import { AuthoringError, type Publication } from "../authoring/core.js";
import type { CheckedSave, ImportSelection, MaintenanceLimits, MaintenanceRequest, MaintenanceResult } from "../generated/authoring.js";

export type { CheckedSave, ImportSelection, MaintenanceLimits, MaintenanceRequest, MaintenanceResult, ObservedRoot, RootKey, PinCursor } from "../generated/authoring.js";
export const defaultMaintenanceLimits: Readonly<MaintenanceLimits> = Object.freeze({ object_reads: 100_000, bytes: 64 * 1024 * 1024, pages: 10_000, work: 1_000_000, page_size: 256, loads: 10_000, load_bytes: 64 * 1024 * 1024 });

export interface MaintenanceOptions {
  /** Already authorized by the host. Bind one immutable library/account instance per handle. */
  readonly storage: CacheStore;
  readonly storeId: Uint8Array;
  readonly limits?: MaintenanceLimits;
}
export class MaintenanceLimit extends Error { override name = "MaintenanceLimit"; }
function checked<T>(operation: () => T): T {
  try { return operation(); } catch (error) {
    const match = error instanceof Error ? /^([a-z0-9_]+) at (.*?): ([\s\S]*)$/.exec(error.message) : null;
    if (match?.[1] !== undefined && match[2] !== undefined && match[3] !== undefined) throw new AuthoringError(match[1], match[2], match[3]);
    throw error;
  }
}
function limits(input: MaintenanceLimits): MaintenanceLimits {
  const maximum = { object_reads: 1_000_000, bytes: 256 * 1024 * 1024, pages: 100_000, work: 10_000_000, page_size: 1024, loads: 100_000, load_bytes: 256 * 1024 * 1024 };
  for (const key of Object.keys(maximum) as (keyof MaintenanceLimits)[]) {
    const value = input[key];
    if (!Number.isSafeInteger(value) || value < (key === "page_size" ? 1 : 0) || value > maximum[key]) throw new RangeError(`Invalid maintenance ${key} budget`);
  }
  return { ...input };
}
function safeTime(value: number): void { if (!Number.isSafeInteger(value) || value < 0) throw new RangeError("Expected safe Unix seconds"); }
function hex(bytes: Uint8Array): string {
  if (bytes.length > 32 * 1024 * 1024) throw new MaintenanceLimit("Checkpoint exceeds 32 MiB");
  const table = Array.from({length:256}, (_,i) => i.toString(16).padStart(2,"0"));
  // Keep peak allocations bounded without spreading multi-megabyte argument lists.
  const chunks: string[] = [];
  for (let offset = 0; offset < bytes.length; offset += 8192) chunks.push(Array.from(bytes.subarray(offset,offset+8192), b => table[b]).join(""));
  return chunks.join("");
}

/** Explicitly freed authoring/server capability. Every call starts cold, shares the existing
 * cache load/atomic flush protocol, and publishes its result only after durable confirmation. */
export class Maintenance {
  private readonly raw: RawMaintenance;
  private readonly host: StorageHost;
  private readonly budget: MaintenanceLimits;
  private queue: Promise<unknown> = Promise.resolve();
  private closed = false;
  private closing: Promise<void> | undefined;
  private uncertain = false;
  private attemptedWrite = false;
  private loads = 0;
  private readBytes = 0;
  private readonly library: AuthenticatedLibrary | undefined;
  private unsubscribe: (() => void) | undefined;

  /** @internal Initialize the authoring module via loadAuthoring before constructing. */
  constructor(options: MaintenanceOptions) {
    if (!(options.storeId instanceof Uint8Array) || options.storeId.length !== 16) throw new TypeError("Expected an immutable 16-byte library ID");
    this.budget = limits(options.limits ?? defaultMaintenanceLimits);
    this.library = options.storage instanceof AuthenticatedLibrary ? options.storage : undefined;
    this.library?.assertOpen();
    this.raw = new RawMaintenance();
    this.unsubscribe = this.library?.subscribeClosed(() => { void this.close(); });
    const id = Uint8Array.from(options.storeId);
    const load = options.storage.load.bind(options.storage);
    const persist = options.storage.persist.bind(options.storage);
    const store: CacheStore = {
      load: async request => {
        this.assertOpen();
        if (++this.loads > this.budget.loads || this.readBytes >= this.budget.load_bytes) throw new MaintenanceLimit("Maintenance load budget exhausted");
        const reply = await load(request);
        this.assertOpen();
        this.readBytes += reply.byteLength;
        if (this.readBytes > this.budget.load_bytes) throw new MaintenanceLimit("Maintenance reply bytes exhausted");
        if (!equalBytes(decodeLoaded(reply).store.id, id)) throw new WriteRejected("unavailable", "The library instance was replaced.");
        return reply;
      },
      persist: async flush => {
        this.assertOpen();
        if (!equalBytes(decodeFlush(flush).store, id)) throw new WriteRejected("unavailable", "The write belongs to another library.");
        this.attemptedWrite = true;
        const reply = await persist(flush);
        this.assertOpen();
        return reply;
      },
    };
    this.host = new StorageHost(store, this.raw, this.budget.loads + 1);
  }
  registerPublication(publication: Publication): Promise<void> {
    return this.enqueue(() => checked(() => this.raw.registerPublication(publication.handle)));
  }
  /** Validates page context without touching the library; also resolves remapped legacy IDs. */
  inspectSave(save: Uint8Array | string): Promise<CheckedSave> {
    const text = typeof save === "string" ? save : hex(save);
    return this.enqueue(() => JSON.parse(checked(() => this.raw.inspectSave(text))) as CheckedSave);
  }
  run(request: MaintenanceRequest): Promise<MaintenanceResult> {
    if (request.kind === "collect") { safeTime(request.now); safeTime(request.grace_seconds); }
    if (request.kind === "pins") safeTime(request.now);
    const text = JSON.stringify(request), budget = JSON.stringify(this.budget);
    return this.operation(() => JSON.parse(checked(() => this.raw.run(text,budget))) as MaintenanceResult);
  }
  exportSave(selection: { execution: string; artifact: string; cursor: string; ancestors?: number }): Promise<Uint8Array> {
    const {execution, artifact, cursor, ancestors} = selection;
    if (ancestors !== undefined && (!Number.isSafeInteger(ancestors) || ancestors < 0 || ancestors > 10_000)) throw new RangeError("Expected 0..=10000 ancestors");
    return this.operation(() => checked(() => this.raw.export(execution,artifact,cursor,ancestors,JSON.stringify(this.budget))));
  }
  /** Existing bundle bytes, hexadecimal checkpoint, or legacy SessionExport JSON. The exact
   * checked artifact/cursor must match. null expected_revision means create only if absent. */
  importSave(save: Uint8Array | string, selection: ImportSelection): Promise<string> {
    safeTime(selection.now);
    const text = typeof save === "string" ? save : hex(save), selected = JSON.stringify(selection);
    return this.operation(() => checked(() => this.raw.import(text, selected, JSON.stringify(this.budget))));
  }
  /** Retransmits the original unknown flush. Then re-read roots; confirmation does not rerun
   * the operation or restore its lost return value. Conflicts require a fresh explicit call. */
  retryPending(): Promise<boolean> {
    return this.enqueue(async () => {
      try { const retried = await this.host.retryPending(); this.assertOpen(); this.uncertain = false; return retried; }
      catch (error) { if (error instanceof StoreSuperseded || error instanceof WriteRejected) { await this.discard(); this.uncertain = false; } throw error; }
    });
  }
  /** After a definite quota rejection, discard the candidate, explicitly delete selected
   * roots/collect, and retry import. GC may reclaim nothing when all objects remain rooted. */
  private operation<T>(operation: () => T): Promise<T> {
    return this.enqueue(async () => {
      if (this.uncertain) throw new Error("A maintenance write has an unknown outcome; retryPending or close and verify durable roots.");
      this.raw.refresh();
      this.loads = 0; this.readBytes = 0; this.attemptedWrite = false;
      try { const result = await this.host.run(operation); this.assertOpen(); return result; }
      catch (error) {
        if (!this.closed) {
          if (this.attemptedWrite && !(error instanceof StoreSuperseded) && !(error instanceof WriteRejected)) this.uncertain = true;
          else await this.discard();
        }
        throw error;
      }
    });
  }
  private async discard(): Promise<void> { this.raw.refresh(); await this.host.discardPendingAfterReload(); }
  private assertOpen(): void { if (this.closed) throw new Error("The maintenance handle is closed."); this.library?.assertOpen(); }
  private enqueue<T>(action: () => T | Promise<T>): Promise<T> {
    if (this.closed) return Promise.reject(new Error("The maintenance handle is closed."));
    const result = this.queue.then(() => { this.assertOpen(); return action(); });
    this.queue = result.catch(() => undefined);
    return result;
  }
  close(): Promise<void> {
    if (this.closing) return this.closing;
    this.closed = true;
    this.unsubscribe?.(); this.unsubscribe = undefined;
    this.closing = this.queue.then(() => { this.raw.free(); });
    return this.closing;
  }
}
