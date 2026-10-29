import type { NativeDocument } from "../content/native.js";

/** A host-native document model, deliberately separate from Narrata's standard payload. */
export type SampleNativeValue =
  | { readonly kind: "paragraph"; readonly runs: readonly { readonly text: string; readonly strong?: boolean }[] }
  | { readonly kind: "image"; readonly source: string; readonly alt: string };

export function nativeDocumentSample(revision = "document:1"): NativeDocument<SampleNativeValue> {
  return { revision, blocks: [
    { id: "arrival", text: "A letter waits beside the mountain map.", value: { kind: "paragraph", runs: [
      { text: "A letter waits beside the " }, { text: "mountain map", strong: true }, { text: "." },
    ] } },
    { id: "map", text: "Map showing the mountain pass and the riverside route.", value: {
      kind: "image", source: "host-media:mountain-map", alt: "Map showing the mountain pass and the riverside route.",
    } },
    { id: "decision", choicePoint: `choice-point:${"1".repeat(32)}` },
    { id: "departure", text: "Choose which route to follow.", value: { kind: "paragraph", runs: [{ text: "Choose which route to follow." }] } },
  ] };
}
