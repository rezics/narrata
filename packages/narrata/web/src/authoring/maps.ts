import type { Visits as RawVisits, ReaderRelations as RawReaderRelations } from "../authoring-wasm/narrata_authoring_wasm.js";
import type { Publication } from "./core.js";
import type { VisitReportV1, ReaderMapV1, RelationRow } from "../generated/authoring.js";
export type { VisitReportV1, ReaderMapV1, ReaderUnitV1, ReaderNodeV1, ReaderEdgeV1, VisitFrontierV1 } from "../generated/authoring.js";

/** Checked ancestor evidence, owned by Rust. A deserialized report cannot create this handle. */
export class Visits {
  #raw: RawVisits | undefined;
  /** @internal */
  constructor(raw: RawVisits) { this.#raw = raw; }
  get #handle(): RawVisits { if (!this.#raw) throw new Error("The visits handle was freed."); return this.#raw; }
  report(): VisitReportV1 { return JSON.parse(this.#handle.report()) as VisitReportV1; }
  readerMap(publication: Publication): ReaderMapV1 { return JSON.parse(this.#handle.readerMap(publication.handle)) as ReaderMapV1; }
  readerRelations(publication: Publication): ReaderRelations { return new ReaderRelations(this.#handle.readerRelations(publication.handle)); }
  /** Consumes both reports, retaining each cursor's provenance. */
  merge(other: Visits): Visits {
    const a = this.#handle, b = other.#handle;
    this.#raw = undefined; other.#raw = undefined;
    return new Visits(a.merge(b));
  }
  free(): void { this.#raw?.free(); this.#raw = undefined; }
}

/** Visits-derived sparse allowlist, distinct from an author's complete projection. */
export class ReaderRelations {
  #raw: RawReaderRelations | undefined;
  /** @internal */
  constructor(raw: RawReaderRelations) { this.#raw = raw; }
  get #handle(): RawReaderRelations { if (!this.#raw) throw new Error("The reader relations handle was freed."); return this.#raw; }
  get artifactId(): string { return this.#handle.artifactId; }
  get indexId(): string { return this.#handle.indexId; }
  get count(): number { return this.#handle.count; }
  rows(offset = 0, limit = 256): readonly RelationRow[] { return JSON.parse(this.#handle.rows(offset, limit)) as RelationRow[]; }
  checkRows(offset: number, limit: number, rows: unknown): void { this.#handle.checkRows(offset, limit, JSON.stringify(rows)); }
  free(): void { this.#raw?.free(); this.#raw = undefined; }
}
