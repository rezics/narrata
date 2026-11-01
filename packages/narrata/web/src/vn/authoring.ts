import { Source as RawSource, VnDrafts as RawVnDrafts, validateRecord as rawValidateRecord } from "../authoring-wasm/narrata_authoring_wasm.js";
import { Handle, Preview, loadAuthoringWasmFrom, readRecords, recordBytes } from "../authoring/shared.js";
import type { RecordCheck, RecordInput, RecordsResult, WasmSource } from "../authoring/shared.js";
import type { CompositionLock, DraftRecord, PreviewReport, RecordReport } from "../generated/authoring.js";

/** A single-record structural check; a full record set is required for VN semantics. */
export function checkVnRecord(input: RecordInput): RecordCheck {
  const raw = rawValidateRecord(recordBytes(input));
  try {
    const result = readRecords(raw);
    const record = result.records[0];
    return record
      ? { complete: result.complete, diagnostics: result.diagnostics, record }
      : { complete: result.complete, diagnostics: result.diagnostics };
  } finally { raw.free(); }
}

/** A source-format-3 draft set, checked with the first-party VN registry. */
class VnDrafts extends Handle<RawVnDrafts> {
  constructor() { super(new RawVnDrafts()); }
  add(record: RecordInput): void { this.raw.add(recordBytes(record)); }
  check(): RecordsResult {
    const raw = this.raw.check();
    try { return readRecords(raw); } finally { raw.free(); }
  }
  /** Compiles this snapshot only; the raw VN bridge has no publication continuity route. */
  preview(options: { readonly lock?: CompositionLock } = {}): Preview {
    return new Preview(this.raw.compose(options.lock ? JSON.stringify(options.lock) : undefined));
  }
}

export interface VnAuthoring {
  createVnDrafts(): VnDrafts;
  checkVnRecord: typeof checkVnRecord;
  /** Converts host source JSON into draft records; full VN semantics are checked by the returned draft set. */
  splitVnSource(manifest: unknown, packages: Iterable<readonly [string, unknown]>): RecordsResult;
}

const api: VnAuthoring = {
  createVnDrafts: () => new VnDrafts(),
  checkVnRecord,
  splitVnSource(manifest, packages) {
    const raw = new RawSource(JSON.stringify(manifest));
    try {
      for (const [alias, source] of packages) raw.addPackage(alias, JSON.stringify(source));
      const split = raw.split();
      try { return readRecords(split); } finally { split.free(); }
    } finally { raw.free(); }
  },
};

/** Initializes the same authoring Wasm used by the browser facade, without loading Gamebook APIs. */
export function loadVnAuthoringFrom(source: WasmSource | (() => Promise<WasmSource>)): Promise<VnAuthoring> {
  return loadAuthoringWasmFrom(source, import.meta.url).then(() => api);
}

/** Browser loader. Node/Bun callers use the isolated `@rezics/narrata/vn/server` entry. */
export function loadVnAuthoring(source?: WasmSource): Promise<VnAuthoring> {
  return loadVnAuthoringFrom(source ?? new URL("../authoring-wasm/narrata_authoring_wasm_bg.wasm", import.meta.url));
}

export type { CompositionLock, DraftRecord, PreviewReport, RecordReport, WasmSource };
