# ADR 0006：Stage 2 persistence boundary

状态：Accepted

## 决策

`narrata-core` 继续只产生纯 `TransitionDraft`，不持有 store、clock 或数据库连接。
`SessionCoordinator` 是 draft 成为 host-visible result 的唯一发布路径：Snapshot、Receipt、Commit、
Input index、branch/active Ref，以及完整记录模式下的 Catalog Event/Catalog Head 必须在同一
`SaveStore` transaction 成功后才返回 `CommittedRunResult`。

Stage 1 的 `StateDigest`、`ReceiptDigest` 和 `ProgramArtifactId` 保持不变。Stage 2 另以
`narrata-object\0 || kind || schema || canonical payload` 计算持久 `ObjectId`，再通过不可互换的
`SnapshotId`、`ReceiptId`、`CommitId`、`TimelineCatalogEventId` 和 manifest ID 表达对象角色。
运行摘要与持久对象身份不能互相替代。

`MemoryStore` 是独立于 SQLite 表布局的行为参考；`SqliteStore` 在 `BEGIN IMMEDIATE` 的一致性
视图中应用同一 checked transaction，再以 SQLite 原子提交发布新状态。SQLite row ID、墙上时间、
slot 展示名和 telemetry 不参与确定性对象身份。

## 后果

- 普通 checkpoint 与完整时间线使用不同 bundle tag、manifest 和 Ref 类型；调用方不能通过一个
  布尔字段扩大数据保留承诺。
- save、branch、Catalog Head 与 archive root 都使用 revision CAS。并发失败返回 expected、actual
  和 proposed target，不执行 last-write-wins。
- 外部 bundle 与数据库 row 必须重新经过 envelope、hash、schema、closure、Program 和
  `RuntimeState` 校验；验证完成前不得创建永久 Ref。
- `Complete` 只从 manifest 中声明的 coverage baseline 起完整。删除 save Ref 只移除当前目录项；
  Catalog tombstone 和 archive root 仍可能保留旧 Commit。永久删除需要显式移除或替换 archive
  root，并在 GC 后根据其他 root 报告实际结果。
- Effect ledger 和不可逆宿主 Effect 仍属于 Stage 3，Stage 2 的 `ledger_fence` 固定为零。

