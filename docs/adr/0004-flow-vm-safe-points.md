# ADR 0004：Flow VM safe point

状态：Accepted

## 决策

`RuntimeStateV0` 只表示 safe state。`Say`、`Choice` 与 `Finish` 到达时，每个存活 frame 的
evaluation stack 必须为空；validator 以 CFG dataflow 提前证明，Runtime 与 restore 再防御性检查。
slice yield 只保存 crate-private working state，不能编码或发布。

## 后果

Snapshot 保留 global/local、显式 frame 与 continuation，但不让 expression temporary 跨互动存活，
从而缩小 restore/migration 的状态空间。

