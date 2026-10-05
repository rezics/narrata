/* Generated from the Rust content JSON Schema; checked by content contract tests. */

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
export type ContentKey = string;
export type ProviderId = string;
export type Content = Segment | ContentRef;
export type AnchorId = string;

export interface ResolveRequest {
  context: ResolveContext;
  items: ResolveItem[];
}
export interface ResolveContext {
  /**
   * Preferred languages, most preferred first.
   */
  languages?: string[];
  /**
   * The host's selected translation or version; opaque to Narrata and ignored locally.
   */
  realization?: string | null;
  /**
   * The host's reader identity; opaque to Narrata and ignored locally.
   */
  viewer?: string | null;
}
export interface ResolveItem {
  /**
   * The values `{name}` placeholders take; `ref` values are resolved to their text.
   */
  args?: {
    [k: string]: ViewScalar;
  };
  content: Content;
}
export interface ContentRef {
  key: ContentKey;
  provider: ProviderId;
}
export interface Segment {
  first?: AnchorId | null;
  last?: AnchorId | null;
  unit: ContentRef;
}
