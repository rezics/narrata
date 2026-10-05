/* Generated from the Rust content JSON Schema; checked by content contract tests. */

export type ProviderId = string;
export type AnchorId = string;
/**
 * This interface was referenced by `undefined`'s JSON-Schema definition
 * via the `patternProperty` "^[ -~]{1,128}$".
 */
export type ChoicePointId = string;

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
