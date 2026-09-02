# Stage 5：Migrations, Bindings and Debugging

状态：已实施（G5）；Nickel optional Gate 未通过且按 ADR 隔离
日期：2026-09-02
格式稳定性：冻结一代兼容 corpus；migration/protocol/binding API 仍为 `0.x` unstable

## 已交付能力

Stage 5 为旧存档建立持续兼容证据，并让 native、C、C#、Wasm 和 TypeScript 共用同一个
versioned pull protocol 与 canonical Rust runtime。

- `fixtures/compat/stage5-v0/` 冻结 Program、Snapshot、Receipt、Commit、Checkpoint Bundle、
  Timeline Archive、coverage/Catalog、Build Provenance、expected continuation 与每个文件的 SHA-256。
  当前版本会读取、导入并继续该 Say safe point，而不是只做 decode。
- Envelope、Program 和 Snapshot 有独立 version dispatch；unknown required version 返回
  `UnsupportedVersion`，所有旧 wire 仍需经过 checked domain 与大小/深度限制。
- Migration Registry 支持自动或显式多步路径、歧义拒绝、全套 Flow/Statechart/value/effect
  relocation、dry-run、有损 recovery 报告和三类显式确认。成功只新增 child Commit；失败事务不
  发布对象或 Ref。Instruction/Local 可用 Flow-scoped location 映射，重号不会跨 Flow 串线。
- authoring sidecar 保存 Visual/DSL Stable ID 与 source span，严格 JSON round-trip；duplicate ID
  同时报出两个位置。compiler relocation hint 与可执行 migration descriptor 是不同类型。
- `narrata migrate inspect|dry-run|apply` 支持 SQLite store、intermediate Program、显式 path、Ref
  revision CAS 与 recovery/effect/barrier 确认。
- `narrata-protocol` 定义 Engine/Program/Session、切片执行、Checkpoint/Timeline Archive、
  capability negotiation、Effect response、result 与 diagnostic。Timeline Archive 使用真实完整
  Catalog 记录，可导出后重新导入为活动 session。
- `narrata-ffi` 提供版本化窄 C ABI；真实 C harness 执行 ProgramLoad → SessionCreate → Dispatch，
  覆盖 buffer 单次所有权、double free、invalid handle、panic containment 和 oversized payload。
- C# binding 使用生成 DTO、`SafeHandle` 与 source-generated P/Invoke；conformance executable 通过
  native ABI 跑同一三步轨迹。Wasm browser target 编译，并与 native protocol 比较 Snapshot、
  Receipt、Commit 和 state hash。
- TypeScript DTO 由 Buf 从同一 `.proto` 生成；wrapper 提供 Promise/async iterator。IndexedDB
  adapter 在一个 transaction 中写 object 与 CAS ref，并测试冲突时无部分可见写入。
- Unity native 与 WebGL 使用互斥编译路径；WebGL 只经 `.jslib`/Wasm owned-byte bridge，不复用
  桌面 P/Invoke。
- debugger API 提供 timeline/branch/Catalog coverage、frame/state/value inspection、敏感 global/
  field redaction、instruction-slice breakpoint、migration preview 和给定原始输入的 Receipt
  deterministic replay verification。breakpoint 不是 durable safe point。

## Migration 安全规则

| 情况 | 行为 |
| --- | --- |
| relocation 缺失或 target 不存在 | 整步失败，不更新 Ref |
| 存在多条最短路径 | 要求显式 `--path` |
| 丢弃 frame/interaction/effect | 仅允许声明过的 recovery point，并要求 `--confirm-lossy` |
| pending Effect 重建身份 | 要求 `--confirm-effect-rekey`，报告新 Effect ID |
| recovery 跨外部效果 barrier | 额外要求 `--confirm-barrier` |
| migration chain 成功 | 每步一个 parent-linked migration Commit，只原子发布最终 Ref |

旧 Commit 和对象永远保留原 Program Artifact 身份。迁移不是原地改写，也不让新 Program 假装与旧
Snapshot 语义兼容。

## 可执行验证

完整 Gate：

```powershell
./scripts/check-g5.ps1
```

该脚本递归执行 G1–G4，再运行冻结 corpus、migration failure/CAS、source map、debug replay、
protocol/Timeline Archive、C harness、C# conformance、Wasm browser build、native/Wasm hash、生成
TypeScript DTO drift、async pull、IndexedDB 原子性和 Unity 平台分离检查。

## Nickel 结果与明确限制

Nickel optional Gate 没有全部满足，因此未发布 adapter，详见
[ADR 0010](../adr/0010-nickel-adapter-not-published.md)。core/FFI/Wasm 依赖图不含 Nickel。

本阶段仍不提供 Visual Graph editor、完整文本 DSL、timeline merge、多人同步、任意宿主 callback、
WIT Component 唯一 ABI 或通用云存档服务。C#/TS/Unity 是 protocol adapter，不拥有运行语义。
