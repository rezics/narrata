import { decodeCanonical, equalBytes, type CanonicalCbor as Value } from "../storage/cbor.js";
import { objectId, sha256 } from "../storage/digest.js";
import type { ContentRef } from "../content/request.js";

export class GraphDecodeError extends Error { override name = "GraphDecodeError"; }
function fail(message: string): never { throw new GraphDecodeError(message); }
export const graphLimits = Object.freeze({ objectBytes: 512 * 1024 * 1024, nodes: 1_000_000, edges: 4_000_000, tiles: 6_000_001, tileElements: 2_000 });
export type DetailLevel = "work" | "cluster" | "node";
const levels = ["work", "cluster", "node"] as const;
export interface ClusterIndex { readonly id: string; readonly originX: bigint; readonly originY: bigint; readonly tiles: readonly number[] }
export interface TileEntry { readonly level: DetailLevel; readonly cluster: number | null; readonly partition: number | null; readonly ordinal: number; readonly objectId: string; readonly bounds: readonly bigint[] | null }
declare const checkedIndex: unique symbol;
export interface TileIndex { readonly [checkedIndex]: true; readonly clusters: readonly ClusterIndex[]; readonly tiles: readonly TileEntry[] }
const indexes = new WeakSet<TileIndex>();
export interface TileColumns {
  readonly ids: readonly string[]; readonly x: readonly number[]; readonly y: readonly number[];
  readonly clusters: readonly number[]; readonly kinds: readonly number[]; readonly importance: readonly number[];
  readonly nodeFlags: readonly number[]; readonly authorOrder: readonly bigint[]; readonly hasAuthorOrder: readonly number[];
  readonly sources: readonly number[]; readonly targets: readonly number[]; readonly edgeKindMasks: readonly number[];
  readonly edgeKindCounts: readonly (readonly number[])[]; readonly edgeImportance: readonly number[]; readonly edgeFlags: readonly number[];
}
export interface GraphTile { readonly level: DetailLevel; readonly cluster: number | null; readonly partition: number | null; readonly ordinal: number; readonly columns: TileColumns }
export interface LabelTable { readonly cluster: string; readonly titles: readonly { readonly node: string; readonly content: Readonly<ContentRef> }[] }
export function hex(bytes: Uint8Array): string { return Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join(""); }
function id(value: Value | undefined, size: number): string { return hex(bytes(value, size)); }
function bytes(value: Value | undefined, size: number): Uint8Array {
  if (!(value instanceof Uint8Array) || value.length !== size) fail("column byte length or identity width");
  return value;
}
function uint(value: Value | undefined, max = 0xffff_ffff): number {
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0 || value > max) fail("unsigned integer or row budget");
  return value;
}
function signed(value: Value | undefined): bigint {
  if (typeof value !== "bigint" && (typeof value !== "number" || !Number.isSafeInteger(value))) fail("signed integer");
  const result = BigInt(value);
  if (result < -(1n << 63n) || result >= 1n << 63n) fail("i64 coordinate overflow");
  return result;
}
function optional(value: Value | undefined): number | null { return value === null ? null : uint(value); }
function array(value: Value | undefined, max: number, exact = false): readonly Value[] {
  if (!Array.isArray(value) || (exact ? value.length !== max : value.length > max)) fail("array shape or row budget");
  return value;
}
function fields(value: Value, count: number): ReadonlyMap<number, Value> {
  if (!(value instanceof Map) || value.size !== count || [...value.keys()].some((key, i) => key !== i)) fail("exact field set");
  return value;
}
async function payload(input: Uint8Array, expected: string, kind: number): Promise<Value> {
  if (!/^[0-9a-f]{64}$/.test(expected)) fail("graph object identity syntax");
  // Copy before awaiting SHA-256: a host cannot change bytes between checksum and parse.
  if (input.length < 56 || input.length > graphLimits.objectBytes) fail("graph object byte budget or truncated envelope");
  const data = new Uint8Array(input);
  const view = new DataView(data.buffer);
  if (hex(data.subarray(0, 8)) !== "4e41525241544100" || view.getUint16(8) !== 0 || view.getUint16(14) !== 0) fail("envelope magic, version or flags");
  if (view.getUint16(10) !== kind || view.getUint16(12) !== 1) fail("object kind or schema");
  if (view.getBigUint64(16) !== BigInt(data.length - 56)) fail("envelope payload length");
  const content = data.subarray(56);
  if (!equalBytes(await sha256(content), data.subarray(24, 56))) fail("payload SHA-256 mismatch");
  if (hex(await objectId(kind, 1, content)) !== expected) fail("graph object identity mismatch");
  try {
    return decodeCanonical(content, { maxMessageBytes: graphLimits.objectBytes - 56, maxStringBytes: graphLimits.objectBytes,
      maxArrayItems: graphLimits.tiles, maxNodes: graphLimits.tiles * 24, maxDepth: 8 });
  } catch (error) { fail(`canonical CBOR: ${error instanceof Error ? error.message : String(error)}`); }
}
function shape(entry: Pick<TileEntry, "level" | "cluster" | "partition" | "ordinal">, clusters: number) {
  const { level, cluster, partition, ordinal } = entry;
  if (cluster !== null && cluster >= clusters) fail("tile cluster index");
  if (level === "work" ? cluster !== null || partition !== null || ordinal !== 0
    : level === "cluster" ? cluster === null || partition !== null || ordinal !== 0
      : cluster === null || partition === null || partition >= graphLimits.nodes) fail("tile ownership");
}
export async function decodeIndex(input: Uint8Array, expectedId: string): Promise<TileIndex> {
  const map = fields(await payload(input, expectedId, 0x0120), 2);
  let mappingRows = 0;
  const clusters = array(map.get(0), graphLimits.nodes).map(value => {
    const row = array(value, 4, true);
    const tiles = array(row[3], graphLimits.tiles).map(value => uint(value));
    mappingRows += tiles.length;
    if (mappingRows > graphLimits.tiles + graphLimits.nodes) fail("cluster mapping budget");
    return Object.freeze({ id: id(row[0], 16), originX: signed(row[1]), originY: signed(row[2]), tiles: Object.freeze(tiles) });
  });
  const tiles = array(map.get(1), graphLimits.tiles).map(value => {
    const row = array(value, 6, true);
    return Object.freeze({ level: levels[uint(row[0], 2)]!, cluster: optional(row[1]), partition: optional(row[2]), ordinal: uint(row[3]),
      objectId: id(row[4], 32), bounds: row[5] === null ? null : Object.freeze(array(row[5], 4, true).map(signed)) });
  });
  if (tiles.length < clusters.length + 1 || tiles[0]?.level !== "work" || clusters.some((c, i) => i > 0 && clusters[i - 1]!.id >= c.id)) fail("index order or work tile");
  const mapping = clusters.map(() => [0]);
  const identities = new Set<string>();
  let previous: TileEntry | undefined;
  let partition: number | null = null;
  let partitionCluster: number | null = null;
  let ordinal = 0;
  for (const [row, tile] of tiles.entries()) {
    shape(tile, clusters.length);
    if (previous) {
      const key = (t: TileEntry) => [levels.indexOf(t.level), t.cluster ?? -1, t.partition ?? -1, t.ordinal];
      const a = key(previous), b = key(tile);
      const difference = a.map((v, i) => v - b[i]!).find(v => v !== 0) ?? 0;
      if (difference >= 0) fail("tile order");
    }
    previous = tile;
    if (identities.has(tile.objectId)) fail("duplicate tile identity");
    identities.add(tile.objectId);
    if (tile.bounds ? tile.bounds[0]! > tile.bounds[2]! || tile.bounds[1]! > tile.bounds[3]! : row !== 0 || clusters.length !== 0) fail("tile bounds");
    if (row > 0 && row <= clusters.length && (tile.level !== "cluster" || tile.cluster !== row - 1)) fail("cluster tile coverage");
    if (row > clusters.length && tile.level !== "node") fail("node tile coverage");
    if (tile.level === "node") {
      if (tile.partition !== partition) {
        if (tile.partition !== (partition === null ? 0 : partition + 1) || tile.ordinal !== 0) fail("partition sequence");
        partition = tile.partition; partitionCluster = tile.cluster;
      } else if (tile.ordinal !== ordinal + 1 || tile.cluster !== partitionCluster) fail("tile ordinal sequence");
      ordinal = tile.ordinal;
    }
    if (tile.cluster !== null) mapping[tile.cluster]!.push(row);
  }
  clusters.forEach((cluster, i) => {
    if (cluster.tiles.length !== mapping[i]!.length || cluster.tiles.some((row, j) => row !== mapping[i]![j]) || !cluster.tiles.some(row => tiles[row]?.level === "node")) fail("cluster tile mapping");
  });
  // Only this checked constructor supplies the opaque index; all nested rows are frozen.
  const index = Object.freeze({ clusters: Object.freeze(clusters), tiles: Object.freeze(tiles) }) as TileIndex;
  indexes.add(index);
  return index;
}
export async function decodeTile(input: Uint8Array, index: TileIndex, tileIndex: number): Promise<GraphTile> {
  if (!indexes.has(index) || !Number.isSafeInteger(tileIndex) || tileIndex < 0) fail("checked index or tile index");
  const entry = index.tiles[tileIndex];
  if (!entry) fail("tile index out of bounds");
  const map = fields(await payload(input, entry.objectId, 0x0121 + levels.indexOf(entry.level)), 6);
  const cluster = optional(map.get(0)), partition = optional(map.get(1)), ordinal = uint(map.get(2));
  const nodes = uint(map.get(3), graphLimits.nodes), edges = uint(map.get(4), graphLimits.edges);
  if (entry.level === "node" && nodes + edges > graphLimits.tileElements) fail("node tile element budget");
  if (cluster !== entry.cluster || partition !== entry.partition || ordinal !== entry.ordinal) fail("tile index ownership metadata");
  const cols = fields(map.get(5)!, 15);
  const column = <T>(key: number, rows: number, width: number, read: (view: DataView, at: number) => T): readonly T[] => {
    const data = bytes(cols.get(key), rows * width);
    const view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    return Object.freeze(Array.from({ length: rows }, (_, row) => read(view, row * width)));
  };
  const u32 = (view: DataView, at: number) => view.getUint32(at, true);
  const u8 = (view: DataView, at: number) => view.getUint8(at);
  const idBytes = bytes(cols.get(0), nodes * 16);
  const ids = Object.freeze(Array.from({ length: nodes }, (_, row) => hex(idBytes.subarray(row * 16, row * 16 + 16))));
  const c: TileColumns = Object.freeze({ ids, x: column(1, nodes, 4, (v, p) => v.getInt32(p, true)), y: column(2, nodes, 4, (v, p) => v.getInt32(p, true)),
    clusters: column(3, nodes, 4, u32), kinds: column(4, nodes, 1, u8), importance: column(5, nodes, 1, u8), nodeFlags: column(6, nodes, 4, u32),
    authorOrder: column(7, nodes, 8, (v, p) => v.getBigUint64(p, true)), hasAuthorOrder: column(8, nodes, 1, u8),
    sources: column(9, edges, 4, u32), targets: column(10, edges, 4, u32), edgeKindMasks: column(11, edges, 1, u8),
    edgeKindCounts: column(12, edges, 20, (v, p) => Object.freeze(Array.from({ length: 5 }, (_, i) => u32(v, p + i * 4)))),
    edgeImportance: column(13, edges, 1, u8), edgeFlags: column(14, edges, 1, u8) });
  if ((nodes === 0 && (entry.level !== "work" || index.clusters.length !== 0)) || (entry.level === "work" && nodes !== index.clusters.length) || new Set(ids).size !== nodes) fail("tile node identities");
  let bounds: bigint[] | null = null;
  for (let row = 0; row < nodes; row++) {
    const owner = c.clusters[row]!, flags = c.nodeFlags[row]!, kind = c.kinds[row]!;
    if (owner >= index.clusters.length || c.hasAuthorOrder[row]! > 1 || (c.hasAuthorOrder[row] === 0 && c.authorOrder[row] !== 0n) || (flags & ~63) !== 0) fail("node columns");
    if (entry.level === "work" ? kind !== 6 || owner !== row || ids[row] !== index.clusters[row]!.id
      : entry.level === "cluster" ? kind !== 7 || owner !== cluster
        : kind > 5 || (owner !== cluster && !(flags & 32)) || (kind === 5 && !(flags & 4))) fail("node kind or level ownership");
    if (entry.level !== "node" && (flags !== 0 || c.hasAuthorOrder[row] !== 0)) fail("aggregate node flags");
    const frame = index.clusters[owner]!;
    const x = signed(BigInt(c.x[row]!) + (entry.level === "work" ? 0n : frame.originX));
    const y = signed(BigInt(c.y[row]!) + (entry.level === "work" ? 0n : frame.originY));
    bounds = bounds === null ? [x, y, x, y] : [x < bounds[0]! ? x : bounds[0]!, y < bounds[1]! ? y : bounds[1]!, x > bounds[2]! ? x : bounds[2]!, y > bounds[3]! ? y : bounds[3]!];
  }
  if (bounds === null ? entry.bounds !== null : entry.bounds === null || bounds.some((v, i) => v !== entry.bounds![i])) fail("tile index bounds mismatch");
  let multiplicity = 0;
  for (let row = 0; row < edges; row++) {
    const source = c.sources[row]!, target = c.targets[row]!;
    if (source >= nodes || target >= nodes) fail("edge endpoint index");
    const counts = c.edgeKindCounts[row]!;
    const mask = counts.reduce((mask, count, i) => mask | (count > 0 ? 1 << i : 0), 0);
    multiplicity += counts.reduce((sum, count) => sum + count, 0);
    if (mask === 0 || mask !== c.edgeKindMasks[row] || (c.edgeFlags[row]! & ~3) !== 0 || c.edgeImportance[row] !== Math.max(c.importance[source]!, c.importance[target]!)) fail("edge counts, mask, flags or importance");
    if (multiplicity > graphLimits.edges) fail("edge multiplicity budget");
  }
  return Object.freeze({ level: entry.level, cluster, partition, ordinal, columns: c });
}
export async function decodeLabels(input: Uint8Array, expectedId: string, expectedCluster: string): Promise<LabelTable> {
  const map = fields(await payload(input, expectedId, 0x0124), 2);
  const cluster = id(map.get(0), 16);
  if (cluster !== expectedCluster) fail("label cluster mismatch");
  let previous = "";
  const titles = array(map.get(1), graphLimits.nodes).map(value => {
    const row = array(value, 2, true), node = id(row[0], 16);
    if (node <= previous) fail("label ID order or duplicate");
    previous = node;
    const reference = fields(row[1]!, 2), provider = reference.get(0), key = reference.get(1);
    if (typeof provider !== "string" || !/^[a-z0-9-]{1,32}$/.test(provider) || typeof key !== "string" || new TextEncoder().encode(key).length < 1 || new TextEncoder().encode(key).length > 256 || /[\u0000-\u001f\u007f-\u009f]/u.test(key)) fail("content reference");
    return Object.freeze({ node, content: Object.freeze({ provider, key }) });
  });
  return Object.freeze({ cluster, titles: Object.freeze(titles) });
}
