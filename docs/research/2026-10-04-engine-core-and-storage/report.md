# 把 Narrata 内核收窄为身份、事实与提交

Narrata 的核心不应是"一张叙事图"，也不应绑定某种数据库。它应当是存储、媒体和时间三个方向都变化时仍然不变的那几样东西：**稳定身份**（逻辑构件、内容修订、内容出现点三类 ID 分开）、**类型化事实状态**、**确定性转移语义**，以及**内容寻址的提交历史**。用户的三个论点中，"逻辑抽象为核心"得到叙事学、交互叙事研究和 2025–2026 年 LLM 长程一致性研究的一致支持，但"逻辑"要写得更具体：类型化状态，加上带前置条件的内容单元、可插拔的选择策略和讲述（discourse）层。图只是创作视图，以及单元内部的局部控制流。"存储可插拔"成立，但可插拔的层级要比用户设想的低一层，落在窄的"对象 + 引用 CAS + 原子批次 + 有序扫描"契约上。按这个契约，PostgreSQL、SQLite 和 IndexedDB 是一等后端，JSON 只适合做单写者后端和交换格式，图数据库应降为可重建的派生投影。"与媒体文字分离"在运行时和产品层面成立，在创作层面不成立：内核仍须携带说话者、视角、选择与超时、抽象节拍、行 ID 和类型化参数这些跨媒介抽象。仓库的设计文档大体已经站在这个立场上（ADR 0006、`REBUILD_PROPOSAL.md` §4、`docs/research/2026-09-05-narrative-model-evidence.md` §3、`docs/rezics-gamebook-integration.md`）。真正的阻碍在实现里的几个具体卡点：`SaveStore` 只有全量枚举接口，SQLite 适配器每次操作都整库加载、整库重写，Program 是单块构件，文本同时进入 `ProgramArtifactId` 和 Snapshot 的 `StateDigest`，`SceneState` 是强制字段，两套引擎栈既不共享存储也不共享身份。因此，第一步不是增加 PostgreSQL 或图数据库后端，而是按下面的顺序推进：先补基准；再收窄存储契约，并把 SQLite 改成行级适配器；然后把文本从逻辑身份中剥离（这一步需要新的格式 ADR 和新的冻结兼容语料）；之后才做分块加载、迁移阶梯和更多后端。超长叙事与动态更新的能力来自分块身份、索引和迁移设计，与选用哪种数据库关系不大。

## 三个论点：一个成立，一个要降一层，一个只在运行时成立

下表是逐条检验的结论，后文给出证据和推理。

| 用户论点 | 判定 | 需要修正的地方 | 关键证据 |
|---|---|---|---|
| 把叙事逻辑抽象出来作为核心 | 成立 | "逻辑"不是一张全局叙事图。它由四部分组成：类型化事实状态、带前置条件与效果的内容单元、可插拔的选择策略、讲述层。图与状态机只承担创作视图和单元内部的局部控制流；状态应当类型化，不宜全部是数字变量 | 叙事学的三层模型；Façade、Versu、storylet 的规模经验；2025–2026 年 LLM 长程一致性基准 |
| 存储后端插件化（图数据库、PostgreSQL/ORM、JSON） | 部分成立 | 可插拔层级应是窄的存储原语契约，不是通用 ORM 或查询层。PostgreSQL、SQLite、IndexedDB 是一等后端；JSON 只做单写者后端和交换格式；图数据库降为派生投影。超长叙事靠分块与索引，与后端种类无关 | FoundationDB 的分层设计、Stonebraker & Pavlo 2024、DuckPGQ、PostgreSQL 19 的 SQL/PGQ、Kùzu 归档、ORM 批评 |
| 叙事引擎与媒体、文字等"真实内容"完全分开 | 运行时与产品层成立，创作层不成立 | 内核须保留跨媒介抽象（说话者、视角、选择与超时、节拍、行 ID 与类型化参数）。"行"是跨越边界的原子单位。会影响规则判断的媒体或文本属于逻辑，必须进入语义锁 | BBC OBM/StoryPlayer、Yarn 字符串表、MessageFormat 2/Fluent；inkle 与 Failbetter 的反方经验 |

### 逻辑为核心：各方证据一致，但"逻辑"要换一种写法

叙事学和计算叙事研究都把"发生了什么"与"怎么讲、用什么媒介讲"分开处理。Narrative Ontology 把叙事形式化为三部分：fabula、一个或多个 narration，以及二者之间的 reference 关系。它明确规定，一个 fabula 可以对应多种语言和媒介的 narration ([Meghini et al. 2021](https://www.semantic-web-journal.net/system/files/swj2539.pdf))。2024 年 ICAPS 的叙事规划综述沿用 plot、discourse、narration 三层，并指出多数规划器默认按流水线处理这三层 ([Cardona-Rivera et al. 2024](https://ojs.aaai.org/index.php/ICAPS/article/download/31509/33669))。媒介意识叙事学则区分了三类概念：**与媒介无关**的（人物、行动、场景、因果、动机），**跨媒介**的（叙述者、聚焦、交互性），以及**媒介专属**的（漫画的格框、留白、气泡）([Baroni, Goudmand & Ryan 2023](https://www.marilaur.info/baronigoudmadryan.pdf))。这套三分法可以直接当作 Narrata 的边界规则使用。

最新、也最有力的证据来自 LLM 时代。预印本 NstAgent 用带类型、带键的状态记录人物快照、事件和"未来要求"（已埋下、待兑现的伏笔）。从 1 万词到 10 万词，它的一致性保持稳定，而自由文本滚动摘要的基线随长度退化 ([Wan & Chen 2026](https://arxiv.org/html/2609.35759))。在 NCP-Bench 上，表现最好的模型 20 轮后只有 **42% 的存活率**，各模型的事实冲突率在 **40%–68%** 之间 ([Ma et al. 2026](https://arxiv.org/abs/2608.08160))。StateMemBench 测得现有记忆系统的当前状态准确率只有 **0.149–0.205**，主要失败在于分不清现行事实和已被取代的事实 ([arXiv 2608.19652](https://arxiv.org/abs/2608.19652))。商业产品也在转向同一方向。Latitude 的 Voyage 用一个"确定性 World Engine"管理世界状态与规则 ([TechCrunch](https://techcrunch.com/2026/04/21/voyage-is-an-ai-rpg-platform-for-creating-custom-gaming-worlds-with-ai-generated-npc-interactions/))。Hidden Door 的反面案例则说明，结构层一旦太薄，渲染出来的文本就会变成事实上的状态 ([Bicking 2025](https://ianbicking.org/blog/2025/08/hidden-door-design-review-llm-driven-game.html))。

需要修正的是"逻辑"的形状。用户把图建模当作核心表示，而证据表明，全局分支图本身不能扩展到长叙事。Ashwell 的模式分类显示，只要分支需要重新汇合（branch-and-bottleneck），就几乎必然重度依赖状态追踪 ([Ashwell 2015](https://heterogenoustasks.wordpress.com/2015/01/26/standard-patterns-in-choice-based-games/))。Façade 一段约 20 分钟的体验需要约 2,500 个对话行为。为了避免因果链组合爆炸，作者把叙事拆成多条并行推进、基本互不依赖的线 ([Mateas & Stern 2005](https://users.soe.ucsc.edu/~michaelm/publications/mateas-aiide2005.pdf))。Versu 的作者论证，社交情境（practice）的表达力严格强于有限状态机 ([Evans & Short](https://versu.com/wp-content/uploads/2014/05/versu.pdf))。真正扩展到超长、持续增量的是以状态为中心的模型：Fallen London 靠 storylet 和 quality 运营 15 年以上，累积了 **超过 450 万词** ([Failbetter 2025](https://www.failbettergames.com/news/fallen-londons-15th-birthday))；Valve 的规则库之所以便于更新，是因为新内容"全部是追加" ([Ruskin, GDC 2012](https://steamcdn-a.akamaihd.net/apps/valve/2012/GDC2012_Ruskin_Elan_DynamicDialog.pdf))。图和状态机在这些系统里仍然有位置，但只在单元内部。Visual SceneMaker 用层次化 statechart 承载控制流，把台词放在另一份类剧本的文本中 ([Gebhard et al. 2012](https://link.springer.com/article/10.1007/s12193-011-0077-1))。

还有两点修正。第一，**讲述层是逻辑，不是渲染细节**。叙事话语规划在"受众信念空间"中进行，PLOTSHOT 和 BIPOCL 这类系统甚至让情节和讲述共同演化 ([Cardona-Rivera et al. 2024](https://ojs.aaai.org/index.php/ICAPS/article/download/31509/33669))。如果内核只建模世界事实，那么"揭示了什么、读者知道什么"的逻辑就会泄漏进文本脚本。第二，**状态要类型化**。Alexis Kennedy 回顾 Fallen London 时承认，当年把所有 quality 都做成同一种数字，抹掉了有用的区分，后来的游戏只能靠变通手段补救 ([Kennedy 2017](https://weatherfactory.biz/qbn-to-resource-narratives/))。仓库自己的 `docs/research/2026-09-05-narrative-model-evidence.md` §3 推荐"类型化、可组合的叙事定义图＋共享领域状态＋活动实例执行"，方向与此一致。本报告的补充是：定义图是创作侧的组织结构，运行时的基底是状态与单元。

### 存储可插拔：方向对，层级错了一层

"让存储可替换"本身有充分先例。六边形架构的初衷，就是让应用能脱离具体的运行设备和数据库独立开发、测试 ([Cockburn](https://alistair.cockburn.us/hexagonal-architecture/))。叙事中间件已经在变量和存档层做到了这一点：Yarn Spinner 的变量存储被设计成可替换的抽象类 ([Yarn 文档](https://docs.yarnspinner.dev/3.1/components/variable-storage/variable-storage))；Pixel Crushers 把序列化器和存储器拆成两个可自定义的层 ([Pixel Crushers](https://www.pixelcrushers.com/dialogue_system/manual2x/html/save_system.html))。

问题出在"图数据库、PG/ORM 模拟、JSON 三者互换"这个层级上。有失败记录的，恰恰是夹在中间的通用查询/ORM 层。Neward 把对象关系映射称为收益递减的泥潭 ([Neward 2006](https://blogs.newardassociates.com/blog/2006/the-vietnam-of-computer-science.html))，Spolsky 指出逻辑等价的查询性能可以相差数千倍 ([Spolsky 2002](https://www.joelonsoftware.com/2002/11/11/the-law-of-leaky-abstractions/))。这样的中间层要么退化成各后端的最小公分母，要么到处漏出底层细节。经得住考验的是两头。低的一头是有序、事务化的键值或对象存储：FoundationDB 只靠有序键、范围读和 ACID 事务，就在 KV 之上分层构建出文档、图和 SQL 模型 ([FoundationDB](https://apple.github.io/foundationdb/layer-concept.html))；SurrealDB 对 RocksDB、内存和 IndexedDB 等引擎只要求"按单键与键范围的事务读写" ([SurrealDB](https://surrealdb.com/docs/surrealdb/introduction/architecture))。高的一头是窄的领域端口。后端能力不同时，成熟做法是让后端声明能力，并由引擎为缺失的能力保留正确的回退路径，OpenDAL 的 `Capability` 和 DataFusion 的过滤下推都是这样设计的 ([OpenDAL](https://opendal.apache.org/docs/rust/opendal/struct.Capability.html)；[DataFusion](https://docs.rs/datafusion/latest/datafusion/datasource/trait.TableProvider.html))。

图数据库是这个论点里最弱的一环。Stonebraker 与 Pavlo 认为，图数据库的价值取决于"长链遍历"场景是否足够多，并判断 **OLTP 图应用将主要由关系数据库承担**。他们还引用 DuckDB 的 SQL/PGQ 实现，在某些测试中比领先的图数据库快 **最多 10 倍** ([Stonebraker & Pavlo 2024](https://dl.acm.org/doi/10.1145/3685980.3685984)；[DuckPGQ, CIDR 2023](https://www.cidrdb.org/cidr2023/papers/p66-wolde.pdf))。PostgreSQL 19 已经合入 SQL/PGQ 的 `GRAPH_TABLE`，但暂不支持可变长路径和最短路径 ([PostgreSQL commit](https://www.postgresql.org/message-id/E1w247I-0000Tk-2Y@gemulon.postgresql.org))。Memgraph 自己的测试显示，在 4 跳可达性上它比 PG19 快 11.5 倍。这是厂商在一个刻意收窄的负载上做的测试，但它说明深度遍历确实是图数据库的主场 ([Memgraph](https://memgraph.com/blog/postgresql-19-alternatives-memgraph))。

Narrata 的运行时负载并不在那个主场。它做的是：读一个对象、跟随几条边、追加一个提交。这是 Git、Dolt 这类 Merkle-DAG 加 KV 的工作负载 ([Dolt](https://www.dolthub.com/docs/architecture/storage-engine))。深度可达性查询只出现在创作分析中（死路、不可达结局、"这次改动影响哪些存档"）。可嵌入的图库连续性风险也很高：Kùzu 被 Apple 收购后归档 ([UWaterloo](https://uwaterloo.ca/computer-science/news/waterloo-based-graph-database-start-up-kuzu-acquired-apple))；CozoDB 的最后一次发布停在 2023 年 12 月 ([CozoDB releases](https://github.com/cozodb/cozo/releases))。

JSON 作为主存储，会失去原子的多对象提交、CAS 和二级索引。浏览器的 localStorage 甚至只有 10 MiB 上限 ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria))。JSON 适合按 Git 松散对象的方式做单写者后端，也适合做可读的交换格式。PostgreSQL 内的 JSONB 也有陷阱：规划器对 JSONB 内部字段没有统计信息，曾导致一个查询计划慢了约 **2000 倍**（数据来自 2016 年）([Heap](https://www.heap.io/blog/when-to-avoid-jsonb-in-a-postgresql-schema))。

行业实践给出了同样的边界。在本轮调研覆盖的引擎中，没有一个在运行时直接对可变的内容数据库执行叙事逻辑。Yarn 明确要求在编辑期编译，而不是运行时编译 ([Yarn FAQ](https://docs.yarnspinner.dev/faq))。Valve 的 Ruskin 解释过规则库为什么不是关系数据库：用关系表表示时，每条规则都要拖着数千个空列 ([Ruskin](https://steamcdn-a.akamaihd.net/apps/valve/2012/GDC2012_Ruskin_Elan_DynamicDialog.pdf))。

最后，后端种类本身不会带来超长叙事能力。sql.js-httpvfs 在一个 670 MB 的数据库上做一次带索引的查找，只抓取了约 1 kB 数据 ([phiresky](https://phiresky.github.io/blog/2021/hosting-sqlite-databases-on-github-pages/))。真正决定规模能力的是索引与分区设计。

### 与媒体分离：分开的是产物与运行时，不是写作过程

运行时分离有扎实先例。BBC 一线的 OBM 研究把架构分为三部分：一个**与制作无关的叙事引擎**，一个按引擎指令拼装媒体的合成引擎，以及媒体仓库。叙事引擎输出的是"播放清单"，而不是媒体本身。它还允许没有任何媒体引用的"空原子对象"，所以故事结构可以在素材存在之前就完成搭建和测试 ([Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf))。BBC StoryPlayer 把"走哪个叙事元素"和"渲染哪个表现形式"交给两个不同的推理器，二者读同一个变量存储 ([bbc/storyplayer](https://github.com/bbc/storyplayer/blob/main/docs/implementation.md))。Yarn Spinner 的编译器把脚本拆成代码和按语言划分的字符串表，运行指令只携带行 ID ([Yarn v1 文档](https://v1.yarnspinner.dev/docs/unity/components/yarn-programs/))。Yarn 的复数、性别等标记只在消息层按地区替换整行，周围的逻辑不变 ([Yarn markup](https://docs.yarnspinner.dev/write-yarn-scripts/advanced-scripting/markup))。在 Valve 的规则系统里，"响应"就是一个语音文件或一段动画 ([Ruskin](https://steamcdn-a.akamaihd.net/apps/valve/2012/GDC2012_Ruskin_Elan_DynamicDialog.pdf))。

消息格式标准也已经成熟。Unicode MessageFormat 2.0 于 **2025-03-13** 在 CLDR 47 / ICU 77 中进入 Stable ([Unicode](http://blog.unicode.org/2025/03/unicode-cldr-47-release-messageformat-2.html))。Fluent 主张让译者不必征得开发者同意就能用尽母语的表达力 ([Fluent](https://projectfluent.org/))。产品策略同样支持分离：Yarn Spinner 承诺核心永远免费开源，表现层套件作为付费附加品单独提供 ([Yarn Spinner 2026](https://yarnspinner.dev/blog/yarn-spinner-in-2026))。

反方证据集中在创作层。inkle 的 Jon Ingold 坚持让编程嵌在文字之中，而不是反过来。Failbetter 的 Emily Short 说，只有看到台词与美术、界面放在一起时，才是调整节奏的时机 ([MCV/DEVELOP 2021](https://mcvuk.com/development-news/want-your-story-to-drive-your-game-rather-than-vice-versa-inkle-and-failbetter-discuss-the-storytelling-potential-of-the-open-source-ink/))。OBM 的制作复盘写道：写作是最难的一环；内容拍完之后，原先设计的结构被大幅简化 ([Ursu et al. 2020](https://eprints.whiterose.ac.uk/163696/1/obm_final_version.pdf))。Jenkins 观察到，跨媒介叙事在同一作者统筹或强协作时效果最好 ([Jenkins 2007](http://henryjenkins.org/blog/2007/03/transmedia_storytelling_101.html))。《Heaven's Vault》让 ink 同时承担叙事、呈现和编排，因此作者能在发售前 10 周重写一整场戏的每一行，而不必与其他团队协调 ([Game Developer](https://www.gamedeveloper.com/design/knockin-on-i-heaven-s-vault-i-how-inkle-designed-its-first-3d-game))。

这些证据并不推翻用户的论点，而是限定了它的成立范围：**创作时并置，编译时分离，运行时只交换语义事件**。社区的 Ink-Localiser 正是这样处理的。它报错拒绝句内片段拼接，并建议运行时只用 ink 承担逻辑、不承担内容 ([Ink-Localiser](https://github.com/wildwinter/Ink-Localiser))。

还有一处必须修正："完全分开"在边缘不成立。会影响规则判断的资源（例如作为谜题条件的图像，或参与比较的名字）属于逻辑。仓库的 `REBUILD_PROPOSAL.md` §8 已经规定这类资源进入语义锁。此外，Netflix 于 2025-05-12 下架全部互动作品，称该技术"已成为限制" ([Variety](https://variety.com/2025/tv/news/black-mirror-bandersnatch-removal-netflix-1236392097))。这一失败指向的是把叙事运行时绑死在单一播放器上的做法，不是分离本身。

## 方案：内核只拥有身份、事实与转移，其余一律走协议

### 核心抽象由六个部件组成，全局图不在其中

| 部件 | 职责 | 仓库对应物 | 现状 |
|---|---|---|---|
| 身份体系 | 逻辑构件 ID、内容修订 ID、内容出现点/行 ID、包 ID 彼此独立；渲染文本不参与逻辑身份 | `crates/narrata-core/src/identity/authored.rs`（已有 `ContentOccurrenceId`）、`identity/derived.rs`（已有 `ContentLockId`、`BuildProvenanceId`）、`authoring.rs` 的 `StableIdKind::ContentOccurrence` | 类型已经齐备，但 `ProgramArtifactId` 把文本一并哈希 |
| 类型化事实状态 | 按模块命名空间存放资源、关系、标记、信念、未兑现承诺 | `RuntimeStateV0.globals`；`REBUILD_PROPOSAL.md` §7.1 提出的按实例与命名空间组织的 module states | 只有全局 `globals` 和强制的 `SceneState` |
| 内容单元 | 带前置条件、效果和参数；单元内部由 Flow VM 或 Statechart 驱动 | Flow VM、Statechart（ADR 0008）、`narrata-nodes` 的 `NodePlan` | 有流程和节点，没有 storylet 式的可选单元 |
| 选择策略 | 玩家选择、显著性/storylet、导演或规划器、LLM 建议 | `Choice` 指令、`narrata.decision` 节点 | 只有玩家选择 |
| 讲述层 | 已讲述、已揭示、读者已知，按观察者区分 | 无 | 缺失；建议做成第一方领域模块，不写进内核枚举 |
| 转移与历史 | `reduce` → `TransitionDraft` → Commit/Receipt/Ledger | `docs/architecture/runtime-model.md`、ADR 0006、ADR 0007 | 已实现，是最值得保留的资产 |

讲述层、storylet 选择和"未兑现承诺"放在第一方领域包，而不是内核，原因在于 `REBUILD_PROPOSAL.md` §4.1 已经规定：内核不必把台词、立绘、好感度、任务节点逐个写进一个总枚举，但必须支持"注册后可检查的领域类型与处理器"。

NstAgent 和 NCP-Bench 都把伏笔与承诺当作长叙事最先丢失的状态，这为承诺模块提供了依据。《Heaven's Vault》维护"玩家已知"模型来调取相关对话，这为讲述层提供了产业先例 ([Game Developer](https://www.gamedeveloper.com/design/knockin-on-i-heaven-s-vault-i-how-inkle-designed-its-first-3d-game))。外部规划器或 LLM 只能作为选择策略接入，遵循"LLM 提议、符号规则裁决"的模式 ([Góngora et al. 2026](https://arxiv.org/abs/2605.24719))，结果经 Recorded Query 或效果账本进入状态。

### 七层结构与三条硬边界

| 层 | 内容 | 仓库位置 | 可替换性 |
|---|---|---|---|
| L0 确定性内核 | 纯 `reduce`、类型化值、身份、规范 CBOR、限额 | `crates/narrata-core` | 不可替换，是唯一的语义真相 |
| L1 第一方领域包 | Gamebook 节点词汇、VN Scene、讲述层、storylet 选择、任务 | `packages/narrata/nodes`，以及未来的 R2/R3 | 按作品组合 |
| L2 会话与持久化引擎 | Coordinator、提交、效果账本、目录、GC、迁移、包加载 | `crates/narrata-store` | 只写一次，所有后端共用 |
| L3 存储契约与适配器 | 对象、引用、批次、扫描、能力声明 | 新拆出的无 core 依赖 crate，加上 `crates/narrata-store-sqlite` 等适配器 | 可插拔 |
| L4 投影 | PG 报表列、图数据库导出、全文/向量索引、分析用 CSR | 新增，可选 | 可删除后重建 |
| L5 宿主协议 | Protobuf 拉取协议、C ABI、Wasm、各语言绑定 | `crates/narrata-protocol`、`crates/narrata-ffi`、`crates/narrata-wasm`、`bindings/` | 按宿主选择 |
| L6 呈现与内容产品 | 文本与本地化（REZICS）、媒体清单与管线、阅读器、LLM 叙述者 | REZICS、规划中的 `narrata-media`、`examples/gamebook-web` | 独立产品 |

三条硬边界的规则如下。

第一，L2 与 L3 之间只能通过存储契约交互，引擎不得假设后端能"列出全部"。

第二，L0/L1 与 L6 之间只交换语义事件和类型化参数。呈现侧可以读取只读的状态视图，但不能直接写状态。

第三，L1 与内容产品之间只交换 ID 和内容锁。REZICS 继续拥有正文、Portable Text、本地化与权限。Narrata 的图节点引用 `ContentStructureNode` 的出现点，而不是 Post ID（`docs/rezics-gamebook-integration.md`）。`crates/narrata-core/src/content.rs` 里已经有 `ContentQuery`、`RevisionPolicy::{Pinned, RecordFirstResolution, LivePresentationOnly}` 和 `ExternalContentResolver`，只是运行时没有调用点。应当在协议边界接通它们，而不是在 `reduce` 里调用。

### 呈现边界只交付语义事件与类型化参数

下面的草图综合了 BBC StoryPlayer 的事件流、Yarn 的行 ID 加字符串表、Dink 把流程、行元数据和各语言文本拆成三类产物的做法 ([Dink](https://github.com/wildwinter/dink))，以及 Elm 的"状态 → 视图"架构 ([Elm](https://guide.elm-lang.org/architecture/))。仓库现有的 `ReconcileScene` 就是 Elm 模式在时间旅行上的应用：读档或回退时，呈现由状态重新推导，而不是重放"显示 A、隐藏 B"的命令历史。

```rust
// 内核 → 呈现：只含身份与类型化参数，不含任何已解析文本、语言或媒体路径（草图）
pub enum PresentationEvent {
    Utterance { occurrence: ContentOccurrenceId, line: LineId,
                speaker: Option<EntityId>, class: LineClass, args: TypedArgs },
    ChoiceSet { choices: Vec<ChoiceView>, timeout: Option<TimeoutSpec> },
    Beat(BeatKind),               // pause / await_ack / section_break / scene_change，无毫秒、无打字速度
    Scene(SceneProjection),       // 可选 VN 模块的声明式状态；读档/回退时整体 Reconcile
    Cue { cue: CueId, args: TypedArgs },  // 音乐、音效、吠叫概念等语义意图
    Effect(EffectRequest),        // 沿用 ADR 0007 的账本路径
}
pub struct ChoiceView { id: ChoiceId, label: LineId, args: TypedArgs,
                        enabled: bool, reason: Option<LineId> }
// 呈现 → 内核：只能是被记录的输入（选择、超时结果、Recorded Query 结果），从不直接写状态
```

这个协议附带五条不变量。

第一，外部解析出的任何东西都不能改变守卫、选择或效果的结果，除非它作为记录输入重新进入内核。这正是 `docs/architecture/program-versioning-and-migration.md` 对 `LivePresentationOnly` 的规定。

第二，**"行"是原子单位**，内核从不拼接文本片段。复数、性别、格、敬语等语法变体由 MessageFormat 2 或 Fluent 在消息层根据传入的类型化参数选择。"叙事变体"（信息不同、后果不同）则是逻辑选出的不同 `LineId`。

第三，`LineId` 由编译器分配并写回源文件，配一个源文本锁定哈希，用来发现过期的翻译和配音。Yarn 和 Naninovel 都靠这一机制应对源文本修改。

第四，存档按策略钉住内容修订。

第五，屏幕阅读器、纯文本 CLI、Gamebook 阅读器和 LLM 叙述者都只是渲染器。LLM 叙述者需要的是"为什么"（实体关系、节拍目的、视角），所以讲述层和事实视图必须对渲染器只读开放。

计时只有在交互层面属于叙事：带超时的选择和默认选项属于内核，超时作为记录输入出现。句内停顿、打字速度、与音乐的同步则属于行元数据。

## 存储：五个原语加能力声明，图数据库只做投影

### 最小存储契约

这份契约综合了 FoundationDB、SurrealDB 的 KV 层、`object_store` 的条件写入（`PutMode::Create` / `Update`）([object_store](https://docs.rs/object_store/latest/object_store/enum.PutMode.html))，以及 Git/Dolt "内容寻址对象加一次根指针交换"的做法。它要求后端提供五件事：幂等的对象写入、按摘要读取、带修订号的引用 CAS、全成或全败的原子批次、有序且带游标的有界扫描。此外，后端要用一份能力声明说明自己的事务、持久性和尺寸上限。

```rust
// 草图：不依赖 narrata-core，只认字节、摘要与键，使 narrata-nodes 也能直接使用（ADR 0011）
pub trait StorageBackend {
    fn capabilities(&self) -> Capabilities;
    fn get(&self, id: &ObjectId) -> Result<Option<Arc<[u8]>>, BackendError>;
    fn has_many(&self, ids: &[ObjectId]) -> Result<Vec<bool>, BackendError>;
    fn read_ref(&self, key: &RefKey) -> Result<Option<(RefValue, Revision)>, BackendError>;
    fn scan(&self, space: KeySpace, prefix: &[u8], after: Option<&[u8]>, limit: u32)
        -> Result<ScanPage, BackendError>;                  // 有序、带游标、有界
    fn apply(&mut self, batch: AtomicBatch) -> Result<(), ApplyError>; // 全成或全败
}
pub struct AtomicBatch {
    pub put_if_absent: Vec<(ObjectId, Arc<[u8]>)>,          // 幂等，已存在即跳过
    pub ref_cas: Vec<(RefKey, Option<Revision>, Option<RefValue>)>, // 期望修订不符则整批失败
    pub index_put: Vec<(KeySpace, Vec<u8>, Vec<u8>)>,       // 引擎维护的二级索引与数据同批提交
    pub index_delete: Vec<(KeySpace, Vec<u8>)>,
}
pub struct Capabilities {
    pub atomic_multi_key: bool, pub ordered_scan: bool, pub concurrent_writers: bool,
    pub durability: Durability,       // Durable | Evictable | Memory
    pub max_value_bytes: u64, pub max_batch_bytes: u64,
}
```

现有的 `CommitTransaction` 包含对象、引用 CAS、目录、归档、复合存档、输入索引和 pin（`crates/narrata-store/src/store.rs:64-98`）。改造后，它由 L2 引擎翻译成一个 `AtomicBatch`。效果账本的租约（`claim_effect`、`renew_effect_lease`、`record_effect_outcome`）、目录冲突、复合存档、保留 GC、完整性扫描和迁移，全部在契约之上**只写一次**。这正是 FoundationDB 的"layer"思路：索引键与数据在同一个事务里更新。如果不这样做，现在大约 25 个领域方法的 `SaveStore` 会迫使每个新后端重写一遍领域逻辑，也就是 Neward 和 Stonebraker 描述的"在应用里重做 join"的代价。

不支持多键原子性的后端（文件目录、部分对象存储）必须如实声明。引擎对它们改用单根引用模式：先写入全部内容寻址对象，最后对一个根引用做 CAS。读取时，引擎照旧校验摘要（`CheckedObject` 已经这样做），所以后端不需要被信任。

能力声明决定引擎拒绝运行还是正确模拟。例如，没有 CAS 就只允许单写者。

后端还会暴露硬限制，必须声明出来。FoundationDB 的单个事务不能超过 10 MB、持续时间不能超过 5 秒、单值不能超过 100 kB ([FoundationDB 限制](https://apple.github.io/foundationdb/known-limitations.html))；浏览器存储则可能被整源驱逐。

需要说明一点：`crates/narrata-testkit/src/backend.rs` 里的 `ConformanceBackend` 用于检验内核转移的一致性，不是存储一致性套件。存储一致性套件应从 `crates/narrata-store/tests/model.rs` 和 `tests/faults_gc.rs` 中抽取，它们已有覆盖 snapshot、receipt、commit、edge、ref CAS、catalog、GC 和账本的 `FaultPoint` 故障注入点，改造为可对任意后端运行即可。

### 三种存储角色与各后端定位

行业证据把"存储"拆成三个角色，混在一起讨论会得出错误结论 ([Yarn FAQ](https://docs.yarnspinner.dev/faq)；[articy 4.0](https://www.articy.com/help/adx/Changes_4_0.html))。第一种是**创作源仓库**，可以是文件、PostgreSQL，或 Loro 这类 CRDT 协作文档，它属于 Studio/REZICS 一侧，通过编译与引擎交互。第二种是**编译后的不可变构件**，即内容寻址对象，放在契约里。第三种是**会话与存档状态**，即对象、引用和账本，也放在契约里。图数据库最合适的位置是第一种角色的分析侧，或者作为第二、三种角色的投影。CRDT 适合协作编辑，但会累积历史，模式迁移问题也基本未解决 ([Ink & Switch](https://www.inkandswitch.com/essay/local-first/))，因此它应止步于编译边界，不能进入确定性运行时存档。

| 后端 | 定位 | 适合场景 | 注意事项 |
|---|---|---|---|
| `MemoryStore` | 行为参考与测试基线 | 单元测试、Wasm 内的临时会话 | 保持 ADR 0006 给它的"行为参考"地位 |
| SQLite（原生） | 默认本地后端 | 桌面、移动端、CLI、`narrata migrate` | 必须改成行级读写；表结构为对象表、引用表和索引键空间 |
| PostgreSQL | 服务器或多用户后端 | 云存档、REZICS 侧持久化 | 对象以 `bytea` 存规范 CBOR，被查询的字段投影成普通列，热路径不查 JSONB；同步 trait 如何匹配网络 I/O 尚未定 |
| IndexedDB / SQLite-Wasm（OPFS） | 浏览器后端 | Wasm 阅读器 | OPFS 协作同步 VFS 可承载 1 GB 以上的库，IndexedDB VFS 约 100 MB 以上会退化 ([PowerSync](https://powersync.com/blog/sqlite-persistence-on-the-web))；Safari 会删除 7 天无交互的脚本存储，驱逐是整源进行的 ([MDN](https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria))；应申请 `persist()` 并提供导出 |
| JSON 目录 | 单写者后端兼可读导出 | 调试、小型单机存档、交换格式 | 文件名即摘要，引用清单用原子替换更新；声明 `concurrent_writers = false`；记录格式仍用规范 CBOR，以规避 JSON 的浮点与键序不确定性 ([DAG-CBOR](https://ipld.io/specs/codecs/dag-cbor/spec/)) |
| 对象存储（S3 类） | 冷归档与内容包分发 | 已归档的历史和内容包 | 用 create-if-absent 和 if-match 实现根引用 CAS |
| 图数据库（Neo4j、Memgraph、Apache AGE） | 派生投影 | 作者侧的可达性和影响分析 | 从真相源重建，不实现存储契约；AGE 仍在活跃发布 ([Apache AGE](https://age.apache.org/release-notes/))，可嵌入的图库连续性风险高 |

## 超长叙事与动态更新：靠分块身份和迁移阶梯，而不是数据库

### 超长叙事：让"包"成为加载、版本和归档的单位

先看规模。Fallen London 积累了 450 万词以上。Disco Elysium 的剧本超过百万词，它的写作工具 articy 一度完全卡死 ([PC Gamer/Yahoo](https://tech.yahoo.com/gaming/articles/disco-elysium-had-much-text-032304926.html))。粉丝从博德之门 3 中提取出 **173,642 个语音文件** ([PC Gamer](https://www.pcgamer.com/games/baldurs-gate/baldurs-gate-3-fan-discovers-the-game-has-nearly-237-hours-of-spoken-dialogue-in-it-and-a-whopping-14-hours-are-narrator-amelia-tylers-alone-though-astarion-gives-her-a-run-for-her-money/))。仓库的文档里没有任何关于叙事规模的数值目标。本报告建议把 10^6–10^7 词、10^4–10^5 个内容单元、单个存档 10^4 次提交作为基准目标。这是提案，不是现有约定。

对照这些规模，现有上限差了几个数量级。默认解码上限 16 MiB（`crates/narrata-core/src/limits.rs`）。节点栈的单文档上限 4 MiB，整个产品最多 4,096 个节点（`packages/narrata/nodes/crates/narrata-nodes/src/compile.rs`）。存档最多 512 次提交、总计 2 MiB。Program 一次性解码成 `CheckedProgram`，`ProgramLoad{bytes}` 用一条消息发送整个构件。Web 阅读器每次操作都把整份故事源（最多 4 MiB）和整份存档写进同一条 IndexedDB 记录（`examples/gamebook-web/src/storage.ts`）。

行业的应对是分区加流式加载。Valve 把规则按区域切成独立数据库，随关卡数据流式载入，只常驻一小组全局规则 ([Ruskin](https://steamcdn-a.akamaihd.net/apps/valve/2012/GDC2012_Ruskin_Elan_DynamicDialog.pdf))。Cyberpunk 把任务拆成 questphase 和 scene 文件。对 Narrata 的建议有四条。

**第一，把 Program 改成"清单加包级块"。** 清单是 Merkle 根，指向按包或章节切分的逻辑块和独立的内容块，每块单独内容寻址。Snapshot 记录组合身份，即每个包对应哪个构件，`REBUILD_PROPOSAL.md` §8 已有此要求。`CheckedProgram` 退化为按包填充的缓存，按声明的依赖和可达性懒加载。

**第二，归档不等于删除。** Destiny 2 出于体积考虑移除战役，伤害延续数年。直到 2026 年 9 月，Bungie 才承诺恢复这些内容，并承认技术上很难 ([PCGamesN](https://www.pcgamesn.com/destiny-2/campaign-removed)；[GamerHub](https://gamerhub.co.uk/bungie-destiny-2-unvaulting-red-war/))。冷包应进入对象存储，并配上墓碑或重定向记录，保证事实永远不会悬空。

**第三，用索引替代全扫描。** 给提交记录 generation 号，加父子索引键，再加"已验证到此为止"的水位。这样加载时不必再像 `validate_ancestor_closure` 那样每次回溯到 Genesis（`crates/narrata-store/src/coordinator.rs:1814-1889`）。全量校验交给 `integrity_scan` 和导入流程完成。

**第四，用基准驱动存档体积优化。** 仓库基准记录的 Snapshot 体积是：100K 变量约 2.27 MB，1K 变量约 21.9 KB（`docs/plan/stage-2-time-travel-persistence.md:80-107`）。按此推算，1 万次提交的完整快照分别约为 **22.7 GB** 和 **219 MB**。单次延迟达标，不代表长历史的存储量可以接受。顺序应遵循计划 P2.10 已有的原则：先建立可复现的基准，再拆出最大的不可变子树做结构共享，参照 Dolt prolly tree 那种与插入顺序无关的结构 ([Dolt](https://www.dolthub.com/docs/architecture/storage-engine))，而且不引入无界的增量链。保留策略要作为用户可见的保证写明，因为裁剪历史就意味着失去回退到裁剪点之前的能力 ([Loro](https://docs.rs/loro/latest/loro/struct.LoroDoc.html))。

当内容单元达到上万个时，选择必须带索引。每个包声明自己读写哪些事实，只重新评估输入事实发生了变化的守卫，可以借鉴 Winnow 的增量匹配 ([Winnow, AIIDE 2021](https://ojs.aaai.org/index.php/AIIDE/article/view/18903))。同一份读写声明还能支撑静态分析：不可达内容、死写入、旧存档缺少的新事实。更深的有界可达性查询可以离线导出给 ASP 求解器，DendryScope 就是这样做的 ([DendryScope](https://ojs.aaai.org/index.php/AIIDE/article/view/27527))。

### 动态更新：先拆身份，再建迁移阶梯

在拆分身份之前，迁移机制谈不上用处。目前 `ProgramArtifactId` 对整个常量池做哈希（`crates/narrata-core/src/program/validate.rs:227`），已解析的文本又写进了 Snapshot，恢复时还会逐字比对（`crates/narrata-core/src/snapshot/restore.rs:725-753`）。结果是修一个错别字就会产生新构件，所有停在那一行的存档都无法精确恢复。这与文档中"`LivePresentationOnly` 内容变化不改变 Effect/choice/state hash"的必需验证项直接冲突（`docs/architecture/program-versioning-and-migration.md`）。节点栈的情况相同：`artifact_id` 对包含正文的整个 bundle 做哈希，恢复时要求完全相等。

把文本移出逻辑身份之后，"只改内容"就降为下表中的第 1 级，不再需要迁移。

| 级别 | 触发条件 | 处理 | 先例 |
|---|---|---|---|
| 0 精确恢复 | 逻辑构件与内容修订都一致 | 直接恢复 | 现有默认策略 |
| 1 仅内容变化 | 逻辑 ID 不变，内容修订变化 | 按 `Pinned`、`Compatible`、`LivePresentationOnly` 策略解析，无需迁移 | Yarn 的字符串表 |
| 2 结构迁移 | 逻辑 ID 变化，且存在描述符链 | 用现有的 relocation 迁移，记录为 Migration Commit | ADR 0009 |
| 3 重放验证 | 有输入日志 | 在新构件上确定性重放，报告第一个分歧点 | Inky 重编译后快进到原位置 ([Inky](https://github.com/inkle/inky))；Inform Skein 回放已确认的记录 ([Inform](http://inform7.com/book/WI_2_8.html)) |
| 4 事实携带与安全点重入 | 重放出现分歧，或历史过长 | 携带事实，新事实取默认值，回到最近的安全点，执行补偿并通知玩家 | ink 0.8 ([ink releases](https://github.com/inkle/ink/releases))；FFXIV 的放弃、重接、跳到下一个任务 ([FFXIV 5.3](https://na.finalfantasyxiv.com/lodestone/topics/detail/c2f8c5ba14d9cfaa09d5aab63c5c2da260eae21a))；Cyberpunk 2.0 重置并退还技能点 ([Gamepur](https://www.gamepur.com/guides/can-you-play-old-saves-on-cyberpunk-2077-2-0)) |
| 5 不兼容 | 以上都不适用 | 返回 `NeedsProgram` 或 `Incompatible`；可以走问答式的事实重建 | Telltale 的"故事生成器"提供 42 个起点 ([PCGamesN](https://www.pcgamesn.com/the-walking-dead-season-three/the-walking-dead-season-3-save-game-transfers)) |

这张阶梯背后的原则是"**跨版本迁移的是事实，不是执行位置**"。行业里成功的延续都只搬运一组声明好的事实：质量效应 3 可以读取 1,000 多个变量 ([Game Rant](https://gamerant.com/mass-effect-3-1000-variables/))，Dragon Age Keep 编辑 300 多项决定 ([Dragon Age Wiki](https://dragonage.fandom.com/wiki/Dragon_Age_Keep))。存档能经受内容修改的系统，都依赖显式命名的锚点。ink 的选择点和汇合点按位置递增编号，所以增删一个选择就会让旧存档错位 ([inky #253](https://github.com/inkle/inky/issues/253))。Ren'Py 只保存当前语句和返回栈，新加入的语句不会执行 ([Ren'Py](https://www.renpy.org/doc/html/save_load_rollback.html))。

Narrata 已经具备几个基础：ADR 0004 只允许在安全点持久化，身份是作者分配的稳定 ID，迁移基于 relocation。还需要补上四样东西：每个包声明后续包可读事实及其默认值的**包级导出清单**；`MigrationDescriptor` 中授予或退还事实、给新事实设默认值、向玩家显示通知的**补偿原语**；重命名与删除时的**墓碑和重定向**；以及充当回归预言的**黄金通关记录**。热重载不需要另造机制。文档已经规定它只在已提交的安全点发生，本质上与迁移是同一件事；落地时把第 1 级和第 3 级串起来即可。

关于事件溯源，这里需要说清楚。学术侧的综合建议"以事件日志为真相"。但 ADR 0006 和 `docs/architecture/time-travel-and-save.md` 已经决定以完整的版本化 Snapshot 作为恢复真相，并明确说明这不是纯事件溯源。Azure 的指南也指出，迁入事件溯源代价高昂，快照只是优化手段 ([Microsoft Learn](https://learn.microsoft.com/en-us/azure/architecture/patterns/event-sourcing))。所以本方案保留快照作为真相，把输入日志和 Receipt 用作审计记录、迁移验证和热重载快进的材料，不推翻已有决定。

LLM 生成的内容只能作为通过校验、带来源标记、被记录的事务进入事实存储，生成结果本身按 `LivePresentationOnly` 缓存。双时间维事实，即像 Zep 那样用失效标记代替删除 ([Zep](https://arxiv.org/html/2501.13956))，适合"在故事时间 T、截至提交 C，这个存档相信什么"这类查询，但应在主线完成后再评估是否值得。

## 落到仓库：保留验证资产，拆掉九个耦合点，分七步走

### 保留什么

应当原样保留、并作为后续一切工作基础的资产有这些：`narrata-core` 的纯度（只依赖 hex、sha2、serde、serde_json 和 thiserror，没有 I/O、时钟、随机数或异步）；ADR 0003 的规范 CBOR 配置；内容寻址对象、带修订号的引用 CAS 和显式事务（`crates/narrata-store/src/object.rs`、`store.rs`）；作为行为参考的 `MemoryStore`，连同 `FaultPoint` 故障注入和模型测试；ADR 0007 的效果账本、屏障和能力协商；ADR 0009 的迁移描述符、`MigrationRegistry`、dry-run 与 `fixtures/compat/stage5-v0` 冻结语料；bundle 的 `receiver_has` 增量导出；节点栈的包模型（显式导入导出、lock 文件、按包计算的摘要、与图分开按键存放的 content 映射）；`content.rs` 中的内容解析类型；以及 `ReconcileScene` 的声明式呈现思路。这些正是 `REBUILD_PROPOSAL.md` §9 列为"值得保留"的验证资产。

### 改什么

| # | 耦合点 | 位置 | 改法 | 是否触及冻结格式 |
|---|---|---|---|---|
| 1 | `SaveStore` 只有全量 `list_*`，没有范围或游标 | `crates/narrata-store/src/store.rs:310-369` | 拆成窄契约加引擎层；`list_*` 改为游标扫描 | 否，属于 API 层 |
| 2 | SQLite 每次读写都整库加载、删除全部行后重新插入 | `crates/narrata-store-sqlite/src/lib.rs:192-357, 762-800` | 改为行级读写，边表在写入时增量维护 | 否，但需要 schema v3 迁移 |
| 3 | 为计算边要逐个解码全部 Program | 同一文件 `:948-977` | 写入 Commit 时记录 program 对象的索引 | 否 |
| 4 | 每次加载都回溯到 Genesis；`redo_candidates` 做全扫描；`find_program_object` 是死代码 | `crates/narrata-store/src/coordinator.rs:1814-1889, 942-957, 2088-2102` | 父子索引键、generation 号、已验证水位 | 否 |
| 5 | `ProtocolEngine` 写死 `MemoryStore`，持久化只能走 bundle 字节 | `crates/narrata-protocol/src/engine.rs:75, 228, 329, 585` | 让存储可注入；跨 ABI 定义宿主存储回调或分页批次协议 | 需要升协议版本 |
| 6 | TypeScript 的 IndexedDB 存储是另一份独立实现 | `bindings/typescript/src/indexeddb-store.ts` | 改成契约在浏览器侧的适配器 | 否 |
| 7 | 文本进入常量池、`ProgramArtifactId` 和 Snapshot/`StateDigest` | `crates/narrata-core/src/program/wire.rs:20-31`、`program/validate.rs:227`、`snapshot/wire.rs:180-235`、`snapshot/restore.rs:725-753` | 用行 ID 取代文本；内容表单独寻址；构件增加 content lock 字段；接通 `ExternalContentDeclV0` 占位类型 | 是 |
| 8 | `SceneState` 强制存在，`ReconcileScene` 把场景字面量嵌进指令 | `crates/narrata-core/src/runtime/state.rs:24`、`scene.rs`、`program/instruction.rs:143-146` | 改为可选的模块状态（`REBUILD_PROPOSAL.md` §9；竞争架构文档 §3.4） | 是 |
| 9 | 节点栈：构件 ID 对含正文的整包做哈希；选项标签和 `disabled_reason` 内联；结束语是硬编码的中文；阅读器每一步都写入整份源 | `packages/narrata/nodes/crates/narrata-nodes/src/compile.rs:464-467`、`model.rs:176-188`、`runtime.rs:653-659`、`examples/gamebook-web/src/storage.ts` | 图摘要与内容包摘要分开；标签改为键；结束语改为内容引用；存档接入契约 | 是，涉及节点 JSON profile 的版本 |

两套引擎栈应当如何收拢，需要单独说明。ADR 0011 规定 `narrata-nodes` 不依赖旧的 `narrata-core`，旧接口今后通过显式 adapter 接回。所以存储契约必须放在一个不依赖 core 的新 crate 里，只依赖 sha2，只认识字节、摘要和键。这样两个栈都能使用它。

本报告的建议是**先在存储与身份层统一，暂缓合并内核**。两个栈都需要对象、引用和提交，统一这一层风险最低、收益最大。节点栈接入后，还能直接获得 CAS、GC、bundle 和故障注入测试。

### 分阶段顺序与验证标准

| 阶段 | 范围 | 验证标准 |
|---|---|---|
| P0 测量与决策 | 新写一份 ADR（例如 0012），记录本报告的边界决定。补充基准：SQLite 在 1K/10K 次提交下的提交与加载延迟；10^5–10^6 条指令或常量的 Program；节点栈在 4 MiB / 4,096 节点上限处的表现；Wasm 峰值内存 | 基准纳入与 `scripts/benchmark-g2.ps1` 相同的流程并留档，P1 前后可以对比 |
| P1 存储契约 | 新建契约 crate；实现 `MemoryStore` 和行级 `SqliteStore`；把账本、目录、GC、完整性扫描改写在契约之上；把 `tests/model.rs` 和 `tests/faults_gc.rs` 抽成可对任意后端运行的一致性套件 | `fixtures/compat/stage5-v0` 的所有哈希不变；10K 次提交时单次提交延迟与 1K 次时处于同一量级；加载单个提交不触发全对象枚举（用计数包装器断言） |
| P2 内容身份拆分 | 引入行 ID 与内容表；逻辑构件 ID 不再包含文本；Snapshot 不再存文本；构件增加 content lock；把 `content.rs` 的解析路径接到协议边界；定义呈现事件协议 | 新的冻结语料（如 `stage6-v0`），且从 stage5-v0 迁移的 dry-run 通过；属性测试：只改文本时逻辑 ID 和 `StateDigest` 都不变，切换语言不改变 `StateDigest`；fuzz 目标覆盖新格式 |
| P3 节点栈接入 | Gamebook 存档写入对象/引用模型；阅读器只保存存档，不再保存整份源；内容包与图分开；去掉硬编码文本 | 修改 `products/gamebook-demo/packages/road.json` 中的一段正文后，旧存档能从原节点继续；Playwright e2e 覆盖这一场景；`narrata-book` 产物中的图摘要不随正文变化 |
| P4 分块与懒加载 | Program 和产品改成"清单加包级块"；按可达性加载；`ProgramLoad` 分块传输；归档与墓碑 | 用 testkit 的生成器构造 10^4 个包、10^6 行的合成作品：峰值内存与活跃包数成正比，而不是与作品总量成正比；冷包归档后没有悬空的事实引用 |
| P5 迁移阶梯与热重载 | 重放分歧检测、安全点重入、事实补偿、玩家可见通知、包导出清单；热重载走同一路径 | 每个包都有黄金通关记录；CI 运行"旧存档 × 新内容"矩阵，并按阶梯级别报告结果 |
| P6 更多后端与投影 | PostgreSQL 适配器、JSON 目录适配器、图投影导出（AGE/Neo4j）、浏览器 OPFS | 新后端通过 P1 的一致性套件和故障注入；投影删除后能从真相源重建且校验一致；JSON 后端在并发写入时被引擎拒绝 |

P1 和 P2 的先后顺序是有意安排的。P1 不改任何冻结字节，可以独立交付，并为 P2 提供可靠的存储基础。P2 是唯一必须新写格式 ADR、新建冻结语料的阶段，ADR 0009 规定冻结语料是只读的兼容证据，不能原地修改。P6 排在最后，是因为契约稳定之前就加后端，只会把契约现有的缺陷复制到更多地方。

## 风险、反方观点与未决问题

### 最强的反方观点

第一种反方观点认为"图就是核心"。articy、Arcweave、CDPR 的 questphase 和 BBC 的 StoryFormer 都以图为创作中心，图对作者也最直观。本方案并不取消图，只把图放回创作视图和单元内部的控制流。真正要防止的，是竞争架构文档 §5 提出的"图优先还是文本优先"这个问题悬而未决时，出现两份互相漂移的真相。

第二种认为"文字与逻辑不可分"。inkle、Failbetter、PLOTSHOT/BIPOCL 的证据都是真实的，本方案通过"创作时并置、编译时分离、维护一个同步更新的参考渲染器"来接住这一点。但代价不会消失：ID、锁定哈希和元数据管线都是额外工作。研究也表明，元数据经常在制作流程的后段丢失 ([Weller et al. 2024](https://arxiv.org/abs/2407.19590))。

第三种认为"直接用图数据库或 PostgreSQL 当运行时存储更简单"。深度遍历上图数据库确实更快。但 Narrata 的运行时负载是浅读加追加写，深度查询属于作者侧分析，放在投影里更合适。

第四种认为"事件溯源天然带来后端无关" ([Kleppmann](https://www.confluent.io/blog/turning-the-database-inside-out-with-apache-samza/))。这个观点在原理上成立，但它与已有 ADR 冲突。实践者也报告过代价：第一个额外投影就会让触碰事件流的代码量翻倍，重写日志则会失去复现历史状态的能力 ([Kiehl](https://chriskiehl.com/article/event-sourcing-is-hard))。

第五种认为"媒体也会影响规则"。这一点成立，所以本方案让这类资源进入语义锁，而不是坚持"完全分开"。

### 主要风险

**过度抽象**会把作者变成记账员。Kennedy 回顾了统一 quality 的教训，Versu 的作者说调优众多并发情境的效用值困难而且没有回报，Bruno Dias 警告叙事 DSL 容易膨胀成通用编程语言 ([Dias 2017](https://brunodias.dev/2017/05/30/an-ideal-qbn-system.html))。应对办法是提供类型化的领域原语、检查器和确定性回放工具。

**格式变动的成本很高**。P2 和 P3 都要求新格式 ADR、新冻结语料和迁移，两个栈会让这份工作加倍。

**团队带宽有限**。仓库只有 15 次提交，又计划拆分出 Studio、Web Player 和媒体管线。Netflix 自建整套栈、最终全部下架的结局是一个警示。

**浏览器驱逐**会整源清空存档，Safari 有 7 天规则，所以内容寻址的导出和同步不是可选项。

**放宽加载时的全量校验**可能削弱"行与字节在通过检查前不可信"的纪律（`crates/narrata-store/src/lib.rs` 的头注释）。"已验证水位"必须只由引擎自己的校验写入，导入时必须重新校验。

**LLM 渲染不确定**。需要按"事件、渲染器、模型版本、种子"缓存生成结果，否则回退后的重新渲染会和之前不一致。

### 未决问题

以下问题本报告无法给出确定答案，需要所有者决定，或需要进一步证据。

| 问题 | 为什么还没有答案 | 建议的决策时点 |
|---|---|---|
| 长期以哪套栈为内核 | `REBUILD_PROPOSAL.md` 指向 R 系列，而 Stage 1–5 掌握着大部分验证资产 | P3 完成、两栈共用存储层之后 |
| 叙事规模的数值目标写进哪份文档 | 现有文档没有任何数值目标，上文的目标只是提案 | P0 |
| 存储契约用同步还是异步 | 现有 trait 同步、写入取 `&mut self`，适合嵌入式，对 PostgreSQL 的网络 I/O 却很别扭 | P1 设计评审，最迟 P6 |
| 讲述层与"未兑现承诺"放在第一方包是否足够 | 证据支持把它们当作状态，但没有证据说明它们是否需要内核专门的状态类型 | R2/R3 领域包设计时 |
| 语法变体用 MessageFormat 2、Fluent，还是完全交给 REZICS | REZICS 拥有本地化；Rust 侧 MF2 crate 的成熟度没有评估 | P2 |
| `StructureOccurrence.occurrence: u32` 是否稳定 | `REBUILD_PROPOSAL.md` §9 已标出这一疑问 | P2 |
| 现有迁移能否接受空 relocation 的描述符（A→B），作为 P2 之前处理文本修订的临时方案 | 尚未验证 | P0 |
| 双时间维事实是否值得做 | 业界没有单个存档事实规模的公开数据 | P5 之后 |
| 严格拒绝未知字段与扩展生态如何并存 | ADR 0003 的严格性能防止旧阅读器损坏新存档，但长寿命生态需要命名空间化的扩展字段 | P2 格式 ADR |

## 结论

"叙事引擎的核心是什么"，可以换一个问法：当存储、媒体和时间全都会变时，什么必须保持不变？答案是身份，以及建立在身份之上的类型化事实、确定性转移和提交历史。用户的三个直觉其实指向同一个工程杠杆。存储能否插拔，取决于内核是否只对后端提出五个原语加能力声明。媒体能否分离，取决于文本是否退出逻辑身份。超长叙事与动态更新能否成立，取决于身份是否按包分块、执行位置是否可以由事实重建。三件事都在身份层，而不在后端的数量或种类。

由此得出一个与直觉相反的优先级：Narrata 现在最需要的不是 PostgreSQL 或图数据库适配器，而是把 `SaveStore` 改成可扫描的窄契约，并把文本从 `ProgramArtifactId` 和 `StateDigest` 中移出去。完成这两步之后，加后端是常规工作。做不到，加多少后端都会继承整库重写和"改一个字就让存档失效"这两个缺陷。在本轮调研覆盖的引擎中，没有一个公开描述过跨内容修订的确定性重放。Narrata 已有确定性内核、提交图和迁移描述符，有条件把"在活着的存档之下持续演化的叙事"做成自己的核心卖点。这比"支持多少种数据库"更值得写进定位。
