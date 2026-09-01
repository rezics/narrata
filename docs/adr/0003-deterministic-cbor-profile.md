# ADR 0003：Narrata deterministic CBOR profile

状态：Accepted

## 决策

使用只覆盖 Narrata v0 schema 的 manual encoder/strict reader：

- shortest integer/length；definite length only；
- no float、tag、indefinite item；
- schema map key 只用递增 unsigned integer；
- duplicate/unknown/unsorted field 与 trailing bytes 拒绝；
- exact UTF-8，不做 Unicode normalization；
- decode 后重新编码必须与原 payload 完全相同。

Envelope 固定 magic、独立 envelope/kind/schema version、flags、u64 payload length、payload SHA-256。

## 后果

hash 不依赖泛型 Serde、allocator、平台 `usize` 或 map insertion order。新增字段必须显式版本化。

