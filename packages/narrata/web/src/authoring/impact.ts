import { evolutionSampleKey } from "../authoring-wasm/narrata_authoring_wasm.js";
import type { AssessmentV1, Evolution, EvolutionOptions } from "./evolution.js";

/** Authorized lookup hints; only the loaded, checked commit proves their meaning. */
export interface AuthorizedSaveHead { readonly commit: string; readonly artifact: string }
export interface ImpactLimits {
  /** Distinct commits to assess, at most 10,000. */
  readonly saves: number;
  readonly totalBytes: number;
  /** Each input is bounded to ADR 0023's 16 MiB, independently of the whole job. */
  readonly saveBytes: number;
  /** Maximum Ref heads to enumerate; no extra head is pulled to test exhaustion. */
  readonly scanHeads?: number;
}
type Counts = Record<AssessmentV1["kind"], number>;
export interface ImpactReportV1 {
  readonly format_version: 1;
  readonly target: string;
  readonly migration_ids: readonly string[];
  readonly from_artifacts: readonly string[];
  readonly scan: { readonly revision_start?: string; readonly revision_end?: string; readonly enumerated: number; readonly exhausted: boolean };
  readonly sample: { readonly algorithm: "artifact-digest-v1"; readonly limit: number; readonly distinct_population: number | null; readonly selected: number };
  readonly examined: number;
  readonly evaluated_refs: number;
  readonly counts: Readonly<Counts>;
  readonly distinct_counts: Readonly<Counts>;
  readonly confirmation: Readonly<Record<"recovery" | "divergence" | "unverified", number>>;
  readonly unverified_reasons: Readonly<Record<string, number>>;
  readonly skipped: { readonly sampled_out: number; readonly stale_scan: number; readonly limit: number; readonly invalid_head: number };
  readonly examples: readonly { readonly kind: "divergence" | "recovery" | "unverified"; readonly reason?: string; readonly discarded_frames?: number; readonly differing_fields?: readonly string[] }[];
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
  readonly loadSave: (head: AuthorizedSaveHead) => Promise<string | undefined | { readonly kind: "stale_scan" }>;
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
  const examples: { kind: "divergence" | "recovery" | "unverified"; reason?: string; discarded_frames?: number; differing_fields?: readonly string[] }[] = [];
  let enumerated = 0, exhausted = false, examined = 0, evaluated = 0, bytes = 0, missing = 0, rejected = 0, oversized = 0, stale = 0, limited = 0;
  let stopped: ImpactReportV1["stopped"] = "source_exhausted";
  const population = new Map<string,{head: AuthorizedSaveHead; key: string; weight: number}>();
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
      const id = `${head.artifact}/${head.commit}`, existing = population.get(id);
      if (existing) { existing.weight++; continue; }
      population.set(id,{head,weight:1,key:`${head.artifact}/${hex(evolutionSampleKey(head.artifact,head.commit))}/${head.commit}`});
    }
  } finally { await iterator.return?.(); }
  if (!exhausted && stopped !== "cancelled") stopped = "limit";
  const selected = [...population.values()].sort((a,b) => a.key < b.key ? -1 : a.key > b.key ? 1 : 0).slice(0,limits.saves);
  const selectedRefs = selected.reduce((sum,row) => sum+row.weight,0);
  const sampledOut = [...population.values()].reduce((sum,row) => sum+row.weight,0)-selectedRefs;
  if (sampledOut && stopped !== "cancelled") stopped = "limit";
  const encoder = new TextEncoder();
  for (const {head,weight} of selected) {
    if (input.signal?.aborted || stopped === "cancelled") { stopped = "cancelled"; break; }
    if (bytes >= limits.totalBytes) { limited += weight; stopped = "limit"; continue; }
    const save = await input.loadSave({ ...head });
    if (input.signal?.aborted) { stopped = "cancelled"; break; }
    if (typeof save === "object") { stale += weight; continue; }
    examined++;
    if (save === undefined) { missing += weight; weighted.needs_objects += weight; distinct.needs_objects++; evaluated += weight; continue; }
    if (save.length > limits.saveBytes) { oversized += weight; limited += weight; stopped = "limit"; continue; }
    const size = encoder.encode(save).byteLength;
    if (size > limits.saveBytes || size > limits.totalBytes-bytes) { oversized += weight; limited += weight; stopped = "limit"; continue; }
    bytes += size;
    const result = await input.evolution.assessSave(save,options);
    const source = result.kind === "exact" || result.kind === "old_version_only" ? result.source
      : result.kind === "automatic" || result.kind === "needs_confirmation" ? {source_commit:result.plan.source_commit,artifact_id:result.plan.report.from_artifact} : undefined;
    if (source && (source.source_commit !== head.commit || source.artifact_id !== head.artifact)) { rejected += weight; continue; }
    weighted[result.kind] += weight; distinct[result.kind]++; evaluated += weight;
    if (result.kind === "needs_confirmation") confirmation[result.reason] += weight;
    if (result.kind === "automatic" || result.kind === "needs_confirmation") {
      const report = result.plan.report;
      if (report.path.kind === "unverified") {
        unverified[report.path.reason] = (unverified[report.path.reason] ?? 0)+weight;
        if (examples.length < 8) examples.push({kind:"unverified",reason:report.path.reason});
      } else if (report.path.kind === "diverged" && examples.length < 8) examples.push({kind:"divergence",reason:report.path.reason,differing_fields:report.path.differing_fields});
      if (report.losses.length && examples.length < 8) examples.push({kind:"recovery",discarded_frames:report.losses.reduce((sum,loss) => sum+loss.discarded_frames.length,0)});
    }
  }
  return {
    format_version:1,target:options.target,migration_ids:options.migrationIds,from_artifacts:[...new Set(selected.map(row => row.head.artifact))].sort(),
    scan:{...scan,enumerated,exhausted},sample:{algorithm:"artifact-digest-v1",limit:limits.saves,distinct_population:exhausted ? population.size : null,selected:selected.length},
    examined,evaluated_refs:evaluated,counts:weighted,distinct_counts:distinct,confirmation,unverified_reasons:unverified,
    skipped:{sampled_out:sampledOut,stale_scan:stale,limit:limited,invalid_head:rejected},examples,read_bytes:bytes,
    missing_saves:missing,rejected_heads:rejected,oversized_saves:oversized,complete:stopped === "source_exhausted" && stale === 0 && rejected === 0,stopped,
  };
}
