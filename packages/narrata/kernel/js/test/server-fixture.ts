import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import { PGlite } from "@electric-sql/pglite";
import { Pool, type PoolClient } from "pg";
import {
  PostgresStore, type LibraryQuota, type SqlParameter, type SqlTransaction, type TransactionalSql,
} from "../server/postgres";
import type { CacheStore } from "../src/host";
import type { StoreFixture } from "./support/store-fixture";

export const ddl = readFileSync(new URL("../server/001-libraries.sql", import.meta.url), "utf8");
export const generousQuota: LibraryQuota = { bytes: 256 * 1024 * 1024, objects: 20_000, keys: 20_000 };
export interface TestDatabase extends TransactionalSql, SqlTransaction {
  exec(text: string): Promise<void>;
  close(): Promise<void>;
}

export async function pgliteDatabase(dataDir?: string): Promise<TestDatabase> {
  const db = dataDir === undefined ? await PGlite.create() : await PGlite.create(dataDir);
  return {
    async exec(text) { await db.exec(text); },
    async query(text, params = []) { return db.query<Record<string, unknown>>(text, [...params]); },
    transaction(mode, run) {
      return db.transaction(async (tx) => {
        await tx.query(`SET TRANSACTION ISOLATION LEVEL ${mode}`);
        return run({ query: (text, params = []) => tx.query<Record<string, unknown>>(text, [...params]) });
      });
    },
    close: () => db.close(),
  };
}

/** Tests own a random schema, leaving every pre-existing table and schema untouched. */
export async function postgresDatabase(connectionString: string): Promise<TestDatabase> {
  const pool = new Pool({ connectionString, max: 6, connectionTimeoutMillis: 10_000 });
  const schema = `narrata_test_${randomUUID().replaceAll("-", "")}`;
  try { await pool.query(`CREATE SCHEMA ${schema}`); }
  catch (error) { await pool.end(); throw error; }
  const reserve = async () => {
    const client = await pool.connect();
    try {
      await client.query(`SET search_path TO ${schema}`);
      await client.query("SET statement_timeout TO '15s'");
      return client;
    } catch (error) { client.release(true); throw error; }
  };
  const query = (client: PoolClient, text: string, params: readonly SqlParameter[] = []) =>
    client.query<Record<string, unknown>>(text, params.map((p) => p instanceof Uint8Array ? Buffer.from(p) : p));
  return {
    async exec(text) {
      const client = await reserve();
      try { await client.query(text); } finally { client.release(); }
    },
    async query(text, params) {
      const client = await reserve();
      try { return await query(client, text, params); } finally { client.release(); }
    },
    async transaction(mode, run) {
      const client = await reserve();
      let broken = false;
      try {
        await client.query(`BEGIN ISOLATION LEVEL ${mode}`);
        const result = await run({ query: (text, params) => query(client, text, params) });
        await client.query("COMMIT");
        return result;
      } catch (error) {
        try { await client.query("ROLLBACK"); } catch { broken = true; }
        throw error;
      } finally { client.release(broken); }
    },
    async close() {
      try { await pool.query(`DROP SCHEMA ${schema} CASCADE`); } finally { await pool.end(); }
    },
  };
}

export function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => { resolve = done; });
  return { promise, resolve };
}

export interface SqlFixture extends StoreFixture { readonly id: Uint8Array }

export async function sqlFixture(database: TestDatabase, quota = generousQuota): Promise<SqlFixture> {
  let id = crypto.getRandomValues(new Uint8Array(16));
  let failAfter: number | undefined;
  let onCommit: (() => void) | undefined;
  const closers: (() => void)[] = [];
  const owner = randomUUID();
  const create = () => database.query("INSERT INTO narrata_libraries (id, owner_key, work_key) VALUES ($1, $2, $3)", [id, owner, "work"]);
  await create();
  const instrumented: TransactionalSql = {
    async transaction(mode, run) {
      const result = await database.transaction(mode, (tx) => run({
        async query(text, params) {
          const result = await tx.query(text, params);
          if (text.startsWith("INSERT INTO narrata_keys") && failAfter !== undefined && --failAfter === 0) {
            failAfter = undefined;
            await tx.query("SELECT 1 / 0");
          }
          return result;
        },
      }));
      if (mode === "READ COMMITTED READ WRITE") {
        const committed = onCommit; onCommit = undefined; committed?.();
      }
      return result;
    },
  };
  const connect = (): CacheStore => {
    const store = new PostgresStore(instrumented, id, quota);
    let closed = false;
    closers.push(() => { closed = true; });
    const open = () => { if (closed) throw new Error("library handle is closed"); };
    return {
      async load(message) { open(); return store.load(message); },
      async persist(message) { open(); return store.persist(message); },
    };
  };
  const erase = () => database.transaction("READ COMMITTED READ WRITE", async (tx) => {
    await tx.query("SELECT id FROM narrata_libraries WHERE id = $1 FOR UPDATE", [id]);
    await tx.query("DELETE FROM narrata_libraries WHERE id = $1", [id]);
  });
  return {
    get id() { return id; },
    store: connect(),
    async connect() { return connect(); },
    async reopen() { for (const close of closers) close(); return connect(); },
    async clear() {
      for (const close of closers) close();
      await erase(); id = crypto.getRandomValues(new Uint8Array(16)); await create();
      return connect();
    },
    failNextPersist(afterWrites = 1) { failAfter = afterWrites; },
    observeNextCommit(committed) { onCommit = committed; },
    async dispose() { for (const close of closers) close(); onCommit = undefined; await erase(); },
  };
}
