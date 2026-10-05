# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-06） |
| --- | --- |
| Done | 维护者讨论确定第一阶段范围（决定 15–17、[目标·以后](../../product/goal.md#以后)）。ADR 0021–0023 已接受，能力请求 R1/R4/R6/R9/R12 改写、新增 R13。已合并：CI 三平台全绿并加实库 PostgreSQL job（G-019、G-028 修 SQLite 并发打开）；`@rezics/narrata` 包、类型化运行时、tarball 安装测试与预算（G-023）；共享只读执行轨迹（G-025）；存档宿主一致性套件与 put-if-absent（G-026）；解析上下文、下一步预取与 `@rezics/narrata/testing` 模拟 REZICS 内容方（G-027）；PostgreSQL 参考宿主（G-029，CI 实库 46 项通过）；阅读器控制器与 `@rezics/narrata/react` 渲染插槽（G-030）。 |
| Running | G-024 创作核心（ADR 0022 §1–2）；G-031 作者图视图。 |
| Next | G-024 合并后：读者 Wasm 瘦身（现 gzip 约 800 KB，因依赖整个 `narrata-node-tools` 提供 `publishGraph`；移入 authoring Wasm、Wasm 专用 release 配置与 wasm-opt、重定预算、CI 跑 `check:web`）；发布对象集合与关系投影；节点构件演化核心（ADR 0023）。之后：visits 与读者地图、authoring Wasm 与 npm server 子路径、节点存档与 GC 维护入口、网络与在线阅读器生命周期、模拟宿主验收与交付。 |
| Coupling | REZICS 一侧由另一台电脑的 manager 实施；能力请求随 ADR 更新。维护者 2026-10-05 授权 manager 推送 `main`；CI 修复合并后推送并在 GitHub 上确认。 |
