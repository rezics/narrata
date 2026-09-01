# Phase 2：时间旅行与持久化

状态：已实施（G2）；合并实现与可执行 Gate 见
[Stage 2：Time Travel and Crash-Safe Persistence](./stage-2-time-travel-persistence.md)
前置：[Phase 1](./01-runtime-kernel.md)
完成后解锁：[Phase 3](./03-effects-and-host-coordination.md)、Phase 5 migration/binding

## 目标

把 Phase 1 的 safe-point Snapshot 变成崩溃安全的不可变 Commit 图。交付 save/load、rewind、
redo、fork、CAS save slot、Checkpoint Bundle、默认关闭的完整 Timeline Archive 和 GC；仍不
执行不可逆宿主 Effect。

## P2.1 Object、Commit、Receipt 与 Ref

在 `narrata-store` 实现 checked 类型：

```text
ObjectEnvelope / ObjectId / ObjectKind
SnapshotObject / TransitionReceipt / Commit
RefKey / RefValue / RefRevision
TimelineCatalogEvent / CatalogHeadRef / TimelineArchiveRef / TimelineCoverage / ArchiveRefName
CheckpointBundleManifest / TimelineArchiveManifest
Pin / Lease / RetentionPolicy
```

交付：

- canonical payload/hash 使用 Phase 0 codec；
- Commit 引用 parent、Execution、Program、Snapshot、Receipt、LedgerFence；
- Commit Ref、Catalog Head Ref 与 Timeline Archive Ref 使用不同 checked key/target 类型；
- object kind substitution、悬空引用和 schema mismatch 被拒绝；
- wall time、slot name、telemetry 不进入确定性 hash；
- initial/normal Commit 的 golden vectors。

## P2.2 MemoryStore 与参考模型

交付：

- `MemoryStore` 实现 put/get、transaction、read/CAS Ref、pin、mark/sweep；
- 简单 reference model 独立表达预期 object/ref 集合；
- model-based command 序列：put、commit、CAS、rewind、fork、开启/关闭完整记录、Catalog
  Event、pin、unpin、prune、GC；
- 重复 put 同 Object ID/bytes 成功去重，同 ID/不同 bytes 报 corruption。

验收：随机命令序列后实现与 reference model 的可达对象、Ref revision、冲突结果一致。

## P2.3 SessionCoordinator

实现 Runtime 与 SaveStore 之间的唯一发布路径：

```text
checked Input
→ core TransitionDraft
→ store transaction
→ committed CommitId
→ CommittedRunResult
```

交付：

- draft 未提交前不对 host 可见；
- expected parent/ref revision 校验；
- safe point 强制检查；
- Input ID duplicate/conflict index；
- `Complete` 模式下 Commit、branch Ref、Catalog Event 与 Catalog Head 原子提交；
- transaction 失败保留 parent active cursor；
- `dispatch` 返回 typed store/runtime/conflict error。

## P2.4 Timeline cursor、rewind、redo 与 fork

交付：

- `TimelineSession { selected_branch, cursor }`；
- `TimelineRecordingMode::Standard/Complete` 与不可伪造的 `TimelineCoverage`；
- branch head Ref；
- rewind 只移动 cursor；
- redo 沿 selected branch 的 ancestor path；
- 从非 head cursor 继续自动创建新 branch Ref；
- 相同 transition 复用既有 Commit；
- branch 歧义返回候选，不任意选择；
- `Standard` 中 abandoned branch 使用可配置 lease，save/bookmark 长期 pin；
- `Complete` 从 baseline 起保留所有 branch/save/bookmark 目录事件及其 Commit closure；
- 开启 `Complete` 原子写 `RecordingStarted` 与当时 Ref 的当前值，不声称恢复此前目录操作；
- 关闭记录显式选择 `Seal` 或 `Delete`；再次开启创建新的 coverage segment。

CLI 增加 `timeline log/rewind/redo/fork/bookmark`，输出 Commit 短摘要和 turn，不暴露内部
对象路径。

## P2.5 用户 save/load API

交付：

- `Standard` 中 `save(slot, expected_revision)` 仅 CAS Ref；
- `Complete` 模式中 save/bookmark/branch 的 create/update/rename/delete 同时追加 Catalog Event；
- 非 safe point 返回 `SaveDeferred` 并在下一个 committed result 完成；
- `load(slot)` 执行完整 closure/hash/schema/Program/RuntimeState 验证；
- load 失败不移动 active cursor；
- 冲突包含 expected/actual revision 和双方 Commit；
- `Standard` 中 slot rename/copy/delete 只操作 Ref；`Complete` 中还保留目录事件；
- API 分开表达“从当前存档列表移除”与“prune/删除完整时间线数据”。

## P2.6 SqliteStore

在 `narrata-store-sqlite` 实现 native reference adapter。最小逻辑表：

```text
objects(id, kind, schema, payload)
object_edges(source, target, edge_kind)
refs(namespace, name, revision, commit_id, metadata)
catalog_heads(timeline, revision, event_id, coverage, metadata)
archive_refs(timeline, name, revision, manifest_id, metadata)
pins(owner, object_id, expires_at)
```

交付：

- object/edge/Commit/Ref/Catalog Head/Archive Ref CAS 在一个 SQLite transaction；
- schema migration 和 integrity check；
- 明确 journal/synchronous 配置及平台支持声明；
- 多进程/多连接 CAS 测试；
- busy/IO/full/corrupt 的 typed error；
- 不把 SQLite row ID 当 Narrata identity。

参考 SQLite 的原子提交机制，但以 fault test 证明本 adapter，而不是仅引用数据库承诺：
[SQLite Atomic Commit](https://www.sqlite.org/atomiccommit.html)。

## P2.7 Crash/fault injection matrix

在以下动作前后强制终止 transaction/process：

```text
Snapshot write
Receipt write
Commit write
edge index write
Ref CAS
Catalog Event write
Catalog Head CAS
Archive Ref CAS
transaction commit/fsync boundary
GC mark/sweep
bundle import Ref creation
```

每次重开 store 后必须满足：

- Ref 指向旧完整 Commit 或新完整 Commit；
- 不存在可见悬空 Ref；
- orphan 可以存在但 GC 可回收；
- integrity scan 能指出损坏 Object ID；
- retry 同一 operation 不重复 generation 或产生不同 Commit。

## P2.8 Checkpoint Bundle、Timeline Archive 与导入

交付：

- `CheckpointBundle` 与 `TimelineArchiveBundle` 使用不同判别 tag 和 checked manifest；
- deterministic manifest，ObjectDescriptor 按 ID 排序；
- Checkpoint 从单 root 枚举 closure，不宣称包含 sibling branch 或 Ref 历史；
- Timeline Archive 包含单一 Execution 的 coverage、多 root、Catalog chain、cursor、selected
  branch 和全部对象 closure；
- 账户级全量导出组合多个 bundle，不跨 Execution 合并 Commit DAG；
- incremental export 可排除接收方已有对象；
- streaming size/hash verification，验证完成前不建永久 Ref；
- 缺对象、重复 descriptor、kind/hash 置换、zip bomb/超限防护；
- Checkpoint import 最后以 expected revision CAS 目标 slot；
- Timeline import 验证全部 archive-local 名称映射后，在一个事务中创建 archive root、Catalog
  Head 和映射后的 Ref；
- round-trip native/MemoryStore fixture。

## P2.9 GC 与 retention

交付：

- root：Ref、active cursor、bookmark、Catalog Head、Timeline Archive、pin/lease；
- closure：parent、Snapshot、Receipt、Program/Content Lock、Catalog Event、archive manifest；
- grace period；
- “older than N days”保留阈值时刻存在的最近 predecessor；
- dry-run `GcReport`；
- GC 与 commit transaction/temporary pin 同步；
- 删除 material 时报告是否仍有其他 root，不能声称已经物理清除；
- Catalog tombstone 保留旧 save Commit；永久清除只能通过显式 prune 或删除 archive root；
- `Standard` 不保证 lease/grace 之外可构造完整历史，`Complete` 不得回收 coverage 内对象。

## P2.10 性能门槛

建立可复现 benchmark，而不是先做 delta：

- 1K/10K/100K variables；
- call depth 与 scene size；
- 1K/10K commits 的 load、rewind、GC；
- 1K/10K Catalog Event、多 branch/save root 的 archive export/import；
- Snapshot bytes、encode/decode、SQLite transaction latency；
- native 与 Wasm 内存峰值。

先记录产品目标再设置阈值。若全量 Snapshot 不达标，按测量结果拆分最大的 immutable
subtree；Commit/Ref protocol 和 conformance hash 版本化升级，不引入无界 delta chain。

## Phase 2 exit gate

- 两个进程/设备模拟器同时写 slot 时，CAS 无静默丢失；
- crash matrix 全部保持 Ref 原子性；
- 任意仍被 policy 保留的 Commit 可 load、rewind、fork；
- Checkpoint/Timeline bundle 导入不可信数据无 panic/部分 Ref；
- Timeline Archive 对 branch/save/bookmark、Catalog chain、coverage 和 session view 完整 round-trip；
- `Standard` 不被误报为完整时间线，启用后的 coverage 边界不可伪造；
- save tombstone 与 archive 永久删除具有不同且经过测试的保留结果；
- GC 的实现与模型对随机历史一致；
- Program Artifact closure 被 save root 保留；
- 完整 Snapshot 性能数据和是否需要结构共享的决定已记录。
