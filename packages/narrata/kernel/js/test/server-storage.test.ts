import { readFileSync } from "node:fs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import {
  LibraryQuotaExceeded, LibraryUnavailable, PostgresStore, SqlStoreFormatError,
  type TransactionalSql,
} from "../server/postgres";
import { decodeFlush, decodeFlushReply, decodeLoaded, encodeFlush, encodeLoadRequest } from "../src/protocol";
import { batch, conformanceCases, digest, key, load, runConformanceCase } from "./support/conformance";
import { ddl, deferred, generousQuota, pgliteDatabase, postgresDatabase, sqlFixture, type TestDatabase } from "./server-fixture";

const empty = encodeLoadRequest({ keys: [], objects: [], keyRanges: [], objectRanges: [] });
const put = (id: Uint8Array, base = 0) => encodeFlush({ store: id, batches: [batch(base, {
  putObjects: [[digest(1), new Uint8Array([2])]], putKeys: [[key(), new Uint8Array([3])]],
})] });

const postgresUrl = process.env.NARRATA_POSTGRES_URL;
if (!postgresUrl) console.info("SKIPPED: real PostgreSQL conformance and dual-connection tests; set NARRATA_POSTGRES_URL. PGlite does not prove concurrency or WAL durability.");

for (const [name, create] of [
  ["PGlite (single connection; sequential CAS)", pgliteDatabase],
  ["PostgreSQL", () => postgresDatabase(postgresUrl!)],
] as const) {
  describe.skipIf(name === "PostgreSQL" && !postgresUrl)(`server CacheStore: ${name}`, () => {
    let db: TestDatabase;
    beforeAll(async () => { db = await create(); await db.exec(ddl); });
    afterAll(async () => { await db?.close(); });
    const factory = () => sqlFixture(db);

    for (const test of conformanceCases) {
      it(test.name, () => runConformanceCase(test.name, factory));
    }

    it("installs exactly the ADR reference tables and constraints", async () => {
      const f = await factory();
      try {
        const other = new Uint8Array(16).fill(10);
        for (const [text, params] of [
          ["INSERT INTO narrata_libraries (id, owner_key, work_key) SELECT $1, owner_key, work_key FROM narrata_libraries WHERE id = $2", [other, f.id]],
          ["INSERT INTO narrata_libraries (id, owner_key, work_key) VALUES ($1, 'invalid', 'work')", [new Uint8Array(15)]],
          ["UPDATE narrata_libraries SET active = false WHERE id = $1", [f.id]],
          ["UPDATE narrata_libraries SET revision = -1 WHERE id = $1", [f.id]],
          ["UPDATE narrata_libraries SET revision = 9007199254740992 WHERE id = $1", [f.id]],
          ["INSERT INTO narrata_objects VALUES ($1, $2, $3)", [other, digest(1), new Uint8Array()]],
          ["INSERT INTO narrata_objects VALUES ($1, $2, $3)", [f.id, new Uint8Array(31), new Uint8Array()]],
          ["INSERT INTO narrata_objects VALUES ($1, $2, $3)", [f.id, digest(1), new Uint8Array(16777217)]],
          ["INSERT INTO narrata_keys VALUES ($1, $2, $3, 1)", [f.id, new Uint8Array(1), new Uint8Array()]],
          ["INSERT INTO narrata_keys VALUES ($1, $2, $3, 1)", [f.id, new Uint8Array(1027), new Uint8Array()]],
          ["INSERT INTO narrata_keys VALUES ($1, $2, $3, 1)", [f.id, key(), new Uint8Array(16777217)]],
          ["INSERT INTO narrata_keys VALUES ($1, $2, $3, 0)", [f.id, key(), new Uint8Array()]],
          ["INSERT INTO narrata_keys VALUES ($1, $2, $3, 9007199254740992)", [f.id, key(), new Uint8Array()]],
        ] as const) await expect(db.query(text, params)).rejects.toThrow();

        // Non-active instances can share the owner/work route when they have an expiry.
        await db.query("INSERT INTO narrata_libraries (id, owner_key, work_key, active, expires_at) SELECT $1, owner_key, work_key, false, now() FROM narrata_libraries WHERE id = $2", [other, f.id]);
        await db.query("DELETE FROM narrata_libraries WHERE id = $1", [other]);
        await f.store.persist(put(f.id));
        await db.transaction("READ COMMITTED READ WRITE", async (tx) => {
          await tx.query("SELECT id FROM narrata_libraries WHERE id = $1 FOR UPDATE", [f.id]);
          await tx.query("DELETE FROM narrata_libraries WHERE id = $1", [f.id]);
        });
        for (const table of ["narrata_keys", "narrata_objects"]) {
          expect(String((await db.query(`SELECT count(*) AS n FROM ${table} WHERE library_id = $1`, [f.id])).rows[0]?.n)).toBe("0");
        }
        await expect(f.store.load(empty)).rejects.toBeInstanceOf(LibraryUnavailable);
        await expect(f.store.persist(put(f.id))).rejects.toBeInstanceOf(LibraryUnavailable);
      } finally { await f.dispose(); }
    });

    it("refuses inactive bound handles without creating or resurrecting a library", async () => {
      const f = await factory();
      try {
        await db.query("UPDATE narrata_libraries SET active = false, expires_at = now() WHERE id = $1", [f.id]);
        await expect(f.store.persist(put(f.id))).rejects.toBeInstanceOf(LibraryUnavailable);
        await expect(f.store.load(empty)).rejects.toBeInstanceOf(LibraryUnavailable);
      } finally { await f.dispose(); }
    });

    it("checks final byte, object and key quotas atomically and counts net growth", async () => {
      const f = await factory();
      try {
        // One digest + one object byte + two key bytes + one value byte = 36.
        for (const quota of [{ ...generousQuota, bytes: 35 }, { ...generousQuota, objects: 0 }, { ...generousQuota, keys: 0 }]) {
          const store = new PostgresStore(db, f.id, quota);
          await expect(store.persist(put(f.id))).rejects.toBeInstanceOf(LibraryQuotaExceeded);
          const after = await load(f.store, { keys: [key()], objects: [digest(1)] });
          expect(after.store.revision).toBe(0); expect(after.keys[0]?.[1]).toBeNull(); expect(after.objects[0]?.[1]).toBeNull();
        }
        const quota = { bytes: 36, objects: 1, keys: 1 };
        const identity = Buffer.from(f.id);
        const store = new PostgresStore(db, identity, quota);
        identity.fill(0); quota.bytes = 0;
        expect(decodeFlushReply(await store.persist(put(f.id)))).toEqual({ persisted: 1 });
        const message = encodeFlush({ store: f.id, batches: [
          batch(1, { deleteKeys: [key()], deleteObjects: [digest(1)] }),
          batch(2, { putKeys: [[key(1), new Uint8Array()]], putObjects: [[digest(2), new Uint8Array()]] }),
          batch(3, { putObjects: [[digest(2), new Uint8Array([99])]] }),
        ] });
        expect(decodeFlushReply(await store.persist(message))).toEqual({ persisted: 4 });
        expect((await load(store, { objects: [digest(2)] })).objects[0]?.[1]).toEqual(new Uint8Array());
      } finally { await f.dispose(); }
    });

    it("bounds database revision conversions and accepts the exact safe maximum", async () => {
      const f = await factory();
      try {
        for (const revision of ["9007199254740991", 9007199254740991n, Number.MAX_SAFE_INTEGER]) {
          const store = new PostgresStore(replaceState(db, revision), f.id, generousQuota);
          expect(decodeLoaded(await store.load(empty)).store.revision).toBe(Number.MAX_SAFE_INTEGER);
        }
        for (const revision of ["9007199254740992", 9007199254740992n, 9007199254740992, "1.5", "-1", " 1", "1e0", NaN, Infinity, undefined]) {
          const store = new PostgresStore(replaceState(db, revision), f.id, generousQuota);
          await expect(store.load(empty)).rejects.toBeInstanceOf(SqlStoreFormatError);
          await expect(store.persist(put(f.id))).rejects.toBeInstanceOf(SqlStoreFormatError);
        }
        await db.query("UPDATE narrata_libraries SET revision = $2 WHERE id = $1", [f.id, Number.MAX_SAFE_INTEGER - 1]);
        expect(decodeFlushReply(await f.store.persist(put(f.id, Number.MAX_SAFE_INTEGER - 1)))).toEqual({ persisted: Number.MAX_SAFE_INTEGER });
        await expect(f.store.persist(encodeFlush({ store: f.id, batches: [batch(Number.MAX_SAFE_INTEGER, {
          revision: Number.MAX_SAFE_INTEGER,
        })] }))).rejects.toThrow();
        expect((await load(f.store, { keys: [key()] })).keys[0]?.[1]?.revision).toBe(Number.MAX_SAFE_INTEGER);
      } finally { await f.dispose(); }
    });

    it("fails closed when SQL key revisions or binary values do not match the protocol", async () => {
      const f = await factory();
      try {
        await db.query("INSERT INTO narrata_keys VALUES ($1, $2, $3, 1)", [f.id, key(), new Uint8Array()]);
        await expect(load(f.store, { keys: [key()] })).rejects.toBeInstanceOf(SqlStoreFormatError);
        await expect(new PostgresStore(replaceState(db, 0, "not bytea"), f.id, generousQuota).load(empty)).rejects.toBeInstanceOf(SqlStoreFormatError);
      } finally { await f.dispose(); }
    });

    it("rejects an oversized scan before fetching any page values", async () => {
      const f = await factory();
      try {
        for (let n = 0; n < 5; n++) {
          await db.query("INSERT INTO narrata_keys VALUES ($1, $2, decode(repeat('00', 16777216), 'hex'), 1)", [f.id, key(n)]);
        }
        await db.query("UPDATE narrata_libraries SET revision = 1 WHERE id = $1", [f.id]);
        let fetchedValues = 0;
        const observed: TransactionalSql = { transaction: (mode, run) => db.transaction(mode, (tx) => run({
          async query(text, params) {
            if (text.startsWith("SELECT value, revision")) fetchedValues++;
            return tx.query(text, params);
          },
        })) };
        const store = new PostgresStore(observed, f.id, generousQuota);
        await expect(load(store, { keyRanges: [{ lower: new Uint8Array(), upper: null, limit: 5 }] })).rejects.toThrow("host limit");
        expect(fetchedValues).toBe(0);
        expect((await load(f.store)).store.revision).toBe(1);
      } finally { await f.dispose(); }
    });

    it("uses one snapshot for loads and waits for commit; a commit-time error never confirms", async () => {
      const f = await factory();
      try {
        const modes: string[] = [];
        const pending = deferred(); const reached = deferred();
        const delayed: TransactionalSql = {
          async transaction(mode, run) {
            modes.push(mode);
            return db.transaction(mode, async (tx) => {
              const reply = await run(tx); reached.resolve(); await pending.promise; return reply;
            });
          },
        };
        let confirmed = false;
        const writing = new PostgresStore(delayed, f.id, generousQuota).persist(put(f.id)).then((r) => { confirmed = true; return r; });
        try { await reached.promise; expect(confirmed).toBe(false); }
        finally { pending.resolve(); }
        expect(decodeFlushReply(await writing)).toEqual({ persisted: 1 });
        await new PostgresStore(delayed, f.id, generousQuota).load(empty);
        expect(modes).toEqual(["READ COMMITTED READ WRITE", "REPEATABLE READ READ ONLY"]);

        // This deferred constraint passes each INSERT and fails at COMMIT itself.
        const table = `commit_failure_${Buffer.from(f.id).toString("hex")}`;
        await db.query(`CREATE TABLE ${table} (n integer UNIQUE DEFERRABLE INITIALLY DEFERRED)`);
        const failing: TransactionalSql = {
          transaction: (mode, run) => db.transaction(mode, async (tx) => {
            const reply = await run(tx);
            await tx.query(`INSERT INTO ${table} VALUES (1), (1)`);
            return reply;
          }),
        };
        try {
          await expect(new PostgresStore(failing, f.id, generousQuota).persist(put(f.id, 1))).rejects.toThrow();
          expect((await load(f.store)).store.revision).toBe(1);
        } finally { await db.query(`DROP TABLE ${table}`); }
      } finally { await f.dispose(); }
    });

    it("reads frozen protocol v1 vectors and handles an unchanged frozen flush", async () => {
      const f = await factory();
      try {
        const corpus = new URL("../../../../../fixtures/compat/browser-storage-v1/", import.meta.url);
        const vectors: unknown = JSON.parse(readFileSync(new URL("protocol-vectors.json", corpus), "utf8"));
        if (typeof vectors !== "object" || vectors === null || !("flush" in vectors) ||
          typeof vectors.flush !== "object" || vectors.flush === null || !("hex" in vectors.flush)) {
          throw new Error("frozen protocol vectors have no flush bytes");
        }
        const frozen = new Uint8Array(readFileSync(new URL("flush.cbor", corpus)));
        expect(vectors.flush.hex).toBe(Buffer.from(frozen).toString("hex"));
        const flush = decodeFlush(frozen);
        await db.query("UPDATE narrata_libraries SET id = $2, revision = $3 WHERE id = $1", [f.id, flush.store, flush.batches[0]!.base]);
        try {
          const store = new PostgresStore(db, flush.store, generousQuota);
          const persisted = new Uint8Array(readFileSync(new URL("persisted.cbor", corpus)));
          expect(await store.persist(frozen)).toEqual(persisted);
          const loaded = decodeLoaded(await store.load(new Uint8Array(readFileSync(new URL("loadRequest.cbor", corpus)))));
          expect(loaded.store.revision).toBe(flush.batches.at(-1)!.revision);
          expect(decodeFlushReply(await store.persist(frozen))).toEqual({ conflict: loaded.store });
        } finally { await db.query("DELETE FROM narrata_libraries WHERE id = $1", [flush.store]); }
      } finally { await f.dispose(); }
    });
  });
}

it("requires finite quotas and a valid library identity", () => {
  const unavailable: TransactionalSql = { async transaction() { throw new Error("unused"); } };
  for (const quota of [{ ...generousQuota, bytes: Infinity }, { ...generousQuota, objects: -1 }, { ...generousQuota, keys: 0.5 }]) {
    expect(() => new PostgresStore(unavailable, new Uint8Array(16), quota)).toThrow(RangeError);
  }
  expect(() => new PostgresStore(unavailable, new Uint8Array(15), generousQuota)).toThrow(SqlStoreFormatError);
});

function replaceState(db: TransactionalSql, revision: unknown, id?: unknown): TransactionalSql {
  return { transaction: (mode, run) => db.transaction(mode, (tx) => run({
    async query(text, params) {
      const result = await tx.query(text, params);
      return text.startsWith("SELECT id, revision, active")
        ? { rows: result.rows.map((row) => ({ ...row, revision, ...(id === undefined ? {} : { id }) })) }
        : result;
    },
  })) };
}
