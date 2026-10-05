import { readFileSync, readdirSync } from "node:fs";
import { resolve } from "node:path";
import { createHash } from "node:crypto";
import { expect, test } from "vitest";
import { decodeIndex, decodeTile, decodeLabels, GraphDecodeError } from "@rezics/narrata/graph";
import { decodeCanonical, encodeCanonical, decode, encode, type CanonicalCbor } from "../dist/storage/cbor.js";

const root = resolve(import.meta.dirname, "../../../..");
export const graphDirectory = resolve(root, "products/gamebook-demo/story.graph");
export const publication: { index_object_id: string; labels: { cluster: string; object_id: string }[] } = JSON.parse(readFileSync(resolve(root, "products/gamebook-demo/story.graph.json"), "utf8"));
export const object = (directory: string, id: string) => new Uint8Array(readFileSync(resolve(directory, `${id}.cbor`)));
const limits = { maxMessageBytes: 512 * 1024 * 1024, maxStringBytes: 512 * 1024 * 1024, maxArrayItems: 6_000_001, maxNodes: 144_000_024, maxDepth: 8 };
function map(value: CanonicalCbor): Map<number, CanonicalCbor> { if (!(value instanceof Map)) throw new Error("fixture map"); return value; }
function list(value: CanonicalCbor | undefined): CanonicalCbor[] { if (!Array.isArray(value)) throw new Error("fixture array"); return value; }
function envelope(kind: number, value: CanonicalCbor | Uint8Array) {
  const payload = value instanceof Uint8Array ? value : encodeCanonical(value);
  const bytes = new Uint8Array(56 + payload.length), view = new DataView(bytes.buffer);
  bytes.set(new TextEncoder().encode("NARRATA\0")); view.setUint16(10, kind); view.setUint16(12, 1); view.setBigUint64(16, BigInt(payload.length));
  bytes.set(createHash("sha256").update(payload).digest(), 24); bytes.set(payload, 56);
  return { bytes, id: createHash("sha256").update("narrata-object\0").update(bytes.subarray(10, 14)).update(payload).digest("hex") };
}
const valueOf = (bytes: Uint8Array) => map(decodeCanonical(bytes.subarray(56), limits));

test("every frozen geometry and published demo object reads with its requested identity", async () => {
  for (const [directory, indexId] of [
    [resolve(root, "fixtures/compat/graph-v1"), JSON.parse(readFileSync(resolve(root, "fixtures/compat/graph-v1/manifest.json"), "utf8")).index_object_id],
    [graphDirectory, publication.index_object_id],
  ] as const) {
    const index = await decodeIndex(object(directory, indexId), indexId);
    const readers = [{ id: indexId, read: (bytes: Uint8Array) => decodeIndex(bytes, indexId) }, ...index.tiles.map((entry, row) => ({ id: entry.objectId, read: (bytes: Uint8Array) => decodeTile(bytes, index, row) }))];
    for (const reader of readers) {
      const data = object(directory, reader.id);
      await expect(reader.read(data)).resolves.toBeDefined();
      const corrupt = new Uint8Array(data); corrupt[corrupt.length - 1]! ^= 1;
      await expect(reader.read(corrupt)).rejects.toBeInstanceOf(GraphDecodeError);
    }
    await expect(decodeIndex(object(directory, indexId), "0".repeat(64))).rejects.toThrow("identity mismatch");
    await expect(decodeTile(object(directory, index.tiles[0]!.objectId), index, index.tiles.length)).rejects.toThrow("out of bounds");
    await expect(decodeTile(object(directory, index.tiles[0]!.objectId), index, 1)).rejects.toThrow("kind");
    expect(Object.isFrozen(index.clusters[0]?.tiles)).toBe(true);
    expect(readers.length).toBe(readdirSync(directory).filter(file => file.endsWith(".cbor")).length - (directory === graphDirectory ? publication.labels.length : 0));
  }
});

test("all frozen and demo label objects validate cluster ownership, identities and byte corruption", async () => {
  const frozen = resolve(root, "fixtures/compat/graph-v1-labels");
  const manifest: { files: { cluster: string; object_id: string }[] } = JSON.parse(readFileSync(resolve(frozen, "manifest.json"), "utf8"));
  for (const [directory, rows] of [[frozen, manifest.files], [graphDirectory, publication.labels]] as const) for (const row of rows) {
    const bytes = object(directory, row.object_id), cluster = row.cluster.slice(8);
    await expect(decodeLabels(bytes, row.object_id, cluster)).resolves.toMatchObject({ cluster });
    const corrupt = new Uint8Array(bytes); corrupt[corrupt.length - 1]! ^= 1;
    await expect(decodeLabels(corrupt, row.object_id, cluster)).rejects.toThrow("SHA-256");
    await expect(decodeLabels(bytes, "0".repeat(64), cluster)).rejects.toThrow("identity mismatch");
    await expect(decodeLabels(bytes, row.object_id, "0".repeat(32))).rejects.toThrow("cluster mismatch");
  }
});

test("a recomputed object ID cannot hide invalid endpoints, columns, classifications, bounds or budgets", async () => {
  const indexBytes = object(graphDirectory, publication.index_object_id);
  const originalIndex = await decodeIndex(indexBytes, publication.index_object_id);
  const tileRow = originalIndex.tiles.findIndex(tile => tile.level === "node");
  const tileBytes = object(graphDirectory, originalIndex.tiles[tileRow]!.objectId);
  const mutations: ((value: Map<number, CanonicalCbor>) => void)[] = [
    value => map(value.get(5)!).set(1, new Uint8Array()),
    value => { const data = map(value.get(5)!).get(10); if (!(data instanceof Uint8Array)) throw new Error(); new DataView(data.buffer).setUint32(0, 1_000_000, true); },
    value => { const data = map(value.get(5)!).get(3); if (!(data instanceof Uint8Array)) throw new Error(); new DataView(data.buffer).setUint32(0, originalIndex.clusters.length, true); },
    value => map(value.get(5)!).set(4, new Uint8Array(Number(value.get(3))).fill(255)),
    value => map(value.get(5)!).set(6, new Uint8Array(Number(value.get(3)) * 4).fill(255)),
    value => map(value.get(5)!).set(11, new Uint8Array(Number(value.get(4)))),
    value => map(value.get(5)!).set(14, new Uint8Array(Number(value.get(4))).fill(4)),
    value => map(value.get(5)!).set(8, new Uint8Array(Number(value.get(3))).fill(2)),
    value => map(value.get(5)!).set(12, new Uint8Array(Number(value.get(4)) * 20).fill(255)),
    value => value.set(1, null),
    value => value.set(3, 2001),
    value => value.set(6, null),
    value => { const data = map(value.get(5)!).get(1); if (!(data instanceof Uint8Array)) throw new Error(); new DataView(data.buffer).setInt32(0, -2147483648, true); },
  ];
  for (const mutate of mutations) {
    const tileValue = valueOf(tileBytes); mutate(tileValue);
    const tile = envelope(0x0123, tileValue);
    const indexValue = valueOf(indexBytes);
    list(list(indexValue.get(1))[tileRow])[4] = Buffer.from(tile.id, "hex");
    const indexObject = envelope(0x0120, indexValue), index = await decodeIndex(indexObject.bytes, indexObject.id);
    await expect(decodeTile(tile.bytes, index, tileRow)).rejects.toBeInstanceOf(GraphDecodeError);
  }
});

test("index field sets, mapping indices, order, envelopes and noncanonical payloads are rejected", async () => {
  const original = object(graphDirectory, publication.index_object_id);
  for (const mutate of [
    (value: Map<number, CanonicalCbor>) => value.set(2, null),
    (value: Map<number, CanonicalCbor>) => { list(list(list(value.get(0))[0])[3])[0] = 0xffff_ffff; },
    (value: Map<number, CanonicalCbor>) => { list(value.get(0)).reverse(); },
    (value: Map<number, CanonicalCbor>) => { list(list(value.get(1))[0])[0] = 2; },
  ]) {
    const value = valueOf(original); mutate(value); const data = envelope(0x0120, value);
    await expect(decodeIndex(data.bytes, data.id)).rejects.toBeInstanceOf(GraphDecodeError);
  }
  for (const at of [0, 8, 10, 12, 14, 16, 24]) { const data = new Uint8Array(original); data[at]! ^= 1; await expect(decodeIndex(data, publication.index_object_id)).rejects.toBeInstanceOf(GraphDecodeError); }
  const noncanonical = new Uint8Array(original.length - 55); noncanonical.set([0xb8, 2]); noncanonical.set(original.subarray(57), 2);
  const data = envelope(0x0120, noncanonical);
  await expect(decodeIndex(data.bytes, data.id)).rejects.toThrow("canonical CBOR");
  await expect(decodeIndex(original.subarray(0, 55), publication.index_object_id)).rejects.toThrow("truncated");
});

test("canonical graph profile preserves i64/u64 without changing the storage profile", () => {
  const value = new Map<number, CanonicalCbor>([[0, -(1n << 63n)], [1, (1n << 64n) - 1n], [2, -1], [3, "雪"]]);
  expect(decodeCanonical(encodeCanonical(value), limits)).toEqual(value);
  expect(() => decode(encodeCanonical(value))).toThrow("outside the profile");
  expect(() => encode(-1)).toThrow("storage profile");
  expect(() => decodeCanonical(new Uint8Array([0xa2, 0, 0, 0, 0]), limits)).toThrow("map key order");
  expect(() => decodeCanonical(new Uint8Array([0x9a, 0, 0xff, 0xff, 0xff]), limits)).toThrow();
});

test("empty work still checks zero-length ID columns and label tables reject malformed content and duplicate IDs", async () => {
  const columns = new Map<number, CanonicalCbor>(Array.from({ length: 15 }, (_, key) => [key, new Uint8Array()]));
  const tileValue = new Map<number, CanonicalCbor>([[0, null], [1, null], [2, 0], [3, 0], [4, 0], [5, columns]]);
  const tile = envelope(0x0121, tileValue);
  const indexValue = new Map<number, CanonicalCbor>([[0, []], [1, [[0, null, null, 0, Buffer.from(tile.id, "hex"), null]]]]);
  const indexObject = envelope(0x0120, indexValue), index = await decodeIndex(indexObject.bytes, indexObject.id);
  await expect(decodeTile(tile.bytes, index, 0)).resolves.toMatchObject({ columns: { ids: [] } });
  columns.set(0, new Uint8Array(1));
  const invalidTile = envelope(0x0121, tileValue);
  list(list(indexValue.get(1))[0])[4] = Buffer.from(invalidTile.id, "hex");
  const invalidIndex = envelope(0x0120, indexValue);
  await expect(decodeTile(invalidTile.bytes, await decodeIndex(invalidIndex.bytes, invalidIndex.id), 0)).rejects.toThrow("byte length");
  const cluster = "0".repeat(31) + "1";
  for (const content of [new Map<number, CanonicalCbor>([[0, "LOCAL"], [1, "key"]]), new Map<number, CanonicalCbor>([[0, "local"], [1, ""]]), new Map<number, CanonicalCbor>([[0, "local"], [1, "a\u0085b"]]), new Map<number, CanonicalCbor>([[0, "local"], [1, "雪".repeat(86)]])]) {
    const value = new Map<number, CanonicalCbor>([[0, Buffer.from(cluster, "hex")], [1, [[new Uint8Array(16), content]]]]);
    const data = envelope(0x0124, value);
    await expect(decodeLabels(data.bytes, data.id, cluster)).rejects.toThrow("content reference");
  }
  const row: CanonicalCbor[] = [new Uint8Array(16), new Map<number, CanonicalCbor>([[0, "local"], [1, "key"]])];
  const value = new Map<number, CanonicalCbor>([[0, Buffer.from(cluster, "hex")], [1, [row, row]]]);
  const data = envelope(0x0124, value);
  await expect(decodeLabels(data.bytes, data.id, cluster)).rejects.toThrow("duplicate");
});
