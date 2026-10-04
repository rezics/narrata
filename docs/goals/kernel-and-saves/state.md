# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | Goal 已定义；存储与存档契约草案；manager `narrata-kernel-and-saves` 已登记。 |
| Running | G-001 基线基准（codex）；G-002 存储后端契约 + 内存/按行 SQLite 后端 + 一致性套件，ADR 0012（claude xhigh）；G-003 身份/摘要/规范 CBOR 抽取为 `narrata-kernel`（codex）。 |
| Next | G-002 合并后：引擎任务——把 `narrata-store` 的领域逻辑（账本、目录、复合存档、pin、GC、完整性、bundle 闭包）改写在契约之上，定义键布局 ADR，加载不再走到创世提交（计数包装器断言），`model.rs`/`faults_gc.rs` 改为对任意后端运行，旧 SQLite schema v2 的读取或迁移路径。之后：协议引擎存储可注入与宿主批次协议、IndexedDB、旧栈去文本 ADR（等 `narrative-core` 的内容引用协议）、Flow/Statechart 成为 kernel 上的领域包、节点会话经 kernel 存档。 |
| Decisions | 存储契约与 kernel 放在 `packages/narrata/kernel/crates/`，不依赖 `narrata-core`；契约是同步 trait，异步宿主走"引擎产出批次、宿主落盘确认"。ADR 0012 归本 Goal，0013 归 `narrative-core`（节点 R2 格式）。 |
| Coupling | `narrative-core` 将让 `narrata-nodes` 依赖 `narrata-kernel` 的摘要分帧与规范 CBOR（G-003 合并后通知对方）；节点 R2 State 有受检规范解码，提交逻辑内容为 (artifact, parent, input, state)，等本 Goal 提出提交 API；对方将新增 `fixtures/compat/nodes-r1/`（已同意）。`web-and-rezics` 依赖 IndexedDB 后端与存档字节。 |
