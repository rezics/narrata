# 叙事节点抽象：真实实现与学术研究依据

日期：2026-09-05。性质：架构研究与候选方案依据，不是已实施规范。

研究目标是支持从交互式 Gamebook、VN，逐步发展到 Skyrim 式世界任务/情境和 Baldur’s Gate 3 式叙事/同伴能力。比较标准是目标覆盖、语义清晰、可组合、可制作与可验证，不要求制造竞品没有的概念。

本次具体查阅了 BG3 官方工具文档及示例、三个公开项目的固定提交源码，以及论文相关章节。没有读取 Bethesda/Larian 私有引擎源码，没有在本机运行 Skyrim/BG3 Toolkit，也没有运行这些第三方项目的测试。公开格式工具的实现是其自身的第一手证据，不能冒称游戏原厂的全部内部实现。

## 1. 实际实现提供的依据

### 1.1 Skyrim：Scene 是有参与者和过程结构的记录

检查 Mutagen 的 Skyrim 格式实现，固定提交为 `dfc1bc03ce42a3428d3596739430af14f846112d`。该项目是处理 Bethesda mod 数据的公开库，不是 Bethesda 引擎源码。

| 文件 | 实际观察 | 可借鉴的抽象 |
| --- | --- | --- |
| [Scene.xml](https://github.com/Mutagen-Modding/Mutagen/blob/dfc1bc03ce42a3428d3596739430af14f846112d/Mutagen.Bethesda.Skyrim/Records/Major%20Records/Scene.xml) | Scene 具有 Phases、Actors、Actions、Quest 引用和 Conditions；阶段具有开始/完成条件；动作关联 ActorID、起止阶段 | Scene 可以是有角色绑定和阶段结构的复合节点，不能只理解为画面快照 |
| [Quest.xml](https://github.com/Mutagen-Modding/Mutagen/blob/dfc1bc03ce42a3428d3596739430af14f846112d/Mutagen.Bethesda.Skyrim/Records/Major%20Records/Quest.xml) | Quest 有阶段、目标、aliases；alias 含条件、对象来源、Keywords 等字段 | 任务、目标、角色引用与场景既关联又各有类型 |
| [StoryManagerNodes.xml](https://github.com/Mutagen-Modding/Mutagen/blob/dfc1bc03ce42a3428d3596739430af14f846112d/Mutagen.Bethesda.Skyrim/Records/Major%20Records/StoryManagerNodes.xml) | Story Manager 有共同基类以及 Event、Branch、Quest 三种节点；基类含条件和层级链接 | 公共节点身份不妨碍不同节点拥有专门语义；节点模型并不必然等于对白图 |

数据字段本身不证明运行时的完整调度算法、条件求值时机或抢占保证。本次不从这些格式推导未验证的内部行为，也不把 Quest 阶段编号照搬为所有 Narrata 任务的必需模型。

### 1.2 BG3：世界规则、行为、剧情与日志各有职责

BG3 官方 [Osiris 入门](https://docs.baldursgate3.game/index.php?title=Scripting:_Introduction_to_Osiris)描述了事件、DB facts、规则与调用，脚本由有 INIT/KB/EXIT 的 goals 组织。它用于对世界事件作出反应，并不要求从某个对白入口顺序运行。官方 [Story Editor](https://docs.baldursgate3.game/index.php?title=Osiris:_Using_the_Story_Editor)展示了实际 goal 层级及 Act/Region/Situation 的组织方式。

[Anubis 的官方节点示例](https://docs.baldursgate3.game/index.php?title=Anubis:_Modifying_Simple_Behaviours_with_Interruptions_and_Selectors)则包含 Action、Selector、CanEnter、Valid 和事件处理；持续有效条件改变可导致行为中断并重新选择。它说明“可进入”与“执行中仍然有效”是不同条件。

官方 [Journal 结构](https://docs.baldursgate3.game/index.php?title=Journal_Structure_Overview)区分 Quest、Objective 和 Step，并允许一个任务跨多个 situation。日志显示的阶段也不能一概视为世界事实的完整真相。

对 Narrata 的推论：可以用统一类型化节点与引用组织这些东西，但要保留事件规则、持续行为、作者内容、目标和显示投影的语义边界。不能把所有节点都解释成“读完后走 next”。

### 1.3 BG3 同伴：身份、控制、关系与剧情事件分别建模

官方公开接口包含：

- [RegisterAsCompanion](https://docs.baldursgate3.game/index.php?title=RegisterAsCompanion)：招募状态可对应加入队伍或留在营地。
- [AddPartyFollower](https://docs.baldursgate3.game/index.php?title=AddPartyFollower)：跟随者与完整玩家角色不同。
- [AssignToUser](https://docs.baldursgate3.game/index.php?title=AssignToUser)：角色控制权由用户分配，不等同于人物身份。
- [GetApprovalRating](https://docs.baldursgate3.game/index.php?title=GetApprovalRating)：关系值包含评价者与被评价者；官方示例将它与营地状态连接以触发剧情。
- [ApprovalRatingChanged](https://docs.baldursgate3.game/index.php?title=ApprovalRatingChanged)：关系变化作为事件，并结合其他状态判断后续剧情。
- [CharacterLeftParty](https://docs.baldursgate3.game/index.php?title=CharacterLeftParty)：离队事件示例会调整相关对白入口。

这些证据足以说明伙伴叙事需要持续身份、关系事实、生命周期和事件联系；不足以证明我们已经完整重建 BG3 的同伴实现。也没有理由要求 Narrata 默认照搬 BG3 的关系维度或组队规则。

### 1.4 LSLib：同一种“Node”外形下仍有不同类型

检查固定提交 `6f5f6987d10d8485876b8368431c948e4dea4f83`：

- [Node.cs](https://github.com/Norbyte/lslib/blob/6f5f6987d10d8485876b8368431c948e4dea4f83/LSLib/LS/Story/Node.cs)的节点类别包含 Database、Proc、Query、And、NotAnd、RelOp、Rule 等；数据库归属也有检查。
- [Compiler/IR.cs](https://github.com/Norbyte/lslib/blob/6f5f6987d10d8485876b8368431c948e4dea4f83/LSLib/LS/Story/Compiler/IR.cs)分别建模 IRGoal、IRFact、IRRule、条件、动作与变量。

这是一套面向 Larian 数据与脚本的工具实现，可验证“统一节点标识＋专用类型语义＋编译分层”是实际采用的模式。它不证明 Narrata 应采用 Osiris 的具体节点集合，也不等于 BG3 原厂 compiler 源码。

### 1.5 Anansi：标签、查询、实例化是不同机制

检查固定提交 `daffc1222f89e5a13927b93be90e6e5bc7b3587a`：

- [Storylet.cs](https://github.com/ShiJbey/Anansi/blob/daffc1222f89e5a13927b93be90e6e5bc7b3587a/Packages/com.shijbey.anansi/Runtime/Storylet.cs)包含标签、前置条件、输入绑定、重复/冷却策略、权重与 Ink 内容入口。
- [StoryletInstance.cs](https://github.com/ShiJbey/Anansi/blob/daffc1222f89e5a13927b93be90e6e5bc7b3587a/Packages/com.shijbey.anansi/Runtime/StoryletInstance.cs)将一个 storylet 与具体参数绑定配对。
- [StoryletPrecondition.cs](https://github.com/ShiJbey/Anansi/blob/daffc1222f89e5a13927b93be90e6e5bc7b3587a/Packages/com.shijbey.anansi/Runtime/StoryletPrecondition.cs)包含查询及输出变量。

它对 Narrata 最直接的启发是：可复用定义、满足条件的候选绑定、实际运行实例应该分开；tag 只是组合的一部分。该实现依赖 Ink 和 Unity，不能直接当作 Narrata 的跨平台状态/身份协议使用。

## 2. 学术研究支持什么，不支持什么

| 研究 | 查阅内容与启示 | 本方案的使用边界 |
| --- | --- | --- |
| Kreminski / Wardrip-Fruin，ICIDS 2018，[Sketching a Map of the Storylets Design Space](https://mkremins.github.io/publications/Storylets_SketchingAMap.pdf) | 将前置条件、可重复性、内部结构和选择机制作为独立维度；讨论查询并绑定角色/物品的参数化 storylet | 支持有条件、可组合叙事单元；没有证明一种 tag 或节点粒度适用于全部作品 |
| Mateas / Stern，2003，[Façade 架构与作者经验](https://faculty.cc.gatech.edu/~isbell/classes/reading/papers/MateasSternGDC03.pdf) | beat 内包含行为/子目标，反应可插入过程；讨论顺序、并行、同步和行为资源冲突 | 支持复合节点、打断与局部控制；不照搬其完整 drama manager 或自然语言系统 |
| Evans / Short，[The AI Architecture of Versu](https://versu.com/wp-content/uploads/2014/05/versu.pdf) | 情境提供角色和行动机会，人物选择行动；支持多个情境共存 | 叙事场景应能绑定参与者并与人物状态协作；不强制使用同一种效用系统 |
| Riedl / Young，JAIR 2010，[Narrative Planning: Balancing Plot and Character](https://faculty.cc.gatech.edu/~riedl/pubs/jair.pdf) | 将事件因果可成立与角色行动有意图分别处理；规划出可达结果并不足以自动产生可信人物 | 保留条件、后果、目标与动机的表达；第一阶段不建设通用自动剧情规划器 |

共同的工程启示是：叙事片段的内容、适用条件、角色绑定、执行结构和选择政策应可分别表达。研究没有给出“所有叙事统一成带字符串 tag 的节点”这一唯一最佳答案。本方案采用有类型的可组合节点族，是根据目标和这些证据作出的设计判断。

## 3. 推荐的抽象与可替代方案

推荐建立 **类型化、可组合的叙事定义图＋共享领域状态＋活动实例执行**。

| 选项 | 判断 |
| --- | --- |
| 所有东西都是同一种流程节点 | Gamebook 容易起步，但持续任务、关系和可中断活动会被迫展开成流程状态 |
| 所有东西都是 tag 和属性 | 容易拼接数据，却无法单靠标签定义如何执行、何时提交、怎样失败和恢复 |
| 每个叙事子系统完全独立 | 各自容易实现，但跨内容包组合、引用和存档需要重复协调 |
| 统一身份/类型/组合契约，各节点类型拥有自己的数据和行为 | 推荐；既能统一组织与工具，又保留 Scene、Quest、Character 等不同语义 |

作者定义图不决定物理数据库和所有执行算法。GraphFragment 可以编译为既有 Flow IR、事件规则或节点处理器；纯数据引用不需要进入执行队列。共享的是可检查的语义与身份，不要求所有系统使用一种存储容器。

## 4. SceneState 从哪里自然产生

候选关系：

```text
NodeType
  → DefinitionSchema
  → 可用组合端口与行为契约
  → StateSchema（该类型需要持久状态时）

NodeDefinition<Scene>
  → 角色参数 + 内部子图/过程 + 进入/退出/打断规则

NodeInstance<Scene>
  → SceneState
     已绑定参与者、子活动进度、局部状态、等待和占用关系
```

这实现了“Scene 是节点族中的一种类型，SceneState 随该类型的实例自然产生”。它不再是每个 Session 都必须包含的特殊二维画面字段，也无需由标签去猜它是什么。

旧 SceneState 中的立绘位置、相机和音轨保留为可选呈现节点/组件状态。一个 Scene 可使用它们，也可只包含文本和行动。持久角色关系属于共享领域状态，通过引用使用；不能复制进每次 Scene 然后各自变化。

所谓自然产生，是根据明确的节点类型、绑定和执行规则构造并推进状态，不是自动从任意正文推断一套可靠 schema。schema 可以由类型/声明生成，正确性仍需契约与测试。

## 5. 组合的层次与标签的候选职责

必须区分：

1. **结构装配**：把片段的入口、结果、角色参数和所需能力绑定起来；可在构建时检查。
2. **运行选择**：从候选片段中筛选满足当前事实和绑定条件的实例；可能由玩家或 selector 决定。
3. **执行与后果**：运行被选中的片段，提交结果、更新事实并产生事件；定义失败、取消和恢复行为。

可支持的连接包括顺序、调用/返回、条件选择、事件触发、角色/数据引用。它们要有可区分的类型，不能把所有箭头都理解成“下一节点”。子图可封装并导出少量端口，使三套独立节点包通过配置组合。

标签暂不冻结。第一阶段可以仅保留可选、带命名空间的作者注释；需要查询/选择时再决定是否采用注册标签、分类集合或类型化属性。关系/生命状态/承诺等通常用可查询领域事实表达，而不默认全部变成标签。

若标签参与运行时选择，它就是行为相关数据，必须参与相应组合身份/版本判断；纯编辑器颜色或分类不必如此。这一边界应由实际消费者决定，不能永久把所有 tags 当作非语义 metadata。

候选角色在绑定后可能死亡、离队或被另一活动占用。需要绑定有效性、占用范围和失效分支；“类型能接上”不证明未来所有运行状态下都能执行。叙事文学上的连贯仍由作者规则和作品验证负责。

## 6. 用什么案例检验可以向复杂作品发展

这些是未来架构测试目标，不声称本次已实现：

| 案例 | 暴露的抽象要求 |
| --- | --- |
| 先得到物品，再遇到发布任务的人 | 目标按世界事实满足，不强制按日志顺序经过每一节点 |
| 任务发布人死亡或离开 | 角色绑定与失效政策，替代角色/失败/继续路径 |
| 两个事件同时需要同一同伴发言 | 活动占用、优先级、排队/取消，避免两个控制器同时控制人物 |
| 同伴离队但仍存活，后来重逢 | 人物身份、队伍关系、记忆和内容入口分别存续 |
| 同伴被不同用户控制 | User、Character、Party、Relationship scope 不混用；多人支持以后实现 |
| 对话进行中发生攻击 | 进入条件与持续有效条件分开；中断并保留或取消局部状态 |
| 内容包中的一段互动被不同人物复用 | 参数化定义与实例绑定；人物动机/知识限制内容选择 |
| 主线、同伴、环境事件三个包组合 | 显式端口/事件/事实约定、局部状态隔离和来源追踪 |

## 7. 实施取舍

先交付可运行的 Web Gamebook：内容节点、选择、条件、局部/共享状态、可复用子图和保存恢复。用三个独立小型内容包证明配置组合。第一阶段只实现实际需要的节点类型，不要求作者先建立人物模拟。

随后用同一机制加入 VN Scene/Dialogue 节点及基础媒体。再扩展事件驱动任务、关系、同伴生命周期、可中断活动和更复杂选择策略。复杂目标用于阻止错误的基础假设，不能成为推迟第一个 Gamebook 的理由。

首次实现前仍需通过小型 executable spec 决定：节点状态的类型注册/存储布局、子图调用与返回、局部/共享事实边界，以及流程驱动和事件驱动如何共存。标签本体、任意热替换和自动剧情规划不进入首批冻结范围。
