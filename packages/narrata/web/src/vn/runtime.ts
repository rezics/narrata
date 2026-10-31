import { VnBook } from "../reader-wasm/narrata_nodes_wasm.js";
import { RuntimeTransport, type ReaderEngine } from "../transport.js";
import type { CacheStore } from "../storage.js";
import type { ArtifactLoader } from "../artifacts.js";
import { initialize, type WasmSource } from "../runtime.js";
import validate from "../generated/validate-vn-view.js";
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
  | { manifest: Uint8Array; fetchChunk: (id: string) => Promise<Uint8Array>; names?: Uint8Array; pack?: never }
  | { pack: Uint8Array; manifest?: never; fetchChunk?: never; names?: never }
);

export interface VnReader {
  readonly artifactId: ArtifactId;
  inspect(): Promise<VnViewV1>;
  historyArtifacts(expected: CommitId): Promise<Readonly<Record<CommitId, ArtifactId>>>;
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
  await initialize(options.wasm);
  const history = options.historyPacks ?? [];
  if (history.length > 64 || history.reduce((sum, bytes) => sum + bytes.byteLength, 0) > 64 * 1024 * 1024) {
    throw new RangeError("Historical packs exceed 64 artifacts / 64 MiB");
  }
  const raw = options.pack !== undefined
    ? new VnBook(options.pack, options.execution)
    : VnBook.fromManifest(options.manifest, options.execution);
  try {
    for (const pack of history) raw.registerHistoryPack(pack);
    if (options.names) raw.loadNames(options.names);
    return new RuntimeVn(raw, options.storage, options.fetchChunk, options.loadArtifact);
  } catch (error) {
    raw.free();
    throw error;
  }
}

export class RuntimeVn implements VnReader {
  private readonly transport: RuntimeTransport;
  private readonly durable: boolean;

  constructor(
    private readonly raw: VnBook & ReaderEngine,
    storage?: CacheStore,
    fetchChunk?: (id: string) => Promise<Uint8Array>,
    loadArtifact?: ArtifactLoader,
  ) {
    this.durable = storage !== undefined;
    this.transport = new RuntimeTransport(raw, storage, fetchChunk, loadArtifact, "The VN reader is closed.");
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
}
