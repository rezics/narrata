# Program 身份、依赖锁与迁移

状态：已决定
日期：2026-09-01

## 问题

存档不仅保存变量，还保存“程序执行到哪里”。只写产品版本或脚本文件名无法证明旧 frame、
return address、active state 和 pending choice 在新 Program 中仍有相同含义。

默认恢复策略必须是：**用存档固定的精确 Program Artifact 继续执行**。只有精确构件不可用
或产品明确升级存档时，才运行声明过的 migration。

## 四类身份

| 类型 | 用途 | 是否持久 | 不能替代它的值 |
| --- | --- | --- | --- |
| `ProgramId` | 一个作品/程序家族的作者语义身份 | 是 | 标题、包名 |
| `StableId` | Flow、continuation、state、choice 等元素身份 | 是 | 行号、byte offset、数组位置 |
| `ProgramArtifactId` | 精确 runtime-relevant 编译产物摘要 | 是 | SemVer、Git branch |
| `BuildProvenanceId` | source/compiler/Nickel/debug 信息，可重建和审计 | 是，但不决定 continuation 兼容 | `ProgramArtifactId` |

`ProgramArtifactId` 只 hash 会影响 Runtime 行为、Effect payload 或外部内容解析的规范化构件。
source map、构建时间和绝对路径放入单独 `BuildProvenance`；这样只改注释或本地路径不会让
玩家存档失配。

## Program Artifact

概念结构：

```rust
struct ProgramArtifactV1 {
    format_version: ProgramFormatVersion,
    semantics_version: SemanticsVersion,
    program: ProgramId,
    ir: TypedProgram,
    stable_ids: StableIdTable,
    capabilities: CapabilityManifest,
    content_lock: ContentLockId,
}
```

`ProgramArtifactId` 使用与 save object 相同的 domain-separated deterministic encoding/hash。
Artifact decoder 先产生 untrusted wire value，再通过 compiler/runtime 的 checked constructor
证明：

- 所有 ID 唯一且类型正确；
- jump/call/return/state target 存在；
- expression operand/result 类型一致；
- capability 版本可协商；
- eventless loop、stack、constant/collection 大小受限；
- content dependency 满足声明的 resolution policy。

Artifact 通过 hash 不代表以上领域规则成立。

## Stable ID

### 必须稳定的对象

- Flow、Statechart state/region/transition；
- 所有会进入 Snapshot 的 instruction/continuation anchor；
- call frame return target；
- Choice、Checkpoint、Effect call site；
- schema type/variant/field；
- 外部内容 occurrence reference。

Compiler 内部且不可能出现在 safe-point continuation 中的临时寄存器或 basic-block offset 不必
成为永久公共 ID。

### 生成和维护

Visual Editor 创建元素时生成 opaque 128-bit Stable ID，移动、改名、重排和文本修改不改变
它。文本 DSL 支持显式 anchor；编辑器/formatter 可以维护 source sidecar，但 sidecar 是作者
资产，不能在每次 build 重新按行号生成。

以下算法只能用于作者工具提出 migration 建议，不能在 runtime 静默采用：

- 相似文本；
- 相邻 source span；
- 同名 label；
- AST shape；
- 当前数组位置。

## Content Lock

借鉴 flake lock 对依赖图的固定，Narrata Artifact 至少记录：

```rust
struct ContentLockV1 {
    capability_schemas: Vec<LockedCapabilitySchema>,
    external_content: Vec<LockedContentDependency>,
    runtime_modules: Vec<LockedRuntimeModule>,
}
```

`ContentLockId` 是 `ContentLockV1` 的 domain-separated 内容摘要。Program Artifact 引用该
ID，因此 lock 内容变化会改变 `ProgramArtifactId`，同时 Checkpoint/Timeline Bundle 与 GC
又能把 lock 当作独立 immutable object 遍历和去重。

Source、compiler、authoring import 与 Nickel evaluator 的精确锁属于 `BuildProvenance`；它们
生成的最终结果已经体现在 canonical IR 中。只有运行时仍需解析、尚未完全嵌入 IR 的依赖才
进入 `ContentLock`，避免同一 typed Program 因无关构建路径不同而产生不同 Artifact ID。

发布清单可以另外保存 `ArtifactRecord { artifact, provenance }`。`BuildProvenance` 可随调试或
审计 bundle 导出，但不是恢复剧情所需的 save closure，也不能在 load 时替代精确 Artifact。

每项使用可验证 revision/content hash，而不是 `latest`。允许 live 内容时，必须显式选择
resolution policy：

| Policy | 恢复语义 |
| --- | --- |
| `Pinned` | 必须取得相同 revision/hash，否则 `NeedsContent` |
| `Compatible` | provider 用已声明兼容规则证明可替代，否则失败 |
| `LivePresentationOnly` | 可取当前内容，但该内容不得影响 guard、choice 或 Effect payload |

依赖某个 save 的 Program Artifact 与 Content Lock 都是 GC closure 的一部分。

## REZICS 外部内容

Narrata 引用 REZICS `ContentStructureNode` occurrence，而不是 Post ID，详见
[REZICS Gamebook 集成边界](../rezics-gamebook-integration.md)。恢复还需验证：

- provider、structure 和 occurrence node identity；
- 当前调用者权限；
- node 是否仍属于结构且未 retired；
- lock policy 要求的 revision/compatibility；
- resolver response schema。

权限、存在性和当前本地化是每次跨信任边界重新建立的事实。格式正确的引用本身不证明
用户有权读取。若 resolver 结果会影响分支，它作为 Recorded Query 写入 Receipt；若只是
`LivePresentationOnly` 正文，可以重新解析但不能改变 Runtime 结果。

## Save schema 与 Program migration 分离

至少有四个独立版本轴：

1. **Object/Envelope Schema**：怎样读取 bytes、对象 kind 和字段；
2. **Snapshot Schema**：当前 Rust 数据结构怎样表达同一运行语义；
3. **Program Artifact/Semantics**：指令、Statechart 和 Effect 的含义；
4. **Binding Protocol**：C/Protobuf/Wasm request/response。

升级 decoder 并不证明旧 continuation 能在新 Program 执行。反之，Program migration 也不应
要求 FFI wire version 同时变化。

## Migration Registry

CheckpointBundle 和 TimelineArchiveBundle 都不携带可执行代码。可信应用/compiler 注册
migration：

```rust
struct MigrationDescriptor {
    id: MigrationId,
    from: ProgramArtifactId,
    to: ProgramArtifactId,
    accepted_snapshot_schemas: VersionRange,
    relocations: RelocationTable,
    recovery_points: RecoveryTable,
}

trait TrustedMigration {
    fn migrate(
        &self,
        source: CheckedOldState,
        source_program: &CheckedProgram,
        target_program: &CheckedProgram,
    ) -> Result<CheckedTargetState, MigrationError>;
}
```

`RelocationTable` 分开处理：

- `InstructionId → InstructionId`；
- `FlowId → FlowId`；
- `StateId/history → StateId/history`；
- schema type/field/variant rename；
- pending Choice/Effect/Interaction；
- 删除目标到作者声明 recovery checkpoint 的映射。

值转换可以是受信 Rust function 或未来经过审计的 declarative migration IR。v1 不从存档、
Nickel 或网络下载任意 migration code。

## Migration 算法

1. 完整验证 source Commit、Snapshot 和精确 source Artifact；
2. 找到调用方明确选择的 `(from, to)` migration path；
3. 每一步先升级 Object/Snapshot schema，不改变剧情语义；
4. 对 value/schema、VM frame、return target、active state/history、pending interaction/effect
   分别迁移；
5. 使用 target Program checked constructor 验证所有引用和状态组合；
6. 在纯环境中执行 dry-run determinism check；
7. 创建以 source Commit 为 parent 的新 migration Commit；
8. 保留旧 Commit，不原地覆盖；
9. 只有 migration Commit 持久化成功后，CAS 更新目标 Ref；
10. 输出包含所有 relocation/recovery 的报告。

多跳 migration path 若存在歧义，必须由调用方或发布 manifest 指定，不能选最短路径猜测。

## Recovery checkpoint

当某个旧 continuation 已被删除，作者可以声明：

```text
old InstructionId -> target RecoveryCheckpointId
```

这是有损迁移，报告必须列出：

- 丢弃了哪些 frame/local/pending interaction；
- 哪些 values 被保留、重置或转换；
- 是否跨越 external effect barrier；
- 玩家将在何处重新进入。

默认 UI 需要用户确认。没有 recovery declaration 时返回 `UnmappedContinuation`。

## Hot reload

Hot reload 只在 committed safe point 发生，本质上是同一套 migration：

- 若新 build 的 `ProgramArtifactId` 不变，只换 source/debug metadata；
- 若改变，必须存在 migration 或继续使用旧 Artifact；
- 不能在 instruction、microstep 或 Effect dispatch 中间替换 Program；
- editor 可以先 dry-run 所有活跃 session，再决定发布。

## Nickel provenance

若启用 Nickel：

```text
.ncl + locked imports + exact Nickel evaluator
  → deep evaluation / contracts forced
  → Rust checked AuthoringManifest
  → canonical Program Artifact
```

`LockedEvaluator`、source hash 和 authoring import lock 进入 `BuildProvenance`。如果它们改变
runtime 输出，canonical IR 自然改变 `ProgramArtifactId`；不再把同一信息重复塞进
`ContentLock`。相同最终 typed Program 可以得到同一 `ProgramArtifactId`，即使作者源文件
布局不同。

Nickel contract 不替代 target Program 的 Rust checked constructor，也不执行玩家存档迁移。

## Load 决策

```text
存档 Artifact 可用？
  ├─ 是 → exact restore
  └─ 否 → target 与 migration 是否明确？
           ├─ 否 → NeedsProgram / Incompatible
           └─ 是 → source Artifact 可用且 migration 验证通过？
                    ├─ 是 → 创建 migration Commit 后恢复
                    └─ 否 → Incompatible，保留原存档
```

SemVer range、相同 ProgramId 或相同显示标题都不能跳过这棵决策树。

## 必需验证

- 重排/改名但 Stable ID 不变时，continuation 可由声明 migration 精确迁移；
- Stable ID 重复在编译期失败并指出两个 source span；
- Artifact 缺失时不会偷偷用“当前版本”加载；
- migration Fault、panic、超限或 target 验证失败不更新 Ref；
- migration 是确定的：同一 source/target/descriptor 得到同一 target Snapshot/Commit；
- source Commit 和 Artifact 在成功迁移后仍可读取，直到 retention 明确删除；
- `LivePresentationOnly` 内容变化不改变 Effect/choice/state hash；
- Nickel 源文件等价变换不改变 typed Program 时，runtime Artifact hash 保持不变；
- native/Wasm binding 升级不要求重写 Snapshot，除非相应语义版本也改变。
