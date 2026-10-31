/* Generated from Rust VN View v1 JSON Schema. Run task web:build. */

export type ArtifactId = string;
export type CommitId = string;
export type ExecutionId = string;
export type ChoicePointId = string;
export type ContentKey = string;
export type ProviderId = string;
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
export type NodeId = string;
/**
 * The closed interaction vocabulary for VN View version 1.
 */
export type VnInteractionV1 =
  | {
      dialogue: NodeId;
      graph: GraphRef;
      kind: "advance";
      line: Segment;
      speaker?: ContentRef | null;
      voice?: MediaRef | null;
    }
  | {
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
      options: OptionView[];
      proposals?: boolean;
    }
  | {
      body?: Segment | null;
      kind: "finished";
      outcome: string;
      title?: ContentRef | null;
    };
export type AnchorId = string;
export type MediaKind = "image" | "audio";
export type OptionId = string;
export type OutcomeKind = "local" | "branch";
/**
 * Music intent performed by a scene transition. `Keep` carries the existing logical track.
 */
export type MusicAction =
  | {
      kind: "keep";
    }
  | {
      kind: "play";
      track: MediaRef;
    }
  | {
      kind: "stop";
    };
/**
 * A title is a reference; body and reply items are segments.
 */
export type Presented = Segment | ContentRef;
export type Role = "title" | "body" | "reply";

/**
 * The independently versioned, text-free view exchanged by the installed VN reader.
 * The wire version is checked by Serde and constrained to version 1 in the generated schema.
 */
export interface VnViewV1 {
  version: number;
  view: VnSessionViewV1;
}
/**
 * The reader-safe VN view. Unlike the shared internal session view, the scene is required.
 */
export interface VnSessionViewV1 {
  artifact_id: ArtifactId;
  cursor: CommitId;
  depth: number;
  execution: ExecutionId;
  frames: FrameView[];
  history: HistoryView[];
  interaction: VnInteractionV1;
  music_action?: MusicAction | null;
  presentation: PresentationItem[];
  product: ProductView;
  scene: SceneTarget;
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
/**
 * A graph of one package instance. Package aliases and graph names are composition
 * structure, so unlike node aliases they are part of the artifact identity.
 */
export interface GraphRef {
  graph: string;
  package: string;
}
export interface VariableView {
  label?: ContentRef | null;
  name: string;
  value: ViewScalar;
}
export interface ContentRef {
  key: ContentKey;
  provider: ProviderId;
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
export interface Segment {
  first?: AnchorId | null;
  last?: AnchorId | null;
  unit: ContentRef;
}
/**
 * A content-owned media identity. Narrata stores the provider's immutable key and revision,
 * never a URL, credential, decoded byte buffer or player handle.
 */
export interface MediaRef {
  key: string;
  kind: MediaKind;
  provider: string;
  revision: string;
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
export interface ProductView {
  id: string;
  title?: ContentRef | null;
}
/**
 * The entire recoverable visual/audio target for a VN scene safe point.
 */
export interface SceneTarget {
  background?: MediaRef | null;
  layers: CharacterLayer[];
  music?: MediaRef | null;
}
/**
 * A stable character layer in a complete visual scene target.
 */
export interface CharacterLayer {
  asset: MediaRef;
  id: string;
  order: number;
}
