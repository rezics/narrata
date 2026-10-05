CREATE DOMAIN narrative_digest AS bytea CHECK (octet_length(VALUE) = 32);
CREATE DOMAIN narrative_id AS bytea CHECK (octet_length(VALUE) = 16);
CREATE TABLE narrative_projection (
  artifact narrative_digest NOT NULL, version smallint NOT NULL CHECK (version > 0),
  projection_index narrative_digest NOT NULL, PRIMARY KEY (artifact, version)
);
CREATE TABLE narrative_unit (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  unit narrative_digest NOT NULL, provider text NOT NULL, content_key text NOT NULL,
  PRIMARY KEY (artifact, version, unit), UNIQUE (artifact, version, provider, content_key),
  FOREIGN KEY (artifact, version) REFERENCES narrative_projection
);
CREATE TABLE narrative_cluster (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  cluster narrative_id NOT NULL, unit narrative_digest, package text, graph text,
  CHECK ((unit IS NOT NULL AND package IS NULL AND graph IS NULL) OR
         (unit IS NULL AND package IS NOT NULL AND graph IS NOT NULL)),
  PRIMARY KEY (artifact, version, cluster),
  FOREIGN KEY (artifact, version) REFERENCES narrative_projection,
  FOREIGN KEY (artifact, version, unit) REFERENCES narrative_unit
);
CREATE TABLE narrative_node (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  node narrative_id NOT NULL, package text NOT NULL, graph text NOT NULL,
  kind smallint NOT NULL CHECK (kind BETWEEN 0 AND 4), unit narrative_digest,
  first_anchor text, last_anchor text, cluster narrative_id NOT NULL,
  PRIMARY KEY (artifact, version, node),
  FOREIGN KEY (artifact, version, unit) REFERENCES narrative_unit,
  FOREIGN KEY (artifact, version, cluster) REFERENCES narrative_cluster
);
CREATE INDEX narrative_node_unit ON narrative_node (artifact, version, unit, node);
CREATE INDEX narrative_node_cluster ON narrative_node (artifact, version, cluster, node);
CREATE TABLE narrative_edge (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  edge_no bigint NOT NULL CHECK (edge_no >= 0), source_unit narrative_digest NOT NULL,
  target_unit narrative_digest NOT NULL, source_node narrative_id NOT NULL,
  target_node narrative_id NOT NULL, kind smallint NOT NULL CHECK (kind BETWEEN 0 AND 4),
  choice_point narrative_id, option_id narrative_id,
  CHECK ((kind = 0 AND choice_point IS NOT NULL AND option_id IS NOT NULL) OR
         (kind <> 0 AND choice_point IS NULL AND option_id IS NULL)),
  PRIMARY KEY (artifact, version, edge_no),
  FOREIGN KEY (artifact, version, source_unit) REFERENCES narrative_unit,
  FOREIGN KEY (artifact, version, target_unit) REFERENCES narrative_unit,
  FOREIGN KEY (artifact, version, source_node) REFERENCES narrative_node,
  FOREIGN KEY (artifact, version, target_node) REFERENCES narrative_node
);
CREATE INDEX narrative_edge_from ON narrative_edge (artifact, version, source_unit, source_node);
CREATE INDEX narrative_edge_to ON narrative_edge (artifact, version, target_unit, target_node);
CREATE TABLE narrative_entry (
  artifact narrative_digest NOT NULL, version smallint NOT NULL, node narrative_id NOT NULL,
  PRIMARY KEY (artifact, version, node),
  FOREIGN KEY (artifact, version, node) REFERENCES narrative_node
);
CREATE TABLE narrative_ending (
  artifact narrative_digest NOT NULL, version smallint NOT NULL, node narrative_id NOT NULL,
  class text NOT NULL CHECK (class IN ('unspecified','success','failure','neutral')),
  reachable boolean NOT NULL, PRIMARY KEY (artifact, version, node),
  FOREIGN KEY (artifact, version, node) REFERENCES narrative_node
);
CREATE TABLE narrative_route (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  ending_node narrative_id NOT NULL, cluster narrative_id NOT NULL,
  PRIMARY KEY (artifact, version, ending_node, cluster),
  FOREIGN KEY (artifact, version, ending_node) REFERENCES narrative_ending,
  FOREIGN KEY (artifact, version, cluster) REFERENCES narrative_cluster
);
CREATE INDEX narrative_route_cluster ON narrative_route (artifact, version, cluster, ending_node);
CREATE TABLE narrative_first (
  artifact narrative_digest NOT NULL, version smallint NOT NULL,
  unit narrative_digest NOT NULL, node narrative_id NOT NULL,
  distance bigint NOT NULL CHECK (distance >= 0), PRIMARY KEY (artifact, version, unit, node),
  FOREIGN KEY (artifact, version, unit) REFERENCES narrative_unit,
  FOREIGN KEY (artifact, version, node) REFERENCES narrative_node
);
CREATE INDEX narrative_unit_reference ON narrative_unit (provider, content_key, artifact, version);
