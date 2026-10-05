/** ADR 0021 reference host. The application installs SQL and supplies authentication and connections. */
import { equalBytes } from "../src/cbor";
import type { CacheStore } from "../src/host";
import { decodeHostFlush, decodeHostLoadRequest, encodeHostLoaded, HOST_LIMITS } from "../src/limits";
import {
  DIGEST_BYTES, STORE_ID_BYTES, type KeyRecord, type Loaded, type Range, type StoreState,
  encodeFlushReply, ProtocolError,
} from "../src/protocol";

export type SqlParameter = Uint8Array | number | string;
export interface SqlTransaction {
  /** Return driver-decoded bytea as Uint8Array (including Buffer); bigint may be string or bigint. */
  query(text: string, params?: readonly SqlParameter[]): Promise<{ rows: readonly Record<string, unknown>[] }>;
}

export type TransactionMode = "REPEATABLE READ READ ONLY" | "READ COMMITTED READ WRITE";
export interface TransactionalSql {
  /**
   * Reserve one connection for the callback, BEGIN in this mode, ROLLBACK on any failure,
   * and resolve only after COMMIT succeeds. Do not use pool.query inside the callback.
   * The authoritative server must have fsync and synchronous_commit enabled.
   */
  transaction<T>(mode: TransactionMode, run: (sql: SqlTransaction) => Promise<T>): Promise<T>;
}

export interface LibraryQuota {
  /** Object digests + object bytes + stored keys + key values; excludes SQL row overhead. */
  bytes: number;
  objects: number;
  keys: number;
}

export class LibraryUnavailable extends Error { override name = "LibraryUnavailable"; }
export class LibraryQuotaExceeded extends Error { override name = "LibraryQuotaExceeded"; }
export class SqlStoreFormatError extends Error { override name = "SqlStoreFormatError"; }

function unsigned(value: unknown): bigint {
  if (typeof value === "bigint" && value >= 0n) return value;
  if (typeof value === "number" && Number.isSafeInteger(value) && value >= 0) return BigInt(value);
  if (typeof value === "string" && /^(0|[1-9][0-9]*)$/.test(value)) return BigInt(value);
  throw new SqlStoreFormatError("SQL counter is not an unsigned integer");
}

function revision(value: unknown): number {
  const integer = unsigned(value);
  if (integer > BigInt(Number.MAX_SAFE_INTEGER)) throw new SqlStoreFormatError("SQL revision exceeds the safe range");
  return Number(integer);
}

function binary(value: unknown, minimum: number, maximum: number): Uint8Array {
  if (!(value instanceof Uint8Array) || value.length < minimum || value.length > maximum) {
    throw new SqlStoreFormatError("SQL bytea has an invalid type or length");
  }
  return value;
}

function record(row: Record<string, unknown>, state: StoreState): KeyRecord {
  const rev = revision(row.revision);
  if (rev === 0 || rev > state.revision) throw new SqlStoreFormatError("key revision is outside the library revision");
  return { value: binary(row.value, 0, HOST_LIMITS.valueBytes), revision: rev };
}

/** Bound to the authenticated library instance, never to client-supplied owner/work fields. */
export class PostgresStore implements CacheStore {
  private readonly id: Uint8Array;
  private readonly quota: LibraryQuota;

  constructor(private readonly database: TransactionalSql, libraryId: Uint8Array, quota: LibraryQuota) {
    // Buffer.slice() shares memory, so copy through Uint8Array even for Node driver ids.
    this.id = Uint8Array.from(binary(libraryId, STORE_ID_BYTES, STORE_ID_BYTES));
    for (const maximum of [quota.bytes, quota.objects, quota.keys]) {
      if (!Number.isSafeInteger(maximum) || maximum < 0) throw new RangeError("library quotas must be finite safe unsigned integers");
    }
    this.quota = { ...quota };
  }

  private async state(sql: SqlTransaction, lock: boolean): Promise<StoreState> {
    const { rows } = await sql.query(
      `SELECT id, revision, active FROM narrata_libraries WHERE id = $1${lock ? " FOR UPDATE" : ""}`, [this.id]);
    const row = rows[0];
    if (row === undefined || row.active === false) throw new LibraryUnavailable("library is missing or no longer active");
    if (rows.length !== 1 || row.active !== true) throw new SqlStoreFormatError("invalid library row");
    const id = binary(row.id, STORE_ID_BYTES, STORE_ID_BYTES);
    if (!equalBytes(id, this.id)) throw new SqlStoreFormatError("SQL returned a different library");
    return { id, revision: revision(row.revision) };
  }

  async load(message: Uint8Array): Promise<Uint8Array> {
    const wanted = decodeHostLoadRequest(message);
    return this.database.transaction("REPEATABLE READ READ ONLY", async (sql) => {
      const store = await this.state(sql, false);
      const loaded: Loaded = { store, keys: [], objects: [], keyRanges: [], objectRanges: [] };
      let replyBytes = 0;
      const reserve = (bytes: number) => {
        replyBytes += bytes;
        if (replyBytes > HOST_LIMITS.batchBytes) throw new ProtocolError("loaded value bytes exceeds host limit");
      };
      for (const key of wanted.keys) {
        const { rows } = await sql.query("SELECT value, revision FROM narrata_keys WHERE library_id = $1 AND key = $2", [this.id, key]);
        const entry = rows[0] === undefined ? null : record(rows[0], store);
        if (entry !== null) reserve(entry.value.length);
        loaded.keys.push([key, entry]);
      }
      for (const digest of wanted.objects) {
        const { rows } = await sql.query("SELECT bytes FROM narrata_objects WHERE library_id = $1 AND digest = $2", [this.id, digest]);
        const bytes = rows[0] === undefined ? null : binary(rows[0].bytes, 0, HOST_LIMITS.valueBytes);
        if (bytes !== null) reserve(bytes.length);
        loaded.objects.push([digest, bytes]);
      }
      for (const range of wanted.keyRanges) {
        const rows = await this.page(sql, "keys", range);
        // Check the whole page's lengths before a driver can materialize up to 1,024 large values.
        for (const row of rows) {
          const bytes = unsigned(row.value_bytes);
          if (bytes > BigInt(HOST_LIMITS.valueBytes)) throw new SqlStoreFormatError("SQL key value exceeds host limit");
          reserve(Number(bytes));
        }
        const entries: [Uint8Array, KeyRecord][] = [];
        for (const row of rows) {
          const key = binary(row.key, 2, HOST_LIMITS.keyBytes + 2);
          const { rows: values } = await sql.query("SELECT value, revision FROM narrata_keys WHERE library_id = $1 AND key = $2", [this.id, key]);
          if (values[0] === undefined) throw new SqlStoreFormatError("key disappeared inside the load snapshot");
          const entry = record(values[0], store);
          if (BigInt(entry.value.length) !== unsigned(row.value_bytes)) throw new SqlStoreFormatError("key length changed inside the load snapshot");
          entries.push([key, entry]);
        }
        loaded.keyRanges.push([range, entries]);
      }
      for (const range of wanted.objectRanges) {
        const rows = await this.page(sql, "objects", range);
        loaded.objectRanges.push([range, rows.map((row) => binary(row.digest, DIGEST_BYTES, DIGEST_BYTES))]);
      }
      return encodeHostLoaded(loaded);
    });
  }

  private async page(sql: SqlTransaction, table: "keys" | "objects", range: Range) {
    const column = table === "keys" ? "key" : "digest";
    const fields = table === "keys" ? "key, octet_length(value) AS value_bytes" : "digest";
    const params: SqlParameter[] = [this.id, range.lower];
    const upper = range.upper === null ? "" : ` AND ${column} < $${params.push(range.upper)}`;
    const limit = params.push(range.limit);
    // Identifiers come only from the two literals above; every bytea stays parameterized.
    return (await sql.query(`SELECT ${fields} FROM narrata_${table} WHERE library_id = $1 AND ${column} >= $2${upper} ORDER BY ${column} LIMIT $${limit}`, params)).rows;
  }

  async persist(message: Uint8Array): Promise<Uint8Array> {
    const flush = decodeHostFlush(message);
    const reply = await this.database.transaction("READ COMMITTED READ WRITE", async (sql) => {
      const state = await this.state(sql, true);
      if (!equalBytes(state.id, flush.store) || state.revision !== flush.batches[0]!.base) {
        return { conflict: state };
      }
      for (const batch of flush.batches) {
        for (const [digest, bytes] of batch.putObjects) {
          await sql.query("INSERT INTO narrata_objects (library_id, digest, bytes) VALUES ($1, $2, $3) ON CONFLICT (library_id, digest) DO NOTHING", [this.id, digest, bytes]);
        }
        for (const digest of batch.deleteObjects) {
          await sql.query("DELETE FROM narrata_objects WHERE library_id = $1 AND digest = $2", [this.id, digest]);
        }
        for (const [key, value] of batch.putKeys) {
          await sql.query("INSERT INTO narrata_keys (library_id, key, value, revision) VALUES ($1, $2, $3, $4) ON CONFLICT (library_id, key) DO UPDATE SET value = EXCLUDED.value, revision = EXCLUDED.revision", [this.id, key, value, batch.revision]);
        }
        for (const key of batch.deleteKeys) {
          await sql.query("DELETE FROM narrata_keys WHERE library_id = $1 AND key = $2", [this.id, key]);
        }
      }
      await this.checkQuota(sql);
      const persisted = flush.batches[flush.batches.length - 1]!.revision;
      await sql.query("UPDATE narrata_libraries SET revision = $2 WHERE id = $1", [this.id, persisted]);
      return { persisted };
    });
    // The transaction adapter has committed before the protocol can claim persistence.
    return encodeFlushReply(reply);
  }

  private async checkQuota(sql: SqlTransaction): Promise<void> {
    const { rows } = await sql.query(`SELECT
      (SELECT count(*) FROM narrata_objects WHERE library_id = $1) AS objects,
      (SELECT count(*) FROM narrata_keys WHERE library_id = $1) AS keys,
      (SELECT coalesce(sum(octet_length(digest)::bigint + octet_length(bytes)), 0) FROM narrata_objects WHERE library_id = $1)
        + (SELECT coalesce(sum(octet_length(key)::bigint + octet_length(value)), 0) FROM narrata_keys WHERE library_id = $1) AS bytes`, [this.id]);
    const usage = rows[0];
    if (rows.length !== 1 || usage === undefined) throw new SqlStoreFormatError("SQL returned no quota totals");
    for (const dimension of ["bytes", "objects", "keys"] as const) {
      if (unsigned(usage[dimension]) > BigInt(this.quota[dimension])) {
        throw new LibraryQuotaExceeded(`library ${dimension} quota exceeded`);
      }
    }
  }
}
