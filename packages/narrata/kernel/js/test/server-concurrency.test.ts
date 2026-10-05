import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { PostgresStore, type TransactionalSql } from "../server/postgres";
import { decodeFlushReply, encodeFlush } from "../src/protocol";
import { batch, digest, key, load } from "./support/conformance";
import { ddl, deferred, generousQuota, postgresDatabase, sqlFixture, type TestDatabase } from "./server-fixture";

const url = process.env.NARRATA_POSTGRES_URL;
describe.skipIf(!url)("real PostgreSQL dual-connection locking and MVCC", () => {
  let db: TestDatabase;
  beforeAll(async () => { db = await postgresDatabase(url!); await db.exec(ddl); });
  afterAll(async () => { await db?.close(); });

  it("blocks a same-base writer on the library row, then reports the committed conflict", async () => {
    const f = await sqlFixture(db);
    const locked = deferred(); const release = deferred(); const attempting = deferred();
    let loserPid: unknown;
    const first: TransactionalSql = { transaction: (mode, run) => db.transaction(mode, (tx) => run({
      async query(text, params) {
        const result = await tx.query(text, params);
        if (text.endsWith("FOR UPDATE")) { locked.resolve(); await release.promise; }
        return result;
      },
    })) };
    const second: TransactionalSql = { transaction: (mode, run) => db.transaction(mode, (tx) => run({
      async query(text, params) {
        if (text.endsWith("FOR UPDATE")) {
          loserPid = (await tx.query("SELECT pg_backend_pid() AS pid")).rows[0]?.pid;
          attempting.resolve();
        }
        return tx.query(text, params);
      },
    })) };
    const message = (n: number) => encodeFlush({ store: f.id, batches: [batch(0, { putKeys: [[key(), new Uint8Array([n])]] })] });
    const a = new PostgresStore(first, f.id, generousQuota).persist(message(1));
    // Attach rejection handlers immediately so a broken database does not create an unhandled promise.
    void a.catch(() => undefined);
    let b: Promise<Uint8Array> | undefined;
    try {
      await locked.promise;
      b = new PostgresStore(second, f.id, generousQuota).persist(message(2));
      void b.catch(() => undefined);
      await attempting.promise;
      expect(typeof loserPid).toBe("number");
      if (typeof loserPid !== "number") throw new Error("missing PostgreSQL backend PID");
      const pid = loserPid;
      await waitUntil(async () => (await db.query("SELECT wait_event_type FROM pg_stat_activity WHERE pid = $1", [pid])).rows[0]?.wait_event_type === "Lock");
      release.resolve();
      expect(decodeFlushReply(await a)).toEqual({ persisted: 1 });
      expect(decodeFlushReply(await b)).toEqual({ conflict: { id: f.id, revision: 1 } });
      expect((await load(f.store, { keys: [key()] })).keys[0]?.[1]?.value).toEqual(new Uint8Array([1]));
    } finally {
      release.resolve(); await Promise.allSettled([a, ...(b ? [b] : [])]); await f.dispose();
    }
  });

  it("keeps library state, point reads and scans in one snapshot while another connection commits", async () => {
    const f = await sqlFixture(db);
    const snapshot = deferred(); const resume = deferred();
    const paused: TransactionalSql = { transaction: (mode, run) => db.transaction(mode, (tx) => run({
      async query(text, params) {
        const result = await tx.query(text, params);
        if (text.startsWith("SELECT id, revision, active")) { snapshot.resolve(); await resume.promise; }
        return result;
      },
    })) };
    const wanted = { keys: [key()], objects: [digest(1)],
      keyRanges: [{ lower: new Uint8Array(), upper: null, limit: 10 }],
      objectRanges: [{ lower: new Uint8Array(), upper: null, limit: 10 }] };
    const reading = load(new PostgresStore(paused, f.id, generousQuota), wanted);
    void reading.catch(() => undefined);
    try {
      await snapshot.promise;
      const reply = await f.store.persist(encodeFlush({ store: f.id, batches: [batch(0, {
        putKeys: [[key(), new Uint8Array([7])]], putObjects: [[digest(1), new Uint8Array([8])]],
      })] }));
      expect(decodeFlushReply(reply)).toEqual({ persisted: 1 });
      resume.resolve();
      const before = await reading;
      expect(before.store.revision).toBe(0);
      expect(before.keys[0]?.[1]).toBeNull(); expect(before.objects[0]?.[1]).toBeNull();
      expect(before.keyRanges[0]?.[1]).toEqual([]); expect(before.objectRanges[0]?.[1]).toEqual([]);
      const after = await load(f.store, wanted);
      expect(after.store.revision).toBe(1); expect(after.keys[0]?.[1]?.revision).toBe(1);
      expect(after.keyRanges[0]?.[1]).toHaveLength(1); expect(after.objectRanges[0]?.[1]).toHaveLength(1);
    } finally { resume.resolve(); await Promise.allSettled([reading]); await f.dispose(); }
  });
});

async function waitUntil(predicate: () => Promise<boolean>): Promise<void> {
  const deadline = Date.now() + 10_000;
  while (!(await predicate())) {
    if (Date.now() >= deadline) throw new Error("writer never entered a PostgreSQL lock wait");
    await new Promise((resolve) => setTimeout(resolve, 10));
  }
}
