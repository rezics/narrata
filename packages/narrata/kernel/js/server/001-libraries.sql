-- ADR 0021 reference DDL. Install with the host's numbered SQL migrations; the package never runs it.
CREATE TABLE narrata_libraries (
  id bytea PRIMARY KEY CHECK (octet_length(id) = 16),
  owner_key text COLLATE "C" NOT NULL,
  work_key text COLLATE "C" NOT NULL,
  active boolean NOT NULL DEFAULT true,
  expires_at timestamptz,
  revision bigint NOT NULL DEFAULT 0 CHECK (revision BETWEEN 0 AND 9007199254740991),
  CHECK (active OR expires_at IS NOT NULL)
);
CREATE UNIQUE INDEX narrata_active_library
  ON narrata_libraries (owner_key, work_key) WHERE active;
CREATE INDEX narrata_library_owner ON narrata_libraries (owner_key, work_key);
CREATE INDEX narrata_library_expiry ON narrata_libraries (expires_at) WHERE NOT active;
CREATE TABLE narrata_objects (
  library_id bytea NOT NULL REFERENCES narrata_libraries(id) ON DELETE CASCADE,
  digest bytea NOT NULL CHECK (octet_length(digest) = 32),
  bytes bytea NOT NULL CHECK (octet_length(bytes) <= 16777216),
  PRIMARY KEY (library_id, digest)
);
CREATE TABLE narrata_keys (
  library_id bytea NOT NULL REFERENCES narrata_libraries(id) ON DELETE CASCADE,
  key bytea NOT NULL CHECK (octet_length(key) BETWEEN 2 AND 1026),
  value bytea NOT NULL CHECK (octet_length(value) <= 16777216),
  revision bigint NOT NULL CHECK (revision BETWEEN 1 AND 9007199254740991),
  PRIMARY KEY (library_id, key)
);
