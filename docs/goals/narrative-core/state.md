# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | manager `narrata-narrative-core` 已启动。[ADR 0013](../../adr/0013-r2-text-free-node-format.md)：R2 不含正文的格式、身份、选择模型、块与 kernel 提交对象。R1 冻结语料 `fixtures/compat/nodes-r1/` 已建。 |
| Running | G-005（claude）：R2 核心、工具、示例作品与阅读器适配。G-006（codex）：`narrata-graph` 分析、布局与瓦片。 |
| Next | G-005 合并后：内容大纲诊断、动态提议、合成作品基准与按块懒加载、`narrata-graph` 接入 R2（CLI 与 Wasm）、语义摘要导出。最后收缩契约页。 |
| Decisions | 节点对象用规范 CBOR，kind code 为 `0x0100–0x01FF`。恢复不重放，`verify_path` 可选。`ContentRef` 放在 `narrata-kernel`。阅读器由本 Goal 一次性认领：web-and-rezics 尚未启动，维护者把决定交给 manager。 |
| Coupling | kernel-and-saves：commit API 草案已接受（提交 `{artifact, parent, input, state, depth}`）。对方同意本 Goal 改 `narrata-kernel/src/content/**`，并建 `fixtures/compat/nodes-r1`、`nodes-r2`。节点会话接入 kernel 存储要等对方的 node-session API。 |
