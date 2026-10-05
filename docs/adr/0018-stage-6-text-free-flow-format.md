# ADR 0018：Stage 1–5 栈去文本——Program 格式 1、Snapshot schema 1 与协议 v2

状态：Accepted（2026-10-05）。把 [决定 2](../product/decisions.md#2-内核不含正文) 落到 Flow/Statechart
栈；内容引用的形状沿用 [ADR 0013](0013-r2-text-free-node-format.md) §2；Program 与 Snapshot 的
版本轴、迁移机制与协议边界沿用 [ADR 0009](0009-stage-5-migration-and-protocol-boundary.md)。
[ADR 0014](0014-save-engine-key-layout.md) 的键布局与 [ADR 0015](0015-kernel-history-layer.md)
的历史层不变。

## 背景

Stage 1–5 的读者文本是 `ProgramArtifactV0.constants` 里的 `Value::String`，`Say`、`Choice` 的
提示和选项按常量下标引用它们，因此文本进入 `ProgramArtifactId`。运行时把文本复制进待处理交互，
Snapshot 编码它，`StateDigest`、回执、提交与 bundle 都随之依赖文本；逻辑分配计数按文本字节
计费，回执里的计数也随文本变化。`RuntimeStateV0.scene` 不可选。协议的 `Result.text`、
`Choice.label` 携带文本，`speaker` 在 DTO 中被丢弃。`ExternalContentDeclV0` 是从未接通的
占位类型。

## 决定

### 1. Program 格式 1

Program 对象（kind 1）的信封 schema 与载荷字段 0 的 `format_version` 都是 1，解码要求二者相同。
载荷的 map 键与格式 0 相同，变化只有三处：

- **内容表**：键 8 在格式 0 中是必须为空的"外部内容声明"，格式 1 中是内容表，每项为
  `[0, ContentRef]`（引用）或 `[1, Segment]`（段），`ContentRef`/`Segment` 用 `narrata-kernel`
  `content` 模块的规范 CBOR。`ExternalContentDeclV0` 删除，字段改名 `content`（`ContentEntryV1`）；
  格式 0 的字节不变。
- **呈现操作数**：`Say { speaker, text }`、`Choice { prompt }` 与选项 `label` 在线上仍是整数。
  格式 1 中它们是内容表下标：`text`（正文）必须指向段，`speaker`、`prompt`、`label` 必须指向
  引用；格式 0 中仍是字符串常量的下标。内存中的操作数是 `ContentOperand::{Constant, Content}`，
  解码按格式产生其一，校验拒绝与格式不符的一种。
- **常量**只给规则使用（`Const` 指令）。格式 1 允许字符串常量（谜题答案、要比较的名字），
  但呈现操作数不能引用常量。

引用（provider、key、锚点）是结构，进入 `ProgramArtifactId`；它们解析出的文字不进入。
`ProgramArtifactId = digest_bytes("program-artifact", format_version, 载荷)`，格式 0 的身份不变。
内容表最多 1,000,000 项（`ProgramLimits::max_content_entries`），不要求去重或全部被引用。

**Scene 是可选组件。** 含 `ReconcileScene` 指令的 Program 使用 scene（`CheckedProgram::uses_scene`）。

### 2. Snapshot schema 1 与运行时状态

Program 格式 0 只配 Snapshot schema 0，格式 1 只配 schema 1（`ProgramFormatVersion::snapshot_schema`）；
恢复与迁移拒绝其他组合。内存中的 `RuntimeStateV0` 增加 `snapshot_schema`，`scene` 改为
`Option<SceneState>`：schema 0 的状态总是 `Some`，编码规则与字节不变。

schema 1 的载荷键与 schema 0 相同，区别是：

- 键 7（scene）与键 8（Statechart）各自可选、按键序出现；键 7 在且仅在状态带 `SceneState` 时
  出现，默认值也写出。恢复要求它与 Program 是否使用 scene 一致。
- 待处理交互只记录内容表下标（`PendingContent::Content`，格式 0 为 `LegacyText`）：`Say` 为
  `[0, interaction, origin, parent_state, input, occurrence, speaker 下标|null, 正文下标, resume_to]`，
  `Choice` 为 `[1, interaction, origin, parent_state, input, occurrence, prompt 下标|null,
  [[choice, label 下标, target]…]]`。下标与起源指令的操作数冗余，恢复时逐个对照 Program 校验；
  导出拒绝与自身 schema 不符的状态（`SnapshotExportError::SchemaMismatch`）。
- `StateDigest = digest_bytes("runtime-state", 1, 载荷)`，Snapshot 信封 schema 为 1。

**逻辑分配计数。** 格式 1 每呈现一个内容引用计 1 个单位；格式 0 仍按 1 + 文本字节计。回执的
schema 与字段不变；格式 1 的回执只含摘要与计数，因此同样不随文字变化。提交、存储回执、
Checkpoint Bundle 与时间线归档的格式不变：它们按 ID 引用 Program 与 Snapshot，信封 schema 已在
object id 里。

结果：改格式 1 内容包里的一个字，`ProgramArtifactId`、`StateDigest`、Snapshot、回执、提交与
bundle 的字节都不变；改 provider 或 key 会改变它们（属性测试见 `narrata-store` 的
`content_identity`）。

### 3. 视图与呈现键

`SayView`/`ChoiceView` 的文字字段改为 `ContentView`：格式 1 给出 `Ref(ContentRef)` 或
`Segment(Segment)`，格式 0 给出 `LegacyText`。视图由待处理交互与 Program 一起得出
（`pending_view`），宿主解析引用（[内容引用契约](../contracts/content-references.md)）。

视图带交互的 `occurrence`（执行内交互计数器的值）。呈现键是 `(ExecutionId, CommitId,
occurrence)`：Flow 栈每个提交至多产生一个交互，同一交互里的说话人、正文、提示与选项文字由
宿主按角色区分。回到同一提交得到同一个键。

### 4. 协议 v2

- Protobuf 包改为 `narrata.protocol.v2`，`protocol_version`（`PROTOCOL_VERSION`）与 `abi_version`
  都是 2。版本不是 2 的请求，包括 v1 宿主的全部请求，得到 `NAR-P0002 incompatible` 诊断，响应
  按 v2 编码。C ABI 的符号不变，`nar_abi_version()` 返回 2，.NET 绑定据此拒绝不匹配的原生库；
  TypeScript 与 C# 的 DTO 从同一 `.proto` 重新生成。
- `Result` 保留字段号与名字 3（`text`），新增 `speaker = 7`、`body = 8`、`prompt = 9`
  （`ContentRef`/`Segment` 消息，锚点是 proto3 `optional`）、`capability = 10`（Effect 的能力 ID，
  v1 借用 `text`）与 `occurrence = 11`。`Choice` 保留字段号 2，名字 `label` 改指字段 3 的
  `ContentRef`。`SessionCreated` 增加 `execution_id = 4`，宿主据此组成呈现键。
- `ProgramLoad` 拒绝格式 0（`NAR-P0002`，提示先升级）：协议只运行不含文字的 Program，宿主从不经
  协议收到文字。
- `narrata-protocol` 的 DTO 是手写的 prost 结构；测试固定一个 `Result` 线上向量，TypeScript 测试
  用生成的代码解码同一向量，二者因此不会悄悄分叉。Unity 的 `INarrataPresenter` 收到的结果只含
  引用，`INarrataContentResolver` 是宿主按批解析它们的接口。

### 5. 格式 0 的读取与升级

格式 0 与 schema 0 继续可读、可校验、可在 Rust 内核与存档中运行，冻结语料与既有存档照常加载；
所有生成方（testkit 生成器、协议、以后的编译器）只产出格式 1。升级是确定性的：

- **Program**：`upgrade_program_v0` 把已校验的格式 0 Program 变为格式 1 Program 加一个本地内容包。
  - 被呈现操作数引用的每段文字 T 变为引用 `{ provider: "local", key: "stage5-" ‖
    hex(digest_bytes("narrata.stage5-text", 1, UTF-8(T)) 的前 16 字节) }`（`upgraded_text_reference`）。
    同一文字总得到同一引用，所以多个构件的内容包可以合并而不冲突；两段不同文字落到同一 key 时
    升级失败。
  - `speaker`、`prompt`、`label` 成为引用项，`text` 成为整单元段 `Segment { unit: 引用 }`。
    内容表按首次使用排序：流程按序、指令按序、操作数按 speaker、text 或 prompt、各 label 的
    顺序，相同的项只出现一次。
  - 只保留被 `Const` 引用的常量，保持原顺序并重写 `Const` 下标；只供呈现的文字常量因此离开
    构件。ID、流程、全局量、能力、Statechart、`ProgramId` 与语义版本不变。
  - 内容包是 [ADR 0013 §9](0013-r2-text-free-node-format.md#9-本地内容方) 的本地内容包 JSON
    （`format_version: 1, provider: "local"`，`LocalContentPack`），文字中的 `{`、`}` 写成
    `{{`、`}}`；语言由调用方给出（缺省 `und`），不影响构件。`narrata-content-local` 可直接解析它。
- **存档**：用 ADR 0009 的迁移机制，不另写重写路径。`format_upgrade_migration` 先重算源 Program 的
  升级结果并要求与目标逐字段相同，再给出描述符：空的重定位表、只接受 schema 0、没有恢复点，
  `MigrationId` 取 `digest_bytes("narrata.stage5-format-upgrade", 1, from ‖ to)` 的前 16 字节。
  `migrate_checked_state` 从源状态读取 Snapshot schema（不再由调用方传入），按目标 Program 的
  格式重建待处理交互与 scene：交互 ID 与 Effect ID 由记录的父状态摘要、输入摘要与起源计算，
  保持不变；不使用 scene 的目标得到 `None`，源状态的 scene 不是默认值时拒绝
  （`SceneWithoutTarget`）。迁移不允许降格式（`FormatDowngrade`）。
- **提交与 bundle**：`apply_migration` 写入格式 1 Program、schema 1 Snapshot 与 cause 为
  `Migration` 的提交，父提交是原来的格式 0 提交。之前的提交保持原样，可读，可在格式 0 上继续或
  再次升级。Checkpoint Bundle 先导入、再升级、再从迁移提交导出；时间线归档照常导入，宿主要继续
  的提交按同样方式升级。
- **CLI**：`narrata upgrade program <格式 0> --program-out <路径> --content-out <内容包>
  [--language <标签>]` 写出格式 1 Program（`.hex` 结尾时写十六进制）与内容包；
  `narrata upgrade dry-run|apply <SQLite 存档> --source <格式 0> --commit <id>` 报告或执行上述
  迁移，`apply` 另需 `--ref-owner`、`--ref-slot`，把存档 Ref 指向迁移提交。

### 6. 存储注册

`narrata-store` 的注册（ADR 0015 的 `Registry`）列出 Program 与 Snapshot 支持的 schema
（`PROGRAM_SCHEMAS`、`SNAPSHOT_SCHEMAS`：0、1）。其他 schema 的 Snapshot 写入失败；其他 schema
的 Program 照旧存而不索引，提交无法命名它。协调器、调试器与迁移按 Program 格式和状态的 schema
写对象，不再写死 0；`CommittedRunResult.reconcile_scene` 在 Program 没有 scene 时为 `None`。
种类、键空间与布局版本不变。

### 7. 冻结语料

`fixtures/compat/stage6-v0/` 冻结与 `stage5-v0` 同一组对象：格式 1 的 `branch-call-choice` 故事
（testkit 生成器）停在 Choice 时的 Program、Snapshot、存储回执、提交、Checkpoint Bundle 与时间线
归档，加清单、预期的待处理交互（提示与选项的引用、`occurrence`）与内容包。`emit_stage6_corpus`
示例把它们写进一个目录，两次运行逐字节相同；语料只读，重跑只用于比较。冻结测试证明以后的
版本能逐字节读取它们、导入 bundle 并运行到结束；`stage5-v0` 的测试另外证明它能经 dry-run、
升级与 bundle 往返后在格式 1 上运行到结束。

## 后果

- 宿主必须解析内容引用才能显示文字；`narrata-content-local` 能直接解析升级得到的内容包。
- 格式 0 留在内核与存档中只为读取与继续旧存档；新的能力只加在格式 1 上（决定 11 也限制它）。
- 绑定（.NET、TypeScript、Unity）必须与原生库同时升到 ABI 2；v1 宿主得到明确的版本诊断。
- 构件不记录 content lock：恢复从不解析内容（[决定 4](../product/decisions.md#4-内容引用协议与内容方无关)），
  需要"当时看到哪一版"的宿主按内容引用契约把解析得到的 `revision` 附在自己的记录上。
- Stage 1–5 的内容解析辅助（`ExternalContentResolver`、`CheckedContentResolution`）不受影响：
  会影响分支的内容仍作为 Recorded Query Effect 进入回执。
