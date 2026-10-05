/* Generated from the Rust BookView JSON Schema. Run npm run prepare:runtime. */

export type NodeId = string;
export type EdgeKind = "next" | "choice" | "condition" | "return" | "call";
export type ContentKey = string;
export type ProviderId = string;
export type NodeKind = "passage" | "branch" | "mutate" | "call" | "return";
/**
 * Exchange form of a value. Integers are decimal strings so that 64-bit values survive
 * JavaScript; `ref` values are resolved by the host's content provider.
 */
export type ViewScalar =
  | {
      type: "bool";
      value: boolean;
    }
  | {
      type: "int";
      value: string;
    }
  | {
      type: "text";
      value: string;
    }
  | {
      type: "ref";
      value: ContentRef;
    };
/**
 * A title is a reference; body and reply items are segments.
 */
export type Presented = Segment | ContentRef;
export type AnchorId = string;
export type Role = "title" | "body" | "reply";
export type ArtifactId = string;
export type CommitId = string;
export type ExecutionId = string;
export type ChoicePointId = string;
export type Interaction =
  | {
      /**
       * Arguments for option labels and disabled reasons.
       */
      args: {
        [k: string]: ViewScalar;
      };
      choice_point: ChoicePointId;
      graph: GraphRef;
      key?: string | null;
      kind: "choose";
      max: number;
      min: number;
      node: NodeId;
      /**
       * Visible options in order, proposed ones after the choice point's own; hidden ones
       * are left out.
       */
      options: OptionView[];
      /**
       * Whether the host may propose options and passages here.
       */
      proposals?: boolean;
    }
  | {
      body?: Segment | null;
      kind: "finished";
      outcome: string;
      title?: ContentRef | null;
    };
export type OptionId = string;
export type OutcomeKind = "local" | "branch";

/**
 * What the browser reader receives: the session view, the reading page of the current
 * passage, and the text-free graph analysis.
 */
export interface BookView {
  diagnostics: Diagnostic[];
  graphs: GraphAnalysis[];
  /**
   * The presentation of the step that entered the current passage (or ended the story),
   * followed by those of the local choices made in it since.
   */
  page: PresentationItem[];
  view: SessionView;
}
export interface Diagnostic {
  code: string;
  message: string;
  path: string;
}
export interface GraphAnalysis {
  entry: NodeId;
  exported: boolean;
  nodes: NodeAnalysis[];
  reference: GraphRef;
  title?: ContentRef | null;
}
export interface NodeAnalysis {
  /**
   * The graph a call runs.
   */
  callee?: GraphRef | null;
  edges: EdgeAnalysis[];
  id: NodeId;
  key?: string | null;
  kind: NodeKind;
  /**
   * The outcome a return reports.
   */
  outcome?: string | null;
  /**
   * A passage's title.
   */
  title?: ContentRef | null;
}
/**
 * A graph of one package instance. Package aliases and graph names are composition
 * structure, so unlike node aliases they are part of the artifact identity.
 */
export interface GraphRef {
  graph: string;
  package: string;
}
export interface EdgeAnalysis {
  /**
   * An option alias, an outcome name, or `true`/`false` for conditions.
   */
  key?: string | null;
  kind: EdgeKind;
  /**
   * An option label.
   */
  label?: ContentRef | null;
  target: NodeAddress;
}
export interface ContentRef {
  key: ContentKey;
  provider: ProviderId;
}
export interface NodeAddress {
  graph: string;
  node: NodeId;
  package: string;
}
/**
 * One item to present. `(execution, commit, occurrence)` is the presentation key that
 * generative content providers cache by.
 */
export interface PresentationItem {
  /**
   * The passage's named arguments when the item was presented.
   */
  args: {
    [k: string]: ViewScalar;
  };
  /**
   * The commit whose step presented the item.
   */
  commit: string;
  content: Presented;
  node: NodeId;
  /**
   * The item's position in that commit's presentation.
   */
  occurrence: number;
  role: Role;
}
export interface Segment {
  first?: AnchorId | null;
  last?: AnchorId | null;
  unit: ContentRef;
}
export interface SessionView {
  artifact_id: ArtifactId;
  cursor: CommitId;
  depth: number;
  execution: ExecutionId;
  frames: FrameView[];
  history: HistoryView[];
  interaction: Interaction;
  /**
   * What to show since the previous interaction, recomputed from the parent state and the
   * input rather than stored.
   */
  presentation: PresentationItem[];
  product: ProductView;
  shared: VariableView[];
}
export interface FrameView {
  at?: ChoicePointId | null;
  graph: GraphRef;
  instance: number;
  key?: string | null;
  locals: VariableView[];
  node: NodeId;
  parameters: VariableView[];
}
export interface VariableView {
  label?: ContentRef | null;
  name: string;
  value: ViewScalar;
}
export interface HistoryView {
  current: boolean;
  depth: number;
  finished: boolean;
  graph: GraphRef1;
  id: CommitId;
  instance: number;
  key?: string | null;
  node: NodeId;
  parent?: CommitId | null;
  /**
   * The waiting passage's title, or the ending's title.
   */
  title?: ContentRef | null;
}
/**
 * A graph of one package instance. Package aliases and graph names are composition
 * structure, so unlike node aliases they are part of the artifact identity.
 */
export interface GraphRef1 {
  graph: string;
  package: string;
}
export interface OptionView {
  enabled: boolean;
  id: OptionId;
  key?: string | null;
  label?: ContentRef | null;
  outcome: OutcomeKind;
  /**
   * Only for a disabled option that declares a reason.
   */
  reason?: ContentRef | null;
}
export interface ProductView {
  id: string;
  title?: ContentRef | null;
}
