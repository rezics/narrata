# Narrata repository current state: logic, storage, and content boundaries (local codebase only)

Scope: local repo `D:/rezics-repos/narrata` at `main` / `f68af97` (15 commits, 2026-08-26 → 2026-09-05). This covers only code and in-repo docs, with no web research. Links are relative to this notes file (`../../` = repo root). "IMPL" means code exists and I read it. "DOC" means a design statement, plan, or proposal. Line numbers are approximate when marked `~`.

---

## Q1. What crates and packages exist, what each is responsible for, and how they depend on each other

### Takeaway
The repo has **two engine stacks that share no Rust code**:
- The **"Stage 1–5" stack** is a CBOR Program with a Flow VM and Statecharts. It spans `narrata-core`, then `narrata-store` and `narrata-store-sqlite`, then `narrata-protocol`, then FFI, Wasm, and C#/TS/Unity bindings.
- The **"R0/R1" node stack** is a JSON node/graph Gamebook in `packages/narrata/nodes` and `packages/narrata/tooling`. It does **not** depend on `narrata-core` or `narrata-store` at all.

The only bridge is that `narrata-cli` depends on `narrata-node-tools` for its `gamebook` subcommand.

### Cited Findings
- The workspace has 11 members:
  - 8 under `crates/`: core, store, store-sqlite, testkit, cli, protocol, ffi, wasm.
  - 2 under `packages/narrata/nodes/crates/`: `narrata-nodes` and `narrata-nodes-wasm`.
  - 1 under `packages/narrata/tooling/crates/`: `narrata-node-tools`.
  - `fuzz` is excluded.
  - Edition 2024, rust-version 1.98, license AGPL-3.0-only. Lints forbid `unsafe_code` and deny `unwrap`, `expect`, `panic`, and `todo`. — [Cargo.toml](../../../../Cargo.toml)
- **Dependency graph (IMPL, from each Cargo.toml):**
  - `narrata-core` depends only on hex, sha2, serde, serde_json, and thiserror. It has no internal dependencies. — [crates/narrata-core/Cargo.toml](../../../../crates/narrata-core/Cargo.toml)
  - `narrata-testkit` depends on core. — [crates/narrata-testkit/Cargo.toml](../../../../crates/narrata-testkit/Cargo.toml)
  - `narrata-store` depends on core and sha2. It uses criterion for benches. — [crates/narrata-store/Cargo.toml](../../../../crates/narrata-store/Cargo.toml)
  - `narrata-store-sqlite` depends on core, store, and rusqlite (bundled). — [crates/narrata-store-sqlite/Cargo.toml](../../../../crates/narrata-store-sqlite/Cargo.toml)
  - `narrata-protocol` depends on core, store, and prost. — [crates/narrata-protocol/Cargo.toml](../../../../crates/narrata-protocol/Cargo.toml)
  - `narrata-ffi` depends on protocol. — [crates/narrata-ffi/Cargo.toml](../../../../crates/narrata-ffi/Cargo.toml)
  - `narrata-wasm` depends on protocol and wasm-bindgen. — [crates/narrata-wasm/Cargo.toml](../../../../crates/narrata-wasm/Cargo.toml)
  - `narrata-cli` depends on core, store, store-sqlite, testkit, and **narrata-node-tools**. — [crates/narrata-cli/Cargo.toml](../../../../crates/narrata-cli/Cargo.toml)
  - `narrata-nodes` depends on serde, serde_json, schemars, sha2, hex, and thiserror. It has **no narrata-core dependency**. — [packages/narrata/nodes/crates/narrata-nodes/Cargo.toml](../../../../packages/narrata/nodes/crates/narrata-nodes/Cargo.toml)
  - `narrata-nodes-wasm` depends on nodes. — [Cargo.toml](../../../../packages/narrata/nodes/crates/narrata-nodes-wasm/Cargo.toml)
  - `narrata-node-tools` depends on nodes, and its binary is named **`narrata-book`**. — [Cargo.toml](../../../../packages/narrata/tooling/crates/narrata-node-tools/Cargo.toml)
- **Responsibilities (IMPL):**
  - **narrata-core** is the pure deterministic kernel: "performs no I/O, clock, randomness, callbacks, or asynchronous work". Its modules are authoring, codec, content, debugger, diagnostic, effect, identity, limits, migration, program, runtime, scene, snapshot, statechart, value, and version. — [crates/narrata-core/src/lib.rs:1-23](../../../../crates/narrata-core/src/lib.rs)
  - **narrata-store** covers Commit/Ref, the effect ledger, barriers, federated (compound) saves, GC, bundles, migrations, and the `SessionCoordinator`. Its header says "Bytes, bundle entries, database rows, and mutable references remain untrusted until they pass the checked constructors… Immutable objects are content addressed; refs are the only mutable, revisioned storage surface." — [crates/narrata-store/src/lib.rs:1-37](../../../../crates/narrata-store/src/lib.rs)
  - **narrata-store-sqlite** is the native reference adapter (a single 1,324-line file). — [crates/narrata-store-sqlite/src/lib.rs](../../../../crates/narrata-store-sqlite/src/lib.rs)
  - **narrata-protocol** provides the Protobuf pull protocol plus a `ProtocolEngine`. **narrata-ffi** is an opaque-handle C ABI. **narrata-wasm** is a browser adapter over the same engine. — [docs/plan/README.md:44-70](../../../../archive/plan/README.md); [crates/narrata-protocol/proto/narrata.proto](../../../../crates/narrata-protocol/proto/narrata.proto)
  - **bindings/** contains three bindings:
    - `dotnet/`: SafeHandle and P/Invoke.
    - `typescript/`: Buf-generated DTOs, a Wasm wrapper, and an IndexedDB store.
    - `unity/`: a native client for non-WebGL platforms and a WebGL `.jslib`.
    - Sources: [bindings/typescript/src/indexeddb-store.ts](../../../../bindings/typescript/src/indexeddb-store.ts); [docs/adr/0009](../../../adr/0009-stage-5-migration-and-protocol-boundary.md)
  - **narrata-nodes** provides "Typed authoring nodes and explicit subgraph composition, independent of the legacy Flow VM". Its pieces are Bundle, NodeRegistry, compile, CheckedProduct, Session, and BookView. — [packages/narrata/nodes/crates/narrata-nodes/src/lib.rs:1-49](../../../../packages/narrata/nodes/crates/narrata-nodes/src/lib.rs)
  - **narrata-node-tools** "负责文件读取、项目组合、lock 校验和 CLI，不参与确定性运行" (file reading, project composition, lock validation, and the CLI; it takes no part in deterministic execution). — [packages/narrata/tooling/README.md](../../../../packages/narrata/tooling/README.md)
  - Package manifests:
    - `narrata.nodes` provides `node-compilation.v1` and `gamebook-session.v1`.
    - `narrata.tooling` provides `gamebook-compose.v1` and `gamebook-cli.v1`, and requires `narrata.nodes`.
    - Sources: [packages/narrata/nodes/narrata-package.json](../../../../packages/narrata/nodes/narrata-package.json); [packages/narrata/tooling/narrata-package.json](../../../../packages/narrata/tooling/narrata-package.json)
  - **examples/gamebook-web** is a React/Vite reader. It runs `narrata-nodes-wasm` in a Worker and stores data in IndexedDB. — [examples/gamebook-web/src/worker.ts](../../../../examples/gamebook-web/src/worker.ts); [examples/gamebook-web/src/storage.ts](../../../../examples/gamebook-web/src/storage.ts)
  - **products/gamebook-demo** ("山口来信") is made of three content packages (main, road, camp), plus `project.json`, `project.lock.json`, and a composed self-contained `story.nar.json`. — [products/gamebook-demo/README.md](../../../../products/gamebook-demo/README.md)
  - **fixtures/** holds codec vectors, conformance stories, negative cases, a statechart fixture, and a frozen `compat/stage5-v0` corpus. **fuzz/** holds 8 decoder/reducer fuzz targets.
- Note: [REBUILD_PROGRESS.md](../../../../archive/2026-09-05-rebuild-progress.md) refers to "独立 `narrata-book`" (a standalone `narrata-book`). That is the **binary name** of the `narrata-node-tools` crate, not a separate crate.
- ADR 0011 makes the split deliberate: "在 `packages/narrata/nodes` 建立独立 owner，内部 `narrata-nodes` 不依赖旧 `narrata-core`。旧 Stage 1–5 接口、CBOR 与存档继续保持原样，逐步通过显式 adapter 接回" (a standalone owner in `packages/narrata/nodes`; `narrata-nodes` does not depend on the old `narrata-core`; Stage 1–5 interfaces, CBOR, and saves stay as they are and will be reconnected step by step through explicit adapters). — [docs/adr/0011-r1-node-composition.md](../../../adr/0011-r1-node-composition.md)
- CI coverage (IMPL, [.github/workflows/ci.yml](../../../../.github/workflows/ci.yml)):
  - It runs nodes/node-tools clippy and tests, a web build, Playwright e2e, and a generated-file drift check.
  - It runs workspace clippy and tests, release-mode golden/effects/federated/sqlite tests, and `wasm32-wasip1` checks for core, testkit, and store.
  - It runs cargo deny and a fuzz build check.

### Inferences
- "Kernel vs. domain package" is still split between two codebases. The node stack has its own identity scheme, save format, and session. There is no shared kernel and no shared storage trait. Any pluggable-storage proposal must first decide which stack (or which future merged kernel) the storage abstraction belongs to.
- `narrata-core` already has the dependency hygiene you would want for an abstract logic core: no I/O crates and no internal dependencies.

### Gaps
- I did not inspect the dotnet/unity sources beyond file listings. I did not check whether `bindings/typescript/dist` is in sync with `src`.

---

## Q2. Core runtime data model, and how narrative content (text, choices, media refs) is represented

### Takeaway
In the Stage 1–5 stack, **all display text is embedded in the Program**: Say/Choice instructions index into a `constants: Vec<Value>` pool. That text is **also copied into every Snapshot** taken at a Say/Choice safe point and is verified on restore. Media is referenced only by opaque 16-byte `EntityId` inside a built-in VN `SceneState`. A provider-neutral external-content resolver type exists, but no runtime path calls it. There is **no localization or string-table concept in code**.

The node stack keeps text in a per-package `content` map referenced by key. That map is still inside the same bundle and the same artifact hash. Choice labels and some engine-generated copy are inline.

### Cited Findings
**Program / Flow VM (IMPL)**
- `ProgramArtifactV0` has these fields: format_version, semantics_version, program_id, entry_flow, `constants: Vec<Value>`, globals, flows, capabilities, `external_content`, and an optional statechart. — [crates/narrata-core/src/program/wire.rs:20-31](../../../../crates/narrata-core/src/program/wire.rs)
- The encoder always writes an **empty** array for `external_content` (field 8). `ExternalContentDeclV0` is a unit struct (a placeholder). — [wire.rs:~69-70](../../../../crates/narrata-core/src/program/wire.rs); [program/flow.rs:34](../../../../crates/narrata-core/src/program/flow.rs)
- The opcodes are Const, Load, Store, Unary, Binary, Jump, JumpIfFalse, Call, Return, `Say{speaker: Option<ConstIndex>, text: ConstIndex}`, `Choice{prompt, choices}`, `Effect{capability}`, `ReconcileScene{target: SceneState}`, `Raise{event}`, and Finish. `ChoiceArmV0.label` is a `ConstIndex`. — [crates/narrata-core/src/program/instruction.rs:86-174](../../../../crates/narrata-core/src/program/instruction.rs)
- `ProgramArtifactId` is computed as `digest_bytes("program-artifact", 0, &payload)` over the whole canonical payload, constants (text) included. — [crates/narrata-core/src/program/validate.rs:227](../../../../crates/narrata-core/src/program/validate.rs); [codec/digest.rs](../../../../crates/narrata-core/src/codec/digest.rs)
- `CheckedProgram` holds the full `ProgramArtifactV0` plus BTreeMap indices for flows, instructions, globals, stack limits, and statechart. `constant(index)` is a direct `Vec` lookup. — [crates/narrata-core/src/program/checked.rs:15-55](../../../../crates/narrata-core/src/program/checked.rs)
- Value kinds are Null, Bool, I64, String, List, Record, Variant, and Entity. There are no floats, which is consistent with the CBOR profile. — [crates/narrata-core/src/value/kind.rs](../../../../crates/narrata-core/src/value/kind.rs); [ADR 0003](../../../adr/0003-deterministic-cbor-profile.md)

**Runtime state (IMPL)**
- `RuntimeStateV0` has these fields: semantics_version, execution_id, program_artifact_id, turn, interaction_counter, `globals: BTreeMap<GlobalId, Value>`, **`scene: SceneState` (mandatory)**, optional `statechart`, and status.
- Status is one of: Ready, Awaiting(pending interaction), AwaitingEffect, StatechartStable, AwaitingStatechartEffect, StatechartFinished, or Finished.
- Source: [crates/narrata-core/src/runtime/state.rs:17-51](../../../../crates/narrata-core/src/runtime/state.rs)
- `PendingInteractionV0::Say` stores `speaker: Option<Arc<str>>` and `text: Arc<str>`. `Choice` stores `prompt` and `offered: Vec<PendingChoiceItemV0{id, label: Arc<str>, target}>`. In other words, it holds **resolved strings, not indices**. — [crates/narrata-core/src/runtime/interaction.rs:~70-110](../../../../crates/narrata-core/src/runtime/interaction.rs)
- The Snapshot encoder writes those strings (`writer.text(text)`, `writer.text(&choice.label)`). — [crates/narrata-core/src/snapshot/wire.rs:180-235](../../../../crates/narrata-core/src/snapshot/wire.rs)
- Restore rejects a snapshot unless the stored text equals the program constant: `text.as_ref() != constant_text(program, expected_text)` returns "pending Say payload". The same check applies to prompt and choice labels. — [crates/narrata-core/src/snapshot/restore.rs:725-753, ~795, ~850](../../../../crates/narrata-core/src/snapshot/restore.rs)
- `state_digest` hashes the full `encode_state_payload`, so the text is part of `StateDigest`. — [crates/narrata-core/src/snapshot/export.rs](../../../../crates/narrata-core/src/snapshot/export.rs)
- The Transition Receipt holds no text, speaker, or label. — [crates/narrata-core/src/runtime/receipt.rs](../../../../crates/narrata-core/src/runtime/receipt.rs)
- The transport DTOs carry text as plain strings: `Result.text` and `Choice.label`. — [narrata.proto:96-105](../../../../crates/narrata-protocol/proto/narrata.proto)

**Scene and media (IMPL)**
- `SceneState` holds layers, actors, camera, audio channels, and an interaction view. Asset references are `LayerState.asset: Option<EntityId>`, `ActorState.asset: EntityId`, and `AudioState.asset: EntityId`. Coordinates are integer milli-units. There is also `ResumeSupport{Restart, Seek, BestEffort}`. — [crates/narrata-core/src/scene.rs:11-74](../../../../crates/narrata-core/src/scene.rs)
- `ReconcileScene` embeds a literal target `SceneState` inside the Program instruction. — [instruction.rs:143-146](../../../../crates/narrata-core/src/program/instruction.rs)
- The toolchain roadmap confirms that "当前 Core 资源引用是 EntityId… 场景加载不要求 Core 知道 URL、文件扩展名或 CDN" (core asset references are currently EntityId; scene loading does not require the core to know URLs, file extensions, or CDNs). — [docs/research/2026-09-05-narrata-toolchain-roadmap.md:210](../../2026-09-05-narrata-toolchain-roadmap.md)

**External content (IMPL types, not wired)**
- These types exist:
  - `StructureOccurrence{structure: EntityId, occurrence: u32}`
  - `ContentQuery{provider: CapabilityId, occurrence, permission, revision}`
  - `RevisionPolicy::{Pinned(ObjectId), RecordFirstResolution, LivePresentationOnly}`
  - `CheckedContentResolution::{Branching{recorded_value, revision}, LivePresentationOnly{value, revision}}`
  - `trait ExternalContentResolver { fn resolve(&self, &ContentQuery) -> Result<ResolvedContent, _> }`
- Source: [crates/narrata-core/src/content.rs:1-116](../../../../crates/narrata-core/src/content.rs)
- The resolver and the occurrence types are used **only in tests**; grep finds no runtime or store call site. — [crates/narrata-core/tests/effects.rs:75-99](../../../../crates/narrata-core/tests/effects.rs)
- REBUILD_PROPOSAL §9 acknowledges this: "ExternalContentDeclV0 仍是占位类型… 内容解析已有代码不等于已完成可组合内容依赖" (ExternalContentDeclV0 is still a placeholder type; having content-resolution code does not mean composable content dependencies are done). — [REBUILD_PROPOSAL.md:421-440](../../../../archive/2026-09-05-rebuild-proposal.md)

**Identities (IMPL)**
- Author/host-supplied IDs are 16-byte newtypes, including ProgramId, FlowId, InstructionId, ChoiceId, EntityId, LayerId, ActorId, AudioChannelId, StateId, MigrationId, RecoveryCheckpointId, SourceDocumentId, CheckpointId, and ContentOccurrenceId. — [crates/narrata-core/src/identity/authored.rs:40-65](../../../../crates/narrata-core/src/identity/authored.rs)
- Content-derived IDs are 32-byte SHA-256 newtypes, including ProgramArtifactId, StateDigest, ObjectId, SnapshotId, CommitId, EffectId, HostSnapshotDigest, ContentLockId, CompoundSaveManifestId, and BuildProvenanceId. — [identity/derived.rs:36-56](../../../../crates/narrata-core/src/identity/derived.rs); [ADR 0002](../../../adr/0002-identities-and-digests.md)
- Authoring support: `StableIdKind` includes `ContentOccurrence`, and there is a source-map sidecar (≤1,000,000 entries). Relocation hints "cannot be converted into a trusted migration". — [crates/narrata-core/src/authoring.rs:1-60](../../../../crates/narrata-core/src/authoring.rs)
- Default limits:
  - Decoding: 16 MiB envelope and payload, 1 MiB string, 1M collection items.
  - Program: 16,384 flows, 1,000,000 instructions, 1,000,000 constants, 65,536 globals.
  - Runtime: call depth 1,024.
  - Source: [crates/narrata-core/src/limits.rs:1-118](../../../../crates/narrata-core/src/limits.rs)

**Node stack (IMPL, R1)**
- `Bundle{format_version, product, packages}`.
- `NarrativePackage{id, version, exports, content: BTreeMap<String, Content{title, paragraphs: Vec<String>}>, graphs}`.
- `Graph{title, parameters, locals, shared, imports, outcomes, entry, nodes: BTreeMap<String, NodeDefinition{type_id, data: serde_json::Value}>}`.
- Source: [packages/narrata/nodes/crates/narrata-nodes/src/model.rs:61-140](../../../../packages/narrata/nodes/crates/narrata-nodes/src/model.rs)
- The lowered `NodePlan` is one of `Content{content: String /*key*/, label, next}`, `Decision{content, choices}`, `Branch`, `Mutate`, `Call{target, arguments, on_return}`, or `Return`. `Choice.label` and `disabled_reason` are **inline strings**. — [model.rs:142-188](../../../../packages/narrata/nodes/crates/narrata-nodes/src/model.rs)
- Templates support `{{parameter.x}}`, `{{local.x}}`, and `{{shared.x}}`. — [expr.rs:93-134](../../../../packages/narrata/nodes/crates/narrata-nodes/src/expr.rs); [nodes README](../../../../packages/narrata/nodes/README.md)
- The demo package's content map and inline choice labels are both in Chinese. — [products/gamebook-demo/packages/road.json](../../../../products/gamebook-demo/packages/road.json)
- The node runtime **hard-codes a Chinese end-of-story paragraph** ("这段旅程已经结束。你可以回到任一历史节点，尝试另一条路线。"). — [packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs:653-659](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs)
- `NodeRegistry::gamebook()` registers `narrata.{content, decision, branch, mutate, call, return}` at revision "1". The extension type is a build-time `fn(&serde_json::Value) -> Result<NodePlan>` ("not callbacks executed by the story runtime"). — [registry.rs:1-44](../../../../packages/narrata/nodes/crates/narrata-nodes/src/registry.rs)

**Localization**
- grep across `crates`, `packages`, and `docs` finds no locale, i18n, or string-table type in code. The docs assign localization to REZICS or the host: "Unit、Post、Portable Text 和本地化内容 → REZICS" (units, posts, Portable Text, and localized content belong to REZICS) and "不让 Narrata 成为 Post 正文、授权、本地化或 SEO 的权威来源" (Narrata is not to be the authoritative source for post bodies, authorization, localization, or SEO). — [docs/rezics-gamebook-integration.md:30-41, 205](../../../../archive/2026-08-31-rezics-gamebook-integration.md)
- Docs also treat "当前本地化" (the current localization) as a per-request fact resolved across the trust boundary. — [docs/architecture/program-versioning-and-migration.md:129](../../../architecture/program-versioning-and-migration.md)
- The competitive-architecture doc lists "语言、富文本参数、配音与逻辑资源引用" (language, rich-text parameters, voice-over, and logical asset references) as **missing** and cites Yarn line IDs and shadow lines as the model to adopt. — [docs/research/2026-09-05-narrata-competitive-architecture.md:87-101](../../2026-09-05-narrata-competitive-architecture.md)

### Inferences
- Text is "referenced by ID" only inside the binary format, as a ConstIndex into the program's own pool. Semantically it is **embedded**: changing one typo changes `ProgramArtifactId` and invalidates exact-restore of every snapshot paused on that line. That conflicts with the doc invariant "`LivePresentationOnly` 内容变化不改变 Effect/choice/state hash" (a LivePresentationOnly content change does not change Effect/choice/state hashes) ([program-versioning-and-migration.md:269](../../../architecture/program-versioning-and-migration.md)) for internally-held text, even though that invariant was written for external content.
- The node stack is a step toward content separation, since the content map is keyed separately from the graph. The separation is lexical only: the same JSON, the same digest, no locale dimension, and inline labels.

### Gaps
- I did not trace whether Statechart IR carries any strings (it seems unlikely). I did not read `debugger.rs`.

---

## Q3. Persistence: what `narrata-store` defines, how SQLite implements it, what is stored, and whether anything is lazy

### Takeaway
There **is** a real storage trait boundary (`SaveStore`) with a behavioural reference (`MemoryStore`) and a SQLite adapter. However:
1. The trait is shaped around enumerate-everything operations (`list_objects() -> Vec<CheckedObject>`).
2. **`SqliteStore` implements every read and write by loading the whole database into a `MemoryStore`, mutating it, then `DELETE`-ing every table and re-inserting every row.**
3. The Program is stored and loaded as **one monolithic blob**. Snapshots are **full** per commit. There is **no lazy loading, paging, streaming, or chunking** anywhere.
4. The protocol engine used by FFI, Wasm, and TS is hard-wired to an in-process `MemoryStore`. The R1 Gamebook uses neither `SaveStore` nor SQLite.

### Cited Findings
**The trait (IMPL).** `pub trait SaveStore` has:
- Reads: `get_object`, **`list_objects`**, `read_ref`/`list_refs`, `read_catalog_head`/`list_catalog_heads`, `read_timeline_archive`/`list_timeline_archives`, `read_compound_save`/`list_compound_saves`, `read_input`, `list_pins`, `read_effect`/`list_effects`, and `current_ledger_fence`.
- Writes: `claim_effect`, `renew_effect_lease`, `record_effect_outcome`, `mark_effect_compensated`, **`commit(CommitTransaction)`**, `collect(RetentionPolicy)`, and `integrity_scan`.
- It is synchronous, takes `&mut self` for writes, and has no async, iterator, or range APIs.
- Source: [crates/narrata-store/src/store.rs:310-369](../../../../crates/narrata-store/src/store.rs)

**Transactions and objects (IMPL)**
- `CommitTransaction{objects, refs (CAS), catalogs, archives, compound_saves, inputs, pins, remove_pins, observed_at, import_transaction}`. Ref CAS conflicts return expected, actual, and proposed values (no last-write-wins). — [store.rs:64-98, 214-262](../../../../crates/narrata-store/src/store.rs)
- `CheckedObject{id, kind, schema, bytes: Arc<[u8]>}`. `ObjectId = SHA-256("narrata-object\0" || kind || schema || canonical payload)`. — [crates/narrata-store/src/object.rs](../../../../crates/narrata-store/src/object.rs)
- `CommitV1{parent, execution, program: ProgramArtifactId, snapshot: SnapshotId, cause: Genesis | RuntimeTransition(ReceiptId) | Migration(MigrationId), ledger_fence, turn}`. — [crates/narrata-store/src/commit.rs:16-32](../../../../crates/narrata-store/src/commit.rs)

**What is stored (IMPL)**
- Object kinds include Program, Snapshot, Receipt, Commit, TimelineCatalogEvent, CheckpointBundleManifest, TimelineArchiveManifest, CompoundSaveManifest, and HostTimelineManifest. — [object.rs](../../../../crates/narrata-store/src/object.rs)
- Mutable tables cover refs, catalog_heads, archive_refs, compound_save_refs, pins, input_index, effect_ledger, and ledger_fences. — [crates/narrata-store-sqlite/src/lib.rs:55-152](../../../../crates/narrata-store-sqlite/src/lib.rs)
- So the store holds **program definitions** (whole artifact blobs), **runtime state** (a full Snapshot per Commit), **history** (the Commit DAG, Receipts, and Catalog Events when Complete recording is on), and the **effect ledger**. It does not hold media.
- `SessionCoordinator::create_with_capabilities` encodes the **entire Program artifact** as an object at session creation. — [crates/narrata-store/src/coordinator.rs:222-226](../../../../crates/narrata-store/src/coordinator.rs)
- The coordinator holds a single `program: Arc<CheckedProgram>` and the current `state: Arc<RuntimeStateV0>`. — [coordinator.rs:164-177](../../../../crates/narrata-store/src/coordinator.rs)

**SQLite adapter (IMPL)**
- Schema v2 uses STRICT tables: `objects(id BLOB PK, kind, schema, payload BLOB, inserted_at)`, `object_edges`, and so on. It runs with WAL, `synchronous=FULL`, foreign_keys on, and a 5 s busy timeout. — [crates/narrata-store-sqlite/src/lib.rs:30-170](../../../../crates/narrata-store-sqlite/src/lib.rs)
- **Reads:**
  - `fn memory(&self) { load_memory(&self.connection) }`. Every trait read (`get_object`, `read_ref`, …) calls `self.memory()?.<op>`.
  - `load_memory` runs `SELECT … FROM objects ORDER BY id` and **re-hash-verifies every object**, then loads all refs, catalogs, and so on.
  - Source: [lib.rs:192-275, 409-440, ~734](../../../../crates/narrata-store-sqlite/src/lib.rs)
- **Writes:**
  - `commit` and `claim_effect`/`record_effect_outcome` follow this pattern: `BEGIN IMMEDIATE`, then `load_memory`, then the `MemoryStore` operation, then `write_state`, then commit.
  - `write_state` executes `DELETE FROM object_edges; … DELETE FROM objects;` and re-INSERTs **all** objects and edges.
  - Source: [lib.rs:278-357, 762-800](../../../../crates/narrata-store-sqlite/src/lib.rs)
- **Edge recomputation:** for every Commit, it scans all Program objects and **fully decodes and validates each one** (`load_program`) to find the matching artifact. Without a match it fails with `InvalidGraph("Program edge")`. — [lib.rs:948-977](../../../../crates/narrata-store-sqlite/src/lib.rs)
- `MemoryStore` uses `objects: BTreeMap<ObjectId, StoredObject>`. Its export form `MemoryStoreState` is `objects: Vec<(CheckedObject, u64)>`. — [crates/narrata-store/src/memory.rs:38-52](../../../../crates/narrata-store/src/memory.rs)
- ADR 0006 states the intent: "`MemoryStore` 是独立于 SQLite 表布局的行为参考；`SqliteStore` 在 `BEGIN IMMEDIATE` 的一致性视图中应用同一 checked transaction" (`MemoryStore` is a behavioural reference independent of the SQLite table layout; `SqliteStore` applies the same checked transaction inside a `BEGIN IMMEDIATE` consistent view). — [docs/adr/0006-stage-2-persistence-boundary.md](../../../adr/0006-stage-2-persistence-boundary.md)

**Full-history walks (IMPL)**
- `load_commit` restores the snapshot and then `validate_ancestor_closure` walks **every parent back to Genesis**, with a cap of 1,000,000. Each step is a `read_commit`, which is a `get_object`. On SQLite each of those is a whole-DB load. — [coordinator.rs:1814-1889](../../../../crates/narrata-store/src/coordinator.rs)
- `redo_candidates` calls `list_objects()` and decodes every Commit. — [coordinator.rs:942-957](../../../../crates/narrata-store/src/coordinator.rs)
- A dead-code `find_program_object` also scans all objects. — [coordinator.rs:2088-2102](../../../../crates/narrata-store/src/coordinator.rs)

**Bundles (IMPL)**
- CheckpointBundle and TimelineArchiveBundle export the object closure minus a `receiver_has: &BTreeSet<ObjectId>` set (incremental export). — [crates/narrata-store/src/bundle.rs:93-103, 182-265](../../../../crates/narrata-store/src/bundle.rs)
- The closure requires the Program Artifact object. — [bundle.rs:778-800](../../../../crates/narrata-store/src/bundle.rs)
- The bundle code has no streaming reader (grep finds no `Read`, `stream`, or `chunk`). The plan item "streaming size/hash verification" is therefore not visibly implemented. — [docs/plan/02-time-travel-and-persistence.md:165](../../../../archive/plan/02-time-travel-and-persistence.md)

**Capability negotiation (IMPL)**
- `negotiate_capabilities(&program.artifact().capabilities, host)` runs at session create and open. — [coordinator.rs:213](../../../../crates/narrata-store/src/coordinator.rs)
- `CapabilityDeclV0{id, version, requirement, request_schema, response_schema, delivery, rewind}`. — [crates/narrata-core/src/effect.rs:171-179](../../../../crates/narrata-core/src/effect.rs)
- Effect ledger rules: the ledger is keyed by `(ExecutionId, EffectId)`, sits outside Snapshots, has a monotonic fence, and uses rewind barriers. — [docs/adr/0007](../../../adr/0007-stage-3-effect-host-boundary.md)

**Compound saves (IMPL)**
- `CompoundSaveManifest{narrative: CommitId, host, program, content_lock: ContentLockId, ledger_fence}`. A host trait verifies the content lock (`fn verify_content_lock(&self, ContentLockId)`). — [crates/narrata-store/src/federated.rs:80-150](../../../../crates/narrata-store/src/federated.rs)

**Protocol engine (IMPL)**
- The engine keeps `programs: BTreeMap<ProgramArtifactId, Arc<CheckedProgram>>` and a per-session `store: MemoryStore`, created with `MemoryStore::new()` at lines 228, 329, and 585. — [crates/narrata-protocol/src/engine.rs:75-105](../../../../crates/narrata-protocol/src/engine.rs)
- Persistence crosses the boundary only as `CheckpointExport`/`Import` and `TimelineArchiveExport`/`Import` bundle bytes. `ProgramLoad{bytes artifact}` sends the whole artifact in one message. — [narrata.proto:40-55](../../../../crates/narrata-protocol/proto/narrata.proto)

**TS IndexedDB store (IMPL)**
- It has two object stores, `objects` and `refs`, and writes objects plus CAS refs in one IndexedDB transaction.
- Defaults are 128 MiB max per object and 512 MiB max per commit.
- It is not wired as a `SaveStore`: it is a TypeScript re-implementation of the object/ref idea.
- Source: [bindings/typescript/src/indexeddb-store.ts:1-60](../../../../bindings/typescript/src/indexeddb-store.ts)

**R1 Gamebook persistence (IMPL)**
- `Session` is an in-memory `Vec<Commit>` with immutable single-parent commits. — [runtime.rs:105-113](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs)
- `SaveArchive{format_version, artifact_id, commits: Vec<SavedCommit{id, parent, action, snapshot: serde_json::Value}>, cursor}`. Restore re-validates everything by replay. — [runtime.rs:175-193, 448-525](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs)
- Limits: 512 commits, 2 MiB retained, 128 KiB per state, and 1M replay steps. — [runtime.rs:14-19](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs)
- The web reader stores **the entire story source (≤4 MiB) plus the entire save JSON (≤4 MiB) in one IndexedDB record keyed "active"** on every action, with a revision-UUID CAS. — [examples/gamebook-web/src/storage.ts](../../../../examples/gamebook-web/src/storage.ts); [worker.ts:17-23](../../../../examples/gamebook-web/src/worker.ts)

**In-memory loading (IMPL)**
- `load_program` decodes the full artifact, which is bounded by default `DecodeLimits` (16 MiB). — [limits.rs](../../../../crates/narrata-core/src/limits.rs)
- The node `compile` builds the whole `CheckedProduct` holding the full `Bundle` (`source`), and enforces `MAX_DOCUMENT_BYTES = 4 MiB`. — [compile.rs:21-27, 197](../../../../packages/narrata/nodes/crates/narrata-nodes/src/compile.rs)
- `MAX_NODES = 4096` is counted product-wide: one counter is initialized at compile.rs:231 and checked at 325-326. — [compile.rs](../../../../packages/narrata/nodes/crates/narrata-nodes/src/compile.rs); [lib.rs:37-41](../../../../packages/narrata/nodes/crates/narrata-nodes/src/lib.rs)

**Doc vs. implementation drift**
- The architecture doc's sketch of `SaveStore` has `put_object`, `roots()`, and `get_object -> Bytes`. The implemented trait has no `put_object` (writes go through `commit`), no `roots()`, and has the `list_*` enumerators. — [docs/architecture/time-travel-and-save.md:499-523](../../../architecture/time-travel-and-save.md) vs [store.rs:310-369](../../../../crates/narrata-store/src/store.rs)

### Inferences
- The **conceptual** storage model is backend-agnostic and well suited to many backends: content-addressed immutable objects, revisioned CAS refs, an append-only ledger, and explicit transactions. The **implemented** trait plus SQLite adapter, however, scale O(total DB size) per operation, including reads. Load is O(history length × DB size) because of the ancestor walk. GC and edges are effectively O(commits × programs × program size).
- That is fine for G2's stated scope (correctness proofs) but blocks "ultra-long narratives". It also means the SQLite adapter is not a model a PostgreSQL or graph-DB adapter should copy.
- A new backend could implement `SaveStore` today, but it would have to support `list_objects()` returning everything. The `MemoryStore`-roundtrip pattern would push any adapter toward the same whole-state rewrite.

### Gaps
- No benchmark covers SQLite latency or growth over thousands of commits. The plan explicitly defers this to hosts. — [docs/plan/stage-2-time-travel-persistence.md:104-107](../../../../archive/plan/stage-2-time-travel-persistence.md)
- I did not read `memory.rs` GC internals beyond the struct, or `catalog.rs` and `manifest.rs` in detail.

---

## Q4. Mechanisms for updating narratives while saves exist (versioning, migrations, descriptors, frozen-save compatibility, hot reload)

### Takeaway
The policy is **exact-artifact restore by default, otherwise an explicit trusted migration**. Migration is implemented as relocation tables between exact `ProgramArtifactId`s, recorded as a Migration Commit. It is exposed via `narrata migrate inspect|dry-run|apply` over SQLite and guarded by a frozen compatibility corpus. **Hot reload is design-only.** Because text is part of the artifact hash, *any* content edit (including typo fixes) is a new artifact that needs either the old artifact or a migration descriptor. The node stack has no migration at all: restore requires `archive.artifact_id == product.artifact_id`.

### Cited Findings
- Policy: "默认恢复策略必须是：用存档固定的精确 Program Artifact 继续执行。只有精确构件不可用或产品明确升级存档时，才运行声明过的 migration" (by default, a save resumes on the exact Program Artifact it was pinned to; a declared migration runs only when that artifact is unavailable or the product explicitly upgrades saves).
- The doc defines a load decision tree: exact restore, else migration path, else `NeedsProgram`/`Incompatible`.
- It defines four identity types: ProgramId, StableId, ProgramArtifactId, and BuildProvenanceId.
- Source: [docs/architecture/program-versioning-and-migration.md:6-25, 247-260](../../../architecture/program-versioning-and-migration.md)
- `MigrationDescriptor{id, from: ProgramArtifactId, to, accepted_snapshot_schemas: VersionRange, relocations: RelocationTable, recovery_points, recovery_locations}`.
  - `RelocationTable` maps instructions, flows, globals, locals, choices, actions, states, histories, events, types, fields, variants, and capabilities, plus dropped globals and locals.
  - `MigrationOptions{allow_lossy_recovery, allow_effect_rekey}`.
  - Source: [crates/narrata-core/src/migration.rs:28-104](../../../../crates/narrata-core/src/migration.rs)
- `ProgramRegistry` and `MigrationRegistry` (path search across artifacts) and `MigrationDryRun`. — [crates/narrata-store/src/migration.rs:27-235](../../../../crates/narrata-store/src/migration.rs)
- A migration Commit has `cause = Migration(MigrationId)`. The ancestor validator requires that the program *changes* across a migration edge and stays the same across runtime edges. — [coordinator.rs:1862-1880](../../../../crates/narrata-store/src/coordinator.rs)
- ADR 0009 sets three rules:
  - It uses a chain of explicit trusted descriptors and does not let a new Program guess an old continuation.
  - A pending effect relocation changes idempotency identity and needs confirmation.
  - "冻结 corpus 是只读兼容证据" (the frozen corpus is read-only compatibility evidence).
  - Source: [docs/adr/0009-stage-5-migration-and-protocol-boundary.md](../../../adr/0009-stage-5-migration-and-protocol-boundary.md)
- The frozen corpus `fixtures/compat/stage5-v0/manifest.json` holds SHA-256 hashes for program, snapshot, receipt, commit, checkpoint bundle, and timeline archive bytes, plus an expected continuation `{kind: "say", text: "Hello"}`. — [fixtures/compat/stage5-v0/manifest.json](../../../../fixtures/compat/stage5-v0/manifest.json)
- The CLI `migrate` runs against `SqliteStore`. — [crates/narrata-cli/src/commands/migrate.rs:22, 92](../../../../crates/narrata-cli/src/commands/migrate.rs)
- Separate version axes exist for ProgramFormat, Semantics, SnapshotSchema, ReceiptSchema, Envelope, and Protocol. — [crates/narrata-core/src/version.rs](../../../../crates/narrata-core/src/version.rs); [program-versioning-and-migration.md:133-144](../../../architecture/program-versioning-and-migration.md)
- **Hot reload is DOC only:** "只在 committed safe point 发生，本质上是同一套 migration… 若新 build 的 ProgramArtifactId 不变，只换 source/debug metadata" (it happens only at a committed safe point and is essentially the same migration; if the new build's ProgramArtifactId is unchanged, only source/debug metadata is swapped). A grep for hot reload in `crates`, `packages`, `bindings`, and `examples` finds no code. — [program-versioning-and-migration.md:218-225](../../../architecture/program-versioning-and-migration.md)
- **Content Lock is DOC (conceptual):**
  - `ContentLockV1{capability_schemas, external_content, runtime_modules}` is referenced by the artifact.
  - Resolution policies are `Pinned`, `Compatible`, and `LivePresentationOnly` ("可取当前内容，但该内容不得影响 guard、choice 或 Effect payload", meaning current content may be fetched but must not affect guards, choices, or Effect payloads).
  - In code, `ProgramArtifactV0` has **no content_lock field**. `ContentLockId` appears only in compound-save manifests.
  - Sources: [program-versioning-and-migration.md:84-117](../../../architecture/program-versioning-and-migration.md); [federated.rs](../../../../crates/narrata-store/src/federated.rs)
- **Node stack:**
  - `artifact_id = digest("NARRATA-NODES-PRODUCT-1", (&source /*full bundle incl. content text*/, &node_types, graph_fingerprints))`. — [compile.rs:464-467](../../../../packages/narrata/nodes/crates/narrata-nodes/src/compile.rs)
  - `Session::restore` rejects archives whose `artifact_id` differs. — [runtime.rs:448-456](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs)
  - The README says "节点 ID 是持久身份，修改内容或显示标题时保持 ID" (node IDs are persistent identities; keep the ID when editing content or display titles), but the save still breaks on any content edit. — [products/gamebook-demo/README.md](../../../../products/gamebook-demo/README.md)
  - The web reader persists its own copy of the story source next to the save, so a newly deployed story does not replace a stored session. — [worker.ts:25-40](../../../../examples/gamebook-web/src/worker.ts)
- REBUILD_PROPOSAL §8 lays out graded replaceability:
  - Visual asset swap.
  - A different provider for a new work.
  - A provider swap for an existing save.
  - Live unload ("后置能力", a deferred capability).
  - Snapshots should record selected package instances, state schemas, and composition identity. Unknown state namespaces may be preserved as an archive, but execution stops if a required provider is missing.
  - Source: [REBUILD_PROPOSAL.md:400-419](../../../../archive/2026-09-05-rebuild-proposal.md)

### Inferences
- For "dynamically-updated narratives", the existing machinery is strong for **structural** changes: stable IDs, relocation, lossy recovery with reports, and barrier awareness. It is maximally strict for **content-only** changes, because nothing separates "presentation text revision" from "logic artifact" identity. The Content Lock / LivePresentationOnly design is exactly the hook needed, but it is unimplemented and currently contradicted by text living in Snapshots.
- Migration descriptors are hand-authored maps between two exact artifacts. There is no notion of an artifact made of independently versioned chunks (per-chapter or per-package). Updating one chapter of a very long work therefore re-identifies the whole work.

### Gaps
- I did not verify whether a no-op or identity `MigrationDescriptor` (artifact A→B with empty relocations) is accepted for pure text edits. That would be the current workaround.

---

## Q5. What the ADRs, architecture docs, plans, research, proposal, and `.temp` notes decided (kernel vs domain vs presentation vs host; storage; media; long narratives; scale)

### Takeaway
The docs make firm, accepted decisions in five areas:
- A pure deterministic core: no I/O, callbacks, or async.
- A content-addressed immutable object graph with CAS refs, using full Snapshots and not pure event sourcing.
- An explicit effect ledger outside rewindable state.
- Exact-artifact restore plus explicit migration.
- Strict canonical CBOR.

The 2026-09-05 **REBUILD_PROPOSAL** reframes the product as "Kernel + first-party domain packages + replaceable presentation + host". It explicitly lists "存储 adapter 的公共协议" (a public protocol for storage adapters) as a kernel responsibility and puts Studio, Web Player, and media pipeline in separate repos. It does **not** specify long-narrative storage (chunking or lazy loading) or DB backends beyond SQLite and IndexedDB.

### Cited Findings
**ADRs (all "Accepted" unless noted)**
- **0001**: Stage 1 merges G0 and G1. Store and Statechart are deferred, and no empty `narrata-store` is created. — [0001](../../../adr/0001-stage-1-scope.md)
- **0002**: Authored IDs are non-interchangeable 16-byte types. Derived IDs are 32-byte domain-separated SHA-256. — [0002](../../../adr/0002-identities-and-digests.md)
- **0003**: A custom deterministic CBOR profile: shortest integer encoding, definite lengths, no floats or tags, increasing integer map keys, reject unknown or duplicate fields, decode→re-encode must be byte-identical, and an envelope carrying a payload SHA-256. "新增字段必须显式版本化" (new fields must be explicitly versioned). — [0003](../../../adr/0003-deterministic-cbor-profile.md)
- **0004**: `RuntimeStateV0` represents only safe states. The eval stack is empty at Say, Choice, and Finish. Slice yield is never persistable. — [0004](../../../adr/0004-flow-vm-safe-points.md)
- **0005**: Input idempotency belongs to the coordinator or store, not the core. The dedup history is not placed in the Snapshot. — [0005](../../../adr/0005-input-idempotency-ownership.md)
- **0006**: The core emits pure `TransitionDraft`. `SessionCoordinator` is the only publish path: Snapshot, Receipt, Commit, input index, Ref, and Catalog are written in one `SaveStore` transaction. `ObjectId` is separate from runtime digests. MemoryStore is the behavioural reference, SQLite applies the same checked transaction, and row IDs and wall time are not part of identity. — [0006](../../../adr/0006-stage-2-persistence-boundary.md)
- **0007**: Capability negotiation happens at session load. At most one pending effect is allowed. EffectId is derived before dispatch, and commit happens before dispatch. The ledger sits outside the Snapshot with a monotonic fence and barriers. Compound saves combine Commit, Host Snapshot, Program, Content Lock, and fence. There is no generic exactly-once guarantee. "Declarative `SceneState` 是呈现真相" (declarative SceneState is the presentation truth). — [0007](../../../adr/0007-stage-3-effect-host-boundary.md)
- **0008**: Statechart is an optional typed extension of Program and RuntimeState. Only the ordered leaf set persists. It is a W3C SCXML run-to-completion subset. — [0008](../../../adr/0008-stage-4-statechart-semantics.md)
- **0009**: Migration uses an explicit descriptor chain. Protobuf is transport-only and never inside a canonical hash. The C ABI uses opaque handles. Bindings must not keep authoritative state. "IndexedDB adapter 用一个 transaction 写 immutable objects 与 CAS refs" (the IndexedDB adapter writes immutable objects and CAS refs in one transaction). — [0009](../../../adr/0009-stage-5-migration-and-protocol-boundary.md)
- **0010**: No Nickel adapter is published (no sandbox proof). Strict JSON and Rust typed manifests are used instead. — [0010](../../../adr/0010-nickel-adapter-not-published.md)
- **0011** ("已实施的 alpha 基线", an implemented alpha baseline):
  - R1 nodes are independent of the old core.
  - The checked vocabulary is content, decision, branch, mutate, call, and return.
  - The JSON/hash profile is separate from CBOR.
  - Saves carry full snapshots of all retained commits and are replay-validated, with these limits: 4 MiB document, 128 KiB per state, 512 commits, 2 MiB total.
  - "这些预算和执行词汇不限制未来 Scene/Quest 的领域模型" (these budgets and the execution vocabulary do not constrain the future Scene/Quest domain model).
  - Source: [0011](../../../adr/0011-r1-node-composition.md)

**Architecture docs ("已决定", decided)**
- **runtime-model.md**: `reduce(program, committed_state, input, limits) → TransitionDraft`. "数据库、文件、墙上时间…都不能由 core 直接读取" (databases, files, wall time, and so on must never be read directly by the core). Flow VM is for short sequences and Statecharts for long-lived, event-driven modes.
  - The sketch lists `rng`, `logical_time`, and `internal_events` fields that are not in the implemented `RuntimeStateV0`.
  - Source: [docs/architecture/runtime-model.md:6-83](../../../architecture/runtime-model.md)
- **time-travel-and-save.md** (status "已决定；编码库与性能阈值仍需 Phase 0 spike 证明", decided, though the encoding library and performance thresholds still need a Phase 0 spike):
  - Immutable Snapshot Commit graph plus mutable Refs. "这不是纯 Event Sourcing… Narrata 反过来把版本化的完整 Runtime State 当恢复真相" (this is not pure event sourcing; Narrata instead treats the versioned full Runtime State as the recovery truth). — [lines 6-25](../../../architecture/time-travel-and-save.md)
  - Standard checkpoint vs. opt-in Complete timeline. — [lines 44-80](../../../architecture/time-travel-and-save.md)
  - "v1 在每个持久 safe point 编码完整 Snapshot… 测量证明存储或延迟不合格后，再把 `ValueStore`、scene 或大 bytes 拆成可共享 object；Commit 与 Ref 协议无需因此改变" (v1 encodes a full Snapshot at every persistent safe point; only after measurements show storage or latency is inadequate will ValueStore, the scene, or large byte blobs be split into shareable objects, without changing the Commit/Ref protocol). — [line 166](../../../architecture/time-travel-and-save.md)
  - Adapters are MemoryStore, SqliteStore, and a Wasm/IndexedDB adapter. "自制目录 + rename 的文件存储留到 SQLite 版本通过 crash test 后" (a home-made directory-plus-rename file store waits until the SQLite version passes crash tests). — [lines 499-530](../../../architecture/time-travel-and-save.md)
  - Mark-and-sweep GC over Refs, Pins, sessions, and effect roots. — [lines 532-556](../../../architecture/time-travel-and-save.md)
- **program-versioning-and-migration.md**: covered in Q4 (StableId rules, Content Lock, REZICS external content, hot reload, load tree).
- **effects-and-host-state.md**: the rewindable timeline is separate from the monotonic external world. Three host-state modes are defined: narrative-authoritative, federated save, and external-authoritative. — [docs/architecture/effects-and-host-state.md:6-22, 272-336](../../../architecture/effects-and-host-state.md)
- **rezics-gamebook-integration.md** (decided 2026-08-31):
  - REZICS owns posts, Portable Text, localized content, content structures, occurrence identity, and permissions.
  - Narrata owns edges, entries, branches, endings, execution state, snapshots, and the reader interaction.
  - Graph nodes reference a REZICS `ContentStructureNode` occurrence, not a Post ID.
  - REZICS "可以保存 Narrata 关系文档和用户存档… 这种持久化不使 REZICS 成为叙事关系的语义所有者" (may store Narrata relation documents and user saves, but that persistence does not make REZICS the semantic owner of narrative relations).
  - Source: [docs/rezics-gamebook-integration.md:6-79, 149-208](../../../../archive/2026-08-31-rezics-gamebook-integration.md)

**Plans**
- **plan/README.md**:
  - Explicitly deferred: visual editor and full DSL, arbitrary callbacks, async continuation, float branching, multiple concurrent effects, timeline merge/CRDT, cloud sync auto-merge, WIT as the only ABI, and Nix/Nickel at runtime.
  - Highest risk #2: "在每个 safe point 写全量 Snapshot 的延迟与存储量是否可接受" (whether the latency and storage of writing a full Snapshot at every safe point are acceptable). #3: whether SQLite and IndexedDB give atomic object-plus-Ref visibility.
  - Source: [docs/plan/README.md:102-123](../../../../archive/plan/README.md)
- **02-time-travel-and-persistence.md P2.10**: "建立可复现 benchmark，而不是先做 delta" (build a reproducible benchmark instead of starting with deltas). It targets 1K/10K/100K variables, 1K/10K commits for load, rewind, and GC, Catalog archives, SQLite transaction latency, and native/Wasm peak memory. If needed, "拆分最大的 immutable subtree… 不引入无界 delta chain" (split the largest immutable subtree; do not introduce unbounded delta chains). — [docs/plan/02-time-travel-and-persistence.md:186-198](../../../../archive/plan/02-time-travel-and-persistence.md)

**Research (2026-09-05)**
- **competitive-architecture §3.2**: separate content identity, occurrence identity, and run-time interaction identity, plus language, rich-text parameters, voice, and logical asset references. Adopt Yarn line-ID and shadow-line ideas. — [docs/research/2026-09-05-narrata-competitive-architecture.md:87-101](../../2026-09-05-narrata-competitive-architecture.md)
- **competitive-architecture §3.4**: SceneState is a VN abstraction, not host-neutral. The preference is to keep it as an optional first-party VN model. — [same doc:110-121](../../2026-09-05-narrata-competitive-architecture.md)
- **competitive-architecture §4**, if benchmarks warrant:
  - compact execution addresses with persistent-ID mapping;
  - structural sharing in memory;
  - "物理存储采用分块去重、压缩或带检查点的增量表示，同时保留可验证的完整逻辑状态" (physical storage using chunked dedup, compression, or checkpointed incremental representations, while keeping a verifiable full logical state);
  - "Core/Store 已经分离，应改进默认 API 和打包" (Core and Store are already separated; improve the default API and packaging instead);
  - and it says "本次没有 benchmark，不能据此宣布性能差" (there was no benchmark this round, so poor performance cannot be claimed on this basis).
  - Source: [same doc:143-155](../../2026-09-05-narrata-competitive-architecture.md)
- **competitive-architecture §5**: graph-first vs. text-first authority is still open. Avoid two drifting truths. — [same doc:157-163](../../2026-09-05-narrata-competitive-architecture.md)
- **toolchain-roadmap §3** proposes a repo split, not yet created:
  - `narrata`: runtime, IR, store, compiler, protocol.
  - `narrata-studio`: authoring.
  - `narrata-web`: player, scene presentation, VN UI.
  - `narrata-media`: import, transcode, variants, preload groups, Media Manifest.
  - "Core 不依赖 DOM、CSS 或浏览器媒体对象" (the core does not depend on DOM, CSS, or browser media objects).
  - Source: [docs/research/2026-09-05-narrata-toolchain-roadmap.md:48-75](../../2026-09-05-narrata-toolchain-roadmap.md)
- **toolchain-roadmap §7** (media pipeline):
  - The build pipeline is separate from the playback runtime.
  - The Manifest records logical ID, kind, variants, hash, bytes, dimensions/duration, language, preload group, and fallback.
  - EntityId serves as the manifest index for now.
  - Character, line, translation, and voice association uses persistent content identity.
  - The release manifest pins `ProgramArtifactId` and the media manifest identity *separately*.
  - "避免一次下载所有路线的全部资源" (avoid downloading every route's resources at once), with chapter preload groups.
  - Source: [same doc:188-243](../../2026-09-05-narrata-toolchain-roadmap.md)
- **narrative-model-evidence §3**: recommends a "类型化、可组合的叙事定义图＋共享领域状态＋活动实例执行" (a typed, composable narrative definition graph plus shared domain state plus activity-instance execution). **"作者定义图不决定物理数据库和所有执行算法… 不要求所有系统使用一种存储容器"** (the author-facing definition graph does not dictate the physical database or every execution algorithm, and does not require all systems to use one storage container). — [docs/research/2026-09-05-narrative-model-evidence.md:76-87](../../2026-09-05-narrative-model-evidence.md)
- **narrative-model-evidence §4**: SceneState should arise from `NodeInstance<Scene>`. — [same doc:89-111](../../2026-09-05-narrative-model-evidence.md)
- **nix-nickel-lessons**: adopt the Nix ideas of immutable objects plus a generation pointer, GC roots and closures, and an exact identity distinct from semantic identity. Nickel is at most an optional build-time config front-end and never touches saves. — [docs/research/nix-nickel-lessons.md:6-21](../../nix-nickel-lessons.md)
- **source-register**: pins the commits of cloned Nix, Nixpkgs, Nickel, Ink, Ren'Py, and Yarn sources used as evidence. — [docs/research/source-register.md](../../source-register.md)

**REBUILD_PROPOSAL.md** (status: authorized for batched implementation; R0/R1 done, the rest is proposal)
- §1 positioning: "可组合的叙事引擎" (a composable narrative engine), built as composable capability packages, then compose/validate/lock, then a unified session state with deterministic actions, then content, actions, events, presentation projections, and host effects, then the Reader, VN, or game host. — [REBUILD_PROPOSAL.md:15-39](../../../../archive/2026-09-05-rebuild-proposal.md)
- §4 Kernel responsibilities include "预算、因果链、诊断、**存储 adapter 的公共协议**" (budgets, causal chains, diagnostics, and a public protocol for storage adapters). "Kernel 不需要把台词、立绘、好感度、任务节点逐个写进一个总枚举。它也不负责图形、碰撞和操作系统窗口" (the kernel need not hard-code lines, character art, affinity, or quest nodes into one master enum, and it is not responsible for graphics, collision, or OS windows). — [REBUILD_PROPOSAL.md:184-192](../../../../archive/2026-09-05-rebuild-proposal.md)
- §5.1 the package manifest must declare "内容/资源：内容 schema、逻辑资源引用、可选显示绑定、构建闭包" (content/resources: content schema, logical asset references, optional display bindings, build closure). — [REBUILD_PROPOSAL.md:219-231](../../../../archive/2026-09-05-rebuild-proposal.md)
- §7.1 candidate `SessionState`: "module states keyed by instance and namespace", restored through registered versioned codecs. — [REBUILD_PROPOSAL.md:342-360](../../../../archive/2026-09-05-rebuild-proposal.md)
- §8 "作者布局、纯皮肤和不参与规则的媒体变体可独立管理；会影响判断…的资源仍进入语义锁。媒体包和规则包分别可替换，但并非任何替换都不影响存档" (author layout, pure skins, and media variants that do not affect rules can be managed independently; assets that affect decisions still enter the semantic lock; media and rule packages are separately replaceable, but not every replacement leaves saves unaffected). — [REBUILD_PROPOSAL.md:419](../../../../archive/2026-09-05-rebuild-proposal.md)
- §9 table of current assumptions to redesign:
  - the forced `entry_flow`;
  - global `globals` with a mandatory SceneState;
  - Say/Choice-only interactions;
  - the placeholder external content;
  - the `u32` occurrence;
  - the Store/coordinator "与旧 RuntimeState 形状绑定" (tied to the shape of the old RuntimeState), to be refactored into cross-module transaction coordination.
  - "值得保留：确定性、checked decoding、显式效果、持久身份、提交/恢复与迁移的验证资产" (worth keeping: determinism, checked decoding, explicit effects, persistent identities, and the verification assets for commit/restore and migration).
  - Source: [REBUILD_PROPOSAL.md:421-440](../../../../archive/2026-09-05-rebuild-proposal.md)
- §10 "当前 repo 维护 kernel、第一方领域能力、公共格式与测试；Studio、Web Player、媒体生产管线继续可以独立 repo" (this repo maintains the kernel, first-party domain capabilities, shared formats, and tests; Studio, Web Player, and the media production pipeline can remain separate repos). — [REBUILD_PROPOSAL.md:442-480](../../../../archive/2026-09-05-rebuild-proposal.md)
- §11 roadmap: R0/R1 Gamebook (done), R2 VN/Scene, R3 quests/companions. **R4 "围绕实际能力建设作者工具和媒体管线"** (build authoring tools and the media pipeline around actual capabilities) covers progression, relationship, situation, causal-timeline, and VN scene editors, plus "资源包可以换画风、声音或媒体变体，作品领域规则保持独立" (asset packages can swap art style, sound, or media variants while the work's domain rules stay independent). — [REBUILD_PROPOSAL.md:482-538](../../../../archive/2026-09-05-rebuild-proposal.md)
- §12 success criteria include "不同 renderer 不掌握领域事实的第二份真相" (no renderer holds a second copy of the domain facts) and "headless 和无媒体产品不被强制链接图形依赖" (headless and media-free products are not forced to link graphics dependencies). — [REBUILD_PROPOSAL.md:540-556](../../../../archive/2026-09-05-rebuild-proposal.md)

**.temp notes (non-normative per [docs/README.md](../../../README.md))**
- **`.temp/叙事引擎调研.md`** (early research):
  - It recommended persistent data structures (structural sharing) for cheap checkpoints, and "週期性完整快照＋每個互動邊界的 persistent root＋外部 Event/EffectResponse 日誌" (periodic full snapshots, plus a persistent root at every interaction boundary, plus an external Event/EffectResponse log).
  - The implementation chose full snapshots and BTreeMaps instead (see the stage-2 perf decision).
  - It proposed a separate repo with `visual-*` crates, where "visual-runtime 不依賴 UI、網路、檔案系統或具體遊戲引擎；所有 I/O 都經由 Effect" (visual-runtime depends on no UI, network, filesystem, or specific game engine; all I/O goes through Effects).
  - Source: [.temp/叙事引擎调研.md:381-470, 669-720](../../../../.temp/%E5%8F%99%E4%BA%8B%E5%BC%95%E6%93%8E%E8%B0%83%E7%A0%94.md)
- **`.temp/ChatGPT-Narrata_项目价值.md`**:
  - Its first verdict: reposition as a "Durable Narrative Runtime" and defer Statechart extras, multiplayer and timeline merge, a cloud save service, and a full renderer.
  - After the owner said Narrata competes with Ink and Yarn and that they plan to "做一些媒体管线，走 web 路线，解决 Ren'Py 99.999% 的问题" (build some media pipelines, take the web route, and solve 99.999% of Ren'Py's problems), the doc revised its view to "open-source, engine-independent narrative engine with a deterministic portable runtime at its core", with the principle "Narrative logic never directly depends on the host engine".
  - Source: [.temp/ChatGPT-Narrata_项目价值.md:11-35, 317-355, 395-520](../../../../.temp/ChatGPT-Narrata_%E9%A1%B9%E7%9B%AE%E4%BB%B7%E5%80%BC.md)

### Inferences
- The owner's thesis (abstract logic core, pluggable storage, content separated from logic) is **already partially stated in settled docs**:
  - ADR 0006 sets a core with no store.
  - Narrative-model-evidence §3 says the definition graph does not dictate the physical database.
  - REBUILD_PROPOSAL §4 says the kernel owns a public storage-adapter protocol.
  - REZICS integration and toolchain §3 and §7 put content, localization, and media outside the engine.
  - A proposal should cite these as settled and extend them, not re-argue them.
- The new parts would be:
  - storage for programs and content too large for memory, which no doc covers;
  - non-SQL backends (PostgreSQL, graph DB, JSON), where the docs mention only SQLite, IndexedDB, and a deferred file store;
  - making content updates not change the logic artifact identity, where the docs designed `LivePresentationOnly` and Content Lock but the implementation contradicts them for internal text.
- One possible conflict to flag: the docs deliberately reject pure event sourcing and use full snapshots as truth. A "dynamically updated, ultra-long" design that leans on replay would contradict settled decision ADR 0006 and time-travel-and-save §核心决定 unless it argues explicitly.

### Gaps
- No doc states numeric scale targets for narrative size (number of nodes, words, or chapters). The only stated thresholds concern Snapshot variables (100K) and node-stack caps.

---

## Q6. Concrete coupling points that block (a) storage swapping, (b) larger-than-memory narratives, (c) live content updates, (d) text/media separation, and what to preserve

### Takeaway
The architecture *intent* is decoupled. The *implementation* has specific choke points:
- the `list_objects()`-style trait plus whole-DB SQLite emulation;
- a monolithic Program blob and in-memory `CheckedProgram`/`CheckedProduct`;
- artifact identity hashing all text;
- text copied into Snapshots and verified on restore;
- a mandatory VN `SceneState` in every RuntimeState;
- two disjoint stacks with separate save formats.

The verified correctness assets (checked decoding, canonical CBOR, CAS refs, effect ledger, migration, frozen corpus) are the parts to preserve.

### Cited Findings and coupling points
**(a) Swapping storage backends**
1. `SaveStore::list_objects() -> Vec<CheckedObject>` and the other `list_*` methods force full enumeration. There are no range, prefix, iterator, or index queries such as "children of commit X". — [store.rs:310-369](../../../../crates/narrata-store/src/store.rs)
2. The coordinator relies on full scans (`redo_candidates`) and on genesis-to-head ancestry walks for every load. — [coordinator.rs:942-957, 1814-1889](../../../../crates/narrata-store/src/coordinator.rs)
3. `SqliteStore` is MemoryStore-over-SQLite: a full load and a full rewrite per operation, including reads. — [narrata-store-sqlite/src/lib.rs:192-357, 409, 762-800](../../../../crates/narrata-store-sqlite/src/lib.rs)
4. Edge derivation requires decoding every Program object per Commit. — [lib.rs:966-977](../../../../crates/narrata-store-sqlite/src/lib.rs)
5. `ProtocolEngine` (FFI, Wasm, TS) hard-codes `MemoryStore` and exposes persistence only as bundle bytes, so hosts cannot plug in a store across the ABI. — [engine.rs:75, 228, 329, 585](../../../../crates/narrata-protocol/src/engine.rs)
6. The TS IndexedDB store is a separate re-implementation, not an adapter of the Rust trait. — [indexeddb-store.ts](../../../../bindings/typescript/src/indexeddb-store.ts)
7. The R1 Gamebook bypasses `SaveStore` entirely and uses a JSON `SaveArchive` plus a single IndexedDB record. — [runtime.rs:175-193](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs); [storage.ts](../../../../examples/gamebook-web/src/storage.ts)
8. The trait is sync and `&mut self`. That is fine for embedded use but awkward for network databases such as PostgreSQL (inference).

**(b) Narratives too big for memory**
1. The Program is one canonical CBOR blob, decoded whole into `CheckedProgram`, which holds `artifact` plus BTreeMaps. — [checked.rs:15-24](../../../../crates/narrata-core/src/program/checked.rs)
2. The default decode cap is 16 MiB (configurable). — [limits.rs](../../../../crates/narrata-core/src/limits.rs)
3. The Program is stored as one object, and every bundle closure includes it. — [coordinator.rs:222-226](../../../../crates/narrata-store/src/coordinator.rs); [bundle.rs:778-800](../../../../crates/narrata-store/src/bundle.rs)
4. `ProtocolEngine` loads the artifact in one `ProgramLoad{bytes}` message. — [narrata.proto:42](../../../../crates/narrata-protocol/proto/narrata.proto)
5. The node stack holds the full `Bundle` in `CheckedProduct.source`. Caps are 4 MiB per document and 4,096 nodes product-wide, and the web reader stores the whole source in IndexedDB on every action. — [compile.rs:21-27, 197, 231, 325](../../../../packages/narrata/nodes/crates/narrata-nodes/src/compile.rs); [worker.ts:17-23](../../../../examples/gamebook-web/src/worker.ts)
6. There are no chunk, lazy-load, or streaming code paths, and no streaming bundle reader (grep).
7. Snapshot growth: each Commit stores a full Snapshot (by design). A node-stack session retains at most 512 commits / 2 MiB. — [time-travel-and-save.md:166](../../../architecture/time-travel-and-save.md); [runtime.rs:14-19](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs)

**(c) Live content updates**
1. `ProgramArtifactId` hashes constants, including all text. `ProgramArtifactV0` has no content-lock field, and there is no BuildProvenance/content split for text. — [validate.rs:227](../../../../crates/narrata-core/src/program/validate.rs); [wire.rs:20-31](../../../../crates/narrata-core/src/program/wire.rs)
2. Exact restore requires `state.program_artifact_id == program.artifact_id()`. — [restore.rs:85](../../../../crates/narrata-core/src/snapshot/restore.rs); [coordinator.rs:1819-1821](../../../../crates/narrata-store/src/coordinator.rs)
3. The node stack's `artifact_id` digests the full bundle, including content, and restore demands equality. — [compile.rs:464-467](../../../../packages/narrata/nodes/crates/narrata-nodes/src/compile.rs); [runtime.rs:448-456](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs)
4. Hot reload and `LivePresentationOnly` are DOC only. The external content resolver is not wired. — [program-versioning-and-migration.md:110-131, 218-225](../../../architecture/program-versioning-and-migration.md); [content.rs](../../../../crates/narrata-core/src/content.rs)
5. A session or coordinator is bound to a single `Arc<CheckedProgram>`. — [coordinator.rs:164-177](../../../../crates/narrata-store/src/coordinator.rs)

**(d) Separating text and media from logic**
1. Say, Choice, and prompt text live in the program constant pool. — [instruction.rs:86-146](../../../../crates/narrata-core/src/program/instruction.rs)
2. Resolved strings are persisted in Snapshots and checked against program constants on restore, so the text is part of `StateDigest`. — [snapshot/wire.rs:180-235](../../../../crates/narrata-core/src/snapshot/wire.rs); [restore.rs:725-753](../../../../crates/narrata-core/src/snapshot/restore.rs)
3. A VN-specific `SceneState` (layers, actors, camera, audio) is mandatory in `RuntimeStateV0`, and `ReconcileScene` embeds scene literals in the Program. — [state.rs:24](../../../../crates/narrata-core/src/runtime/state.rs); [scene.rs](../../../../crates/narrata-core/src/scene.rs)
4. Media is referenced by an untyped `EntityId`: there is no AssetId and no manifest type in code. — [toolchain-roadmap.md:210](../../2026-09-05-narrata-toolchain-roadmap.md)
5. Node choice labels, `disabled_reason`, titles, and `shared_labels` are inline. The engine also hard-codes Chinese end text. — [model.rs:176-188](../../../../packages/narrata/nodes/crates/narrata-nodes/src/model.rs); [runtime.rs:659](../../../../packages/narrata/nodes/crates/narrata-nodes/src/runtime.rs)
6. There are no locale or line-ID types, and `StructureOccurrence.occurrence` is a bare `u32`, which REBUILD §9 flags as possibly unstable. — [content.rs](../../../../crates/narrata-core/src/content.rs); [REBUILD_PROPOSAL.md:421-440](../../../../archive/2026-09-05-rebuild-proposal.md)

**Already done well, to preserve (IMPL plus accepted ADRs)**
- **Pure core with no I/O:**
  - `narrata-core` has no store, clock, RNG, or async, and depends only on hashing and serde. — [lib.rs:1-4](../../../../crates/narrata-core/src/lib.rs); [ADR 0006](../../../adr/0006-stage-2-persistence-boundary.md)
  - `TransitionDraft` is published only via the coordinator.
- **Content-addressed immutable objects plus revisioned CAS refs plus explicit transactions:**
  - This is a backend-agnostic model that maps naturally onto KV, SQL, or object stores.
  - `ObjectId` is domain-separated from runtime digests.
  - Source: [object.rs](../../../../crates/narrata-store/src/object.rs); [store.rs](../../../../crates/narrata-store/src/store.rs)
- **`MemoryStore` as a behavioural reference, plus a model test and fault-injection points:**
  - `FaultPoint` covers snapshot, receipt, commit, edge, ref CAS, catalog, GC, and ledger.
  - This is a ready-made conformance harness for any new backend.
  - Source: [store.rs:122-138](../../../../crates/narrata-store/src/store.rs); [crates/narrata-store/tests/model.rs](../../../../crates/narrata-store/tests/model.rs); [tests/faults_gc.rs](../../../../crates/narrata-store/tests/faults_gc.rs)
- **Untrusted-input discipline:**
  - Checked constructors, decode limits, canonical re-encode equality, and fuzz targets for program, value, snapshot, bundles, and protocol.
  - Source: [ADR 0003](../../../adr/0003-deterministic-cbor-profile.md); [fuzz/fuzz_targets](../../../../fuzz/fuzz_targets)
- **Effect ledger and barriers outside rewindable state**, and compound saves with a content-lock hook. — [ADR 0007](../../../adr/0007-stage-3-effect-host-boundary.md); [federated.rs](../../../../crates/narrata-store/src/federated.rs)
- **Stable IDs, relocation-based migration with lossy-recovery reporting, and a frozen compat corpus.** — [migration.rs](../../../../crates/narrata-core/src/migration.rs); [fixtures/compat/stage5-v0](../../../../fixtures/compat/stage5-v0/manifest.json)
- **Bundle incremental export** via `receiver_has`, which already supports sync and dedup. — [bundle.rs:93-103](../../../../crates/narrata-store/src/bundle.rs)
- **The node stack's package model:**
  - Explicit imports and exports, bindings, a lock with per-package digests and node-type revisions, and a content map separated from the graph by key.
  - It is a foundation for per-package or per-chapter artifacts.
  - Source: [model.rs](../../../../packages/narrata/nodes/crates/narrata-nodes/src/model.rs); [project.lock.json](../../../../products/gamebook-demo/project.lock.json)
- **Content resolution types** (Pinned, RecordFirstResolution, LivePresentationOnly), which are the right abstraction even though they are unwired. — [content.rs](../../../../crates/narrata-core/src/content.rs)

### Inferences
- The highest-leverage refactors that stay consistent with settled docs are:
  1. Turn `SaveStore` into a narrower, index-capable trait (get, put via transaction, ref CAS, parent/child or index queries, iterators) and make SQLite a real row-level adapter.
  2. Split the Program and composition artifact into a logic artifact plus separately identified, lockable content chunks or string tables, implementing the documented ContentLock and LivePresentationOnly.
  3. Stop persisting resolved text in Snapshots: store indices or content-occurrence IDs and resolve at the view boundary.
  4. Make SceneState an optional module state, as REBUILD §4.3 and §9 already propose.
- Each of these touches frozen-corpus bytes (the Snapshot encodes text), so ADR 0009 requires a new format ADR or migration, not in-place edits.

### Gaps
- I did not check whether `MemoryStore::collect` (GC) is O(N) per call (likely) or whether tests assert any complexity bounds.

---

## Q7. Performance numbers and benchmarks

### Takeaway
The only recorded numbers are a 2026-09-01 Criterion baseline for full-Snapshot encode/decode and MemoryStore put/GC. They meet the stated product thresholds, so full snapshots were kept. **There are no SQLite, long-history, large-Program, Wasm-memory, or node-stack benchmarks.**

### Cited Findings
- `scripts/benchmark-g2.ps1` runs `cargo bench -p narrata-store --bench persistence` and `cargo bench -p narrata-testkit --bench kernel`. — [scripts/benchmark-g2.ps1](../../../../scripts/benchmark-g2.ps1)
- The `persistence` bench measures MemoryStore put and GC for 1K/10K objects, and full-Snapshot encode and decode-plus-checked-restore with 1K/10K/100K globals. — [crates/narrata-store/benches/persistence.rs](../../../../crates/narrata-store/benches/persistence.rs)
- The `kernel` bench measures `branch_call_choice` from start to the first Say. — [crates/narrata-testkit/benches/kernel.rs](../../../../crates/narrata-testkit/benches/kernel.rs)
- Recorded on 2026-09-01 (i9-14900HX, Windows, Rust 1.98 release, Criterion 10 samples). Source: [docs/plan/stage-2-time-travel-persistence.md:80-107](../../../../archive/plan/stage-2-time-travel-persistence.md)

  | Workload | Snapshot bytes | Time |
  | --- | ---: | ---: |
  | Encode, 1K variables | 21,942 | 19.2–19.7 µs |
  | Decode + restore, 1K | 21,942 | 304–352 µs |
  | Encode, 10K | 219,942 | 284–350 µs |
  | Decode + restore, 10K | 219,942 | 3.57–4.02 ms |
  | Encode, 100K | 2,268,872 | 4.72–5.59 ms |
  | Decode + restore, 100K | 2,268,872 | 43.3–53.7 ms |
  | MemoryStore put, 1K objects | — | 192–196 µs |
  | MemoryStore put, 10K objects | — | 2.57–2.61 ms |
  | MemoryStore GC, 1K orphans | — | 56–57.6 µs |
  | MemoryStore GC, 10K orphans | — | 810–847 µs |

- Thresholds are 100K-variable encode under 10 ms and restore under 75 ms. Both were met, so "G2 保持全量 Snapshot，不引入 delta chain 或结构共享" (G2 keeps full Snapshots and introduces neither delta chains nor structural sharing).
- "SQLite transaction latency、真实故事 call depth、1K/10K Commit/Catalog archive 与 native/Wasm peak memory 必须由具体宿主在其发布硬件上继续记录" (SQLite transaction latency, real-story call depth, 1K/10K Commit/Catalog archives, and native/Wasm peak memory must be recorded by each host on its release hardware). — [same doc:104-107](../../../../archive/plan/stage-2-time-travel-persistence.md)
- REBUILD_PROPOSAL §9 states "本次检查的是源码与文档，没有运行新的整体 gate 或 benchmark" (this round examined source and docs only and ran no new overall gate or benchmark). — [REBUILD_PROPOSAL.md:423](../../../../archive/2026-09-05-rebuild-proposal.md)

### Inferences
- The benchmarked paths (in-memory snapshot codec and MemoryStore) do not exercise the SQLite whole-DB round-trip, which is the most likely scaling cliff. A proposal should add benchmarks over N commits for SQLite and any new backend, and for large programs (instructions and constants) and large bundles, before choosing chunking or delta strategies. This matches the existing doc rule "建立可复现 benchmark，而不是先做 delta" (build a reproducible benchmark instead of starting with deltas) ([02-time-travel-and-persistence.md:186-198](../../../../archive/plan/02-time-travel-and-persistence.md)).

### Gaps
- No numbers exist for SQLite operations, program load at scale (for example 1M instructions or constants), Wasm memory, node-stack compile/restore at the 4 MiB / 4,096-node caps, or bundle import of large closures. I did not run any benchmarks myself (read-only task).
