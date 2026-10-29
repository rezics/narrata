import { evolutionSampleKey } from "../authoring-wasm/narrata_authoring_wasm.js";
import { defaultComparisonBudget, type AssessmentV1, type ComparisonBudgetV1, type Evolution, type EvolutionOptions } from "./evolution.js";

/** Authorized lookup hints; only the loaded, checked commit proves their meaning. */
export interface AuthorizedSaveHead { readonly commit: string; readonly artifact: string }
/** Advisory allowances for the next host load. A callback can ignore them; received bytes
 * are still charged and an excess stops further loads, rather than proving a network cap. */
export interface ImpactLoadBudget {
  readonly remainingBytes: number;
  readonly remainingSaveBytes: number;
}
/** Host-authorized object/ref loads using ADR 0017/0021. The job receives no writer. */
export interface ImpactStoreSource {
  readonly kind: "store";
  readonly execution: string;
  readonly storage: { load(request: Uint8Array, budget: ImpactLoadBudget): Promise<Uint8Array> };
}
class ImpactReadLimit extends Error { constructor(readonly reason: "input_bytes" | "job_bytes") { super(reason); } }
export interface ImpactLimits {
  /** Distinct commits to assess, at most 10,000. */
  readonly saves: number;
  readonly totalBytes: number;
  /** UTF-8 save bytes or cumulative encoded store replies for one save, at most 16 MiB. */
  readonly saveBytes: number;
  /** Maximum Ref heads to enumerate; no extra head is pulled to test exhaustion. */
  readonly scanHeads?: number;
}
type Counts = Record<AssessmentV1["kind"], number>;
export interface ImpactReportV1 {
  readonly format_version: 1;
  readonly target: string;
  readonly migration_ids: readonly string[];
  readonly comparison_budget: ComparisonBudgetV1;
  readonly limits: Required<ImpactLimits>;
  readonly from_artifacts: readonly string[];
  readonly scan: { readonly revision_start?: string; readonly revision_end?: string; readonly enumerated: number; readonly exhausted: boolean };
  readonly sample: { readonly algorithm: "artifact-digest-v1"; readonly limit: number; readonly distinct_population: number | null; readonly selected: number };
  readonly examined: number;
  readonly evaluated_refs: number;
  readonly counts: Readonly<Counts>;
  readonly distinct_counts: Readonly<Counts>;
  readonly confirmation: Readonly<Record<"recovery" | "divergence" | "unverified", number>>;
  readonly unverified_reasons: Readonly<Record<string, number>>;
  readonly limit_reasons: Readonly<Record<"input_bytes" | "job_bytes" | "comparison", number>>;
  readonly coverage: { readonly equivalent_refs: number; readonly diverged_refs: number; readonly unverified_refs: number };
  readonly skipped: { readonly sampled_out: number; readonly stale_scan: number; readonly limit: number; readonly invalid_head: number };
  readonly examples: readonly { readonly kind: "divergence" | "recovery" | "unverified"; readonly reason?: string; readonly discarded_frames?: number; readonly differing_fields?: readonly string[] }[];
  /** Returned UTF-8 save bytes and encoded store replies, including rejected inputs.
   * May exceed the requested limits when a host callback returns an oversized payload. */
  readonly read_bytes: number;
  readonly missing_saves: number;
  readonly rejected_heads: number;
  readonly oversized_saves: number;
  readonly complete: boolean;
  readonly stopped: "source_exhausted" | "limit" | "cancelled";
}
const counts = (): Counts => ({ exact: 0, automatic: 0, needs_confirmation: 0, old_version_only: 0, needs_objects: 0, invalid_save: 0 });
const defaults: Required<ImpactLimits> = { saves: 1000, totalBytes: 64 * 1024 * 1024, saveBytes: 16 * 1024 * 1024, scanHeads: 100_000 };
const hex = (bytes: Uint8Array) => [...bytes].map(b => b.toString(16).padStart(2,"0")).join("");

// Match TextEncoder's UTF-8 length, including replacement of unpaired surrogates, without
// allocating a second body-sized buffer for an already oversized host result.
function utf8Bytes(text: string): number {
  let bytes = 0;
  for (let index = 0; index < text.length; index++) {
    const code = text.charCodeAt(index);
    if (code <= 0x7f) bytes++;
    else if (code <= 0x7ff) bytes += 2;
    else if (code >= 0xd800 && code <= 0xdbff && text.charCodeAt(index+1) >= 0xdc00 && text.charCodeAt(index+1) <= 0xdfff) { bytes += 4; index++; }
    else bytes += 3;
  }
  return bytes;
}

/**
 * Read-only authorized impact job. One head/save crosses each call (below the 256-head
 * maximum). Programs are registered once and shared by every snapshot. A bounded scan
 * deduplicates commits while retaining Ref weight, then selects a deterministic sample.
 * Only aggregates and bounded redacted examples leave the job; no reader identity/value
 * or save cursor is returned. A truncated scan never claims a known whole population.
 */
export async function assessImpact(input: {
  readonly evolution: Evolution;
  readonly heads: AsyncIterable<AuthorizedSaveHead> | Iterable<AuthorizedSaveHead>;
  readonly loadSave: (head: AuthorizedSaveHead, budget: ImpactLoadBudget) => Promise<string | undefined | ImpactStoreSource | { readonly kind: "stale_scan" }>;
  readonly options: EvolutionOptions;
  readonly limits?: ImpactLimits;
  readonly scan?: { readonly revision_start: string; readonly revision_end: string };
  readonly signal?: AbortSignal;
}): Promise<ImpactReportV1> {
  const limits = { ...defaults, ...input.limits };
  for (const key of ["saves","totalBytes","saveBytes","scanHeads"] as const) {
    const value = limits[key], max = key === "saves" ? 10_000 : key === "scanHeads" ? 100_000 : key === "saveBytes" ? 16 * 1024 * 1024 : 64 * 1024 * 1024;
    if (!Number.isSafeInteger(value) || value < 0 || value > max) throw new RangeError(`Invalid impact ${key} limit`);
  }
  if (input.evolution.connected) throw new Error("Impact assessment requires an isolated snapshot context.");
  const options: EvolutionOptions = { target: input.options.target, migrationIds: [...input.options.migrationIds], ...(input.options.budget ? { budget: { ...input.options.budget } } : {}) };
  const scan = input.scan && { ...input.scan };
  const weighted = counts(), distinct = counts(), confirmation = { recovery: 0, divergence: 0, unverified: 0 };
  const unverified: Record<string,number> = {};
  const limitReasons = { input_bytes: 0, job_bytes: 0, comparison: 0 };
  const coverage = { equivalent_refs: 0, diverged_refs: 0, unverified_refs: 0 };
  const examples: { kind: "divergence" | "recovery" | "unverified"; reason?: string; discarded_frames?: number; differing_fields?: readonly string[] }[] = [];
  let enumerated = 0, exhausted = false, examined = 0, evaluated = 0, bytes = 0, missing = 0, rejected = 0, oversized = 0, stale = 0, limited = 0;
  let stopped: ImpactReportV1["stopped"] = "source_exhausted";
  const population = new Map<string,{head: AuthorizedSaveHead; key: string; weight: number; conflicting: boolean}>();
  const iterable = input.heads;
  const iterator = Symbol.asyncIterator in iterable ? iterable[Symbol.asyncIterator]() : iterable[Symbol.iterator]();
  try {
    while (enumerated < limits.scanHeads) {
      if (input.signal?.aborted) { stopped = "cancelled"; break; }
      const next = await iterator.next();
      if (next.done) { exhausted = true; break; }
      enumerated++;
      const head = { ...next.value };
      if (!/^commit:[0-9a-f]{64}$/.test(head.commit) || !/^artifact:[0-9a-f]{64}$/.test(head.artifact)) { rejected++; continue; }
      const id = head.commit, existing = population.get(id);
      if (existing) { existing.weight++; existing.conflicting ||= existing.head.artifact !== head.artifact; continue; }
      population.set(id,{head,weight:1,conflicting:false,key:`${head.artifact}/${hex(evolutionSampleKey(head.artifact,head.commit))}/${head.commit}`});
    }
  } finally { await iterator.return?.(); }
  if (!exhausted && stopped !== "cancelled") stopped = "limit";
  // A commit can have only one checked artifact. Contradictory hints cannot choose a
  // different loader or cause the same commit to be assessed twice.
  for (const [commit,row] of population) if (row.conflicting) { rejected += row.weight; population.delete(commit); }
  const selected = [...population.values()].sort((a,b) => a.key < b.key ? -1 : a.key > b.key ? 1 : 0).slice(0,limits.saves);
  const selectedRefs = selected.reduce((sum,row) => sum+row.weight,0);
  const sampledOut = [...population.values()].reduce((sum,row) => sum+row.weight,0)-selectedRefs;
  if (sampledOut && stopped !== "cancelled") stopped = "limit";
  for (const {head,weight} of selected) {
    if (input.signal?.aborted || stopped === "cancelled") { stopped = "cancelled"; break; }
    if (bytes >= limits.totalBytes) { limited += weight; limitReasons.job_bytes += weight; stopped = "limit"; continue; }
    const save = await input.loadSave({ ...head },{remainingBytes:limits.totalBytes-bytes,remainingSaveBytes:limits.saveBytes});
    const size = typeof save === "string" ? utf8Bytes(save) : 0;
    bytes += size;
    if (input.signal?.aborted) { stopped = "cancelled"; break; }
    if (typeof save === "object" && save.kind === "stale_scan") { stale += weight; continue; }
    examined++;
    if (save === undefined) { missing += weight; weighted.needs_objects += weight; distinct.needs_objects++; evaluated += weight; continue; }
    let result: AssessmentV1;
    if (typeof save === "object") {
      let saveReadBytes = 0;
      const fork = await input.evolution.fork();
      try {
        await fork.connectStore(save.execution,{
          load:async request => {
            if (input.signal?.aborted || bytes >= limits.totalBytes) throw new ImpactReadLimit("job_bytes");
            if (saveReadBytes >= limits.saveBytes) throw new ImpactReadLimit("input_bytes");
            const reply = await save.storage.load(request,{remainingBytes:limits.totalBytes-bytes,remainingSaveBytes:limits.saveBytes-saveReadBytes});
            bytes += reply.byteLength;
            saveReadBytes += reply.byteLength;
            if (input.signal?.aborted) throw new ImpactReadLimit("job_bytes");
            if (saveReadBytes > limits.saveBytes) throw new ImpactReadLimit("input_bytes");
            if (bytes > limits.totalBytes) throw new ImpactReadLimit("job_bytes");
            return reply;
          },
          // No writer capability is accepted from the host. The existing-cursor assessment
          // is read-only; a regression trying to flush fails here before any external write.
          persist:async () => { throw new Error("Impact assessment cannot persist storage batches."); },
        });
        result = await fork.assess(options);
      } catch (error) {
        if (!(error instanceof ImpactReadLimit)) throw error;
        if (input.signal?.aborted) { stopped = "cancelled"; break; }
        limited += weight; limitReasons[error.reason] += weight; stopped = "limit"; break;
      } finally { await fork.close(); }
    } else {
      if (size > limits.saveBytes) { oversized += weight; limited += weight; limitReasons.input_bytes += weight; stopped = "limit"; break; }
      if (bytes > limits.totalBytes) { limited += weight; limitReasons.job_bytes += weight; stopped = "limit"; break; }
      result = await input.evolution.assessSave(save,options);
    }
    const source = result.kind === "exact" || result.kind === "old_version_only" ? result.source
      : result.kind === "automatic" || result.kind === "needs_confirmation" ? {source_commit:result.plan.source_commit,artifact_id:result.plan.report.from_artifact} : undefined;
    if (source && (source.source_commit !== head.commit || source.artifact_id !== head.artifact)) {
      if (typeof save === "object") stale += weight; else rejected += weight;
      continue;
    }
    weighted[result.kind] += weight; distinct[result.kind]++; evaluated += weight;
    if (result.kind === "needs_confirmation") confirmation[result.reason] += weight;
    if (result.kind === "automatic" || result.kind === "needs_confirmation") {
      const report = result.plan.report;
      if (report.path.kind === "unverified") {
        coverage.unverified_refs += weight;
        if (report.path.reason === "limit") limitReasons.comparison += weight;
        unverified[report.path.reason] = (unverified[report.path.reason] ?? 0)+weight;
        if (examples.length < 8) examples.push({kind:"unverified",reason:report.path.reason});
      } else if (report.path.kind === "diverged") {
        coverage.diverged_refs += weight;
        if (examples.length < 8) examples.push({kind:"divergence",reason:report.path.reason,differing_fields:report.path.differing_fields});
      } else coverage.equivalent_refs += weight;
      if (report.losses.length && examples.length < 8) examples.push({kind:"recovery",discarded_frames:report.losses.reduce((sum,loss) => sum+loss.discarded_frames.length,0)});
    }
  }
  return {
    format_version:1,target:options.target,migration_ids:options.migrationIds,from_artifacts:[...new Set(selected.map(row => row.head.artifact))].sort(),
    comparison_budget:{...(options.budget ?? defaultComparisonBudget)},limits,
    scan:{...scan,enumerated,exhausted},sample:{algorithm:"artifact-digest-v1",limit:limits.saves,distinct_population:exhausted ? population.size : null,selected:selected.length},
    examined,evaluated_refs:evaluated,counts:weighted,distinct_counts:distinct,confirmation,unverified_reasons:unverified,limit_reasons:limitReasons,coverage,
    skipped:{sampled_out:sampledOut,stale_scan:stale,limit:limited,invalid_head:rejected},examples,read_bytes:bytes,
    missing_saves:missing,rejected_heads:rejected,oversized_saves:oversized,complete:stopped === "source_exhausted" && stale === 0 && rejected === 0,stopped,
  };
}
