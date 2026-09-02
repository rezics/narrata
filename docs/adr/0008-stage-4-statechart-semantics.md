# ADR 0008：Stage 4 Statechart 语义与 Flow 边界

状态：Accepted

## 决策

Statechart 作为 `ProgramArtifactV0` 和 `RuntimeStateV0` 的可选 typed extension 实现，而不是新增
独立 runtime 或 XML interpreter。它与 Flow 共用同一份 transactional working state、Input、
Snapshot、Receipt、Commit 和 hard limits。旧 artifact 没有 Statechart 字段时保持 Stage 1–3 bytes
和行为。

active configuration 只持久化有序 leaf set；loader 的 immutable index恢复祖先关系。transition
选择采用 deepest-source priority、Program order tie-break 和 exit-set conflict；exit/action/entry 的
全局顺序固定并进入 trace。eventless work 优先于 FIFO internal event。并行写冲突拒绝，Effect 按
canonical action 顺序串行，不依赖 hash iteration 或 response arrival order。

Flow invocation 是显式 deferred work：configuration 先稳定，仍 active 的 owner 才启动 Flow。Flow
Await 是完整 safe point；Flow raise 在 Flow 完成后进入 Statechart，避免在半个 VM continuation 中
重入 Statechart。Statechart/Flow Effect 都复用 Stage 3 commit-before-dispatch ledger，并通过 typed
Effect path 保存各自 continuation。

## 后果

- Statechart macrostep 与 Flow 指令共享原子 rollback；slice yield 永远不是可保存状态。
- Snapshot restore 必须重新证明 active-region coverage、history/completion、invocation/action、Effect
  identity 与 limits，不能只信任 decoder shape。
- child-machine invocation、运行中 child cancellation 和 external deferred-event policy 不属于 G4；
  它们若进入后续版本必须新增 ownership、Snapshot 与 Effect 规则。
- Narrata 只声明 W3C SCXML run-to-completion 概念的明确子集，不声明 XML/data model/I/O processor
  兼容性。
