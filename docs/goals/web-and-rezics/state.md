# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | 依赖的两个 Goal 已关闭。维护者要求将第三个 Goal 拆为本机 Web 宿主 SDK 和另一台机器的 REZICS 接入；交接见 `docs/integrations/rezics-goal.md`。 |
| Running | 无。 |
| Next | 依赖已就绪：R2 节点格式与内容引用（ADR 0013）、`narrata-nodes-wasm` 的仅清单打开与缺块重试、Wasm 发布分析（瓦片、标签表、摘要，ADR 0016）、本地内容方、kernel 历史层与 IndexedDB 宿主缓存（ADR 0015、0017）。`narrative-core` 曾一次性认领阅读器做 R2 与持久化适配；阅读器外壳、渲染插槽、模拟 REZICS 内容方与网络懒加载归本 Goal。 |
| Coupling | 本机只实施 Narrata，用模拟宿主与打包后的 npm 交付物验收。另一台机器的 REZICS manager 负责真实适配器和网站，最后一个任务用固定发行版本联合验证；版本、提交与包校验和随交付记录。公开发布仍需维护者授权。 |
