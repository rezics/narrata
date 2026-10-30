import validate from "./validate-reader-map.js";
import type { ReaderMapV1 } from "../generated/authoring.js";
export type { ReaderMapV1, ReaderUnitV1, ReaderNodeV1, ReaderEdgeV1, VisitFrontierV1 } from "../generated/authoring.js";

export interface ReaderMapIdentity { readonly artifactId: string; readonly cursor: string }
const MAX_ITEMS = 100_000;
const MAX_RESPONSE_BYTES = 8 * 1024 * 1024;

/** Checks an untrusted single-cursor host response. This proves the DTO boundary and
 * requested identity; it does not grant authorization or turn client visits into evidence. */
export function decodeReaderMap(value: unknown, expected: ReaderMapIdentity): ReaderMapV1 {
  if (!validate(value) || value.format_version !== 1 || value.projection_version !== 1
    || value.artifact_id !== expected.artifactId || value.sources.length !== 1
    || value.sources[0]?.cursor !== expected.cursor) throw new Error("Invalid reader map response");
  let items = value.nodes.length + value.edges.length + value.units.length + value.endings.length;
  for (const unit of value.units) items += unit.segments.length;
  for (const source of value.sources) items += source.frontier.length + source.missing.length;
  if (items > MAX_ITEMS) throw new Error("Reader map response exceeds the host limit");
  return value;
}

/** The optional decoder is loaded only when requesting a map; HTTP input stays bounded. */
export async function readReaderMapResponse(response: Response, expected: ReaderMapIdentity): Promise<ReaderMapV1> {
  if (!response.ok || !response.body) throw new Error("Map service unavailable");
  const length = response.headers.get("content-length");
  if (length !== null && (!/^[0-9]+$/.test(length) || Number(length) > MAX_RESPONSE_BYTES)) throw new Error("Reader map response exceeds the host limit");
  const reader = response.body.getReader(), chunks: Uint8Array[] = [];
  let size = 0;
  try {
    for (;;) {
      const next = await reader.read(); if (next.done) break;
      size += next.value.length;
      if (size > MAX_RESPONSE_BYTES) { await reader.cancel(); throw new Error("Reader map response exceeds the host limit"); }
      chunks.push(next.value);
    }
  } finally { reader.releaseLock(); }
  const bytes = new Uint8Array(size); let offset = 0;
  for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.length; }
  const value: unknown = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  return decodeReaderMap(value, expected);
}
