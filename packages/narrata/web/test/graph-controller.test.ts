import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { expect, test, vi } from "vitest";
import { createGraphController, decodeIndex, decodeTile, type GraphOptions } from "@rezics/narrata/graph";

const root = resolve(import.meta.dirname, "../../../..");
const publication: { index_object_id: string; labels: { cluster: string; object_id: string }[] } = JSON.parse(readFileSync(resolve(root, "products/gamebook-demo/story.graph.json"), "utf8"));
const directory = resolve(root, "products/gamebook-demo/story.graph");
const fetchObject = async (id: string) => new Uint8Array(readFileSync(resolve(directory, `${id}.cbor`)));
const options: GraphOptions = { indexId: publication.index_object_id, labels: publication.labels.map(row => ({ cluster: row.cluster.slice(8), objectId: row.object_id })),
  fetchObject, resolver: { resolve: async request => request.items.map(item => ({ status: "ok", revision: "host", payload: { text: "unit" in item.content ? "" : item.content.key } })) } };

test("three levels use published coordinates, batch only visible titles, preserve context and bounded cache", async () => {
  const fetch = vi.fn(fetchObject), requests: Parameters<GraphOptions["resolver"]["resolve"]>[0][] = [];
  const controller = createGraphController({ ...options, fetchObject: fetch, maxCachedObjects: 2, maxCacheBytes: 4000, maxResolveItems: 1,
    context: { languages: ["zh-Hans"], realization: "edition", viewer: "author" },
    resolver: { resolve: async request => { requests.push(request); return options.resolver.resolve(request); } } });
  try {
    expect(fetch).not.toHaveBeenCalled();
    expect(await controller.start()).toBe(true);
    expect(fetch).toHaveBeenCalledTimes(2);
    expect(requests).toEqual([]);
    const index = await decodeIndex(await fetchObject(options.indexId), options.indexId);
    const work = await decodeTile(await fetchObject(index.tiles[0]!.objectId), index, 0);
    expect(controller.getSnapshot().screen?.nodes.map(node => [node.x, node.y])).toEqual(work.columns.x.map((x, i) => [x, work.columns.y[i]]));
    for (const cluster of index.clusters.keys()) {
      expect(await controller.enterCluster(cluster)).toBe(true);
      const partitions = controller.getSnapshot().screen?.nodes;
      expect(controller.getSnapshot().screen?.level).toBe("cluster");
      const enter = partitions?.[0]?.enter;
      if (!enter || enter.partition === undefined) throw new Error("partition navigation");
      expect(await controller.enterPartition(cluster, enter.partition)).toBe(true);
      const screen = controller.getSnapshot().screen!;
      expect(screen.level).toBe("node");
      const tileRow = index.tiles.findIndex(entry => entry.cluster === cluster && entry.partition === enter.partition && entry.level === "node");
      const tile = await decodeTile(await fetchObject(index.tiles[tileRow]!.objectId), index, tileRow);
      const first = screen.nodes.find(node => node.id === tile.columns.ids[0])!;
      const origin = index.clusters[tile.columns.clusters[0]!]!;
      expect(first.x).toBe(Number(BigInt(tile.columns.x[0]!) + origin.originX - index.clusters[cluster]!.originX));
      expect(controller.cacheStats().objects).toBeLessThanOrEqual(2);
      expect(controller.cacheStats().bytes).toBeLessThanOrEqual(4000);
      expect(await controller.back()).toBe(true);
      expect(controller.getSnapshot().screen?.level).toBe("cluster");
    }
    expect(requests.length).toBeGreaterThan(0);
    expect(requests.every(request => request.items.length <= 1 && request.context.viewer === "author" && request.context.realization === "edition" && request.context.languages?.[0] === "zh-Hans")).toBe(true);
  } finally { controller.dispose(); }
  expect(controller.cacheStats()).toEqual({ objects: 0, bytes: 0 });
});

test("node and edge budgets retain the prior level, including when the overview itself exceeds limits", async () => {
  const controller = createGraphController({ ...options, maxNodes: 1, maxEdges: 1 });
  expect(await controller.start()).toBe(false);
  expect(controller.getSnapshot()).toMatchObject({ phase: "error", screen: null, limited: true });
  controller.dispose();
  const index = await decodeIndex(await fetchObject(options.indexId), options.indexId);
  const work = await decodeTile(await fetchObject(index.tiles[0]!.objectId), index, 0);
  const bounded = createGraphController({ ...options, maxNodes: work.columns.ids.length, maxEdges: work.columns.sources.length });
  try {
    expect(await bounded.start()).toBe(true);
    // Invalid navigation also leaves geometry and labels usable.
    const previous = bounded.getSnapshot().screen;
    expect(await bounded.enterPartition(0, 999999)).toBe(false);
    expect(bounded.getSnapshot().screen).toBe(previous);
    expect(bounded.getSnapshot().error?.message).toContain("partition");
    expect(await bounded.enterCluster(-1)).toBe(false);
    expect(bounded.getSnapshot().screen).toBe(previous);
  } finally { bounded.dispose(); }
  const frozen = resolve(root, "fixtures/compat/graph-v1");
  const manifest: { index_object_id: string } = JSON.parse(readFileSync(resolve(frozen, "manifest.json"), "utf8"));
  for (const budget of [{ maxNodes: 6, maxEdges: 150000 }, { maxNodes: 50000, maxEdges: 6 }]) {
    const view = createGraphController({ ...options, indexId: manifest.index_object_id, labels: [], ...budget,
      fetchObject: async id => new Uint8Array(readFileSync(resolve(frozen, `${id}.cbor`))) });
    try {
      expect(await view.start()).toBe(true);
      expect(await view.enterCluster(0)).toBe(true);
      const previous = view.getSnapshot().screen;
      expect(await view.enterPartition(0, 0)).toBe(false);
      expect(view.getSnapshot()).toMatchObject({ screen: previous, phase: "ready", limited: true, error: null });
      expect(view.getSnapshot().screen).toBe(previous);
    } finally { view.dispose(); }
  }
});

test("unavailable titles keep IDs as placeholders and resolver failures preserve geometry", async () => {
  for (const resolver of [
    { resolve: async (request: Parameters<GraphOptions["resolver"]["resolve"]>[0]) => request.items.map(() => ({ status: "unavailable" as const })) },
    { resolve: async () => { throw new Error("offline"); } },
  ]) {
    const controller = createGraphController({ ...options, resolver });
    try {
      await controller.start(); await controller.enterCluster(0);
      const enter = controller.getSnapshot().screen?.nodes[0]?.enter;
      if (!enter || enter.partition === undefined) throw new Error("partition");
      await controller.enterPartition(0, enter.partition);
      expect(controller.getSnapshot().screen?.nodes.every(node => node.label === `〔${node.id}〕`)).toBe(true);
      expect(controller.getSnapshot().screen?.level).toBe("node");
    } finally { controller.dispose(); }
  }
});

test("late fetches cannot replace newer navigation or resurrect a disposed controller", async () => {
  const index = await decodeIndex(await fetchObject(options.indexId), options.indexId);
  let release: (() => void) | undefined;
  const controller = createGraphController({ ...options, fetchObject: async id => {
    if (id === index.tiles[1]!.objectId) await new Promise<void>(resolve => { release = resolve; });
    return fetchObject(id);
  } });
  await controller.start();
  const pending = controller.enterCluster(0);
  await vi.waitFor(() => expect(release).toBeDefined());
  expect(await controller.showWork()).toBe(true);
  release!();
  expect(await pending).toBe(false);
  expect(controller.getSnapshot().screen?.level).toBe("work");
  controller.dispose();
  expect(await controller.enterCluster(1)).toBe(false);
  expect(controller.cacheStats().objects).toBe(0);
});
