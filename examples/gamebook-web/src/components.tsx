import type { BookView, GraphAnalysis, NodeAddress, ReaderController, VariableView } from "@rezics/narrata";
import { ReaderPath } from "@rezics/narrata/react";
import { graphTitle, nodeLabel, nodeName, type Texts } from "./texts";
import { messages, slots } from "./reader";

export function Arrow({ back = false }: { back?: boolean }) {
  return <svg width="24" height="24" viewBox="0 0 24 24" fill="none" aria-hidden="true" style={back ? { transform: "rotate(180deg)" } : undefined}><path d="M4 12h15m-6-6 6 6-6 6" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>;
}

export function sameNode(a: NodeAddress, b: NodeAddress): boolean { return a.package === b.package && a.graph === b.graph && a.node === b.node; }

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

export function Outline({ graphs, texts, current, selected, open, onSelect, onGraph }: {
  graphs: GraphAnalysis[]; texts: Texts; current: NodeAddress; selected: NodeAddress | null; open: boolean;
  onSelect: (node: NodeAddress) => void; onGraph: () => void;
}) {
  return <aside className={`outline side-panel ${open ? "mobile-open" : ""}`} aria-label="叙事目录">
    <div className="panel-title"><h2>目录</h2><button className="text-button" onClick={onGraph}>结构图</button></div>
    {graphs.map(graph => <details className="graph-group" key={`${graph.reference.package}.${graph.reference.graph}`} open>
      <summary>{graphTitle(graphs, graph.reference, texts)}</summary>
      <ol>{orderedNodes(graph).filter(n => n.kind === "passage" || n.kind === "call").map(node => {
        const key = { ...graph.reference, node: node.id };
        const active = sameNode(key, current);
        const inspected = selected && sameNode(key, selected);
        return <li key={node.id} className={`${active ? "active" : ""} ${inspected ? "inspected" : ""}`}>
          <button onClick={() => onSelect(key)} aria-current={active ? "location" : undefined} title={`查看 ${nodeName(graphs, key)} 的连接`}>
            <span className="node-dot" />{nodeLabel(node, graphs, texts).replace(/^调用 /, "")}
          </button>
        </li>;
      })}</ol>
    </details>)}
    <p className="outline-note">选择目录节点可检查连接。<br />故事进度由正文中的行动推进。</p>
  </aside>;
}

function VariableList({ values, texts }: { values: VariableView[]; texts: Texts }) {
  return <dl className="variable-list">{values.map(value => {
    const text = texts.scalar(value.value);
    return <div key={value.name}><dt>{texts.text(value.label, value.name)}</dt><dd title={text}>{text}</dd></div>;
  })}</dl>;
}

export function Inspector({ book, texts, title, selected, open, controller, busy }: { book: BookView; texts: Texts; title: string; selected: NodeAddress | null; open: boolean; controller: ReaderController; busy: boolean }) {
  const view = book.view;
  const here = view.history.find(commit => commit.current);
  const pickedGraph = selected ? book.graphs.find(g => g.reference.package === selected.package && g.reference.graph === selected.graph) : undefined;
  const picked = pickedGraph?.nodes.find(n => n.id === selected?.node);
  const edgeKinds: Record<string, string> = { call: "调用", return: "返回后", choice: "选择", condition: "条件", next: "继续" };
  return <aside className={`inspector side-panel ${open ? "mobile-open" : ""}`} aria-label="旅程检查器">
    <div className="panel-title"><h2>旅程</h2></div>
    <section className="inspector-section"><h3>当前节点</h3><p className="current-location">{here ? graphTitle(book.graphs, here.graph, texts) : null} / {title}</p></section>
    <section className="inspector-section"><h3>变量</h3><VariableList values={view.shared} texts={texts} /></section>
    <section className="inspector-section"><h3>旅程时间线</h3>
      <ReaderPath controller={controller} slots={slots} messages={messages} className="history" disabled={busy} />
    </section>
    {picked && selected ? <section className="inspector-section node-details"><h3>节点连接</h3><strong>{nodeLabel(picked, book.graphs, texts)}</strong><code>{nodeName(book.graphs, selected)}</code>
      <ul>{picked.edges.map((edge, i) => <li key={i}><span>{edgeKinds[edge.kind] ?? edge.kind}{edge.label || edge.key ? ` · ${texts.text(edge.label, edge.key ?? "")}` : ""}</span><code>{nodeName(book.graphs, edge.target)}</code></li>)}</ul>
      {picked.edges.length === 0 ? <p>{picked.kind === "return" ? "此节点返回所属子图。" : "此节点没有出边。"}</p> : null}
    </section> : null}
    <details className="advanced"><summary>运行详情</summary>
      <div className="technical-id"><span>作品</span><code>{view.artifact_id}</code></div>
      <div className="technical-id"><span>会话</span><code>{view.execution}</code></div>
      <div className="technical-id"><span>提交</span><code>{view.cursor}</code></div>
      {view.frames.map(frame => <section key={frame.instance}><h4>{frame.graph.package}.{frame.graph.graph} · 实例 {frame.instance}</h4><VariableList values={[...frame.parameters, ...frame.locals]} texts={texts} /></section>)}
      {book.diagnostics.map((d, i) => <p key={i}>{d.path}：{d.message}</p>)}
    </details>
  </aside>;
}
