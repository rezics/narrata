import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { reader } from "./client";
import { GraphMap, Inspector, Outline } from "./components";
import { Reader, ReaderNavigation } from "@rezics/narrata/react";
import type { NodeAddress } from "@rezics/narrata";
import { contentKey, errorMessage, limits, type Command, type ViewReply } from "./protocol";
import { graphTitle, heading, Texts } from "./texts";
import { classes, messages, referenceReader, slots } from "./reader";

const subscribeNone = () => () => undefined;
const noSnapshot = () => null;

function download(data: string | Uint8Array, filename: string, type: string) {
  const url = URL.createObjectURL(new Blob([data as BlobPart], { type: type === "application/json" ? `${type};charset=utf-8` : type }));
  const link = document.createElement("a"); link.href = url; link.download = filename;
  document.body.append(link); link.click(); link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function App() {
  const [state, setState] = useState<ViewReply | null>(null);
  const [externalBusy, setBusy] = useState(true);
  const [binding, setBinding] = useState<ReturnType<typeof referenceReader> | null>(null);
  const snapshot = useSyncExternalStore(binding?.controller.subscribe ?? subscribeNone, binding?.controller.getSnapshot ?? noSnapshot);
  const busy = externalBusy || snapshot?.phase !== "ready";
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<NodeAddress | null>(null);
  const [map, setMap] = useState(false);
  const [mobilePanel, setMobilePanel] = useState<"outline" | "inspector" | null>(null);
  const workFiles = useRef<HTMLInputElement>(null);
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
        binding?.accept(reply);
        await binding?.controller.refresh();
        if (command.kind === "open") { setSelected(null); setMap(false); }
        if (["choose", "checkout", "restore", "restart"].includes(command.kind)) setMobilePanel(null);
      } else if (reply.kind === "file") for (const file of reply.files) download(file.data, file.filename, file.type);
    } catch (error) { setError(errorMessage(error)); }
    finally { setBusy(false); }
  }

  useEffect(() => {
    let mounted = true;
    let opened: ReturnType<typeof referenceReader> | undefined;
    reader.call({ kind: "boot" }).then(reply => {
      if (mounted && reply.kind === "view") {
        setState(reply);
        opened = referenceReader(reply, next => { if (mounted) setState(next); });
        setBinding(opened);
        void opened.controller.start();
      }
    }).catch(error => { if (mounted) setError(errorMessage(error)); }).finally(() => { if (mounted) setBusy(false); });
    return () => { mounted = false; opened?.controller.dispose(); };
  }, []);

  const texts = useMemo(() => {
    const entries = { ...state?.texts };
    for (const { item, resolution } of snapshot?.screen?.content ?? []) entries[contentKey(item.content, item.args)] = resolution.status === "ok"
      ? { ok: true, blocks: "text" in resolution.payload ? [resolution.payload.text] : resolution.payload.blocks.map(block => block.text) }
      : { ok: false, reason: resolution.status };
    return new Texts(entries);
  }, [state, snapshot?.screen]);
  const book = snapshot?.screen?.book;
  const view = book?.view;
  const productTitle = view ? texts.text(view.product.title, view.product.id) : null;
  useEffect(() => { if (productTitle) document.title = `${productTitle} · Narrata`; }, [productTitle]);

  const previousCursor = useRef<string | null>(null);
  useEffect(() => {
    const cursor = view?.cursor;
    if (!cursor) return;
    if (previousCursor.current && previousCursor.current !== cursor && !map) {
      setMobilePanel(null);
      document.getElementById("passage-title")?.focus({ preventScroll: true });
      if (window.innerWidth <= 920) window.scrollTo({ top: 0 });
    }
    previousCursor.current = cursor;
  }, [view?.cursor, map]);

  async function loadWork(files: File[]) {
    if (files.length === 0) return;
    const packs = files.filter(file => file.name.endsWith(".narpack"));
    const content = files.filter(file => !file.name.endsWith(".narpack"));
    if (packs.length !== 1 || content.length === 0) { setError("请同时选择一个构件（.narpack）和至少一个内容包（.json）。R1 作品请先用 narrata-book migrate-r1 转换。"); return; }
    const [pack] = packs;
    if (!pack || pack.size > limits.pack || content.some(file => file.size > limits.content)) { setError("文件超过 16 MiB，无法载入。"); return; }
    if (content.length > 8) { setError("一次最多载入 8 个内容包。"); return; }
    try {
      await execute({ kind: "open", pack: new Uint8Array(await pack.arrayBuffer()), content: await Promise.all(content.map(file => file.text())) });
    } catch (error) { setError(errorMessage(error)); }
  }

  async function loadSave(file: File | undefined) {
    if (!file) return;
    if (file.size > limits.save) { setError("存档超过 64 MiB，无法载入。"); return; }
    try { await execute({ kind: "restore", save: await file.text() }); } catch (error) { setError(errorMessage(error)); }
  }

  const here = view?.history.find(commit => commit.current);
  const current: NodeAddress | null = here ? { package: here.graph.package, graph: here.graph.graph, node: here.node } : null;
  const rootPackage = view?.history[0]?.graph.package;
  const graphs = book ? [...book.graphs].sort((a, b) => Number(b.reference.package === rootPackage) - Number(a.reference.package === rootPackage) || graphTitle(book.graphs, a.reference, texts).localeCompare(graphTitle(book.graphs, b.reference, texts), "zh-CN") || a.reference.graph.localeCompare(b.reference.graph)) : [];
  const title = book ? heading(book, texts) : null;

  return <div className="app">
    <header className="topbar"><a className="wordmark" href="/" aria-label="Narrata 首页">Narrata</a><span className="work-title">{productTitle ?? "Gamebook"}</span>
      <nav aria-label="作品与存档"><button disabled={busy} onClick={() => workFiles.current?.click()}>导入作品</button><button disabled={busy || !state} onClick={() => void execute({ kind: "export_save" })}>导出存档</button><button disabled={busy || !state} onClick={() => saveFile.current?.click()}>导入存档</button><button disabled={busy || !state} onClick={() => void execute({ kind: "restart" })}>重新开始</button></nav>
      <input ref={workFiles} data-testid="work-files" type="file" accept=".narpack,.json,application/json" multiple hidden onChange={event => { void loadWork([...event.target.files ?? []]); event.target.value = ""; }} />
      <input ref={saveFile} data-testid="save-file" type="file" accept=".hex,.json,text/plain,application/json" hidden onChange={event => { void loadSave(event.target.files?.[0]); event.target.value = ""; }} />
    </header>
    <div className="mobile-bar"><button aria-expanded={mobilePanel === "outline"} onClick={() => setMobilePanel(mobilePanel === "outline" ? null : "outline")}>目录 <span>⌄</span></button><button aria-expanded={mobilePanel === "inspector"} onClick={() => setMobilePanel(mobilePanel === "inspector" ? null : "inspector")}>旅程 <span>⌄</span></button></div>
    {error ? <div role="alert" className="message error"><span>{error}</span><button onClick={() => setError(null)} aria-label="关闭错误提示">×</button></div> : null}
    {state?.warning ? <div role="status" className="message warning">{state.warning}</div> : null}
    {book && view && current && title && binding ? <div className="workspace">
      <Outline graphs={graphs} texts={texts} current={current} selected={selected} open={mobilePanel === "outline"} onSelect={inspectNode} onGraph={() => { setMap(true); setMobilePanel(null); }} />
      <main className="main-column">
        {map ? <GraphMap graphs={graphs} texts={texts} current={current} onSelect={inspectNode} onRead={() => { setMap(false); setMobilePanel(null); }} />
          : <Reader controller={binding.controller} slots={slots} messages={messages} classes={classes} headingId="passage-title" showPath={false} showState={false} showNavigation={false} disabled={externalBusy} />}
        <ReaderNavigation controller={binding.controller} messages={messages} classes={classes} disabled={externalBusy} />
        <details className="project-export"><summary>作品文件</summary><button className="text-button" disabled={busy} onClick={() => void execute({ kind: "export_pack" })}>导出构件</button> · <button className="text-button" disabled={busy} onClick={() => void execute({ kind: "export_content" })}>导出内容包</button><p>作品由构件（结构）与内容包（文字）组成，导入时一并选择。存档需要对应的构件；换用另一份内容包不影响存档。重新开始会回到起点并保留已有路线。</p></details>
      </main>
      <Inspector book={book} texts={texts} title={title.text} selected={selected} open={mobilePanel === "inspector"} controller={binding.controller} busy={externalBusy} />
    </div> : <main className="loading">{binding ? <Reader controller={binding.controller} slots={slots} messages={messages} showState={false} showPath={false} showNavigation={false} /> : <p role="status">{busy ? "正在打开故事……" : "暂时无法打开故事，请刷新后重试。"}</p>}</main>}
  </div>;
}
