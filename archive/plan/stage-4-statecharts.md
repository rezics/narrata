# Stage 4：Deterministic Statecharts

状态：已实施（G4）
日期：2026-09-02
格式稳定性：Statechart Program/Snapshot/Receipt 扩展与 Rust API 为 `0.x` unstable

## 已交付能力

Stage 4 在既有纯 reducer、Commit 和 Effect 协议内加入 typed Statechart，而没有引入 XML、脚本、
callback 或宿主 I/O。Program 可选携带 Statechart；不携带时，Stage 1–3 的 canonical Program、
Snapshot 和 Receipt bytes 保持不变。

- `StateId`、`RegionId`、`TransitionId`、`EventTypeId`、`HistoryId`、`ActionId` 是互不混用的
  strong ID；IR 覆盖 atomic、compound、parallel、final、shallow/deep history、guard、entry/exit/
  transition action 与 source span。
- loader 建立 immutable index，并验证父子/region/initial/final/history/target、guard/action 类型、
  引用、不可达和无出口 atomic state、静态冲突、并行写冲突及无条件 eventless cycle。
- kernel 使用有序 active-leaf configuration、FIFO internal queue、run-to-completion、completion
  event、history 和确定性的 microstep/internal-event/runtime-state hard limits。
- `StartFlow` 可运行完整 Flow VM，包括 Say/Choice/Effect；Flow raise 的事件在 Flow 完成后按 FIFO
  交回 Statechart。`EmitEffect` 与 Flow Effect 都只在一致 configuration 上 Await，并继续遵守
  commit-before-dispatch。
- Snapshot 精确保存 active/history/completed/internal/deferred work/invocation；restore 重新验证
  configuration coverage、history scope、completion、invocation/action、Effect identity 和所有 limits。
- Commit/rewind/replay、MemoryStore、SQLite reopen、CLI status/event/trace 与 Receipt metrics 已贯通。

## 稳定语义规则

fixture 与实现使用以下规则作为 G4 基线：

| 规则 | 决定 |
| --- | --- |
| SC-R01 | active configuration 只保存 atomic/final leaf，以 `StateId` 有序集合编码；每个已进入 region 恰有一个 active direct child，child 为 parallel 时可展开为多个 leaf。 |
| SC-R02 | 一个 external Event 启动一个 macrostep；eventless transition 先于 queued internal event，二者排空后才形成 Stable。 |
| SC-R03 | 每个 active leaf 从最深 state 向祖先寻找第一个 enabled transition；同 source 按 Program 中的 transition 顺序取第一个。 |
| SC-R04 | 候选按 source depth 降序、Program 顺序升序；exit set 相交者冲突，先者胜出；静态无条件歧义直接拒绝。 |
| SC-R05 | 一个 microstep 先按 depth 降序/`StateId` 升序 exit，再按 transition 顺序执行 actions，最后按 canonical region/`StateId` 顺序 entry。 |
| SC-R06 | targetless internal transition 不 exit/entry；external self transition exit 并重新 entry source，但不重新进入未退出祖先。 |
| SC-R07 | action 写入立即对后续 guard 可见；并行 transition 写同一 global 在 load-time 或 runtime 拒绝，不使用 last-writer-wins。 |
| SC-R08 | Raise 和 completion event 进入 FIFO internal queue；无条件 eventless cycle 在 load-time 拒绝，动态循环由 hard limit 终止。 |
| SC-R09 | shallow history 保存 region 的直接 child；deep history保存 active leaf；无记录时使用 checked default targets。 |
| SC-R10 | entry/exit 中的 Flow/Effect 先进入 deterministic deferred-work queue；若 owner 在发布前退出则取消，否则按 canonical action 顺序一次发布一个。 |
| SC-R11 | invoked Flow 独占当前 continuation；其 Await 是 safe point。Flow 内 Raise 暂存到 Flow 完成，再交回 Statechart；运行中的 Flow 不并发接受 Statechart Event。 |
| SC-R12 | 任一 Flow/Statechart Fault 或 hard-limit exhaustion 丢弃整个 working state；slice budget 只影响让出时机，不影响 trace/state/receipt。 |

这里采用 SCXML 的 run-to-completion 核心概念，但 Narrata 的 conflict tie-break、typed value model、
Effect 协议和 wire format都是自己的契约，不声称实现完整 SCXML。

## Wire 与持久化

Statechart 是 Program/Snapshot 的尾部可选 canonical CBOR 字段。旧 map 长度仍由 decoder 接受且旧
golden bytes 不变；新字段存在时使用严格 key order、array bounds、value-node bounds 和 round-trip
canonical check。`EffectPathV0` 显式区分 Flow instruction continuation 与 Statechart action
continuation，因此 restore 不靠 cast 或约定猜测 pending Effect 类型。

Statechart Stable Receipt 增加独立 result tag，并仅在 Statechart 执行时编码 microstep/internal-event
metrics。Stage 4 Program、Awaiting Snapshot 和 Receipt 的 SHA-256 golden 在
`fixtures/codec/statechart-v0.sha256`；完整执行语义 fixture 在
`fixtures/statechart/parallel-history-v0.json`。

## 可执行验证

完整 G4 Gate：

```powershell
./scripts/check-g4.ps1
```

Gate 递归执行 G1–G3，再覆盖 eventless、internal/external/self、层级优先级、并行 interleaving 与
Effect 串行、shallow/deep history、completion、Flow Await/Effect/raise、entry-work cancellation、
slice invariance、microstep 原子回滚、Program/Snapshot/Receipt golden、96-case reference-model
property test、commit-before-dispatch、rewind/replay、SQLite reopen，以及 Stage 4 decoder fuzz build。

## 明确限制

- P4.7 标为可延后的 child-machine invocation、child Snapshot/Effect ownership 与 external deferred
  event 未实现；当前 invocation 是 Statechart action 启动同一 Program 内的 Flow。
- 同时只能有一个权威 pending Effect；并行 region 产生多个 Effect 时按 canonical 顺序串行。
- guard 仅支持纯 typed expression；无 XML、ECMAScript、任意宿主 callback、时间或随机读取。
- Statechart wire/API 仍为 `0.x`；跨版本 relocation/migration 属于 Stage 5，不能把缺失 Stable ID
  静默映射到别的 state。
