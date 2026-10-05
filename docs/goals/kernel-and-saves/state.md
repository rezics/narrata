# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | ADR 0012 `narrata-storage`；`narrata-kernel`；ADR 0014 存档引擎（10K 与 1K 提交延迟同一量级，加载读取不随深度增长）；ADR 0015 `narrata-history`；协议引擎可注入存储并复用 `SessionCoordinator`；ADR 0017 浏览器存储（缓存后端 + IndexedDB + Playwright）；ADR 0018 旧栈去文本（`stage6-v0`）；ADR 0019 浅存档；ADR 0020 通用 Effect 账本与迁移协调；存储契约页已收缩。所有批次 gate（g5、web-storage、docs）在 c4c229c 通过。 |
| Running | 无。 |
| Next | 等 `narrative-core` 的 G-014（NodeDomain 接入 `narrata-history`、阅读器用 CacheBackend/IndexedDB）合并，核对验收"节点会话的保存、恢复、导出、导入通过 kernel 完成"；然后把 Goal 的决定并入归属文档、从根 `GOAL.md` 删行并 `goal close`。FFI/Wasm 旧协议引擎仍默认内存 + 存档字节（不在验收内，记在契约页缺口）。 |
| Decisions | 存储契约与 kernel 在 `packages/narrata/kernel/crates/`，不依赖 `narrata-core`；同步 trait；修订号全库单调；加载只校验目标提交，闭包完整由写入校验归纳保证；kernel 是"存储驱动"的，领域的 step 由调用方运行。ADR 0012/0014/0015 归本 Goal，0013 归 `narrative-core`。种类码：1–11 旧栈，0x0100–0x01FF 节点栈。 |
| Coupling | `narrative-core` 接受提交 API 草案（提交载荷与上面相同，State 受检解码见 ADR 0013 §6）；对方 G-005 往 `narrata-kernel/src/content/**` 加 ContentRef/Segment 与可选 `serde` feature（已同意 `--allow-area`）、新增 `fixtures/compat/nodes-r1|r2/`。G-007 合并后通知对方接入节点会话。`web-and-rezics` 依赖 IndexedDB 后端与存档字节。 |
| Engines | 维护者 2026-10-05：Claude 用量不足，worker 默认用 codex（GPT-6.1 Sol）。 |
| Host | 后台 `wait` 曾因内存压力被 Claude Code 回收（worker 不受影响）；改用轮询退出码文件。合并后的批次 gate 在固定提交的 `.temp/worktrees/verify-kernel-and-saves` 中运行，并总是跑 `task docs:check`。 |
