# Stage 2：Time Travel and Crash-Safe Persistence

状态：已实施（G2）
日期：2026-09-01
格式稳定性：`v1` save wire 为实验格式；Rust API 为 `0.x` unstable

## 面向集成者的能力

Stage 2 把 Stage 1 的 safe-point `RuntimeState` 发布为不可变 Commit 图：

```text
checked Input
→ isolated TransitionDraft
→ Snapshot + Receipt + Commit
→ atomic branch/active Ref CAS
→ CommittedRunResult
```

`narrata-store` 提供：

- domain-separated `ObjectId` 和不可互换的 Snapshot/Receipt/Commit/Catalog/manifest ID；
- `MemoryStore`、typed transaction、revision CAS、InputId 幂等索引、pin/lease 与 mark/sweep；
- `SessionCoordinator` 的 save/load、rewind、redo、显式/自动 fork 和 bookmark；save 支持
  create/update/rename/copy/delete，bookmark 支持 create/rename/delete，branch 支持
  create/advance/delete；
- 默认关闭的 `TimelineRecordingMode::Complete`、不可伪造 coverage baseline、Catalog Event chain、
  `Seal`/`Delete` 两种停止语义；
- 使用不同判别 tag 的 `CheckpointBundle` 与 `TimelineArchiveBundle`，支持 receiver-have-set
  incremental export、archive-local 名称映射和验证后原子 import；
- 有总 bytes、单对象 bytes 和对象数限制的 bundle decoder，以及 bundle fuzz target。

`narrata-store-sqlite` 提供 native reference adapter。它使用 bundled SQLite，创建逻辑
`objects`、`object_edges`、`refs`、`catalog_heads`、`archive_refs`、`pins` 和 `input_index` 表；
所有 identity 使用 Narrata bytes，不依赖 SQLite row ID。

## SQLite 部署边界

文件数据库请求 `journal_mode=WAL`、`synchronous=FULL`、foreign keys 和 5 秒 busy timeout；每笔
写入使用 `BEGIN IMMEDIATE`。不同连接在同一 transaction snapshot 上执行 checked graph 与 CAS
验证，因此一个 slot 的两个竞争写入最多一个成功。

这些设置不替宿主承诺任意网络文件系统、虚拟磁盘或断电硬件的持久性。部署方应在目标文件系统
执行 crash/power-loss 验证；不确定时使用本机磁盘，并把 bundle 同步视为独立的不可信输入路径。
`StoreError` 分开表达 busy、full、I/O、database corruption、object corruption 与 CAS conflict。

## 保存、归档与删除语义

普通 save slot 是当前 Commit 的一个 Ref。覆盖、改名或删除 Ref 不会记录旧目录状态；其旧 Commit
仅在仍被其他 root、lease 或 grace period 保留时可达。

完整记录会从明确 baseline 起追加 branch/save/bookmark Catalog Event，并由 Catalog Head 或已封存
archive root 保留其 closure。用户从存档列表删除 save 后，旧 Commit 仍可由 tombstone/Catalog
访问。只有删除或显式 prune archive root、移除其他 root、再完成 GC 后，宿主才可报告物理清除。
这一区分必须出现在产品的保存与隐私 UI 中。

## 可执行验证

完整本地 Gate：

```powershell
./scripts/check-g2.ps1
```

Gate 覆盖：

- initial/normal Commit golden bytes；
- restore/commit/run 等价、rewind replay 去重、不同未来自动 fork；
- 独立 CAS reference model 的随机命令序列；
- Complete 的 Catalog、save tombstone、session view 和 archive round-trip；
- checkpoint 缺失/截断/hash 损坏/wrong tag/limit failure；
- Snapshot、Receipt、Commit、edge、Ref、Catalog、Archive Ref、bundle import、GC mark/sweep 与
  transaction boundary 的故障注入原子性；
- GC dry-run/sweep、root closure 和 Program Artifact retention；
- SQLite reopen/integrity、双连接 slot CAS 和 transaction rollback；
- bundle fuzz build，以及 `narrata-store` 的 `wasm32-wasip1` build。

CLI 的 `timeline log/rewind/redo/fork/bookmark` 只输出 turn、短 Commit 摘要和 Ref revision，不暴露
数据库内部对象路径。运行 `cargo run -p narrata-cli --` 可查看参数格式。

## 性能基线与决定

基线命令：

```powershell
./scripts/benchmark-g2.ps1
```

2026-09-01 在 Intel Core i9-14900HX、Windows x86_64、Rust 1.98 release build 上，以 Criterion
10 samples、1 秒 warm-up/measurement 得到：

| workload | bytes | observed interval |
| --- | ---: | ---: |
| full Snapshot encode，1K variables | 21,942 | 19.2–19.7 µs |
| full Snapshot decode + checked restore，1K | 21,942 | 304–352 µs |
| full Snapshot encode，10K | 219,942 | 284–350 µs |
| full Snapshot decode + checked restore，10K | 219,942 | 3.57–4.02 ms |
| full Snapshot encode，100K | 2,268,872 | 4.72–5.59 ms |
| full Snapshot decode + checked restore，100K | 2,268,872 | 43.3–53.7 ms |
| MemoryStore checked put，1K objects | — | 192–196 µs |
| MemoryStore checked put，10K objects | — | 2.57–2.61 ms |
| MemoryStore mark/sweep，1K orphan objects | — | 56.0–57.6 µs |
| MemoryStore mark/sweep，10K orphan objects | — | 810–847 µs |

当前产品门槛是 100K-variable Snapshot encode 小于 10 ms、checked restore 小于 75 ms；本基线
满足门槛，因此 G2 保持全量 Snapshot，不引入 delta chain 或结构共享。SQLite transaction latency、
真实故事 call depth、1K/10K Commit/Catalog archive 与 native/Wasm peak memory 必须由具体宿主在其
发布硬件上继续记录；它们是容量规划数据，不改变本阶段的 Commit/Ref 正确性协议。

## Stage 边界

Stage 2 不执行不可逆宿主 Effect，`ledger_fence` 保持零。Effect ledger、commit-before-dispatch、
rollback barrier 和联合 Host Snapshot 属于 Stage 3。Program migration、C/Wasm binding 和 Nickel
adapter 仍按 Phase 5 进入。
