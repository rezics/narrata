# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | Goal 已定义；存储与存档契约草案。 |
| Running | 无。 |
| Next | 维护者启动 manager 后：先补基准（SQLite 1K/10K 提交、长历史加载），再派发存储契约与一致性套件、按行 SQLite，之后是 kernel 抽取与旧栈格式 ADR。 |
| Coupling | 与 `narrative-core` 约定节点会话的提交 API；`web-and-rezics` 依赖 IndexedDB 后端与存档字节。 |
