import {
  Drafts as RawDrafts, IdMinter as RawMinter, Publication as RawPublication,
  PublicationLoader as RawLoader, Records as RawRecords, Source as RawSource, validateRecord as rawValidateRecord,
} from "../authoring-wasm/narrata_authoring_wasm.js";
import { createAuthoringPreview, Handle, Preview, loadAuthoringWasmFrom, readRecords, recordBytes } from "./shared.js";
import type { CheckedRecord, RecordCheck, RecordsResult, RecordInput, WasmSource } from "./shared.js";
import type {
  AuthoredId, CompositionLock, ContentOutline, Diagnostic, DraftRecord, MintedIds, ObjectReference, PackageSource,
  PreviewReport, ProjectManifest, PublicationDescription, PublishReport, RecordReport,
  RelationIndex, RelationKind, RelationPart, RelationRow, AuthorRelationQuery, AuthorRelationPage,
} from "../generated/authoring.js";
import { Evolution, EvolutionPlan, planEvolution, readEvolution } from "./evolution.js";
import { Maintenance, type MaintenanceOptions } from "../maintenance/index.js";
export * from "../maintenance/index.js";
export * from "./evolution.js";
export * from "./impact.js";
export * from "./evolution-controller.js";
export * from "./maps.js";
export type { RelationIndex, RelationKind, RelationPart, RelationRow, AuthorRelationQuery, AuthorRelationPage } from "../generated/authoring.js";

export type { CheckedRecord, RecordCheck, RecordsResult, WasmSource } from "./shared.js";

/** A thrown authoring failure; checks and publication report diagnostics instead of throwing. */
export class AuthoringError extends Error {
  constructor(readonly code: string, readonly path: string, detail: string) {
    super(`${code} at ${path}: ${detail}`);
    this.name = "AuthoringError";
  }
}

function call<T>(operation: () => T): T {
  try { return operation(); } catch (error) {
    const match = error instanceof Error ? /^([a-z0-9_]+) at (.*?): ([\s\S]*)$/.exec(error.message) : null;
    if (match?.[1] !== undefined && match[2] !== undefined && match[3] !== undefined) throw new AuthoringError(match[1], match[2], match[3]);
    throw error;
  }
}

const decoder = new TextDecoder("utf-8", { fatal: true });
/** Stored record bytes, or a typed record encoded as JSON for checking and normalization. */
export type { RecordInput } from "./shared.js";
/** Checks external bytes with the Rust record validator before returning a typed record. */
export function decodeRecord(bytes: Uint8Array): DraftRecord {
  const result = records(call(() => rawValidateRecord(bytes)));
  const checked = result.records[0];
  if (!checked) {
    const diagnostic = result.diagnostics.find(diagnostic => diagnostic.severity === "error");
    throw new AuthoringError(diagnostic?.code ?? "record", diagnostic?.source_path ?? "$", diagnostic?.message ?? "No checked record");
  }
  // Only Rust-validated, canonical bytes establish the generated DraftRecord contract.
  return JSON.parse(decoder.decode(checked.bytes)) as DraftRecord;
}

export interface AssembledSource {
  readonly complete: boolean;
  readonly diagnostics: readonly Diagnostic[];
  /** The normalized R2 source for file export; absent when the records do not assemble. */
  readonly source?: { readonly manifest: ProjectManifest; readonly packages: ReadonlyMap<string, PackageSource> };
}

function records(raw: RawRecords): RecordsResult {
  try { return readRecords(raw); } finally { raw.free(); }
}

/** A checked, immutable publication: description and objects, retrieved one at a time. */
export class Publication extends Handle<RawPublication> {
  /** Complete checked projection metadata. Row transport stays bounded and Rust-backed. */
  relationIndex(): RelationIndex { return JSON.parse(call(() => this.raw.relationIndex())) as RelationIndex; }
  get relationIndexId(): string { return this.raw.relationIndexId(); }
  relationRows(kind: RelationKind, offset = 0, limit = 256): readonly RelationRow[] {
    return JSON.parse(call(() => this.raw.relationRows(JSON.stringify(kind), offset, limit))) as RelationRow[];
  }
  relationPart(ordinal: number): RelationPart | undefined {
    const part = call(() => this.raw.relationPart(ordinal));
    return part === undefined ? undefined : JSON.parse(part) as RelationPart;
  }
  /** Driver-decoded unknown rows return to Rust for exact column and page comparison. */
  checkRelationRows(kind: RelationKind, offset: number, limit: number, rows: unknown): void {
    call(() => this.raw.checkRelationRows(JSON.stringify(kind), offset, limit, JSON.stringify(rows)));
  }
  queryRelations(query: AuthorRelationQuery, offset = 0, limit = 256): AuthorRelationPage {
    return JSON.parse(call(() => this.raw.queryRelations(JSON.stringify(query), offset, limit))) as AuthorRelationPage;
  }
  checkRelationQuery(query: AuthorRelationQuery, offset: number, limit: number, rows: unknown): void {
    call(() => this.raw.checkRelationQuery(JSON.stringify(query), offset, limit, JSON.stringify(rows)));
  }
  /** Consumes this candidate and returns its new publication identity. Retain the old publication in host storage. */
  withEvolution(previous: Publication, plan: EvolutionPlan): Publication {
    const raw = this.take();
    return new Publication(call(() => raw.withEvolution(previous.handle, plan.handle)));
  }
  readonly publicationId: string;
  readonly artifactId: string;
  readonly descriptionBytes: Uint8Array;
  readonly description: PublicationDescription;
  /** @internal */
  constructor(raw: RawPublication) {
    super(raw);
    this.publicationId = raw.publicationId;
    this.artifactId = raw.artifactId;
    this.descriptionBytes = raw.description();
    this.description = JSON.parse(decoder.decode(this.descriptionBytes)) as PublicationDescription;
  }
  /** By description ID (hex) or a reader chunk request (`object:<hex>`). */
  object(id: string): Uint8Array | undefined { return call(() => this.raw.object(id)); }
  /** Every referenced object, read lazily in description order. */
  *objects(): IterableIterator<readonly [ObjectReference, Uint8Array]> {
    for (const reference of this.description.objects) {
      const bytes = this.object(reference.object_id);
      if (!bytes) throw new Error(`Publication object ${reference.object_id} is missing`);
      yield [reference, bytes];
    }
  }
  /** The legacy single-file pack, limited to 64 MiB. */
  pack(): Uint8Array { return call(() => this.raw.pack()); }
  /** @internal */
  get handle(): RawPublication { return this.raw; }
}

export { Preview } from "./shared.js";

export interface PublishResult {
  readonly report: PublishReport;
  /** Normalized records including appended tombstones, to persist before switching. */
  readonly records: readonly CheckedRecord[];
  /** The checked candidate; free it when the host has stored its objects. */
  readonly publication?: Publication;
}

export interface ComposeOptions {
  /** The work's current publication. Every publication after the first must pass it. */
  readonly previous?: Publication;
  /** Locked composition: tombstones must already be recorded and the lock must match. */
  readonly lock?: CompositionLock;
}

/** Persisted records and outline batches; records are named `records[<insertion index>]` in diagnostics. */
export class Drafts extends Handle<RawDrafts> {
  add(record: RecordInput): void { this.raw.add(recordBytes(record)); }
  /** One outline batch; batches of a provider assemble into its single outline. */
  addOutline(outline: ContentOutline): void { call(() => this.raw.addOutline(JSON.stringify(outline))); }
  /** Record, assembly and semantic checks with derived indexes; nothing is compiled. */
  check(): RecordsResult { return records(this.raw.check()); }
  assemble(): AssembledSource {
    const raw = this.raw.check();
    try {
      const report = JSON.parse(raw.report()) as RecordReport;
      const manifest = raw.manifest();
      if (manifest === undefined) return { complete: report.complete, diagnostics: report.diagnostics };
      const packages = new Map<string, PackageSource>();
      for (const alias of raw.packages()) packages.set(alias, JSON.parse(raw.package(alias) ?? "null") as PackageSource);
      return { complete: report.complete, diagnostics: report.diagnostics, source: { manifest: JSON.parse(manifest) as ProjectManifest, packages } };
    } finally { raw.free(); }
  }
  /** Compiles program objects for an immediate preview with the reader's openBook. */
  preview(options: ComposeOptions = {}): Preview {
    const lock = options.lock && JSON.stringify(options.lock);
    return createAuthoringPreview(call(() => options.previous ? this.raw.composeAfter(options.previous.handle, lock) : this.raw.compose(lock)), call);
  }
  publish(options: ComposeOptions = {}): PublishResult {
    const lock = options.lock && JSON.stringify(options.lock);
    const raw = call(() => options.previous ? this.raw.publishAfter(options.previous.handle, lock) : this.raw.publish(lock));
    try {
      const report = JSON.parse(raw.report()) as PublishReport;
      const publication = raw.takePublication();
      const result = { report, records: records(raw.takeRecords()).records };
      return publication ? { ...result, publication: new Publication(publication) } : result;
    } finally { raw.free(); }
  }
}

/** Restores a previous publication from its checked description and individually fetched objects. */
export class PublicationLoader extends Handle<RawLoader> {
  readonly references: readonly ObjectReference[];
  /** @internal */
  constructor(raw: RawLoader) { super(raw); this.references = JSON.parse(raw.references()) as ObjectReference[]; }
  missing(): string[] { return this.raw.missing(); }
  supply(id: string, bytes: Uint8Array): void { call(() => this.raw.supply(id, bytes)); }
  /** Verifies the whole collection and consumes the loader, whether or not it succeeds. */
  finish(previous?: Publication): Publication {
    const raw = this.take();
    return new Publication(call(() => previous ? raw.finishAfter(previous.handle) : raw.finish()));
  }
}

/** UUIDv7 inputs. Defaults are Date.now and the host's Web Crypto random source. */
export interface Entropy {
  now(): number;
  random(bytes: Uint8Array): void;
}
export const hostEntropy: Entropy = {
  now: () => Date.now(),
  random: bytes => {
    if (!globalThis.crypto?.getRandomValues) throw new Error("Web Crypto is unavailable; supply an Entropy source.");
    globalThis.crypto.getRandomValues(bytes);
  },
};
export interface MintCounts { readonly nodes?: number; readonly choicePoints?: number; readonly options?: number }

/** Mints IDs avoiding every occupied live or deleted ID; it never generates host database keys. */
export class IdMinter extends Handle<RawMinter> {
  occupy(ids: Iterable<AuthoredId>): void { call(() => this.raw.occupy([...ids])); }
  mint(counts: MintCounts): MintedIds {
    return JSON.parse(call(() => this.raw.mint(counts.nodes ?? 0, counts.choicePoints ?? 0, counts.options ?? 0))) as MintedIds;
  }
  /** Fills only the missing IDs of an R2 source, keeping every existing one, and splits it into records. */
  fill(manifest: ProjectManifest, packages: Iterable<readonly [string, PackageSource]>): RecordsResult {
    const source = rawSource(manifest, packages);
    try { return records(call(() => this.raw.fill(source))); } finally { source.free(); }
  }
}

function rawSource(manifest: ProjectManifest, packages: Iterable<readonly [string, PackageSource]>): RawSource {
  const source = call(() => new RawSource(JSON.stringify(manifest)));
  try {
    for (const [alias, pkg] of packages) call(() => source.addPackage(alias, JSON.stringify(pkg)));
    return source;
  } catch (error) { source.free(); throw error; }
}

export interface Authoring {
  planEvolution: typeof planEvolution;
  readEvolution: typeof readEvolution;
  /** A separate assessment/application context. Only register trusted publications. */
  createEvolution(): Evolution;
  createMaintenance(options: MaintenanceOptions): Maintenance;
  /** Checks one record's encoding, fields and independently decidable constraints. */
  validateRecord(record: RecordInput): RecordCheck;
  createDrafts(): Drafts;
  /** Splits a minted R2 source, given as its manifest and packages, into draft records. */
  splitSource(manifest: ProjectManifest, packages: Iterable<readonly [string, PackageSource]>): RecordsResult;
  createMinter(entropy?: Entropy): IdMinter;
  openPublication(description: Uint8Array, publicationId: string): PublicationLoader;
  /** Fetches each referenced object once through `load`, then verifies the publication. */
  readPublication(description: Uint8Array, publicationId: string, load: (reference: ObjectReference) => Promise<Uint8Array>, previous?: Publication): Promise<Publication>;
  publicationFromPack(pack: Uint8Array): Publication;
}

const authoring: Authoring = {
  planEvolution,
  readEvolution,
  createEvolution: () => new Evolution(),
  createMaintenance: options => new Maintenance(options),
  validateRecord(record) {
    const result = records(rawValidateRecord(recordBytes(record)));
    const checked = result.records[0];
    return checked ? { complete: result.complete, diagnostics: result.diagnostics, record: checked } : { complete: result.complete, diagnostics: result.diagnostics };
  },
  createDrafts: () => new Drafts(new RawDrafts()),
  splitSource(manifest, packages) {
    const source = rawSource(manifest, packages);
    try { return records(source.split()); } finally { source.free(); }
  },
  createMinter: (entropy = hostEntropy) => new IdMinter(new RawMinter({ now: () => entropy.now(), random: (bytes: Uint8Array) => entropy.random(bytes) })),
  openPublication: (description, publicationId) => new PublicationLoader(call(() => new RawLoader(description, publicationId))),
  async readPublication(description, publicationId, load, previous) {
    const loader = authoring.openPublication(description, publicationId);
    try {
      for (const reference of loader.references) loader.supply(reference.object_id, await load(reference));
      return loader.finish(previous);
    } finally { loader.free(); }
  },
  publicationFromPack: pack => new Publication(call(() => RawPublication.fromPack(pack))),
};

/** Initializes the authoring module once per JS realm, independently of the reader's module. */
export function loadAuthoringFrom(source: WasmSource | (() => Promise<WasmSource>)): Promise<Authoring> {
  return loadAuthoringWasmFrom(source, import.meta.url).then(() => authoring);
}

export type {
  AuthoredId, ChoicePointId, CompositionLock, ContentOutline, Diagnostic, DraftRecord, MintedIds, NodeId, ObjectReference, OptionId,
  OptionIndex, OwnershipIndex, PackageSource, PreviewReport, ProjectManifest, PublicationDescription, PublicationStatus, PublishReport,
  RecordIndexes, RecordReport, GraphSource, NodeSource, ChoicePointSource, OptionSource, OutcomeSource, ContentRef, Segment, ProductSource,
} from "../generated/authoring.js";
