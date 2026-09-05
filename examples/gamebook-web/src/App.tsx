import { useEffect, useRef, useState } from "react";
import { reader } from "./client";
import { Arrow, GraphMap, Inspector, Outline, Reading } from "./components";
import type { NodeAddress } from "./generated/book-view";
import { errorMessage, type Command, type ViewReply } from "./protocol";

function download(text: string, filename: string) {
  const url = URL.createObjectURL(new Blob([text], { type: "application/json;charset=utf-8" }));
  const link = document.createElement("a"); link.href = url; link.download = filename;
  document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function App() {
  const [state, setState] = useState<ViewReply | null>(null);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<NodeAddress | null>(null);
  const [map, setMap] = useState(false);
  const [mobilePanel, setMobilePanel] = useState<"outline" | "inspector" | null>(null);
  const sourceFile = useRef<HTMLInputElement>(null);
  const saveFile = useRef<HTMLInputElement>(null);

  function inspectNode(node: NodeAddress) {
    setSelected(node);
    if (window.matchMedia("(max-width: 920px)").matches) setMobilePanel("inspector");
  }

  async function execute(command: Command) {
    setBusy(true); setError(null);
    try {
      const reply = await reader.call(command);
      if (reply.kind === "view") {
        setState(reply);
        if (command.kind === "open") { setSelected(null); setMap(false); }
        if (["select", "checkout", "restore", "restart"].includes(command.kind)) setMobilePanel(null);
      } else if (reply.kind === "file") download(reply.text, reply.filename);
    } catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }

  useEffect(() => {
    let mounted = true;
    reader.call({ kind: "boot" }).then(reply => { if (mounted && reply.kind === "view") setState(reply); }).catch(error => { if (mounted) setError(errorMessage(error)); }).finally(() => { if (mounted) setBusy(false); });
    return () => { mounted = false; };
  }, []);

  useEffect(() => { if (state) document.title = `${state.book.view.product_title} · Narrata`; }, [state?.book.view.product_title]);

  const previousCursor = useRef<string | null>(null);
  useEffect(() => {
    const cursor = state?.book.view.cursor;
    if (!cursor) return;
    if (previousCursor.current && previousCursor.current !== cursor && !map) {
      document.getElementById("passage-title")?.focus({ preventScroll: true });
      if (window.innerWidth <= 920) window.scrollTo({ top: 0 });
    }
    previousCursor.current = cursor;
  }, [state?.book.view.cursor, map]);

  async function loadFile(file: File | undefined, kind: "open" | "restore") {
    if (!file) return;
    if (file.size > 4 * 1024 * 1024) { setError("文件超过 4 MiB，无法载入。"); return; }
    try {
      const text = await file.text();
      await execute(kind === "open" ? { kind, source: text } : { kind, save: text });
    } catch (error) { setError(errorMessage(error)); }
  }

  const book = state?.book;
  const view = book?.view;
  const rootPackage = view?.history[0]?.node.package;
  const graphs = book ? [...book.graphs].sort((a, b) => Number(b.reference.package === rootPackage) - Number(a.reference.package === rootPackage) || a.title.localeCompare(b.title, "zh-CN") || a.reference.graph.localeCompare(b.reference.graph)) : [];
  const current = view?.history.find(c => c.id === view.cursor);
  const children = view?.history.filter(c => c.parent === view.cursor) ?? [];
  const next = view?.actions.length === 1 && view.actions[0]?.enabled ? view.actions[0] : null;
  const savedTime = state?.saved_at ? new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit" }).format(new Date(state.saved_at)) : null;

  return <div className="app">
    <header className="topbar"><a className="wordmark" href="/" aria-label="Narrata 首页">Narrata</a><span className="work-title">{view?.product_title ?? "Gamebook"}</span>
      <nav aria-label="作品与存档"><button disabled={busy} onClick={() => sourceFile.current?.click()}>导入作品</button><button disabled={busy || !state} onClick={() => void execute({ kind: "export_save" })}>导出存档</button><button disabled={busy || !state} onClick={() => saveFile.current?.click()}>导入存档</button><button disabled={busy || !state} onClick={() => void execute({ kind: "restart" })}>重新开始</button></nav>
      <input ref={sourceFile} data-testid="project-file" type="file" accept=".json,application/json" hidden onChange={event => { void loadFile(event.target.files?.[0], "open"); event.target.value = ""; }} />
      <input ref={saveFile} data-testid="save-file" type="file" accept=".json,application/json" hidden onChange={event => { void loadFile(event.target.files?.[0], "restore"); event.target.value = ""; }} />
    </header>
    <div className="mobile-bar"><button aria-expanded={mobilePanel === "outline"} onClick={() => setMobilePanel(mobilePanel === "outline" ? null : "outline")}>目录 <span>⌄</span></button><button aria-expanded={mobilePanel === "inspector"} onClick={() => setMobilePanel(mobilePanel === "inspector" ? null : "inspector")}>旅程 <span>⌄</span></button></div>
    {error ? <div role="alert" className="message error"><span>{error}</span><button onClick={() => setError(null)} aria-label="关闭错误提示">×</button></div> : null}
    {state?.warning ? <div role="status" className="message warning">{state.warning}</div> : null}
    {book && view ? <div className="workspace">
      <Outline graphs={graphs} current={view.node} selected={selected} open={mobilePanel === "outline"} onSelect={inspectNode} onGraph={() => { setMap(true); setMobilePanel(null); }} />
      <main className="main-column">
        {map ? <GraphMap graphs={graphs} current={view.node} onSelect={inspectNode} onRead={() => { setMap(false); setMobilePanel(null); }} /> : <Reading view={view} busy={busy} onAction={action => void execute({ kind: "select", expected: view.cursor, action })} />}
        <footer className="reading-footer"><button className="outline-button" disabled={busy || !current?.parent} onClick={() => { if (current?.parent) void execute({ kind: "checkout", commit: current.parent }); }}><Arrow back /> 上一步</button>
          <p className="save-status" role="status"><span aria-hidden="true">{busy ? "◌" : savedTime ? "✓" : "○"}</span>{busy ? "正在处理" : savedTime ? "保存到本机" : "尚未保存到本机"}{savedTime ? <time dateTime={state.saved_at ?? undefined}>{savedTime}</time> : null}</p>
          <button className="primary-button" disabled={busy || (!next && children.length !== 1)} onClick={() => { if (next) void execute({ kind: "select", expected: view.cursor, action: next.id }); else if (children[0]) void execute({ kind: "checkout", commit: children[0].id }); }}>下一步 <Arrow /></button>
        </footer>
        <details className="project-export"><summary>作品文件</summary><button className="text-button" disabled={busy} onClick={() => void execute({ kind: "export_project" })}>导出当前作品</button><p>存档需要对应版本的作品。重新开始会回到起点并保留已有路线。</p></details>
      </main>
      <Inspector book={book} selected={selected} open={mobilePanel === "inspector"} busy={busy} onCheckout={commit => void execute({ kind: "checkout", commit })} />
    </div> : <main className="loading"><p role="status">{busy ? "正在打开故事……" : "暂时无法打开故事，请刷新后重试。"}</p></main>}
  </div>;
}
