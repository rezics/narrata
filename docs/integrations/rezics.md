# REZICS 集成

日期：2026-10-05。取代 [2026-08-31 的集成边界](../../archive/2026-08-31-rezics-gamebook-integration.md)，
后者使用的 Unit、Post、Portable Text、ContentStructureNode 等名称来自旧仓库 `rezics`，
rezics-next 中已不存在。本页依据 rezics-next 的代码（`D:/rezics-repos/rezics-next`，
2026-10-04 至 05 阅读）与 REZICS 当时的 Work 分层调研结论。

Narrata 的工作不修改 REZICS 仓库（[决定 13](../product/decisions.md)）。REZICS 需要增加的
能力列在[能力请求](#能力请求)，由 REZICS 的 Goal 决定是否以及如何实施；状态变化时更新本页。

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
| Narrata 草稿（每个选择点一行，选项 ID、来源单元与块、目标单元做成索引列，条件与效果为 Narrata 规范载荷） | 内容 PostgreSQL | 查询字段不放进 JSONB 内部；载荷只有 Narrata 解读并校验 |
| 选项文字 | 内容 PostgreSQL | 同上 |
| 发布后的程序构件 | 对象存储（与 Structure 清单相同的内容寻址方式） | 发布后不可变，按需加载 |
| 语义摘要（入口、结局、路线、首次出现） | Jena | 每次发布写一次，供发现与剧透边界使用；不投影选择边全集 |
| 读者存档 | 内容 PostgreSQL，按用户存不透明字节 | 参照 `structure.progress` 的并发与幂等做法 |

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
| R1 | 非线性 Book 阅读模式：上一章/下一章、章节编号、"继续阅读"、未读数、按顺序的剧透判断交给 Narrata；阅读进度记录 Narrata 存档所在的 Occurrence | Occurrence、阅读 | 新增模式，跨多个模块 | 提出 |
| R2 | `narrata-choice` 块类型（只存选择点 ID），翻译时原样复制 | 文本（`rezics-document`） | 新 type，需新的 profile 版本；或用现有 `extensionBlock` 的 `definition` | 提出 |
| R3 | 译本保留原文块 ID | 文本 | 按段落追踪翻译状态本来需要；需确认 | 提出 |
| R4 | Narrata 草稿表（每个选择点一行，索引列见上） | 内容 PostgreSQL | 新表，需按"新领域不加表"规则拍板 | 提出 |
| R5 | 选项文字：稳定 ID、多语言、按译本 | 内容 PostgreSQL | 新表或复用内容变体 | 提出 |
| R6 | 读者存档：按用户存不透明字节、乐观并发、幂等 | 内容 PostgreSQL | 新表，参照 `structure.progress` | 提出 |
| R7 | 分支段落可阅读但不进入公开搜索（防剧透） | 发布、搜索 | 新增 | 提出 |
| R8 | 公开的有上限批量正文读取，用于预取（内部已有一次最多 64 份、4 MiB） | API | 公开已有能力 | 提出 |
| R9 | 发布时保存 Narrata 构件到对象存储并写入语义摘要 | 发布、Jena | 新增 profile | 提出 |
| R10 | 互动小说路由懒加载 Narrata npm 包与 Wasm；作者图视图依赖 sigma.js | apps/web | 新依赖，首个 Wasm | 提出 |
| R11 | 解析器随"章节 Post"调整，Occurrence 仍可解析到当前文本修订 | 文本、Occurrence | REZICS 自身的 Work 分层工作 | 提出 |

除 R1 需要新增一种阅读模式并触及多个阅读模块外，其余都是增加能力，不要求重做现有设计。

## 观察到的 REZICS 侧事实

供 REZICS 自己判断，不是 Narrata 的请求：

- 现有"1 万章"测试是 1 万个位置指向同一个章节 Work，没有测到 1 万份独立正文。
- 全站公开可搜索文本有 2 万条上限；章节标题索引每次编辑全量重建；每次插入或移动章节都会
  重建全站发现数据。互动小说作品的章节数增长时这些会先成为瓶颈。
- 详见 [REZICS 章节规模调研](../research/2026-10-05-choice-granularity-and-scale/rezics_chapter_scale.md)。
