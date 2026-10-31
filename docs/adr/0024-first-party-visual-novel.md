# ADR 0024：第一方视觉小说语义与兼容边界

状态：Accepted（2026-10-31，Web VN Goal manager 审阅后接受）。本页决定实现方向与兼容边界；此处所述 VN 行为、格式和公开 API 尚未实现或检查，接受设计不代表能力已经交付。

本 ADR 依据[决定 2、11、14、17、18](../product/decisions.md)，为 Web VN VN0 选定一条实现路线。它只决定 Narrata 自己的执行、状态、资源引用及兼容边界；不增加 RuntimeStateV0 能力，不实现播放器、编辑器、宿主媒体服务或 VN 完整产品。

## 当前基线与约束

当前完整 Gamebook 基线是节点栈。`SourcePlan` 只有 Passage、Branch、Mutate、Call、Return；`Plan` 对应这些受检执行形式。Passage 有标题引用、正文 Segment、参数、ChoicePoint 和后继节点，选项用稳定 ID、条件、效果及局部或分支结果表示。见[源稿类型](../../packages/narrata/nodes/crates/narrata-nodes/src/source.rs)与[检查后计划](../../packages/narrata/nodes/crates/narrata-nodes/src/plan.rs)。[NodeRegistry](../../packages/narrata/nodes/crates/narrata-nodes/src/registry.rs)的编译器回调只在构建期把作者节点降低到 SourcePlan；执行器仍只运行 Narrata 检查过的计划。

节点[State 与 Input](../../packages/narrata/nodes/crates/narrata-nodes/src/state.rs)保存图帧、节点、当前选择点、变量和结束状态，交互输入当前是 Choose 或 Propose。[Session::advance](../../packages/narrata/nodes/crates/narrata-nodes/src/session.rs)要求调用方提供期望父 Commit，并在当前游标不匹配时拒绝 stale input；输入解码还会按当前 State 检查选择点和选项。[SessionView](../../packages/narrata/nodes/crates/narrata-nodes/src/view.rs)输出 title、body、reply 呈现项和 choose/finished 交互。它们提供可复用的节点、历史、分支与持久化基础，但没有 VN 的逐句安全点、说话者或场景目标。

公开 [Book API](../../packages/narrata/web/src/runtime.ts) 只有 choose、checkout、保存与恢复等 Gamebook 操作；[ContentResolver 结果](../../packages/narrata/web/src/content/resolution.ts)是文字或文字块。当前 `ContentRef`/`Segment` 与正文解析契约不构成媒体类型，不能把它们断言成图片或音频。浏览器端还已有有界正文预取、取消过期解析和保存失败后的单独重试模式，见[阅读器控制器](../../packages/narrata/web/src/reader/controller.ts)。

旧 Flow 栈已具有 `Say`、`Choice` 和 `ReconcileScene`；交互只接受当前 Interaction ID，空 evaluation stack 是 safe point，旧测试覆盖对白推进、分支/调用、说话者 View 与 snapshot restore 后继续。旧[SceneState](../../crates/narrata-core/src/scene.rs)表示背景/图层、角色、音轨与当前交互，[RuntimeStateV0](../../crates/narrata-core/src/runtime/state.rs)保存场景状态。[决定 11](../product/decisions.md#11-节点组合模型是长期内核的语义基础)已规定不再向 RuntimeStateV0 增加产品能力。旧 Flow 和 Scene 可作为行为证据，但本 ADR 不把它扩展成 VN 执行栈，也不把它直接当作节点格式的状态类型。

## 决定

### 1. 在节点栈上增加第一方的类型化 VN 语义

VN 程序采用节点图执行器上的第一方类型化语义扩展：作者计划、检查后的 Plan、状态、输入与 View 都能表达对白交互和可恢复场景目标；既有 Node Session、Commit/历史分支、存档与按图块加载继续承担执行和恢复。现有 Branch、Mutate、Call、Return 及稳定 ID 的 ChoicePoint/Option 仍负责控制流与选择。新增语义是 Narrata 编译器和检查器支持的闭合类型，不是故事回调、运行时插件或宿主专用节点。

比较过的路线：

- **选定：扩展第一方节点语义。** 需要跨源稿、受检计划、状态、输入、View、确定性编码与 Web 声明的实现，成本高于只拼装现有节点；但它可以让说话者、当前对白位置和场景状态保持类型化，并由同一个确定性 Session 提交和恢复。
- **不选：仅复用 Gamebook lowering。** 可以把每句伪装成一个 Passage，并为每句造一个单选“继续”选项；这会把对白推进误报成普通选择，用 Passage title 冒充 speaker，并且仍然没有背景、角色层或音轨的持久目标。宿主不能从类型上区别推进与选择，也不能从存档恢复场景而不重建额外约定。它不满足 VN1 的公开类型、可恢复呈现和参考播放器验收。
- **不选：扩展旧 Flow / RuntimeStateV0。** 虽有 Say/Choice/Scene 先例，但产品决定 11 已冻结该扩展方向；它也不会自然复用当前 Gamebook 的发布、节点程序块与 Session 历史路径。

因此 VN 使用一套节点图解释器及 Session，不另建通用 VM、平行历史、插件锁或运行时工厂。节点图分支与汇合仍以现有稳定结构 ID 表示；自定义作者编译器只有在编译成第一方受检 VN 计划后才能运行。

### 2. 逐句对白与安全点

对白行是一个独立的逻辑交互安全点。View 暴露当前行的正文引用和可选说话者名称引用；行推进是与选项选择分开的类型化输入。每次有效推进最多越过当前这一行，并停在下一对白行、选项交互或结局；自动节点步可以在一次输入内执行，但不能跳过中间对白安全点。初始执行也在首个交互处停止。

每个输入都绑定到当前执行、期望的父 Commit 和受检 View 中的当前交互。对白推进携带当前对白交互身份；选择携带当前 ChoicePoint 与有序的稳定 Option ID。一次有效输入对 State、对白位置、场景目标和分支目标原子地产生一个新 Commit，再由播放器呈现新 View。内容或媒体失败不会撤销已确认的 Commit。

输入通过当前 Session 的 expected-Commit 检查。`Session::advance` 在进入历史层前先要求 expected Commit 仍是当前内存游标；游标已移动时，即使重发的是完全相同的旧请求，也会返回 stale，而不会自动确认旧结果。若游标仍停在预期父 Commit，历史层可以复用该父状态下已记录的同一规范 Input 对象，不再执行一次转换。VN 的可观察规则是：同一行的第二次点击若在第一次已移动游标后到达，得到 stale，不能越过下一行或产生第二个转移；调用方重新读取 View 后再决定动作。

持久化结果未知时，在线存档层重试/核实的是原始写入字节；它不再次调用 `choose` 或 `advance`。因此“当前父状态可复用同一转移”“游标已移动时旧提交得到 stale”“未知写入重试原字节”是三件不同的事。本 ADR 沿用[ADR 0005](0005-input-idempotency-ownership.md)的职责边界，但不据此承诺旧按钮请求在游标移动后自动得到原 Commit，也不新增 RPC 幂等 API 或身份/载荷冲突约定；R2 Input 的对象身份由其规范载荷摘要确定。

存档、回退和重开恢复到同一执行、构件和语义交互位置；从旧 Commit 继续可形成另一历史分支。它们恢复对白行与当前选择，而不是重新执行此前的规则转换、动画序列或宿主副作用。VN 初版不提供外部 Effect；今后若增加影响故事的宿主效果，必须另行决定其提交前后时序、幂等键、恢复与回退策略。

### 3. 可恢复场景与简单音轨策略

场景是持久化的声明式呈现目标，不是逐帧动作日志。最低语义包括背景、带稳定层身份和明确层序的角色呈现，以及当前对白位置；资源只以受检引用表示。进入新场景时，新视觉目标完整替换旧背景和角色层，未列出的视觉层被清除。播放器可在首次进入时加局部装饰性过渡，但 checkout、恢复或重开直接协调到当前完整目标，不重播历史动画。

初版区分一条可替换/停止的持续音乐轨与绑定到当前对白行的单次语音。新场景默认清理上一行语音，音乐则跨场景延续，直到叙事明确替换或停止它；替换从曲目开头起播，停止移除当前播放意图。状态记录当前音乐引用和播放意图，不保存播放头或声波/解码状态。保存后恢复时，持续音乐从头重启；当前对白行的语音若启用也从头重启，先前对白的语音不重放。初版不做 seek、不承诺采样级同步或精密导演时间轴。

浏览器可能要求用户手势后才能播放。尚未解锁时，View/播放器保留播放意图，显示“开启声音”操作；用户手势只解锁并从头启动当前应播放的音轨，不移动叙事游标。解码或授权失败保持场景和对白位置不变，可重试或静音继续；音频状态不决定分支。

### 4. 正文、名称和媒体的宿主边界

正文、说话者显示名称与选项文字继续通过 `ContentRef` 或 `Segment` 引用，媒体通过指向资源版本的有类型引用表示。引用属于 Narrata 的结构：现有 Passage Plan 保存 title/body 引用，Option Plan 保存 label 引用；它们进入检查后的程序对象与相应 artifact identity。若内容引用、媒体引用或动态 `Scalar::Ref` 值被存入 State/场景目标，该受检引用值也进入相应 State 编码与摘要；改变引用可能改变 Program、State 及其后续 Commit 身份。不要把“正文是宿主内容”解释为引用不参加 Narrata 身份。

宿主仍拥有解析出的正文、说话者名、选项标签、本地化实现和媒体字节。相同的文本引用可按宿主选择的语言/实现策略解析为不同文字，而这些解析字节不进入逻辑对象；变更程序中记录的引用则会改变相应逻辑对象。媒体版本身份参与引用它的程序或场景状态，实际字节、授权地址、DOM 元素、音频上下文/播放器句柄、解码缓冲、播放头以及用户手势授权属于播放端或宿主，不能进入持久叙事状态。

Web 媒体边界必须有独立的媒体描述/解析结果类型，明确资源类别、不可变修订和受检变体；现有只返回文字 Payload 的 ContentResolver 不通过断言、`unknown` 到 media cast 或未检查字段承载媒体。Narrata 只消费已验证的资源描述，访问控制和授权 URL 由宿主在实际请求时提供。

### 5. 资源准备与逻辑提交分离

执行推进可按需读取并受检装载必要的逻辑程序块；它不以正文/媒体的网络解析、解码或资源授权就绪为提交前提。新叙事安全点提交后，其逻辑位置已确定。播放器可在当前场景之外，有界准备当前 View 和各直接候选走向所需的剧情块、文字与媒体，不能因预取走遍整部作品。游标、分支、账号/授权范围或内容解析上下文变化时，取消过期请求并使其代次失效；晚到结果只有在当前 Session/代次、artifact、资源版本以及宿主提供的授权与内容上下文都仍匹配时才可进入 View 或缓存。artifact/asset/version/variant 身份单独不足以证明某个私有结果可以跨账号或解析上下文复用。离开当前场景且不在预取范围的解码/播放资源可释放。

首次打开时，必要的当前对白正文和场景视觉资源未就绪，则显示明确的加载/不可用面板；从已有画面推进后，目标资源未就绪则保留上一完整画面并用非交互遮罩说明等待或失败。两种情况下都保留已提交的语义位置、禁用后续叙事输入，并提供资源重试；可选音频失败则允许静音继续。失败不被显示成结局。重试只重做资源解析/加载，不复用或重新发出对白推进/选择输入。

媒体引用表示不可变的资源版本：资源字节或版本改变时必须使用新的资源身份；A 与 B 确实引用相同不可变资源时可以共享该身份和资源，不要求发布 B 时复制未变资产。artifact ID 固定逻辑程序，不固定宿主选择的正文语言/实现或授权政策。媒体解析必须按当前程序中的确切资源版本执行；缺失时报告 unavailable，不能用 `latest`、可变别名或同名的 B 资源代替 A。解析后缓存或呈现还必须通过当前授权与内容上下文检查。

资源操作归属于播放器当前执行、Commit/artifact、宿主授权/账号或 library 上下文及内容解析上下文。宿主应以类型化上下文和请求代次提供这些边界，不用未受检的拼接字符串当成身份。账号、权限或 viewer/language/realization 上下文切换时，递增/替换活动代次并取消旧解析、下载和解码；晚到结果不得呈现，私有结果也不得进入新上下文的缓存。跨上下文复用仅在宿主明确证明相同授权范围和解析策略时允许；公共且不可变的字节也仍须遵循宿主的缓存政策。凭据和签名 URL 不进入 State、View 或长期缓存身份。

关闭 reader/session 时，播放器取消所有 session-owned 预取、阻止待完成回调发布结果、停止并释放播放器/解码资源与临时 URL；账号/library 关闭或删除时，按宿主政策清除其私有资源缓存和授权上下文。资源清理与 Narrata 存档生命周期分开，清除媒体/程序缓存不得删除未同步存档写入；存档删除仍是独立的显式操作。

这要求宿主向按需 resolver 提供有界批次、取消/过期判定、重试和资源释放接缝。REZICS 的媒体身份、版本清单、独立交付及按场景预取请求已记录在[集成文档 Web VN 能力节](../integrations/rezics.md#web-vn-后续需要的宿主能力)；本 ADR 不修改宿主实现，也不要求大媒体 Range 或整部作品离线下载。

### 6. 新格式、读取路径和冻结证据

当前 Gamebook 基线为源/组合格式 2、节点对象 envelope schema 1、内建 `GAMEBOOK_REVISION = 2`；现有源稿、固定兼容向量、受检程序对象、State/Input 和 Gamebook `BookView` 语义构成必须保留的读写路径。对同一份既有 Gamebook format 2 源稿继续编译时，内建 revision 2 的受检行为、程序身份/对象字节和 schema 1 存档读取必须保持不变。

VN 使用明确选择的新源格式 3；新 VN 源稿选择第一方 VN 语义版本，而 Gamebook 节点仍选择 `GAMEBOOK_REVISION = 2`。只有实际增加/改变 payload 的对象种类才采用新的 schema 2 编码；未改变的对象种类不因 VN 存在而整体重写。具体字段/tag/CBOR map 由实现提出，本 ADR 未批准任何新字节布局。读取和编译需按明确的源 dialect、内建节点 revision、对象 kind/schema 与 artifact 语义版本受检 dispatch，保留 format 2/schema 1 的解码和原写路径。

这不是当前代码已经提供的双格式能力：当前编译入口采用单一 `FORMAT_VERSION`，wire `seal/open` 使用一个全局 `SCHEMA`，`NodeDomain` 的注册也声明一组 State/Input schema。实现 VN 时必须增加等价的版本感知编译、codec 与 Domain dispatch/注册能力，不能盲目把全局 `FORMAT_VERSION`、`SCHEMA` 或 `GAMEBOOK_REVISION` 改成新值。实现测试须证明：旧 Gamebook 源稿仍走格式 2/revision 2/schema 1 并保留现有对象与 `BookView` 语义；VN 程序仅用其新版本；旧、新 State/Input 均按对应 artifact 受检恢复。若通用 Commit/Checkpoint 容器要改格式，必须先证明现有容器无法仅通过承载带版本对象满足读取，再另行决定；当前只提议在容器确实保持通用时复用它。

Web 交换 View 是另一条版本边界：通过独立的 `@rezics/narrata/vn` 公共子路径提供 version 1 的判别联合，按来源 schema 生成 TS 声明和运行时校验；现有 Gamebook `BookView` 的 choose/finished、presentation 与 reader 交互语义不变，不把 VN 交互伪装成旧 View 字段。View 只包含交互、场景及内容/媒体引用，不包含渲染句柄。安装 VN 子路径的消费者明确选择新依赖，旧 Gamebook 消费者继续使用现有入口。

VN 作者源稿检查与同一候选程序的预览也需通过已安装的第一方公开 API，而不扩张现有 Gamebook `Drafts` 语义。拟提供 VN 专用 browser `@rezics/narrata/vn/authoring` 与 Node/Bun `@rezics/narrata/vn/server` 入口；二者都校验 VN source format 3，并让预览打开本次检查通过的同一候选对象，不发布作品、不切换正式发布，也不写读者存档。当前 `@rezics/narrata/authoring` 和 `@rezics/narrata/server` 保持其 Gamebook 类型与入口。

版本 2 的现有 Gamebook 源稿、schema 1 程序对象与存档继续由保留的受检解码器读取并按原语义运行。已有通用 Checkpoint Bundle 继续装载对象信封；读取器按对象 kind/schema 和其精确 artifact dispatch。已经支持的 R1 存档仍沿用 `packages/narrata/nodes/crates/narrata-nodes/src/r1.rs` 的受检 `r1::migrate_save` 路径迁至 Gamebook 节点格式 2，再按原格式恢复；这不是 VN 迁移。

不对 Gamebook passage 自动补造 speaker、场景或音轨，也不把其现有存档状态强转成 VN State：旧构件缺少证明这些语义所需的来源信息。把 Gamebook 改造成 VN 是显式的作者源稿改写并建立新的 VN 执行/根存档；旧构件和存档继续可读、可分支，不被覆盖。任何未来无损或有损的自动内容转换都需单独决定、定义接受边界并配套语料。

新版本接受前必须添加 `fixtures/compat/nodes-vn-v1/` 冻结语料；不修改 `fixtures/compat/nodes-r2/` 等现存语料。新语料冻结 schema 2 程序与交互/状态对象、当前对白和选项 View 引用、场景目标引用及可导入的历史/Checkpoint；媒体和正文仍是宿主拥有的外部 fixture，不打包其授权地址或解码状态。生成器连续两次输出必须逐字节相同；兼容测试证明旧 Gamebook corpus 在新运行时仍逐字节读取/执行，VN corpus 能在每个对白与选择安全点恢复、分支/回退并稳定拒绝未知 tag、坏引用、错误 schema 和不一致 State/Input。

## VN0–VN3 的后续验收切片

下表是建议的独立实现任务和源码归属。路径是未来拥有者，不表示这些实现目前存在；每项均需在实际任务中写测试并由相应 manager 运行检查。

| 切片 | 主要源码归属 | 最小验收断言 |
| --- | --- | --- |
| VN0：类型、执行、安全点与兼容 | `packages/narrata/nodes/crates/narrata-nodes/src/{source.rs,plan.rs,compile.rs,check.rs,runtime.rs,state.rs,view.rs,wire.rs,session.rs}`；`packages/narrata/nodes/schemas/`；该 crate 的单元/集成测试；`packages/narrata/tooling/crates/narrata-node-tools/` 的语料生成器；新 `fixtures/compat/nodes-vn-v1/` | 连续对白每行均停在安全点；speaker/正文/选项仅输出内容引用；一条选择能分支再汇合；场景目标由状态完整恢复；相同程序和输入轨迹产生相同对象/Commit；无效、stale、重复冲突输入不能跳过行或部分提交；重开/回退不重放旧规则效果；旧 format2/schema1 与现存 corpus 继续通过受检读取。 |
| VN1：可安装 API 和桌面/手机播放器 | `packages/narrata/web/src/vn/`、对应 Rust/Wasm View 与生成校验声明、`packages/narrata/web/package.json` 与 `README.md`；新增 `examples/visual-novel-web/` 及浏览器套件 | 固定安装包的 `@rezics/narrata/vn` 子路径公开有类型的打开/推进/选择/呈现 API；桌面及手机宽度可完整玩完含多行对白、背景、立绘、分支、汇合和至少一条音乐或语音的短场景；关闭重开、回退和另选分支恢复相应对白/场景；播放器使用来源宿主提供的媒体解析，不使用 V0 global 或 DOM 句柄作存档。 |
| VN2：按需媒体/剧情与恢复 | `packages/narrata/web/src/vn/`；与 `packages/narrata/web/src/artifacts.ts`、`content/`、`storage.ts` 的窄接口整合；`examples/visual-novel-web/` 的模拟宿主和浏览器测试 | 首屏不取全作；当前场景及直接候选的预取受界、过期时取消；失败可重试且不重复输入；A 运行期间发布 B 不混用引用；不可用/撤回不假报结局；离开场景可释放解码资源；清理资源缓存不删除未同步存档；恢复遵守音乐从头和语音当前行从头的策略。 |
| VN3：作者即预览、短篇和固定包交接 | `packages/narrata/tooling/crates/narrata-authoring/`、`packages/narrata/tooling/crates/narrata-authoring-wasm/`；`packages/narrata/web/src/vn/` 的公开 authoring/server 接缝；`packages/narrata/web/package.json`、README 与 pack/install scripts；`examples/visual-novel-web/` | 通过已安装的 VN browser authoring 与 Node/Bun server API 校验 VN 源稿、预览与发布；预览的是同一检查通过的候选且不发布/写存档。交付多场景、多条对白、分支与结局短篇，含背景/立绘/音轨和中途恢复。固定 tarball 须在浏览器 reader、browser authoring 及所需 Node/Bun authoring/build/publish consumer 路径安装验证；记录首屏和候选资源请求身份/字节数、安装包大小、保存恢复与失败重试证据及来源提交和 tarball SHA-256。桌面、手机浏览器回归通过。 |

Gamebook 作者、发布、存档、历史、地图及维护能力继续沿用现有 Goal 已验收的归属和接口；本 ADR 不重开它们。VN0–VN3 均完成也只是 Narrata 单仓库能力与固定包的交付，REZICS 的真实联合旅程仍按[集成文档](../integrations/rezics.md#web-vn-真实联合验收)单独验收，不据此宣布 VN 产品上线。

## 后果与未验证项

接受此 ADR 后，后续实现才可以据此增加节点类型、对象 schema、生成 View/API、资源预取及场景组件。格式和源稿版本的精确编码、编译时不变量、Schema/TS 生成、恢复校验、缓存键实现和浏览器回归均尚未编写或运行。当前并不存在可安装 VN API、VN reader、媒体 resolver、场景播放器或新的冻结 corpus；不能把本页的路线称为已交付功能。

实现至少应验证：稳定 ID 与选项顺序如何定义对白位置；程序 schema 是否按对象 kind 单独版本化；Checkpoint Bundle 对新增 schema 的通用导入是否确实无需变更；媒体不可变版本引用如何由模拟宿主生成/授权；内存/IndexedDB 多标签和用户关闭声音后的恢复在目标浏览器如何表现。若其中一项推翻本文格式边界或恢复语义，应先提出 ADR 修订，再冻结语料。
