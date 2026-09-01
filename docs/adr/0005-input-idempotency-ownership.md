# ADR 0005：Input 幂等职责属于 coordinator

状态：Accepted

## 决策

纯 core 保证相同 checked Program、parent state 与 input 产生相同 draft bytes。Receipt 保存
`InputId` 与 `InputPayloadDigest`，但 core 不查询历史。

同 ID/同 payload 返回旧结果、同 ID/不同 payload 报 conflict 的索引由 Phase 2 coordinator/store
维护。Stage 1 testkit 的 `EphemeralSession` 只提供进程内语义示范。

## 后果

Runtime kernel 不依赖 Commit graph 或 persistence，也不会把不断增长的去重历史塞进 Snapshot。

