import { afterAll, beforeAll, expect, test } from "vitest";
import { readFileSync } from "node:fs";
import { randomUUID } from "node:crypto";
import { loadAuthoring, type Publication } from "../../../web/dist/server/index.js";
import { loadRelations, queryRelations, readReaderRelations } from "../server/relations.js";
import { deferred, pgliteDatabase, postgresDatabase, type TestDatabase } from "./server-fixture.js";
import type { TransactionalSql, SqlParameter } from "../server/postgres.js";

const ddl = readFileSync(new URL("../../../tooling/crates/narrata-authoring/sql/relations-v1.sql", import.meta.url), "utf8");
const pack = new Uint8Array(readFileSync(new URL("../../../../../fixtures/compat/nodes-r2/story.narpack", import.meta.url)));
const url = process.env.NARRATA_POSTGRES_URL;
for (const real of [false, true]) {
  let db: TestDatabase, source: Publication;
  let driver: TransactionalSql;
  const params: SqlParameter[] = [];
  const suite = real && !url ? test.skip : test;
  beforeAll(async () => {
    if (real && !url) return;
    db = real ? await postgresDatabase(url!) : await pgliteDatabase();
    await db.exec(ddl);
    source = (await loadAuthoring()).publicationFromPack(pack);
    driver = { transaction: (mode, run) => db.transaction(mode, sql => run({ query: (text, values = []) => {
      params.push(...values);
      // PGlite has a single connection; concurrency evidence runs only on PostgreSQL.
      return !real && text.includes("pg_advisory_xact_lock") ? Promise.resolve({ rows: [] }) : sql.query(text, values);
    } })) };
  });
  afterAll(async () => { if (db) { source.free(); await db.close(); } });
  const prefix = real ? "PostgreSQL" : "PGlite";
  suite(`${prefix}: invalid query numbers reject before SQL or transaction`, async () => {
    let transactions = 0;
    const unused: TransactionalSql = { transaction: async () => { transactions++; throw new Error("must not reach SQL"); } };
    const row = source.relationRows("node", 0, 1)[0]; if (row?.row_type !== "node") throw new Error("missing node");
    for (const bad of [2 ** 32 + 1, 0.5, NaN, Infinity, -Infinity, -1, 4_000_001]) {
      await expect(queryRelations(unused, source, { kind: "node", node: row.node_id }, bad, 1)).rejects.toThrow();
      await expect(queryRelations(unused, source, { kind: "node", node: row.node_id }, 0, bad)).rejects.toThrow();
    }
    expect(transactions).toBe(0);
  });
  suite(`${prefix}: immutable full-row idempotence, native null/boolean and exact bounded queries`, async () => {
    const scalars = await driver.transaction("REPEATABLE READ READ ONLY", sql => sql.query("SELECT $1::boolean AS disabled, $2::boolean AS enabled, $3::text AS absent", [false, true, null]));
    expect(scalars.rows).toEqual([{ disabled: false, enabled: true, absent: null }]);
    await loadRelations(driver, source); await loadRelations(driver, source);
    expect(params).toContain(null); expect(params.some(value => typeof value === "boolean")).toBe(true);
    const node = source.relationRows("node", 0, 1)[0];
    if (!node || node.row_type !== "node") throw new Error("missing node");
    for (const query of [{ kind: "node", node: node.node_id }, { kind: "cluster", cluster: node.cluster_id }] as const) {
      expect(await queryRelations(driver, source, query)).toEqual(source.queryRelations(query));
    }
    for (const unit of source.relationRows("unit", 0, 256)) {
      if (unit.row_type !== "unit") throw new Error("wrong row");
      for (const query of [{ kind: "content", reference: { provider: unit.provider, key: unit.content_key } }, { kind: "first", unit: unit.unit_id }, { kind: "forward", unit: unit.unit_id }, { kind: "reverse", unit: unit.unit_id }] as const) {
        expect(await queryRelations(driver, source, query, 0, 1)).toEqual(source.queryRelations(query, 0, 1));
      }
    }
    for (const ending of source.relationRows("ending", 0, 256)) {
      if (ending.row_type !== "ending") throw new Error("wrong ending row");
      for (const query of [{ kind: "ending", node: ending.node_id }, { kind: "route", ending: ending.node_id }] as const) expect(await queryRelations(driver, source, query)).toEqual(source.queryRelations(query));
    }
    await expect(loadRelations(driver, new Proxy(source, { get(target, key) { return key === "relationIndexId" ? "00".repeat(32) : typeof Reflect.get(target, key) === "function" ? Reflect.get(target, key).bind(target) : Reflect.get(target, key); } }))).rejects.toThrow("conflicting projection");
  });
  suite(`${prefix}: identical index cannot hide missing, extra or altered rows`, async () => {
    await db.query("UPDATE narrative_ending SET reachable = NOT reachable");
    await expect(loadRelations(driver, source)).rejects.toThrow();
    await db.query("UPDATE narrative_ending SET reachable = NOT reachable");
    await db.query("INSERT INTO narrative_unit SELECT artifact, version, $1::bytea, provider, 'unexpected' FROM narrative_unit LIMIT 1", [new Uint8Array(32).fill(77)]);
    await expect(loadRelations(driver, source)).rejects.toThrow();
    await db.query("DELETE FROM narrative_unit WHERE content_key = 'unexpected'");
    await db.query("DELETE FROM narrative_first");
    await expect(loadRelations(driver, source)).rejects.toThrow();
    // Reset the isolated fixture; the loader never repairs an incomplete immutable set.
    await db.exec("TRUNCATE narrative_projection CASCADE");
  });
  suite(`${prefix}: middle failure rolls back all rows; sparse reader results return to Rust`, async () => {
    const failing: TransactionalSql = { transaction: (mode, run) => driver.transaction(mode, sql => run({ query: async (text, values) => {
      if (text.startsWith("INSERT INTO narrative_node")) throw new Error("injected failure");
      return sql.query(text, values);
    } })) };
    await expect(loadRelations(failing, source)).rejects.toThrow("injected");
    expect(String((await db.query("SELECT count(*) AS count FROM narrative_projection")).rows[0]?.count)).toBe("0");
    await loadRelations(driver, source);
    const { openBook } = await import("../../../web/dist/reader/index.js");
    const book = await openBook({ pack, execution: "execution:" + "01".repeat(16) });
    const evolution = (await loadAuthoring()).createEvolution();
    try {
      await evolution.registerPublication(source); await evolution.restoreSave(await book.exportSave());
      const visits = await evolution.visits(); const allowed = visits.readerRelations(source);
      try {
        await readReaderRelations(driver, allowed);
        const bad: TransactionalSql = { transaction: (mode, run) => driver.transaction(mode, sql => run({ query: async (text, values) => {
          const result = await sql.query(text, values); return text.includes("FROM narrative_node") ? { rows: result.rows.map(row => ({ ...row, kind: 99 })) } : result;
        } })) };
        await expect(readReaderRelations(bad, allowed)).rejects.toThrow();
      } finally { allowed.free(); visits.free(); }
    } finally { await evolution.close(); await book.close(); }
  });
  suite(`${prefix}: two same-artifact loaders are idempotent`, async () => {
    const race = async (second: Publication) => {
      if (!real) return Promise.allSettled([loadRelations(driver, source), loadRelations(driver, second)]);
      const acquired = deferred(), attempted = deferred(), release = deferred(); let calls = 0;
      const competing: TransactionalSql = { transaction: (mode, run) => driver.transaction(mode, sql => run({ query: async (text, values) => {
        if (!text.includes("pg_advisory_xact_lock")) return sql.query(text, values);
        if (++calls === 1) { const result = await sql.query(text, values); acquired.resolve(); await release.promise; return result; }
        attempted.resolve(); return sql.query(text, values);
      } })) };
      const first = loadRelations(competing, source);
      await acquired.promise;
      const next = loadRelations(competing, second);
      await attempted.promise;
      try {
        // A third connection observes the second connection actually blocked by the
        // first, while no projection is installed. This is cold-install contention.
        let waiting = false;
        for (let i = 0; i < 100; i++) {
          const locks = await db.query("SELECT count(*) AS count FROM pg_locks WHERE locktype = 'advisory' AND NOT granted");
          if (Number(locks.rows[0]?.count) > 0) { waiting = true; break; }
          await new Promise(done => setTimeout(done, 10));
        }
        expect(waiting).toBe(true);
        expect(String((await db.query("SELECT count(*) AS count FROM narrative_projection")).rows[0]?.count)).toBe("0");
      } finally { release.resolve(); }
      return Promise.allSettled([first, next]);
    };
    await db.exec("TRUNCATE narrative_projection CASCADE");
    expect((await race(source)).every(r => r.status === "fulfilled")).toBe(true);
    await loadRelations(driver, source); // exact complete rows after the cold race
    const conflict = new Proxy(source, { get(target, key) { return key === "relationIndexId" ? "00".repeat(32) : typeof Reflect.get(target, key) === "function" ? Reflect.get(target, key).bind(target) : Reflect.get(target, key); } });
    await db.exec("TRUNCATE narrative_projection CASCADE");
    const results = await race(conflict);
    expect(results.filter(r => r.status === "fulfilled")).toHaveLength(1);
    expect(results.filter(r => r.status === "rejected")).toHaveLength(1);
    await loadRelations(driver, source); // the winner's full rows remain unchanged
    // A/B rows coexist, with all queries bound to the supplied exact artifact.
    await db.exec("TRUNCATE narrative_projection CASCADE");
    await loadRelations(driver, source);
    const other = (await loadAuthoring()).publicationFromPack(new Uint8Array(readFileSync(new URL("../../../../../fixtures/compat/nodes-r2-proposals/story.narpack", import.meta.url))));
    try {
      expect(other.artifactId).not.toBe(source.artifactId);
      await loadRelations(driver, other);
      expect(String((await db.query("SELECT count(*) AS count FROM narrative_projection")).rows[0]?.count)).toBe("2");
      const row = source.relationRows("unit", 0, 1)[0]; if (row?.row_type !== "unit") throw new Error("missing unit");
      expect(await queryRelations(driver, source, { kind: "content", reference: { provider: row.provider, key: row.content_key } })).toEqual(source.queryRelations({ kind: "content", reference: { provider: row.provider, key: row.content_key } }));
    } finally { other.free(); }
  });
  (real && url ? test : test.skip)(`${prefix}: loader needs only SELECT and INSERT`, async () => {
    const role = `narrata_insert_${randomUUID().replaceAll("-", "")}`;
    const schema = (await db.query("SELECT current_schema() AS schema")).rows[0]?.schema;
    if (typeof schema !== "string" || !/^narrata_test_[0-9a-f]+$/.test(schema)) throw new Error("unsafe test schema");
    await db.query(`CREATE ROLE ${role}`);
    try {
      await db.query(`GRANT USAGE ON SCHEMA ${schema} TO ${role}`);
      await db.query(`GRANT SELECT, INSERT ON ALL TABLES IN SCHEMA ${schema} TO ${role}`);
      await db.exec("TRUNCATE narrative_projection CASCADE");
      const restricted: TransactionalSql = { transaction: (mode, run) => driver.transaction(mode, async sql => { await sql.query(`SET LOCAL ROLE ${role}`); return run(sql); }) };
      await loadRelations(restricted, source); await loadRelations(restricted, source);
    } finally { await db.query(`DROP OWNED BY ${role}`); await db.query(`DROP ROLE ${role}`); }
  });
}
