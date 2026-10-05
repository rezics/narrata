import { useMemo, useRef, useState, useSyncExternalStore } from "react";
import type { ContentResolver, GraphAnalysis, NodeAddress } from "@rezics/narrata";
import { AuthorGraph } from "@rezics/narrata/graph/react";
import type { GraphController, GraphSnapshot, MountGraphOptions } from "@rezics/narrata/graph";
import publication from "../../../products/gamebook-demo/story.graph.json";

// URLs live in this lazy module. Objects stay separate files in production as well as dev.
const objects = import.meta.glob<string>("../../../products/gamebook-demo/story.graph/*.cbor", { query: "?url&no-inline", import: "default", eager: true });
const initial: GraphSnapshot = { phase: "idle", screen: null, limited: false, error: null };
const subscribeNone = () => () => undefined;
const noSnapshot = () => initial;
const levelNames = { work: "作品", cluster: "作者簇", node: "节点" };

export default function GraphMap({ artifactId, graphs, resolver, onSelect, onRead }: {
  artifactId: string; graphs: GraphAnalysis[]; resolver: ContentResolver; onSelect: (node: NodeAddress) => void; onRead: () => void;
}) {
  const [controller, setController] = useState<GraphController | null>(null);
  const [mountError, setMountError] = useState<Error | null>(null);
  const callbacks = useRef({ graphs, onSelect });
  callbacks.current = { graphs, onSelect };
  const snapshot = useSyncExternalStore(controller?.subscribe ?? subscribeNone, controller?.getSnapshot ?? noSnapshot, noSnapshot);
  const options = useMemo<MountGraphOptions>(() => ({
    indexId: publication.index_object_id,
    labels: publication.labels.map(label => ({ cluster: label.cluster.slice(8), objectId: label.object_id })),
    resolver, context: { languages: [...navigator.languages] },
    fetchObject: async id => {
      const url = objects[`../../../products/gamebook-demo/story.graph/${id}.cbor`];
      if (!url) throw new Error("作品缺少此图对象");
      const response = await fetch(url);
      if (!response.ok) throw new Error(`无法载入图对象（HTTP ${response.status}）`);
      return new Uint8Array(await response.arrayBuffer());
    },
    onSelect: node => {
      for (const graph of callbacks.current.graphs) {
        const picked = graph.nodes.find(candidate => candidate.id === `node:${node.id}` || candidate.id === node.id);
        if (picked) { callbacks.current.onSelect({ ...graph.reference, node: picked.id }); return; }
      }
    },
  }), [resolver]);
  const screen = snapshot.screen;
  const compatible = artifactId === publication.artifact_id;
  const activate = (node: NonNullable<typeof screen>["nodes"][number]) => {
    if (node.enter) void (node.enter.partition === undefined ? controller?.enterCluster(node.enter.cluster) : controller?.enterPartition(node.enter.cluster, node.enter.partition));
    else options.onSelect?.(node);
  };
  return <section className="graph-view" aria-label="故事结构图">
    <div className="graph-toolbar"><div><h1>故事结构图</h1><p>作者视图会显示完整结构，包含尚未走过的路线。</p></div><button className="text-button" onClick={onRead}>返回阅读</button></div>
    {compatible ? <>
      <nav className="graph-levels" aria-label="图层级"><button disabled={!screen || screen.level === "work" || snapshot.phase === "loading"} onClick={() => void controller?.showWork()}>作品概览</button>
        <button disabled={!screen || screen.level === "work" || snapshot.phase === "loading"} onClick={() => void controller?.back()}>上一层</button>
        <span data-testid="graph-level">{screen ? levelNames[screen.level] : "正在载入"}</span></nav>
      <AuthorGraph options={options} onController={setController} onError={setMountError} className="author-graph" style={{ height: 480, position: "relative" }} label="故事结构关系" />
      <p role="status" data-testid="graph-status">{snapshot.phase === "loading" ? "正在载入图……" : screen ? `${screen.nodes.length} 个节点 · ${screen.edges.length} 条关系` : "正在打开图……"}</p>
      {screen?.level === "node" ? <p className="graph-label-hint">叠点数字表示重合的节点数；可从下方列表分别检查。</p> : null}
      {snapshot.limited ? <p role="status">此层超过绘制上限，保留上一层视图。请选择较小的单元。</p> : null}
      {snapshot.error || mountError ? <p role="alert">无法显示此图。{(snapshot.error ?? mountError)?.message}</p> : null}
      {screen ? <details className="graph-list" open><summary>图中单元（{screen.nodes.length}）</summary><ul>
        {screen.nodes.slice(0, 200).map(node => <li key={node.id}><button disabled={snapshot.phase === "loading"} onClick={() => activate(node)}>{node.enter ? `进入 ${node.label}` : `检查 ${node.label}`}</button></li>)}
      </ul>{screen.nodes.length > 200 ? <p>列表显示前 200 项；在图中选择其他节点。</p> : null}</details> : null}
    </> : <p role="status">这份作品没有匹配的已发布图对象。</p>}
  </section>;
}
