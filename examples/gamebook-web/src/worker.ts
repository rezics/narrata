import { z } from "zod";
import initialize, { LocalContent, NodeBook } from "./generated/wasm/narrata_nodes_wasm";
import defaultPackUrl from "./generated/story/story.narpack?url";
import defaultContent from "./generated/story/zh-Hans.json?raw";
import type { BookView, VariableView } from "./generated/book-view";
import { contentKey, decodeBook, errorMessage, requestSchema, type Args, type Content, type Reply, type Request, type Resolved } from "./protocol";
import { persist, readStored, confirmedAt, type R1Record, type ActiveRecord, type KernelRecord } from "./storage";
import { IndexedDbStore, StorageHost, StoreSuperseded } from "../../../packages/narrata/kernel/js/src/index";

const initialized = initialize();
type Work = { book: NodeBook; content: LocalContent; texts: string[]; execution: string; store?: IndexedDbStore; host?: StorageHost };
let work: Work | undefined;
let revision: string | null = null;
let savedAt: string | null = null;
let storageAvailable = true;
let warning: string | null = null;

/** A fresh UUIDv7 execution ID. */
function executionId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  let time = Date.now();
  for (let i = 5; i >= 0; i--) { bytes[i] = time % 256; time = Math.floor(time / 256); }
  bytes[6] = 0x70 | ((bytes[6] ?? 0) & 0x0f);
  bytes[8] = 0x80 | ((bytes[8] ?? 0) & 0x3f);
  return `execution:${Array.from(bytes, byte => byte.toString(16).padStart(2, "0")).join("")}`;
}

function openWork(pack: Uint8Array, texts: string[], execution = executionId()): Work {
  const book = new NodeBook(pack, execution);
  const content = new LocalContent();
  try { for (const text of texts) content.add(text); } catch (error) { book.free(); content.free(); throw error; }
  return { book, content, texts, execution };
}
function freeWork(value: Work | undefined) { value?.store?.close(); value?.book.free(); value?.content.free(); }
async function defaultWork(): Promise<Work> {
  const response = await fetch(defaultPackUrl);
  if (!response.ok) throw new Error(`无法载入默认作品（HTTP ${response.status}）`);
  return openWork(new Uint8Array(await response.arrayBuffer()), [defaultContent]);
}
function current(): Work { if (!work) throw new Error("故事尚未载入"); return work; }

const resolutionSchema = z.array(z.discriminatedUnion("status", [
  z.object({ status: z.literal("ok"), revision: z.string(), payload: z.union([
    z.object({ text: z.string() }).strict(),
    z.object({ blocks: z.array(z.object({ id: z.string(), text: z.string() }).strict()) }).strict(),
  ]) }).strict(),
  z.object({ status: z.literal("unavailable") }).strict(),
  z.object({ status: z.literal("incompatible"), reason: z.string() }).strict(),
]));
const BATCH = 4096;

/** Every content reference the page shows, with the arguments it is formatted with. */
function collect(book: BookView): { content: Content; args: Args }[] {
  const items = new Map<string, { content: Content; args: Args }>();
  const add = (content: Content | null | undefined, args: Args = {}) => { if (content) items.set(contentKey(content, args), { content, args }); };
  const variable = (value: VariableView) => { add(value.label); if (value.value.type === "ref") add(value.value.value); };
  const view = book.view;
  add(view.product.title);
  for (const item of book.page) add(item.content, item.args);
  const interaction = view.interaction;
  if (interaction.kind === "choose") for (const option of interaction.options) { add(option.label, interaction.args); add(option.reason, interaction.args); }
  else { add(interaction.title); add(interaction.body); }
  view.shared.forEach(variable);
  for (const frame of view.frames) { frame.parameters.forEach(variable); frame.locals.forEach(variable); }
  for (const commit of view.history) add(commit.title);
  for (const graph of book.graphs) { add(graph.title); for (const node of graph.nodes) { add(node.title); for (const edge of node.edges) add(edge.label); } }
  return [...items.values()];
}

function resolve(content: LocalContent, book: BookView): Record<string, Resolved> {
  const languages = [...self.navigator.languages];
  const items = collect(book);
  const texts: Record<string, Resolved> = {};
  for (let start = 0; start < items.length; start += BATCH) {
    const batch = items.slice(start, start + BATCH);
    const results = resolutionSchema.parse(JSON.parse(content.resolve(JSON.stringify({ context: { languages }, items: batch }))));
    batch.forEach((item, index) => {
      const result = results[index];
      texts[contentKey(item.content, item.args)] = result?.status === "ok"
        ? { ok: true, blocks: "text" in result.payload ? [result.payload.text] : result.payload.blocks.map(block => block.text) }
        : { ok: false, reason: result?.status === "incompatible" ? result.reason : "unavailable" };
    });
  }
  return texts;
}

async function view(id: number): Promise<Reply> {
  const value = decodeBook(await run(current(), () => current().book.inspect()));
  const { content } = current();
  // Lookahead does not write storage or change the view. Failed hints leave normal checked
  // loading in choose; a missing candidate must not prevent reading the current passage.
  try { current().book.prefetch(); } catch { /* choose will check the selected route */ }
  return { id, kind: "view", book: value, texts: resolve(content, value), saved_at: savedAt, warning };
}

async function run<T>(value: Work, operation: () => T): Promise<T> {
  if (value.host) return value.host.run(operation);
  const result = operation(); value.book.confirm_memory(); return result;
}

async function attach(value: Work, database: string): Promise<void> {
  const store = await IndexedDbStore.open(database);
  value.store = store;
  // A long history's view can visit many ancestors; the host still loads only missed keys.
  value.host = new StorageHost(store, value.book, 100_000);
  await run(value, () => value.book.open());
}

async function saveWork(next: Work, backup?: R1Record | ActiveRecord): Promise<void> {
  if (!storageAvailable) return;
  const artifact = next.book.artifact_id;
  const record: KernelRecord = { version: 3, revision: crypto.randomUUID(), artifact_id: artifact,
    execution: next.execution, database: `narrata-nodes-${next.execution}`, saved_at: new Date().toISOString() };
  await persist(record, revision, { work: { version: 2, artifact_id: artifact, pack: new Uint8Array(next.book.pack()), content: next.texts }, backup });
  revision = record.revision; savedAt = record.saved_at;
}

async function boot(id: number): Promise<Reply> {
  if (work) return view(id);
  await initialized;
  let candidate: Work | undefined;
  try {
    const stored = await readStored();
    if (stored?.kind === "kernel") {
      candidate = openWork(stored.work.pack, stored.work.content, stored.active.execution);
      if (candidate.book.artifact_id !== stored.active.artifact_id) throw new Error("本机作品与存档不匹配；原记录已保留");
      revision = stored.active.revision; savedAt = stored.active.saved_at;
      await attach(candidate, stored.active.database);
    } else if (stored?.kind === "r2") {
      candidate = openWork(stored.work.pack, stored.work.content);
      revision = stored.active.revision;
      await attach(candidate, `narrata-nodes-${candidate.execution}`);
      await run(candidate, () => candidate!.book.restore(stored.active.session));
      await saveWork(candidate, stored.active);
    } else if (stored?.kind === "r1") {
      candidate = await defaultWork(); revision = stored.record.revision;
      await attach(candidate, `narrata-nodes-${candidate.execution}`);
      await run(candidate, () => candidate!.book.migrate_r1(stored.record.save, candidate!.content, candidate!.execution));
      await saveWork(candidate, stored.record);
    } else {
      candidate = await defaultWork();
      await attach(candidate, `narrata-nodes-${candidate.execution}`);
      await saveWork(candidate);
    }
    work = candidate;
    void IndexedDbStore.requestPersistence().catch(() => undefined);
  } catch (error) {
    freeWork(candidate);
    storageAvailable = false; savedAt = null;
    warning = `无法使用本机自动存档：${errorMessage(error)}。本次进度仅保留在此页面，请及时导出存档。`;
    work = await defaultWork(); work.book.memory();
  }
  return view(id);
}

async function change(id: number, action: (book: NodeBook) => void): Promise<Reply> {
  const value = current();
  try {
    await run(value, () => action(value.book));
    savedAt = storageAvailable ? new Date().toISOString() : null;
    // The history batch is already confirmed. A failed clock update must not turn a durable
    // choice into a reported failure; the in-page clock still records this confirmation.
    if (savedAt) await confirmedAt(value.execution, savedAt).catch(() => undefined);
  } catch (error) {
    if (value.host) {
      value.book.reload();
      await run(value, () => value.book.open());
    }
    if (error instanceof StoreSuperseded || errorMessage(error).includes("stale_input")) throw new Error("另一页面已更新这份旅程。当前操作未保存，请刷新后继续。");
    throw error;
  }
  return view(id);
}

function language(text: string, index: number): string {
  try {
    const value: unknown = JSON.parse(text);
    if (value && typeof value === "object" && "language" in value && typeof value.language === "string" && /^[A-Za-z0-9-]{1,64}$/.test(value.language)) return value.language;
  } catch { /* the pack was checked when it was added */ }
  return `content-${index + 1}`;
}

async function handle(request: Request): Promise<Reply> {
  if (request.kind === "boot") return boot(request.id);
  await initialized;
  switch (request.kind) {
    case "choose": return change(request.id, book => { book.choose(request.expected, request.choice_point, request.options); });
    case "checkout": return change(request.id, book => { book.checkout(request.commit); });
    case "restore": return change(request.id, book => { book.restore(request.save); });
    case "restart": {
      const root = decodeBook(await run(current(), () => current().book.inspect())).view.history[0];
      if (!root) throw new Error("找不到故事起点");
      return change(request.id, book => book.checkout(root.id));
    }
    case "open": {
      const candidate = openWork(request.pack, request.content);
      try {
        if (storageAvailable) await attach(candidate, `narrata-nodes-${candidate.execution}`);
        else candidate.book.memory();
        decodeBook(await run(candidate, () => candidate.book.inspect())); await saveWork(candidate);
      } catch (error) { freeWork(candidate); throw error; }
      freeWork(work); work = candidate;
      return view(request.id);
    }
    case "export_save": return { id: request.id, kind: "file", files: [{ data: await run(current(), () => current().book.export()), filename: "narrata-journey.checkpoint.hex", type: "text/plain" }] };
    case "export_pack": return { id: request.id, kind: "file", files: [{ data: new Uint8Array(current().book.pack()), filename: "story.narpack", type: "application/octet-stream" }] };
    case "export_content": return { id: request.id, kind: "file", files: current().texts.map((text, index) => ({ data: text, filename: `${language(text, index)}.json`, type: "application/json" })) };
  }
}

let queue = Promise.resolve();
self.addEventListener("message", (event: MessageEvent<unknown>) => {
  queue = queue.then(async () => {
    const parsed = requestSchema.safeParse(event.data);
    if (!parsed.success) { self.postMessage({ id: 0, kind: "error", message: "无法识别的运行请求" } satisfies Reply); return; }
    try { self.postMessage(await handle(parsed.data)); }
    catch (error) { self.postMessage({ id: parsed.data.id, kind: "error", message: errorMessage(error) } satisfies Reply); }
  });
});
