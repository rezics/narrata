import { z } from "zod";
import { limits } from "./protocol";

// One object store: "active" holds the session record, "work" the pack and content packs it
// runs on, "r1-backup" the R1 record a migration replaced.
const r1Schema = z.object({
  version: z.literal(1), revision: z.uuid(), source: z.string().max(limits.r1Save),
  save: z.string().max(limits.r1Save), saved_at: z.iso.datetime(),
}).strict();
const activeSchema = z.object({
  version: z.literal(2), revision: z.uuid(), artifact_id: z.string(), session: z.string().max(limits.save), saved_at: z.iso.datetime(),
}).strict();
const workSchema = z.object({
  version: z.literal(2), artifact_id: z.string(), pack: z.instanceof(Uint8Array).refine(value => value.byteLength <= limits.pack),
  content: z.array(z.string().max(limits.content)).min(1).max(8),
}).strict();
const anyActive = z.union([activeSchema, r1Schema]);
export type R1Record = z.infer<typeof r1Schema>;
export type ActiveRecord = z.infer<typeof activeSchema>;
export type WorkRecord = z.infer<typeof workSchema>;
export type Stored = { kind: "r2"; active: ActiveRecord; work: WorkRecord } | { kind: "r1"; record: R1Record };

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

export async function readStored(): Promise<Stored | null> {
  const db = await open();
  return new Promise((resolve, reject) => {
    const store = db.transaction("session", "readonly").objectStore("session");
    const active = store.get("active");
    const work = store.get("work");
    const corrupt = () => reject(new Error("本机存档格式损坏；原记录已保留"));
    active.onerror = () => reject(active.error ?? new Error("无法读取本机存档"));
    work.onerror = () => reject(work.error ?? new Error("无法读取本机存档"));
    work.onsuccess = () => {
      const value: unknown = active.result;
      if (value === undefined) { resolve(null); return; }
      const record = anyActive.safeParse(value);
      if (!record.success) { corrupt(); return; }
      if (record.data.version === 1) { resolve({ kind: "r1", record: record.data }); return; }
      const stored = workSchema.safeParse(work.result);
      if (!stored.success || stored.data.artifact_id !== record.data.artifact_id) { corrupt(); return; }
      resolve({ kind: "r2", active: record.data, work: stored.data });
    };
  });
}

/** Replaces the active record if it is still at `expectedRevision`, together with the work it
 * runs on and the R1 record it replaces, when given. A kept R1 backup is never overwritten. */
export async function persist(record: ActiveRecord, expectedRevision: string | null, extra: { work?: WorkRecord; backup?: R1Record } = {}): Promise<void> {
  const db = await open();
  return new Promise((resolve, reject) => {
    const tx = db.transaction("session", "readwrite");
    const store = tx.objectStore("session");
    let failure: Error | null = null;
    const current = store.get("active");
    current.onsuccess = () => {
      const value: unknown = current.result;
      const parsed = value === undefined ? null : anyActive.safeParse(value);
      if (parsed && !parsed.success) { failure = new Error("本机存档格式损坏；没有覆盖原记录"); tx.abort(); return; }
      const revision = parsed?.success ? parsed.data.revision : null;
      if (revision !== expectedRevision) { failure = new Error("另一页面已更新这份旅程。请刷新后继续，当前操作未保存。"); tx.abort(); return; }
      store.put(record, "active");
      if (extra.work) store.put(extra.work, "work");
      const backup = extra.backup;
      if (backup) {
        const kept = store.get("r1-backup");
        kept.onsuccess = () => { if (kept.result === undefined) store.put(backup, "r1-backup"); };
      }
    };
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(failure ?? tx.error ?? new Error("存储事务被中止，当前操作未保存"));
    tx.onerror = () => reject(failure ?? tx.error ?? new Error("无法保存到本机，当前操作已撤回"));
  });
}
