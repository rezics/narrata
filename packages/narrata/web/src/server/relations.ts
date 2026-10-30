import type { Publication } from "../authoring/core.js";
import type { ReaderRelations } from "../authoring/maps.js";
import type { AuthorRelationQuery, AuthorRelationPage } from "../generated/authoring.js";
// The build copies the existing kernel SQL driver and its TransactionalSql boundary here.
import { loadRelations, queryRelations, readReaderRelations } from "./relation-driver.js";
import type { TransactionalSql } from "./postgres.js";
export type { SqlParameter, SqlTransaction, TransactionalSql } from "./postgres.js";
export { RelationStoreError } from "./relation-driver.js";

/** The application installs relations-v1.sql, supplies authorized connections, and decides
 * which callers may use complete author queries or visits-constrained reader queries. */
export class PostgresRelations {
  constructor(private readonly database: TransactionalSql) {}
  load(publication: Publication): Promise<void> { return loadRelations(this.database, publication); }
  query(publication: Publication, query: AuthorRelationQuery, offset = 0, limit = 256): Promise<AuthorRelationPage> {
    return queryRelations(this.database, publication, query, offset, limit);
  }
  readReader(allowlist: ReaderRelations): Promise<void> { return readReaderRelations(this.database, allowlist); }
}
