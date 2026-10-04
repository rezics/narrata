# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | 基线基准（`docs/development/benchmarks/storage.md`：MemoryStore 10K 提交 124 ms/次，SQLite 1K 打开 20 s）；ADR 0012 与 `narrata-storage`（契约、内存与按行 SQLite 后端、一致性套件、故障注入与计数包装器）；`narrata-kernel`（摘要分帧、公开的规范 CBOR、受检解码、u16 种类码封套、ID 宏），stage5 字节不变。 |
| Running | G-004 存档引擎：`narrata-store` 的领域逻辑在契约之上只实现一次，键布局 ADR 0014，加载不走到创世提交，v2 SQLite 迁移与两份新冻结语料。 |
| Next | G-004 之后：宿主存储协议（引擎产出批次、宿主落盘确认）+ 协议引擎存储可注入 + IndexedDB 后端；旧栈去文本 ADR（等 `narrative-core` 的 ADR 0013 与 ContentRef 放置）；kernel 的领域注册（Flow/Statechart/节点种类码与对象图接缝）；节点会话提交 API 提案给 `narrative-core`；"浅存档"是否需要（10K 深度 bundle 约 11 MB）。 |
| Decisions | 存储契约与 kernel 在 `packages/narrata/kernel/crates/`，不依赖 `narrata-core`；同步 trait，异步宿主走批次确认协议；修订号全库单调。ADR 0012、0014 归本 Goal，0013 归 `narrative-core`。 |
| Coupling | `narrative-core` 让 `narrata-nodes` 依赖 `narrata-kernel`（已通知）；已提议 ContentRef 放进 `narrata-kernel`，等对方答复；对方新增 `fixtures/compat/nodes-r1/`（已同意）。`web-and-rezics` 依赖 IndexedDB 后端与存档字节。 |
