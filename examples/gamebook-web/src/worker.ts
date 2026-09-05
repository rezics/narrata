import initialize, { NodeBook } from "./generated/wasm/narrata_nodes_wasm";
import defaultSource from "./generated/story.nar.json?raw";
import { decodeBook, errorMessage, requestSchema, type Reply, type Request } from "./protocol";
import { persist, readStored } from "./storage";

const initialized = initialize();
let book: NodeBook | undefined;
let revision: string | null = null;
let savedAt: string | null = null;
let storageAvailable = true;
let warning: string | null = null;

function current(): NodeBook { if (!book) throw new Error("故事尚未载入"); return book; }
function view(id: number): Reply { return { id, kind: "view", book: decodeBook(current().inspect()), saved_at: savedAt, warning }; }

async function saveBook(candidate: NodeBook): Promise<void> {
  if (!storageAvailable) return;
  const next = { version: 1 as const, revision: crypto.randomUUID(), source: candidate.source(), save: candidate.save(), saved_at: new Date().toISOString() };
  await persist(next, revision);
  revision = next.revision;
  savedAt = next.saved_at;
}

async function boot(id: number): Promise<Reply> {
  if (book) return view(id);
  await initialized;
  try {
    const stored = await readStored();
    if (stored) {
      const candidate = new NodeBook(stored.source);
      try { candidate.restore(stored.save); } catch (error) { candidate.free(); throw error; }
      book = candidate; revision = stored.revision; savedAt = stored.saved_at;
    } else {
      book = new NodeBook(defaultSource);
      await saveBook(book);
    }
  } catch (error) {
    book?.free();
    book = new NodeBook(defaultSource);
    storageAvailable = false; savedAt = null;
    warning = `无法使用本机自动存档：${errorMessage(error)}。本次进度仅保留在此页面，请及时导出存档。`;
  }
  return view(id);
}

async function change(id: number, action: (value: NodeBook) => void): Promise<Reply> {
  const engine = current();
  const before = engine.save();
  try {
    action(engine);
    // Persist first. No changed view is sent to the page before this transaction completes.
    await saveBook(engine);
  } catch (error) {
    engine.restore(before);
    throw error;
  }
  return view(id);
}

async function handle(request: Request): Promise<Reply> {
  if (request.kind === "boot") return boot(request.id);
  await initialized;
  switch (request.kind) {
    case "select": return change(request.id, value => { value.select(request.expected, request.action); });
    case "checkout": return change(request.id, value => { value.checkout(request.commit); });
    case "restore": return change(request.id, value => { value.restore(request.save); });
    case "restart": return change(request.id, value => {
      const root = decodeBook(value.inspect()).view.history[0];
      if (!root) throw new Error("找不到故事起点");
      value.checkout(root.id);
    });
    case "open": {
      const candidate = new NodeBook(request.source);
      try { decodeBook(candidate.inspect()); await saveBook(candidate); } catch (error) { candidate.free(); throw error; }
      book?.free(); book = candidate;
      return view(request.id);
    }
    case "export_save": return { id: request.id, kind: "file", text: current().save(), filename: "narrata-journey.save.json" };
    case "export_project": return { id: request.id, kind: "file", text: current().source(), filename: "narrata-story.nar.json" };
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
