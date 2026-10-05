# State

本 Goal 的 manager 检查点（[Goal](GOAL.md)）；存活任务：`task goal -- status`。

| 字段 | 状态（2026-10-05） |
| --- | --- |
| Done | ADR 0013（R2）、R1/R2 冻结语料。G-006/G-008：`narrata-graph`、ADR 0016、`graph-v1`。G-005：R2 核心与阅读器适配。G-011：动态提议（`nodes-r2-proposals`）。G-012：合成作品基准、有界块缓存。G-014：会话经 kernel 历史层（ADR 0015）与浏览器 CacheBackend/IndexedDB 持久化，`nodes-r2-history`；长游玩 18 轮共 18 万次选择，p99 0.116 ms，最大 0.572 ms。合并后 r1 与 g5 在 4e8dc64 通过。 |
| Running | 无。Goal 已结束（2026-10-05）。 |
| Next | 无。G-017 发布时检查与 G-018 收尾（Wasm 发布分析、仅清单打开、完整的只改文字属性测试）已合并于 c5f2bc3；只读审计的其余条目均满足。单图 4,096 节点、单块 4 MiB 是 ADR 0013 取代全局上限的设计；单次选择低于 1 ms 依赖宿主预取，见 `nodes-scale.md`。 |
| Decisions | 节点对象用规范 CBOR，kind code 为 `0x0100–0x01FF`。恢复不重放，`verify_path` 可选。`ContentRef` 放在 `narrata-kernel`。阅读器由本 Goal 一次性认领：web-and-rezics 尚未启动，维护者把决定交给 manager。 |
| Efficiency | G-006：codex/xhigh，29 分钟，首次交接即接受、无返工，改动在范围内（停在需要格式 ADR 处，判断正确）。G-008：codex/xhigh，约 2.7 小时（含主机重启续跑），首次交接即接受。G-005：claude/xhigh，约 3.5 小时加一次 rebase 冲突续跑（medium），首次交接即接受，104 文件 +17.8k/−4.9k，在范围内。 |
| Coupling | kernel-and-saves：节点会话接入已交付（G-014），对方最后一条验收解除。codex 登录探测只查本地文件，会把失效会话报为已登录，派发前用一次真实 `codex exec` 验证。 |
