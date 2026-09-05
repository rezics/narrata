import { useId } from "react";
import type { BookView, GraphAnalysis, NodeAddress, SessionView, VariableView } from "./generated/book-view";

export function Arrow({ back = false }: { back?: boolean }) {
  return <svg width="24" height="24" viewBox="0 0 24 24" fill="none" aria-hidden="true" style={back ? { transform: "rotate(180deg)" } : undefined}><path d="M4 12h15m-6-6 6 6-6 6" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>;
}

export function sameNode(a: NodeAddress, b: NodeAddress): boolean { return a.package === b.package && a.graph === b.graph && a.node === b.node; }
export function address(node: NodeAddress): string { return `${node.package}.${node.graph}.${node.node}`; }

function orderedNodes(graph: GraphAnalysis) {
  const index = new Map(graph.nodes.map(node => [node.id, node]));
  const seen = new Set<string>();
  const queue = [graph.entry];
  const ordered: GraphAnalysis["nodes"] = [];
  for (let i = 0; i < queue.length; i++) {
    const id = queue[i];
    if (!id || seen.has(id)) continue;
    seen.add(id);
    const node = index.get(id);
    if (!node) continue;
    ordered.push(node);
    for (const edge of node.edges) if (edge.target.package === graph.reference.package && edge.target.graph === graph.reference.graph && edge.kind !== "call") queue.push(edge.target.node);
  }
  return [...ordered, ...graph.nodes.filter(node => !seen.has(node.id))];
}

export function Outline({ graphs, current, selected, open, onSelect, onGraph }: {
  graphs: GraphAnalysis[]; current: NodeAddress; selected: NodeAddress | null; open: boolean;
  onSelect: (node: NodeAddress) => void; onGraph: () => void;
}) {
  return <aside className={`outline side-panel ${open ? "mobile-open" : ""}`} aria-label="叙事目录">
    <div className="panel-title"><h2>目录</h2><button className="text-button" onClick={onGraph}>结构图</button></div>
    {graphs.map(graph => <details className="graph-group" key={`${graph.reference.package}.${graph.reference.graph}`} open>
      <summary>{graph.title}</summary>
      <ol>{orderedNodes(graph).filter(n => ["narrata.content", "narrata.decision", "narrata.call"].includes(n.type_id)).map(node => {
        const key = { ...graph.reference, node: node.id };
        const active = sameNode(key, current);
        const inspected = selected && sameNode(key, selected);
        return <li key={node.id} className={`${active ? "active" : ""} ${inspected ? "inspected" : ""}`}>
          <button onClick={() => onSelect(key)} aria-current={active ? "location" : undefined} title={`查看 ${address(key)} 的连接`}>
            <span className="node-dot" />{node.label.replace(/^调用 /, "")}
          </button>
        </li>;
      })}</ol>
    </details>)}
    <p className="outline-note">选择目录节点可检查连接。<br />故事进度由正文中的行动推进。</p>
  </aside>;
}

export function Reading({ view, busy, onAction }: { view: SessionView; busy: boolean; onAction: (action: string) => void }) {
  return <article className="reading" aria-labelledby="passage-title" aria-busy={busy}>
    <div className="reading-heading"><p className="chapter-label">{view.product_title}</p><h1 id="passage-title" tabIndex={-1}>{view.title}</h1></div>
    <div className="prose">{view.paragraphs.map((text, index) => <p key={`${view.cursor}-${index}`}>{text}</p>)}</div>
    <div className="choices" aria-label="当前可执行行动">{view.actions.map(action => <div className="choice-item" key={action.id}>
      <button className="choice" disabled={busy || !action.enabled} onClick={() => onAction(action.id)} aria-describedby={!action.enabled && action.reason ? `reason-${action.id}` : undefined}>
        <span>{action.label}</span><Arrow />
      </button>
      {!action.enabled && action.reason ? <p className="choice-reason" id={`reason-${action.id}`}>{action.reason}</p> : null}
    </div>)}</div>
    <p className="reading-note">{view.finished ? "已有路线保留在旅程中，可以随时回看。" : "当前选择会保留为独立的故事分支。"}</p>
  </article>;
}

function VariableList({ values }: { values: VariableView[] }) {
  return <dl className="variable-list">{values.map(value => <div key={value.name}>
    <dt>{value.label}</dt><dd title={value.value}>{value.kind === "bool" ? value.value === "true" ? "是" : "否" : value.value}</dd>
  </div>)}</dl>;
}

export function Inspector({ book, selected, open, busy, onCheckout }: { book: BookView; selected: NodeAddress | null; open: boolean; busy: boolean; onCheckout: (id: string) => void }) {
  const view = book.view;
  const graph = book.graphs.find(g => g.reference.package === view.node.package && g.reference.graph === view.node.graph);
  const pickedGraph = selected ? book.graphs.find(g => g.reference.package === selected.package && g.reference.graph === selected.graph) : undefined;
  const picked = pickedGraph?.nodes.find(n => n.id === selected?.node);
  const branches = new Map<string, number>();
  for (const commit of view.history) if (commit.parent) branches.set(commit.parent, (branches.get(commit.parent) ?? 0) + 1);
  return <aside className={`inspector side-panel ${open ? "mobile-open" : ""}`} aria-label="旅程检查器">
    <div className="panel-title"><h2>旅程</h2></div>
    <section className="inspector-section"><h3>当前节点</h3><p className="current-location">{graph?.title} / {view.title}</p></section>
    <section className="inspector-section"><h3>变量</h3><VariableList values={view.shared} /></section>
    <section className="inspector-section"><h3>旅程时间线</h3>
      <ol className="history">{view.history.map((commit, index) => <li key={commit.id} className={`${commit.current ? "current" : ""} ${(branches.get(commit.parent ?? "") ?? 0) > 1 ? "branch" : ""}`}>
        <button disabled={busy} onClick={() => onCheckout(commit.id)} aria-current={commit.current ? "step" : undefined} title={`回到 ${commit.title} · ${commit.id.slice(0, 10)}`}>
          <span className="history-dot" /><span>{index === 0 ? "故事开始" : commit.title}<small>{index === 0 ? "旧驿站" : (branches.get(commit.parent ?? "") ?? 0) > 1 ? "另一条路线" : `第 ${index} 步`}</small></span>
        </button>
      </li>)}</ol>
    </section>
    {picked && selected ? <section className="inspector-section node-details"><h3>节点连接</h3><strong>{picked.label}</strong><code>{address(selected)}</code>
      <ul>{picked.edges.map((edge, i) => <li key={i}><span>{({ call: "调用", return: "返回后", choice: "选择", condition: "条件", next: "继续" }[edge.kind] ?? edge.kind)}{edge.label ? ` · ${edge.label}` : ""}</span><code>{address(edge.target)}</code></li>)}</ul>
      {picked.edges.length === 0 ? <p>此节点返回所属子图。</p> : null}
    </section> : null}
    <details className="advanced"><summary>运行详情</summary>
      <div className="technical-id"><span>作品</span><code>{view.artifact_id}</code></div>
      <div className="technical-id"><span>提交</span><code>{view.cursor}</code></div>
      {view.frames.map(frame => <section key={frame.instance}><h4>{frame.graph.package}.{frame.graph.graph} · 实例 {frame.instance}</h4><VariableList values={[...frame.parameters, ...frame.locals]} /></section>)}
      {book.diagnostics.map((d, i) => <p key={i}>{d.path}：{d.message}</p>)}
    </details>
  </aside>;
}

export function GraphMap({ graphs, current, onSelect, onRead }: { graphs: GraphAnalysis[]; current: NodeAddress; onSelect: (node: NodeAddress) => void; onRead: () => void }) {
  const marker = useId().replaceAll(":", "");
  let count = 0;
  const nodes = graphs.flatMap((graph, column) => orderedNodes(graph).flatMap((node, row) => {
    count++;
    return count <= 120 ? [{ node, key: { ...graph.reference, node: node.id }, x: column * 250 + 30, y: row * 102 + 62 }] : [];
  }));
  const positions = new Map(nodes.map(n => [address(n.key), n]));
  const width = Math.max(560, Math.min(graphs.length, 120) * 250 + 40);
  const height = Math.max(350, ...nodes.map(n => n.y + 100));
  return <section className="graph-view" aria-label="故事结构图">
    <div className="graph-toolbar"><div><h1>故事结构图</h1><p>连线来自实际节点契约；查看结构不会改变故事进度。</p></div><button className="text-button" onClick={onRead}>返回阅读</button></div>
    {count > 120 ? <p role="status">为保持可读性，此视图展示前 120 个节点；完整结构可通过 CLI inspect 查看。</p> : null}
    <div className="graph-scroll"><svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="group" aria-label="内容包及节点的调用、选择和返回关系">
      <defs><marker id={marker} markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto"><path d="M0 0 L7 3.5 L0 7" fill="#78918a" /></marker></defs>
      {graphs.slice(0, 120).map((g, i) => <text className="graph-package" x={i * 250 + 30} y={28} key={`${g.reference.package}.${g.reference.graph}`}>{g.title}</text>)}
      {nodes.flatMap(source => source.node.edges.map((edge, i) => {
        const target = positions.get(address(edge.target));
        if (!target) return null;
        const x1 = source.x + 190, y1 = source.y + 29, x2 = target.x, y2 = target.y + 29;
        const bend = source.x === target.x ? source.x + 228 : (x1 + x2) / 2;
        return <path key={`${address(source.key)}-${i}`} d={`M${x1},${y1} C${bend},${y1} ${bend},${y2} ${x2},${y2}`} stroke={edge.kind === "call" ? "#b76b43" : "#78918a"} strokeDasharray={edge.kind === "return" ? "5 4" : undefined} fill="none" markerEnd={`url(#${marker})`}><title>{edge.kind}: {edge.label}</title></path>;
      }))}
      {nodes.map(item => <g key={address(item.key)} className={`map-node ${sameNode(item.key, current) ? "active" : ""}`} role="button" tabIndex={0} aria-label={`检查 ${item.node.label}`} onClick={() => onSelect(item.key)} onKeyDown={e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onSelect(item.key); } }}>
        <rect x={item.x} y={item.y} width={190} height={58} rx={5} /><text x={item.x + 13} y={item.y + 24}>{Array.from(item.node.label).slice(0, 12).join("")}</text><text className="map-id" x={item.x + 13} y={item.y + 44}>{item.node.id}</text>
      </g>)}
    </svg></div>
    <p className="graph-legend">实线：继续或选择　虚线：子图返回　棕线：调用子图</p>
  </section>;
}
