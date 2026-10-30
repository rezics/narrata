import { Evolution as RawEvolution, EvolutionPlan as RawPlan, planEvolution as rawPlan, readEvolution as rawRead } from "../authoring-wasm/narrata_authoring_wasm.js";
import { StorageHost, StoreSuperseded, type CacheStore } from "../storage.js";
import { AuthoringError, type Publication } from "./core.js";
import type { AssessmentV1, ApplyV1, ComparisonBudgetV1, EvolutionPlanningV1, PlanV1, RecoveryDeclarationsV1 } from "../generated/authoring.js";
import { Visits } from "./maps.js";

export type { AssessmentV1, ApplyV1, ComparisonBudgetV1, EvolutionDescriptorV1, EvolutionPlanningV1, MigrationId, MigrationReportV1, PathComparisonV1, PlanV1, RecoveryDeclarationsV1 } from "../generated/authoring.js";

function checked<T>(operation: () => T): T {
  try { return operation(); } catch (error) {
    const match = error instanceof Error ? /^([a-z0-9_]+) at (.*?): ([\s\S]*)$/.exec(error.message) : null;
    if (match?.[1] !== undefined && match[2] !== undefined && match[3] !== undefined) throw new AuthoringError(match[1], match[2], match[3]);
    throw error;
  }
}

/** A descriptor checked against complete source and target publications, or an Exact result. */
export class EvolutionPlan {
  #raw: RawPlan | undefined;
  readonly report: EvolutionPlanningV1;
  /** @internal */
  constructor(raw: RawPlan) { this.#raw = raw; this.report = JSON.parse(raw.report()) as EvolutionPlanningV1; }
  /** Canonical descriptor CBOR; Exact has no self edge. */
  bytes(): Uint8Array | undefined { return this.handle.bytes(); }
  /** @internal */
  get handle(): RawPlan { if (!this.#raw) throw new Error("The evolution plan was freed."); return this.#raw; }
  free(): void { this.#raw?.free(); this.#raw = undefined; }
}
export function planEvolution(previous: Publication, candidate: Publication, declarations?: RecoveryDeclarationsV1 | string): EvolutionPlan {
  return new EvolutionPlan(checked(() => rawPlan(previous.handle, candidate.handle, typeof declarations === "string" ? declarations : declarations && JSON.stringify(declarations))));
}
export function readEvolution(bytes: Uint8Array, migrationId: string, previous: Publication, candidate: Publication): EvolutionPlan {
  return new EvolutionPlan(checked(() => rawRead(bytes, migrationId, previous.handle, candidate.handle)));
}

export const defaultComparisonBudget: Readonly<ComparisonBudgetV1> = Object.freeze({ transitions: 10_000, automatic_steps: 100_000, object_bytes: 64 * 1024 * 1024 });
export interface EvolutionOptions {
  readonly target: string;
  /** The complete explicit path, in application order, including every intermediate publication. */
  readonly migrationIds: readonly string[];
  readonly budget?: ComparisonBudgetV1;
}
export type StoredEvolutionApply = ApplyV1 | { readonly kind: "storage_conflict" };

/**
 * An isolated assessment/application context. Snapshot operations touch only the supplied
 * save. Connected operations use the host's existing binary storage protocol and active
 * execution cursor; they never create a missing execution or write a save slot.
 */
export class Evolution {
  /** Replays the selected snapshot or authorized store cursor without writing. */
  visits(budget: ComparisonBudgetV1 = defaultComparisonBudget): Promise<Visits> {
    const text = JSON.stringify(budget);
    return this.#run(() => new Visits(checked(() => this.#handle.visits(text))));
  }
  #raw: RawEvolution | undefined;
  #host: StorageHost | undefined;
  #queue: Promise<unknown> = Promise.resolve();
  #pendingResult: ApplyV1 | undefined;
  /** @internal Hosts normally use Authoring.createEvolution(). */
  constructor(raw: RawEvolution = new RawEvolution()) { this.#raw = raw; }
  get connected(): boolean { return this.#host !== undefined; }
  get pendingSave(): boolean { return this.#host !== undefined && this.#pendingResult !== undefined; }
  get #handle(): RawEvolution { if (!this.#raw) throw new Error("The evolution context was freed."); return this.#raw; }
  /** Shares trusted metadata with an isolated context. Close forks before registering more publications. */
  fork(): Promise<Evolution> { return this.#enqueue(() => new Evolution(this.#handle.fork())); }
  /** Register programs required by an exact save or historical migration segments. */
  registerPublication(publication: Publication): Promise<void> { return this.#enqueue(() => checked(() => this.#handle.registerPublication(publication.handle))); }
  /** Register only a descriptor obtained from a trusted publication's evolution role. */
  registerEvolution(previous: Publication, candidate: Publication): Promise<string> { return this.#enqueue(() => checked(() => this.#handle.registerEvolution(previous.handle, candidate.handle))); }
  /** Select an isolated checkpoint/legacy JSON snapshot; application never persists it. */
  restoreSave(save: string): Promise<void> {
    return this.#enqueue(() => {
      if (this.#host) throw new Error("Create a separate context for snapshot assessment.");
      checked(() => this.#handle.restore(save));
    });
  }
  /** Select an existing authorized store/execution. Assessment does not create or write it. */
  connectStore(execution: string, storage: CacheStore): Promise<void> {
    return this.#enqueue(() => {
      if (this.#host) throw new Error("This context is already connected.");
      checked(() => this.#handle.connect(execution));
      this.#host = new StorageHost(storage, this.#handle, 100_000);
    });
  }
  assess(options: EvolutionOptions): Promise<AssessmentV1> {
    const target = options.target, ids = JSON.stringify(options.migrationIds), budget = JSON.stringify(options.budget ?? defaultComparisonBudget);
    return this.#run(() => JSON.parse(checked(() => this.#handle.assess(target, ids, budget))) as AssessmentV1);
  }
  /** A convenience for authorized impact jobs; failed input stays an explicit classification. */
  assessSave(save: string, options: EvolutionOptions): Promise<AssessmentV1> {
    const target = options.target, ids = JSON.stringify(options.migrationIds), budget = JSON.stringify(options.budget ?? defaultComparisonBudget);
    return this.#enqueue(() => {
      if (this.#host) throw new Error("Create a separate context for snapshot assessment.");
      return JSON.parse(checked(() => this.#handle.assessSave(save,target,ids,budget))) as AssessmentV1;
    });
  }
  /** Explicitly approves the exact plan. Changed cursor, target, declarations or budget invalidate it. */
  apply(plan: PlanV1, budget: ComparisonBudgetV1 = defaultComparisonBudget): Promise<StoredEvolutionApply> {
    const text = JSON.stringify(plan), limits = JSON.stringify(budget);
    return this.#run(() => {
      const result = JSON.parse(checked(() => this.#handle.apply(text, limits))) as ApplyV1;
      this.#pendingResult = result;
      return result;
    }).then(result => { this.#pendingResult = undefined; return result; }, error => {
      if (error instanceof StoreSuperseded) { this.#pendingResult = undefined; return { kind: "storage_conflict" }; }
      // Keep the original result while StorageHost retains unknown-outcome flush bytes.
      throw error;
    });
  }
  /** After a transport failure, retries only the original write. No migration is rerun. */
  retrySave(): Promise<StoredEvolutionApply | undefined> {
    return this.#enqueue(async () => {
      if (!this.#host || !this.#pendingResult) return undefined;
      try {
        await this.#host.retryPending();
        const result = this.#pendingResult; this.#pendingResult = undefined; return result;
      } catch (error) {
        if (error instanceof StoreSuperseded) { this.#pendingResult = undefined; return { kind: "storage_conflict" }; }
        throw error;
      }
    });
  }
  /** Explicit readback after an unknown write. Reassess before attempting another apply. */
  reload(): Promise<void> {
    return this.#enqueue(async () => {
      this.#handle.refresh();
      await this.#host?.discardPendingAfterReload();
      this.#pendingResult = undefined;
    });
  }
  exportSave(): Promise<string> { return this.#run(() => checked(() => this.#handle.export())); }
  /** Existing shallow checkpoint encoding, for a save whose older ancestors are absent. */
  exportShallow(ancestors = 1): Promise<string> {
    if (!Number.isSafeInteger(ancestors) || ancestors < 0 || ancestors > 10_000) return Promise.reject(new RangeError("Invalid shallow ancestor limit"));
    return this.#run(() => [...checked(() => this.#handle.exportShallow(ancestors))].map(byte => byte.toString(16).padStart(2,"0")).join(""));
  }
  /** Release after queued operations finish. The host retains ownership of its storage connection. */
  close(): Promise<void> { return this.#enqueue(() => { this.#raw?.free(); this.#raw = undefined; }); }
  #enqueue<T>(operation: () => T | Promise<T>): Promise<T> {
    const result = this.#queue.then(operation); this.#queue = result.catch(() => undefined); return result;
  }
  #run<T>(operation: () => T): Promise<T> {
    return this.#enqueue(async () => {
      if (!this.#host) return operation();
      // Refresh happens inside StorageHost's operation, after it rejects unknown writes.
      // Retry loads must reuse the same cache until this operation settles.
      let refreshed = false;
      return this.#host.run(() => { if (!refreshed) { this.#handle.refresh(); refreshed = true; } return operation(); });
    });
  }
}
