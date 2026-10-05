# REZICS 集成

日期：2026-10-05。取代 [2026-08-31 的集成边界](../../archive/2026-08-31-rezics-gamebook-integration.md)，
后者使用的 Unit、Post、Portable Text、ContentStructureNode 等名称来自旧仓库 `rezics`，
rezics-next 中已不存在。本页依据 rezics-next 的代码（`D:/rezics-repos/rezics-next`，
2026-10-04 至 05 阅读）与 REZICS 当时的 Work 分层调研结论。

Narrata 的工作不修改 REZICS 仓库（[决定 13](../product/decisions.md)）。REZICS 需要增加的
能力列在[能力请求](#能力请求)，由 REZICS 的 Goal 决定是否以及如何实施；状态变化时更新本页。

维护者于 2026-10-05 将实现拆为两个 manager：本机执行 Narrata 的
[Web 宿主 SDK Goal](../goals/web-and-rezics/GOAL.md)，另一台电脑在 REZICS 仓库执行
[REZICS 接入 Goal 交接简报](rezics-goal.md)。真实联合验证放在 REZICS Goal 的最后一个任务，
使用固定版本的 Narrata npm/Wasm 发行交付物。

## REZICS 的分层

| 层 | 承载什么 | 与 Narrata 的关系 |
| --- | --- | --- |
| Work | 评分、书评、搜索、关注、署名、作品关系 | 互动小说本身是一个 Book Work；Narrata 不引用章节级 Work |
| 文本（Expression） | 语言、不可变修订链、贡献者、字数、翻译来源；`rezics-document-v1` 块带稳定 `attrs.id` | 节点正文与局部回应以"单元 + 块锚点"引用这里的块 |
| Post | 发布人、时间、可见性、授权、回复串 | 无直接关系；作者附言不进入文本，不影响 Narrata |
| Occurrence | 结构中的位置、标签、阅读进度 | Narrata 节点引用的内容单元；移动、重排、恢复时 ID 不变，删除留墓碑 |

REZICS 正在把"章节铸成 Work"改为"章节 Post 实现 Work 的一部分文本"。Narrata 只依赖
Occurrence 与文本块 ID，这一调整对 Narrata 透明，前提是 REZICS 的解析器随之更新。

## 所有权

| 能力 | 所有者 |
| --- | --- |
| 正文、选项文字、标题、本地化、授权、审核、生命周期、canonical 页面与 SEO | REZICS |
| 章节结构（Structure、Occurrence 的顺序与分组） | REZICS |
| 节点、选择点、选项、条件、效果、去向，以及它们的 ID | Narrata |
| 叙事执行状态、提交、存档语义、迁移 | Narrata |
| 叙事图的分析、布局、作者视图与读者地图 | Narrata |
| 用户身份、存档的存放、同步与删除说明 | REZICS |

## 引用形状

```text
ContentRef { provider: "rezics", key: <Occurrence IRI> }
Segment    { unit: ContentRef, first?: <block attrs.id>, last?: <block attrs.id> }
labelRef   { provider: "rezics", key: <选项标签 ID> }
```

- 节点引用 Occurrence，不引用 Work 或 Post：同一份文本可以在多处出现，每次出现承担不同的
  叙事位置。Occurrence ID 唯一确定其 Structure，`structureId` 不再需要作为键，可以作为校验。
- 改指向（retarget）或跨 Structure 移动会产生新的 Occurrence ID，等同于删除加新增；Narrata
  的编译诊断会指出受影响的节点。
- 一个内容单元相当于一场戏：正文是扁平块序列，局部选择的回应是同一单元里的块
  （见 [选择模型](../contracts/choices.md)）。以每单元约 20 个选择计，10 万选择点约为数千个
  单元。REZICS 当前按自身量级接入即可，不需要为 30 万章节做准备。

## 选项与标记块

- 正文中可放 `narrata-choice` 标记块，只存 `ChoicePointId`，用于编辑器专门渲染和不支持
  Narrata 时的占位；翻译时原样复制。
- 选项集合、条件、效果、去向在 Narrata；REZICS 只保存选项文字。
- 选项文字有稳定 ID，原生多语言，并且**跟随正文的译本**：读者选了某个译本，选项文字也来自
  该译本，缺失时回退到原文语言。REZICS 可以按"标签 ID × 文本表达（译本）"保存。

## 存放位置

| 数据 | 放哪里 | 依据 |
| --- | --- | --- |
| Narrata 草稿记录（每个选择点一条，另有项目、包、图、节点头记录与逐 ID 墓碑；载荷为 Narrata 规范 JSON，派生索引列由 Narrata 计算） | 内容 PostgreSQL | 查询字段不放进 JSONB 内部；载荷只有 Narrata 解读并校验（[ADR 0022](../adr/0022-authoring-publication-and-reader-map.md)） |
| 选项文字 | 内容 PostgreSQL | 同上 |
| 发布对象集合（程序清单、程序块、墓碑、图瓦片、标签表、语义摘要、关系投影片、可选演化描述符） | 对象存储，按内容寻址，永久保留 | 发布后不可变，按需加载；作品当前发布以修订号 CAS 指向 `publication_id`（ADR 0022、[ADR 0023](../adr/0023-node-artifact-evolution.md)） |
| 关系投影（单元、节点、跨单元边、入口、结局、路线、簇、首次出现） | 内容 PostgreSQL | 以（构件, 投影版本）为键只插入；投影版本变化时从保留构件重建（[决定 16](../product/decisions.md#16-程序的关系投影进入宿主数据库)） |
| 语义摘要（入口、结局、路线、首次出现） | Jena | 每次发布写一次，供发现与剧透边界使用；不投影选择边全集 |
| 读者存档 | 内容 PostgreSQL 的库、对象、键三张表，每个"用户 × 作品"一个库 | 浏览器经批次协议增量读写；Narrata 的结构变化由 Narrata 逐库迁移，表结构不随 Narrata 版本变化（[决定 15](../product/decisions.md#15-服务器存档实现同一个存储契约)、[ADR 0021](../adr/0021-server-save-storage.md)） |

Jena 是单写者（TDB2），写入经过带回执的命令模块，适合每次发布一次的语义事实，不适合每次
点击的私人输入；这与 REZICS 自己把 Structure 完整清单放对象存储、Jena 只放投影的做法一致。

## 解析与结果

阅读时 Narrata 输出当前节点的 `Segment`、选项的 `labelRef` 与下一步可能进入的单元；REZICS
批量解析并渲染正文（`DocumentBody` 是自然的渲染插槽）。结果只用 `ok`、`unavailable`、
`incompatible`：REZICS 对"不存在"与"无权限"统一返回 404，以免泄露信息，Narrata 不区分两者。

## 网站集成

- rezics-next 目前没有任何 Wasm 或 Rust；每页压缩脚本预算 330 KB。Narrata 的 Wasm 运行时
  （当前节点栈约 960 KB 未压缩）只在互动小说路由懒加载。
- 交付方式沿用 REZICS 消费兄弟仓库的先例：带版本的 npm 包（参照 `native-i18n`）。
- rezics-next 没有图形库；作者图视图使用懒加载的 sigma.js（见 [图与分析](../contracts/graph-and-analysis.md)）。

## 能力请求

状态：`提出` 表示 Narrata 已提出、REZICS 尚未决定。

| # | 能力 | REZICS 层 | 性质 | 状态 |
| --- | --- | --- | --- | --- |
| R1 | 非线性 Book 阅读模式：上一章/下一章、章节编号、"继续阅读"、未读数、按顺序的剧透判断交给 Narrata；阅读进度记录 Narrata 存档所在的 Occurrence。存档打开固定构件；遇到新发布时由宿主按策略处理演化评估结果（推荐：`Automatic` 直接应用并告知，`NeedsConfirmation` 弹出确认，其余继续旧版或重新开始），确认前与落盘成功前保留原进度（ADR 0023） | Occurrence、阅读 | 新增模式，跨多个模块 | 提出 |
| R2 | `narrata-choice` 块类型（只存选择点 ID），翻译时原样复制 | 文本（`rezics-document`） | 新 type，需新的 profile 版本；或用现有 `extensionBlock` 的 `definition` | 提出 |
| R3 | 译本保留原文块 ID | 文本 | 按段落追踪翻译状态本来需要；需确认 | 提出 |
| R4 | 保存 Narrata 草稿记录 v1：每个选择点一条，另有项目、包、图、节点头记录与逐 ID 墓碑；单条不超过 1 MiB，载荷不透明。Narrata 提供校验、规范化、组装、ID 铸造与派生索引；用 Content model projection recipe 或等价适配层装载选项 ID、来源单元与块、目标节点与单元等派生列，载荷与受影响索引按修订号原子更新（ADR 0022） | 内容 PostgreSQL | 表需主键，表名不含 `game`/`vn` 等领域词；宿主不解释条件或效果 | 提出 |
| R5 | 选项文字：稳定 ID、多语言、按译本 | 内容 PostgreSQL | 新表或复用内容变体 | 提出 |
| R6 | 读者存档：每个"用户 × 作品"一个不透明库，库、对象、键三张表实现窄存储契约，认证后的 HTTPS 承载 ADR 0017 v1 的增量加载与原子落盘；全库乐观并发、对象 put-if-absent、持久化后才确认、重复请求不二次生效、结果未知可核实；配额、库删除与防复活、账号隔离；授权发布服务只读枚举存档用于有界的演化影响评估。第一阶段登录读者只在线，配额与常规 GC 由服务器 Node 运行 Narrata Wasm 执行（ADR 0021、0023） | 内容 PostgreSQL、API | 新增三表与认证路由，参照 `structure.progress` 的并发与命令回执；REZICS 手写编号 SQL，外部包不带迁移；逻辑布局迁移与存档语义归 Narrata | 提出 |
| R7 | 分支段落可阅读但不进入公开搜索（防剧透） | 发布、搜索 | 新增 | 提出 |
| R8 | 公开的有上限批量正文读取，用于预取（内部已有一次最多 64 份、4 MiB） | API | 公开已有能力 | 提出 |
| R9 | 发布服务用 Narrata 的 authoring 模块（npm 包的 server 子路径，Node/Bun 中运行 Wasm）从草稿记录与大纲生成内容寻址的对象集合，并以 `planEvolution` 生成可选演化描述符；全部对象上传核验、关系投影装载、Jena 摘要取得回执后，以修订号 CAS 切换作品的 `publication_id`。失败可幂等重试，旧发布仍可读；全部发布永久保留，切换当前发布不改读者存档（ADR 0022、0023） | 发布、对象存储、Jena | 新增 profile | 提出 |
| R10 | 互动小说路由懒加载 Narrata npm 包与 Wasm；作者图视图依赖 sigma.js | apps/web | 新依赖，首个 Wasm | 提出 |
| R11 | 解析器随"章节 Post"调整，Occurrence 仍可解析到当前文本修订 | 文本、Occurrence | REZICS 自身的 Work 分层工作 | 提出 |
| R12 | 内容大纲：按 Occurrence 批量返回原文语言的 `ContentOutline` v1（完整有序块 ID 与 `narrata-choice` 标记位置，不含正文），固定到本次内容修订，可分批收集为每个内容方一份；Narrata 发布时据此检查锚点、顺序与标记块，有缺陷时宿主阻止切换发布（[ADR 0013](../adr/0013-r2-text-free-node-format.md) §4、ADR 0022） | 文本、API | 只读投影，可由已有文档结构导出 | 提出 |
| R13 | 关系投影表：按 ADR 0022 参考 DDL 保存单元、节点、跨单元边、入口、结局、路线、簇、首次出现的受检事实行，键含构件与投影版本，只插入；提供按内容引用、已访问单元与节点、正反向边的有界查询。完整装载后启用，新投影版本从保留构件重建后切换并清理旧行。服务器用 Narrata 纯函数生成读者地图，可选从服务器存档验证已访问集合；授权与隐私由 REZICS 负责 | 内容 PostgreSQL、阅读 | 新增表（表名不含领域词），宿主手写编号 SQL | 提出 |

除 R1 需要新增一种阅读模式并触及多个阅读模块外，其余都是增加能力，不要求重做现有设计。

## 观察到的 REZICS 侧事实

供 REZICS 自己判断，不是 Narrata 的请求：

- 现有"1 万章"测试是 1 万个位置指向同一个章节 Work，没有测到 1 万份独立正文。
- 全站公开可搜索文本有 2 万条上限；章节标题索引每次编辑全量重建；每次插入或移动章节都会
  重建全站发现数据。互动小说作品的章节数增长时这些会先成为瓶颈。
- 详见 [REZICS 章节规模调研](../research/2026-10-05-choice-granularity-and-scale/rezics_chapter_scale.md)。
