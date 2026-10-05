# ADR 0018：Stage 1–5 栈去文本——Program 格式 1、Snapshot schema 1 与协议 v2

状态：Accepted（2026-10-05）。把 [决定 2](../product/decisions.md#2-内核不含正文) 落到 Flow/Statechart
栈；内容引用的形状沿用 [ADR 0013](0013-r2-text-free-node-format.md) §2；Program 与 Snapshot 的
版本轴、迁移机制与协议边界沿用 [ADR 0009](0009-stage-5-migration-and-protocol-boundary.md)。
ADR 0014/0015 的键布局与历史层不变。

## 背景

Stage 1–5 的读者文本是 `ProgramArtifactV0.constants` 里的 `Value::String`，由 `Say`、`Choice`
的提示和选项按常量索引引用，因此进入 `ProgramArtifactId`。运行时把文本复制进待处理交互，
Snapshot 编码它，`StateDigest`、回执、提交与 bundle 都随之依赖文本；逻辑分配计数按文本字节计费，
回执里的计数也随文本变化。`RuntimeStateV0.scene` 不可选。协议 `Result.text`、`Choice.label` 携带
文本，`speaker` 在 DTO 中被丢弃。`ExternalContentDeclV0` 是一个从未接通的占位类型。

## 决定

### 1. Program 格式 1

Program 对象（kind 1）的 schema 与载荷字段 0 的 `format_version` 都是 1，二者必须相同。载荷的
map 键与格式 0 相同，变化只有三处：

- **内容表**：键 8 在格式 0 中是必须为空的"外部内容声明"，格式 1 中是内容表，每项为
  `[0, ContentRef]`（引用）或 `[1, Segment]`（段），`ContentRef`/`Segment` 使用 `narrata-kernel`
  `content` 模块的规范 CBOR。`ExternalContentDeclV0` 删除，字段改名为 `content`；格式 0 的字节
  不变。
- **呈现操作数**：`Say { speaker, text }`、`Choice { prompt }` 与选项 `label` 的整数在格式 1 中是
  内容表下标：`text`（正文）必须指向段，`speaker`、`prompt`、`label` 必须指向引用。格式 0 中仍是
  字符串常量的下标。内存中的操作数是 `ContentOperand::{Constant, Content}`，解码按格式产生
  对应的一种，校验拒绝与格式不符的一种。
- **常量**只给规则使用（`Const` 指令）。格式 1 允许字符串常量（谜题答案、要比较的名字），
  但呈现操作数不能引用常量。

引用（provider + key + 锚点）是结构，进入 `ProgramArtifactId`；它们解析出的文字不进入。
`ProgramArtifactId = digest_bytes("program-artifact", format_version, 载荷)`，格式 0 的身份不变。
内容表最多 1,000,000 项（`ProgramLimits::max_content_entries`），不要求去重或全部被引用。

**Scene 是可选组件。** 含 `ReconcileScene` 指令的 Program 使用 scene；格式 1 的状态在且仅在
Program 使用 scene 时带 `SceneState`。

### 2. Snapshot schema 1 与运行时状态

Program 格式 0 只配 Snapshot schema 0，格式 1 只配 schema 1；恢复与迁移拒绝其他组合。内存中的
`RuntimeStateV0` 增加 `snapshot_schema`，`scene` 改为 `Option<SceneState>`（schema 0 的状态总是
`Some`，编码规则不变）。

schema 1 的载荷键与 schema 0 相同，区别是：

- 键 7（scene）在且仅在状态带 `SceneState` 时出现，默认值也写出；恢复要求它与 Program 是否
  使用 scene 一致。
- 待处理交互只记录内容表下标：`Say` 为
  `[0, interaction, origin, parent_state, input, occurrence, speaker 下标|null, 正文下标, resume_to]`，
  `Choice` 为 `[1, interaction, origin, parent_state, input, occurrence, prompt 下标|null,
  [[choice, label 下标, target]…]]`。下标与起源指令的操作数冗余，恢复时逐个对照 Program 校验。
- `StateDigest = digest_bytes("runtime-state", 1, 载荷)`，Snapshot 信封 schema 为 1。

**逻辑分配计数。** 格式 1 每呈现一个内容引用计 1 个单位，格式 0 仍按 1 + 文本字节计。回执的
schema 与字段不变；格式 1 的回执只含摘要与计数，因此同样不随文字变化。提交、存储回执、
Checkpoint Bundle 与时间线归档的格式不变：它们按 ID 引用 Program 与 Snapshot，信封 schema 已在
object id 里。

结果：改格式 1 内容包里的一个字，`ProgramArtifactId`、`StateDigest`、Snapshot、回执、提交与
bundle 的字节都不变；改 provider 或 key 会改变它们。

### 3. 视图与呈现键

运行时视图 `SayView`/`ChoiceView` 的文字字段改为 `ContentView`：格式 1 给出 `Ref(ContentRef)`
或 `Segment(Segment)`，格式 0 给出 `LegacyText`。视图由待处理交互与 Program 一起得出
（`pending_view`），宿主解析引用（[内容引用契约](../contracts/content-references.md)）。

视图带交互的 `occurrence`（执行内交互计数器的值）。呈现键是 `(ExecutionId, CommitId,
occurrence)`：Flow 栈每个提交至多产生一个交互，同一交互里的说话人、正文、提示与选项文字由
宿主按角色区分。回到同一提交得到同一个键。

### 4. 协议 v2

- Protobuf 包改为 `narrata.protocol.v2`，`protocol_version` 与 `abi_version` 都是 2。版本不是 2
  的请求（包括 v1 宿主的全部请求）得到 `NAR-P0002 incompatible` 诊断，响应按 v2 编码；C ABI 的
  符号不变，`nar_abi_version()` 返回 2，.NET 与 TypeScript 绑定据此拒绝不匹配的原生库。
- `Result` 保留字段 3（`text`），新增 `speaker`、`body`、`prompt`（`ContentRef`/`Segment` 消息）、
  `capability`（Effect 的能力 ID，原来借用 `text`）与 `occurrence`；`Choice` 保留字段 2（`label`），
  `label` 改为字段 3 的 `ContentRef`。`speaker` 不再丢失。
- `ProgramLoad` 拒绝格式 0（`NAR-P0002`，提示先升级）：协议只运行不含文字的 Program，宿主从不经
  协议收到文字。

### 5. 格式 0 的读取与升级

格式 0 与 schema 0 继续可读、可校验、可在 Rust 内核中运行，冻结语料与既有存档照常加载；所有
生成方（testkit 生成器、协议、以后的编译器）只产出格式 1。升级是确定性的：

- **Program**：`upgrade_program_v0` 把格式 0 Program 变为格式 1 Program 加一个本地内容包。
  - 被呈现操作数引用的每段文字 T 变为引用 `{ provider: "local", key: "stage5-" ‖
    hex(digest_bytes("narrata.stage5-text", 1, UTF-8(T)) 的前 16 字节) }`。同一文字总得到同一
    引用，所以多个构件的内容包可以合并而不冲突；两段不同文字落到同一 key 时升级失败。
  - `speaker`、`prompt`、`label` 成为引用项，`text` 成为整单元段 `Segment { unit: 引用 }`。
    内容表按首次使用排序：流程按序、指令按序、操作数按 speaker、text 或 prompt、各 label 的
    顺序，相同的项只出现一次。
  - 只保留被 `Const` 引用的常量，保持原顺序并重写 `Const` 下标；只供呈现的文字常量因此离开
    构件。ID、流程、全局量、能力、Statechart、`ProgramId` 与语义版本不变。
  - 内容包是 [ADR 0013 §9](0013-r2-text-free-node-format.md#9-本地内容方) 的本地内容包 JSON
    （`format_version: 1, provider: "local"`），文字中的 `{`、`}` 写成 `{{`、`}}`，语言由调用方
    给出，缺省 `und`。
- **存档**：用 ADR 0009 的迁移机制，而不是另写一条重写路径。`format_upgrade_migration` 给出
  从格式 0 构件到其升级结果的描述符：空的重定位表、只接受 schema 0、没有恢复点，`MigrationId`
  取 `digest_bytes("narrata.stage5-format-upgrade", 1, from ‖ to)` 的前 16 字节。
  `migrate_checked_state` 按目标 Program 的格式重建待处理交互与 scene：交互 ID 与 Effect ID
  由记录的父状态摘要、输入摘要与起源计算，保持不变；不使用 scene 的 Program 得到 `None`，
  若源状态的 scene 不是默认值则拒绝。迁移不允许降格式（格式 1 到格式 0）。
- **提交与 bundle**：`apply_migration` 写入格式 1 Program、schema 1 Snapshot 与 cause 为
  `Migration` 的提交，父提交是原来的格式 0 提交。之前的提交保持原样，可读、可在格式 0 上
  继续或再次升级。bundle 先导入、再升级、再导出。
- **CLI**：`narrata upgrade program` 写出格式 1 Program 与内容包；`narrata upgrade dry-run|apply`
  在 SQLite 存档上报告或执行上述迁移。

### 6. 存储注册

`narrata-store` 的注册（ADR 0015 的 `Registry`）列出 Program 与 Snapshot 支持的 schema（0、1），
写入时拒绝其他 schema；协调器、调试器与迁移按 Program 格式和状态的 schema 写对象，不再写死 0。
种类、键空间与布局版本不变。

### 7. 冻结语料

`fixtures/compat/stage6-v0/` 冻结与 `stage5-v0` 同一组对象（Program、Snapshot、存储回执、提交、
Checkpoint Bundle、时间线归档）、清单、预期的待处理交互与示例内容包。冻结测试证明以后的版本
能逐字节读取它们、导入 bundle 并运行到结束；`stage5-v0` 的测试另外证明它能升级并在格式 1 上
运行到结束。

## 后果

- 宿主必须解析内容引用才能显示文字；`narrata-content-local` 能直接解析升级得到的内容包。
- 格式 0 留在内核中只为读取与继续旧存档；新的能力只加在格式 1 上（决定 11 也限制它）。
- 绑定（.NET、TypeScript、Unity）需要与原生库同时升到 ABI 2。
- Stage 1–5 的内容解析辅助（`ExternalContentResolver`、`CheckedContentResolution`）不受影响：
  会影响分支的内容仍作为 Recorded Query Effect 进入回执。
