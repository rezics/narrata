# Phase 1：最小确定性 Runtime

状态：已实施（Stage 1 / G1）
前置：[Phase 0](./00-contracts-and-test-oracles.md)
完成后解锁：[Phase 2](./02-time-travel-and-persistence.md)

## 目标

在内存中执行一个没有 parser、随机数、真实时间和 Statechart 的短篇 Program。每次交互都停
在完整可编码的 continuation，能 snapshot/restore 后继续得到同一结果。

## v0 指令集

```text
Const / Load / Store / Unary / Binary
Jump / JumpIfFalse
Call / Return
Say / Choice
Finish
```

`Say` 与 `Choice` 生成 typed pending interaction；不直接调用 UI。表达式先限制为 Value 常量、
变量读取、相等/布尔/整数基本运算。除零、溢出和类型不匹配有确定诊断。

`And / Or` 由未来 compiler 展开为 `JumpIfFalse` 以保留 short-circuit。所有互动 safe point 的
evaluation stack 必须为空。

## P1.1 Typed Program IR 与 validator

交付：

- `ProgramArtifactV0`、Flow/instruction table、constant pool、capability manifest 空骨架；
- 所有可保存 continuation 和 return target 使用 Stable `InstructionId`；
- validator 检查 ID 唯一、target、operand、call target、choice target 和终结路径；
- 测试直接构造/加载 IR fixture，不先实现用户 DSL；
- `ProgramArtifactId` 从 canonical runtime-relevant artifact 计算。

验收：悬空 jump/call、重复 ID、错误 operand 和无效 Choice 在 Runtime 创建前失败。

## P1.2 VM State

交付：

```text
instruction pointer
call frames with stable return target and locals
evaluation stack
global/flow value stores
RuntimeStatus
turn / logical time placeholder
ExecutionId
```

- frame、stack、locals 有独立上限；
- `RuntimeStatus` 联合类型保证 `Ready` 无 pending、`Awaiting` 恰好一个 pending；
- 无 Rust call stack 递归执行 Flow；`Call` 只写显式 frame；
- 每次 snapshot 都能完整 encode/decode。

## P1.3 纯 reducer 与事务 working state

实现：

```text
reduce(program, committed_state, checked_input, limits)
  -> TransitionDraft | typed error
```

交付：

- 只在与 `RuntimeStatus` 匹配时接受 Input；
- macrostep 在隔离 working state 上运行；
- `AwaitInteraction/Stable/Finished` 才返回可提交 draft；
- Fault/total limit 丢弃 working state；
- slice pause 对宿主不可观察，只能 `continue_slice`；
- ordered `TransitionReceiptV0`。

验收：在每条指令注入 Fault，parent state bytes/hash 均不变。

## P1.4 Interaction 协议

交付：

- `SayView { interaction_id, speaker, content_ref/text, tags }`；
- `ChoiceView { interaction_id, choices: Vec<ChoiceViewItem> }`；
- `Advance`、`Select` checked Input；
- Interaction ID 包含 Execution、parent、instruction 和 occurrence，retry 稳定；
- 过期/错误 interaction、隐藏 choice 和重复 Input ID 的 typed error；
- 当前 interaction 完整进入 Snapshot。

v0 可以内嵌文本 fixture；外部 REZICS content resolver 留到 Effect/Content Lock 集成。

## P1.5 In-memory snapshot/restore

使用 Phase 0 codec 提供：

- `export_snapshot` 仅接受 safe-point Runtime State；
- `restore_snapshot` 执行 wire parse、limit、Program/Stable ID 和状态组合验证；
- clone/fork 使用新的 session cursor，但保留 `ExecutionId`；
- “复制为新 Execution”是显式 API，并重新生成 interaction/effect scope。

验收：在每个 `Say/Choice/Finish` 前后 snapshot/restore，后续 trace 与未中断执行一致。

## P1.6 Budget、循环与可观测性

交付：

- instruction slice budget；
- macrostep instruction/call/allocation hard limit；
- CLI `run --trace` 输出 instruction ID、status、state/effect hash，不输出敏感 Value 默认值；
- 多个 slice size 的 conformance matrix；
- 无限 Jump fixture 在 hard limit 确定失败。

## P1.7 Property 与 fuzz tests

至少验证：

```text
same program + state + input => same draft/receipt
restore(snapshot(state)) == state
run(restore(snapshot(S)), I) == run(S, I)
different slice size => same final draft
invalid input => unchanged state
```

生成小型合法/非法 Program、输入序列、snapshot boundary 和 stack depth。Fuzz target 覆盖 IR
decoder、Snapshot decoder、validator 和单 macrostep；设置超时/内存上限。

## Phase 1 exit gate

- CLI 可运行至少两个含 call/branch/choice 的 fixture；
- 所有 interaction safe point 可保存并恢复；
- core 无文件、网络、系统 RNG、wall clock、callback 或 async continuation；
- 同一 conformance trace 在 native/Wasm 和多种 slice budget 下 hash 一致；
- 所有负例保持 committed state 不变；
- parser/editor/Statechart 没有反向影响已冻结的 reducer contract。
