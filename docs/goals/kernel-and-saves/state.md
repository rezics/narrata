# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | ADR 0012 `narrata-storage`；`narrata-kernel`；ADR 0014 存档引擎（10K 与 1K 提交延迟同一量级，加载读取不随深度增长）；ADR 0015 `narrata-history`（通用历史层、领域注册、无重放恢复、`kernel-history-v1`）；协议引擎可注入存储并复用 `SessionCoordinator`；ADR 0017 浏览器存储（`narrata-storage-host` 缓存后端、IndexedDB 适配器 `packages/narrata/kernel/js`、Playwright）；ADR 0018 旧栈去文本（Program format 1、Snapshot schema 1、protocol v2、`stage6-v0`、stage5 升级）。 |
| Running | G-015 浅存档（ADR 0019，`kernel-history-v2`）与浏览器存储冻结语料，补 `store.rs` 的 match 分支后重跑 gate。 |
| Next | Flow VM/Statechart 成为 kernel 上的领域包、Effect 账本与迁移协调的通用部分进入 kernel（决定 11 剩余部分）；节点会话经 kernel 存档由 `narrative-core` 接入（已通知）。之后核对 Goal 验收并收尾。 |
| Decisions | 存储契约与 kernel 在 `packages/narrata/kernel/crates/`，不依赖 `narrata-core`；同步 trait；修订号全库单调；加载只校验目标提交，闭包完整由写入校验归纳保证；kernel 是"存储驱动"的，领域的 step 由调用方运行。ADR 0012/0014/0015 归本 Goal，0013 归 `narrative-core`。种类码：1–11 旧栈，0x0100–0x01FF 节点栈。 |
| Coupling | `narrative-core` 接受提交 API 草案（提交载荷与上面相同，State 受检解码见 ADR 0013 §6）；对方 G-005 往 `narrata-kernel/src/content/**` 加 ContentRef/Segment 与可选 `serde` feature（已同意 `--allow-area`）、新增 `fixtures/compat/nodes-r1|r2/`。G-007 合并后通知对方接入节点会话。`web-and-rezics` 依赖 IndexedDB 后端与存档字节。 |
| Engines | 维护者 2026-10-05：Claude 用量不足，worker 默认用 codex（GPT-6.1 Sol）。 |
| Host | 后台 `wait` 曾因内存压力被 Claude Code 回收（worker 不受影响）；改用轮询退出码文件。合并后的批次 gate 在固定提交的 `.temp/worktrees/verify-kernel-and-saves` 中运行，并总是跑 `task docs:check`。 |
