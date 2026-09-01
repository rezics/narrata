# ADR 0007：Stage 3 Effect 与宿主状态边界

状态：Accepted

## 决策

Core 继续是无 I/O、无 callback 的纯状态转换器。Program 在加载 session 时与宿主提供的
capability/version/schema/delivery/rewind policy 做协商；故事不能放宽宿主安全策略。Runtime v1
一次至多持有一个完整 `PendingEffectV0`，并把它写入 safe-point Snapshot。

Effect 只可由带持久 `CommitId` 的 `CommittedRunResult::AwaitEffect` 发布。`EffectId` 在调用宿主前
由 Execution、parent Commit、Input digest、Instruction、occurrence 和完整 request digest 决定。
Response 必须先以 checked object 写入单调 ledger，再作为 checked Runtime Input 恢复执行。

Ledger 以 `(ExecutionId, EffectId)` 唯一索引，不进入可回滚 Snapshot，也不受 branch、Ref 或 GC
回滚。terminal outcome 分配单调 `LedgerFence`。未补偿 Barrier 的 fence 在移动 cursor/Ref 前阻止
恢复更旧 Commit；只读 simulation 可以检查旧状态，但没有 dispatch API。

宿主状态影响分支时，只有 Narrata Commit、不可变 Host Snapshot、Program、Content Lock 与 fence
共同验证后才能 CAS 发布 `CompoundSaveRef`。完整 federated timeline 必须为每个宣称可继续执行的
Narrative Commit 提供已验证的 Host Snapshot 映射；缺失映射只能查看 narrative-only 状态。

## 后果

- Narrata 不承诺通用 exactly-once。保证上限是 stable Effect ID、at-least-once/transactional host
  contract、durable response 与显式 `UnknownOutcome`；未知结果不会自动 retry 或宣称成功。
- Recorded Query 重放只读 ledger；presentation-only resolver 结果的类型不暴露 branching value。
- Declarative `SceneState` 是呈现真相；加载/rewind 输出完整 `ReconcileScene` target，不重放命令史。
- compensation 是新的 Effect，并在 ledger 中保留原 response/fence 与 compensator 关系。
- Stage 3 wire、SQLite schema v2 和 Rust API 仍是 `0.x` 实验契约，后续破坏性变更必须配 migration。
