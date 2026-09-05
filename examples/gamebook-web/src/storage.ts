import { z } from "zod";

const recordSchema = z.object({
  version: z.literal(1), revision: z.uuid(), source: z.string().max(4 * 1024 * 1024),
  save: z.string().max(4 * 1024 * 1024), saved_at: z.iso.datetime(),
}).strict();
export type StoredSession = z.infer<typeof recordSchema>;

let database: Promise<IDBDatabase> | undefined;
function open(): Promise<IDBDatabase> {
  database ??= new Promise((resolve, reject) => {
    const request = indexedDB.open("narrata-gamebook", 1);
    request.onupgradeneeded = () => request.result.createObjectStore("session");
    request.onerror = () => reject(request.error ?? new Error("无法打开本机存储"));
    request.onblocked = () => reject(new Error("存储被其他页面占用，请关闭旧页面后重试"));
    request.onsuccess = () => { request.result.onversionchange = () => request.result.close(); resolve(request.result); };
  });
  return database;
}

export async function readStored(): Promise<StoredSession | null> {
  const db = await open();
  return new Promise((resolve, reject) => {
    const request = db.transaction("session", "readonly").objectStore("session").get("active");
    request.onerror = () => reject(request.error ?? new Error("无法读取本机存档"));
    request.onsuccess = () => {
      const value: unknown = request.result;
      if (value === undefined) { resolve(null); return; }
      const parsed = recordSchema.safeParse(value);
      if (parsed.success) resolve(parsed.data); else reject(new Error("本机存档格式损坏；原记录已保留"));
    };
  });
}

export async function persist(record: StoredSession, expectedRevision: string | null): Promise<void> {
  const db = await open();
  return new Promise((resolve, reject) => {
    const tx = db.transaction("session", "readwrite");
    const store = tx.objectStore("session");
    let failure: Error | null = null;
    const current = store.get("active");
    current.onsuccess = () => {
      const value: unknown = current.result;
      const parsed = value === undefined ? null : recordSchema.safeParse(value);
      if (parsed && !parsed.success) { failure = new Error("本机存档格式损坏；没有覆盖原记录"); tx.abort(); return; }
      const revision = parsed?.success ? parsed.data.revision : null;
      if (revision !== expectedRevision) { failure = new Error("另一页面已更新这份旅程。请刷新后继续，当前操作未保存。"); tx.abort(); return; }
      store.put(record, "active");
    };
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(failure ?? tx.error ?? new Error("存储事务被中止，当前操作未保存"));
    tx.onerror = () => reject(failure ?? tx.error ?? new Error("无法保存到本机，当前操作已撤回"));
  });
}
