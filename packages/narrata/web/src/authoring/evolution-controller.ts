import type { AssessmentV1, Evolution, EvolutionOptions, StoredEvolutionApply } from "./evolution.js";

/** Hosts render these states using their own copy and authorization policy. */
export type EvolutionControllerState =
  | { readonly kind: "idle" }
  | { readonly kind: "review"; readonly assessment: AssessmentV1 }
  | { readonly kind: "continued_old"; readonly assessment: AssessmentV1 }
  | { readonly kind: "applied_snapshot"; readonly result: Extract<StoredEvolutionApply, { kind: "migrated" }> }
  | { readonly kind: "upgraded"; readonly result: Extract<StoredEvolutionApply, { kind: "migrated" }> }
  | { readonly kind: "retry_required"; readonly result: Exclude<StoredEvolutionApply, { kind: "migrated" }> }
  | { readonly kind: "save_unknown"; readonly error: unknown }
  | { readonly kind: "failed"; readonly error: unknown };

/** A minimal framework-free host flow. It leaves Automatic as a plan until explicitly
 * applied, requires a separate confirmation call, and shows upgraded only after persistence. */
export class EvolutionController {
  #state: EvolutionControllerState = { kind: "idle" };
  #options: EvolutionOptions | undefined;
  #busy = false;
  constructor(readonly evolution: Evolution) {}
  get state(): EvolutionControllerState { return this.#state; }
  async assess(options: EvolutionOptions): Promise<AssessmentV1> {
    return this.#exclusive(async () => {
      this.#state = { kind: "idle" };
      this.#options = { target: options.target, migrationIds: [...options.migrationIds], ...(options.budget ? { budget: { ...options.budget } } : {}) };
      const assessment = await this.evolution.assess(this.#options);
      this.#state = { kind: "review", assessment };
      return assessment;
    });
  }
  continueOld(): void {
    if (this.#busy || this.#state.kind !== "review" || !["exact", "automatic", "needs_confirmation", "old_version_only"].includes(this.#state.assessment.kind)) throw new Error("No checked old save is available.");
    this.#state = { kind: "continued_old", assessment: this.#state.assessment };
  }
  applyAutomatic(): Promise<StoredEvolutionApply> { return this.#apply(false); }
  confirm(): Promise<StoredEvolutionApply> { return this.#apply(true); }
  #apply(confirmed: boolean): Promise<StoredEvolutionApply> {
    return this.#exclusive(async () => {
      const state = this.#state;
      if (state.kind !== "review" || (state.assessment.kind !== "automatic" && !(confirmed && state.assessment.kind === "needs_confirmation"))) throw new Error("Assess and explicitly confirm this plan before applying.");
      try {
        const result = await this.evolution.apply(state.assessment.plan, this.#options?.budget);
        this.#settled(result); return result;
      } catch (error) { this.#state = { kind: this.evolution.pendingSave ? "save_unknown" : "failed", error }; throw error; }
    });
  }
  async retrySave(): Promise<StoredEvolutionApply | undefined> {
    return this.#exclusive(async () => {
      if (this.#state.kind !== "save_unknown") throw new Error("There is no unknown write to retry.");
      const result = await this.evolution.retrySave();
      if (result) this.#settled(result);
      return result;
    });
  }
  #settled(result: StoredEvolutionApply): void {
    this.#state = result.kind === "migrated" ? { kind: this.evolution.connected ? "upgraded" : "applied_snapshot", result } : { kind: "retry_required", result };
  }
  async #exclusive<T>(operation: () => Promise<T>): Promise<T> {
    if (this.#busy) throw new Error("An evolution operation is already running.");
    this.#busy = true;
    try { return await operation(); } finally { this.#busy = false; }
  }
}
