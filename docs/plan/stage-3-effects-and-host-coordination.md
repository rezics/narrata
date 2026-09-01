# Stage 3：Effects and Host Coordination

状态：已实施（G3）
日期：2026-09-01
格式稳定性：Stage 3 object/schema 与 Rust API 为 `0.x` unstable

## 已交付能力

`narrata-core` 提供 checked capability contract、五个内建 capability、完整 request/response schema、
execution-scoped Effect ID、单 pending Effect Snapshot、Declarative `SceneState`/`ReconcileScene`，以及
将 branching external content 与 `LivePresentationOnly` 分开的 resolver 结果类型。

`narrata-store` 提供：

- Program load/open capability negotiation；
- pending Commit 成功后才发布的 `CommittedRunResult::AwaitEffect`；
- claim/renew、request conflict、completed/rejected/retryable/unknown outcome、recorded response object；
- 每个 Execution 单调 `LedgerFence`，ledger origin/response GC roots，以及不删除原事实的 compensation；
- response-commit 崩溃窗口的 deterministic catch-up，期间拒绝其他 Input；
- 在 cursor/Ref 移动前检查的 Barrier 与无 dispatch surface 的 simulation inspection；
- checked `HostSnapshotRef`、Compound Save CAS、Host Timeline 映射和 narrative-only availability。

`narrata-store-sqlite` 使用 schema v2 保存 ledger fence、Effect materialized view、recorded response、
compensation relation 和 Compound Save Ref。claim/outcome/compensation 都在 `BEGIN IMMEDIATE` 中应用与
`MemoryStore` 相同的 checked transaction。

## 宿主协议

```text
dispatch checked Input
→ atomically persist Snapshot + Receipt + pending Commit
→ receive committed AwaitEffect
→ claim ledger lease
→ invoke host using EffectId as idempotency key
→ record completed/rejected/retryable/unknown outcome
→ recover/resume from recorded response
→ atomically persist next Commit
```

只有 `Completed`/`Rejected` 的 recorded response 可以推进 Runtime。`UnknownOutcome` 是 terminal stop，
必须由宿主查询外部事务或交给 operator 决策；Narrata 不会自动重试。过期 lease 不能续租，其他
claimant 只能在 expiry 后以同一 Effect ID 接管。

## Delivery 与 rewind 边界

| Delivery | 允许的恢复语义 |
| --- | --- |
| `Reconcile` | 从完整 declarative target 重建，可重复 |
| `RecordedQuery` | 首次 response 持久化；回放不访问 adapter |
| `AtLeastOnceIdempotent` | 宿主按 Effect ID 去重；Narrata 可重复投递 |
| `HostTransactional` | 宿主事务/查询 API 决定最终 outcome |

`Barrier` 完成后阻止恢复到较低 fence；`Compensatable` 只有在新的补偿 Effect 完成且 capability 匹配
后才标记原记录。两种策略都不撤销或删除外部事实。

## 可执行验证

完整 G3 Gate：

```powershell
./scripts/check-g3.ps1
```

Gate 覆盖 capability drift/duplicate、Effect ID golden 与 execution isolation、pending Snapshot、
commit-before-dispatch、lease expiry/concurrency model、request conflict、crash catch-up、重复 query、
unknown stop、Barrier 原子检查、compensation、ledger GC roots、Scene 单 Snapshot reconcile、Compound Save
验证/CAS、Host Timeline mapping、SQLite reopen/双连接竞争、故障注入，以及 Stage 3 object fuzz build。

## 明确限制

- v1 一次只有一个权威 pending Effect；乱序/非当前 response 一律拒绝；
- 没有幂等键、事务查询或人工 unknown-resolution 流程的不可逆 command 不应注册为可自动重试；
- Host Snapshot 的 bytes 由宿主拥有，Narrata 只保存 checked opaque ref、format version 与 digest；
- Program v0 不内嵌 external-content declaration wire；外部内容通过 negotiated recorded-query Effect 和
  checked resolver boundary 接入；
- callback/reentrancy、并发 Effect batch、Statechart、跨语言 binding 与 migration 仍在后续 Stage。
