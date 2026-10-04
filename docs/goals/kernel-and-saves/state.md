# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | ADR 0012 `narrata-storage`（契约、内存/按行 SQLite、一致性套件、故障注入与计数包装器）；`narrata-kernel`（摘要分帧、规范 CBOR、受检解码、u16 种类码封套、ID 宏）；ADR 0014 存档引擎：`narrata-store` 的领域逻辑在契约之上只实现一次，10K 提交时内存提交约 0.03 ms、SQLite 约 2 ms、打开约 0.014 ms，与 1K 同一量级；加载读取计数不随深度增长；v2 SQLite 显式迁移；冻结语料 `store-sqlite-v2`、`store-layout-v1`。 |
| Running | G-007 kernel 历史层（ADR 0015）：通用引擎抽取为 `narrata-history`，领域注册、通用提交 `{artifact, parent, input, state, depth}`、无重放恢复、测试领域与 `kernel-history-v1` 语料。 |
| Next | G-007 之后：G-008 浏览器存储（缓存后端 + 批次确认协议 + IndexedDB 适配器 + 浏览器测试，ADR 0016，考虑 codex xhigh）；G-009 旧栈去文本（Program/Snapshot 用 kernel 的 ContentRef/Segment，`SceneState` 可选，`stage6-v0`，stage5 迁移，协议与绑定）；浅存档 ADR（10K 深度 bundle 约 11 MB）；协议引擎改用 `SessionCoordinator` 与可注入存储。 |
| Decisions | 存储契约与 kernel 在 `packages/narrata/kernel/crates/`，不依赖 `narrata-core`；同步 trait；修订号全库单调；加载只校验目标提交，闭包完整由写入校验归纳保证；kernel 是"存储驱动"的，领域的 step 由调用方运行。ADR 0012/0014/0015 归本 Goal，0013 归 `narrative-core`。种类码：1–11 旧栈，0x0100–0x01FF 节点栈。 |
| Coupling | `narrative-core` 接受提交 API 草案（提交载荷与上面相同，State 受检解码见 ADR 0013 §6）；对方 G-005 往 `narrata-kernel/src/content/**` 加 ContentRef/Segment 与可选 `serde` feature（已同意 `--allow-area`）、新增 `fixtures/compat/nodes-r1|r2/`。G-007 合并后通知对方接入节点会话。`web-and-rezics` 依赖 IndexedDB 后端与存档字节。 |
| Host | 后台 `wait` 曾因内存压力被 Claude Code 回收（worker 不受影响）；改用轮询退出码文件。合并后的批次 gate 在固定提交的 `.temp/worktrees/verify-kernel-and-saves` 中运行，并总是跑 `task docs:check`。 |
