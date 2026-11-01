import init, { Source as RawSource, VnDrafts as RawVnDrafts, validateRecord as rawValidateRecord } from "../authoring-wasm/narrata_authoring_wasm.js";
import type { CompositionLock, DraftRecord, PreviewReport, RecordReport } from "../generated/authoring.js";
import type { CheckedRecord, RecordInput, WasmSource } from "../authoring/core.js";

const encoder = new TextEncoder();
const decoder = new TextDecoder("utf-8", { fatal: true });
const bytesOf = (record: RecordInput): Uint8Array => record instanceof Uint8Array ? record : encoder.encode(JSON.stringify(record));

function call<T>(operation: () => T): T {
  return operation();
}

function checked(raw: { report(): string; key(index: number): string | undefined; bytes(index: number): Uint8Array | undefined }): { report: RecordReport; records: CheckedRecord[] } {
  const report = JSON.parse(raw.report()) as RecordReport;
  const records: CheckedRecord[] = [];
  for (let index = 0; index < report.records; index++) {
    const key = raw.key(index), bytes = raw.bytes(index);
    if (key === undefined || bytes === undefined) throw new Error("The VN authoring module omitted a checked record");
    records.push({ key, bytes });
  }
  return { report, records };
}

/** A single-record structural check; complete VN semantics require checkVnRecords. */
export function checkVnRecord(input: RecordInput): { report: RecordReport; record?: CheckedRecord } {
  const raw = rawValidateRecord(bytesOf(input));
  try {
    const result = checked(raw);
    return result.records[0] ? { report: result.report, record: result.records[0] } : { report: result.report };
  } finally { raw.free(); }
}

/** Immutable compiled VN candidate; it has no publication or reader-save side effects. */
class VnPreview {
  readonly report: PreviewReport;
  #raw: ReturnType<RawVnDrafts["compose"]>;
  #freed = false;
  constructor(raw: ReturnType<RawVnDrafts["compose"]>) { this.#raw = raw; this.report = JSON.parse(raw.report()) as PreviewReport; }
  #live() { if (this.#freed) throw new Error("This VN preview was freed."); return this.#raw; }
  get compiled(): boolean { return this.report.artifact_id != null; }
  manifest(): Uint8Array | undefined { return this.#live().manifest(); }
  names(): Uint8Array | undefined { return this.#live().names(); }
  object(id: string): Uint8Array | undefined { return call(() => this.#live().object(id)); }
  pack(): Uint8Array { return call(() => this.#live().pack()); }
  free(): void { if (!this.#freed) this.#raw.free(); this.#freed = true; }
}

/** A source-format-3 draft set. It validates the complete VN snapshot before composing. */
class VnDrafts {
  #raw = new RawVnDrafts();
  #freed = false;
  #live(): RawVnDrafts { if (this.#freed) throw new Error("These VN drafts were freed."); return this.#raw; }
  add(record: RecordInput): void { this.#live().add(bytesOf(record)); }
  check(): { report: RecordReport; records: CheckedRecord[] } {
    const raw = this.#live().check();
    try { return checked(raw); } finally { raw.free(); }
  }
  /** Compile exactly this snapshot; no previous-publication continuity is available in the bridge. */
  preview(options: { lock?: CompositionLock } = {}): VnPreview {
    return new VnPreview(call(() => this.#live().compose(options.lock ? JSON.stringify(options.lock) : undefined)));
  }
  free(): void { if (!this.#freed) this.#raw.free(); this.#freed = true; }
}

export interface VnAuthoring {
  createVnDrafts(): VnDrafts;
  checkVnRecord: typeof checkVnRecord;
  splitVnSource(manifest: unknown, packages: Iterable<readonly [string, unknown]>): { report: RecordReport; records: CheckedRecord[] };
}
const api: VnAuthoring = {
  createVnDrafts: () => new VnDrafts(),
  checkVnRecord,
  splitVnSource(manifest, packages) {
    const raw = call(() => new RawSource(JSON.stringify(manifest)));
    try {
      for (const [alias, source] of packages) call(() => raw.addPackage(alias, JSON.stringify(source)));
      const split = raw.split();
      try { return checked(split); } finally { split.free(); }
    } finally { raw.free(); }
  },
};
let initialized: Promise<void> | undefined;

/** Initializes the shared authoring Wasm lazily; Node/Bun callers should use server.loadVnAuthoring. */
export function loadVnAuthoringFrom(source: WasmSource | (() => Promise<WasmSource>)): Promise<VnAuthoring> {
  if (!initialized) initialized = (async () => {
    let input = typeof source === "function" ? await source() : source;
    const url = input instanceof URL ? input : typeof input === "string" ? new URL(input, import.meta.url) : undefined;
    if (url?.protocol === "file:") {
      const module = "node:fs/promises";
      const { readFile }: typeof import("node:fs/promises") = await import(/* @vite-ignore */ module);
      input = new Uint8Array(await readFile(url));
    }
    await init({ module_or_path: input });
  })().catch(error => { initialized = undefined; throw error; });
  return initialized.then(() => api);
}

export function loadVnAuthoring(source?: WasmSource): Promise<VnAuthoring> {
  return loadVnAuthoringFrom(source ?? new URL("../authoring-wasm/narrata_authoring_wasm_bg.wasm", import.meta.url));
}

export type { CompositionLock, DraftRecord, PreviewReport, RecordReport, WasmSource };
