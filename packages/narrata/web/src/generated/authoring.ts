/* Generated from Rust authoring interface JSON Schema. Run task web:build. */

export type AuthoredId = string;
export type ArtifactId = string;
export type CommitId = string;
export type ApplyV1 =
  | {
      kind: "migrated";
      migration_commit: CommitId;
      report: MigrationReportV1;
      source_commit: CommitId;
      target_artifact: ArtifactId;
    }
  | {
      actual_cursor?: CursorV1 | null;
      kind: "conflict";
    }
  | {
      kind: "plan_changed";
    }
  | {
      diagnostic: Diagnostic2;
      kind: "failed";
    };
export type ExternalEffectsV1 = "not_applicable";
export type NodeId = string;
/**
 * This interface was referenced by `undefined`'s JSON-Schema definition
 * via the `patternProperty` "^[ -~]{1,128}$".
 */
export type ChoicePointId = string;
/**
 * Only continuation positions can declare recovery; options cannot relocate a state.
 */
export type DeletedId = NodeId | ChoicePointId;
export type PathComparisonV1 =
  | {
      kind: "equivalent";
    }
  | {
      differing_fields: string[];
      input_id?: ObjectId | null;
      kind: "diverged";
      new_location?: LocationV1 | null;
      old_location?: LocationV1 | null;
      reason: DivergenceReason;
      source_commit: CommitId;
    }
  | {
      baseline?: CommitId | null;
      checked_through?: CommitId | null;
      kind: "unverified";
      reason: UnverifiedReason;
    };
export type ObjectId = string;
export type DivergenceReason = "initial_state" | "invalid_input" | "execution" | "semantic_state" | "control_trace";
export type UnverifiedReason =
  "missing_ancestor" | "missing_artifact" | "missing_descriptor" | "limit" | "baseline" | "trace_unavailable";
export type CursorRevision = string;
export type AssessmentV1 =
  | {
      kind: "exact";
      source: SourceV1;
    }
  | {
      kind: "automatic";
      plan: PlanV1;
    }
  | {
      kind: "needs_confirmation";
      plan: PlanV1;
      reason: ConfirmationReason;
    }
  | {
      kind: "old_version_only";
      reason: string;
      restart_available: boolean;
      source: SourceV1;
      target: ArtifactId;
    }
  | {
      digests: ObjectId[];
      kind: "needs_objects";
      retryable: boolean;
    }
  | {
      diagnostic: Diagnostic2;
      kind: "invalid_save";
      restart_available: boolean;
    };
export type MigrationId = string;
export type ConfirmationReason = "recovery" | "divergence" | "unverified";
export type EvolutionPlanningV1 =
  | {
      artifact_id: ArtifactId;
      kind: "exact";
    }
  | {
      descriptor: EvolutionDescriptorV1;
      kind: "descriptor";
      migration_id: MigrationId;
    };
export type ExecutionId = string;
export type MaintenanceRequest =
  | {
      after?: RootKey | null;
      kind: "roots";
      limit: number;
    }
  | {
      after?: PinCursor | null;
      kind: "pins";
      limit: number;
      now: number;
    }
  | {
      kind: "delete_roots";
      roots: ObservedRoot[];
    }
  | {
      kind: "integrity";
    }
  | {
      dry_run: boolean;
      grace_seconds: number;
      kind: "collect";
      now: number;
    };
export type MaintenanceResult =
  | {
      items: ObservedRoot[];
      kind: "roots";
      more: boolean;
    }
  | {
      items: PinView[];
      kind: "pins";
      more: boolean;
    }
  | {
      kind: "deleted";
    }
  | {
      issues: IntegrityView[];
      kind: "integrity";
    }
  | {
      dry_run: boolean;
      kind: "collected";
      reachable: string;
      removed: CollectedKind[];
      retained: CollectedKind[];
      roots: string;
    };
export type R1ArtifactId = string;
/**
 * A state value. `text` is data that rules compare (decision 2); translatable names travel as
 * `ref`, which only supports equality.
 */
export type Scalar = boolean | number | string | ContentRef;
export type ContentKey = string;
export type ProviderId = string;
export type AnchorId = string;
export type OptionId = string;
export type Encoding = "cbor" | "json";
export type Digest = string;
/**
 * Lexical role order is the descriptor order, including ADR 0023 evolution metadata.
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
export type VisitMissingV1 =
  | {
      id: ObjectId;
      kind: "object";
    }
  | {
      at: CommitId;
      kind: "artifact";
    }
  | {
      at: CommitId;
      kind: "descriptor";
    }
  | {
      kind: "limit";
    };
export type TargetStatus = "local" | "passage" | "control" | "unresolved";
export type RelationKind = "unit" | "node" | "edge" | "entry" | "ending" | "route" | "cluster" | "first";
export type RelationRow =
  | {
      artifact_id: ArtifactId;
      content_key: ContentKey;
      projection_version: Version1;
      provider: ProviderId;
      row_type: "unit";
      unit_id: Digest;
    }
  | {
      artifact_id: ArtifactId;
      cluster_id: ClusterId;
      first_anchor?: AnchorId | null;
      graph: string;
      kind: number;
      last_anchor?: AnchorId | null;
      node_id: NodeId;
      package: string;
      projection_version: Version1;
      row_type: "node";
      unit_id?: Digest | null;
    }
  | {
      artifact_id: ArtifactId;
      choice_point?: ChoicePointId | null;
      edge_kind: number;
      edge_no: number;
      option?: OptionId | null;
      projection_version: Version1;
      row_type: "edge";
      source_node: NodeId;
      source_unit: Digest;
      target_node: NodeId;
      target_unit: Digest;
    }
  | {
      artifact_id: ArtifactId;
      node_id: NodeId;
      projection_version: Version1;
      row_type: "entry";
    }
  | {
      artifact_id: ArtifactId;
      class: EndingClass;
      node_id: NodeId;
      projection_version: Version1;
      reachable: boolean;
      row_type: "ending";
    }
  | {
      artifact_id: ArtifactId;
      cluster_id: ClusterId;
      ending_node: NodeId;
      projection_version: Version1;
      row_type: "route";
    }
  | {
      artifact_id: ArtifactId;
      cluster_id: ClusterId;
      graph?: string | null;
      package?: string | null;
      projection_version: Version1;
      row_type: "cluster";
      unit_id?: Digest | null;
    }
  | {
      artifact_id: ArtifactId;
      distance: number;
      node_id: NodeId;
      projection_version: Version1;
      row_type: "first";
      unit_id: Digest;
    };
export type EndingClass = "unspecified" | "success" | "failure" | "neutral";
export type AuthorRelationQuery =
  | {
      kind: "content";
      reference: ContentRef;
    }
  | {
      kind: "node";
      node: NodeId;
    }
  | {
      cluster: ClusterId;
      kind: "cluster";
    }
  | {
      kind: "ending";
      node: NodeId;
    }
  | {
      ending: NodeId;
      kind: "route";
    }
  | {
      kind: "first";
      unit: Digest;
    }
  | {
      kind: "forward";
      unit: Digest;
    }
  | {
      kind: "reverse";
      unit: Digest;
    };
export type VisitActionV1 =
  | {
      kind: "visit";
      static_node: boolean;
    }
  | {
      kind: "segment";
      role?: Role | null;
      segment: Segment;
    }
  | {
      branch: boolean;
      choice_point: ChoicePointId;
      kind: "selected";
      option: OptionId;
    }
  | {
      kind: "branch";
      target: NodeId;
    }
  | {
      graph: GraphRef;
      instance: number;
      kind: "call";
      node: NodeId;
    }
  | {
      continuation?: NodeId | null;
      kind: "return";
      outcome: string;
    }
  | {
      kind: "finished";
      outcome: string;
    };
export type Role = "title" | "body" | "reply";

/**
 * Every exchanged shape, so one generated declaration file shares its definitions.
 */
export interface AuthoringInterface {
  authored_id: AuthoredId;
  checked_save: CheckedSave;
  comparison_budget: ComparisonBudgetV1;
  evolution_apply: ApplyV1;
  evolution_assessment: AssessmentV1;
  evolution_declarations: RecoveryDeclarationsV1;
  evolution_planning: EvolutionPlanningV1;
  import_selection: ImportSelection;
  lock: CompositionLock;
  maintenance_limits: MaintenanceLimits;
  maintenance_request: MaintenanceRequest;
  maintenance_result: MaintenanceResult;
  manifest: ProjectManifest;
  minted_ids: MintedIds;
  object_reference: ObjectReference;
  outline: ContentOutline;
  package: PackageSource;
  preview_report: PreviewReport;
  publication: PublicationDescription;
  publish_report: PublishReport;
  reader_map: ReaderMapV1;
  record: DraftRecord;
  record_indexes: RecordIndexes;
  record_report: RecordReport;
  relation_index: RelationIndex;
  relation_page: AuthorRelationPage;
  relation_part: RelationPart;
  relation_query: AuthorRelationQuery;
  visits: VisitReportV1;
}
export interface CheckedSave {
  artifact: ArtifactId;
  cursor: CommitId;
}
/**
 * Per-assessment limits, also bound to the confirmation in this driver. Values may only
 * tighten the core's limits. JSON integers fit exactly in JavaScript.
 */
export interface ComparisonBudgetV1 {
  automatic_steps: number;
  object_bytes: number;
  transitions: number;
}
export interface MigrationReportV1 {
  external_effects: ExternalEffectsV1;
  from_artifact: ArtifactId;
  losses: RecoveryLossV1[];
  migration_commit: CommitId;
  path: PathComparisonV1;
  source_commit: CommitId;
  to_artifact: ArtifactId;
}
export interface RecoveryLossV1 {
  cleared_finished?: NodeId | null;
  discarded_frames: DiscardedFrameV1[];
  reset_local_keys: string[];
  retained_shared_keys: string[];
  target: RecoveryTarget;
  tombstones: DeletedId[];
}
export interface DiscardedFrameV1 {
  instance: number;
  local_keys: string[];
  node: NodeId;
  overlay_ids: AuthoredId[];
  parameter_keys: string[];
  pending_choice?: ChoicePointId | null;
}
export interface RecoveryTarget {
  choice_point: ChoicePointId;
  node: NodeId;
}
export interface LocationV1 {
  choice_point?: ChoicePointId | null;
  graph: GraphRef;
  instance: number;
  node: NodeId;
}
/**
 * A graph of one package instance. Package aliases and graph names are composition
 * structure, so unlike node aliases they are part of the artifact identity.
 */
export interface GraphRef {
  graph: string;
  package: string;
}
export interface CursorV1 {
  commit: CommitId;
  revision: CursorRevision;
}
export interface Diagnostic2 {
  code: string;
  message: string;
  path: string;
}
export interface SourceV1 {
  artifact_id: ArtifactId;
  source_commit: CommitId;
}
export interface PlanV1 {
  expected_cursor_revision: CursorRevision;
  migration_commit: CommitId;
  migration_ids: MigrationId[];
  report: MigrationReportV1;
  report_digest: ObjectId;
  source_commit: CommitId;
  target_artifact: ArtifactId;
}
export interface RecoveryDeclarationsV1 {
  format_version: number;
  from_artifact: ArtifactId;
  /**
   * @maxItems 4096
   */
  recoveries: Recovery[];
}
export interface Recovery {
  deleted: DeletedId;
  target: RecoveryTarget;
}
export interface EvolutionDescriptorV1 {
  format_version: number;
  from_artifact: ArtifactId;
  /**
   * @maxItems 4096
   */
  recoveries: Recovery[];
  to_artifact: ArtifactId;
}
export interface ImportSelection {
  artifact: ArtifactId;
  cursor: CommitId;
  execution: ExecutionId;
  expected_revision?: string | null;
  now: number;
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
export interface MaintenanceLimits {
  bytes: number;
  /**
   * Cumulative encoded reply receipt/acceptance threshold. One conforming bounded reply
   * may arrive before rejection, before maintenance/cache acceptance and Wasm transfer.
   * This does not bound host allocation or I/O before the host returns its reply.
   */
  load_bytes: number;
  /**
   * Host load calls, including retries on a cold cache.
   */
  loads: number;
  object_reads: number;
  page_size: number;
  pages: number;
  work: number;
}
export interface RootKey {
  name: string;
  /**
   * Existing RefNamespace code: save=0, branch=1, bookmark=2, active=3, temporary=4.
   */
  namespace: number;
  owner: string;
}
export interface PinCursor {
  object: ObjectId;
  owner: string;
}
export interface ObservedRoot {
  commit: CommitId;
  key: RootKey;
  revision: string;
}
export interface PinView {
  expired: boolean;
  expires_at?: string | null;
  object: ObjectId;
  owner: string;
}
export interface IntegrityView {
  diagnostic: string;
  object: ObjectId;
}
export interface CollectedKind {
  bytes: string;
  kind: number;
  objects: string;
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
export interface ReaderMapV1 {
  artifact_id: ArtifactId;
  edges: ReaderEdgeV1[];
  endings: NodeId[];
  format_version: number;
  nodes: ReaderNodeV1[];
  projection_version: Version1;
  sources: ReaderMapSourceV1[];
  units: ReaderUnitV1[];
}
export interface ReaderEdgeV1 {
  choice_point?: ChoicePointId | null;
  option?: OptionId | null;
  source: NodeId;
  target: NodeId;
}
export interface ReaderNodeV1 {
  node: NodeId;
  unit_id?: Digest | null;
  x: number;
  y: number;
}
export interface ReaderMapSourceV1 {
  complete: boolean;
  cursor: CommitId;
  frontier: VisitFrontierV1[];
  missing: VisitMissingV1[];
}
/**
 * The only information about an unvisited choice target that may leave the map core.
 */
export interface VisitFrontierV1 {
  enabled: boolean;
  label?: ContentRef | null;
  option: OptionId;
}
export interface ReaderUnitV1 {
  content: ContentRef;
  segments: Segment[];
  unit_id: Digest;
  x: number;
  y: number;
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
export interface RelationIndex {
  artifact_id: ArtifactId;
  counts: {
    cluster?: number;
    edge?: number;
    ending?: number;
    entry?: number;
    first?: number;
    node?: number;
    route?: number;
    unit?: number;
  };
  parts: PartReference[];
  projection_version: Version1;
  row_count: number;
}
export interface PartReference {
  bytes: number;
  kind: RelationKind;
  object_id: Digest;
  ordinal: number;
  row_count: number;
}
export interface AuthorRelationPage {
  artifact_id: ArtifactId;
  next_offset?: number | null;
  projection_version: Version1;
  rows: RelationRow[];
}
export interface RelationPart {
  artifact_id: ArtifactId;
  kind: RelationKind;
  projection_version: Version1;
  rows: RelationRow[];
}
export interface VisitReportV1 {
  format_version: number;
  sources: VisitSourceV1[];
}
export interface VisitSourceV1 {
  /**
   * Established by the checked cursor header, even when ancestor replay cannot start.
   * None means that header itself was unavailable; no supplied projection can fill it in.
   */
  artifact_id?: ArtifactId | null;
  complete: boolean;
  cursor: CommitId;
  frontier: VisitFrontierV1[];
  missing: VisitMissingV1[];
  segments: ArtifactVisitsV1[];
}
/**
 * Segments remain separate even if a later migration returns to the same artifact.
 */
export interface ArtifactVisitsV1 {
  artifact_id: ArtifactId;
  events: VisitEventV1[];
}
export interface VisitEventV1 {
  action: VisitActionV1;
  commit: CommitId;
  graph: GraphRef;
  instance: number;
  node: NodeId;
}
