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
- 节点正文、局部回应是 `Segment`；选项文字、禁用原因、段落标题、产品与图的标题、共享变量的
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
- 删除留墓碑：包源稿的 `tombstones` 列出已删除的 ID。工具在 `compose` 时与上一次的构件比较，
  为消失的 ID 追加墓碑并报告；`--locked` 时需要追加即失败。存活 ID 与墓碑重合是编译错误。
  构件清单引用全作品的墓碑集；`lookup(id)` 返回存活位置、`deleted` 或 `unknown`，供迁移与诊断
  使用。
- 动态结构的 ID 由运行时派生：取 `digest_bytes("narrata.nodes.proposal-id", 1, 父提交 ‖ u16 序号)`
  的前 16 字节，并按 UUIDv8 设置版本与变体位。派生 ID 记录在 Input 中，解码时复核。

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

- 选择点按文档顺序排列。进入段落时呈现 `body.first … 第一个选择点的 placement`（无选择点时到
  `body.last`）；在选择点 k 选择后依次执行所选选项的效果，呈现各自的 `reply`，再呈现
  `rejoin … 选择点 k+1 的 placement`，到达下一个选择点；分支结果进入目标节点。最后一个选择点
  之后的流程（或没有选择点的段落）继续到 `next`；可能走到段落末尾却没有 `next` 是编译错误。
- 只有最后一个选择点的局部选项可以省略 `rejoin`，表示继续到正文末尾。局部结果要求段落有
  `body`，`reply` 必须与 `body` 在同一内容单元。
- 多选（`max > 1`）的选项必须全部是局部结果并共享同一 `rejoin`；效果与回应按选项顺序执行，
  整个选择是一个 Input。`0 ≤ min ≤ max ≤ 选项数`，`min = 0` 只允许全部为局部结果的选择点。
  可见且可选的选项少于 `max(min, 1)` 时运行时报 `no_actions`。
- 有两个以上选项的选择点，每个选项都必须有 `label`。
- R1 的 `content` 节点等价于一个只有单个分支选项的选择点，`decision` 等价于末尾一个全部为分支
  选项的选择点。
- 编译器看不到文档，锚点顺序只能在有**内容大纲**时检查：内容方提供不含正文的
  `{ 单元: { blocks: [AnchorId], markers: { AnchorId: ChoicePointId } } }`，编译时据此报告锚点不存在、
  顺序错误、回应不在 placement 与 rejoin 之间、标记块与 `placement` 不一致或重复。这些是诊断
  而不是错误：内容方可能稍后修正。没有大纲时这些检查跳过，解析时宿主得到 `incompatible`。

### 5. 动态选项与节点

`proposals = true` 的选择点接受宿主提议。提议是一种 Input：

```text
Input = choose  { choice_point, options: [OptionId] }           // 按选择点内的顺序，不重复
      | propose { choice_point, options: [OptionDraft], nodes: [PassageDraft] }
```

- 草稿与作者结构同形但不含 ID；引擎按编译期规则校验（类型、引用、表达式、局部结果规则），
  分支目标只能是本图已有节点或同一提议中的节点。
- 通过后派生 ID，结构写入当前栈帧的叠加层（overlay），交互仍停在同一选择点；之后照常选择。
  栈帧返回时叠加层随之消失。每个提议最多 16 个选项与 16 个节点，每个栈帧最多 256 个动态节点。
- 回退、恢复与迁移使用 Input 中记录的结构，从不重新生成。

### 6. 会话对象

```text
State  = { shared: {名字: Scalar}, frames: [Frame], next_instance: u32,
           finished?: { node, instance, outcome } }
Frame  = { graph: GraphRef, node: NodeId, at?: ChoicePointId, instance: u32,
           parameters, locals, overlay? }
Commit = { artifact: [u8;32], parent?: CommitId, input?: ObjectId, state: ObjectId, depth: u64 }
```

- State 只含 ID 与标量，不含任何文字；顶层栈帧在交互处必有 `at`。`decode_state` 是构造 State 的
  唯一途径：规范 CBOR 往返，并对照构件校验每个 ID、类型、栈帧与叠加层。
- 提交与 kernel 的节点会话提交 API 一致：提交身份是上面载荷的 object id，不含执行 ID；根提交
  没有 `parent` 与 `input`，深度为 0。相同父提交与相同 Input 复用既有提交。
- **恢复不重放。** 恢复读入对象、复核摘要、确认构件相同，再用 `decode_state` 校验。这证明状态
  对该构件良构，不证明它可经游玩到达；需要时用可选的 `verify_path` 从根重放审计。
- `ExecutionId`（16 字节，宿主在新会话开始时铸造 UUIDv7）属于会话或引用的元数据。
  呈现键 `PresentationKey = (ExecutionId, CommitId, occurrence)`，`occurrence` 是该提交呈现项的
  序号。
- 在 kernel 的节点会话存储落地之前，会话导出使用临时 JSON 容器
  `{ format_version: 2, artifact_id, execution, cursor, objects: [十六进制信封] }`，保留 R1 的
  512 个提交与 2 MiB State 上限。容器里的对象与 kernel 存储的对象逐字节相同，切换时只换容器。

### 7. 呈现与 view

运行时只输出引用。view（JSON）给出：

- `presentation`：自上一个交互以来要显示的项，`{ occurrence, role: title|body|reply, node,
  content: Segment|ContentRef, args }`；某个提交的呈现由父 State 与 Input 确定性重算，不存储。
- `interaction`：`choose { choice_point, min, max, options: [{ id, key?, label?, enabled, reason?,
  outcome }] }` 或 `finished { outcome, title?, body? }`；结局显示由清单的 `endings` 提供。
- 共享变量（含 `label?: ContentRef`）、栈帧与历史。别名仅在加载名字表时附带。

### 8. 构件：清单、程序块与打包

- **清单**：产品（`id`、`title?`、入口图、入口参数、共享变量的初值与显示名、`endings`、绑定）、
  包实例（别名、包 ID、版本）、每个图的签名、是否导出与所在块序号、块的 object id 列表、节点类型
  修订号、墓碑集 object id。
- **程序块**：同一包实例的一个或多个图，含图头（参数、局部初值、使用的共享变量、导入签名、
  结局集合、入口节点、标题）与按 `NodeId` 排列的检查后节点计划。调用只经清单中的签名跨块，
  分支只在图内，因此每个块对照清单即可独立完成受检解码。
- 运行时按需加载块：只持有清单与当前栈帧所在的块，块经内容方式（object id）校验后才使用。
  默认每个图一个块，相邻的小图可以合并；块的划分是编译策略，改变它不改变语义，但会改变
  `artifact_id`。
- **打包容器**：单文件分发（示例作品、本地游戏）把清单、全部块、墓碑集与可选的名字表放在
  一个信封中；分发身份仍是清单的 `artifact_id`。
- **上限**：取消全作品 4,096 节点与 4 MiB 上限，改为单个源稿包 16 MiB、单图 4,096 节点、
  单块与清单各 4 MiB、每段落 64 个选择点、每选择点 64 个选项、每段落 32 个参数、4,096 个共享
  变量；State 128 KiB、每个 Input 自动执行 4,096 步、调用深度 64 不变。
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
  抽出全部文字（含默认"继续"、"条件尚未满足"与结局页），把模板改写为命名参数，铸造 ID，
  `content`/`decision` 改写为段落。
- R1 存档迁移按别名把 R1 的动作在迁移后的 R2 构件上重放，并逐个比较共享变量、栈帧位置、参数与
  局部值，一致后输出 R2 会话导出。R1 存档只能对应它自己的 R1 构件（`artifact_id` 必须匹配）。
- 冻结语料：`fixtures/compat/nodes-r1/` 保存 R1 示例作品的源稿、bundle、lock、分析输出与存档，
  作为迁移测试的输入；`fixtures/compat/nodes-r2/` 在 R2 实现完成时冻结打包构件、会话导出、
  本地内容包与内容大纲。

## 后果

- 改正文、改译本、改别名都不改变 `artifact_id`、提交或 State 的摘要；属性测试证明这一点。
- 阅读器必须自己解析引用（经本地内容方）；阅读器外壳与渲染插槽由 Goal `web-and-rezics` 完成。
- 存档恢复从"整段重放"改为"受检解码"，接入 kernel 的提交 API 时只换容器，不换对象字节。
- 块以图为单位，单图上限 4,096 节点；如果基准显示需要切分单个图，另写 ADR。
- 本 ADR 不规定发布时分析的输出格式，只保留 kind code。
