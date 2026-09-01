# Phase 3：Effect 与宿主协调

状态：待实施
前置：[Phase 2](./02-time-travel-and-persistence.md)
完成后解锁：有外部内容/游戏状态的生产集成、[Phase 4](./04-statecharts.md)

## 目标

在不破坏确定性和时间旅行的前提下接入呈现、外部查询与宿主写操作。建立
commit-before-dispatch、单调 Effect ledger、rewind barrier 和 Federated Save。

## P3.1 Capability schema 与 negotiation

交付：

- `CapabilityId`/version newtype；
- request/response schema 与允许的 delivery/rewind policy；
- Program load 时 host capability negotiation；
- required/optional capability 和版本不兼容的 typed diagnostic；
- schema 从单一权威定义生成 binding，或有 drift contract test；
- 故事代码不能覆盖 host 声明的安全 policy。

先内建：

```text
visual.dialogue@1
visual.choice@1
visual.scene@1
host.query@1
host.command@1
```

## P3.2 Effect ID 与 pending state

交付：

- `EffectRequestDigest` 覆盖 capability/version、payload digest、delivery 与 rewind policy；
- ID hash 包含 Execution、parent、Input、Instruction、ordinal 与完整 request digest；
- Runtime v1 每次最多一个 pending Effect；
- pending Effect 完整进入 safe-point Snapshot；
- Effect Response 校验 ID、完整 request digest、capability/version 和 response schema；
- 两个 Execution 的相同 trace 产生不同 Effect ID；
- retry 同一 transition 产生相同 ID。

## P3.3 Declarative SceneState

交付：

- scene/layer/actor/camera/audio/interaction checked types；
- `ReconcileScene(target)`，加载/rewind 不重放命令序列；
- adapter capability 声明 animation/audio 是 Restart、Seek 或 BestEffort；
- scene 差异算法只做优化，完整 target 是权威；
- presentation 结果不能写剧情变量，除非作为新的 checked Input。

## P3.4 Effect Store 与 ledger

在 store transaction 模型中增加：

```text
claim / renew lease
complete / reject / retryable failure / unknown outcome
request-digest conflict detection
monotonic LedgerFence per Execution
recorded response object
```

交付：

- Ledger 不受 branch rewind/delete/GC 影响；
- `(ExecutionId, EffectId)` uniqueness；
- completed response 重复读取不 dispatch；
- lease 到期与并发 claimant 模型测试；
- ledger response/pending Commit 作为 GC root；
- telemetry 与业务 response 分离。

## P3.5 Commit-before-dispatch coordinator

交付：

- 只有 `CommittedRunResult::AwaitEffect` 可交给 host；
- dispatch 前查/claim ledger；
- recorded query/idempotent command/host transaction 分别实现策略；
- response 先写 ledger，再 resume Runtime；
- crash 后从 pending Commit 恢复并复用 response；
- ledger 已完成但 response Commit 缺失时自动 catch-up，期间不发布旧状态或接受其他 Input；
- `UnknownOutcome` 停止自动推进，要求 host 查询或人工决策。

测试 host 在每个步骤重复、延迟、乱序 response；v1 对非当前 pending ID 全部拒绝。

## P3.6 Rewind barrier 与 compensation

交付：

- Commit 记录 observed LedgerFence；
- completed Barrier 分配新 fence；
- load/rewind 在移动 cursor 前检查目标 fence；
- `BlockedByExternalBarrier` 包含 capability/effect/target，不泄露 payload；
- simulation/read-only 模式绝不 dispatch external Effect；
- compensation 使用新 Effect ID，ledger 保留原记录并连接关系；
- 非幂等且不可查询的 command 在 persistent mode 默认拒绝注册。

## P3.7 Recorded Query 与外部内容 resolver

交付：

- query response 成为 checked Input/Receipt；
- replay/load 同一 pending query 复用 response；
- REZICS resolver 验证 provider、structure occurrence、权限、lifecycle、revision policy；
- 影响分支的内容固定/记录；`LivePresentationOnly` 不进入 guard/effect；
- not-found/forbidden/retired/incompatible 分开建模。

## P3.8 Federated Save

交付：

- `HostSnapshotRef` 与 `CompoundSaveManifest`；
- immutable host snapshot digest；
- Narrata Commit、Host Snapshot、Program、Content Lock、LedgerFence 联合验证；
- orphan host/narrative object retention；
- 任一半缺失或版本不兼容不移动 save Ref；
- narrative-authoritative 模式无需 Host Snapshot。

## P3.9 Effect fault matrix

覆盖：

```text
pending Commit 前/后
claim 前/后
外部调用前/后
ledger outcome 前/后
Runtime resume 前/后
next Commit 前/后
```

对支持 idempotency 的测试系统，业务结果最多一次；对故意不支持的系统，必须稳定进入
`UnknownOutcome`，不能宣称成功或自动 retry。

## Phase 3 exit gate

- callback/reentrancy 不进入 core；
- Effect 一定在 pending Commit 后发布；
- ledger response 可以跨 crash 恢复；
- Barrier 在 load/rewind 前阻止旧 Commit；
- Scene 可由单个 Snapshot reconcile；
- Recorded Query 重放不访问外部系统；
- Federated Save 原子验证两侧；
- exactly-once 限制在 API 与文档中明确，没有 catch-all `ExternalCommit` 误导。
