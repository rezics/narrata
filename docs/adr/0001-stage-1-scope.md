# ADR 0001：Stage 1 合并 G0 与 G1

状态：Accepted

## 决策

第一个实现里程碑同时交付契约/判据与最小内存 Runtime，而不是创建不能执行故事的空类型层。
workspace 仅包含 core、testkit、CLI；Store 与 Statechart 延后到依赖它们的阶段。

## 后果

第一阶段可独立证明 deterministic execution 与 snapshot/restore 等价；不会用空 `narrata-store`
或伪 `CommitId` 提前冻结持久化契约。

