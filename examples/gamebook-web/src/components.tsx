import { useId, useState } from "react";
import type { BookView, GraphAnalysis, Interaction, NodeAddress, PresentationItem, Segment, VariableView } from "./generated/book-view";
import type { Args } from "./protocol";
import { graphTitle, missing, nodeLabel, nodeName, type Texts } from "./texts";

export function Arrow({ back = false }: { back?: boolean }) {
  return <svg width="24" height="24" viewBox="0 0 24 24" fill="none" aria-hidden="true" style={back ? { transform: "rotate(180deg)" } : undefined}><path d="M4 12h15m-6-6 6 6-6 6" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>;
}

export function sameNode(a: NodeAddress, b: NodeAddress): boolean { return a.package === b.package && a.graph === b.graph && a.node === b.node; }
function address(node: NodeAddress): string { return `${node.package}.${node.graph}.${node.node}`; }

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

function Blocks({ texts, content, args }: { texts: Texts; content: Segment; args: Args }) {
  const resolved = texts.resolved(content, args);
  if (!resolved?.ok) return <p className="missing" title={resolved?.reason}>{missing(content)}</p>;
  return <>{resolved.blocks.map((text, index) => <p key={index}>{text}</p>)}</>;
}

function Item({ item, texts }: { item: PresentationItem; texts: Texts }) {
  const content = item.content;
  if (!("unit" in content)) return <h2 className="passage-subtitle">{texts.text(content, missing(content), item.args)}</h2>;
  if (item.role === "reply") return <blockquote className="reply"><Blocks texts={texts} content={content} args={item.args} /></blockquote>;
  return <Blocks texts={texts} content={content} args={item.args} />;
}

function pageKey(item: PresentationItem) { return `${item.commit}-${item.occurrence}`; }

type Choose = Extract<Interaction, { kind: "choose" }>;

function Choices({ interaction, texts, busy, onChoose }: { interaction: Choose; texts: Texts; busy: boolean; onChoose: (options: string[]) => void }) {
  const [chosen, setChosen] = useState<string[]>([]);
  const single = interaction.min === 1 && interaction.max === 1;
  const label = (option: Choose["options"][number]) => texts.text(option.label, option.key ?? option.id, interaction.args);
  const reason = (option: Choose["options"][number]) => !option.enabled && option.reason ? <p className="choice-reason" id={`reason-${option.id}`}>{texts.text(option.reason, missing(option.reason), interaction.args)}</p> : null;
  if (single) return <div className="choices" aria-label="当前可执行行动">{interaction.options.map(option => <div className="choice-item" key={option.id}>
    <button className="choice" disabled={busy || !option.enabled} onClick={() => onChoose([option.id])} aria-describedby={!option.enabled && option.reason ? `reason-${option.id}` : undefined}>
      <span>{label(option)}</span><Arrow />
    </button>
    {reason(option)}
  </div>)}</div>;
  const { min, max } = interaction;
  const hint = min === 0 ? `最多选择 ${max} 项，也可以都不选` : min === max ? `选择 ${min} 项` : `选择 ${min}–${max} 项`;
  const ready = chosen.length >= min && chosen.length <= max;
  return <fieldset className="choices multi" aria-describedby="choice-hint">
    <legend id="choice-hint">{hint}</legend>
    {interaction.options.map(option => {
      const checked = chosen.includes(option.id);
      return <div className="choice-item" key={option.id}>
        <label className={`choice choice-check ${!option.enabled ? "disabled" : ""}`}>
          <input type="checkbox" checked={checked} disabled={busy || !option.enabled || (!checked && chosen.length >= max)} aria-describedby={!option.enabled && option.reason ? `reason-${option.id}` : undefined}
            onChange={() => setChosen(checked ? chosen.filter(id => id !== option.id) : [...chosen, option.id])} />
          <span>{label(option)}</span>
        </label>
        {reason(option)}
      </div>;
    })}
    <button className="primary-button choice-submit" disabled={busy || !ready} onClick={() => onChoose(interaction.options.filter(option => chosen.includes(option.id)).map(option => option.id))}>
      {chosen.length === 0 ? "都不选，继续" : `确定（${chosen.length} 项）`} <Arrow />
    </button>
  </fieldset>;
}

export function Reading({ book, texts, title, lead, busy, onChoose }: { book: BookView; texts: Texts; title: string; lead: number; busy: boolean; onChoose: (options: string[]) => void }) {
  const view = book.view;
  const interaction = view.interaction;
  const before = book.page.slice(0, Math.max(lead, 0));
  const after = book.page.slice(lead + 1);
  return <article className="reading" aria-labelledby="passage-title" aria-busy={busy}>
    {before.length ? <div className="prose lead-in">{before.map(item => <Item key={pageKey(item)} item={item} texts={texts} />)}</div> : null}
    <div className="reading-heading"><p className="chapter-label">{texts.text(view.product.title, view.product.id)}</p><h1 id="passage-title" tabIndex={-1}>{title}</h1></div>
    <div className="prose">
      {after.map(item => <Item key={pageKey(item)} item={item} texts={texts} />)}
      {interaction.kind === "finished" && interaction.body ? <Blocks texts={texts} content={interaction.body} args={{}} /> : null}
    </div>
    {interaction.kind === "choose" ? <Choices key={view.cursor} interaction={interaction} texts={texts} busy={busy} onChoose={onChoose} /> : null}
    <p className="reading-note">{interaction.kind === "finished" ? "已有路线保留在旅程中，可以随时回看。" : "当前选择会保留为独立的故事分支。"}</p>
  </article>;
}

function VariableList({ values, texts }: { values: VariableView[]; texts: Texts }) {
  return <dl className="variable-list">{values.map(value => {
    const text = texts.scalar(value.value);
    return <div key={value.name}><dt>{texts.text(value.label, value.name)}</dt><dd title={text}>{text}</dd></div>;
  })}</dl>;
}

export function Inspector({ book, texts, title, selected, open, busy, onCheckout }: { book: BookView; texts: Texts; title: string; selected: NodeAddress | null; open: boolean; busy: boolean; onCheckout: (id: string) => void }) {
  const view = book.view;
  const here = view.history.find(commit => commit.current);
  const pickedGraph = selected ? book.graphs.find(g => g.reference.package === selected.package && g.reference.graph === selected.graph) : undefined;
  const picked = pickedGraph?.nodes.find(n => n.id === selected?.node);
  const branches = new Map<string, number>();
  for (const commit of view.history) if (commit.parent) branches.set(commit.parent, (branches.get(commit.parent) ?? 0) + 1);
  const edgeKinds: Record<string, string> = { call: "调用", return: "返回后", choice: "选择", condition: "条件", next: "继续" };
  return <aside className={`inspector side-panel ${open ? "mobile-open" : ""}`} aria-label="旅程检查器">
    <div className="panel-title"><h2>旅程</h2></div>
    <section className="inspector-section"><h3>当前节点</h3><p className="current-location">{here ? graphTitle(book.graphs, here.graph, texts) : null} / {title}</p></section>
    <section className="inspector-section"><h3>变量</h3><VariableList values={view.shared} texts={texts} /></section>
    <section className="inspector-section"><h3>旅程时间线</h3>
      <ol className="history">{view.history.map((commit, index) => {
        const passage = texts.text(commit.title, commit.key ?? commit.node);
        const branch = (branches.get(commit.parent ?? "") ?? 0) > 1;
        return <li key={commit.id} className={`${commit.current ? "current" : ""} ${branch ? "branch" : ""}`}>
          <button disabled={busy} onClick={() => onCheckout(commit.id)} aria-current={commit.current ? "step" : undefined} title={`回到 ${passage} · ${commit.id.slice(7, 17)}`}>
            <span className="history-dot" /><span>{index === 0 ? "故事开始" : passage}<small>{index === 0 ? passage : branch ? "另一条路线" : `第 ${commit.depth} 步`}</small></span>
          </button>
        </li>;
      })}</ol>
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

export function GraphMap({ graphs, texts, current, onSelect, onRead }: { graphs: GraphAnalysis[]; texts: Texts; current: NodeAddress; onSelect: (node: NodeAddress) => void; onRead: () => void }) {
  const marker = useId().replaceAll(":", "");
  let count = 0;
  const nodes = graphs.flatMap((graph, column) => orderedNodes(graph).flatMap((node, row) => {
    count++;
    return count <= 120 ? [{ node, label: nodeLabel(node, graphs, texts), key: { ...graph.reference, node: node.id }, x: column * 250 + 30, y: row * 102 + 62 }] : [];
  }));
  const positions = new Map(nodes.map(n => [address(n.key), n]));
  const width = Math.max(560, Math.min(graphs.length, 120) * 250 + 40);
  const height = Math.max(350, ...nodes.map(n => n.y + 100));
  return <section className="graph-view" aria-label="故事结构图">
    <div className="graph-toolbar"><div><h1>故事结构图</h1><p>连线来自实际节点契约；查看结构不会改变故事进度。</p></div><button className="text-button" onClick={onRead}>返回阅读</button></div>
    {count > 120 ? <p role="status">为保持可读性，此视图展示前 120 个节点；完整结构可通过 CLI inspect 查看。</p> : null}
    <div className="graph-scroll"><svg width={width} height={height} viewBox={`0 0 ${width} ${height}`} role="group" aria-label="内容包及节点的调用、选择和返回关系">
      <defs><marker id={marker} markerWidth="7" markerHeight="7" refX="6" refY="3.5" orient="auto"><path d="M0 0 L7 3.5 L0 7" fill="#78918a" /></marker></defs>
      {graphs.slice(0, 120).map((g, i) => <text className="graph-package" x={i * 250 + 30} y={28} key={`${g.reference.package}.${g.reference.graph}`}>{graphTitle(graphs, g.reference, texts)}</text>)}
      {nodes.flatMap(source => source.node.edges.map((edge, i) => {
        const target = positions.get(address(edge.target));
        if (!target) return null;
        const x1 = source.x + 190, y1 = source.y + 29, x2 = target.x, y2 = target.y + 29;
        const bend = source.x === target.x ? source.x + 228 : (x1 + x2) / 2;
        return <path key={`${address(source.key)}-${i}`} d={`M${x1},${y1} C${bend},${y1} ${bend},${y2} ${x2},${y2}`} stroke={edge.kind === "call" ? "#b76b43" : "#78918a"} strokeDasharray={edge.kind === "return" ? "5 4" : undefined} fill="none" markerEnd={`url(#${marker})`}><title>{edge.kind}: {texts.text(edge.label, edge.key ?? "")}</title></path>;
      }))}
      {nodes.map(item => <g key={address(item.key)} className={`map-node ${sameNode(item.key, current) ? "active" : ""}`} role="button" tabIndex={0} aria-label={`检查 ${item.label}`} onClick={() => onSelect(item.key)} onKeyDown={e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onSelect(item.key); } }}>
        <rect x={item.x} y={item.y} width={190} height={58} rx={5} /><text x={item.x + 13} y={item.y + 24}>{Array.from(item.label).slice(0, 12).join("")}</text><text className="map-id" x={item.x + 13} y={item.y + 44}>{item.node.key ?? item.node.id.slice(5, 17)}</text>
      </g>)}
    </svg></div>
    <p className="graph-legend">实线：继续或选择　虚线：子图返回　棕线：调用子图</p>
  </section>;
}
