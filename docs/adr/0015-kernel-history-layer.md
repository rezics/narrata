# ADR 0015：kernel 历史层与领域注册的会话

状态：Accepted。建立在 [ADR 0012](0012-narrow-storage-backend-contract.md) 的后端契约与
[ADR 0014](0014-save-engine-key-layout.md) 的存档引擎之上；ADR 0014 的键空间、编码、信任边界
与 GC 栅栏全部保留，本 ADR 只把其中与领域无关的部分移进 kernel，并为新领域增加一种通用提交。

## 背景

ADR 0014 的引擎写在 `narrata-store` 里，种类、边、索引与根都按 Stage 1–5 的
`ObjectKind` 硬编码。节点栈（[ADR 0013](0013-r2-text-free-node-format.md)）需要同样的
内容寻址对象、Ref、GC 和 Checkpoint Bundle，但它的提交是 `{artifact, parent, input, state,
depth}`，没有回执、目录与 Effect 账本；把它塞进 `narrata-store` 会让节点栈依赖 Stage 1–5
的全部领域类型，复制一份又会有两套 GC 与校验。

## 决策

### 两层

`packages/narrata/kernel/crates/narrata-history` 是历史层，只依赖 `narrata-kernel` 与
`narrata-storage`，可编译到 `wasm32-unknown-unknown`，没有 async，没有 `dyn`。

| 历史层（`narrata-history`） | 注册方（例如 `narrata-store`） |
| --- | --- |
| 读对象时重算摘要（`Object::verify`），写对象时按注册表检查种类 | 声明种类、边与写入时的语义检查 |
| touch 键、`meta/graph` 与 `meta/sweep` 栅栏、按引用顺序分批写入 | 自己键空间里的根与索引键，由规划闭包放进同一批次 |
| Ref、Pin、children 与 transitions 索引 | 命名边的解析索引（Stage 1–5 的 programs） |
| GC 标记与拓扑删除、完整性扫描、结果未知时的回读 | GC 的额外根与删除对象时要删的索引键 |
| Checkpoint Bundle 容器与清单（kind 7）、闭包、导入校验 | 自己的 bundle 种类（Stage 1–5 的时间线归档）复用容器 |
| 通用提交格式、`Domain` 注册与会话 API | 领域状态与输入的编码和受检解码 |

`narrata-store` 现在是 `History<B, Legacy>` 加上 Stage 1–5 的注册：kinds 1–11 的边与语义、
目录、归档、复合存档、输入、Effect 账本与 fence、目录操作、按回合的提交索引和
`SessionCoordinator`。它原有的 GC、对象校验、Ref/Pin 编解码与 bundle 容器已删除；公开的
`SaveStore` 接口、错误与冻结语料不变。Ref 名称、`RefKey`、`BranchId`、分页、保留策略与 GC 报告
从历史层再导出；执行的分支 Ref 由 `timeline_branch(execution, branch)` 构造。

### 注册表

注册表是静态类型（`Registry: Sized`），引擎直接调用：

- `kind(code) -> Option<KindInfo>`：名称、是否叶子（不引用其他对象）、是否可作 Ref 目标。
- `references(object)`：对象引用的对象，`Reference::Object { id, kind, descriptor }` 或按名称的
  `Reference::Named { kind, name }`（Stage 1–5 提交按 artifact 引用 Program）。
- `name`、`resolve`：命名对象的名称与解析索引；索引只是提示，引擎读出对象后核对名称。
- `validate(object, view)`：引擎已确认引用存在且种类正确之后的语义检查，返回要写的索引键。
  `View` 先看本批次再看库，缓存读取与解析，`planned`/`plan` 让同批次的对象看见彼此的索引键。
- `spaces`、`roots`、`unindex`、`conflict`：注册方的键空间、额外的 GC 根、删除对象时的索引键、
  以及根键前置条件失败时的错误。

kind 7（Checkpoint Bundle 清单）属于历史层，注册表不会被问到它。

### 通用提交与 `Domain`

一个领域实现静态 trait `Domain`：提交、状态、输入的 kind 与 schema，`artifact_id()`，状态与输入
的规范编码，以及对照构件的受检解码。kernel 从不调用领域的执行逻辑。

```text
Commit = {0: artifact bstr32, 1: parent bstr32 | null, 2: input ObjectId | null,
          3: state ObjectId, 4: depth uint}
CommitId = object_id(COMMIT_KIND, 1, payload)
```

根提交没有 parent 与 input、深度为 0；其余提交两者都有、深度为父提交加一，其他形状解码失败。
身份不含执行 ID：同一构件、同一状态和输入得到同一提交，跨会话共享。

写入时引擎检查：父提交存在、种类相同、artifact 相同、深度加一，state 与 input 引用存在且种类
与 schema 正确，并写入 children 与 transitions 索引键。状态与输入的内容由会话 API 在写入前
检查：编码、受检解码、再编码必须逐字节相同，交还调用方的是解码出的值。

### transitions 索引与会话

| 空间 | 名称 | 键 | 值 |
| ---: | --- | --- | --- |
| 14 | transitions | 父提交（32）‖ 输入（32） | `{0: commit}` |

transitions 键与提交在同一批次写入（前置条件 Absent，冲突时重新规划），GC 删除提交时一并删除
仍指向它的键；它可以由提交载荷重建，不是新的真相来源。同一 (父提交, 输入) 已记录另一个提交时，
写入报告 `Nondeterministic { parent, input, recorded, proposed }`：无论来自本机的另一次推进，
还是导入的 bundle。

会话是 Ref `active/<name>/cursor` 加分支头 `branches/<name>/<branch>`：

- `create` 写入根提交与第一个分支。
- `advance(expected_head, input, step)` 先查 transitions：已记录就复用，不运行 `step`；否则用
  解码出的父状态与输入运行调用方的 `step`。从所选分支的头推进时移动该分支，从别处推进时在新
  提交处开分支，分支 ID 是 `sha256("narrata-branch\0" ‖ commit)` 的前 16 字节，同一分叉总得到
  同一分支。游标与分支都以读到的修订号做 CAS；冲突原样报告，kernel 不重试。
- `checkout`、分页的 `children`、存档槽的 `save` 与 `load_save`、`export` 与 `import`。
- 打开会话或检出某个分支头时选中该分支；检出其他提交时保留原选择。

### 恢复的信任边界

恢复不重放。加载读一个提交和一个状态：重算摘要、核对种类与 schema、核对 artifact，再用领域的
受检解码构造状态，读取量与历史深度无关。这证明状态对该构件良构，**不证明它可经游玩到达**；
需要时调用可选的 `verify_path`，用调用方的 `step` 从根重放并逐个比较状态摘要。

导入 bundle 时，容器先按清单核对每个对象的摘要、种类与大小（种类须已注册），再核对闭包与清单
一致；写入前用导入方的领域检查根提交的 artifact 与根状态的解码；随后每个对象经过与普通写入相同
的检查。被篡改的字节因此在写入任何东西之前失败；bundle 里非根的状态在被加载时才解码。

### 种类分配

| 范围 | 用途 |
| --- | --- |
| 1–11 | Stage 1–5，注册方 `narrata-store`；其中 7 是历史层的 Checkpoint Bundle 清单 |
| `0x0100–0x01FF` | 节点栈（ADR 0013）：`0x0100` 构件清单、`0x0101` 程序块、`0x0102` 墓碑集、`0x0103` 名字表、`0x0104` 打包容器、`0x0110` 提交（本 ADR 的通用格式）、`0x0111` State、`0x0112` Input、`0x0120–0x013F` 发布时分析 |
| `0xFF00–0xFFFF` | 测试与一致性领域，产品存储从不注册；计数器测试领域用 `0xFF10`–`0xFF12` |

其余未分配，新的分配由 ADR 决定。

### 键空间与布局版本

历史层拥有空间 0、1、2、7、12 和新的 14；Stage 1–5 注册方拥有 3–6、8–11、13。新的空间用新
编号，注册方通过 `spaces()` 声明自己的空间，使"库是否为空"的判断覆盖它们。

布局版本仍为 1：ADR 0014 的空间与编码一字未改，新增的只有空间 14 与新的种类。ADR 0014 的引擎
读到未知种类的对象时报告损坏，GC 在删除任何东西之前失败，所以旧版本打开新库是失败关闭的。

### GC 与未注册的种类

GC 不知道未注册种类的对象引用了什么，删除它独自保活的对象会丢数据。因此：标记时遇到未注册
种类、或待删除的候选中有未注册种类，GC 报告 `UnregisteredKind` 并不删除任何东西；完整性扫描把
它们列为问题；写入与 bundle 解码拒绝未注册种类。不同注册方共用一个库时，必须由同时注册双方种类
的一个注册表打开它才能回收。

### 冻结语料

`fixtures/compat/kernel-history-v1/`：计数器测试领域的一个会话（主线、分支、存档槽、Pin、
children 与 transitions 键）的 SQLite 库、该会话分支头的 Checkpoint Bundle，以及列出全部对象与
键的清单。生成器是确定的，两次运行得到相同摘要；测试保证以后的版本能打开、加载、复用转换并继续
这个会话，并能受检导入这个 bundle。

## 后果

- Stage 1–5 的读取计数、批次键顺序、错误类型与冻结语料不变；`narrata-store` 只剩注册与领域逻辑。
- 一个新领域只需实现 `Domain`（和需要时的注册表组合），就得到提交、去重、分支、存档槽、GC、
  完整性扫描与 Checkpoint Bundle，且加载不随深度增长。
- 状态与输入在写入时只检查种类与 schema；领域内容在会话 API 写入前与每次加载时检查。绕过会话
  API 直接写入的不良状态能存入库，但加载与导入会拒绝它。
- `narrata-core` 与历史层各有一个 `ObjectId` 类型，`narrata-store` 在边界按字节转换。

## 推迟

节点栈的注册与 Effect 账本、通用领域的完整时间线目录、构件变更时的迁移提交、通用领域的回执、
浅存档、宿主批次协议与 async、协议引擎的存储注入、多个注册方在同一库中的组合。
