import { resolveContent, type ContentResolver, type ResolveContext } from "../content/index.js";
import { decodeIndex, decodeLabels, decodeTile, GraphDecodeError, type DetailLevel, type TileIndex } from "./wire.js";

export interface GraphOptions {
  readonly indexId: string;
  readonly labels?: readonly { readonly cluster: string; readonly objectId: string }[];
  readonly fetchObject: (id: string) => Promise<Uint8Array>;
  readonly resolver: ContentResolver;
  readonly context?: ResolveContext;
  readonly maxResolveItems?: number;
  readonly maxNodes?: number;
  readonly maxEdges?: number;
  readonly maxCachedObjects?: number;
  readonly maxCacheBytes?: number;
}
export interface GraphNode {
  readonly id: string; readonly x: number; readonly y: number; readonly label: string; readonly cluster: number;
  readonly ghost: boolean; readonly enter: { readonly cluster: number; readonly partition?: number } | null;
}
export interface GraphEdge { readonly id: string; readonly source: string; readonly target: string; readonly mask: number }
export interface GraphScreen { readonly level: DetailLevel; readonly cluster: number | null; readonly partition: number | null; readonly nodes: readonly GraphNode[]; readonly edges: readonly GraphEdge[] }
export interface GraphSnapshot { readonly phase: "idle" | "loading" | "ready" | "error"; readonly screen: GraphScreen | null; readonly limited: boolean; readonly error: Error | null }
function positive(value: number | undefined, fallback: number): number {
  const result = value ?? fallback;
  if (!Number.isSafeInteger(result) || result < 1) throw new RangeError("Graph budgets must be positive safe integers");
  return result;
}
function coordinate(value: bigint): number {
  const result = Number(value);
  if (!Number.isSafeInteger(result)) throw new GraphDecodeError("view coordinate exceeds exact JavaScript integer range");
  return result;
}
class DrawingLimit extends Error {}

/** Framework-free, checked lazy loading. A failed or over-budget transition keeps the previous view. */
export function createGraphController(options: GraphOptions) {
  const maxNodes = positive(options.maxNodes, 50_000), maxEdges = positive(options.maxEdges, 150_000);
  const maxObjects = positive(options.maxCachedObjects, 64), maxBytes = positive(options.maxCacheBytes, 32 * 1024 * 1024);
  const batchSize = positive(options.maxResolveItems, 256);
  const context = structuredClone(options.context ?? {});
  const labelObjects = new Map<string, string>();
  for (const label of options.labels ?? []) {
    if (!/^[0-9a-f]{32}$/.test(label.cluster) || !/^[0-9a-f]{64}$/.test(label.objectId) || labelObjects.has(label.cluster)) throw new GraphDecodeError("label object mapping");
    labelObjects.set(label.cluster, label.objectId);
  }
  let index: TileIndex | undefined;
  let snapshot: GraphSnapshot = Object.freeze({ phase: "idle", screen: null, limited: false, error: null });
  const subscribers = new Set<() => void>();
  const cache = new Map<string, Uint8Array>();
  let cacheBytes = 0, generation = 0, disposed = false;
  const update = (next: GraphSnapshot) => { snapshot = Object.freeze(next); for (const subscriber of subscribers) subscriber(); };
  async function fetch(id: string): Promise<Uint8Array> {
    const existing = cache.get(id);
    if (existing) { cache.delete(id); cache.set(id, existing); return existing; }
    const result = await options.fetchObject(id);
    if (!(result instanceof Uint8Array)) throw new GraphDecodeError("fetchObject must return bytes");
    const data = new Uint8Array(result);
    if (!disposed && data.byteLength <= maxBytes) {
      // Cache bytes only: decoded columns belong to the bounded active screen.
      const previous = cache.get(id); if (previous) { cacheBytes -= previous.byteLength; cache.delete(id); }
      cache.set(id, data); cacheBytes += data.byteLength;
      while (cache.size > maxObjects || cacheBytes > maxBytes) {
        const oldest = cache.keys().next().value!;
        cacheBytes -= cache.get(oldest)!.byteLength; cache.delete(oldest);
      }
    }
    return data;
  }
  async function navigate(level: DetailLevel, cluster: number | null, partition: number | null): Promise<boolean> {
    if (disposed) return false;
    const token = ++generation;
    const active = () => !disposed && token === generation;
    update({ ...snapshot, phase: "loading", limited: false, error: null });
    try {
      index ??= await decodeIndex(await fetch(options.indexId), options.indexId);
      if (!active()) return false;
      if (level !== "work" && (cluster === null || !index.clusters[cluster])) throw new GraphDecodeError("view cluster index");
      const rows = level === "work" ? [0] : index.clusters[cluster!]!.tiles.filter(row => {
        const entry = index!.tiles[row]!;
        return entry.level === level && (level !== "node" || entry.partition === partition);
      });
      if (rows.length === 0) throw new GraphDecodeError("view partition index");
      const nodes = new Map<string, GraphNode>(), edges = new Map<string, GraphEdge>();
      const partitions = level === "cluster" ? [...new Set(index.clusters[cluster!]!.tiles.filter(row => index!.tiles[row]!.level === "node").map(row => index!.tiles[row]!.partition!))] : [];
      const frame = cluster === null ? null : index.clusters[cluster]!;
      for (const tileRow of rows) {
        const tile = await decodeTile(await fetch(index.tiles[tileRow]!.objectId), index, tileRow);
        if (!active()) return false;
        const c = tile.columns;
        if (level === "cluster" && c.ids.length !== partitions.length) throw new GraphDecodeError("partition coverage in cluster view");
        for (let row = 0; row < c.ids.length; row++) {
          const id = c.ids[row]!, owner = c.clusters[row]!, origin = index.clusters[owner]!;
          if (!nodes.has(id)) {
            if (nodes.size === maxNodes) throw new DrawingLimit();
            const ghost = (c.nodeFlags[row]! & 32) !== 0;
            nodes.set(id, Object.freeze({ id, cluster: owner, ghost, label: `〔${id}〕`,
              x: coordinate(BigInt(c.x[row]!) + (frame ? origin.originX - frame.originX : 0n)),
              y: coordinate(BigInt(c.y[row]!) + (frame ? origin.originY - frame.originY : 0n)),
              enter: level === "work" ? Object.freeze({ cluster: owner }) : level === "cluster" ? Object.freeze({ cluster: owner, partition: partitions[row]! }) : null }));
          }
        }
        for (let row = 0; row < c.sources.length; row++) {
          const source = c.ids[c.sources[row]!]!, target = c.ids[c.targets[row]!]!, mask = c.edgeKindMasks[row]!;
          // Boundary edges may appear in both adjacent tiles; their identities are the same.
          const id = `${source}:${target}:${mask}`;
          if (!edges.has(id)) {
            if (edges.size === maxEdges) throw new DrawingLimit();
            edges.set(id, Object.freeze({ id, source, target, mask }));
          }
        }
      }
      const screen: GraphScreen = Object.freeze({ level, cluster, partition, nodes: Object.freeze([...nodes.values()]), edges: Object.freeze([...edges.values()]) });
      update({ phase: "loading", screen, limited: false, error: null });
      const references = new Map<string, { provider: string; key: string }>();
      // Aggregate cluster IDs have no node titles. Partition representatives and node rows do.
      if (level !== "work") for (const owner of new Set(screen.nodes.map(node => node.cluster))) {
        const clusterId = index.clusters[owner]!.id, object = labelObjects.get(clusterId);
        if (!object) continue;
        const labels = await decodeLabels(await fetch(object), object, clusterId);
        if (!active()) return false;
        for (const title of labels.titles) if (nodes.has(title.node)) references.set(title.node, title.content);
      }
      const items = [...references].map(([, content]) => ({ content }));
      const resolutions = await resolveContent(options.resolver, items, context, batchSize);
      if (!active()) return false;
      let position = 0;
      for (const [id] of references) {
        const result = resolutions[position++];
        if (result?.status === "ok" && "text" in result.payload) nodes.set(id, Object.freeze({ ...nodes.get(id)!, label: result.payload.text }));
      }
      update({ ...snapshot, phase: "ready", screen: Object.freeze({ ...screen, nodes: Object.freeze([...nodes.values()]) }) });
      return true;
    } catch (error) {
      if (!active()) return false;
      // Failed verification/network reads must be retryable after the host repairs the object.
      if (!(error instanceof DrawingLimit)) { cache.clear(); cacheBytes = 0; }
      update({ ...snapshot, phase: snapshot.screen ? "ready" : "error", limited: error instanceof DrawingLimit,
        error: error instanceof DrawingLimit ? null : error instanceof Error ? error : new Error(String(error)) });
      return false;
    }
  }
  return {
    getSnapshot: () => snapshot,
    subscribe: (subscriber: () => void) => { subscribers.add(subscriber); return () => { subscribers.delete(subscriber); }; },
    start: () => snapshot.phase === "idle" ? navigate("work", null, null) : Promise.resolve(snapshot.screen !== null),
    showWork: () => navigate("work", null, null),
    enterCluster: (cluster: number) => navigate("cluster", cluster, null),
    enterPartition: (cluster: number, partition: number) => navigate("node", cluster, partition),
    back: () => snapshot.screen?.level === "node" ? navigate("cluster", snapshot.screen.cluster, null) : navigate("work", null, null),
    cacheStats: () => Object.freeze({ objects: cache.size, bytes: cacheBytes }),
    dispose: () => { disposed = true; generation++; cache.clear(); cacheBytes = 0; index = undefined; subscribers.clear(); snapshot = Object.freeze({ phase: "idle", screen: null, limited: false, error: null }); },
  };
}
export type GraphController = ReturnType<typeof createGraphController>;
