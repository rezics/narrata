import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

export const execution = "execution:0190f2a0000070008000000000000001";
export const demoPack = new Uint8Array(readFileSync(resolve(import.meta.dirname, "../../../../products/gamebook-demo/story.narpack")));

// Fixture extraction of ADR 0013's canonical pack map; production decoding remains in Rust.
export function splitPack(bytes: Uint8Array) {
  const pack = Buffer.from(bytes);
  let at = 56;
  function size(major: number): number {
    const head = pack.readUInt8(at++);
    if (head >> 5 !== major) throw new Error("Unexpected fixture CBOR type");
    const info = head & 31;
    if (info < 24) return info;
    const width = info === 24 ? 1 : info === 25 ? 2 : info === 26 ? 4 : 0;
    if (!width) throw new Error("Unexpected fixture CBOR size");
    const value = pack.readUIntBE(at, width); at += width; return value;
  }
  function object() { const length = size(2); const value = pack.subarray(at, at + length); at += length; return new Uint8Array(value); }
  if (size(5) !== 4 || size(0) !== 0) throw new Error("Unexpected pack map");
  const manifest = object();
  if (size(0) !== 1) throw new Error("Missing chunks");
  const chunks = Array.from({ length: size(4) }, object);
  if (size(0) !== 2) throw new Error("Missing tombstones");
  const tombstones = object();
  if (size(0) !== 3) throw new Error("Missing names");
  const names = object();
  if (at !== pack.length) throw new Error("Trailing fixture bytes");
  const objects = new Map([...chunks, tombstones].map(bytes => [objectId(bytes), bytes]));
  return { manifest, chunks, tombstones, names, objects };
}
export function objectId(envelope: Uint8Array): string {
  return `object:${createHash("sha256").update("narrata-object\0").update(envelope.subarray(10, 14)).update(envelope.subarray(56)).digest("hex")}`;
}
