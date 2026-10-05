import { errorMessage, replySchema, type Command, type Reply } from "./protocol";
import { StoreSuperseded } from "@rezics/narrata/storage";

class ReaderClient {
  private worker = new Worker(new URL("./worker.ts", import.meta.url), { type: "module" });
  private nextId = 1;
  private pending = new Map<number, { resolve: (value: Reply) => void; reject: (error: Error) => void }>();

  constructor() {
    this.worker.onmessage = (event: MessageEvent<unknown>) => {
      const parsed = replySchema.safeParse(event.data);
      if (!parsed.success) { this.fail(new Error("运行时通信格式不匹配，请刷新页面")); return; }
      const reply = parsed.data;
      const pending = this.pending.get(reply.id);
      this.pending.delete(reply.id);
      if (reply.kind === "error") pending?.reject(reply.superseded ? new StoreSuperseded() : new Error(reply.message)); else pending?.resolve(reply);
    };
    this.worker.onerror = event => this.fail(new Error(event.message || "运行线程发生错误"));
    this.worker.onmessageerror = error => this.fail(new Error(errorMessage(error)));
  }

  private fail(error: Error) { for (const p of this.pending.values()) p.reject(error); this.pending.clear(); }

  call(command: Command): Promise<Reply> {
    const id = this.nextId++;
    return new Promise((resolve, reject) => { this.pending.set(id, { resolve, reject }); this.worker.postMessage({ ...command, id }); });
  }
}

export const reader = new ReaderClient();
