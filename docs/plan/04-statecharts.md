# Phase 4：Statechart

状态：待实施
前置：[Phase 3](./03-effects-and-host-coordination.md)
完成后解锁：复杂任务/模式、并行场景与完整 migration corpus

## 目标

在已经可保存、可重放的 reducer 中加入 Narrata Statechart 子集。采用 SCXML 的
run-to-completion、事件队列与层级语义，不实现 XML、任意 script 或平台 I/O processor。

## P4.1 先写可执行语义 fixture

在实现前固定：

- active configuration 表示；
- atomic/compound/parallel/final；
- transition domain、exit/entry order；
- internal/external/self transition；
- eventless transition；
- guard 纯度；
- conflict priority/tie-break；
- internal queue FIFO；
- completion event；
- shallow/deep history；
- Flow action 与 Effect yield 的关系；
- microstep/macrostep budget。

每一规则至少有正例、冲突例和 expected ordered trace。文档规则与 fixture 使用同一稳定
规则编号。

## P4.2 Typed Statechart IR 与 compiler validation

交付：

- `StateId/RegionId/TransitionId/EventTypeId/HistoryId`；
- immutable parent/child/region index；
- guard/action typed IR，只能读写 Runtime Value/raise event/start Flow/emit Effect；
- target legality、initial/final/history 约束；
- 静态可检测的 transition conflict、无出口、不可达和 eventless cycle diagnostic；
- source span 保留到所有 definition。

## P4.3 Atomic + compound kernel（G4a）

交付：

- active leaf configuration；
- ancestor event matching；
- fixed exit/action/entry order；
- eventless/internal queue drain；
- done/completion event；
- run-to-completion 与 Phase 1 working transaction 共用；
- `StatechartState` 进入 Snapshot/Commit/Receipt。

验收：SCXML 语义 fixture 的 Narrata 子集全部通过；保存只发生在完整 macrostep 或 Await。

## P4.4 Flow VM 集成

明确而非隐式决定：

- entry/exit action 启动同步短 Flow 还是 invoked Flow；
- Flow raise event 何时进入 internal queue；
- Flow Effect intent 怎样延迟到稳定 active configuration 后发布；需要 response 的流程怎样
  降低为显式 awaiting state，并让 response 成为下一次 external Input；
- state exit 怎样取消 invoked Flow；
- 一个 macrostep 中 Value 写入对后续 guard 的可见顺序；
- Fault 时整个 macrostep 回退。

交付跨 Flow/Statechart trace：entry → call Flow → Await → response → internal event → transition。

## P4.5 Parallel regions（G4b）

交付：

- active configuration 使用有序集合，不依 hash iteration；
- 每个 region 独立候选，统一 conflict resolution；
- exit descendants、transition actions、entry ancestors 的全局固定顺序；
- 同 macrostep 写同一 Value 的冲突政策：compiler reject 或显式 reducer，禁止 last writer wins；
- 多 region Effect v1 仍串行化；batch 等 Phase 3 所列条件满足后再开放；
- 所有 event interleaving 的 model tests。

## P4.6 History state

交付：

- shallow/deep history 明确保存的 active descendant 集；
- default history transition；
- Snapshot 编码与 Stable ID migration；
- target 被删除时只接受显式 relocation/recovery；
- parallel region history 的 golden trace。

## P4.7 Invocation、deferred event（可延后子 Gate）

只有实际用例证明需要时增加：

- child machine invocation/cancellation；
- deferred event；
- completion propagation；
- child Snapshot/Effect ownership。

它们不是 G4a/G4b 的前置，不能拖延基本 Statechart 发布。

## P4.8 Model/property tests

生成小型合法 state graph 与事件序列，比较 optimized kernel 和简单 reference interpreter：

- 每个 microstep 的 enabled transition；
- ordered exit/action/entry；
- active configuration/history；
- internal events；
- final Snapshot/Receipt/Effects。

还要生成 eventless loop、冲突 transition、parallel write conflict 和 budget exhaustion 负例。

## Phase 4 exit gate

- atomic/compound 与 parallel/history 分别有独立 Gate；
- 相同 state/input 在 native/Wasm、不同 map order 和 slice budget 下 trace 相同；
- macrostep 中间不可保存或接受下一个 external Input；
- parallel 不引入 response arrival 或 hash iteration 非确定性；
- Flow/Statechart Fault 原子回到 parent Commit；
- Statechart Snapshot 可以 exact restore、rewind 和 fork。

规范基准：[W3C SCXML](https://www.w3.org/TR/scxml/)。Narrata 只声明已实现的子集，不使用
“兼容 SCXML”描述未实现的 XML/data model/I/O processor。
