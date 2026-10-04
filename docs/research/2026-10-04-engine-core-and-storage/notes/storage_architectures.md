# Storage and Persistence Architectures for a Backend-Independent Narrative Engine (Narrata)

Scope note: research current to early October 2026. "Benchmark" = measured result with stated method; "vendor" = published by a party selling one of the compared systems; "preprint" = arXiv, not peer reviewed. Narrata facts come from the local repo (`crates/narrata-store/src/store.rs`), not the web.

---

## 1. Abstraction patterns for pluggable storage, and how they fail

### Takeaway
The patterns that hold up abstract at one of two levels. The low level is an ordered, transactional key-value or blob store with compare-and-swap (FoundationDB "layers", SurrealDB's KV engines, object_store/OpenDAL). The high level is a narrow, purpose-specific domain port (hexagonal architecture). The pattern with a documented record of failure is the middle one: a generic query or ORM layer meant to make rich backends (graph DB, Postgres, JSON) interchangeable. That layer either drops to the lowest common denominator or leaks. When backends differ, the documented fix is to have each backend declare its capabilities, with a correct fallback for anything it can't do.

### Cited Findings
- **Hexagonal / ports-and-adapters (Cockburn, 2005).** The intent is to let an application "be developed and tested in isolation from its eventual run-time devices and databases". Cockburn also writes that "A port identifies a purposeful conversation", and one port usually has several adapters. So the port is defined by what the application needs, not by what the database can do. — [Cockburn, Hexagonal Architecture](https://alistair.cockburn.us/hexagonal-architecture/)
- **ORM critique (Neward, June 2006).** Neward calls O/R mapping a "slippery slope": early wins, then returns that shrink as investment grows. He names specific failure modes: object identity versus state-based identity, the partial-object / load-time paradox (the mapper can't know how query results will be used), schema-ownership conflict, and the dual-schema problem (metadata kept in two places). His six exits are abandonment, wholehearted acceptance (an object store), manual mapping, accepting ORM limits (ORM for about 80%, raw SQL for the rest), relational concepts in the language (LINQ), and relational concepts in frameworks. — [Neward, The Vietnam of Computer Science](https://blogs.newardassociates.com/blog/2006/the-vietnam-of-computer-science.html)
- **Fowler, "OrmHate" (May 2012).** Fowler says the hard part is "synchronizing between two quite different representations of data". He argues a tool that removes 80% of the work is still worth having. He praises Active Record for providing "manholes so you can get down with the SQL". He also says NoSQL avoids the mapping problem when the application model matches the store's structure (aggregates, graphs), but not in general. — [Fowler, OrmHate](https://martinfowler.com/bliki/OrmHate.html)
- **Leaky abstractions (Spolsky, 2002).** "All non-trivial abstractions, to some degree, are leaky." The SQL example: some logically equivalent queries run thousands of times slower than others. — [Joel on Software](https://www.joelonsoftware.com/2002/11/11/the-law-of-leaky-abstractions/)
- **Stonebraker & Pavlo on KV-level abstraction (SIGMOD Record, June 2024):**
  - Using a KV store in a complex application "that requires more than just a binary relation" is called dangerous, because applications must parse fields, have no secondary indexes, and "must implement joins or multi-get operations in their application".
  - "It is not trivial to re-engineer a KV store to make it support a complex data model, whereas RDBMSs easily emulates KV stores."
  - The authors also note a 20-year trend of building full DBMSs on embedded KV storage managers: MySQL's pluggable storage API led to MyRocks, and MongoDB moved to WiredTiger.
  - — [Stonebraker & Pavlo, "What Goes Around Comes Around... And Around..." (ACM)](https://dl.acm.org/doi/10.1145/3685980.3685984); [PDF mirror](https://khoury.northeastern.edu/home/pandey/courses/cs7270/fall25/papers/intro/whatgoesaroundaround-stonebraker.pdf)
- **FoundationDB "layer concept".** "FoundationDB decouples its data storage technology from its data model." Documents, graphs, SQL and records are built as layers on an ordered KV store. Three properties make this work: ordered keys (data and index share prefixes), range reads, and ACID transactions, so that an index and its data update atomically. — [FoundationDB Layer Concept](https://apple.github.io/foundationdb/layer-concept.html)
- **Hard limits leak through the abstraction.** FoundationDB transactions can't exceed 10,000,000 bytes, and over 1 MB is a sign to redesign. They can't run longer than 5 seconds. Keys are limited to 10,000 bytes and values to 100,000 bytes. — [FoundationDB Known Limitations](https://apple.github.io/foundationdb/known-limitations.html)
- **SurrealDB's pluggable KV engines.** The query layer handles parsing, permissions, planning and snapshot-isolation transactions. Every storage engine has to "support transactional read and write of individual keys and key ranges". Engines include RocksDB, SurrealKV (beta, supports time travel), SurrealMX (memory), and IndexedDB in the browser. — [SurrealDB Architecture](https://surrealdb.com/docs/surrealdb/introduction/architecture). SurrealDB 3.0 went stable on 17 Feb 2026. — [SurrealDB 3.0 release](https://surrealdb.com/releases/3.0)
- **Capability negotiation, blob level (Apache OpenDAL).** The `Capability` struct lists per-service flags such as `write_with_if_not_exists`, `write_with_if_match`, `write_with_if_version_match`, `write_can_append`, `write_can_multi`, `list_with_recursive` and `read_with_version`, plus size limits like `write_multi_max_size`. The docs say: "Developers should use `capability` to determine available operations." — [OpenDAL Capability](https://opendal.apache.org/docs/rust/opendal/struct.Capability.html). Latest crate is 0.59.3 (22 Sep 2026); docs.rs failed to build it, last good build was 0.58.2. — [docs.rs opendal](https://docs.rs/opendal/latest/opendal/struct.Capability.html)
- **CAS in an object-store API (`object_store` 0.14.2).** `PutMode::Create` returns `AlreadyExists` if the object exists. `PutMode::Update(UpdateVersion)` writes only if the e-tag/version matches, otherwise `Precondition`. This is the CAS primitive a ref/head store needs. Per the docs, conditional behaviour depends on the backend. — [object_store PutMode](https://docs.rs/object_store/latest/object_store/enum.PutMode.html)
- **Capability negotiation, query level (Apache DataFusion).** `TableProvider::supports_filters_pushdown` returns `Exact`, `Inexact` or `Unsupported` for each filter. DataFusion re-applies `Inexact` and `Unsupported` filters itself, and "if any pushed-down filters are `Inexact`, the `LIMIT` cannot be pushed down." Backends advertise what they can do, and the engine keeps a correct fallback for the rest. — [DataFusion TableProvider](https://docs.rs/datafusion/latest/datafusion/datasource/trait.TableProvider.html)
- **Rust SQL abstraction limits.** SQLx's `Any` driver switches backend at runtime, but SQLx depends on type information from the database, which "widely varies between database systems and are often incomplete". MSSQL support was removed after 0.7 pending a rewrite. — [SQLx discussion #1616](https://github.com/launchbadge/sqlx/discussions/1616); summary via [dev.co SQLx overview](https://dev.co/databases/open-source/sqlx) (secondary).
- **Narrata's current port (local repo).** `SaveStore` in `crates/narrata-store/src/store.rs` is a domain-level port, not a KV trait. It covers:
  - content-addressed objects (`get_object`, `list_objects`)
  - refs, catalog heads, timeline archives and compound saves, each with `read_*`/`list_*`
  - an effect ledger with leases (`claim_effect`, `renew_effect_lease`, `record_effect_outcome`)
  - one atomic `commit(CommitTransaction)` carrying objects, ref/catalog/archive/compound-save mutations, inputs and pins
  - `collect(RetentionPolicy)` for GC and `integrity_scan()`
  - ref updates checked against an expected revision (`check_expected_ref` raises `RefConflict`), i.e. CAS
  - `list_objects()` returns `Vec<CheckedObject>`, which loads every object at once
  - — local source: `D:/rezics-repos/narrata/crates/narrata-store/src/store.rs` (lines ~251–369)

### Inferences
- Narrata's thesis that "the engine core is abstract logic" fits hexagonal architecture well. The weak spot is the idea that one port can serve both a graph database and plain JSON. The evidence supports a two-level design:
  - **Source-of-truth port, KV-level and narrow:** immutable content-addressed blobs, CAS'd refs, atomic multi-key commit, ordered prefix/range listing. A KV store, SQLite, Postgres, IndexedDB, OPFS, an object store, or a directory of files with a manifest can implement this faithfully.
  - **Optional query/projection ports:** a graph database or Postgres is a derived index you can rebuild from the source of truth, not where the truth lives.
- Narrata's `SaveStore` sits between those levels. It has about 25 domain methods, so every new backend has to reimplement domain logic such as lease semantics and catalog conflicts. That is the "dual-schema / reimplement joins in the application" cost Neward and Stonebraker/Pavlo describe. One option: split it into a small `ObjectStore + RefStore (CAS) + atomic batch` core, with the ledger/catalog/compound-save logic written once on top. The FoundationDB "layer" approach is the model for this.
- `list_objects() -> Vec` and the `list_*` methods don't scale to ultra-long narratives or browser memory budgets. A minimal contract should use streaming, prefix-bounded iteration (a range scan with a cursor).
- Backend differences should be explicit capability flags (atomic multi-key commit, CAS, ordered range scan, durability class, max value size, native traversal, time-travel reads), like OpenDAL and DataFusion. The engine should either refuse to run with an unsafe backend (for example, no CAS means single-writer only) or emulate the missing feature correctly.

### Gaps
- Didn't verify the backend traits of diesel or sea-orm, or how their dialect coverage fails. The claim that SeaORM sits on SQLx came only from search snippets.
- Found no empirical study measuring the "lowest common denominator" cost of multi-backend persistence layers. The evidence is essays and design docs.

---

## 2. Candidate backends: graph databases vs. relational adjacency, and 2024–2026 project status

### Takeaway
For Narrata's runtime work (load a node, follow a few edges, append a commit), the evidence says a relational or KV store with adjacency tables is enough. Native graph databases pay off mainly on deep or variable-length traversals and graph analytics. Several high-quality sources argue that relational engines with graph extensions match or beat them even there. The Rust/Wasm-embeddable graph options have serious continuity risk: Kùzu is archived, CozoDB is stagnant, IndraDB is quiet. Neo4j and Memgraph are server products. So a graph database fits best as an optional, derived authoring and analytics backend, not as a primary save store.

### Cited Findings
**Evidence on graph DB vs. relational**
- **Stonebraker & Pavlo (2024):**
  - Neo4j stores edges as pointers, "as in CODASYL", which helps "for traversing long edge chains", whereas an RDBMS has to use joins. Its success "comes down to whether there are enough 'long chain' scenarios that merit forgoing a RDBMS."
  - Any graph can be simulated as `Node(node_id, node_data)` and `Edge(node_id_1, node_id_2, edge_data)`, but "vanilla SQL is not expressive enough", which forces client-server round trips.
  - They cite "several performance studies showing that graph simulation on RDBMSs outperform graph DBMSs", and DuckDB's SQL/PGQ beating "a leading graph DBMS by up to 10×".
  - For graph analytics they recommend compressing the graph into an in-memory structure on one node.
  - Verdict: "OLTP graph applications will be largely served by RDBMSs… We do not expect specialized graph DBMSs to be a large market."
  - — [SIGMOD Record 53(2), 2024](https://dl.acm.org/doi/10.1145/3685980.3685984)
  - The cited studies are Fan, Raj & Patel, "The Case Against Specialized Graph Analytics Engines" (CIDR 2015) — [dblp](https://dblp.org/rec/conf/cidr/FanRP15.html) — and Jindal et al., "Graph analytics using Vertica" (IEEE BigData 2015).
- **DuckPGQ (CIDR 2023, peer-reviewed workshop/conference paper).**
  - Argues that "competent graph data systems must build on all technology that makes up a state-of-the-art relational system", plus many-source path-finding, a compact graph representation (CSR), and research on worst-case-optimal joins and factorized processing.
  - Benchmark: LDBC SNB-derived graphs (SF10–SF3000 for path-finding; SF10–SF100 for LSQB pattern matching). Compared against Neo4j 4.4.2 Enterprise and Umbra on 48 vCPUs, 248 GB RAM, NVMe RAID-0, cold database, 1 h timeout.
  - DuckPGQ ran embedded in Python; Neo4j and Umbra ran in Docker.
  - — [ten Wolde et al., DuckPGQ, CIDR 2023](https://www.cidrdb.org/cidr2023/papers/p66-wolde.pdf)
- **PostgreSQL SQL/PGQ (PostgreSQL 19).**
  - Committed 16 Mar 2026 by Peter Eisentraut, co-authored by Ashutosh Bapat. Adds `GRAPH_TABLE` and `CREATE/ALTER/DROP PROPERTY GRAPH`.
  - A property graph is a view-like relkind, and queries "are rewritten to standard relational queries in the rewriter".
  - Not yet included: quantified path patterns (variable-length paths) and shortest-path queries.
  - — [pgsql-committers commit message](https://www.postgresql.org/message-id/E1w247I-0000Tk-2Y@gemulon.postgresql.org); [PG19 docs: CREATE PROPERTY GRAPH](https://www.postgresql.org/docs/19/sql-create-property-graph.html)
- **Vendor benchmark (Memgraph vs. PG19).** Pokec subset (100,000 vertices, 1,768,515 edges), exact N-hop reachability from one user, 30 s timeout. About 1× at 1 hop, Memgraph 11.5× faster at 4 hops, PG19 timed out at 5 hops versus Memgraph's 259 ms. This is vendor marketing on a deliberately narrow workload. — [Memgraph blog](https://memgraph.com/blog/postgresql-19-alternatives-memgraph)
- **Preprint, ReCAP (Apr 2026).** Argues that neither graph nor relational DBMSs prune intermediate results early in regular path queries with property constraints. DuckDB plus ReCAP reportedly beats "state-of-the-art graph and relational DBMS by a factor up to 400,000". — [Rivera Correa & Riedewald, arXiv:2604.02553](https://arxiv.org/abs/2604.02553) (not peer reviewed)
- **Preprint, advocacy (Sep 2026).** Claims columnar relational engines with a Cypher front end (ClickGraph/DeltaGraph) beat Neo4j by "two-to-four orders of magnitude" on LDBC SNB, and that the node/edge model is "a performance tax". Single author, not peer reviewed. — [arXiv:2609.01525](https://arxiv.org/abs/2609.01525)
- **Recursive CTEs (PostgreSQL docs).** `WITH RECURSIVE` runs iteratively with a working table. `UNION` removes duplicates; `UNION ALL` needs explicit cycle handling. `SEARCH DEPTH/BREADTH FIRST` and `CYCLE … SET is_cycle USING path` clauses exist. Order within levels is undefined, and the docs warn against relying on `LIMIT` to stop recursion. — [PostgreSQL: WITH Queries](https://www.postgresql.org/docs/current/queries-with.html)

**Standards**
- **ISO/IEC 39075:2024 GQL** was published in April 2024. It's the first new ISO database language since SQL (1987), written by the same SC32/WG3 group that maintains SQL, and covers graph types, pattern matching, transactions and security. — [JTC1 article on GQL](https://www.jtc1info.org/wp-content/uploads/2024/04/2024-Article-39075-Database-Language-GQL.docx.pdf); [Wikipedia: GQL](https://en.wikipedia.org/wiki/Graph_Query_Language)
- **SQL/PGQ** is ISO/IEC 9075-16:2023 (SQL:2023 Part 16). PostgreSQL 19 implements part of it, as above. — [PG commit](https://www.postgresql.org/message-id/E1w247I-0000Tk-2Y@gemulon.postgresql.org)

**Project status (2025–2026)**

| System | Status | Embeddable in Rust / runs in browser Wasm |
|---|---|---|
| **Kùzu** | Apple agreed in Oct 2025 to buy Kùzu Inc. and hire some of the team ([UWaterloo CS news, 24 Feb 2026](https://uwaterloo.ca/computer-science/news/waterloo-based-graph-database-start-up-kuzu-acquired-apple)). Repo archived: "September 2025" per [The Data Quarry](https://thedataquarry.com/blog/from-kuzu-to-ladybug/), "October 2025" per [FalkorDB](https://www.falkordb.com/blog/kuzudb-to-falkordb-migration/), a vendor (dates conflict). The npm package is deprecated with unpatched vulnerable transitive dependencies ([dev.to report](https://dev.to/asiaostrich/our-graph-database-was-abandoned-upstream-heres-the-6-line-migration-engramgraph-030-5h1p), secondary). | Community fork **LadybugDB** is repositioning as a "graph lakehouse" (DuckDB storage, Arrow/Parquet) with no corporate backing ([The Data Quarry](https://thedataquarry.com/blog/from-kuzu-to-ladybug/); [ArcadeDB](https://arcadedb.com/blog/neo4j-alternatives-in-2026-a-fair-look-at-the-open-source-options/), a vendor). |
| **CozoDB** | Last release v0.7.6 on 11 Dec 2023 ([releases](https://github.com/cozodb/cozo/releases)). Open issue "Is cozo still being maintained?" since 4 Dec 2025; open issue (11 Apr 2026) that the Sled backend has a broken `del()` and no time travel ([issues](https://github.com/cozodb/cozo/issues)). Pre-1.0 with no promise of storage compatibility ([repo](https://github.com/cozodb/cozo)). | Rust embedded; backends in-memory, SQLite, RocksDB, Sled, TiKV; Wasm build runs in the browser ([repo](https://github.com/cozodb/cozo)). |
| **SurrealDB** | 3.0 stable on 17 Feb 2026 ([release](https://surrealdb.com/releases/3.0)). | Rust; embedded engines include an IndexedDB engine via Wasm ([architecture](https://surrealdb.com/docs/surrealdb/introduction/architecture); [embedded engines](https://surrealdb.com/docs/reference/javascript/concepts/embedded-engines)). |
| **Apache AGE** (openCypher on Postgres) | Apache TLP since May 2022. v1.6.0 on 22 Sep 2025, v1.7.0 on 21 Jan 2026 ([release notes](https://age.apache.org/release-notes/)). Active per [LFX Insights](https://insights.linuxfoundation.org/project/apache-age). | Postgres extension; not embeddable. |
| **TerminusDB** | DFRNT took over maintenance in 2025. TerminusDB 12 released Dec 2025 ([blog](https://terminusdb.org/blog/2025-12-08-terminusdb-12-release/)); 12.0.7 on 10 Aug 2026 ([releases](https://github.com/terminusdb/terminusdb/releases)). | Server (Prolog/Rust); not checked for Wasm. |
| **Oxigraph** (RDF/SPARQL) | Active, v0.5.x ([crates.io](https://crates.io/crates/oxigraph); [npm](https://www.npmjs.com/package/oxigraph)). | Rust library on RocksDB. JS/Wasm binding offers only an in-memory store ([docs.rs](https://docs.rs/oxigraph/latest/oxigraph/index.html)). |
| **HelixDB** | Rust OLTP graph+vector DB, Apache-2.0, "built … on Object Storage", mentions an embedded mode. No independent benchmarks in the repo ([GitHub](https://github.com/HelixDB/helix-db)). | Embedded mode claimed; no Wasm mention. |
| **IndraDB** | Last updated 16 Aug 2025 per [GitHub topic listing](https://github.com/topics/graph-database?l=rust&o=desc&s=forks) (low-confidence signal). | Rust. |
| **Neo4j / Memgraph** | Server products; Memgraph is in-memory (vendor blog above). | Not embeddable in Rust/Wasm (licences and embedding not checked in this pass). |

### Inferences
- A graph database as Narrata's primary save store is the weakest part of the thesis:
  - Narrata's persistent core is a commit DAG of content-addressed objects plus refs. That's a Merkle-DAG and KV workload, the same one Git, Dolt and IPLD run on ordered KV or chunk stores.
  - Graph traversal is needed for authoring analytics (reachability, dead ends, "which saves are affected by this content change"), and it runs over a few thousand to millions of story nodes. Stonebraker & Pavlo and DuckPGQ say a single-node in-memory or relational engine handles that well.
- Postgres with adjacency tables and recursive CTEs is a reasonable server backend today. PG19's SQL/PGQ adds a standard pattern syntax but lacks variable-length paths for now, so deep reachability still needs `WITH RECURSIVE` or application-side BFS.
- For in-process analytics, build the narrative graph into a CSR or adjacency structure in memory, which is what DuckPGQ and Stonebraker recommend. That beats depending on an embedded graph DB whose upstream may vanish, as Kùzu's did.
- If a graph DB is offered at all, make it a derived projection: export story and save graphs into Neo4j, Memgraph or AGE for authoring tools. Don't make it a `SaveStore` implementation.

### Gaps
- Didn't pull official LDBC SNB audited results; only paper-level and vendor comparisons above.
- Didn't verify Neo4j's own "index-free adjacency" documentation. The description used here is Stonebraker & Pavlo's.
- GQL conformance of Neo4j, Memgraph and others wasn't checked.
- Wasm support for LadybugDB, HelixDB and TerminusDB is unknown.
- Found no benchmarks for SurrealDB graph traversal or HelixDB other than marketing.

---

## 3. Event sourcing / CQRS / append-only log as the interchangeable source of truth

### Takeaway
Making a log (events, or Narrata's inputs and commits) the source of truth and treating backends as rebuildable projections is the best-documented way to make storage backends interchangeable. It also matches deterministic replay closely. The documented costs are real: event schema evolution, projection code duplication, eventual consistency, growing history that needs snapshots, deletion and compliance, and replay safety for external effects. Narrata's effect ledger is already the standard answer to the external-effects problem.

### Cited Findings
- **Fowler (2005).** Event sourcing enables "Complete Rebuild", "Temporal Query" and "Event Replay". The main hazard is that external systems must not be re-notified during replay, so wrap them in gateways that detect replay mode. External query responses should be logged and reused on replay so results stay consistent. — [Fowler, Event Sourcing](https://martinfowler.com/eaaDev/EventSourcing.html)
- **Kleppmann, "Turning the database inside-out" (2015).** Treat an append-only log as the source of truth and materialized views as derived data, so you get several representations of the same data plus full replay. Transactions across this design are called an "open research problem". — [Confluent blog](https://www.confluent.io/blog/turning-the-database-inside-out-with-apache-samza/)
- **Microsoft Azure Architecture Center, Event Sourcing pattern (updated 2026):**
  - It's "costly to migrate to or from an event sourcing solution", and "For most systems … traditional data management is sufficient."
  - Versioning strategies: tolerant deserialization, version identifiers, chained upcasters, and in-place migration "as a last resort" because it breaks the audit trail.
  - "Snapshots are an optimization, not a replacement for the eventstream", taken every N events.
  - There's no standard query mechanism over events.
  - Kafka-style brokers "lack per-entity stream queries and optimistic concurrency" and are "not a substitute for an event store".
  - Consumers must be idempotent under at-least-once delivery.
  - Right-to-be-forgotten conflicts with immutability; mitigations are storing personal data outside the store or crypto-shredding.
  - Not suited to MVPs or short-lived systems.
  - — [Microsoft Learn](https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing)
- **Overeem, Spoor & Jansen, SANER 2017 (peer-reviewed).** Converting data in event-sourced systems is hard because the pattern is new, there are no standard tools, and event stores hold "the large amount of data". They propose event-store upgrade operations, techniques including multiple versions, upcasting and lazy transformation, and an upgrade framework checked in interviews with three experts. — [paper PDF](https://www.movereem.nl/files/2017SANER-eventsourcing.pdf); [Utrecht University record](https://research-portal.uu.nl/en/publications/the-dark-side-of-event-sourcing-managing-data-conversion/)
- **Practitioner critique (Kiehl, 2019).** Once you rewrite the ledger for deprecated events, "you've lost the ability to accurately produce the state of your system at the point in time of the rewrite". The first extra projection "doubles the amount of code that touches your event stream". Eventual consistency produces 404s and stale items. Low-level events are "too chatty". — [Kiehl, "Don't Let the Internet Dupe You, Event Sourcing is Hard"](https://chriskiehl.com/article/event-sourcing-is-hard)
- **Snapshots as memoization.** Greg Young: "A snapshot is a memorization of your left fold" (search-result excerpt of the transcript, not fetched in full). — [Kurrent: transcript of Greg Young, Code on the Beach 2014](https://www.kurrent.io/blog/transcript-of-greg-youngs-talk-at-code-on-the-beach-2014-cqrs-and-event-sourcing)
- **EventStoreDB is now KurrentDB.** Rebrand announced Nov–Dec 2024 with $12M funding. KurrentDB 25.0 is the first release under the new name, and the licence changed from ESLv2 to Kurrent License v1. — [KurrentDB 25.0 release](https://www.kurrent.io/releases/kurrentdb/25-0/); [rebrand FAQ](https://www.kurrent.io/blog/kurrent-re-brand-faq); [BigDATAwire](https://www.hpcwire.com/bigdatawire/2024/12/18/event-store-changes-name-to-kurrent-raises-12m-to-unify-streams-and-databases/)

### Inferences
- Narrata is already largely event-sourced. A deterministic program plus a recorded input log plus an effect ledger is the "log is truth, state is a fold" design. Its commits and digests are snapshots, i.e. memoized folds.
- That makes "backend independence via projections" natural. Only the log/object/ref store has to be faithful; any SQL, graph or JSON view can be rebuilt.
- The Fowler/Azure caveat about external systems during replay is exactly what Narrata's lease-based effect ledger (`claim_effect`, `record_effect_outcome`) handles. Keep that ledger in the source-of-truth contract.
- For ultra-long sessions, the risk isn't the backend. It's history growth (a log that grows without bound) and the replay cost of very old saves. You need periodic state checkpoints with retention/GC (Narrata has `collect(RetentionPolicy)`) and a defined policy for compacting old inputs. Keep enough history that rewind targets stay reproducible.
- Narrative-content updates are the event-schema-evolution problem in another form. Inputs recorded against program digest A have to be replayed, upcast or remapped against digest B. Azure's strategy list (tolerant decoding, versioned envelopes, chained upcasters, no in-place rewrites) carries over directly.

### Gaps
- Didn't fetch Greg Young's *Versioning in an Event Sourced System* (Leanpub) for primary quotes.
- Found no quantitative studies of event-log growth or replay cost for long-running interactive sessions specifically (games or narrative engines).

---

## 4. Versioned, immutable and bitemporal stores, branching histories and "dynamic narrative updates"

### Takeaway
Datomic, XTDB, Dolt, TerminusDB, Irmin and Git/IPLD all build history on immutable facts or content-addressed Merkle structures. Branching and diffing are cheap when the structure is history-independent (Dolt's prolly trees) and its encoding is canonical (DAG-CBOR). Narrata's deterministic CBOR plus digests plus commits is the same family. None of these systems solves "content changed under existing saves": that is a migration and semantic-compatibility problem layered on top of versioning.

### Cited Findings
- **Datomic.**
  - Data is immutable "datoms": entity, attribute, value, transaction, op.
  - "A db is a point-in-time, immutable value and will never change". Change happens by accretion: assert and retract, never update in place.
  - "universal schema": "Any entity can then have any attribute."
  - — [Datomic data model](https://docs.datomic.com/whatis/data-model.html)
  - Since April 2023 all editions are free of licence fees, with binaries under Apache-2.0. The source is not open. — [Datomic is Free](https://blog.datomic.com/2023/04/datomic-is-free.html)
  - Datomic Local, an embeddable edition under Apache-2.0, followed in Aug 2023. — [Datomic Local](https://blog.datomic.com/2023/08/datomic-local-is-released.html)
  - It runs on the JVM, so it can't be embedded in Rust or Wasm.
- **XTDB v2.**
  - GA on 12 Jun 2025.
  - Bitemporal: system time (when recorded) and valid time (when true), based on SQL:2011 temporal primitives, with Postgres-compatible SQL.
  - Storage is immutable Apache Arrow on object storage plus an LSM temporal index.
  - JVM (Clojure/Kotlin); MPL-2.0.
  - — [XTDB v2 launch](https://xtdb.com/blog/launching-xtdb-v2); [v2.0.0 release](https://github.com/xtdb/xtdb/releases/tag/v2.0.0)
- **Dolt.**
  - Uses prolly trees, which are "content-addressed B-tree[s]".
  - Table data, schema and database roots hash together.
  - "Sections of the tree that share the same root hash can share storage between versions".
  - Prolly trees are history-independent: the same data gives the same tree and hash whatever the insertion order.
  - Diff walks only the differing paths, so it scales with the size of the change, not the size of the data.
  - A Git-style commit graph sits on top.
  - — [Dolt storage engine](https://www.dolthub.com/docs/architecture/storage-engine); [Dolt fast merge with prolly trees, 2025-07-16](https://www.dolthub.com/blog/2025-07-16-announcing-fast-merge/); [Structural sharing with schema changes](https://www.dolthub.com/blog/2024-01-19-structural-sharing-with-schema-changes/)
- **Irmin (OCaml).**
  - A library for "mergeable, branchable distributed data stores" built on Git principles.
  - Backends: in-memory, a bidirectional Git bridge, and a pack-file store.
  - Merges are 3-way from the lowest common ancestor, using user-specified merge rules.
  - — [Irmin](https://irmin.org/); [Irmin API docs](https://mirage.github.io/irmin/irmin/Irmin/index.html)
- **TerminusDB.** A document graph database with Git-for-data features (branch, merge, history). Maintained by DFRNT since 2025. — [TerminusDB](https://terminusdb.org/); [TerminusDB 12 blog](https://terminusdb.org/blog/2025-12-08-terminusdb-12-release/)
- **IPLD DAG-CBOR (canonical CBOR for content addressing):**
  - map keys sorted length-first, then lexically
  - minimal integer encoding
  - definite lengths only
  - all floats 64-bit; NaN, ±Infinity and −0.0 forbidden
  - links are CIDs under tag 42
  - Strictness guarantees "a single, canonical way of encoding any given set of data". Decoders may relax it for legacy data.
  - — [DAG-CBOR spec](https://ipld.io/specs/codecs/dag-cbor/spec/)
- **Time travel in embeddable Rust stores:**
  - CozoDB enables time travel per relation when the key's last slot has type `Validity`, so you only pay for it where you need it. — [Cozo v0.4 "Time travel"](https://docs.cozodb.org/en/latest/releases/v0.4.html)
  - SurrealKV (beta) supports time-travel queries. — [SurrealDB 3.0](https://surrealdb.com/3.0)
- **Narrative-engine precedent (ink).**
  - ink saves with `state.ToJson()` and restores with `state.LoadJson()`. The official docs don't say whether a save stays valid after the story content changes. — [ink RunningYourInk.md](https://github.com/inkle/ink/blob/master/Documentation/RunningYourInk.md)
  - ink tracks separate story-format and save-state format versions. — [DeepWiki summary of inkle/ink](https://deepwiki.com/inkle/ink) (secondary)
  - Users report state-restore bugs, for example threaded choices not round-tripping. — [ink issue #267](https://github.com/inkle/ink/issues/267); [ink issue #321](https://github.com/inkle/ink/issues/321)

### Inferences
- Narrata's commit/digest model matches the Git/Dolt/IPLD family:
  - deterministic CBOR is the counterpart of DAG-CBOR canonicalization
  - digests are CIDs
  - refs with CAS are branches
  - compound saves and timeline archives are tags or multi-root commits
- The lesson from Dolt is history independence. If save state is chunked into canonical, content-defined chunks (prolly-tree-like) rather than whole-state blobs, ultra-long narratives share structure across commits, and diffs and merges scale with what changed.
- "Dynamic narrative update" splits into two problems:
  1. Versioning content: content packs as immutable, digest-addressed commits. All these systems handle this already.
  2. Migrating live saves from content digest A to B. No storage system solves it. It needs an explicit, authored migration (lens or upcaster) keyed by `(old_program_digest → new_program_digest)`, plus a compatibility policy for each save: pin to the old content, migrate, or replay inputs against the new content and detect divergence.
- Bitemporality maps neatly onto narratives:
  - valid time ≈ story-world time / timeline position
  - system time ≈ real-world time the player committed
  - XTDB/Datomic-style "as-of" queries would let tools ask "what did this save believe at story-time T, as of commit C".
  - Narrata can model this itself; it doesn't need the JVM-based XTDB or Datomic.

### Gaps
- Didn't fetch primary docs on Git's object model. The Merkle-DAG description relies on Dolt's and IPLD's.
- Didn't find Yarn Spinner or Twine documentation on save compatibility across content updates.
- Didn't check whether TerminusDB has a Wasm or embedded mode.

---

## 5. CRDTs and local-first for collaborative authoring and live content updates

### Takeaway
CRDT libraries (Automerge 3, Loro 1.x, Yjs) are now practical for collaborative authoring of narrative source in Rust and Wasm. Loro adds movable-tree and time-travel support. But they keep growing history, and they leave schema migration largely unsolved; Ink & Switch says so itself. They fit the authoring side (editing story graphs together). They don't fit deterministic runtime saves, where Narrata's total ordering is a feature, not a bug.

### Cited Findings
- **Ink & Switch, "Local-first software" (Kleppmann, Wiggins, van Hardenberg, McGranaghan; Onward! 2019).**
  - Seven ideals: no spinners, multi-device, offline, collaboration, longevity, privacy, user control.
  - Open problems: "CRDTs accumulate a large change history, which creates performance problems". Schema migration and compatibility across app versions are unsolved.
  - — [Local-first essay](https://www.inkandswitch.com/essay/local-first/)
- **Automerge 3.0 (July 2025).**
  - Memory "cut … by over 10x". Pasting Moby Dick into a document used about 700 MB in v2 and about 1.3 MB in v3.
  - One document went from 17 hours to 9 seconds to load.
  - Same file format as Automerge 2, API "nearly fully backwards compatible".
  - Stores full history.
  - — [Automerge 3.0 blog](https://automerge.org/blog/automerge-3/)
  - The release date conflicts with an aggregator that says May 2025 ([velt.dev](https://velt.dev/blog/best-crdt-libraries-real-time-data-sync)); the primary source says July.
- **Loro (Rust, Wasm).**
  - 1.0 in 2024, with rich-text and movable-tree CRDTs; the movable tree prevents cycles under concurrent moves. Claims about 10× faster loading. — [Loro 1.0 blog](https://loro.dev/blog/v1.0) (page returned 403; details from the search excerpt)
  - The API offers `checkout()` to a historical version (detached mode, read-only unless `set_detached_editing(true)`), `fork_at(frontiers)`, and `ExportMode::ShallowSnapshot`, which garbage-collects history. Caveat: "peers cannot import updates from before the shallow start". — [docs.rs LoroDoc](https://docs.rs/loro/latest/loro/struct.LoroDoc.html)
- **Cambria (Ink & Switch, Oct 2020; Litt, van Hardenberg, Henry).**
  - Translates between schema versions with bidirectional lenses that compose into a graph; migration takes the shortest path.
  - Limitations they found: some changes only work one way, lenses can't add data that's missing, and recursive schemas and performance are unsolved.
  - — [Cambria](https://www.inkandswitch.com/cambria/)

### Inferences
- Use CRDTs (Loro is the most natural fit for Rust/Wasm because of its movable tree and checkout/fork) for collaborative authoring of the source story graph. Compile CRDT snapshots into Narrata's deterministic CBOR program packs, each with its own digest. That keeps runtime determinism out of reach of CRDT merge nondeterminism, and gives "live content updates" an explicit boundary: a new pack digest.
- Loro's shallow snapshot is the same trade-off as Narrata's retention GC: trim history and lose the ability to merge or rewind before the trim point. Retention policy should be a user-visible guarantee.
- Cambria-style lenses are the closest prior art for migrating saves across content versions (section 4). Their limits, such as one-way changes and missing data, are the honest answer to "can every old save survive every content update?": no.

### Gaps
- Didn't research Yjs/Yrs status in this pass.
- Found no measurements of CRDT documents holding narrative-graph-scale data (100k+ nodes).

---

## 6. JSON and document storage: when it's fine and when it isn't

### Takeaway
JSON files work as an interchange or export format and for small single-player saves. As a primary store they give up atomic multi-object commits, secondary indexes, CAS and efficient partial reads, which are the features Narrata's commit model needs. JSON columns inside SQLite or Postgres are a reasonable middle ground for genuinely semi-structured payloads, as long as hot fields stay in real columns.

### Cited Findings
- **Stonebraker & Pavlo (2024) on document databases.** They're "essentially the same as object-oriented DBMSs from the 1980s and XML DBMSs". Denormalization has old problems: duplicated data when the join isn't one-to-many, prejoins "not necessarily faster than joins", and "no data independence". Almost every NoSQL system added a SQL interface by the end of the 2010s. — [SIGMOD Record 2024](https://dl.acm.org/doi/10.1145/3685980.3685984)
- **SQLite JSONB (from 3.45.0).**
  - A binary encoding stored as a BLOB.
  - 5–10% smaller than text JSON, and needs "less than half the CPU cycles".
  - "JSONB is not intended as an external format to be used by applications", and it isn't compatible with Postgres JSONB.
  - — [SQLite JSONB](https://www.sqlite.org/jsonb.html)
- **Postgres JSONB pitfalls (Heap, 2016; possibly dated).**
  - Postgres keeps no statistics on fields inside JSONB, so the planner uses a hard-coded 0.1% selectivity. In the example that caused a bad nested-loop plan: "584 seconds … about 2000x slower".
  - The JSONB version took 164 MB, "more than twice as much" as the normalized table.
  - Moving 45 common fields out of JSONB saved about 30% of disk.
  - Advice: keep fields that appear in most rows as real columns.
  - — [Heap engineering blog](https://www.heap.io/blog/when-to-avoid-jsonb-in-a-postgresql-schema)
- **Browser localStorage is capped at 10 MiB** (5 MiB local + 5 MiB session per origin) and throws `QuotaExceededError`. — [MDN storage quotas](https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria)
- **Lenses can generate JSON Schema.** Cambria's lens specifications produce JSON Schema validation and TypeScript types for each version. — [Cambria](https://www.inkandswitch.com/cambria/)

### Inferences
- A plain-JSON backend is viable if it implements the source-of-truth contract structurally:
  - an append-only directory of content-addressed blobs (filename = digest, which makes writes idempotent and safe)
  - one small "refs" manifest updated by atomic replace with a revision check, which gives single-writer CAS
  - optional JSON Lines append logs for inputs
- That is Git's loose-object layout. It works for single-player, small saves and debugging. It breaks down with many small objects, concurrent writers and partial reads, and it has no secondary indexes.
- Use JSON (or CBOR diagnostic notation) as the human-readable export or interchange format, and canonical CBOR as the format of record. That avoids JSON's float and key-order nondeterminism; DAG-CBOR's rules in section 4 show what canonicalization requires.
- In Postgres, store immutable objects as `bytea` CBOR keyed by digest, and project only the queried fields (story position, chapter, timestamps) into real columns. Heap's statistics pitfall argues against querying inside JSONB on hot paths.

### Gaps
- Didn't verify whether recent PostgreSQL versions (16–19) improved planner statistics for JSONB expressions; the Heap data is from 2016.
- Found no authoritative benchmark of JSON-file stores against SQLite for many small objects.

---

## 7. Very large graphs in memory-constrained clients (Wasm, browser, mobile)

### Takeaway
In browsers, the strongest storage path in 2026 is SQLite-Wasm on OPFS. Use a cooperative-sync VFS, keep IndexedDB as a fallback, and run it in a dedicated worker. The main risks are eviction and Safari limits, not raw speed. Lazy, page- or chunk-granular loading works very well when the data is indexed and partitioned; sql.js-httpvfs fetched about 1 kB from a 670 MB database for an indexed lookup. Graph analytics should run on compact in-memory structures (CSR) built per region or chapter, not on the persisted store.

### Cited Findings
- **SQLite on the web, state as of May 2026 (PowerSync; vendor of sync tooling, but technically detailed):**
  - `OPFSCoopSyncVFS` (wa-sqlite) is the recommended general-purpose choice and works with databases of 1 GB+.
  - `OPFSWriteAheadVFS` (April 2026) allows reads concurrent with writes but needs Chrome 121+, isn't proven in production, and has weak durability defaults.
  - `IDBBatchAtomicVFS` is the cross-browser fallback but degrades above about 100 MB and overflows the stack on Safari with large queries.
  - Shared workers don't support OPFS.
  - Chrome incognito caps databases at 100 MB; Safari has no OPFS in private mode and no JSPI yet.
  - — [PowerSync: SQLite persistence on the web, May 2026](https://powersync.com/blog/sqlite-persistence-on-the-web)
- **Browser quotas and eviction (MDN):**
  - Default storage is "best-effort"; `navigator.storage.persist()` asks for persistent storage.
  - Firefox best-effort allows the lesser of 10% of disk or 10 GiB; persistent allows up to 50%.
  - Chrome allows up to 60% of disk per origin.
  - Safari "proactively" deletes script-created storage after 7 days without user interaction.
  - Eviction is LRU and removes the whole origin at once: IndexedDB, Cache and OPFS together.
  - — [MDN: Storage quotas and eviction criteria](https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria)
- **IndexedDB vs. SQLite/OPFS performance (low-quality personal blogs; conflicting):**
  - One reports that SQLite wins indexed reads by about 3× and batched inserts by about 1.5×, while IndexedDB wins small single writes by about 18×, small reads by about 29× and startup by about 10× (46 ms vs. about 535 ms). — [recca0120 blog, Mar 2026](https://recca0120.github.io/en/2026/03/06/browser-storage-comparison/)
  - Another claims OPFS-backed SQLite beats IndexedDB by 25×. — [sachinsharma.dev, 2026](https://sachinsharma.dev/blogs/sqlite-wasm-opfs-performance-benchmarks-2026)
  - The results depend on workload; don't rely on either without your own measurements.
- **Lazy, page-granular loading (sql.js-httpvfs, 2021, updated 2023):**
  - HTTP range requests fetch only the SQLite pages a query needs.
  - On a 670 MB, 8M-row database, an indexed lookup fetched about 1 kB, a complex query 130–270 kB over 10–20 requests, and full-text search about 70 kB.
  - Uses 1 KB pages and exponentially growing prefetch for sequential reads.
  - Read-only, and performance depends heavily on index design.
  - — [phiresky: Hosting SQLite databases on GitHub Pages](https://phiresky.github.io/blog/2021/hosting-sqlite-databases-on-github-pages/)
- **Compact in-memory graph representations.**
  - Stonebraker & Pavlo recommend compressing the graph "into a space-efficient data structure that fits in memory on a single node". — [SIGMOD Record 2024](https://dl.acm.org/doi/10.1145/3685980.3685984)
  - DuckPGQ treats fast CSR construction as a core requirement and stresses explicit control over memory locality. — [DuckPGQ CIDR 2023](https://www.cidrdb.org/cidr2023/papers/p66-wolde.pdf)
- **Embeddable Rust KV engines (status):**
  - **redb** is "Stable and maintained", with a file format that "is stable", full ACID, MVCC, savepoints and rollbacks. The README doesn't mention Wasm. — [redb README](https://github.com/cberner/redb)
  - **sled** says of itself "if reliability is your primary constraint, use SQLite. sled is beta", and its "on-disk format is going to change in ways that require manual migrations before the 1.0.0 release". — [sled README](https://github.com/spacejam/sled)
  - **Fjall 3.0** brings a new forward-compatible block format with xxh3 checksums. — [Fjall 3.0 post](https://fjall-rs.github.io/post/fjall-3/)
  - The Fjall 3.0 release date conflicts: the fetched post read as 15 Sep 2026, while a third-party issue says January 2026 with 3.1 in March 2026 ([nicti issue #116](https://github.com/jordanfelle/nicti/issues/116)).
  - Wasm support isn't stated for redb or Fjall.
- **Browser-capable engines.** SurrealDB's Wasm build persists to IndexedDB and can run in a Web Worker. — [SurrealDB embedded engines](https://surrealdb.com/docs/reference/javascript/concepts/embedded-engines)
  - CozoDB runs "a complete CozoDB instance in your browser". — [cozo repo](https://github.com/cozodb/cozo)
  - Oxigraph's JS build is in-memory only. — [docs.rs oxigraph](https://docs.rs/oxigraph/latest/oxigraph/index.html)

### Inferences
- For Narrata's existing browser IndexedDB backend: IndexedDB is fine for small, many-key workloads and starts fast. Consider SQLite-Wasm on OPFS (OPFSCoopSyncVFS) once save histories reach hundreds of MB, keeping IDB as the fallback. Both should sit behind the same minimal KV/object contract.
- Request `navigator.storage.persist()` and surface eviction risk, especially Safari's 7-day rule. Eviction wipes the whole origin, so offer export/sync of the commit DAG; content addressing makes incremental sync easy.
- "Ultra-long narratives" on constrained clients need partitioning:
  - content packs split by chapter or region, each a separately digested object
  - save state chunked by subsystem
  - indexes keyed by `(timeline, position)` so rewind and lookup touch O(log n) pages, not whole histories
- The httpvfs result shows that good index design matters far more than choosing a backend.
- Build reachability and analytics graphs on demand as CSR over the chapters you need. Don't persist a "graph DB" in the client.

### Gaps
- Found no peer-reviewed or vendor-neutral benchmarks for IndexedDB vs. OPFS-SQLite.
- Didn't check mobile-specific constraints (iOS WebView storage behaviour beyond MDN's Safari notes).
- The Wasm status of redb and Fjall is unverified.

---

## 8. A Datalog or relational query layer as the stable abstraction

### Takeaway
A Datalog-style or relational query interface is a sound stable abstraction for authoring and analysis queries: reachability, "which saves visited node X", consistency checks. It doesn't belong in the persistence contract. The evidence favours a small embedded engine running over an in-memory or projected snapshot rather than pushing Datalog down into each backend. The Rust Datalog ecosystem is fragmented: DDlog is archived, CozoDB is stagnant, and the compile-time engines can't take runtime queries.

### Cited Findings
- **Datomic** queries immutable database values; each value is a point-in-time snapshot. — [Datomic data model](https://docs.datomic.com/whatis/data-model.html)
- **CozoDB** is a Rust "transactional, relational-graph-vector database that uses Datalog", with optional per-relation time travel and Wasm. It is stagnant: last release Dec 2023, and a maintenance question is open. — [cozo repo](https://github.com/cozodb/cozo); [releases](https://github.com/cozodb/cozo/releases); [issues](https://github.com/cozodb/cozo/issues)
- **DBSP (Budiu, Ryzhyk et al.).** Won VLDB 2023 Best Research Paper for "Automatic Incremental View Maintenance for Rich Query Languages", the theory behind Feldera, with provably correct incremental evaluation. Also in the VLDB Journal (Apr 2025). — [Feldera blog](https://www.feldera.com/blog/best-research-paper-vldb-2023); [arXiv:2203.16684](https://arxiv.org/abs/2203.16684)
- **DDlog** (Differential Datalog on McSherry's differential dataflow) is archived under `vmware-archive` and "not being actively maintained". The team founded Feldera and "switched from differential Datalog to differential SQL". — [vmware-archive/differential-datalog](https://github.com/vmware-archive/differential-datalog); [HN thread "Datalog in Rust"](https://news.ycombinator.com/item?id=44281727)
- **Rust Datalog crates.** `ascent` and `crepe` are proc-macro engines; programs are fixed at compile time, so "they won't handle getting queries at runtime". `datafrog` is a lightweight engine you drive by hand. — [docs.rs ascent](https://docs.rs/ascent/latest/ascent/); [datafrog](https://github.com/frankmcsherry/datafrog); [HN discussion](https://news.ycombinator.com/item?id=44281727)
- **Standard graph queries.** SQL/PGQ (PG19) and GQL give standard graph pattern syntax over relational and graph backends, with PG19 still lacking variable-length paths. — [PG commit](https://www.postgresql.org/message-id/E1w247I-0000Tk-2Y@gemulon.postgresql.org); [GQL](https://www.jtc1info.org/wp-content/uploads/2024/04/2024-Article-39075-Database-Language-GQL.docx.pdf)

### Inferences
- Give Narrata a query layer as a separate, optional port. Load or project the narrative graph and save history into an in-memory relation set (CSR plus fact tables). Evaluate a small Datalog/relational IR over it, for example `ascent` for fixed analyses or a runtime engine for user queries. Optionally translate the same IR to SQL `WITH RECURSIVE` or SQL/PGQ when a Postgres projection exists. That's the DataFusion pushdown idea (exact / inexact / unsupported) applied to graph queries.
- Incremental view maintenance (DBSP-style) is relevant to "dynamically updated narratives". When a content pack changes, an incrementally maintained "affected saves / unreachable nodes" view avoids recomputing everything. Treat it as a later optimization, not a contract requirement.
- Don't adopt CozoDB as the core dependency, even though its feature set (Datalog, time travel, Wasm, pluggable storage) matches the thesis almost exactly, because maintenance has stalled.

### Gaps
- Didn't evaluate Raqlet ("Cross-Paradigm Compilation for Recursive Queries", arXiv:2508.03978), which appeared in search and may be directly relevant to compiling one recursive query IR to Cypher, SQL and Datalog.
- Didn't check Soufflé or differential-dataflow's current release status.

---

## 9. What to require of a minimal storage contract

### Takeaway
The evidence points to a minimal source-of-truth contract shaped like FoundationDB, SurrealDB's KV layer and Git:
- immutable content-addressed objects
- mutable refs with compare-and-swap
- an atomic multi-key commit
- ordered, prefix-bounded, streaming iteration
- declared capabilities and limits

Everything else (domain semantics, indexes, graph and SQL views, time-travel queries) is a layer or projection the engine owns. Graph DBs and Postgres then become adapters or projections. JSON becomes a constrained single-writer adapter plus an interchange format.

### Cited Findings
- **Ordered keys, range reads and ACID transactions** are named as sufficient to build documents, graphs and SQL as layers. — [FoundationDB layer concept](https://apple.github.io/foundationdb/layer-concept.html)
- **"Transactional read and write of individual keys and key ranges"** is the only thing SurrealDB requires of RocksDB, SurrealKV, memory and IndexedDB to give one set of query semantics everywhere. — [SurrealDB architecture](https://surrealdb.com/docs/surrealdb/introduction/architecture)
- **Create-if-absent and update-if-version-matches** are standard object-store primitives (`AlreadyExists` / `Precondition` errors). — [object_store PutMode](https://docs.rs/object_store/latest/object_store/enum.PutMode.html); [OpenDAL Capability](https://opendal.apache.org/docs/rust/opendal/struct.Capability.html)
- **Optimistic concurrency.** An event store should "reject an append if the stream changed since it was read". Consumers must be idempotent and track the last processed sequence number. — [Microsoft Learn Event Sourcing](https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing)
- **Backend limits leak and must be declared.** For example, FoundationDB's 10 MB / 5 s transaction cap and 100 kB value cap — [FDB known limitations](https://apple.github.io/foundationdb/known-limitations.html) — and the browser's best-effort eviction — [MDN](https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria).
- **Canonical encoding is what makes content addresses portable** across implementations. — [DAG-CBOR spec](https://ipld.io/specs/codecs/dag-cbor/spec/)
- **Capability-driven negotiation with a correct fallback** (`Exact` / `Inexact` / `Unsupported`). — [DataFusion TableProvider](https://docs.rs/datafusion/latest/datafusion/datasource/trait.TableProvider.html)
- **Narrata's `SaveStore`** already requires atomic `commit(CommitTransaction)`, ref CAS through expected revisions, GC (`collect`) and `integrity_scan`. It doesn't require streaming iteration (`list_objects` returns a `Vec`). — local `D:/rezics-repos/narrata/crates/narrata-store/src/store.rs`

### Inferences
Proposed minimal contract, derived from the findings above:

1. **Object space:**
   - `put_if_absent(digest, bytes)`, which is idempotent and lets the backend skip writes for existing digests
   - `get(digest)` and `has(digest)`
   - The engine verifies digests on read, as `CheckedObject` already does.
2. **Ref space:**
   - `read_ref(key) -> (value, revision)`
   - CAS-style `compare_and_set(key, expected_revision, new_value)`
3. **Atomic batch:** objects plus several ref CAS operations commit together or not at all. Backends without multi-key atomicity (plain files, some object stores) must declare it. The engine then uses a single-root-ref design: write everything content-addressed, then CAS one root ref. That's how Git and Dolt get atomicity from one pointer swap.
4. **Ordered, streaming iteration:** `scan_prefix(prefix, cursor, limit)` for refs, catalogs, inputs and GC marking. This replaces the `list_* -> Vec` methods.
5. **Capability and limit descriptor:**
   - `atomic_multi_key`, `cas`, `ordered_scan`
   - `durability` (durable / best-effort-evictable / memory)
   - `max_value_bytes`, `max_txn_bytes`
   - `concurrent_writers`, `native_traversal`, `time_travel_reads`
   - The engine refuses or emulates per capability.
6. **Engine-owned layers above the contract:**
   - the effect ledger and leases
   - catalogs, archives, compound saves
   - retention GC (mark from refs, sweep by scan)
   - integrity scan
   - migration and upcasting across program digests
   - projections and exports (Postgres tables, Neo4j/AGE graph, JSON dumps), rebuildable from the source of truth.

Where the user's thesis needs correcting:
- "Graph DB / Postgres / JSON as interchangeable *primary* backends" invites lowest-common-denominator semantics, or reimplementing the domain in every adapter. Better: make them interchangeable implementations of the small contract above (Postgres and SQLite fit it well; a JSON directory fits as single-writer), and make graph databases derived projections.
- Backend choice doesn't make ultra-long narratives possible. What does: chunked, structurally shared state; checkpoints and snapshots; retention GC; and indexes that are bounded by prefix.
- Dynamic narrative updates are a migration and compatibility problem: lenses or upcasters between program digests. Versioned storage only supplies the immutable inputs for it.

### Gaps
- No source directly evaluates a minimal storage contract for game or narrative save systems. The contract above is synthesized from database and storage-system designs and should be validated against Narrata's conformance testkit (`crates/narrata-testkit/src/backend.rs` defines a `ConformanceBackend` trait, which wasn't reviewed in depth here).
