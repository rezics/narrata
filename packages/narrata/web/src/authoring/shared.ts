import init, { Preview as RawPreview, type Records as RawRecords } from "../authoring-wasm/narrata_authoring_wasm.js";
import type { AuthoredId, Diagnostic, DraftRecord, PreviewReport, RecordIndexes, RecordReport } from "../generated/authoring.js";

export type WasmSource = URL | string | Uint8Array | ArrayBuffer | WebAssembly.Module;
export type RecordInput = Uint8Array | DraftRecord;

export interface CheckedRecord {
  readonly key: string;
  readonly bytes: Uint8Array;
  readonly indexes?: RecordIndexes;
}

export interface RecordsResult {
  readonly complete: boolean;
  readonly diagnostics: readonly Diagnostic[];
  readonly records: readonly CheckedRecord[];
  readonly minted: readonly AuthoredId[];
}

export interface RecordCheck {
  readonly complete: boolean;
  readonly diagnostics: readonly Diagnostic[];
  readonly record?: CheckedRecord;
}

const encoder = new TextEncoder();
export function recordBytes(record: RecordInput): Uint8Array {
  return record instanceof Uint8Array ? record : encoder.encode(JSON.stringify(record));
}

/** Reads the common checked-record surface. The caller owns and frees the Wasm result. */
export function readRecords(raw: RawRecords): RecordsResult {
  const report = JSON.parse(raw.report()) as RecordReport;
  const records: CheckedRecord[] = [];
  for (let index = 0; index < report.records; index++) {
    const key = raw.key(index), bytes = raw.bytes(index), indexes = raw.indexes(index);
    if (key === undefined || bytes === undefined) throw new Error("The authoring module omitted a checked record");
    records.push(indexes === undefined ? { key, bytes } : { key, bytes, indexes: JSON.parse(indexes) as RecordIndexes });
  }
  return { complete: report.complete, diagnostics: report.diagnostics, records, minted: report.minted };
}

/** Freed handles throw before touching their released Wasm memory; free is idempotent. */
export abstract class Handle<T extends { free(): void }> {
  #raw: T | undefined;
  constructor(raw: T) { this.#raw = raw; }
  protected get raw(): T {
    if (!this.#raw) throw new Error("This authoring handle was freed.");
    return this.#raw;
  }
  protected take(): T {
    const raw = this.raw;
    this.#raw = undefined;
    return raw;
  }
  free(): void { this.#raw?.free(); this.#raw = undefined; }
}

type Invoke = <T>(operation: () => T) => T;
const direct: Invoke = operation => operation();

/** Program objects of a preview compilation; never a publication. */
export class Preview extends Handle<RawPreview> {
  readonly report: PreviewReport;
  constructor(raw: RawPreview) { super(raw); this.report = JSON.parse(raw.report()) as PreviewReport; }
  protected invoke<T>(operation: () => T): T { return direct(operation); }
  get compiled(): boolean { return this.report.artifact_id != null; }
  manifest(): Uint8Array | undefined { return this.raw.manifest(); }
  names(): Uint8Array | undefined { return this.raw.names(); }
  object(id: string): Uint8Array | undefined { return this.invoke(() => this.raw.object(id)); }
  pack(): Uint8Array { return this.invoke(() => this.raw.pack()); }
}

class AuthoringPreview extends Preview {
  constructor(raw: RawPreview, private readonly translate: Invoke) { super(raw); }
  protected override invoke<T>(operation: () => T): T { return this.translate(operation); }
}

/** Internal Gamebook construction keeps its established AuthoringError conversion. */
export function createAuthoringPreview(raw: RawPreview, invoke: Invoke): Preview {
  return new AuthoringPreview(raw, invoke);
}

let initialized: Promise<void> | undefined;

/** One lazy Wasm initialization shared by the Gamebook and VN facades in this JS realm. */
export function loadAuthoringWasmFrom(
  source: WasmSource | (() => Promise<WasmSource>),
  baseUrl: string | URL,
): Promise<void> {
  if (!initialized) {
    initialized = (async () => {
      let input = typeof source === "function" ? await source() : source;
      const url = input instanceof URL ? input : typeof input === "string" ? new URL(input, baseUrl) : undefined;
      if (url?.protocol === "file:") {
        const module = "node:fs/promises";
        const { readFile }: typeof import("node:fs/promises") = await import(/* @vite-ignore */ module);
        input = new Uint8Array(await readFile(url));
      }
      await init({ module_or_path: input });
    })().catch(error => { initialized = undefined; throw error; });
  }
  return initialized;
}
