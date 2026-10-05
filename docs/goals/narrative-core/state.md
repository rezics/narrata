# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | manager 已启动。ADR 0013（R2 格式）；R1 冻结语料。G-006/G-008：`narrata-graph`、ADR 0016 图瓦片与摘要、`graph-v1` 语料。G-005：R2 核心（内容引用、16 字节身份、选择模型、CBOR 构件与按块 `Program`、不重放恢复、本地内容方、`migrate-r1`、阅读器适配、`nodes-r2` 语料）已合并 b9da2fd。 |
| Running | G-011（claude/xhigh）：动态提议。G-012（codex/xhigh）：10 万选择点合成基准与有界块缓存。 |
| Next | 内容大纲诊断（compose）；`narrata-graph` 接入 R2 与标签表 0x0124；节点会话接入 kernel 历史层（ADR 0015，节点 kind 0x0110–0x0112，阅读器用独立 store；浏览器部分等 kernel G-009 IndexedDB）；远程内容方适配器（REZICS）；契约页最终收缩。合并后 g5 尚未在 b9da2fd 上跑完（内存不足被系统停止），待重跑或由 kernel 的 g5 覆盖。 |
| Decisions | 节点对象用规范 CBOR，kind code 为 `0x0100–0x01FF`。恢复不重放，`verify_path` 可选。`ContentRef` 放在 `narrata-kernel`。阅读器由本 Goal 一次性认领：web-and-rezics 尚未启动，维护者把决定交给 manager。 |
| Efficiency | G-006：codex/xhigh，29 分钟，首次交接即接受、无返工，改动在范围内（停在需要格式 ADR 处，判断正确）。G-008：codex/xhigh，约 2.7 小时（含主机重启续跑），首次交接即接受。G-005：claude/xhigh，约 3.5 小时加一次 rebase 冲突续跑（medium），首次交接即接受，104 文件 +17.8k/−4.9k，在范围内。 |
| Coupling | kernel-and-saves：commit API 草案已接受（提交 `{artifact, parent, input, state, depth}`）。对方同意本 Goal 改 `narrata-kernel/src/content/**`，并建 `fixtures/compat/nodes-r1`、`nodes-r2`。节点会话接入 kernel 存储要等对方的 node-session API。 |
