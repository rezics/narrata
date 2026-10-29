/* Generated from Rust authoring interface JSON Schema. Run task web:build. */

export type AuthoredId = string;
export type ArtifactId = string;
export type R1ArtifactId = string;
/**
 * A state value. `text` is data that rules compare (decision 2); translatable names travel as
 * `ref`, which only supports equality.
 */
export type Scalar = boolean | number | string | ContentRef;
export type ContentKey = string;
export type ProviderId = string;
export type AnchorId = string;
/**
 * This interface was referenced by `undefined`'s JSON-Schema definition
 * via the `patternProperty` "^[ -~]{1,128}$".
 */
export type ChoicePointId = string;
export type NodeId = string;
export type OptionId = string;
export type Encoding = "cbor" | "json";
export type Digest = string;
/**
 * Lexical role order is the descriptor order. Evolution is reserved for ADR 0023;
 * this publisher neither creates nor accepts unchecked evolution descriptors.
 */
export type ObjectRole =
  | "chunk"
  | "evolution"
  | "graph_index"
  | "graph_tile"
  | "labels"
  | "manifest"
  | "names"
  | "relation_index"
  | "relation_part"
  | "summary"
  | "tombstones";
export type ScalarType = "bool" | "int" | "text" | "ref";
export type Severity = "error" | "warning";
export type ObjectId = string;
export type DraftRecord =
  | {
      format_version: DraftVersion;
      kind: "project";
      payload: ProjectManifest;
    }
  | {
      format_version: DraftVersion;
      kind: "package";
      package: string;
      payload: PackageSource;
    }
  | {
      format_version: DraftVersion;
      graph: string;
      kind: "graph";
      package: string;
      payload: GraphSource;
    }
  | {
      choice_order?: ChoicePointId[] | null;
      format_version: DraftVersion;
      graph: string;
      key: string;
      kind: "node";
      package: string;
      payload: NodeSource;
    }
  | {
      format_version: DraftVersion;
      graph: string;
      kind: "choice_point";
      node: NodeId;
      package: string;
      payload: ChoicePointSource;
    }
  | {
      format_version: DraftVersion;
      kind: "tombstone";
      package: string;
      payload: AuthoredId;
    };
export type DraftVersion = 1;
export type Scope = "parameter" | "local" | "shared";
export type Expr =
  | {
      kind: "literal";
      value: Scalar;
    }
  | {
      kind: "read";
      name: string;
      scope: Scope;
    }
  | {
      kind: "not";
      value: Expr;
    }
  | {
      kind: "binary";
      left: Expr;
      op: BinaryOp;
      right: Expr;
    };
export type BinaryOp = "eq" | "ne" | "lt" | "le" | "gt" | "ge" | "add" | "sub" | "and" | "or";
export type OutcomeSource =
  | {
      kind: "local";
      rejoin?: AnchorId | null;
      reply?: Segment | null;
    }
  | {
      kind: "branch";
      target: string;
    };
export type Version1 = 1;
export type ClusterId = string;
/**
 * Compiled candidates can be retained and inspected even when their outlines prevent
 * switching. Topology warnings do not prevent switching; missing/defective outlines do.
 */
export type PublicationStatus = "compiled" | "switchable";
export type TargetStatus = "local" | "passage" | "control" | "unresolved";

/**
 * Every exchanged shape, so one generated declaration file shares its definitions.
 */
export interface AuthoringInterface {
  authored_id: AuthoredId;
  lock: CompositionLock;
  manifest: ProjectManifest;
  minted_ids: MintedIds;
  object_reference: ObjectReference;
  outline: ContentOutline;
  package: PackageSource;
  preview_report: PreviewReport;
  publication: PublicationDescription;
  publish_report: PublishReport;
  record: DraftRecord;
  record_indexes: RecordIndexes;
  record_report: RecordReport;
}
/**
 * Written next to the project manifest as `project.lock.json`.
 */
export interface CompositionLock {
  artifact_id: ArtifactId;
  format_version: number;
  node_types: {
    [k: string]: string;
  };
  packages: {
    [k: string]: PackageLock;
  };
}
export interface PackageLock {
  /**
   * `digest_bytes("narrata.nodes.package-source", 1, canonical JSON)`, hexadecimal.
   */
  digest: string;
  id: string;
  version: string;
}
/**
 * Resolved by tooling, never by the deterministic runtime.
 */
export interface ProjectManifest {
  format_version: number;
  /**
   * The R1 artifact this work was migrated from; recorded in the name table so that R1
   * saves can only migrate onto it (ADR 0013 §10).
   */
  migrated_from_r1?: R1ArtifactId | null;
  /**
   * Package instance alias to a path relative to the manifest.
   */
  packages: {
    [k: string]: string;
  };
  product: ProductSource;
}
export interface ProductSource {
  arguments?: {
    [k: string]: Scalar;
  };
  bindings?: Binding[];
  /**
   * How each outcome of the entry graph is shown when the story ends.
   */
  endings?: {
    [k: string]: EndingSource;
  };
  entry: GraphRef;
  id: string;
  shared?: {
    [k: string]: Scalar;
  };
  /**
   * Display names of shared variables.
   */
  shared_labels?: {
    [k: string]: ContentRef;
  };
  title?: ContentRef | null;
}
export interface ContentRef {
  key: ContentKey;
  provider: ProviderId;
}
export interface Binding {
  from: ImportRef;
  to: GraphRef;
}
export interface ImportRef {
  graph: string;
  package: string;
  port: string;
}
/**
 * A graph of one package instance. Package aliases and graph names are composition
 * structure, so unlike node aliases they are part of the artifact identity.
 */
export interface GraphRef {
  graph: string;
  package: string;
}
export interface EndingSource {
  body?: Segment | null;
  title?: ContentRef | null;
}
export interface Segment {
  first?: AnchorId | null;
  last?: AnchorId | null;
  unit: ContentRef;
}
/**
 * Newly minted IDs; none of them occurred in the minter's occupied set.
 */
export interface MintedIds {
  choice_points: ChoicePointId[];
  nodes: NodeId[];
  options: OptionId[];
}
export interface ObjectReference {
  bytes: number;
  encoding: Encoding;
  object_id: Digest;
  role: ObjectRole;
}
/**
 * A content provider's outline (ADR 0013 §4): block order and choice-point markers of each
 * content unit, without text. It lets `compose` check placement order without the documents.
 */
export interface ContentOutline {
  format_version: number;
  provider: ProviderId;
  units: {
    [k: string]: UnitOutline;
  };
}
/**
 * This interface was referenced by `undefined`'s JSON-Schema definition
 * via the `patternProperty` "^[^\u0000-\u001f\u007f-\u009f]+$".
 */
export interface UnitOutline {
  /**
   * Every block anchor in document order, markers included.
   */
  blocks: AnchorId[];
  /**
   * Marker blocks and the choice point each one stands for.
   */
  markers?: {
    [k: string]: ChoicePointId;
  };
}
export interface PackageSource {
  exports: string[];
  graphs: {
    [k: string]: GraphSource;
  };
  id: string;
  /**
   * Deleted IDs; they are never reused, and the list only grows.
   */
  tombstones?: AuthoredId[];
  version: string;
}
export interface GraphSource {
  /**
   * Alias of the entry node.
   */
  entry: string;
  imports?: {
    [k: string]: Signature;
  };
  locals?: {
    [k: string]: Scalar;
  };
  /**
   * Keyed by node alias.
   */
  nodes: {
    [k: string]: NodeSource;
  };
  outcomes: string[];
  parameters?: {
    [k: string]: ScalarType;
  };
  shared?: {
    [k: string]: ScalarType;
  };
  title?: ContentRef | null;
}
/**
 * Parameters and outcomes; checked signatures keep outcomes sorted and unique.
 */
export interface Signature {
  outcomes: string[];
  parameters?: {
    [k: string]: ScalarType;
  };
}
export interface NodeSource {
  data: unknown;
  /**
   * Minted by `narrata-book ids`; compile rejects a missing ID.
   */
  id?: NodeId | null;
  type_id: string;
}
/**
 * Preview compilation of records into program objects. A compiled preview is never a
 * publication: switching requires `publish` with outlines.
 */
export interface PreviewReport {
  artifact_id?: ArtifactId | null;
  compared: boolean;
  complete: boolean;
  diagnostics: Diagnostic[];
  lock?: CompositionLock | null;
  /**
   * Program chunks and the tombstone set, retrievable one at a time.
   */
  objects: ObjectId[];
  /**
   * Tombstone records the host persists atomically with the edit before publishing.
   */
  tombstones: DraftRecord[];
}
export interface Diagnostic {
  anchor?: AnchorId | null;
  choice_point?: ChoicePointId | null;
  code: string;
  message: string;
  node?: NodeId | null;
  option?: OptionId | null;
  pointer: string;
  record?: string | null;
  related: Location[];
  severity: Severity;
  source_path?: string | null;
  unit?: ContentRef | null;
}
export interface Location {
  anchor?: AnchorId | null;
  choice_point?: ChoicePointId | null;
  node?: NodeId | null;
  option?: OptionId | null;
  pointer: string;
  record?: string | null;
  source_path?: string | null;
  unit?: ContentRef | null;
}
export interface ChoicePointSource {
  id?: ChoicePointId | null;
  key: string;
  max?: number;
  min?: number;
  options: OptionSource[];
  /**
   * The body block after which the choice point appears; only the last one may omit it.
   */
  placement?: AnchorId | null;
  /**
   * Whether the host may propose options and passages here at run time (ADR 0013 §5).
   */
  proposals?: boolean;
}
export interface OptionSource {
  effects?: Assignment[];
  enabled_if?: Expr | null;
  id?: OptionId | null;
  key: string;
  label?: ContentRef | null;
  outcome: OutcomeSource;
  /**
   * Shown when the option is visible but disabled.
   */
  reason?: ContentRef | null;
  visible_if?: Expr | null;
}
export interface Assignment {
  target: Variable;
  value: Expr;
}
export interface Variable {
  name: string;
  scope: Scope;
}
export interface PublicationDescription {
  artifact_id: ArtifactId;
  /**
   * ADR 0023's full MigrationId digest, addressed by the evolution role.
   */
  evolution?: Digest | null;
  format_version: Version1;
  graph: GraphFiles;
  names?: Digest | null;
  objects: ObjectReference[];
  relation_index: Digest;
  summary: Digest;
}
/**
 * Bootstrap references for object-id-named files in the companion `.graph/` directory.
 */
export interface GraphFiles {
  artifact_id: ArtifactId;
  format_version: number;
  index_object_id: string;
  labels: LabelObject[];
}
export interface LabelObject {
  cluster: ClusterId;
  object_id: string;
}
export interface PublishReport {
  candidate_artifact_id?: ArtifactId | null;
  compared: boolean;
  complete: boolean;
  diagnostics: Diagnostic[];
  lock?: CompositionLock | null;
  previous_artifact_id?: ArtifactId | null;
  publication_id?: Digest | null;
  /**
   * `compiled` candidates can be retained and inspected; only `switchable` ones pass the
   * outline policy for becoming the work's current publication.
   */
  status?: PublicationStatus | null;
  tombstones: DraftRecord[];
}
/**
 * The derived index rows of one record, replaced together with its payload.
 */
export interface RecordIndexes {
  options: OptionIndex[];
  ownership: OwnershipIndex;
}
export interface OptionIndex {
  choice_point: ChoicePointId;
  graph: string;
  node: NodeId;
  option: OptionId;
  ordinal: number;
  outcome: string;
  package: string;
  placement?: AnchorId | null;
  record: string;
  rejoin?: AnchorId | null;
  reply?: Segment | null;
  source_unit?: ContentRef | null;
  tags: ContentRef[];
  target_node?: NodeId | null;
  target_status: TargetStatus;
  target_unit?: ContentRef | null;
}
export interface OwnershipIndex {
  graph?: string | null;
  kind: string;
  node?: NodeId | null;
  package?: string | null;
  record: string;
}
/**
 * A checked record set, a single checked record or a split source. `records` counts the
 * records available by index; it is zero whenever an error prevented a checked value.
 */
export interface RecordReport {
  complete: boolean;
  diagnostics: Diagnostic[];
  /**
   * IDs this operation minted, in allocation order.
   */
  minted: AuthoredId[];
  records: number;
}
