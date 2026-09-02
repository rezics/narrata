# Phase 5：迁移、Binding 与 Nickel 适配

状态：已实施（G5）；详见 [Stage 5 交付说明](./stage-5-migrations-and-bindings.md)
前置：[Phase 2](./02-time-travel-and-persistence.md)；Statechart 迁移依赖
[Phase 4](./04-statecharts.md)
完成：G5 发布门槛；Nickel 为可选子 Gate

## 目标

让冻结后的旧存档有持续兼容测试，并让 C/C#/JavaScript/Wasm 调用同一 Rust 语义。最后以
独立 spike 判断 Nickel 是否值得成为可选 authoring/config frontend。

## Track A：存档与 Program migration

### P5.1 冻结首个兼容 fixture

在宣布 save v1 前，把以下内容复制到不可随普通重构改写的 corpus：

```text
Program Artifact bytes/hash
Snapshot/Receipt/Commit bytes/hash
CheckpointBundle
TimelineArchiveBundle（含 coverage/Catalog fixture）
expected continuation/effects
source BuildProvenance
```

CI 使用当前代码读取并继续运行。更新 expected bytes 必须通过格式 ADR/migration 变更，不能
使用 blanket snapshot update。

### P5.2 Versioned decoder chain

交付：

- Object/Envelope、Snapshot、Program 分开的 version dispatch；
- old wire → checked old domain → current representation；
- 每一步大小/深度限制；
- unknown required version 返回 `UnsupportedVersion`；
- decoder 升级不伪装成 Program semantic compatibility。

### P5.3 Migration Registry 与 CLI

交付：

- `(from Artifact, to Artifact) → TrustedMigration` registry；
- instruction/frame/state/history/choice/effect/value relocation；
- dry-run report；
- migration 成功创建以 source 为 parent 的新 Commit；
- 失败不更新 Ref，不修改 old object；
- 多路径歧义要求显式选择；
- `narrata migrate inspect/dry-run/apply`。

### P5.4 Authoring Stable ID 与 source map

交付：

- Visual/DSL document 的持久 ID；
- sidecar round-trip；
- compiler relocation hints 只生成建议；
- 重排/改名不换 ID；
- duplicate ID 指出两个 source span；
- recovery checkpoint 的有损迁移报告。

## Track B：Versioned Protocol 与 Binding

### P5.5 Protocol schema

定义 versioned message：

```text
EngineCreate / ProgramLoad
SessionCreate / SessionLoad
Dispatch / ContinueSlice
CommittedRunResult / Diagnostic
CheckpointBundle / TimelineArchiveBundle import/export
Capability negotiation / Effect response
```

优先使用 Protobuf 生成 Rust/C#/TS DTO，但明确：

- Protobuf bytes 不参与 Commit/Object hash；
- unknown field、size/depth 和 message version 在边界检查；
- DTO 不能直接成为 checked Runtime types；
- protocol version 与 save/semantics version 分离。

### P5.6 C ABI

交付窄 ABI：opaque handle、owned byte buffer、status code、显式 free。要求：

- 不暴露 Rust enum、`Vec`、borrow、trait、panic；
- handle 单线程、不可重入，或明确同步模型；
- buffer ownership token 双重释放/错误 handle 测试；
- panic 在边界转为 diagnostic；
- ABI symbol/version check；
- C harness 跑完整 conformance trace。

### P5.7 C# 与 Unity

交付：

- `SafeHandle` 与 source-generated P/Invoke；
- typed C# DTO/domain wrapper，不暴露 ownership token；
- async wrapper 只包装 pull protocol，不保存 continuation；
- Unity Presenter/Scene reconciler、Host SaveCoordinator；
- editor/domain assembly 与 runtime assembly 分离；
- native platform matrix；Unity Web 不复用桌面 P/Invoke。

### P5.8 Wasm 与 JavaScript/TypeScript

交付：

- Wasm 调用相同 protocol 和 canonical core；
- JS wrapper 处理 owned bytes、Promise/async iterator；
- IndexedDB adapter 通过 Phase 2 store conformance；
- browser resource limit；
- native/Wasm 相同 trace 产生相同 Snapshot/Receipt/Commit hash；
- Unity Web 的 `.jslib`/Wasm bridge 作为单独 adapter。

## Track C：Nickel optional spike

### P5.9 隔离式 compiler adapter

建立独立 `narrata-nickel` tool/feature，core 默认依赖图不含 Nickel。

Spike 输入：一个 host/capability/content resolver manifest，包含 partial records、default、
override 和 contract。流程：

```text
locked .ncl inputs
→ resource-limited subprocess
→ eval_deep_for_export
→ Serde wire manifest
→ Narrata checked constructor
→ canonical Artifact
```

验收：

- contract 确实在 deep export 时全部触发；
- import root、网络、时间、内存和输出受限；
- exact Nickel version/import lock 进入 provenance；
- contract 由权威 schema 生成或 drift test；
- 相同 locked input 重复 build hash 相同；
- 同 output、不同无关 source layout 得到同 runtime Artifact hash；
- 关闭 feature 后 Runtime/FFI/Wasm 不链接 Nickel。

若失败，记录 ADR 并继续使用 JSON/Rust typed manifest；G5 不被阻塞。Nickel package management
当前仍是实验能力，不能成为 v1 可重复构建的唯一来源：
[Nickel package management](https://nickel-lang.org/user-manual/package-management/)。

## Track D：Debugger 与时间线

在已有 Commit/Receipt 上增加，不改变运行语义：

- timeline/branch/Barrier 可视化；
- 完整时间线的 coverage、save Catalog 与 prune/delete 后果可视化；
- state/value/frame/chart inspector；
- Receipt replay verification；
- breakpoint 仅停在 instruction slice，明确区分 durable safe point；
- migration preview；
- 敏感 Value redaction。

## Phase 5 exit gate

G5 必需：

- 至少一代冻结存档由当前版本读取、继续并通过 expected trace；
- migration 只创建新 Commit，失败不改 Ref；
- C、C#、Wasm/TS 共用 protocol conformance；
- native/Wasm hash 一致；
- FFI ownership、panic、invalid handle、oversize payload tests 全部通过；
- Unity native 与 Web 路径分开验证。

Nickel optional Gate：只有 P5.9 全部满足才发布；否则保留研究结果，不进入核心依赖。

本次 optional spike 未满足资源隔离与 schema drift 全部条件，因此按
[ADR 0010](../adr/0010-nickel-adapter-not-published.md) 不发布 Nickel adapter；这不阻塞 G5。
