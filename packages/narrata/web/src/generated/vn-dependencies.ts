/* Generated from Rust VN dependencies v1 JSON Schema. Run task web:build. */

export type CommitId = string;
export type ArtifactId = string;
export type ContentKey = string;
export type ProviderId = string;
/**
 * What a content reference is for.
 */
export type DependencyContentRoleV1 =
  | "dialogue_line"
  | "dialogue_speaker"
  | "passage_title"
  | "passage_body"
  | "choice_label"
  | "choice_reason"
  | "choice_reply"
  | "ending_title"
  | "ending_body";
export type ObjectId = string;
export type MediaKind = "image" | "audio";
/**
 * How a media reference would be used by the scene node it came from.
 */
export type DependencyMediaRoleV1 =
  | {
      kind: "background";
    }
  | {
      id: string;
      kind: "character_layer";
    }
  | {
      kind: "music_play";
    }
  | {
      kind: "music_keep";
    }
  | {
      kind: "voice";
    };
/**
 * Why the result may be missing references. Each is a fact about this query: an unavailable
 * object may exist and become loadable later.
 */
export type DependencyTruncationV1 =
  | {
      kind: "locations";
      limit: number;
    }
  | {
      kind: "scans";
      limit: number;
    }
  | {
      kind: "candidates";
      limit: number;
    }
  | {
      kind: "references";
      limit: number;
    }
  | {
      graph: GraphRef;
      kind: "graphs";
      limit: number;
    }
  | {
      graph: GraphRef;
      kind: "call_depth";
      node: NodeId;
    }
  | {
      graph: GraphRef;
      kind: "automatic_cycle";
      node: NodeId;
    }
  | {
      chunk: ObjectId;
      error: Diagnostic;
      graph: GraphRef;
      kind: "unavailable";
    }
  | {
      error: Diagnostic;
      graph: GraphRef;
      kind: "unresolved";
      node?: NodeId | null;
    };
export type NodeId = string;
/**
 * What a result still did not evaluate. These never hide references.
 */
export type DependencyUnknownV1 = "conditions" | "assignments" | "call_arguments" | "host_proposals";

/**
 * The schema document: both halves of the exchange, so one file describes the whole boundary.
 */
export interface VnDependenciesV1 {
  request: DependencyRequestV1;
  result: DependencyResultV1;
}
/**
 * A projection request: the commit the caller believes is current, and the bounds to spend.
 */
export interface DependencyRequestV1 {
  cursor: CommitId;
  limits: DependencyLimitsV1;
  version: number;
}
/**
 * The five native bounds of one projection, as exchanged.
 */
export interface DependencyLimitsV1 {
  candidates: number;
  graphs: number;
  locations: number;
  references: number;
  scans: number;
}
/**
 * Potential additional dependencies of the next safe point, tied to the exact artifact and
 * commit they were computed from. A conservative union, not a prediction: the current View's
 * scene resources stay with the caller, and nothing here is bytes, a URL or a credential.
 */
export interface DependencyResultV1 {
  artifact_id: ArtifactId;
  /**
   * The bounded enumeration reached every safe point and ending: `truncations` is empty.
   */
  complete: boolean;
  content: DependencyContentV1[];
  cursor: CommitId;
  /**
   * Graphs of the current frames are not listed. Sorted, without repeats.
   */
  graphs: DependencyGraphV1[];
  limits: DependencyLimitsV1;
  media: DependencyMediaV1[];
  /**
   * Some continuation stops the music before its safe point. Stopping has no reference.
   */
  music_may_stop: boolean;
  /**
   * In discovery order.
   */
  truncations: DependencyTruncationV1[];
  /**
   * Sorted, without repeats.
   */
  unknowns: DependencyUnknownV1[];
  version: number;
  work: DependencyWorkV1;
}
export interface DependencyContentV1 {
  reference: ContentRef;
  role: DependencyContentRoleV1;
}
export interface ContentRef {
  key: ContentKey;
  provider: ProviderId;
}
/**
 * A graph a continuation may enter and the program chunk that holds it.
 */
export interface DependencyGraphV1 {
  chunk: ObjectId;
  graph: GraphRef;
}
/**
 * A graph of one package instance. Package aliases and graph names are composition
 * structure, so unlike node aliases they are part of the artifact identity.
 */
export interface GraphRef {
  graph: string;
  package: string;
}
export interface DependencyMediaV1 {
  reference: MediaRef;
  role: DependencyMediaRoleV1;
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
export interface Diagnostic {
  code: string;
  message: string;
  path: string;
}
/**
 * The work the query did, for hosts that budget or test it.
 */
export interface DependencyWorkV1 {
  candidates: number;
  graphs: number;
  locations: number;
  scans: number;
}
