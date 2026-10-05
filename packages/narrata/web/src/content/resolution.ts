/* Generated from the Rust content JSON Schema; checked by content contract tests. */

export type Resolution =
  | {
      payload: Payload;
      revision: string;
      status: "ok";
    }
  | {
      status: "unavailable";
    }
  | {
      reason: string;
      status: "incompatible";
    };
export type Payload =
  | {
      text: string;
    }
  | {
      blocks: TextBlock[];
    };
export type AnchorId = string;
export type ArrayOf_Resolution = Resolution[];

export interface TextBlock {
  id: AnchorId;
  text: string;
}
