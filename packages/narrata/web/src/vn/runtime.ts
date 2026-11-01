import init, { VnBook } from "../vn-reader-wasm/narrata_nodes_wasm.js";
import { RuntimeTransport, type ReaderEngine } from "../transport.js";
import { CandidateObjects, type CandidateObjectBytes, type CandidateObjectStatus } from "./candidate-objects.js";
export type { CandidateObjectStatus } from "./candidate-objects.js";
import type { CacheStore } from "../storage.js";
import type { ArtifactLoader } from "../artifacts.js";
import type { WasmSource } from "../runtime.js";
import validate from "../generated/validate-vn-view.js";
import validateDependencies from "../generated/validate-vn-dependencies.js";
import type { DependencyLimitsV1, DependencyRequestV1, DependencyResultV1 } from "../generated/vn-dependencies.js";
import type { ArtifactId, ChoicePointId, CommitId, ExecutionId, NodeId, OptionId, VnViewV1 } from "../generated/vn-view.js";

/** Parse Rust-produced or externally supplied JSON through the generated VN v1 schema. */
export function checkedVnView(value: unknown): VnViewV1 {
  if (!validate(value)) throw new Error("The runtime returned an invalid VN View v1; rebuild the runtime.");
  return value;
}

export function decodeVnView(text: string): VnViewV1 {
  return checkedVnView(JSON.parse(text));
}

type SessionOptions = {
  execution: ExecutionId;
  storage?: CacheStore;
  wasm?: WasmSource;
  /** Exact historic artifacts needed to read a saved branch; limited by the runtime cache. */
  historyPacks?: readonly Uint8Array[];
  /** Resolves the exact artifact requested by history, without a product-name fallback. */
  loadArtifact?: ArtifactLoader;
};

export type OpenVnOptions = SessionOptions & (
  | { manifest: Uint8Array; fetchChunk: (id: string, signal?: AbortSignal) => Promise<Uint8Array>; names?: Uint8Array; pack?: never }
  | { pack: Uint8Array; manifest?: never; fetchChunk?: never; names?: never }
);

export type VnDependencyLimits = DependencyLimitsV1;
export type VnDependencyResult = DependencyResultV1;
export interface VnCandidatePreparation {
  readonly status: "complete" | "partial" | "cancelled";
  readonly dependencies: VnDependencyResult;
  readonly objects: readonly {
    readonly object: string;
    readonly status: CandidateObjectStatus;
  }[];
}

let initialized: Promise<unknown> | undefined;
let initializedSource: WasmSource | undefined;
/** Initialize the feature-enabled VN reader module independently of the Gamebook reader. */
function initializeVn(source?: WasmSource): Promise<void> {
  const sourceToLoad = source ?? new URL("../vn-reader-wasm/narrata_nodes_wasm_bg.wasm", import.meta.url);
  if (initialized) {
    if (!initializedSource || !sameWasmSource(initializedSource, sourceToLoad)) {
      return Promise.reject(new Error("A VN reader realm uses one Wasm module; every openVn call must use that same source."));
    }
    return initialized.then(() => undefined);
  }
  const boundSource = sourceToLoad instanceof URL ? new URL(sourceToLoad.href)
    : sourceToLoad instanceof Uint8Array ? new Uint8Array(sourceToLoad)
    : sourceToLoad instanceof ArrayBuffer ? new Uint8Array(new Uint8Array(sourceToLoad)) : sourceToLoad;
  initializedSource = boundSource;
  if (!initialized) {
    initialized = (async () => {
      let input = boundSource;
      const url = input instanceof URL ? input : typeof input === "string" ? new URL(input, import.meta.url) : undefined;
      if (url?.protocol === "file:") {
        const module = "node:fs/promises";
        const { readFile }: typeof import("node:fs/promises") = await import(/* @vite-ignore */ module);
        input = new Uint8Array(await readFile(url));
      }
      await init({ module_or_path: input });
    })().catch(error => { initialized = undefined; initializedSource = undefined; throw error; });
  }
  return initialized.then(() => undefined);
}

function sameWasmSource(left: WasmSource, right: WasmSource): boolean {
  const url = (value: WasmSource): URL | undefined => value instanceof URL
    ? value
    : typeof value === "string" ? new URL(value, import.meta.url) : undefined;
  const leftUrl = url(left);
  const rightUrl = url(right);
  if (leftUrl || rightUrl) return leftUrl?.href === rightUrl?.href;
  const bytes = (value: WasmSource): Uint8Array | undefined => value instanceof Uint8Array
    ? new Uint8Array(value.buffer, value.byteOffset, value.byteLength)
    : value instanceof ArrayBuffer ? new Uint8Array(value) : undefined;
  const leftBytes = bytes(left);
  const rightBytes = bytes(right);
  if (leftBytes || rightBytes) {
    return leftBytes !== undefined && rightBytes !== undefined
      && leftBytes.byteLength === rightBytes.byteLength
      && leftBytes.every((byte, index) => byte === rightBytes[index]);
  }
  return left === right;
}

function requireDependencyModule(raw: VnBook): void {
  const request = JSON.stringify({ version: 1, cursor: `commit:${"0".repeat(64)}`, limits: {
    locations: 1, graphs: 0, candidates: 1, references: 1, scans: 1,
  } });
  for (const query of [
    () => raw.dependencies(request),
    () => raw.dependenciesWithObjects(request, "[]", new Uint8Array()),
  ]) {
    try {
      query();
      throw new Error("The VN dependency feature probe unexpectedly succeeded before opening a session.");
    } catch (error) {
      if (error instanceof Error && /open the session first/i.test(error.message)) continue;
      throw new Error("The VN reader Wasm source must include the `vn-dependencies` feature.", { cause: error });
    }
  }
}

export interface VnReader {
  readonly artifactId: ArtifactId;
  inspect(): Promise<VnViewV1>;
  historyArtifacts(expected: CommitId): Promise<Readonly<Record<CommitId, ArtifactId>>>;
  /** Checked read-only dependency hints for this exact confirmed reader point. */
  dependencies(cursor: CommitId, limits: VnDependencyLimits): Promise<VnDependencyResult>;
  /** Queries first, then fetches candidate graph objects outside the narrative input queue. */
  prepareCandidates(cursor: CommitId, limits: VnDependencyLimits, signal: AbortSignal): Promise<VnCandidatePreparation>;
  /** Drops prepared optional program bytes after authorization/content scope changes. */
  clearPreparedCandidates(): void;
  advance(expected: CommitId, dialogue: NodeId): Promise<CommitId>;
  choose(expected: CommitId, choicePoint: ChoicePointId, options: readonly OptionId[]): Promise<CommitId>;
  checkout(commit: CommitId): Promise<void>;
  exportSave(): Promise<string>;
  restore(save: string): Promise<void>;
  /** Complete-pack readers can return the exact pack; manifest readers leave pack storage to the host. */
  pack(): Uint8Array;
  /** Reopens the durable cursor after a write conflict or failed confirmation. */
  reload(): Promise<void>;
  /** Retries the same pending storage request bytes without applying an interaction again. */
  retrySave(): Promise<boolean>;
  /** Rejects new calls and releases the Wasm session after any queued load settles. */
  close(): Promise<void>;
}

/** Opens a checked visual novel and creates or restores its shared native history session. */
export async function openVn(options: OpenVnOptions): Promise<VnReader> {
  const reader = await prepareVn(options);
  try {
    await reader.start();
    return reader;
  } catch (error) {
    await reader.close();
    throw error;
  }
}

/** Internal two-phase open for hosts that need to inspect an unconfirmed initial root. */
export async function prepareVn(options: OpenVnOptions): Promise<RuntimeVn> {
  await initializeVn(options.wasm);
  const history = options.historyPacks ?? [];
  if (history.length > 64 || history.reduce((sum, bytes) => sum + bytes.byteLength, 0) > 64 * 1024 * 1024) {
    throw new RangeError("Historical packs exceed 64 artifacts / 64 MiB");
  }
  const raw = options.pack !== undefined
    ? new VnBook(options.pack, options.execution)
    : VnBook.fromManifest(options.manifest, options.execution);
  try {
    requireDependencyModule(raw);
    for (const pack of history) raw.registerHistoryPack(pack);
    if (options.names) raw.loadNames(options.names);
    return new RuntimeVn(raw, options.storage, options.fetchChunk, options.loadArtifact, options.pack !== undefined);
  } catch (error) {
    raw.free();
    throw error;
  }
}

export class RuntimeVn implements VnReader {
  private readonly transport: RuntimeTransport;
  private readonly candidates: CandidateObjects;
  private readonly durable: boolean;
  private dependencyObjects = 0;
  private dependencyQueryCalls = 0;

  constructor(
    private readonly raw: VnBook & ReaderEngine,
    storage?: CacheStore,
    fetchChunk?: (id: string, signal?: AbortSignal) => Promise<Uint8Array>,
    loadArtifact?: ArtifactLoader,
    private readonly completePack = false,
  ) {
    this.durable = storage !== undefined;
    this.candidates = new CandidateObjects(artifact => this.transport.programLoader(artifact));
    this.transport = new RuntimeTransport(raw, storage, fetchChunk, loadArtifact, "The VN reader is closed.", this.candidates);
  }

  get artifactId(): ArtifactId {
    this.transport.assertOpen();
    return this.raw.artifact_id as ArtifactId;
  }

  start(existingOnly = false): Promise<void> {
    return this.run(() => {
      if (this.durable) {
        if (existingOnly) this.raw.resume();
        else this.raw.open();
      } else {
        this.raw.memory();
      }
    });
  }

  inspect(): Promise<VnViewV1> {
    return this.run(() => decodeVnView(this.raw.inspect()));
  }

  historyArtifacts(expected: CommitId): Promise<Readonly<Record<CommitId, ArtifactId>>> {
    return this.run(() => {
      const value: unknown = JSON.parse(this.raw.historyArtifacts(expected));
      if (typeof value !== "object" || value === null || Array.isArray(value)) throw new Error("Invalid history artifact provenance");
      const artifacts: Record<CommitId, ArtifactId> = {};
      for (const [id, artifact] of Object.entries(value)) {
        if (!/^commit:[0-9a-f]{64}$/.test(id) || typeof artifact !== "string" || !/^artifact:[0-9a-f]{64}$/.test(artifact)) {
          throw new Error("Invalid history artifact provenance");
        }
        artifacts[id as CommitId] = artifact as ArtifactId;
      }
      return artifacts;
    });
  }

  async dependencies(cursor: CommitId, limits: VnDependencyLimits): Promise<VnDependencyResult> {
    this.transport.assertOpen();
    const request: DependencyRequestV1 = { version: 1, cursor, limits };
    return this.queryDependencies(request, [], this.raw.artifact_id).then(value => value.dependencies);
  }

  async prepareCandidates(cursor: CommitId, limits: VnDependencyLimits, signal: AbortSignal): Promise<VnCandidatePreparation> {
    let dependencies = await this.dependencies(cursor, limits);
    const artifact = dependencies.artifact_id;
    const states = new Map<string, CandidateObjectStatus>();
    if (this.completePack) {
      for (const graph of dependencies.graphs) states.set(graph.chunk, "checked");
      return { status: dependencies.complete ? "complete" : "partial", dependencies,
        objects: [...states].map(([object, status]) => ({ object, status })) };
    }

    for (let round = 0; round < 16 && !signal.aborted; round++) {
      const graphIds = dependencyObjectIds(dependencies);
      const coldIds = dependencies.truncations.flatMap(item => item.kind === "unavailable" ? [item.chunk] : []);
      // Manifest graph listings are not proof that a bounded query read their bytes.
      // Report checked objects only after supplying those exact bytes to Wasm.
      const pending = coldIds.filter(id => !states.has(id));
      if (pending.length > 0) {
        const loaded = await this.candidates.prepareObjects(artifact, pending.slice(0, 16), signal);
        for (const object of loaded) states.set(object.object, object.status);
      }
      if (signal.aborted) {
        this.discardUnverifiedObjects(artifact, states);
        break;
      }

      const candidateIds = [...new Set([...graphIds, ...coldIds])];
      const queryIds = [
        ...candidateIds.filter(id => states.get(id) === "fetched"),
        ...candidateIds.filter(id => states.get(id) === "checked"),
      ].slice(0, 16);
      const bytes = this.candidates.candidateObjectBytes(artifact, queryIds);
      if (bytes.length === 0) break;

      let queried: VnDependencyResult;
      let invalidIds: readonly string[] = [];
      try {
        const result = await this.queryDependencies({ version: 1, cursor, limits }, bytes, artifact);
        queried = result.dependencies;
        invalidIds = result.invalidIds;
      } catch (error) {
        let currentCursor: string | undefined;
        try { currentCursor = await this.run(() => this.raw.cursor()); } catch { /* A closed reader cannot publish candidate readiness. */ }
        if (signal.aborted || currentCursor !== cursor || this.raw.artifact_id !== artifact) {
          this.discardUnverifiedObjects(artifact, states);
          return { status: "cancelled", dependencies, objects: [...states].map(([object, status]) => ({ object, status })) };
        }
        if (dependencyBudgetError(error)) {
          for (const item of bytes) if (states.get(item.object) === "fetched") {
            states.set(item.object, "budget");
            this.candidates.discardCandidateObjects(artifact, [item.object]);
          }
        }
        // A limit failure does not invalidate already checked immutable bytes.
        break;
      }
      if (signal.aborted) break;
      const stillCold = new Set(queried.truncations.flatMap(item => item.kind === "unavailable" ? [item.chunk] : []));
      const corrupt = new Set([...invalidIds, ...bytes.filter(item => stillCold.has(item.object)).map(item => item.object)]);
      for (const item of bytes) {
        if (corrupt.has(item.object)) {
          states.set(item.object, "corrupt");
          this.candidates.discardCandidateObjects(artifact, [item.object]);
        } else {
          states.set(item.object, "checked");
          this.candidates.markCandidateObjectsChecked(artifact, [item.object]);
        }
      }
      dependencies = queried;
      const newlyDiscovered = dependencies.truncations.some(item => item.kind === "unavailable" && !states.has(item.chunk));
      if (!newlyDiscovered) break;
    }

    const objects = [...states].map(([object, status]) => ({ object, status }));
    if (signal.aborted) {
      this.discardUnverifiedObjects(artifact, states);
      return { status: "cancelled", dependencies, objects: [...states].map(([object, status]) => ({ object, status })) };
    }
    return {
      status: dependencies.complete && objects.every(object => object.status === "checked") ? "complete" : "partial",
      dependencies,
      objects,
    };
  }

  clearPreparedCandidates(): void {
    this.candidates.clearPreparedObjects();
  }

  advance(expected: CommitId, dialogue: NodeId): Promise<CommitId> {
    return this.run(() => this.raw.advance(expected, dialogue) as CommitId);
  }

  choose(expected: CommitId, choicePoint: ChoicePointId, options: readonly OptionId[]): Promise<CommitId> {
    return this.run(() => this.raw.choose(expected, choicePoint, [...options]) as CommitId);
  }

  checkout(commit: CommitId): Promise<void> {
    return this.run(() => this.raw.checkout(commit));
  }

  exportSave(): Promise<string> {
    return this.run(() => this.raw.export());
  }

  restore(save: string): Promise<void> {
    return this.run(() => this.raw.restore(save));
  }

  pack(): Uint8Array {
    this.transport.assertOpen();
    return this.raw.pack();
  }

  reload(): Promise<void> {
    return this.transport.reload(() => this.durable ? this.raw.open() : this.raw.memory());
  }

  retrySave(): Promise<boolean> {
    return this.transport.retrySave();
  }

  close(): Promise<void> {
    return this.transport.close();
  }

  private run<T>(operation: () => T): Promise<T> {
    return this.transport.run(operation);
  }

  private queryDependencies(request: DependencyRequestV1, supplied: readonly CandidateObjectBytes[], artifact: string): Promise<{ dependencies: DependencyResultV1; invalidIds: readonly string[] }> {
    const objectCost = supplied.length;
    const callCost = supplied.length === 0 ? 1 : supplied.length + 2;
    if (supplied.length > 16 || this.dependencyObjects + objectCost > 128 || this.dependencyQueryCalls + callCost > 512) {
      return Promise.reject(new RangeError("VN dependency queries exceed the reader lifetime budget."));
    }
    this.dependencyObjects += objectCost;
    this.dependencyQueryCalls += callCost;
    return this.run(() => {
      const requestText = JSON.stringify(request);
      const baseValue: unknown = JSON.parse(this.raw.dependencies(requestText));
      const base = this.checkedDependencyResult(baseValue, request, artifact);
      const invalidIds: string[] = [];
      const validObjects: CandidateObjectBytes[] = [];
      for (const item of supplied) {
        try {
          const probe = candidateObjectBundle([item]);
          const result: unknown = JSON.parse(this.raw.dependenciesWithObjects(requestText, probe.descriptors, probe.bytes));
          this.checkedDependencyResult(result, request, artifact);
          validObjects.push(item);
        } catch (error) {
          if (dependencyBudgetError(error)) throw error;
          invalidIds.push(item.object);
        }
      }
      if (validObjects.length === 0) return { dependencies: base, invalidIds };

      const bundle = candidateObjectBundle(validObjects);
      const value: unknown = JSON.parse(this.raw.dependenciesWithObjects(requestText, bundle.descriptors, bundle.bytes));
      const dependencies = this.checkedDependencyResult(value, request, artifact);
      return { dependencies, invalidIds };
    });
  }

  private checkedDependencyResult(value: unknown, request: DependencyRequestV1, artifact: string): DependencyResultV1 {
    if (!validateDependencies({ request, result: value })) throw new Error("The runtime returned an invalid VN dependency result v1.");
    const checked = value as DependencyResultV1;
    if (checked.cursor !== request.cursor || checked.artifact_id !== this.raw.artifact_id || checked.artifact_id !== artifact
      || checked.limits.locations !== request.limits.locations || checked.limits.graphs !== request.limits.graphs
      || checked.limits.candidates !== request.limits.candidates || checked.limits.references !== request.limits.references
      || checked.limits.scans !== request.limits.scans) {
      throw new Error("The runtime returned VN dependencies for a different reader point or limit set.");
    }
    return checked;
  }

  private discardUnverifiedObjects(artifact: string, states: ReadonlyMap<string, CandidateObjectStatus>): void {
    this.candidates.discardCandidateObjects(artifact,
      [...states].flatMap(([object, status]) => status === "fetched" ? [object] : []));
  }
}

function candidateObjectBundle(objects: readonly CandidateObjectBytes[]): { descriptors: string; bytes: Uint8Array } {
  let total = 0;
  const descriptors = objects.map(item => {
    const offset = total;
    total += item.bytes.byteLength;
    if (!Number.isSafeInteger(total) || total > 64 * 1024 * 1024) {
      throw new RangeError("Temporary VN candidate objects exceed their combined byte budget.");
    }
    return { object: item.object, offset, byte_length: item.bytes.byteLength };
  });
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const item of objects) {
    bytes.set(item.bytes, offset);
    offset += item.bytes.byteLength;
  }
  return { descriptors: JSON.stringify(descriptors), bytes };
}

function dependencyBudgetError(error: unknown): boolean {
  return error instanceof RangeError || error instanceof Error && /^limit at /.test(error.message);
}

function dependencyObjectIds(result: DependencyResultV1): string[] {
  return [...new Set([
    ...result.graphs.map(graph => graph.chunk),
    ...result.truncations.flatMap(item => item.kind === "unavailable" ? [item.chunk] : []),
  ])];
}
