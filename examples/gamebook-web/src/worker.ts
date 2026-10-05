import { z } from "zod";
import initialize, { LocalContent, NodeBook } from "./generated/wasm/narrata_nodes_wasm";
import defaultPackUrl from "./generated/story/story.narpack?url";
import defaultContent from "./generated/story/zh-Hans.json?raw";
import type { BookView, VariableView } from "./generated/book-view";
import { contentKey, decodeBook, errorMessage, requestSchema, type Args, type Content, type Reply, type Request, type Resolved } from "./protocol";
import { persist, readStored, type R1Record } from "./storage";

const initialized = initialize();
type Work = { book: NodeBook; content: LocalContent; texts: string[] };
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

function openWork(pack: Uint8Array, texts: string[]): Work {
  const book = new NodeBook(pack, executionId());
  const content = new LocalContent();
  try { for (const text of texts) content.add(text); } catch (error) { book.free(); content.free(); throw error; }
  return { book, content, texts };
}
function freeWork(value: Work | undefined) { value?.book.free(); value?.content.free(); }
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

function view(id: number): Reply {
  const { book, content } = current();
  const value = decodeBook(book.inspect());
  return { id, kind: "view", book: value, texts: resolve(content, value), saved_at: savedAt, warning };
}

async function save(next: Work, extra: { work?: boolean; backup?: R1Record } = {}): Promise<void> {
  if (!storageAvailable) return;
  const artifact = next.book.artifact_id;
  const record = { version: 2 as const, revision: crypto.randomUUID(), artifact_id: artifact, session: next.book.export(), saved_at: new Date().toISOString() };
  await persist(record, revision, {
    work: extra.work ? { version: 2, artifact_id: artifact, pack: new Uint8Array(next.book.pack()), content: next.texts } : undefined,
    backup: extra.backup,
  });
  revision = record.revision;
  savedAt = record.saved_at;
}

async function boot(id: number): Promise<Reply> {
  if (work) return view(id);
  await initialized;
  try {
    const stored = await readStored();
    if (stored?.kind === "r2") {
      const candidate = openWork(stored.work.pack, stored.work.content);
      try { candidate.book.restore(stored.active.session); } catch (error) { freeWork(candidate); throw error; }
      work = candidate; revision = stored.active.revision; savedAt = stored.active.saved_at;
    } else if (stored?.kind === "r1") {
      // An R1 record is rebuilt on the default work, which was migrated from the R1 one. The
      // R1 record is kept beside the new one; on failure nothing is written.
      const candidate = await defaultWork();
      try {
        try { candidate.book.migrate_r1(stored.record.save, candidate.content, executionId()); }
        catch (error) { throw new Error(`R1 存档无法迁移（${errorMessage(error)}），原记录已保留`); }
        revision = stored.record.revision;
        await save(candidate, { work: true, backup: stored.record });
      } catch (error) { freeWork(candidate); revision = null; throw error; }
      work = candidate;
    } else {
      work = await defaultWork();
      await save(work, { work: true });
    }
  } catch (error) {
    freeWork(work);
    work = undefined;
    storageAvailable = false; savedAt = null;
    warning = `无法使用本机自动存档：${errorMessage(error)}。本次进度仅保留在此页面，请及时导出存档。`;
    work = await defaultWork();
  }
  return view(id);
}

async function change(id: number, action: (book: NodeBook) => void): Promise<Reply> {
  const { book } = current();
  const before = book.export();
  try {
    action(book);
    // Persist first. No changed view is sent to the page before this transaction completes.
    await save(current());
  } catch (error) {
    book.restore(before);
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
    case "restart": return change(request.id, book => {
      const root = decodeBook(book.inspect()).view.history[0];
      if (!root) throw new Error("找不到故事起点");
      book.checkout(root.id);
    });
    case "open": {
      const candidate = openWork(request.pack, request.content);
      try { decodeBook(candidate.book.inspect()); await save(candidate, { work: true }); } catch (error) { freeWork(candidate); throw error; }
      freeWork(work); work = candidate;
      return view(request.id);
    }
    case "export_save": return { id: request.id, kind: "file", files: [{ data: current().book.export(), filename: "narrata-journey.save.json", type: "application/json" }] };
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
