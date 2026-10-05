# ADR 0013：R2 不含正文的节点格式、身份与选择模型

状态：Accepted（2026-10-05）。取代 [ADR 0011](0011-r1-node-composition.md) 的作者格式、内容呈现、
构件身份与保存格式；ADR 0011 的组合、调用、作用域与表达式语义保留。R1 只经迁移读取。

## 背景

[决定 2–6、10](../product/decisions.md) 要求节点栈不含正文、使用 16 字节身份、支持两类选择与
动态结构，并按块切分到 10 万选择点。R1 与这些要求冲突：

- 包内 `content` 表、`label`、`disabled_reason`、图与产品标题、`shared_labels` 都是内联文字，
  `{{scope.name}}` 模板在运行时渲染；运行时硬编码了结局页、默认"继续"与"条件尚未满足"。
- 节点与选项以作者名字为身份，改名即换身份；没有墓碑。
- 选项只能进入另一个节点（分支结果），没有单元内汇合与多选。
- 整包是一个 JSON，全局上限 4,096 节点、4 MiB；运行时在加载时重新编译整部作品。
- 摘要是 SHA-256 加排序 JSON，与 kernel 的 [ADR 0003](0003-deterministic-cbor-profile.md)
  编码不一致，存档只能在同一构件上整段重放。

## 决定

### 1. 三种编码

| 用途 | 编码 | 身份 |
| --- | --- | --- |
| 作者源稿（项目清单、包） | JSON，拒绝重复键与未知字段 | 只用于 lock 的源稿摘要 |
| 构件与会话对象（清单、程序块、墓碑集、State、Input、提交） | ADR 0003 规范 CBOR，经 `narrata-kernel` 编解码 | `object_id(kind, schema, payload)` |
| 与宿主交换（view、解析请求、诊断、分析摘要） | JSON，由 Rust 类型生成 schema | 无；不参与任何摘要 |

节点对象的 kind code 取 kernel 为节点栈保留的 `0x0100–0x01FF` 段，schema 均从 1 开始：

| kind | 对象 |
| --- | --- |
| `0x0100` | 构件清单（manifest）；其 object id 就是构件身份 `artifact_id` |
| `0x0101` | 程序块（chunk） |
| `0x0102` | 墓碑集 |
| `0x0103` | 名字表（作者别名，不参与构件身份） |
| `0x0104` | 打包容器（单文件分发） |
| `0x0110` / `0x0111` / `0x0112` | 提交 / State / Input |
| `0x0120–0x013F` | 保留给发布时分析的输出 |

CBOR map 只用递增的无符号整数键（ADR 0003）；字段编号由实现中的编解码器与冻结语料固定，
本 ADR 只规定逻辑形状。

### 2. 内容引用

`ContentRef { provider, key }` 与 `Segment { unit: ContentRef, first?: AnchorId, last?: AnchorId }`
定义在 `narrata-kernel` 的 `content` 模块中，旧 Flow 栈去文本时共用同一类型。

- `provider`：1–32 字节 `[a-z0-9-]`；`key`：1–256 字节 UTF-8、无控制字符，只按字节比较；
  `AnchorId`：1–128 字节可打印 ASCII。
- 节点正文、局部回应与结局正文是 `Segment`；选项文字、禁用原因、段落标题、产品与图的标题、共享变量的
  显示名、结局标题是 `ContentRef`。全部可选的位置由宿主决定回退显示（例如自己的"继续"）。
- 新增标量类型 `ref`，值为 `ContentRef`，只支持相等比较。可译的名字（如子图参数里的角色称呼）
  用它传递，不再用 `text` 字面量。`text` 标量保留给参与规则判断的数据（决定 2）。
- 模板删除。段落（passage）声明 `args: { 名字: 表达式 }`，每个呈现项附带求值后的参数；
  宿主的内容方按名字格式化（Fluent/ICU 式的命名参数），`ref` 参数由宿主一并解析。

### 3. 身份

- `NodeId`、`ChoicePointId`、`OptionId` 是 16 字节作者身份，文本形式沿用 ADR 0002 的
  `node:`/`choice-point:`/`option:` 加 32 位小写十六进制，写在源稿的 `id` 字段中。
- 工具（`narrata-book ids`）为缺少 `id` 的条目铸造 UUIDv7 并写回源稿；`compile` 从不铸造，
  缺失、格式错误或在整部作品内重复的 ID 都是错误。同一包不能实例化两次。
- 节点在图 `nodes` 中的键、选择点的 `key`、选项的 `key` 是作者别名，规则同 R1 标识符。源稿中
  的边仍按别名书写，编译后全部变成 ID。别名进入名字表，不进入清单与程序块：改别名不改变
  `artifact_id`，只改变源稿摘要与 lock。图名与包实例别名是组合结构，属于构件身份。
- 删除留墓碑：包源稿的 `tombstones` 列出已删除的 ID。存活 ID 与墓碑重合是编译错误。
- `compose` 与上一次的构件比较：
  - 为消失的 ID 追加墓碑并报告；
  - 墓碑只增不减，删掉旧墓碑是错误；
  - 存活 ID 的归属（节点所在图、选择点所在节点、选项所在选择点）改变也是错误：移动或复制
    必须换新 ID（决定 3）。

  `--locked` 时需要追加墓碑即失败；没有上一次构件时这些比较跳过并报告。
- 构件清单引用全作品的墓碑集。工具侧的 `lookup(id)` 扫描构件，返回存活位置、`deleted` 或
  `unknown`，供迁移与诊断使用；运行时的寻址始终是 `(GraphRef, NodeId)`，不需要全局索引。
- 动态结构的 ID 由运行时派生（§5），记录在 Input 中，解码时复核。

### 4. 节点词汇与选择模型

检查后的执行词汇为 `passage`、`branch`、`mutate`、`call`、`return`；后四种语义同 R1，
`passage` 取代 R1 的 `content` 与 `decision`。作者节点类型 `narrata.*` 的语义修订号升为 `2`。

```text
Passage     = { title?: ContentRef, body?: Segment, args, choice_points: [ChoicePoint], next?: NodeId }
ChoicePoint = { id, key, placement?: AnchorId, min = 1, max = 1, options: [Option], proposals = false }
Option      = { id, key, label?: ContentRef, visible_if?, enabled_if?, reason?: ContentRef,
                effects: [Assignment],
                outcome: local { reply?: Segment, rejoin?: AnchorId } | branch { target: NodeId } }
```

- **正文切片与段。** 正文是 `body.unit` 中从 `body.first`（省略时为单元开头）到 `body.last`
  （省略时为单元末尾）的块，`Segment` 两端都包含在内。设 `P(k)` 为选择点 k 的 `placement`；
  只有最后一个选择点可以省略它，省略表示放在 `body.last` 之后。
- **呈现顺序。**
  - 进入段落时先呈现 `title`，再呈现首段：有选择点时为 `[body.first, P(0)]`，没有选择点时为
    整个正文。没有 `body` 的段落没有首段。
  - 在选择点 k 选择分支结果时，进入目标节点。
  - 选择局部结果时，依次呈现所选选项的 `reply`，再呈现尾段：下一个选择点存在时为
    `[rejoin, P(k+1)]`，否则为 `[rejoin, body.last]`。然后到达下一个选择点，或走到段落末尾。
  - `rejoin` 省略表示尾段为空。中间的选择点省略它，就直接到达下一个选择点；最后一个选择点
    省略它，就不再呈现正文。
- **正文内的顺序约束。** 按文档顺序：`body.first ≤ P(0)`，每个 `reply` 落在 `P(k)` 与 `rejoin`
  之间（两端都不含），`rejoin ≤ P(k+1)`，且所有块都在正文切片内。这保证未选的回应永远不会被
  呈现。局部结果要求段落有 `body`，`reply` 必须与 `body` 在同一内容单元。
- 段落走到末尾（没有选择点，或最后一个选择点做了局部选择）后继续到 `next`。可能走到末尾却
  没有 `next` 是编译错误；反过来，最后一个选择点只有分支结果、走不到末尾时声明 `next` 也是
  编译错误。
- **基数。** `1 ≤ max ≤ 选项数`，`0 ≤ min ≤ max`。多选（`max > 1`）或允许不选（`min = 0`）时，
  所有选项都必须是局部结果并共享同一个 `rejoin`（或都省略）；不选就走这个 `rejoin`。
- **可用性。** 可见且可选的选项少于 `min` 时，运行时报 `no_actions`。`min = 0` 且没有可用选项
  时自动跳过：效果等同于选择空集，不产生交互。
- **求值时机。** 选择的每个选项都按父 State 校验可见与可选；然后按选项顺序执行全部效果；
  再按选项顺序呈现回应，参数按执行完效果后的 State 求值。任何一步失败，整个 Input 撤回。
- 有两个以上选项的选择点，每个选项都必须有 `label`。
- R1 的 `content` 节点等价于一个只有单个分支选项的选择点，`decision` 等价于末尾一个全部为分支
  选项的选择点。
- **内容大纲。** 编译器看不到文档，正文内的顺序约束只能在有内容大纲时检查。大纲由内容方提供，
  不含正文：`{ 单元: { blocks: [AnchorId], markers: { AnchorId: ChoicePointId } } }`。内容方能
  提供大纲时，`compose` 必须检查，报告锚点不存在、违反顺序约束、标记块与 `placement` 不一致
  或重复。这些是诊断而不是错误：内容方可能稍后修正。没有大纲时这些检查跳过；运行时宿主解析器
  遇到锚点缺失或 `first` 在 `last` 之后的段，返回 `incompatible`。

### 5. 动态选项与节点

`proposals = true` 的选择点接受宿主提议。

```text
Input = choose  { choice_point, options: [OptionId] }           // 按选择点内的顺序，不重复
      | propose { choice_point, options: [Option], nodes: [Passage] }  // 已校验、已派生 ID
```

- **请求与记录分开。** 宿主提交的是不含 ID 的提议请求（JSON），其中已有节点按 `NodeId` 引用，
  同一请求中的新节点按请求内的临时键引用。引擎按编译期规则校验类型、引用、表达式与局部结果
  规则，分支目标只能是本图已有节点（含本栈帧先前提议的节点）或同一请求中的节点。通过后派生 ID、
  解析临时键，写成上面的 `propose` Input，新节点保持请求中的顺序。记录的结构不含临时键或别名，
  所以别名不会进入 Input 或 State 的摘要。
- **派生 ID。** 取 `digest_bytes("narrata.nodes.proposal-id", 1, 父提交 ‖ 请求的规范摘要 ‖ u16 序号)`
  的前 16 字节（序号为大端），按 UUIDv8 设置版本与变体位。请求的规范摘要是
  `digest_bytes("narrata.nodes.proposal-request", 1, 规范 JSON)`，规范 JSON 里新节点的临时键与
  指向它们的引用都换成节点在请求中的序号，因此能从记录的 Input 重算。序号按一次遍历分配：先是
  追加的选项，再是新节点，每个节点内先选择点、后其选项，都按请求中的顺序。与当前栈帧的图、
  墓碑集或状态中任一叠加层已有的 ID 碰撞时拒绝。`decode_input` 用父提交重算并核对全部派生 ID。
- **追加语义。** 新选项追加在该选择点现有选项（静态的在前，之后按提议顺序）之后，新节点写入
  当前栈帧的叠加层（overlay），追加的选项也按选择点记在叠加层中；交互仍停在同一选择点，之后
  照常选择。校验针对追加后的完整选择点与段落：选项数上限、基数规则、局部结果规则、段落末尾
  规则。栈帧返回时叠加层随之消失。
- **上限。** 每个请求最多 16 个选项与 16 个节点，至少一个选项或节点；每个栈帧最多 256 个动态
  节点。`proposals = true` 的选择点总是停下等待交互：没有可用选项时不报 `no_actions`，`min = 0`
  时也不自动跳过，以便宿主提议。view 的交互带 `proposals` 标记。
- 回退、恢复与迁移使用 Input 中记录的结构，从不重新生成。

### 6. 会话对象

```text
State  = { shared: {名字: Scalar}, frames: [Frame], next_instance: u32,
           finished?: { node, instance, outcome } }
Frame  = { graph: GraphRef, node: NodeId, at?: ChoicePointId, instance: u32,
           parameters, locals, overlay? }
Commit = { artifact: [u8;32], parent: CommitId | null, input: ObjectId | null, state: ObjectId, depth: u64 }
```

- State 只含 ID 与标量，不含任何文字与别名。`decode_state` 是构造 State 的唯一途径：先做规范
  CBOR 往返，再对照构件校验以下不变量。
  - 已结束时栈为空；未结束时栈非空，顶层栈帧的节点是段落，`at` 是它（含叠加层）的选择点。
  - 非顶层栈帧停在 `call` 节点上，该调用的目标解析为上一层栈帧的图。
  - 实例号沿栈严格递增，且都小于 `next_instance`。
  - 参数、局部与共享变量的键集合和类型都与声明一致。
  - 叠加层只引用本图节点或叠加层自身的节点。
  - 每个 ID 都存在，且不是墓碑。
- 提交与 kernel 的节点会话提交 API 一致：提交身份是上面载荷的 object id，不含执行 ID；根提交
  的 `parent` 与 `input` 为 null，深度为 0，非根提交两者都有，深度为父提交加一。相同父提交与相同
  Input 复用既有提交。根提交的 State 是从清单的初始状态确定性推进到第一个交互后的结果。
- **恢复不重放。** 恢复读入对象、复核摘要、确认构件相同，再用 `decode_state` 校验。这证明状态
  对该构件良构，不证明它可经游玩到达；需要时用可选的 `verify_path` 从根重放审计。
- `ExecutionId`（16 字节，宿主在新会话开始时铸造 UUIDv7）属于会话或引用的元数据。
  呈现键 `PresentationKey = (ExecutionId, CommitId, occurrence)`，`occurrence` 是该提交呈现项的
  序号。
- 新导出使用 [kernel Checkpoint Bundle](0015-kernel-history-layer.md)，文本接口以十六进制
  传递原始 bundle 字节；包含当前提交及其祖先，其他分支与存档槽留在存储后端。ExecutionId
  是宿主会话元数据，不在 checkpoint 中；导入当前会话时保留该会话的 ExecutionId。
- 临时 JSON 容器 `{ format_version: 2, artifact_id, execution, cursor, objects: [十六进制信封] }`
  只读。导入先复核旧对象、状态与输入，再把提交改写为 kernel 的五字段载荷，从根开始重映射
  提交身份、父引用与游标；State 与 Input 信封逐字节保留。旧容器的 512 提交与 2 MiB State
  上限只约束此兼容读取路径，不约束 kernel 会话。

### 7. 呈现与 view

运行时只输出引用。view（JSON）给出：

- `presentation`：自上一个交互以来要显示的项，`{ commit, occurrence, role: title|body|reply,
  node, content: Segment|ContentRef, args }`，`(execution, commit, occurrence)` 即呈现键。某个提交的呈现不存储，而是确定性重算：非根提交由父 State
  与 Input 重算，根提交由清单的初始状态推进到第一个交互时重算，`occurrence` 的编号方式相同。
- `interaction`：`choose { choice_point, min, max, args, options: [{ id, key?, label?, enabled,
  reason?, outcome }] }` 或 `finished { outcome, title?, body? }`。`args` 是段落参数在交互时的
  值，选项文字与禁用原因用它格式化。结局显示由清单的 `endings` 提供。
- 共享变量（含 `label?: ContentRef`）、栈帧与历史。别名仅在加载名字表时附带。

阅读器的 book view 另给出 `page`：进入当前段落（或结束故事）那一步的呈现，加上此后在该段落内
各次局部选择的呈现。一页跨多个提交，所以每个呈现项都带自己的 `commit`。

### 8. 构件：清单、程序块与打包

- **清单**：产品（`id`、`title?`、入口图、入口参数、共享变量的初值与显示名、`endings`、绑定）、
  包实例（别名、包 ID、版本）、每个图的签名、是否导出与所在块序号、块的 object id 列表、节点类型
  修订号、墓碑集 object id。
- **程序块**：同一包实例的一个或多个图，含图头（参数、局部初值、使用的共享变量、导入签名、
  结局集合、入口节点、标题）与按 `NodeId` 排列的检查后节点计划，不含别名。调用只经清单中的
  签名跨块，分支只在图内，因此每个块对照清单即可独立完成受检解码，证明块内的不变量（引用、
  类型、调用签名、段落规则）。全作品的 ID 唯一性与墓碑由编译保证；打开完整构件的工具可以用
  `verify_artifact` 复核。运行时按 `(GraphRef, NodeId)` 寻址，不依赖全局唯一性。
- 运行时按需加载块：只持有清单与当前栈帧所在的块，块经内容方式（object id）校验后才使用。
  默认每个图一个块，相邻的小图可以合并；块的划分是编译策略，改变它不改变语义，但会改变
  `artifact_id`。
- **打包容器**：单文件分发（示例作品、本地游戏）把清单、全部块、墓碑集与可选的名字表放在
  一个信封中；分发身份仍是清单的 `artifact_id`。
- **上限**：取消全作品 4,096 节点与 4 MiB 上限，改为单个源稿包 16 MiB、单图 4,096 节点、
  单块与清单各 4 MiB、每段落 64 个选择点、每选择点 128 个选项、每段落 256 个参数、4,096 个共享
  变量（选项与参数上限不低于 R1，迁移的作品不会超限）；State 128 KiB、每个 Input 自动执行 4,096 步、调用深度 64 不变。
- lock（项目清单旁的 `project.lock.json`，`format_version: 2`）记录 `artifact_id`、各包的源稿
  摘要（`digest_bytes("narrata.nodes.package-source", 1, 规范 JSON)`）与节点类型修订号。

### 9. 本地内容方

`products/` 中的作品用本地内容包保存文字：JSON，`{ format_version: 1, provider: "local",
language, entries: { key: { text } | { blocks: [{ id, text } | { id, choice_point }] } } }`，
占位符写作 `{名字}`。参考实现（Rust，经 Wasm 也供浏览器使用）按[内容引用契约](../contracts/content-references.md)
批量解析，返回 `ok`/`unavailable`/`incompatible`，并能导出内容大纲。它是宿主侧组件，节点运行时
不依赖它。

### 10. R1 的读取与迁移

- R1 不再执行。`narrata-book migrate-r1` 把 R1 项目与包转换为 R2 源稿和一个本地内容包：
  - 抽出全部文字，包括默认的"继续""条件尚未满足"和结局页；
  - 把模板改写为命名参数，名字用变量名，跨作用域重名时加 `parameter_`/`local_`/`shared_` 前缀；
  - 铸造 ID；
  - 把 `content`/`decision` 改写为段落：R1 节点 ID 成为节点别名，R1 选项 ID 成为选项 `key`，
    `content` 节点唯一的选项 `key` 为 `continue`。
- 项目清单的 `migrated_from_r1`（R1 `artifact_id` 的 64 位十六进制）记录来源，编译后进入
  名字表，手工整理迁移结果时保留它。R1 存档只能迁移到由它自己的 R1 构件迁移而来的 R2 构件，
  两者不符时拒绝。
- 存档迁移的步骤：
  1. 按存档中的顺序（父在子前）把每个保留的 R1 提交的动作经名字表映射为 Input，在 R2 构件上
     重放，重建整棵提交树；
  2. 逐个比较共享变量、栈帧位置、参数与局部值：R1 的 `text` 值与替换它的 `ref` 按内容方原文
     语言的文字比较；只在 R2 中声明的变量必须仍是初值（R1 的游玩不可能改变它们），所以手工
     整理可以新增变量与路线；
  3. 把游标映射到对应的 R2 提交；
  4. 输出 R2 会话导出。
- 冻结语料：`fixtures/compat/nodes-r1/` 保存 R1 示例作品的源稿、bundle、lock、分析输出与存档，
  作为迁移测试的输入；`fixtures/compat/nodes-r2/` 冻结示例作品的打包构件、本地内容包与内容
  大纲，以及两份 R1 存档迁移而成的会话导出。

## 后果

- 改正文、改译本、改别名都不改变 `artifact_id`、提交或 State 的摘要；属性测试证明这一点。
- 阅读器必须自己解析引用（经本地内容方）；阅读器外壳与渲染插槽由 Goal `web-and-rezics` 完成。
- 存档恢复从"整段重放"改为"受检解码"，接入 kernel 的提交 API 时只换容器，不换对象字节。
- 块以图为单位，单图上限 4,096 节点；如果基准显示需要切分单个图，另写 ADR。
- 本 ADR 不规定发布时分析的输出格式，只保留 kind code。

## 修订

- 2026-10-05，接入 kernel 历史层：早期实现把根提交写成省略 parent / input 的三字段 map，
  这是偏离已约定 kernel API 的实现失误。统一采用五字段 map，根的两个可选值显式为 null；
  CommitId 是该载荷在 kind `0x0110` 下的 object_id。临时容器从此只读，冻结的 nodes-r2 与
  nodes-r2-proposals 不改写，导入时转换提交并保留 State / Input 字节。
  旧提议的派生 ID 使用旧父提交身份；读取与回放时从转换后的祖先载荷重建旧身份并复核该派生，
  不接受任意 ID。新提议只从当前 kernel 提交身份派生。
  阅读器以版本 3 的宿主元数据指向节点专用的 kernel 数据库，升级确认后才替换旧指针，并保留
  R1 / 临时 R2 原记录。checkpoint 的十六进制文本预算为 64 MiB，以支持数千至万次选择的
  导出；旧临时 JSON 仍按 8 MiB 读取。准备下一交互时可预取受检程序块，预取不产生提交。

- 2026-10-05，首个实现：补充原文未规定、实现必须确定的细节——`migrated_from_r1` 写在项目
  清单中（手工整理的作品也要能接收 R1 存档）、结局正文是 `Segment`（与段落正文同样按块解析）、
  R1 比较对 `ref` 与新增变量的处理（否则去文本和手工整理都会让迁移失败）、走不到的 `next` 是
  错误（死边说明作者意图与结构不符）、保留字段报 `unsupported`（未来支持时不改变已有对象的
  字节）、呈现项带 `commit` 与阅读页 `page`（一页跨多个提交，呈现键需要各自的提交）。
- 2026-10-05，实现 §5 的提议：补充原文未规定的细节并收窄一处——
  - 请求的规范摘要取规范 JSON，临时键换成序号：`decode_input` 必须能从记录的 Input 重算派生
    ID，而 Input 不含临时键；宿主怎样给临时键取名也不该改变 ID 与提交。
  - 碰撞只对照当前栈帧的图、墓碑集与状态中的叠加层，而不是整部构件：运行时按
    `(GraphRef, NodeId)` 寻址（§3），只持有用到的块（§8），对照整部构件就要为每次提议加载
    全部块；工具铸造的 ID 是 UUIDv7，与 UUIDv8 的派生 ID 不会相同。
  - 先前提议的节点算作"已有节点"，与 §6 允许叠加层引用叠加层自身的节点一致。
  - 叠加层同时记录追加的选项：被追加的选择点可能属于静态节点，追加结果必须随栈帧保留，
    重新进入该段落时仍在。
  - 接受提议的选择点在 `min = 0` 且没有可用选项时也停下：否则它被自动跳过，宿主没有机会提议。
  - 空提议被拒绝：它不改变任何可选的东西，却会产生一个提交。
