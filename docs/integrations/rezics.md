# REZICS 集成

更新：2026-10-31。首个产品 Gamingbook 采用正文阅读与分支选择；传统视觉小说随后纳入，方向是纯 Web、按需分发。本文据重建后的宿主重新整理首发能力请求，取代 2026-10-05 对 Occurrence、Expression、Structure 等宿主实现的映射假设。

Narrata 的工作不修改 REZICS 仓库（[决定 13](../product/decisions.md)）。本页供维护者交给 REZICS 评估和实施；“提出”表示尚未取得宿主接受及联合验收证据。字段、限额、交换格式以现有 schema 与 ADR 为准；需要改变格式时先写 ADR，本页不另定协议。

## 调研基线与已具备的基础

2026-10-29 通过只读 SSH 检查 Fedora 上的 /home/edge/projects/rezics/rezics-next：main 为 9bc3c83d810e4540995d31ac10bf4c7f404aff58，dev 为 3415ec5b0f。检查了两个分支的 apps、packages、services、contracts、specs，尚未找到 Narrata、Gamingbook、Gamebook 或 narrata-choice 的命名接入；这不能证明没有别名实现或尚未合入的工作。以下是源码审查，没有运行 REZICS 检查或验证部署状态。

| 远端基础 | 证据位置 | 对接含义 |
| --- | --- | --- |
| Document 的块有稳定 ID，移动或编辑保留、复制换 ID；精确外部引用还固定文档修订 | packages/document/README.md 的 Interoperability | 可用于 Segment 锚点；仍需正文单元到文档修订的宿主解析层 |
| extensionBlock / extensionInline 保留 definition、version、payload、fallback | 同页 Components and exchange | 可承载选择标记；不等于已有标记编辑器或叙事执行 |
| RichTextEditor 已接入帖子编辑 | apps/web/src/threads/composer.tsx | 可复用编辑能力；作品草稿、选择编辑和试玩尚需接线 |
| 现有不可变文档读取绑定 imported field、semantic candidate 与 summary source fence | services/kernel/src/content/document-read.ts | GET /v1/documents/:id 不能直接视为通用作品正文解析器 |
| Library progress 只有 scalar_amount、cursor_position、completed_units | services/kernel/src/library/progress.ts | 可承载阅读摘要；不包含游戏变量、调用栈或提交分支 |
| 媒体用途目前是 post-image 和 cover，变体是图片 | services/kernel/src/content/media.ts | 图片底座可复用；尚不能当作通用剧情构件或 VN 音视频资源接口 |
| 命令有分区 authority、事务、receipt 与 outbox | services/kernel/src/command/command.ts | 发布和存档适配应复用宿主的权限与命令机制，保持 Narrata 的原子确认语义 |

旧版 Visual Novels 目录、旧 Studio 与旧章节模型不作为当前宿主的实现证据。2026-10-05 的方案可从本仓库历史读取；旧 [REZICS Goal 简报](rezics-goal.md) 以本次能力复核为输入。

## 所有权

| 能力 | 交付方 |
| --- | --- |
| 正文、选项文字、媒体、译本、授权、审核、内容修订与撤回 | REZICS |
| 作品身份、草稿存放、发布入口、网站路由、用户身份与物理存储 | REZICS |
| 节点、选择、条件、后果、叙事 ID、编译、执行、历史、存档语义与迁移判断 | Narrata |
| 宿主专用内容解析、原生正文渲染、账号与网络存储适配 | REZICS，消费 Narrata 的类型与一致性测试 |
| 通用创作 API、发布对象生成、运行时包、阅读控制器、适配契约与诊断 | Narrata |

正文与结构在运行时分开，作者工作视图里可以一起编辑。REZICS 不需要解释条件表达式、计算程序摘要或另写叙事解释器。

## 能力请求

下面七项按首发用户旅程组织，沿用旧 R1–R13 编号作为追踪对应，不新铸一套协议或任务编号。

| 首发能力 | 对应旧请求 | 状态 |
| --- | --- | --- |
| 作者草稿、选择编辑与私有预览 | R2、R4、R5、R12 | 提出 |
| 内容单元、标签、块大纲与批量解析 | R3、R5、R8、R11、R12 | 提出 |
| 完整发布与不可变对象分发 | R9 | 提出 |
| Gamebook 阅读入口与按需加载 | R1、R10 | 提出 |
| 私人游戏存档与在线确认 | R6 | 提出 |
| 书架继续阅读与完成状态投影 | R1 | 提出 |
| 撤回、访问权限、私有缓存与搜索边界 | R7 及上述请求的访问边界 | 提出 |

### 作者草稿、选择编辑与私有预览

REZICS 为作品保存正文、选项文字和 Narrata 的不透明结构记录，提供编辑权限、修订比较更新与冲突后保留输入。作者能在正文处插入选择、编辑条件与后果、指定去向并立即试玩；选择标记可以使用已注册的 extensionBlock 定义。宿主的记录存储与派生索引消费 Narrata authoring 的校验结果，不自行解读结构载荷（[ADR 0022](../adr/0022-authoring-publication-and-reader-map.md)）。

预览绑定候选草稿及本次内容修订，使用与正式阅读相同的 Narrata 执行语义；预览不会发布作品或写入读者正式存档。

验收：作者创建三段正文、两个选择点、两个结局，刷新后继续编辑；改去向后能试玩；并发保存产生可处理的冲突；匿名访问不能读私有草稿；编译失败定位到对应节点或选择点。

### 解析与结果

REZICS 提供稳定、不透明的内容单元和标签 key，映射到自身当前内容模型，不预设它们必须是 Occurrence、Work 或 Post。Narrata 持有 ContentRef 和 Segment；内容方负责文档、修订、段落与译本的对应。

发布检查获得有界的 ContentOutline：完整有序块 ID 和选择标记位置，不含正文。修订绑定是宿主本次调用或发布作业的上下文元数据，不向现有 ContentOutline 增加 revision 字段。阅读解析对同一画面的正文、回应、标题、标签使用同一语言/译本上下文，批量返回逐项 ok、unavailable 或 incompatible；不存在、无权读取与撤回合并为 unavailable。发布检查绑定当次内容修订；之后阅读可以采用新的当前修订，但该次锚点检查和正文截取须使用同一修订。

逻辑构件固定与正文修订是两种政策：当前[内容引用契约](../contracts/content-references.md)允许正文按宿主当前可用修订重新解析；本次请求不增加“所有正文永久固定”的承诺。若以后提供原样重读或离线固定章节，再交付指定修订读取及保留策略。首发可以单语言，但接口保留语言/译本上下文；多语言上线时再验收块 ID 和标签联动。

验收：正文与标签使用同一上下文；缺锚点得到明确诊断；部分引用不可用不泄露原因；预取不推进状态；宿主原生 Document 渲染与 Narrata 的段范围相同。标准解析载荷仍为文字/文字块；`@rezics/narrata/content/native` 已提供修订绑定的原生快照、段截取和文字回退适配。REZICS 把自己的 Document 转成该快照并提供原生渲染，双方用同一修订验证，不把插槽存在当作宿主转换已完成。

### 完整发布与不可变对象分发

Narrata 从受检草稿与大纲生成发布对象；REZICS 接收对象集合，提供按摘要读取，并为作品保存当前可玩发布的指针。依赖对象完整上传、核验且必要读取已就绪后，以修订号切换当前发布；宿主复用其 command/receipt/outbox，异步工作完成前不把新发布暴露为可玩。

运行中的会话按自身构件摘要加载，不能随“当前发布”变化混入其他版本的程序块。旧的无正文程序构件继续可读；正文和媒体仍遵守各自保留及撤回政策。对象存放不能借用仅接受图片的上传接口。构件与发布集合以 ADR 0013、0016、0022、0023 为基线；实际生成能力随 Narrata 交付包验证。

验收：上传中断或必要对象缺失时继续提供旧发布；重复提交得到可核实的同一结果；读者运行 A 时作者发布 B，A 后续读块仍来自 A，新会话可进入 B；已发布对象内容不能原地覆盖。

### Gamebook 阅读入口与按需加载

REZICS 在作品页给出开始/继续入口，独立的互动阅读路由懒加载 npm/Wasm 包，接入身份、ContentResolver 与宿主正文渲染；由 Narrata 控制选项合法性和推进。当前 SDK 已支持清单加 fetchChunk 的按需加载，普通网站路由无需提前加载引擎。

验收：进入作品只取得运行时、清单和当前必要块；正文、局部回应、选择、结局正确；缺块或解析失败可以重试，不能显示成正常结局；刷新仍打开该游戏会话。首发匿名可用浏览器存档，登录读者按[ADR 0021](../adr/0021-server-save-storage.md)只在线，不先承诺登录离线双向同步。

### 私人游戏存档与在线确认

REZICS 为读者的游戏会话提供隔离的存档库与认证传输，适配既有 ADR 0017/0021 批次协议和 Narrata CacheStore.load/persist。沿用[决定 15](../product/decisions.md#15-服务器存档实现同一个存储契约)：每“用户 × 作品”一个库，物理上使用库、对象、键三张表。新宿主的 authority 与分区如何映射到现行布局，由双方共同核对；需要改变物理布局、库作用域、生命周期或批次语义时，先提出相应决定/ADR 修订，未接受前仍以现行契约为准。

服务端原子处理对象与引用更新、修订号 CAS、重复请求、结果未知核实、配额、删除及删除后防复活。persist 只有在原子批次持久化后确认；receipt 的已接收或 outbox 的已排队不等于已保存。冲突可以首发明确拒绝并重载，不要求自动合并。

验收：收到保存成功后刷新/重新登录恢复同一构件、选择位置和变量；旧设备不能覆盖新设备的进度；超时后可回读判断是否成功；他人不能读取；已删除的库不被延迟重试复活。存档语义、对象校验、迁移与 GC 由 Narrata 提供，不由宿主 ORM 重建。

### 书架继续阅读与完成状态投影

REZICS 在游戏存档确认后，把可恢复会话的引用、最近活动与完成状态投影到 Library。现有进度能力可提供展示接缝，具体 locator 是否适合承载该引用由宿主验证；游戏完整状态保留在 Narrata 存档库。

验收：“继续阅读”恢复相应会话；投影延迟不影响存档和执行；多个分支不被压成一个章节序号；completed_units 不能被当作已做选择的证据。投影可经 outbox 更新，与存档的同步持久化确认区别处理。

### 撤回、访问权限、私有缓存与搜索边界

REZICS 在解析正文和媒体时执行当前权限与撤回政策；公开作品身份、简介与允许搜索的文本，与尚未揭示的分支正文分开处理。首发分支正文不进入普通公开全文搜索/摘要/SEO，按旧 R7 的意图实现。

宿主声明哪些内容可公开缓存、哪些只属当前账号及哪些不能持久缓存；自有 Service Worker/Cache API 也落实该策略。退出账号清理私有资源与会话隔离；清理可重新下载的程序/媒体缓存不删除未同步存档。按需加载只减少流量，不能单独证明客户端资源不被检查或读者已获授权。

验收：无权/撤回内容返回 unavailable，阅读器保留进度并说明当前不可用；切换账号不显示前一账号的私有内容；公共搜索不返回受保护分支。已合法交付到读者设备的字节不能被承诺为远程可彻底收回。

## Gamebook 完整阶段的宿主配合

这些请求对应 Narrata Goal 的 M2，首发接入后继续按实际交付版本验证；仍由 REZICS 实施。

- **作者图（旧 R9/R10）**：在作者权限下提供图索引、瓦片和标签对象的按需读取，把 Narrata 图组件接入作者界面。对象读取与标题解析沿用发布身份和当前权限，不把作者全图当作读者地图下发。验收懒加载、分层切换和受限访问。
- **关系投影与读者地图（旧 R13/R1）**：接入已交付的 visits、揭示地图和稀疏关系 SDK，按 ADR 0022 提供经授权的投影装载与有界查询。服务端先验证账号和存档库权限，再用精确 artifact 对应的受信发布、历史祖先对象及迁移描述符核实访问记录；缺失历史保留不完整结果，不用预取集合补全。作者完整查询与读者稀疏查询分别授权，读者地图接口不下发作者全图。具体入口和安装步骤见[地图与关系接入](#地图与关系接入)。宿主原生分区与表布局适配先核对，涉及格式或现行契约改变时先修订 ADR。
- **作品演化（旧 R9/R6/R1）**：保存发布对象集合中的可选演化描述符；受限服务作业可按授权枚举必要存档头并读取对象，调用 Narrata 影响评估后输出脱敏报告，不因此给作者开放读者存档。阅读器承接自动计划、确认、旧版继续与重新开始结果，确认及落盘前保留原进度；不自行推导迁移。
- **维护与删除（旧 R6）**：接入已交付的受检存档导入导出、有界根/pin 枚举、所选根 CAS 删除、完整性检查和 GC；宿主提供库授权、账号绑定、配额、可信时钟、grace 与作业调度，按[存档维护接入](#存档维护接入)处理清理与失败重试。在线原地浅压缩、影子库路由切换及布局迁移仍推迟。故障重试不跨账号、不复活已删除的库；物理存储继续遵守决定 15，不能把元数据投影当恢复真相。
- **多译本（旧 R3/R5）**：启用时提供同上下文的正文/标签选择、稳定块对应和缺译回退。大纲和截取绑定当次修订；这不要求为首发先建设完整翻译平台。

## Web VN 后续需要的宿主能力

传统 VN 复用前述发布、对象加载、内容引用和存档基础；逐句推进、说话者、背景、立绘、音轨的可恢复呈现状态由 Narrata 的 VN 能力与播放器提供，不进入 Library progress。

REZICS 后续增补：

- 面向作品资源的媒体身份、授权、撤回与上传，支持背景、立绘、音乐、语音等用途，不限于帖子图片容器。
- 有版本的资源清单及类型、字节数、尺寸/时长、变体信息；按授权给出资源地址。图片尺寸和音视频编码变体按实际播放器需要提供，大音视频可按需支持 Range。
- 程序块、正文和媒体独立交付，播放端能按场景预取、取消、重试；不可变资源使用稳定版本地址，更新提供新地址。
- 若产品明确提供离线下载或原样重读，再增加允许下载的范围、指定内容/媒体版本读取、保留与清理政策。浏览器资源缓存本身不承担永久存档保证。

Narrata 负责场景依赖、推进与恢复语义；播放器负责资源准备、浏览器音频解锁、解码失败和呈现协调。把故事改成 Web 分发不要求先下载全作品，也不要求把每次 DOM/音频操作写进宿主业务事件。

参考：[Ren'Py 渐进下载](https://www.renpy.org/doc/html/web.html#progressive-downloading)、[Monogatari 资源预载](https://monogatari.io/v2/configuration-options/game-configuration/asset-preloading)、[浏览器存储与清除](https://developer.mozilla.org/en-US/docs/Web/API/Storage_API/Storage_quotas_and_eviction_criteria)、[Cache API](https://developer.mozilla.org/en-US/docs/Web/API/Cache)。这些是分发经验，不定义 Narrata 格式。

## Narrata 自己的交付依赖

宿主要求随固定版本交付物一起核对。当前主线已提供以下通用入口，调用和生命周期见[包说明](../../packages/narrata/web/README.md#authoring-and-publication)：

- 浏览器 `@rezics/narrata/authoring`、Node/Bun `@rezics/narrata/server` 的 `loadAuthoring()`：逐条记录校验/组装、ID 铸造、诊断、预览、发布及按描述和单个对象受检重开。发布返回对象与规范记录/墓碑；宿主负责完整上传、原子保存草稿及最后的发布引用 CAS。
- 根入口及 `@rezics/narrata/reader` 使用独立 reader Wasm，首屏不带编译、发布、布局或 trace；旧 raw Wasm 保留兼容入口。作者能力按需加载，部署需保留包内分别定位的 Wasm 资源。
- `@rezics/narrata/content/native` 支持同一修订的原生文档片段与文字回退；内容大纲仍不含正文或 revision 字段，具体宿主模型转换与权限由 REZICS 实现。
- 根入口的 `AuthenticatedLibrary` / `createOnlineSession` 提供固定库实例、未知保存原字节重试与保留证明、配额/冲突恢复、关闭/账号失效和删除重建接缝，并按存档的精确 artifact 打开。调用方式与宿主责任见[在线旅程](../../packages/narrata/web/README.md#authenticated-online-journeys)；宿主仍提供认证传输、库授权与对象保留。

M1 本地固定交付已通过独立 tarball 的 Node/Bun/Chromium 验证：公开创作的同一复合短篇、真实正文/标签、冷读取、A/B 逻辑隔离与在线故障恢复均有证据。接受的参考入口为 `/?reference`，`/` 仅作旧兼容示例。固定输入为版本 `0.1.0-alpha.1`、[源码 fa0266b1063428e855efea1408768029818500d0](https://github.com/rezics/narrata/tree/fa0266b1063428e855efea1408768029818500d0)、tarball SHA-256 `524a383e20ecb147351a496a26a18993aa00f00a19b03c565a123d944bf7ff19`；由维护者提供该文件，不把同版本其他构建或 `latest` 当同一输入。尚未发布 npm registry；同一产品源码树的 a6f8952 已通过[主线 CI](https://github.com/rezics/narrata/actions/runs/40923433584)。

正文依宿主当前可用修订解析，同一画面/原生片段使用一次解析快照；固定旧逻辑构件不保证历史正文永久原样可得。visits、揭示读者地图、稀疏关系与有界 PostgreSQL 装载/查询的公开 SDK 已交付并通过固定包验收，涵盖独立安装的 Node/Bun/Worker、参考阅读器浏览器及真实 PostgreSQL 检查。地图增量历史固定输入为版本 `0.1.0-alpha.1`、[源码 5c02e6107c20858b67a0ce9d643b3b9de73bc218](https://github.com/rezics/narrata/tree/5c02e6107c20858b67a0ce9d643b3b9de73bc218)、tarball SHA-256 `e2751a827b71629cf139e5d371123787af80cc19635d6285f1c4756f1fa0aaf3`；由维护者提供该文件，不复用上述 M1 历史包或同版本的其他构建。npm registry 仍未发布。存档维护能力的固定交付见下文；作者端演化规划、描述符、评估/应用控制器与只读影响作业及存档维护均已有各自固定包验收记录。最终完整 Gamebook SDK 固定包（见下文）包含完整阅读器迁移与历史读取能力及全部 M2；REZICS 真实联合验收尚未执行。

存档维护增量已通过固定包验收，涵盖独立安装的 Node/Bun/浏览器、IndexedDB 和真实 PostgreSQL。固定输入为版本 `0.1.0-alpha.1`、[源码 ffc0f966578c184ef45f36559001bc029fbdc3a2](https://github.com/rezics/narrata/tree/ffc0f966578c184ef45f36559001bc029fbdc3a2)、tarball SHA-256 `120771d9253c6185bf3e4a4ca3e5a639600fcd060dceaba791c6fbd0517c54b8`；由维护者提供该文件，不把上述历史包或同版本其他构建当作本次输入。npm registry 尚未发布。

后续接入以本次完整 Gamebook SDK 固定包为基线：版本 `0.1.0-alpha.1`，[源码 a36d4966d7bf40df9e03f38e5d080885d73595ff](https://github.com/rezics/narrata/tree/a36d4966d7bf40df9e03f38e5d080885d73595ff)，tarball SHA-256 `0a1ed393ab8b60ad1400609fcd26b0a060911917fce88169efc459d25819da30`，对应[主线 CI 记录](https://github.com/rezics/narrata/actions/runs/41182162270)。此包包括 M1 与原定全部 M2 的作者图、关系、演化、存档维护及完整阅读器历史/迁移能力。M1、地图、维护的旧 tarball 仍保留各自历史身份与验收证据，不拼成或互换为最终包。最终包本地验收包括 138 项 Web 包测试、25 项独立安装故障用例、Node/Bun/Chromium 提供历史名称的旅程、桌面与 390px 阅读器全矩阵；R1、存储、脚本与文档检查均通过。npm registry 尚未发布；REZICS 真实联合验收尚未执行，七项能力请求仍为“提出”。

REZICS 侧需保留每个历史构件的精确清单、程序块，以及随发布提供的名称元数据，并按需提供该构件；升级界面呈现 SDK 评估报告，对要求确认的计划取得读者确认。存档服务提供耐久的修订比较写入，结果未知时保留原请求并核实，同时落实账号隔离及关闭/删库生命周期。升级已确认后，只有在对应构件的正文与标签解析就绪时才开放新画面的选择；尚未就绪时保留已确认进度并允许重试。接口操作顺序见 [SDK 在线旅程说明](../../packages/narrata/web/README.md#authenticated-online-journeys) 与 [存档维护说明](../../packages/narrata/web/README.md#save-maintenance)。

[Gamebook SDK](../../packages/narrata/web/README.md) 的完整固定包已通过本地及主线 CI 验收，源码与包身份见上文；tarball 由维护者提供，安装前核对固定身份。[Web VN Goal](../goals/web-vn/GOAL.md) 已进入 VN0 语义与兼容设计阶段；此状态不表示 VN 能力已交付。Narrata 的模拟宿主和参考适配在本仓库实现，REZICS 专用实现仍只列为本页能力请求。七项 Gamebook 请求仍为“提出”，npm registry 尚未发布，真实联合验收尚未执行。

### 地图与关系接入

按[包说明](../../packages/narrata/web/README.md#reader-maps-and-relation-hosting)接入，不另建地图协议。宿主在服务端 `loadAuthoring()` 后创建隔离的 evolution 上下文，注册精确构件及历史所需的受信发布；迁移描述符只从受信发布的 evolution 角色登记。恢复已确认 checkpoint 或连接已授权存档库后调用 `visits()`，再生成 `readerMap(publication)` 或 `readerRelations(publication)`。保留祖先和迁移证据以恢复完整地图，结束后释放 visits/关系句柄并关闭上下文。客户端 checkpoint 和 visits 只服务个人剧透保护，不能证明奖励资格、权益或访问权限；结构 first/route 查询也不证明首次阅读或可执行路线。

关系库由宿主显式安装 DDL：从 `@rezics/narrata/server` 的 `referenceRelationsSql` URL 读取包内 `dist/server/relations-v1.sql`，再提供 `TransactionalSql` 给 `PostgresRelations`。驱动参数需保留原生 `null` 和 boolean，不转成字符串。SDK 原子装载不可变完整投影，并校验重试及查询回读；宿主分别授权 `load`、有界作者 `query` 和稀疏读者 `readReader`，提供受授权的事务及查询连接。SDK 不自动安装或迁移宿主数据库 schema，也不代替宿主认证。

读者点击地图时再懒加载 `@rezics/narrata/reader-map`；需要 React 视图时另加载 `@rezics/narrata/reader-map/react`。用 `OnlineSession.exportConfirmedSave()` 的 artifact/cursor 调用 `readReaderMapResponse(response, identity)` 校验响应，账号、库绑定或已确认 cursor 变化后丢弃旧响应；正文及标签仍由宿主按权限解析。这两个独立入口不引入作者图、编译器或 Wasm。

### 存档维护接入

按[包说明](../../packages/narrata/web/README.md#save-maintenance)按需加载 authoring/server，调用 `createMaintenance`，绑定已授权传输和不可变 StoreId，注册存档需要的精确受信发布。先 `inspectSave` 核实完整/浅 checkpoint 或 legacy JSON 的 artifact/cursor，再按观察到的修订调用 `importSave`；`expected_revision: null` 只允许目标不存在时创建。`exportSave` 导出完整或浅 bundle；浅导出的祖先数是下限，额外保留重建精确当前页面、局部回应和调用参数的上下文，不足时在移动持久根前拒绝。完整导出和 replay 仍拒绝真正截断的历史。重新打开时，宿主按存档的精确构件身份提供其清单、必要程序块及可用的名称元数据；完整阅读器迁移、时间线和按需历史读取包含在上文固定 SDK 包中，具体调用见 [在线旅程说明](../../packages/narrata/web/README.md#authenticated-online-journeys)。

用 `run` 分页读取 roots/pins，按返回游标继续；展示可删除的根，在读者选择后用已观察修订和 commit 调用 `delete_roots`，冲突时重读。`integrity` 只读；`collect` 必须显式提供可信 Unix 秒时钟、`grace_seconds` 和 `dry_run`，先展示候选再执行。有限或无效扫描不能授权 GC；全部对象仍被根保活时可能无法回收空间，不能因配额自动删除读者保留的路线。原生读取按预算在复制对象载荷前拒绝；Web 的 `load_bytes` 是完整编码回复的累计接收阈值，可能先收到一个符合传输上限的拒收回复，再阻止其进入维护缓存/Wasm，不能当作宿主网络 I/O 或分配内存的硬上限。

Node/Bun 使用 `openLibraryMaintenance(authorizedPostgresStore)`，从同一 `PostgresStore.usage()` 读取一致的使用量和配额。确定的 quota 拒绝保留原库：重新读取、明确选择根、CAS 删除、检查 dry-run、执行 GC，再显式重试导入。写入结果未知时暂停其他维护操作；`retryPending()` 只重传原字节，随后重读持久根确认结果，不重跑原操作或恢复丢失的返回值。结束后 `close()`；使用 `AuthenticatedLibrary` 继承账号关闭处理，普通已授权传输须由宿主在账号切换或删库时关闭旧句柄。

认证、账号与库绑定及作业调度仍由 REZICS 实施，能力请求保持“提出”。SDK 不自动安装或迁移宿主数据库 schema；在线原地浅压缩、影子库构建与 CAS 路由切换、布局迁移及后台调度仍按 ADR 0021 推迟，不能将浅导出重新导入原库当作原地压缩。存档保留策略见[存储与存档](../contracts/storage-and-saves.md#保留策略由宿主选择)。

## 首次联合验收

使用固定 npm tarball/版本、源码提交与校验和，在 REZICS 的真实浏览器环境完成：作者建短篇并保存，私有预览、修改并发布 A；读者选择后退出；作者发布 B；旧读者恢复 A，新读者进入 B；模拟存档写入结果未知、设备冲突、正文撤回与账号切换。

记录双方源码提交、包版本、内容解析策略、存档模式、实际通过项与缺口。Narrata 的模拟宿主检查和 REZICS 的单仓库检查都不能代替这条联合旅程。真实 REZICS 能力尚未接受时，不把“提出”改写为“已支持”。

### Web VN 真实联合验收

后续 VN 使用固定播放包，在真实宿主打开多场景短篇：开启声音，逐句推进与选择分支，跨场景按需取得背景/立绘及音乐或语音，中途保存后关闭重开并恢复对白和场景/音轨状态；模拟资源失败、重试和撤回，确认不重复推进或丢失进度。

记录资源请求范围、恢复策略、双方版本和实际通过项。该旅程在 REZICS 就绪后执行；Narrata 的模拟宿主与安装包交付可独立完成，但真实联合旅程通过前不宣称 VN 产品已上线。
