/** SQL driver boundary for ADR 0022's authoritative relations-v1.sql. The host installs
 * DDL and supplies an authorized connection. Rust owns row semantics and all proof. */
import type { SqlParameter, SqlTransaction, TransactionalSql } from "./postgres.js";
import type { AuthorRelationPage, AuthorRelationQuery, RelationIndex, RelationKind, RelationRow } from "../../../web/src/generated/authoring.js";

export interface CompleteRelationSource {
  readonly artifactId: string;
  readonly relationIndexId: string;
  relationIndex(): RelationIndex;
  relationRows(kind: RelationKind, offset: number, limit: number): readonly RelationRow[];
  checkRelationRows(kind: RelationKind, offset: number, limit: number, rows: unknown): void;
  queryRelations(query: AuthorRelationQuery, offset: number, limit: number): AuthorRelationPage;
  checkRelationQuery(query: AuthorRelationQuery, offset: number, limit: number, rows: unknown): void;
}
export interface ReaderRelationSource {
  readonly artifactId: string;
  readonly indexId: string;
  readonly count: number;
  rows(offset: number, limit: number): readonly RelationRow[];
  checkRows(offset: number, limit: number, rows: unknown): void;
}

type Column = readonly [sql: string, wire: string, encoding?: "digest" | "node" | "choice-point" | "option" | "cluster" | "integer"];
const common: readonly Column[] = [["artifact", "artifact_id", "digest"], ["version", "projection_version", "integer"]];
const columns: Record<RelationKind, readonly Column[]> = {
  unit: [["unit", "unit_id", "digest"], ["provider", "provider"], ["content_key", "content_key"]],
  cluster: [["cluster", "cluster_id", "cluster"], ["unit", "unit_id", "digest"], ["package", "package"], ["graph", "graph"]],
  node: [["node", "node_id", "node"], ["package", "package"], ["graph", "graph"], ["kind", "kind", "integer"], ["unit", "unit_id", "digest"], ["first_anchor", "first_anchor"], ["last_anchor", "last_anchor"], ["cluster", "cluster_id", "cluster"]],
  edge: [["edge_no", "edge_no", "integer"], ["source_unit", "source_unit", "digest"], ["target_unit", "target_unit", "digest"], ["source_node", "source_node", "node"], ["target_node", "target_node", "node"], ["kind", "edge_kind", "integer"], ["choice_point", "choice_point", "choice-point"], ["option_id", "option", "option"]],
  entry: [["node", "node_id", "node"]],
  ending: [["node", "node_id", "node"], ["class", "class"], ["reachable", "reachable"]],
  route: [["ending_node", "ending_node", "node"], ["cluster", "cluster_id", "cluster"]],
  first: [["unit", "unit_id", "digest"], ["node", "node_id", "node"], ["distance", "distance", "integer"]],
};
const keys: Record<RelationKind, readonly string[]> = {
  unit: ["unit"], node: ["node"], edge: ["edge_no"], entry: ["node"], ending: ["node"],
  route: ["ending_node", "cluster"], cluster: ["cluster"], first: ["unit", "node"],
};
const insertOrder: readonly RelationKind[] = ["unit", "cluster", "node", "edge", "entry", "ending", "route", "first"];
const PAGE = 256;
export class RelationStoreError extends Error { override name = "RelationStoreError"; }

function bytes(text: string): Uint8Array {
  const hex = text.slice(text.lastIndexOf(":") + 1);
  if (!/^(?:[0-9a-f]{2})+$/.test(hex)) throw new RelationStoreError("invalid binary identifier");
  return Uint8Array.from(hex.match(/../g) ?? [], pair => Number.parseInt(pair, 16));
}
function hex(value: unknown): string {
  if (!(value instanceof Uint8Array)) throw new RelationStoreError("driver must decode bytea to Uint8Array");
  return [...value].map(byte => byte.toString(16).padStart(2, "0")).join("");
}
function integer(value: unknown): number {
  const text = typeof value === "bigint" ? value.toString() : value;
  if (typeof text === "number" && Number.isSafeInteger(text) && text >= 0) return text;
  if (typeof text === "string" && /^(0|[1-9][0-9]*)$/.test(text)) {
    const n = Number(text); if (Number.isSafeInteger(n)) return n;
  }
  throw new RelationStoreError("driver returned an inexact SQL integer");
}
function encode(value: unknown, column: Column): SqlParameter {
  if (value === null || typeof value === "boolean") return value;
  if (column[2] !== undefined && column[2] !== "integer") {
    if (typeof value !== "string") throw new RelationStoreError("identifier must be text");
    return bytes(value);
  }
  if (column[2] === "integer") return integer(value);
  if (typeof value !== "string") throw new RelationStoreError("text column must be text");
  return value;
}
function normalize(kind: RelationKind, row: Record<string, unknown>): Record<string, unknown> {
  const result: Record<string, unknown> = { row_type: kind };
  for (const column of [...common, ...columns[kind]]) {
    const value = row[column[0]], encoding = column[2];
    result[column[1]] = value === null ? null : encoding === "integer" ? integer(value)
      : encoding === undefined ? value : `${encoding === "digest" ? (column[0] === "artifact" ? "artifact:" : "") : `${encoding}:`}${hex(value)}`;
  }
  return result;
}
function select(kind: RelationKind): string { return `SELECT ${[...common, ...columns[kind]].map(c => c[0]).join(", ")} FROM narrative_${kind}`; }
function base(artifact: string): SqlParameter[] { return [bytes(artifact), 1]; }
async function projection(sql: SqlTransaction, artifact: string, index: string): Promise<boolean> {
  const { rows } = await sql.query("SELECT projection_index FROM narrative_projection WHERE artifact = $1::bytea AND version = $2", base(artifact));
  if (rows.length === 0) return false;
  if (rows.length !== 1 || hex(rows[0]?.projection_index) !== index) throw new RelationStoreError("conflicting projection index");
  return true;
}
async function compare(sql: SqlTransaction, source: CompleteRelationSource): Promise<void> {
  const index = source.relationIndex();
  for (const kind of insertOrder) {
    const count = integer(index.counts[kind]);
    for (let offset = 0; offset <= count; offset += PAGE) {
      const { rows } = await sql.query(`${select(kind)} WHERE artifact = $1::bytea AND version = $2 ORDER BY ${keys[kind].join(", ")} LIMIT $3 OFFSET $4`, [...base(source.artifactId), PAGE, offset]);
      source.checkRelationRows(kind, offset, PAGE, rows.map(row => normalize(kind, row)));
      if (offset + PAGE > count) break;
    }
  }
}

/** Inserts immutable rows atomically. Advisory serialization needs SELECT only, avoiding
 * UPSERT's UPDATE privileges. A retry compares every row, column and trailing row count. */
export async function loadRelations(database: TransactionalSql, source: CompleteRelationSource): Promise<void> {
  await database.transaction("READ COMMITTED READ WRITE", async sql => {
    const artifact = bytes(source.artifactId);
    const lock = new DataView(artifact.buffer, artifact.byteOffset, artifact.byteLength).getInt32(0);
    await sql.query("SELECT pg_advisory_xact_lock($1::integer, $2::integer)", [lock, 1]);
    if (await projection(sql, source.artifactId, source.relationIndexId)) { await compare(sql, source); return; }
    await sql.query("INSERT INTO narrative_projection (artifact, version, projection_index) VALUES ($1::bytea, $2, $3::bytea)", [...base(source.artifactId), bytes(source.relationIndexId)]);
    for (const kind of insertOrder) {
      const fields = [...common, ...columns[kind]];
      for (let offset = 0; ; offset += PAGE) {
        const rows = source.relationRows(kind, offset, PAGE);
        for (const row of rows) {
          // Rust produced this bounded page. Mapping into driver scalars adds no semantic proof.
          const value: Record<string, unknown> = { ...row };
          await sql.query(`INSERT INTO narrative_${kind} (${fields.map(c => c[0]).join(", ")}) VALUES (${fields.map((column, i) => `$${i + 1}${column[2] !== undefined && column[2] !== "integer" ? "::bytea" : ""}`).join(", ")})`, fields.map(column => encode(value[column[1]], column)));
        }
        if (rows.length < PAGE) break;
      }
    }
    await compare(sql, source);
  });
}

/** Reads only permitted primary keys and their required closure, never the full work.
 * Each bounded result returns to the Rust allowlist, preserving sparse edge_no values. */
export async function readReaderRelations(database: TransactionalSql, source: ReaderRelationSource): Promise<void> {
  await database.transaction("REPEATABLE READ READ ONLY", async sql => {
    if (!await projection(sql, source.artifactId, source.indexId)) throw new RelationStoreError("projection is not installed");
    for (let offset = 0; offset < source.count; offset += PAGE) {
      const actual: Record<string, unknown>[] = [];
      for (const expected of source.rows(offset, PAGE)) {
        const kind = expected.row_type, fields = [...common, ...columns[kind]];
        const value: Record<string, unknown> = { ...expected };
        const keyColumns = keys[kind].map(key => { const column = fields.find(c => c[0] === key); if (!column) throw new RelationStoreError("missing SQL mapping"); return column; });
        const { rows } = await sql.query(`${select(kind)} WHERE artifact = $1::bytea AND version = $2 AND ${keyColumns.map((column, i) => `${column[0]} = $${i + 3}${column[2] !== undefined && column[2] !== "integer" ? "::bytea" : ""}`).join(" AND ")} LIMIT 2`, [...base(source.artifactId), ...keyColumns.map(column => encode(value[column[1]], column))]);
        actual.push(...rows.map(row => normalize(kind, row)));
      }
      source.checkRows(offset, PAGE, actual);
    }
  });
}

/** Complete author queries require separate host authorization. Their first/route facts
 * retain structural meaning and never imply a reader's first visit or feasible route. */
export async function queryRelations(database: TransactionalSql, source: CompleteRelationSource, query: AuthorRelationQuery, offset = 0, limit = PAGE): Promise<AuthorRelationPage> {
  // Own one deep snapshot across connection acquisition and every subsequent await.
  // The caller remains free to reuse or edit its input while this query is pending.
  query = structuredClone(query);
  const expected = source.queryRelations(query, offset, limit); // Rust checks bounds before SQL.
  return database.transaction("REPEATABLE READ READ ONLY", async sql => {
    if (!await projection(sql, source.artifactId, source.relationIndexId)) throw new RelationStoreError("projection is not installed");
    let kind: RelationKind, where: string, params: SqlParameter[];
    switch (query.kind) {
      case "content": kind = "unit"; where = "provider = $3 AND content_key = $4"; params = [query.reference.provider, query.reference.key]; break;
      case "node": kind = "node"; where = "node = $3::bytea"; params = [bytes(query.node)]; break;
      case "ending": kind = "ending"; where = "node = $3::bytea"; params = [bytes(query.node)]; break;
      case "route": kind = "route"; where = "ending_node = $3::bytea"; params = [bytes(query.ending)]; break;
      case "first": kind = "first"; where = "unit = $3::bytea"; params = [bytes(query.unit)]; break;
      case "forward": kind = "edge"; where = "source_unit = $3::bytea"; params = [bytes(query.unit)]; break;
      case "reverse": kind = "edge"; where = "target_unit = $3::bytea"; params = [bytes(query.unit)]; break;
      case "cluster": {
        // This query has two row kinds in Rust's canonical order: node, then cluster.
        const nodes = await sql.query(`${select("node")} WHERE artifact = $1::bytea AND version = $2 AND cluster = $3::bytea ORDER BY node LIMIT $4 OFFSET $5`, [...base(source.artifactId), bytes(query.cluster), limit + 1, offset]);
        const count = await sql.query("SELECT count(*) AS count FROM narrative_node WHERE artifact = $1::bytea AND version = $2 AND cluster = $3::bytea", [...base(source.artifactId), bytes(query.cluster)]);
        const nodeCount = integer(count.rows[0]?.count);
        const actual = nodes.rows.slice(0, limit).map(row => normalize("node", row));
        if (actual.length < limit && offset <= nodeCount) {
          const cluster = await sql.query(`${select("cluster")} WHERE artifact = $1::bytea AND version = $2 AND cluster = $3::bytea LIMIT 2`, [...base(source.artifactId), bytes(query.cluster)]);
          actual.push(...cluster.rows.map(row => normalize("cluster", row)));
        }
        source.checkRelationQuery(query, offset, limit, actual); return expected;
      }
    }
    const { rows } = await sql.query(`${select(kind)} WHERE artifact = $1::bytea AND version = $2 AND ${where} ORDER BY ${keys[kind].join(", ")} LIMIT $${params.length + 3} OFFSET $${params.length + 4}`, [...base(source.artifactId), ...params, limit, offset]);
    source.checkRelationQuery(query, offset, limit, rows.map(row => normalize(kind, row)));
    return expected;
  });
}
